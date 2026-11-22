//! Hollow's own writes to a workspace's shared docs, and the safety net for
//! an update lost between parallel chats (concurrency programme PR 2,
//! 2026-10-20).
//!
//! Every conversation in a workspace's own folder shares three files,
//! [`SHARED_DOCS`]: WORKSPACE.md, PLAN.md and SOURCES.md. Agents edit them
//! with their own tools, which take no Hollow lock. Claude Code's Edit
//! replaces a string in the file as it is now (and fails if the string is
//! gone), but its Write replaces the whole file and refuses only when the
//! file's mtime moved since the agent last read it; Codex's apply_patch is
//! context-based. So an agent that writes a shared doc back from an earlier
//! copy silently undoes whatever landed since. Three defences:
//!
//! 1. **Hollow's own writes** ([`update_doc`]: rename, status, canvas look,
//!    department, the gardener's archive) are compare-and-swap on the file's
//!    revision, retried a few times and then refused with a plain message.
//!    While any run is live in that workspace they move the mtime, so an
//!    agent's stale-read check sees them; with none live they keep it,
//!    because the overview orders workspaces by the brief's mtime.
//! 2. **The prompt note** (`context_compiler::shared_workspace_note`) goes
//!    with every turn in a workspace's own folder while
//!    [`workspace_allows_parallel`] holds, a peer live or not. Since V5b it
//!    names the chat's own `log/<key>.md` ([`chat_log_path`]): chats append
//!    there and write the shared docs only for workspace-wide changes.
//!    Chats that still write the shared docs fall to defence 3.
//! 3. **The safety net** at a scoped run's finish ([`run_finishing`]). Hollow
//!    keeps in memory each shared doc's text at every scoped run's start
//!    ([`run_began`]), each finished run's before and after (while another
//!    run there is still live) and each of its own writes' before and after,
//!    within budgets ([`push_version`]: 256 KB a side, 64 versions a doc,
//!    4 MB a workspace, 16 MB in all; past one, that doc stops being
//!    journaled until the runs live then have finished). At a run's finish,
//!    a version published during its life whose added lines are ALL gone
//!    from the file now (what a write from an older copy looks like) is
//!    acted on, at most [`MAX_RECOVERIES`] per finish:
//!    - **Hollow's own write** (certain provenance; an agent is unlikely to
//!      revert a rename on purpose) is merged back with a line-level
//!      three-way merge (base = its before, ours = the file now, theirs = its
//!      after) and written compare-and-swap, while the run is still live, so
//!      the merge is part of its History change.
//!    - **Another run's version** is never written back into the file (this
//!      run may have removed those lines on purpose, review 2026-10-20): it
//!      is kept beside the file as `<name> (conflicted copy <stamp> <id>).md`,
//!      which Waiting on you offers like any conflict copy (and which is in
//!      the run's History too). A later version that already holds an
//!      earlier one's lines covers it: one copy. A Hollow write that won't
//!      merge cleanly is kept the same way.
//!
//!    Either way the run gets a `sharedDoc` flag (`run_guard`) whose detail
//!    says what happened, including when the copy itself couldn't be made.
//!
//! The compare-and-swap writes themselves catch an agent that writes between
//! Hollow's check and its replace: on macOS the replace is an atomic swap
//! whose displaced file is checked (`vault::write_note_expecting`). That
//! covers a replace-style write (Claude Code's Write) or an in-place write
//! completed in the window, not an in-place writer that holds the old file
//! open across the swap and writes after the check.
//!
//! Limits, chosen on purpose (build log 2026-10-20): a version only some of
//! whose lines are gone is taken as a deliberate edit and left alone; a
//! run's own mid-turn edits are never observed (only starts, finishes and
//! Hollow's writes), so a still-running run's line lost to a peer's stale
//! write is caught only when the run that lost it finished first; a
//! deliberate removal of everything another run added still gets a copy
//! and a flag (never a rewrite); a Hollow value put back exactly on purpose
//! is merged back (flagged); pure deletions aren't detected; the journal
//! lives in this process only.

use crate::run_guard::RunFlag;
use crate::types::NoteContent;
use crate::vault::{self, CasOutcome, Expected};
use once_cell::sync::Lazy;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The files every conversation in a workspace's folder shares.
pub(crate) const SHARED_DOCS: [&str; 3] = ["WORKSPACE.md", "PLAN.md", "SOURCES.md"];
/// Where chat `session_key` keeps its own log, relative to its workspace's
/// folder: `log/<key>.md` (V5b, 2026-11-15). One writer per file, so parallel
/// chats never clash; they touch the shared docs only for a change the
/// whole workspace needs. `None` unless the key is a well-formed session key
/// (lowercase letters, digits and `-`, opening `s-`), so no `..`, `/` or
/// absolute path can reach it. Not a deliverable (those are `outputs/`), not
/// a protected file and not in the context envelope; the safety net below
/// does not watch it.
pub(crate) fn chat_log_path(session_key: &str) -> Option<String> {
    crate::agent_sessions::is_safe_session_key(session_key).then(|| format!("log/{session_key}.md"))
}

/// Compare-and-swap attempts before a write gives up.
const TRIES: usize = 4;
/// A shared doc over this is not journaled (the context limit is 48 KB).
const MAX_DOC_BYTES: u64 = 256 * 1024;
/// Most cells of the line-matching table a merge may use (the common prefix
/// and suffix are taken off first); past it the merge counts as not clean.
const MAX_MATCH_CELLS: usize = 2_000_000;
/// Versions of one doc a journal keeps.
const MAX_VERSIONS_PER_DOC: usize = 64;
/// Bytes (before + after) one workspace's journal keeps.
const MAX_JOURNAL_BYTES: usize = 4 * 1024 * 1024;
/// Bytes all journals keep together.
const MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;

/// Whether workspace `id` lets more than one conversation run in its own
/// folder at once: Settings, Chat, "Chats at once in one
/// workspace" above 1 (concurrency PR 4, 2026-10-20). The setting is
/// app-wide, so every workspace gets the same answer; `id` stays for a
/// per-workspace rule later. The shared-workspace note is sent on every
/// turn in the folder while it holds; at 1 the run pump starts one turn
/// there at a time, so there is no one to share with.
pub(crate) fn workspace_allows_parallel(_vault: &Path, _id: &str) -> bool {
    crate::chat_limits::current().chats_per_workspace > 1
}

/// True while any scoped run (a conversation in the workspace's folder, or
/// its build worktree's turn) is live in workspace `id`.
pub(crate) fn chat_live(vault: &Path, id: &str) -> bool {
    !crate::automated_history::scope_holders(vault, &format!("workspaces/{id}")).is_empty()
}

/// Hollow's own change to shared doc `file` of workspace `id`: read it,
/// `transform` the text, write it back only if the file is still what was
/// read, and try again (up to [`TRIES`] times) if it changed meanwhile. The
/// mtime moves while a run is live there and is kept otherwise (see the
/// module doc). Errors if the file is absent.
pub(crate) fn update_doc(
    vault: &Path,
    id: &str,
    file: &str,
    transform: impl Fn(&str) -> Result<String, String>,
) -> Result<NoteContent, String> {
    let rel = format!("workspaces/{id}/{file}");
    let full = vault::safe_join(vault, &rel)?;
    for _ in 0..TRIES {
        let current = fs::read_to_string(&full).map_err(|e| format!("read: {e}"))?;
        let next = transform(&current)?;
        #[cfg(test)]
        before_write_for_test();
        let live = chat_live(vault, id);
        let expected = vault::revision(current.as_bytes());
        match vault::write_note_expecting_mtime(
            vault,
            &rel,
            &next,
            Expected::Revision(&expected),
            !live,
        )? {
            CasOutcome::Written(note) => {
                note_hollow_write(vault, &format!("workspaces/{id}"), file, &current, &next);
                return Ok(note);
            }
            CasOutcome::Conflict(_) => continue,
        }
    }
    Err(format!(
        "{file} kept changing while Hollow tried to update it (a conversation in this workspace is editing it), so nothing was changed. Try again in a moment."
    ))
}

#[cfg(test)]
thread_local! {
    /// Runs between [`update_doc`]'s read and its write (tests only).
    static BEFORE_WRITE: std::cell::RefCell<Option<Box<dyn FnMut()>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
fn before_write_for_test() {
    BEFORE_WRITE.with(|hook| {
        if let Some(hook) = hook.borrow_mut().as_mut() {
            hook();
        }
    });
}

/// One published version of a shared doc: what it was before and after.
#[derive(Clone)]
struct Version {
    seq: u64,
    file: &'static str,
    before: String,
    after: String,
    /// The run that finished with it, or [`BY_HOLLOW`].
    by: String,
}

const BY_HOLLOW: &str = "hollow";

/// One workspace folder's journal: its live scoped runs (the sequence number
/// each began at and the shared docs it began with) and the versions
/// published since the oldest of them began.
#[derive(Default)]
struct Journal {
    seq: u64,
    runs: HashMap<String, (u64, BTreeMap<&'static str, String>)>,
    versions: Vec<Version>,
    /// Docs no longer journaled because a budget ran out, with the sequence
    /// number it happened at: lost edits to them aren't checked for any run
    /// that began before it (dropped once no such run is live).
    overflow: HashMap<&'static str, u64>,
}

impl Journal {
    fn bytes(&self) -> usize {
        self.versions
            .iter()
            .map(|v| v.before.len() + v.after.len())
            .sum()
    }

    /// Drop what no live run needs: versions from before the oldest live
    /// run began, and overflow marks no live run is older than.
    fn prune(&mut self) {
        match self.runs.values().map(|(began, _)| *began).min() {
            Some(oldest) => {
                self.versions.retain(|v| v.seq > oldest);
                self.overflow.retain(|_, at| *at > oldest);
            }
            None => {
                self.versions.clear();
                self.overflow.clear();
            }
        }
    }
}

/// Add `version` to `scope`'s journal within the budgets (review 2026-10-20):
/// a version over [`MAX_DOC_BYTES`] a side, past [`MAX_VERSIONS_PER_DOC`] for
/// its doc, or past [`MAX_JOURNAL_BYTES`] / [`MAX_TOTAL_BYTES`] stops that
/// doc's journal (its versions are dropped and the log says so) until every
/// run that began before then has finished.
fn push_version(
    all: &mut HashMap<(PathBuf, String), Journal>,
    key: &(PathBuf, String),
    mut version: Version,
) {
    let elsewhere: usize = all
        .iter()
        .filter(|(k, _)| *k != key)
        .map(|(_, journal)| journal.bytes())
        .sum();
    let Some(journal) = all.get_mut(key) else {
        return;
    };
    journal.prune();
    if journal.runs.is_empty() || journal.overflow.contains_key(version.file) {
        return;
    }
    journal.seq += 1;
    version.seq = journal.seq;
    let size = version.before.len() + version.after.len();
    let of_doc = journal
        .versions
        .iter()
        .filter(|v| v.file == version.file)
        .count();
    let own = journal.bytes();
    if version.before.len() as u64 > MAX_DOC_BYTES
        || version.after.len() as u64 > MAX_DOC_BYTES
        || of_doc >= MAX_VERSIONS_PER_DOC
        || own + size > MAX_JOURNAL_BYTES
        || elsewhere + own + size > MAX_TOTAL_BYTES
    {
        let file = version.file;
        journal.versions.retain(|v| v.file != file);
        journal.overflow.insert(file, version.seq);
        eprintln!(
            "[shared-docs] {}/{file}: too much changed while runs are live; lost edits to it are not checked until they finish",
            key.1
        );
        return;
    }
    journal.versions.push(version);
}

/// Journals by (canonical vault, scope). A leaf lock: nothing else is taken
/// while it is held (file reads only).
static JOURNALS: Lazy<Mutex<HashMap<(PathBuf, String), Journal>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
/// The `sharedDoc` flags a finishing run picks up (`take_flags`).
static FLAGS: Lazy<Mutex<HashMap<String, Vec<RunFlag>>>> = Lazy::new(|| Mutex::new(HashMap::new()));

fn journals() -> std::sync::MutexGuard<'static, HashMap<(PathBuf, String), Journal>> {
    JOURNALS.lock().unwrap_or_else(|e| e.into_inner())
}

fn key(vault: &Path, scope: &str) -> (PathBuf, String) {
    let vault = vault.canonicalize().unwrap_or_else(|_| vault.to_path_buf());
    (vault, scope.trim_end_matches('/').to_string())
}

/// `scope`'s shared doc `file` as text, `None` when it is absent, too large
/// or not UTF-8.
fn read_doc(vault: &Path, scope: &str, file: &str) -> Option<String> {
    let full = vault::safe_join(vault, &format!("{scope}/{file}")).ok()?;
    if full.metadata().ok()?.len() > MAX_DOC_BYTES {
        return None;
    }
    fs::read_to_string(full).ok()
}

/// A scoped run began in `scope` (`automated_history::begin_scoped`): note
/// the shared docs it starts from.
pub(crate) fn run_began(vault: &Path, run_id: &str, scope: &str) {
    let docs: BTreeMap<&'static str, String> = SHARED_DOCS
        .iter()
        .filter_map(|file| read_doc(vault, scope, file).map(|text| (*file, text)))
        .collect();
    let mut all = journals();
    let journal = all.entry(key(vault, scope)).or_default();
    journal.seq += 1;
    let seq = journal.seq;
    journal.runs.insert(run_id.to_string(), (seq, docs));
}

/// Hollow wrote `file` in `scope` from `before` to `after`: journal it while
/// a run is live there.
fn note_hollow_write(vault: &Path, scope: &str, file: &str, before: &str, after: &str) {
    let Some(file) = SHARED_DOCS.iter().find(|f| **f == file) else {
        return;
    };
    if before == after || before.len() as u64 > MAX_DOC_BYTES || after.len() as u64 > MAX_DOC_BYTES
    {
        return;
    }
    let key = key(vault, scope);
    let mut all = journals();
    if all.get(&key).is_none_or(|journal| journal.runs.is_empty()) {
        return;
    }
    push_version(
        &mut all,
        &key,
        Version {
            seq: 0,
            file,
            before: before.to_string(),
            after: after.to_string(),
            by: BY_HOLLOW.into(),
        },
    );
}

/// [`note_hollow_write`] for a writer that has the workspace folder's path
/// (attach and refresh append to SOURCES.md).
pub(crate) fn note_folder_write(folder: &Path, file: &str, before: &str, after: &str) {
    let Some(id) = folder.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    let Some(vault) = folder.parent().and_then(Path::parent) else {
        return;
    };
    note_hollow_write(vault, &format!("workspaces/{id}"), file, before, after);
}

/// Scoped run `run_id` is finishing (the public `automated_history::
/// finish_run`, before its diff, while it is still live): merge back what a
/// write from an older copy lost during its life, flag it, then journal the
/// run's own before and after for the runs still live there.
pub(crate) fn run_finishing(vault: &Path, run_id: &str) {
    let found = {
        let all = journals();
        all.iter().find_map(|(k, journal)| {
            journal.runs.get(run_id).map(|(began, _)| {
                let versions: Vec<Version> = journal
                    .versions
                    .iter()
                    .filter(|v| v.seq > *began && v.by != run_id)
                    .cloned()
                    .collect();
                (k.clone(), versions)
            })
        })
    };
    let Some((key, versions)) = found else {
        return;
    };
    let scope = key.1.clone();
    let flags = check_versions(vault, run_id, &scope, versions);
    {
        let mut all = journals();
        let began_with = all
            .get_mut(&key)
            .and_then(|journal| journal.runs.remove(run_id))
            .map(|(_, docs)| docs);
        if let Some(began_with) = began_with {
            for file in SHARED_DOCS {
                let Some(now) = read_doc(vault, &scope, file) else {
                    continue;
                };
                let before = began_with.get(file).cloned().unwrap_or_default();
                if before != now {
                    push_version(
                        &mut all,
                        &key,
                        Version {
                            seq: 0,
                            file,
                            before,
                            after: now,
                            by: run_id.to_string(),
                        },
                    );
                }
            }
        }
        if let Some(journal) = all.get_mut(&key) {
            journal.prune();
            if journal.runs.is_empty() {
                all.remove(&key);
            }
        }
    }
    if !flags.is_empty() {
        FLAGS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(run_id.to_string(), flags);
    }
}

/// Most lost versions one finish acts on (each a merge or a copy); the
/// journal's budgets bound the rest.
const MAX_RECOVERIES: usize = 8;

/// The finishing run's checks, one flag per doc. Hollow's own writes first,
/// oldest first (each merge builds on the last); then other runs' versions,
/// newest first, skipping one whose added lines a version already kept
/// holds (a later peer that saw the earlier one's lines).
fn check_versions(
    vault: &Path,
    run_id: &str,
    scope: &str,
    mut versions: Vec<Version>,
) -> Vec<RunFlag> {
    versions.sort_by_key(|v| {
        (
            v.by != BY_HOLLOW,
            if v.by == BY_HOLLOW {
                v.seq as i64
            } else {
                -(v.seq as i64)
            },
        )
    });
    let mut flags: Vec<RunFlag> = Vec::new();
    let mut kept: Vec<(&'static str, String)> = Vec::new();
    let mut done = 0;
    for version in &versions {
        if done >= MAX_RECOVERIES {
            eprintln!("[shared-docs] {scope}: more lost versions than one finish checks; the rest are skipped");
            break;
        }
        let added = added_lines(&version.before, &version.after);
        if version.by != BY_HOLLOW
            && kept
                .iter()
                .any(|(file, text)| *file == version.file && all_present(&added, text))
        {
            continue;
        }
        let Some(flag) = recover(vault, run_id, scope, version, &added) else {
            continue;
        };
        done += 1;
        if version.by != BY_HOLLOW {
            kept.push((version.file, version.after.clone()));
        }
        match flags.iter_mut().find(|f| f.path == flag.path) {
            Some(seen) => {
                let detail = format!(
                    "{}; {}",
                    seen.detail.as_deref().unwrap_or_default(),
                    flag.detail.as_deref().unwrap_or_default()
                );
                seen.detail = Some(detail);
            }
            None => flags.push(flag),
        }
    }
    flags
}

/// Versions a scope's journal holds and their bytes (tests only).
#[cfg(test)]
fn journal_stats(vault: &Path, scope: &str) -> (usize, usize) {
    journals()
        .get(&key(vault, scope))
        .map_or((0, 0), |journal| {
            (
                journal.versions.len(),
                journal
                    .versions
                    .iter()
                    .map(|v| v.before.len() + v.after.len())
                    .sum(),
            )
        })
}

/// The `sharedDoc` flags [`run_finishing`] left for `run_id` (taken once).
pub(crate) fn take_flags(run_id: &str) -> Vec<RunFlag> {
    FLAGS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(run_id)
        .unwrap_or_default()
}

fn flag(rel: &str, detail: String) -> RunFlag {
    RunFlag {
        kind: crate::run_guard::FLAG_SHARED_DOC.into(),
        path: rel.to_string(),
        change_type: Some("modify".into()),
        detail: Some(detail),
        scan: None,
    }
}

/// If every line `version` added (`added`) is gone from its file now, act
/// on it; the flag that says what happened. Only Hollow's own writes are
/// merged back into the file (review 2026-10-20): their provenance is
/// certain and an agent is unlikely to revert a rename on purpose. Another
/// run's lost version is never written back, since this run may have
/// removed those lines on purpose: it is kept beside the file as a conflict
/// copy for the person to pick from, as is a Hollow write that won't merge.
fn recover(
    vault: &Path,
    run_id: &str,
    scope: &str,
    version: &Version,
    added: &[(String, usize)],
) -> Option<RunFlag> {
    let rel = format!("{scope}/{}", version.file);
    if added.is_empty() {
        return None;
    }
    let full = vault::safe_join(vault, &rel).ok()?;
    let now = fs::read_to_string(&full).ok()?;
    if !all_gone(added, &now) {
        return None;
    }
    let what = if version.by == BY_HOLLOW {
        "a change Hollow made meanwhile"
    } else {
        "an edit another conversation made meanwhile"
    };
    if version.by == BY_HOLLOW {
        let mut now = now;
        for _ in 0..TRIES {
            let Some(merged) = merge3(&version.before, &now, &version.after) else {
                break;
            };
            if merged == now {
                return None;
            }
            let expected = vault::revision(now.as_bytes());
            // A host write made for the finishing run (chat concurrency F1,
            // 2026-10-20): in its History, never credited to it, and left
            // out of the live peers' diffs.
            let written = crate::automated_history::host_write_for(vault, Some(run_id), || {
                let outcome = vault::write_note_expecting(
                    vault,
                    &rel,
                    &merged,
                    Expected::Revision(&expected),
                )?;
                let commits = match &outcome {
                    CasOutcome::Written(_) => vec![(
                        rel.clone(),
                        crate::automated_history::HostCommit::new(
                            Some(now.as_bytes()),
                            Some(merged.as_bytes()),
                        ),
                    )],
                    CasOutcome::Conflict(_) => Vec::new(),
                };
                Ok((outcome, commits))
            });
            match written {
                Ok(CasOutcome::Written(_)) => {
                    return Some(flag(
                        &rel,
                        format!(
                            "{what} was written over from an older copy; Hollow merged it back"
                        ),
                    ))
                }
                Ok(CasOutcome::Conflict(current)) => {
                    if !all_gone(added, &current.content) {
                        return None;
                    }
                    now = current.content;
                }
                Err(error) => {
                    eprintln!("[shared-docs] could not merge {rel} back: {error}");
                    break;
                }
            }
        }
    }
    Some(match keep_copy(vault, Some(run_id), scope, version.file, &version.after) {
        Ok(copy) => flag(
            &rel,
            format!("{what} was written over from an older copy; that version is kept as {copy}"),
        ),
        Err(error) => flag(
            &rel,
            format!("{what} was written over from an older copy, and Hollow couldn't keep it ({error})"),
        ),
    })
}

/// Write `text` beside shared doc `file` as a new conflict copy (exclusive
/// creation, a random id in the name); the copy's file name. A host write
/// made for finishing run `made_for` (chat concurrency F1, 2026-10-20): in
/// its History, never credited to it, and left out of live peers' diffs.
fn keep_copy(
    vault: &Path,
    made_for: Option<&str>,
    scope: &str,
    file: &str,
    text: &str,
) -> Result<String, String> {
    let full = vault::safe_join(vault, &format!("{scope}/{file}"))?;
    crate::automated_history::host_write_for(vault, made_for, || {
        let copy = vault::keep_conflict_copy(&full, text.as_bytes())?;
        let name = copy
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // Beside the shared doc, which sits at the folder's top.
        let commit = crate::automated_history::HostCommit::new(None, Some(text.as_bytes()));
        Ok((name.clone(), vec![(format!("{scope}/{name}"), commit)]))
    })
}

/// A line as compared: trailing whitespace (and `\r`) ignored.
fn norm(line: &str) -> &str {
    line.trim_end()
}

fn counts(text: &str) -> HashMap<&str, usize> {
    let mut out = HashMap::new();
    for line in text.lines().map(norm).filter(|l| !l.is_empty()) {
        *out.entry(line).or_insert(0) += 1;
    }
    out
}

/// The non-blank lines `after` has more of than `before`, each with how
/// many `before` had.
fn added_lines(before: &str, after: &str) -> Vec<(String, usize)> {
    let had = counts(before);
    counts(after)
        .into_iter()
        .filter_map(|(line, n)| {
            let was = had.get(line).copied().unwrap_or(0);
            (n > was).then(|| (line.to_string(), was))
        })
        .collect()
}

/// True when `text` holds every one of `added` as often as the version did.
fn all_present(added: &[(String, usize)], text: &str) -> bool {
    let have = counts(text);
    added
        .iter()
        .all(|(line, was)| have.get(line.as_str()).copied().unwrap_or(0) > *was)
}

/// True when none of `added` is in `now` beyond what was there before.
fn all_gone(added: &[(String, usize)], now: &str) -> bool {
    let now = counts(now);
    added
        .iter()
        .all(|(line, was)| now.get(line.as_str()).copied().unwrap_or(0) <= *was)
}

/// For each line of `a`, the index of the line of `b` it is matched with in
/// a longest common subsequence; `None` past [`MAX_MATCH_CELLS`].
fn matches(a: &[&str], b: &[&str]) -> Option<Vec<Option<usize>>> {
    let mut out = vec![None; a.len()];
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    for (i, slot) in out.iter_mut().enumerate().take(prefix) {
        *slot = Some(i);
    }
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    for k in 0..suffix {
        out[a.len() - 1 - k] = Some(b.len() - 1 - k);
    }
    let (am, bm) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    if am.is_empty() || bm.is_empty() {
        return Some(out);
    }
    let (n, m) = (am.len(), bm.len());
    if n.saturating_mul(m) > MAX_MATCH_CELLS {
        return None;
    }
    // lcs[i * (m + 1) + j] = LCS length of am[i..] and bm[j..].
    let w = m + 1;
    let mut lcs = vec![0u32; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i * w + j] = if am[i] == bm[j] {
                lcs[(i + 1) * w + j + 1] + 1
            } else {
                lcs[(i + 1) * w + j].max(lcs[i * w + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if am[i] == bm[j] {
            out[prefix + i] = Some(prefix + j);
            i += 1;
            j += 1;
        } else if lcs[(i + 1) * w + j] >= lcs[i * w + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    Some(out)
}

/// Line-level three-way merge (diff3): `ours` and `theirs` both descend
/// from `base`. A region only one side changed takes that side; one both
/// changed the same way takes it once; one where both only added lines at
/// the same place keeps both, ours first (two "Current context" entries),
/// with a line break between them even when ours ended the file without one.
/// Lines compare exactly, so a side that rewrote the line endings conflicts.
/// Anything else is a conflict: `None`, and Hollow doesn't guess.
pub(crate) fn merge3(base: &str, ours: &str, theirs: &str) -> Option<String> {
    let b: Vec<&str> = base.split_inclusive('\n').collect();
    let o: Vec<&str> = ours.split_inclusive('\n').collect();
    let t: Vec<&str> = theirs.split_inclusive('\n').collect();
    let mo = matches(&b, &o)?;
    let mt = matches(&b, &t)?;
    let (mut ib, mut io, mut it) = (0, 0, 0);
    let mut out = String::with_capacity(ours.len().max(theirs.len()));
    loop {
        // Lines all three keep, in step.
        while ib < b.len() && mo[ib] == Some(io) && mt[ib] == Some(it) {
            out.push_str(b[ib]);
            ib += 1;
            io += 1;
            it += 1;
        }
        // The next base line both sides kept closes the region between.
        let next = (ib..b.len()).find_map(|k| Some((k, mo[k]?, mt[k]?)));
        let (kb, ko, kt) = next.unwrap_or((b.len(), o.len(), t.len()));
        let (cb, co, ct) = (&b[ib..kb], &o[io..ko], &t[it..kt]);
        let take: Vec<&str> = if co == cb {
            ct.to_vec()
        } else if ct == cb || co == ct {
            co.to_vec()
        } else if cb.is_empty() {
            // Both only added here. Ours may end the file without a line
            // break: give it the file's own (CRLF or LF) so the two last
            // lines are never glued together (review 2026-10-20).
            let mut both: Vec<&str> = co.to_vec();
            if co.last().is_some_and(|line| !line.ends_with('\n')) {
                both.push(if o.iter().any(|line| line.ends_with("\r\n")) {
                    "\r\n"
                } else {
                    "\n"
                });
            }
            both.extend_from_slice(ct);
            both
        } else {
            return None;
        };
        for line in take {
            out.push_str(line);
        }
        (ib, io, it) = (kb, ko, kt);
        if next.is_none() {
            return Some(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::chat_log_path;

    #[test]
    fn a_chat_log_path_is_the_key_under_log_and_never_escapes() {
        assert_eq!(
            chat_log_path("s-20261115-v5b00001").as_deref(),
            Some("log/s-20261115-v5b00001.md")
        );
        for bad in [
            "",
            "..",
            "../x",
            "s-../x",
            "s-a/b",
            "/etc/passwd",
            "S-ABC",
            "x-1",
            "s-a.md",
        ] {
            assert_eq!(chat_log_path(bad), None, "{bad}");
        }
    }

    use crate::automated_history::try_begin_scoped_shared;
    use std::fs;
    use std::path::Path;

    const BRIEF: &str = "---\nhollow_workspace: 1\ntitle: \"W\"\nstatus: active\n---\n\n# W\n\n## Current context\n\n- one\n\n## Next step\n\nShip.\n";
    const DOC: &str = "workspaces/w1/WORKSPACE.md";

    fn workspace(v: &Path) {
        fs::create_dir_all(v.join("workspaces/w1")).unwrap();
        fs::write(v.join(DOC), BRIEF).unwrap();
    }

    fn shared_doc_flags(run: &crate::automated_history::HistoryRun) -> Vec<String> {
        run.flags
            .iter()
            .filter(|f| f.kind == "sharedDoc")
            .map(|f| f.path.clone())
            .collect()
    }

    /// Conflict copies beside the shared doc, by name.
    fn copies(v: &Path) -> Vec<String> {
        let mut out: Vec<String> = fs::read_dir(v.join("workspaces/w1"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("WORKSPACE (conflicted copy "))
            .collect();
        out.sort();
        out
    }

    /// The lost update (2026-10-20): a peer adds a line and finishes, then
    /// the other run writes the brief back from the copy it read at its
    /// start. Another run's lines are never written back into the file (the
    /// run may have removed them on purpose, review 2026-10-20): the lost
    /// version is kept as a conflict copy, inside this run's History, and
    /// the run is flagged.
    #[test]
    fn a_peer_line_written_over_from_an_older_copy_is_kept_as_a_copy() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        workspace(v);
        let file = v.join(DOC);
        let mine = try_begin_scoped_shared(v, "workspaces/w1", "chat", Some("s-mine")).unwrap();
        let stale = fs::read_to_string(&file).unwrap();
        let peer = try_begin_scoped_shared(v, "workspaces/w1", "chat", Some("s-peer")).unwrap();
        let theirs = stale.replace("- one\n", "- one\n- two (peer)\n");
        fs::write(&file, &theirs).unwrap();
        peer.finish("success").unwrap();
        let ours = stale.replace("Ship.", "Ship it today.");
        fs::write(&file, &ours).unwrap();
        let run = mine.finish("success").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), ours, "never rewritten");
        let copies = copies(v);
        assert_eq!(copies.len(), 1, "{copies:?}");
        let copy = format!("workspaces/w1/{}", copies[0]);
        assert_eq!(fs::read_to_string(v.join(&copy)).unwrap(), theirs);
        assert!(run.changes.iter().any(|c| c.vault_path == copy));
        assert_eq!(shared_doc_flags(&run), vec![DOC.to_string()]);
    }

    /// Concurrency PR 1 with PR 2 (red first): the conflict copy Hollow
    /// makes at a run's finish stays in that run's History but isn't
    /// credited to it, and a conversation still live in the folder doesn't
    /// get that copy in its changes (a host write); its own finish keeps a
    /// copy of its own, also uncredited.
    #[test]
    fn a_conflict_copy_is_nobodys_change_but_stays_in_the_finishing_runs_history() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        workspace(v);
        let file = v.join(DOC);
        let mine = try_begin_scoped_shared(v, "workspaces/w1", "chat", Some("s-mine")).unwrap();
        let bystander =
            try_begin_scoped_shared(v, "workspaces/w1", "chat", Some("s-bystander")).unwrap();
        let stale = fs::read_to_string(&file).unwrap();
        let peer = try_begin_scoped_shared(v, "workspaces/w1", "chat", Some("s-peer")).unwrap();
        fs::write(&file, stale.replace("- one\n", "- one\n- two (peer)\n")).unwrap();
        peer.finish("success").unwrap();
        fs::write(&file, stale.replace("Ship.", "Ship it today.")).unwrap();
        let mine_id = mine.id().to_string();
        crate::automated_history::note_own_writes(v, &mine_id, [DOC.to_string()].into());
        let run = mine.finish("success").unwrap();
        let copies = copies(v);
        assert_eq!(copies.len(), 1, "{copies:?}");
        let copy = format!("workspaces/w1/{}", copies[0]);
        let kept = run.changes.iter().find(|c| c.vault_path == copy).unwrap();
        assert!(kept.unattributed, "Hollow's copy, not the agent's work");
        let doc = run.changes.iter().find(|c| c.vault_path == DOC).unwrap();
        assert!(!doc.unattributed, "its own write");
        // The bystander lost the peer's line too, so its own finish keeps
        // a copy of its own; the first run's copy is not in its changes.
        let other = bystander.finish("success").unwrap();
        assert!(other.changes.iter().all(|c| c.vault_path != copy));
        for change in other.changes.iter().filter(|c| c.vault_path != DOC) {
            assert!(change.unattributed, "{}", change.vault_path);
        }
    }

    /// Two peers lost to one stale write, the later having seen the
    /// earlier's line: one copy (the later version) covers both.
    #[test]
    fn a_later_lost_version_that_covers_an_earlier_one_makes_one_copy() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        workspace(v);
        let file = v.join(DOC);
        let mine = try_begin_scoped_shared(v, "workspaces/w1", "chat", Some("s-mine")).unwrap();
        let stale = fs::read_to_string(&file).unwrap();
        let first = try_begin_scoped_shared(v, "workspaces/w1", "chat", Some("s-p1")).unwrap();
        let one = stale.replace("- one\n", "- one\n- two\n");
        fs::write(&file, &one).unwrap();
        first.finish("success").unwrap();
        let second = try_begin_scoped_shared(v, "workspaces/w1", "chat", Some("s-p2")).unwrap();
        let two = one.replace("- two\n", "- two\n- three\n");
        fs::write(&file, &two).unwrap();
        second.finish("success").unwrap();
        fs::write(&file, &stale).unwrap();
        mine.finish("success").unwrap();
        let copies = copies(v);
        assert_eq!(copies.len(), 1, "{copies:?}");
        assert_eq!(
            fs::read_to_string(v.join("workspaces/w1").join(&copies[0])).unwrap(),
            two
        );
    }

    /// Conflict copy names stay free however many are made in one second.
    #[test]
    fn many_copies_in_one_second_all_get_a_name() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        workspace(v);
        for n in 0..25 {
            super::keep_copy(v, None, "workspaces/w1", "WORKSPACE.md", &format!("v{n}\n")).unwrap();
        }
        assert_eq!(copies(v).len(), 25);
    }

    /// The same line changed both ways: Hollow doesn't guess. The run's
    /// version stays, the lost one is kept beside it as a conflict copy
    /// (inside this run's History), and the run is flagged.
    #[test]
    fn a_written_over_change_that_cannot_merge_is_kept_as_a_copy_and_flagged() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        workspace(v);
        let file = v.join(DOC);
        let mine = try_begin_scoped_shared(v, "workspaces/w1", "chat", Some("s-mine")).unwrap();
        let stale = fs::read_to_string(&file).unwrap();
        let peer = try_begin_scoped_shared(v, "workspaces/w1", "chat", Some("s-peer")).unwrap();
        let theirs = stale.replace("- one\n", "- one (peer)\n");
        fs::write(&file, &theirs).unwrap();
        peer.finish("success").unwrap();
        let ours = stale.replace("- one\n", "- one (mine)\n");
        fs::write(&file, &ours).unwrap();
        let run = mine.finish("success").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), ours);
        let copies: Vec<String> = fs::read_dir(v.join("workspaces/w1"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("WORKSPACE (conflicted copy "))
            .collect();
        assert_eq!(copies.len(), 1, "{copies:?}");
        let copy = format!("workspaces/w1/{}", copies[0]);
        assert_eq!(fs::read_to_string(v.join(&copy)).unwrap(), theirs);
        assert_eq!(
            crate::conflict_copies::original_name(&copies[0]).as_deref(),
            Some("WORKSPACE.md")
        );
        assert!(run.changes.iter().any(|c| c.vault_path == copy));
        assert_eq!(shared_doc_flags(&run), vec![DOC.to_string()]);
    }

    /// A run that saw the peer's lines and edited one of them made a choice:
    /// nothing is merged and nothing flagged.
    #[test]
    fn an_edit_of_a_peer_line_the_run_saw_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        workspace(v);
        let file = v.join(DOC);
        let mine = try_begin_scoped_shared(v, "workspaces/w1", "chat", Some("s-mine")).unwrap();
        let peer = try_begin_scoped_shared(v, "workspaces/w1", "chat", Some("s-peer")).unwrap();
        let theirs = BRIEF.replace("- one\n", "- one\n- [ ] a\n- [ ] b\n");
        fs::write(&file, &theirs).unwrap();
        peer.finish("success").unwrap();
        let ours = theirs.replace("- [ ] a", "- [x] a");
        fs::write(&file, &ours).unwrap();
        let run = mine.finish("success").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), ours);
        assert!(shared_doc_flags(&run).is_empty(), "{:?}", run.flags);
    }

    /// Hollow's brief writes are compare-and-swap: a change landing between
    /// the read and the write makes it read again, never overwrite.
    #[test]
    fn a_change_between_read_and_write_makes_a_rename_retry() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let w = crate::workspaces::create(v, "Launch plan", "Ship it").unwrap();
        let file = v.join(format!("workspaces/{}/WORKSPACE.md", w.id));
        let chat = try_begin_scoped_shared(v, &format!("workspaces/{}", w.id), "chat", Some("s-c"))
            .unwrap();
        let racer = file.clone();
        let mut fired = 0;
        super::BEFORE_WRITE.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fired += 1;
                if fired == 1 {
                    let text = fs::read_to_string(&racer).unwrap();
                    fs::write(&racer, format!("{text}\nAn agent's line.\n")).unwrap();
                }
            }))
        });
        let renamed = crate::workspaces::update_meta(v, &w.id, Some("Renamed"), None);
        super::BEFORE_WRITE.with(|hook| *hook.borrow_mut() = None);
        assert_eq!(renamed.unwrap().title, "Renamed");
        let now = fs::read_to_string(&file).unwrap();
        assert!(now.contains("An agent's line."), "{now}");
        assert!(now.contains("title: Renamed\n"), "{now}");
        chat.finish("success").unwrap();
    }

    #[test]
    fn canvas_plan_confirm_keeps_a_concurrent_edit_and_requires_a_new_preview() {
        let dir = tempfile::tempdir().unwrap();
        let app = tempfile::tempdir().unwrap();
        let v = dir.path();
        let w = crate::workspaces::create(v, "Canvas import", "Keep edits").unwrap();
        crate::workspace_canvases::record(
            v,
            app.path(),
            crate::workspace_canvases::Scope {
                workspace_id: Some(&w.id),
                session_key: None,
            },
            "<hc-canvas id=\"rollout\" title=\"Rollout\"><ul><li>First</li></ul></hc-canvas>",
            None,
            None,
            None,
            "2026-10-25T23:00:00Z",
        )
        .unwrap();
        let mut request = crate::canvas_items::CanvasPlanItems {
            workspace_id: w.id.clone(),
            id: "rollout".into(),
            revision: 1,
            heading: "Checks".into(),
            rows: vec![(0, "First".into())],
            expected_preview: None,
        };
        let preview = crate::canvas_items::plan(v, app.path(), &request).unwrap();
        request.expected_preview = Some(preview);
        let path = v.join(format!("workspaces/{}/PLAN.md", w.id));
        let racer = path.clone();
        let mut fired = false;
        super::BEFORE_WRITE.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                if !fired {
                    fired = true;
                    let old = fs::read_to_string(&racer).unwrap();
                    fs::write(&racer, format!("{old}\nConcurrent edit stays.\n")).unwrap();
                }
            }))
        });
        let outcome = crate::canvas_items::plan(v, app.path(), &request);
        super::BEFORE_WRITE.with(|hook| *hook.borrow_mut() = None);
        assert!(outcome.unwrap_err().contains("Plan changed"));
        let kept = fs::read_to_string(&path).unwrap();
        assert!(kept.contains("Concurrent edit stays."));
        assert!(!kept.contains("canvas rollout"));
        request.expected_preview = None;
        request.expected_preview =
            Some(crate::canvas_items::plan(v, app.path(), &request).unwrap());
        let saved = crate::canvas_items::plan(v, app.path(), &request).unwrap();
        assert!(saved.contains("Concurrent edit stays."));
        assert_eq!(saved.matches("canvas rollout v1").count(), 1);
        assert_eq!(
            crate::canvas_items::plan(v, app.path(), &request).unwrap(),
            saved
        );
    }

    /// A file that keeps changing under the write is refused, untouched.
    #[test]
    fn a_rename_that_keeps_conflicting_is_refused_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let w = crate::workspaces::create(v, "Launch plan", "Ship it").unwrap();
        let file = v.join(format!("workspaces/{}/WORKSPACE.md", w.id));
        let racer = file.clone();
        let mut n = 0;
        super::BEFORE_WRITE.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                n += 1;
                let text = fs::read_to_string(&racer).unwrap();
                fs::write(&racer, format!("{text}\nEdit {n}.\n")).unwrap();
            }))
        });
        let renamed = crate::workspaces::update_meta(v, &w.id, Some("Renamed"), None);
        super::BEFORE_WRITE.with(|hook| *hook.borrow_mut() = None);
        let error = renamed.unwrap_err();
        assert!(error.contains("WORKSPACE.md kept changing"), "{error}");
        let now = fs::read_to_string(&file).unwrap();
        assert!(!now.contains("Renamed"), "{now}");
        assert!(now.contains("Edit 4."), "{now}");
    }

    /// A shared-doc flag alone isn't called a protected settings change.
    #[test]
    fn a_shared_doc_flag_alone_is_worded_as_a_written_over_file() {
        let flag = super::flag("workspaces/w1/PLAN.md", "merged back".into());
        let note = crate::run_guard::flag_notification("chat", None, &[flag]).unwrap();
        assert_eq!(note.title, "A shared workspace file was written over");
        assert_eq!(note.body, "chat: workspaces/w1/PLAN.md: merged back");
    }

    #[test]
    fn merge3_takes_each_side_and_refuses_a_real_conflict() {
        let base = "a\nb\nc\n";
        assert_eq!(
            super::merge3(base, "a\nb2\nc\n", "a\nb\nc\nd\n").as_deref(),
            Some("a\nb2\nc\nd\n")
        );
        assert_eq!(
            super::merge3(base, "a\nc\n", "a\nb\nc\n").as_deref(),
            Some("a\nc\n"),
            "a deletion on one side"
        );
        assert_eq!(
            super::merge3(base, "a\nx\nc\n", "a\nx\nc\n").as_deref(),
            Some("a\nx\nc\n"),
            "the same change on both"
        );
        assert_eq!(super::merge3(base, "a\nx\nc\n", "a\ny\nc\n"), None);
        assert_eq!(
            super::merge3("", "mine\n", "theirs\n").as_deref(),
            Some("mine\ntheirs\n")
        );
    }

    /// An agent writing the brief after Hollow's check and before its
    /// replace (the window no lock covers): the rename reads again and keeps
    /// the agent's line (review 2026-10-20).
    #[test]
    fn an_agent_write_inside_the_replace_window_makes_a_rename_retry() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        if !crate::vault::swap_supported_for_test(v) {
            return;
        }
        let w = crate::workspaces::create(v, "Launch plan", "Ship it").unwrap();
        let file = v.join(format!("workspaces/{}/WORKSPACE.md", w.id));
        let racer = file.clone();
        let mut fired = false;
        crate::vault::set_replace_hook_for_test(Some(Box::new(move |point| {
            if point == "before-replace" && !fired {
                fired = true;
                let text = fs::read_to_string(&racer).unwrap();
                fs::write(&racer, format!("{text}\nAn agent's line.\n")).unwrap();
            }
        })));
        let renamed = crate::workspaces::update_meta(v, &w.id, Some("Renamed"), None);
        crate::vault::set_replace_hook_for_test(None);
        assert_eq!(renamed.unwrap().title, "Renamed");
        let now = fs::read_to_string(&file).unwrap();
        assert!(now.contains("An agent's line."), "{now}");
        assert!(now.contains("title: Renamed\n"), "{now}");
    }

    /// The journal stays bounded however long a chat runs: past the per-doc
    /// count Hollow stops journaling that doc (and says so in the log), and
    /// a doc over the size limit is never journaled.
    #[test]
    fn the_journal_is_bounded_for_a_long_running_chat() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let w = crate::workspaces::create(v, "Launch plan", "Ship it").unwrap();
        let scope = format!("workspaces/{}", w.id);
        let chat = try_begin_scoped_shared(v, &scope, "chat", Some("s-long")).unwrap();
        for n in 0..(super::MAX_VERSIONS_PER_DOC + 20) {
            crate::workspaces::update_meta(v, &w.id, Some(&format!("Title {n}")), None).unwrap();
        }
        let (versions, bytes) = super::journal_stats(v, &scope);
        assert!(versions <= super::MAX_VERSIONS_PER_DOC, "{versions}");
        assert!(bytes <= super::MAX_JOURNAL_BYTES, "{bytes}");
        chat.finish("success").unwrap();
        // A huge doc written by Hollow is not journaled.
        let other = crate::workspaces::create(v, "Big one", "Ship it").unwrap();
        let big_scope = format!("workspaces/{}", other.id);
        let brief = v.join(format!("{big_scope}/WORKSPACE.md"));
        let text = fs::read_to_string(&brief).unwrap();
        fs::write(&brief, format!("{text}\n{}\n", "x".repeat(300 * 1024))).unwrap();
        let chat = try_begin_scoped_shared(v, &big_scope, "chat", Some("s-big")).unwrap();
        crate::workspaces::update_meta(v, &other.id, Some("Bigger"), None).unwrap();
        assert_eq!(super::journal_stats(v, &big_scope), (0, 0));
        chat.finish("success").unwrap();
    }

    #[test]
    fn merge3_never_glues_two_unterminated_last_lines() {
        assert_eq!(
            super::merge3("a\n", "a\nmine", "a\npeer").as_deref(),
            Some("a\nmine\npeer")
        );
        assert_eq!(
            super::merge3("a\r\n", "a\r\nmine", "a\r\npeer").as_deref(),
            Some("a\r\nmine\r\npeer"),
            "a CRLF file keeps CRLF"
        );
        assert_eq!(
            super::merge3(
                "a\r\nb\r\nc\r\n",
                "a\r\nb\r\nc\r\nmine\r\n",
                "a2\r\nb\r\nc\r\n"
            )
            .as_deref(),
            Some("a2\r\nb\r\nc\r\nmine\r\n")
        );
        assert_eq!(
            super::merge3("a\r\n", "a\nmine\n", "a\r\npeer\r\n"),
            None,
            "one side rewrote the line endings: a conflict, not a guess"
        );
    }

    /// Hollow's own rename lands while a chat runs; the chat then writes
    /// the brief back from its earlier copy. The rename is merged back.
    #[test]
    fn a_hollow_rename_written_over_by_a_chat_is_merged_back() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let w = crate::workspaces::create(v, "Launch plan", "Ship it").unwrap();
        let rel = format!("workspaces/{}/WORKSPACE.md", w.id);
        let file = v.join(&rel);
        let scope = format!("workspaces/{}", w.id);
        let chat = try_begin_scoped_shared(v, &scope, "chat", Some("s-chat")).unwrap();
        let stale = fs::read_to_string(&file).unwrap();
        crate::workspaces::update_meta(v, &w.id, Some("Launch: phase 2"), None).unwrap();
        fs::write(&file, format!("{stale}\nA line the chat added.\n")).unwrap();
        let run = chat.finish("success").unwrap();
        let now = fs::read_to_string(&file).unwrap();
        assert!(now.contains("title: \"Launch: phase 2\""), "{now}");
        assert!(now.contains("A line the chat added."), "{now}");
        assert_eq!(shared_doc_flags(&run), vec![rel]);
    }
}
