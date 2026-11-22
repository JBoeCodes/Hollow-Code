//! "Delete the memory X?" offers (build run L2, 2026-11-15).
//!
//! A Hollow lesson is a Linear issue in team HOL labelled `hollow-lesson`
//! whose Agent notes name a workaround memory ("Workaround memory:
//! `<file>.md` in <folder>"); Alex decided (11-12 8:02 PM) that the file
//! and its line in `MEMORY.md` go once the fix is installed. This module
//! offers that delete in Waiting on you (kind [`KIND`]) and never deletes
//! without the person's answer.
//!
//! **Fix looks installed.** The issue is completed (state kind `completed`)
//! and its `completedAt` is at least [`MERGE_LAG_MINUTES`] before the
//! installed app's build time (`self_install::installed_built_at`, the
//! provenance `generatedAt`). An issue marked Done before its fix merged can
//! still be offered early; the offer says "looks installed" and Delete is
//! reversible. Issues come from the tracker mirror's local ledger only
//! (`tracker_sync::mirrored_issues`), so there is no Linear call here and
//! team HOL is checked by its id.
//!
//! **Which file.** Issue text is not trusted to name a file on its own. The
//! name after "Workaround memory:" must also be vouched for: either it is the
//! pair [`KNOWN`] holds for that issue (the L1 lesson map, 2026-11-15), or
//! the file's body or its `MEMORY.md` line names the issue key. It is looked
//! up in Claude Code's auto-memory folder of each workspace
//! (`<config>/projects/<slug>/memory/`, the slug of the folder's repository
//! root, as `run_manifest` computes it); the folder an issue names is
//! ignored (the vault moved). Only a plain file with a plain name
//! (`[A-Za-z0-9._-]+.md`, never `MEMORY.md`, no link on the way) is offered.
//!
//! **Order.** PER-1, PER-13, PER-12 and PER-9 first, then by issue number;
//! at most [`OFFER_CAP`] wait at once, so the next comes as one is answered.
//!
//! **Answers.** Delete copies the file into the vault's `.trash/`, records
//! the answer, takes the file's one line out of that folder's `MEMORY.md`
//! (refused when two lines name it, and abandoned with nothing changed when
//! the index changed since it was read), removes the file and adds a History
//! row; any failing step undoes the ones before it. [`restore`] (the
//! snackbar's Undo) puts the file back and its line at its old place when the
//! index is as the delete left it, else at the end. Keep records the choice.
//! Either way the offer is never repeated for that memory. Answers live in
//! app support (`LessonMemories/<vault stem>.json`), outside the vault,
//! beside the conflict-copy marks. The kind stays in Hollow: it is not one of
//! T3b's `MIRRORED_KINDS`, so it is never copied to Linear.

use crate::tracker::{StateKind, TrackerIssue};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The Waiting on you kind.
pub(crate) const KIND: &str = "lesson_memory";
/// The label every lesson issue carries.
const LABEL: &str = "hollow-lesson";
/// Offered before any other, in this order (row L2).
const FIRST: [&str; 4] = ["PER-1", "PER-13", "PER-12", "PER-9"];
/// Most offers waiting at once.
pub(crate) const OFFER_CAP: usize = 4;
/// How long before the build an issue must be Done: a margin for the merge
/// that follows closing it.
pub(crate) const MERGE_LAG_MINUTES: i64 = 15;
/// History's label for the rows a delete or restore adds.
const HISTORY_DELETED: &str = "Lesson memory deleted";
const HISTORY_RESTORED: &str = "Lesson memory restored";
/// The memory each lesson issue names, as the L1 lesson map paired them
/// (outputs/hollow-build-run-2026-11-14/l1-lesson-map.md).
const KNOWN: [(&str, &str); 16] = [
    ("PER-1", "canvas-no-raw-angle-brackets.md"),
    ("PER-2", "install-from-main-turn.md"),
    ("PER-3", "apfs-clones-overstate-cleanup.md"),
    ("PER-4", "checkin-card-answers-keep-accept-edits.md"),
    ("PER-5", "running-chat-takes-no-messages.md"),
    ("PER-6", "read-session-query-matches-date-token.md"),
    ("PER-7", "read-session-long-answer-overflows.md"),
    ("PER-8", "handover-self-wake.md"),
    ("PER-9", "stop-deletes-run-scratch.md"),
    ("PER-10", "parallel-main-wake.md"),
    ("PER-11", "handed-over-chat-still-takes-messages.md"),
    ("PER-12", "wake-me-dropped-mid-turn.md"),
    ("PER-13", "prose-before-tools-gets-folded.md"),
    ("PER-14", "link-paths-vault-relative.md"),
    ("PER-15", "pause-means-stop-at-safe-points.md"),
    ("PER-16", "hollow-current-code-path.md"),
];

static LOCK: Mutex<()> = Mutex::new(());

/// One delete waiting on the person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Offer {
    /// `<issue key>:<file name>`, the id the answer names.
    pub source_id: String,
    pub issue: String,
    pub memory: String,
    pub path: PathBuf,
    pub title: String,
    pub detail: String,
    pub at: String,
    pub workspace_id: Option<String>,
}

/// The person's answer for one memory.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Decision {
    issue: String,
    memory: String,
    /// `deleted` | `kept` | `restored`.
    choice: String,
    at: String,
    /// Where the deleted file's copy is, absolute.
    #[serde(default)]
    folder: Option<String>,
    #[serde(default)]
    trash: Option<String>,
    #[serde(default)]
    hash: Option<String>,
    /// The `MEMORY.md` line taken out and its 0-based position.
    #[serde(default)]
    line: Option<String>,
    #[serde(default)]
    line_at: Option<usize>,
    /// The hash of `MEMORY.md` as the delete left it: Undo puts the line
    /// back at `line_at` only while the index is still that.
    #[serde(default)]
    index_after: Option<String>,
}

/// A lesson issue whose fix looks installed, with its memory's file name.
struct Lesson<'a> {
    issue: &'a TrackerIssue,
    memory: String,
}

/// The file name after "Workaround memory:" in a description, when it is a
/// plain memory file name.
pub(crate) fn memory_named(description: &str) -> Option<String> {
    let at = description.find("Workaround memory:")?;
    let rest = description[at + "Workaround memory:".len()..].trim_start();
    let word = rest.split_whitespace().next()?;
    let name = word.trim_matches(|c| c == '`' || c == '*' || c == '"' || c == '\'');
    let name = name.trim_end_matches([',', '.', ';', ':']);
    plain_name(name).then(|| name.to_string())
}

fn plain_name(name: &str) -> bool {
    name.len() > 3
        && name.len() <= 120
        && name.ends_with(".md")
        && !name.eq_ignore_ascii_case("MEMORY.md")
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Whether `issue`'s fix looks installed in a build made at `built_at`:
/// completed, at least [`MERGE_LAG_MINUTES`] before the build.
pub(crate) fn fix_in_build(issue: &TrackerIssue, built_at: &str) -> bool {
    let parse = |t: &str| chrono::DateTime::parse_from_rfc3339(t).ok();
    let (Some(done), Some(built)) = (
        issue.completed_at.as_deref().and_then(parse),
        parse(built_at),
    ) else {
        return false;
    };
    issue.state.kind == StateKind::Completed
        && done + chrono::Duration::minutes(MERGE_LAG_MINUTES) <= built
}

/// Whether `text` names `key` as a whole key: `PER-1` is not in `PER-13`.
fn names_key(text: &str, key: &str) -> bool {
    text.match_indices(key).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + key.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-')
            && !after.is_some_and(|c| c.is_ascii_alphanumeric())
    })
}

fn known_pair(key: &str, memory: &str) -> bool {
    KNOWN.iter().any(|(k, m)| *k == key && *m == memory)
}

fn issue_number(key: &str) -> u64 {
    key.rsplit('-')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(u64::MAX)
}

/// Lesson issues of team HOL whose fix looks installed, in offer order.
fn lessons<'a>(issues: &'a [TrackerIssue], built_at: &str) -> Vec<Lesson<'a>> {
    let mut out: Vec<Lesson> = issues
        .iter()
        .filter(|i| i.team_id == crate::tracker::linear::PERSONAL_TEAM_ID)
        .filter(|i| i.labels.iter().any(|l| l == LABEL))
        .filter(|i| i.archived_at.is_none() && !i.trashed)
        .filter(|i| fix_in_build(i, built_at))
        .filter_map(|issue| memory_named(&issue.description).map(|memory| Lesson { issue, memory }))
        .collect();
    let rank = |key: &str| {
        FIRST
            .iter()
            .position(|k| *k == key)
            .map_or((1, issue_number(key)), |p| (0, p as u64))
    };
    out.sort_by_key(|l| rank(&l.issue.key));
    out
}

/// Claude Code's config folder: `CLAUDE_CONFIG_DIR`, else `~/.claude`.
fn claude_dir() -> Option<PathBuf> {
    std::env::var_os(crate::usage::claude_env::CONFIG_DIR_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".claude")))
}

/// Each workspace's auto-memory folder under `claude`, with its id.
fn memory_folders(root: &Path, claude: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    for (w, folder) in crate::workspaces::list_with_dirs(root).unwrap_or_default() {
        let repo = crate::run_manifest::claude_memory_root(&folder);
        let dir = claude
            .join("projects")
            .join(crate::run_manifest::claude_project_slug(&repo))
            .join("memory");
        if !out.iter().any(|(_, d)| *d == dir) {
            out.push((w.id, dir));
        }
    }
    out
}

/// A plain file (no link anywhere below `claude`) at `folder/name`.
fn plain_file(claude: &Path, folder: &Path, name: &str) -> Option<PathBuf> {
    let path = folder.join(name);
    let rel = path.strip_prefix(claude).ok()?;
    let mut at = claude.to_path_buf();
    for part in rel.components() {
        if !matches!(part, std::path::Component::Normal(_)) {
            return None;
        }
        at.push(part);
        if at.symlink_metadata().ok()?.file_type().is_symlink() {
            return None;
        }
    }
    path.symlink_metadata()
        .ok()
        .filter(|m| m.is_file())
        .map(|_| path)
}

/// Whether the memory at `path` belongs to issue `key`: the known pair, or
/// the file's body or its `MEMORY.md` line names the key.
fn vouched(path: &Path, key: &str, memory: &str) -> bool {
    if known_pair(key, memory) {
        return true;
    }
    let body = std::fs::read_to_string(path).unwrap_or_default();
    if names_key(&body, key) {
        return true;
    }
    let index = path
        .parent()
        .map(|f| std::fs::read_to_string(f.join("MEMORY.md")).unwrap_or_default())
        .unwrap_or_default();
    matches!(index_line(&index, memory), Ok(Some((_, line))) if names_key(&line, key))
}

fn source_id(issue: &str, memory: &str) -> String {
    format!("{issue}:{memory}")
}

fn offers_in(
    root: &Path,
    claude: &Path,
    built_at: &str,
    issues: &[TrackerIssue],
    decided: &[Decision],
) -> Vec<Offer> {
    let folders = memory_folders(root, claude);
    let mut out = Vec::new();
    for lesson in lessons(issues, built_at) {
        if out.len() >= OFFER_CAP {
            break;
        }
        let key = &lesson.issue.key;
        if decided
            .iter()
            .any(|d| d.issue == *key && d.memory == lesson.memory)
        {
            continue;
        }
        let Some((workspace, path)) = folders.iter().find_map(|(id, dir)| {
            plain_file(claude, dir, &lesson.memory)
                .filter(|p| vouched(p, key, &lesson.memory))
                .map(|p| (id.clone(), p))
        }) else {
            continue;
        };
        let stem = lesson.memory.trim_end_matches(".md");
        out.push(Offer {
            source_id: source_id(key, &lesson.memory),
            issue: key.clone(),
            memory: lesson.memory.clone(),
            path,
            title: format!("Delete the memory {stem}?"),
            detail: format!(
                "The fix looks installed ({key} was Done before this build): {}. Delete removes {} and its line in MEMORY.md, and Undo puts both back; Keep leaves it.",
                lesson.issue.title, lesson.memory
            ),
            at: lesson.issue.completed_at.clone().unwrap_or_default(),
            workspace_id: Some(workspace),
        });
    }
    out
}

fn store_path(root: &Path) -> Option<PathBuf> {
    let stem = crate::session_store::vault_file_stem(root);
    #[cfg(test)]
    {
        Some(
            crate::test_temp::process_dir("hollow-test-lesson-memories-")
                .join(format!("{stem}.json")),
        )
    }
    #[cfg(not(test))]
    {
        crate::instance::support_dir()
            .map(|r| r.join("LessonMemories").join(format!("{stem}.json")))
    }
}

/// Test-only fault points: the next save fails, or `MEMORY.md` is edited
/// between its read and its write.
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Fault {
    SaveFails,
    IndexEdited,
}

#[cfg(test)]
thread_local! {
    pub(super) static FAULT: std::cell::Cell<Option<Fault>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn fault(which: Fault) -> bool {
    FAULT.with(|f| {
        let hit = f.get() == Some(which);
        if hit {
            f.set(None);
        }
        hit
    })
}

fn load(root: &Path) -> Result<Vec<Decision>, String> {
    let Some(p) = store_path(root) else {
        return Err("No app data folder.".into());
    };
    match std::fs::read(&p) {
        Ok(raw) => serde_json::from_slice(&raw)
            .map_err(|_| "The lesson memory answers are unreadable.".to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(_) => Err("The lesson memory answers could not be read.".into()),
    }
}

fn save(root: &Path, all: &[Decision]) -> Result<(), String> {
    #[cfg(test)]
    if fault(Fault::SaveFails) {
        return Err("save failed (test)".into());
    }
    let p = store_path(root).ok_or("No app data folder.")?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_vec_pretty(all).map_err(|e| e.to_string())?;
    crate::vault::atomic_write_private(&p, &text)
}

/// The offers waiting now. Errors when the mirror or the answers can't be
/// read, so the list names the source unavailable instead of reading empty.
pub(crate) fn waiting(root: &Path) -> Result<Vec<Offer>, String> {
    let claude = claude_dir().ok_or("No home folder.")?;
    let issues = crate::tracker_sync::mirrored_issues(root)?;
    let built_at = crate::self_install::installed_built_at();
    let decided = load(root)?;
    Ok(offers_in(root, &claude, &built_at, &issues, &decided))
}

/// The person's answer to offer `source_id`: Delete (`delete`) or Keep.
/// Returns the memory's file name.
pub(crate) fn answer(root: &Path, source_id: &str, delete: bool) -> Result<String, String> {
    let claude = claude_dir().ok_or("No home folder.")?;
    let issues = crate::tracker_sync::mirrored_issues(root)?;
    let built_at = crate::self_install::installed_built_at();
    answer_in(root, &claude, &built_at, &issues, source_id, delete)
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Whether one index line links to `name`: `](name)`, `](./name)`,
/// `](<name>)` or any of them with a title (`](name "title")`).
fn links_to(line: &str, name: &str) -> bool {
    line.match_indices("](").any(|(at, _)| {
        let rest = &line[at + 2..];
        let target = rest.split(')').next().unwrap_or("").trim();
        let target = target.split_whitespace().next().unwrap_or("");
        let target = target.trim_start_matches('<').trim_end_matches('>');
        target.strip_prefix("./").unwrap_or(target) == name
    })
}

/// The `MEMORY.md` line that links to `name`, with its index; `Ok(None)`
/// with none, an error with more than one.
fn index_line(text: &str, name: &str) -> Result<Option<(usize, String)>, String> {
    let found: Vec<(usize, &str)> = text
        .split('\n')
        .enumerate()
        .filter(|(_, l)| links_to(l, name))
        .collect();
    match found.as_slice() {
        [] => Ok(None),
        [(i, l)] => Ok(Some((*i, l.to_string()))),
        _ => Err(format!(
            "MEMORY.md names {name} on {} lines: edit it by hand. Nothing was deleted.",
            found.len()
        )),
    }
}

/// `MEMORY.md`'s text, `None` when there is none.
fn read_index(index: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(index) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("read MEMORY.md: {e}")),
    }
}

/// Write `text` to `MEMORY.md` only if it still reads `expected`.
fn write_index_if(index: &Path, expected: &Option<String>, text: &str) -> Result<(), String> {
    #[cfg(test)]
    if fault(Fault::IndexEdited) {
        let mut t = std::fs::read_to_string(index).unwrap_or_default();
        t.push_str("- [new](new.md) written meanwhile\n");
        std::fs::write(index, t).unwrap();
    }
    if read_index(index)? != *expected {
        return Err("MEMORY.md changed while this ran: nothing was changed. Try again.".into());
    }
    crate::vault::atomic_write(index, text.as_bytes())
}

fn answer_in(
    root: &Path,
    claude: &Path,
    built_at: &str,
    issues: &[TrackerIssue],
    source_id: &str,
    delete: bool,
) -> Result<String, String> {
    let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut decided = load(root)?;
    // Only a memory offered now: its path is derived here, never taken
    // from the caller, so nothing outside a memory folder can be named.
    let offer = offers_in(root, claude, built_at, issues, &decided)
        .into_iter()
        .find(|o| o.source_id == source_id)
        .ok_or("That memory isn't waiting for an answer any more.")?;
    let before = decided.clone();
    decided.retain(|d| !(d.issue == offer.issue && d.memory == offer.memory));
    let mut decision = Decision {
        issue: offer.issue.clone(),
        memory: offer.memory.clone(),
        choice: "kept".into(),
        at: now(),
        folder: None,
        trash: None,
        hash: None,
        line: None,
        line_at: None,
        index_after: None,
    };
    if !delete {
        decided.push(decision);
        save(root, &decided)?;
        return Ok(offer.memory);
    }
    let folder = offer.path.parent().ok_or("No memory folder.")?;
    let bytes = std::fs::read(&offer.path).map_err(|e| format!("read {}: {e}", offer.memory))?;
    let index = folder.join("MEMORY.md");
    let index_text = read_index(&index)?;
    let line = match &index_text {
        Some(t) => index_line(t, &offer.memory)?,
        None => None,
    };
    let new_index = match (&index_text, &line) {
        (Some(text), Some((at, _))) => {
            let mut lines: Vec<&str> = text.split('\n').collect();
            lines.remove(*at);
            Some(lines.join("\n"))
        }
        _ => None,
    };
    // 1. The copy, so the memory can always come back.
    let trash = trash_copy(root, &offer.issue, &offer.memory, &bytes)?;
    let drop_trash = || {
        if let Ok(p) = crate::vault::safe_join(root, &trash) {
            let _ = std::fs::remove_file(p);
        }
    };
    // 2. The answer with everything Undo needs, before anything changes.
    decision.choice = "deleted".into();
    decision.folder = Some(folder.to_string_lossy().into_owned());
    decision.trash = Some(trash.clone());
    decision.hash = Some(hash(&bytes));
    decision.line_at = line.as_ref().map(|(i, _)| *i);
    decision.line = line.map(|(_, l)| l);
    decision.index_after = new_index.as_deref().map(|t| hash(t.as_bytes()));
    decided.push(decision);
    if let Err(e) = save(root, &decided) {
        drop_trash();
        return Err(e);
    }
    let undo_answer = || {
        let _ = save(root, &before);
        drop_trash();
    };
    // 3. The index line, only if the index is still what was read.
    if let Some(text) = &new_index {
        if let Err(e) = write_index_if(&index, &index_text, text) {
            undo_answer();
            return Err(e);
        }
    }
    // 4. The file.
    if let Err(e) = std::fs::remove_file(&offer.path) {
        if let (Some(old), Some(_)) = (&index_text, &new_index) {
            let _ = crate::vault::atomic_write(&index, old.as_bytes());
        }
        undo_answer();
        return Err(format!("delete {}: {e}", offer.memory));
    }
    let _ = crate::automated_history::record_event(root, HISTORY_DELETED, Some(source_id));
    Ok(offer.memory)
}

fn hash(bytes: &[u8]) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(bytes))
}

/// A copy of the deleted memory in the vault's `.trash/`, vault-relative.
fn trash_copy(root: &Path, issue: &str, name: &str, bytes: &[u8]) -> Result<String, String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let rel = format!(".trash/{nanos}-lesson-memory-{issue}-{name}");
    let full = crate::vault::safe_join(root, &rel)?;
    crate::vault::atomic_write_private(&full, bytes)?;
    Ok(rel)
}

/// Undo a Delete: the file comes back from `.trash/` (only the bytes that
/// were deleted, and never over a file there now) and its line goes back to
/// `MEMORY.md`: at its old place while the index is as the delete left it,
/// else at the end. The memory is then kept: not offered again.
pub(crate) fn restore(root: &Path, source_id: &str) -> Result<String, String> {
    restore_in(root, &claude_dir().ok_or("No home folder.")?, source_id)
}

fn restore_in(root: &Path, claude: &Path, source_id: &str) -> Result<String, String> {
    let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut decided = load(root)?;
    let d = decided
        .iter_mut()
        .find(|d| source_id_of(d) == source_id && d.choice == "deleted")
        .ok_or("That memory wasn't deleted here, so there is nothing to put back.")?;
    let (Some(folder), Some(trash), Some(want)) = (&d.folder, &d.trash, &d.hash) else {
        return Err("This delete kept no copy to put back.".into());
    };
    if !plain_name(&d.memory) {
        return Err("Not a memory file name.".into());
    }
    let folder = PathBuf::from(folder);
    if !folder.starts_with(claude.join("projects")) || !folder.ends_with("memory") {
        return Err("That folder isn't a memory folder: nothing was put back.".into());
    }
    let target = folder.join(&d.memory);
    if target.symlink_metadata().is_ok() {
        return Err(format!(
            "{} is there again: nothing was put back.",
            d.memory
        ));
    }
    let bytes = std::fs::read(crate::vault::safe_join(root, trash)?)
        .map_err(|_| "The deleted copy is gone from the vault's .trash.".to_string())?;
    if hash(&bytes) != *want {
        return Err("The deleted copy changed in .trash: nothing was put back.".into());
    }
    let index = folder.join("MEMORY.md");
    let index_text = read_index(&index)?;
    let new_index = match &d.line {
        Some(line) => {
            let text = index_text.clone().unwrap_or_default();
            if index_line(&text, &d.memory)?.is_some() {
                None
            } else {
                let mut lines: Vec<&str> = if text.is_empty() {
                    Vec::new()
                } else {
                    text.split('\n').collect()
                };
                let unchanged = d.index_after.as_deref() == Some(hash(text.as_bytes()).as_str());
                let at = match d.line_at {
                    Some(at) if unchanged && at <= lines.len() => at,
                    // Edited since: at the end, before a final newline.
                    _ => lines.len() - usize::from(lines.last() == Some(&"")),
                };
                lines.insert(at, line);
                Some(lines.join("\n"))
            }
        }
        None => None,
    };
    d.choice = "restored".into();
    d.at = now();
    let memory = d.memory.clone();
    if let Some(text) = &new_index {
        write_index_if(&index, &index_text, text)?;
    }
    let undo_index = || {
        if let (Some(old), Some(_)) = (&index_text, &new_index) {
            let _ = crate::vault::atomic_write(&index, old.as_bytes());
        } else if new_index.is_some() {
            let _ = std::fs::remove_file(&index);
        }
    };
    if let Err(e) = crate::vault::atomic_write(&target, &bytes) {
        undo_index();
        return Err(e);
    }
    if let Err(e) = save(root, &decided) {
        let _ = std::fs::remove_file(&target);
        undo_index();
        return Err(e);
    }
    let _ = crate::automated_history::record_event(root, HISTORY_RESTORED, Some(source_id));
    Ok(memory)
}

fn source_id_of(d: &Decision) -> String {
    source_id(&d.issue, &d.memory)
}

#[cfg(test)]
#[path = "lesson_memories_tests.rs"]
mod tests;
