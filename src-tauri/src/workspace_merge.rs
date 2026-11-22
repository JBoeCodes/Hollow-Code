//! Merge one workspace into another, and undo it (master plan #18,
//! overnight B1).
//!
//! [`plan`] lists exactly what a merge would move: every file under the
//! source workspace's `sources/` and `outputs/` and every conversation that
//! ran there, archived ones included. A name already taken in the target
//! (compared without case, as APFS does) gets a `-merged` suffix, so nothing
//! is overwritten; a source's provenance record follows its file's new name.
//! Symbolic links and Finder's `.DS_Store` never move: links are listed as
//! left behind. Its `hash` binds the person's approval (the rail's confirm
//! dialog, or an agent's card) to that exact list: [`merge`] recomputes the
//! plan and refuses when it differs.
//!
//! A merge then, in this order: moves each conversation the way a chat move
//! does (`session_store::move_agent_session_with`, which also moves its
//! session record); moves the files (`vault::move_note`: never replaces,
//! keeps the search indexes current); appends the source's brief, plan and
//! sources list to the target's under one dated heading; writes the merge
//! record ([`RECORD`], plus a readable [`RECORD_NOTE`]) into the source; and
//! archives the source with `merged_into: <target>`. Nothing is deleted:
//! anything else in the source folder (its AGENTS.md, a session record you
//! edited) stays in the archived workspace.
//!
//! [`undo`] reads that record and puts back exactly what moved: files,
//! conversations still in the target, the appended sections (when they are
//! still there verbatim) and the source's status. History alone could not:
//! it keeps no copy of hidden files (every source's provenance record) or of
//! files over 2 MB, and a History restore is all or nothing. A merge that
//! stopped part-way still writes the record, so Undo merge puts back what
//! moved.

use crate::automated_history::content_hash;
use crate::workspaces::{self, Workspace};
use crate::{frontmatter, session_store, vault};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The folders whose files move.
const MOVED_DIRS: [&str; 2] = ["sources", "outputs"];
/// A plan with more files than this is refused (a card or dialog could not
/// show it, and a merge that large deserves a look in Finder first).
pub(crate) const MAX_FILES: usize = 5_000;
/// Room a merged SOURCES.md keeps for later attachments (a source's entry is
/// about 300 bytes), and what the appended section adds besides the list.
const ATTACH_ROOM: u64 = 2_000;
const SECTION_OVERHEAD: u64 = 300;
/// The machine-readable merge record, in the merged (source) workspace.
pub(crate) const RECORD: &str = ".hollow-merge.json";
/// The readable copy of the record beside it.
pub(crate) const RECORD_NOTE: &str = "MERGED.md";
/// How an appended section's heading opens: `## Merged from “<title>”
/// (<id>), <day>` ([`merged_from_ids`] reads the ids back).
const MERGED_HEADING: &str = "## Merged from “";

/// The workspaces whose documents were merged into `text` (a target's
/// PLAN.md), from its "Merged from" headings (morning P1: their plan rows
/// keep their tasks).
pub(crate) fn merged_from_ids(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // At any depth: a merged workspace's own merged sections come along two
    // levels deeper (`#### Merged from …`) when it is merged on.
    let tail = MERGED_HEADING.trim_start_matches('#');
    for line in text.lines() {
        if !line.starts_with("##") {
            continue;
        }
        let Some(rest) = line.trim_start_matches('#').strip_prefix(tail) else {
            continue;
        };
        // The title may hold anything; the id is the last "(…)," group.
        let Some(close) = rest.rfind("), ") else {
            continue;
        };
        let Some(open) = rest[..close].rfind(" (") else {
            continue;
        };
        let id = &rest[open + 2..close];
        let plain = !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if plain && !out.iter().any(|x| x == id) {
            out.push(id.to_string());
        }
    }
    out
}

/// The documents whose text is appended to the target's.
const DOCS: [&str; 3] = ["WORKSPACE.md", "PLAN.md", "SOURCES.md"];

/// One file that moves, vault-relative.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct MergeFile {
    pub from: String,
    pub to: String,
    /// True when `to` carries a suffix because the name was taken.
    pub renamed: bool,
}

/// One conversation that moves.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct MergeSession {
    pub key: String,
    pub title: String,
    pub archived: bool,
}

/// What a merge would do (see the module docs).
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct MergePlan {
    pub from: String,
    pub from_title: String,
    pub into: String,
    pub into_title: String,
    pub files: Vec<MergeFile>,
    pub sessions: Vec<MergeSession>,
    /// Paths (relative to the source workspace) that stay behind: symbolic
    /// links, which never move.
    pub left: Vec<String>,
    /// True while the source still has its git worktree (it stays; only
    /// the workspace is archived).
    pub keeps_worktree: bool,
    /// Binds an approval to this exact plan.
    pub hash: String,
    /// Why the merge can't run right now (a conversation is working), set
    /// by [`preview`] so the confirm says so before Merge (P1-38). Not part
    /// of the hash: [`merge`] checks again.
    pub blocked: Option<String>,
}

/// What a merge did.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct MergeOutcome {
    pub plan: MergePlan,
    /// Open question cards the conversation moves expired.
    pub expired_cards: usize,
    /// Problems that did not stop the merge (a session record left behind).
    pub warnings: Vec<String>,
}

/// What an undo put back.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UnmergeOutcome {
    pub from: String,
    pub into: String,
    pub files: usize,
    pub sessions: usize,
    /// The conversations that went back (their moved notices follow, P1-38).
    pub session_keys: Vec<String>,
    /// What could not go back, and where it is.
    pub warnings: Vec<String>,
}

/// The merge record ([`RECORD`]): enough to put everything back.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Record {
    pub version: u32,
    pub from: String,
    pub into: String,
    pub at: String,
    pub previous_status: String,
    /// Files that moved, in order.
    pub files: Vec<MergeFile>,
    /// Conversations that moved.
    pub sessions: Vec<String>,
    /// Document name → the exact section appended to the target's copy.
    pub appended: BTreeMap<String, String>,
    /// Documents the merge created in the target (undo removes them when
    /// they are back to their seed).
    #[serde(default)]
    pub created: Vec<String>,
    /// False until the merge finished.
    pub complete: bool,
}

/// The plan for merging workspace `from` into `into`, or why it can't run.
pub fn plan(root: &Path, from: &str, into: &str) -> Result<MergePlan, String> {
    let (from, into) = (from.trim(), into.trim());
    if from == into {
        return Err("A workspace can't be merged into itself.".into());
    }
    if workspaces::is_builtin(from) {
        return Err(
            "That workspace is built in and can't be merged. Move its conversations instead."
                .into(),
        );
    }
    if workspaces::is_builtin(into) {
        return Err(
            "Chats and the COO can't receive a merge. Move conversations there instead.".into(),
        );
    }
    let source = workspaces::summary(root, from)?;
    let target = workspaces::summary(root, into)?;
    if workspaces::tracker_managed(root, from) {
        return Err("That workspace follows its task tracker project and would come back on sync. Merge the other way.".into());
    }
    if is_archived(&source) {
        return Err(format!(
            "“{}” is archived. Unarchive it first if you want to merge it.",
            source.title
        ));
    }
    if source.merged_into.is_some() || from_dir_has_record(root, from) {
        return Err(format!(
            "“{}” already has a merge recorded (it may have stopped part-way). Use Undo merge first.",
            source.title
        ));
    }
    if is_archived(&target) {
        return Err(format!(
            "“{}” is archived. Unarchive it first if you want to merge into it.",
            target.title
        ));
    }
    let from_dir = workspaces::folder(root, from)?;
    let into_dir = workspaces::folder(root, into)?;
    for dir in MOVED_DIRS {
        if fs::symlink_metadata(into_dir.join(dir)).is_ok_and(|m| !m.is_dir()) {
            return Err(format!(
                "“{}”'s {dir}/ is not a plain folder (a file or a symbolic link). Fix that in Finder first.",
                target.title
            ));
        }
    }
    let mut files = Vec::new();
    let mut left = Vec::new();
    let mut taken = Taken::default();
    for dir in MOVED_DIRS {
        let top = from_dir.join(dir);
        if fs::symlink_metadata(&top).is_ok_and(|m| m.file_type().is_symlink()) {
            left.push(format!("{dir}/"));
            continue;
        }
        let mut found = Vec::new();
        walk(&top, Path::new(dir), &mut found, &mut left)?;
        if found.len() > MAX_FILES {
            return Err(too_many(&source));
        }
        found.sort();
        // Provenance records follow their source file (`.provenance/<name>.json`
        // for `sources/<name>`), so they are placed after the files.
        let provenance = Path::new("sources/.provenance");
        let (records, plain): (Vec<PathBuf>, Vec<PathBuf>) = found.into_iter().partition(|rel| {
            dir == "sources"
                && rel.parent() == Some(provenance)
                && rel.extension().is_some_and(|e| e == "json")
        });
        let mut renamed_sources: BTreeMap<String, String> = BTreeMap::new();
        for rel in plain {
            let to = free_name(&into_dir, &rel, &mut taken)?;
            if dir == "sources" && rel.parent() == Some(Path::new("sources")) && to != rel {
                if let (Some(old), Some(new)) = (rel.file_name(), to.file_name()) {
                    renamed_sources
                        .insert(old.to_string_lossy().into(), new.to_string_lossy().into());
                }
            }
            files.push(file_move(from, into, &rel, &to));
        }
        for rel in records {
            let name = rel
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let wanted = match renamed_sources.get(&name) {
                Some(new) => provenance.join(format!("{new}.json")),
                None => rel.clone(),
            };
            let to = free_name(&into_dir, &wanted, &mut taken)?;
            files.push(file_move(from, into, &rel, &to));
        }
        if files.len() > MAX_FILES {
            return Err(too_many(&source));
        }
    }
    // The target must still take a new source afterwards (morning P1): the
    // caps an attach checks, its 100 source files and its 48 KB sources list.
    let moving = workspaces::source_count(root, from)?;
    let sources_after = workspaces::source_count(root, into)? + moving;
    // An attach refuses at the cap itself, so the merge leaves room for one.
    if moving > 0 && sources_after >= workspaces::MAX_LISTED_SOURCES {
        return Err(format!(
            "Together they'd hold {sources_after} source files, and a workspace takes no more attachments at {}, so nothing more could be attached to “{}”. Move some sources out first. Nothing changed.",
            workspaces::MAX_LISTED_SOURCES,
            target.title
        ));
    }
    let size = |dir: &Path| fs::metadata(dir.join("SOURCES.md")).map_or(0, |m| m.len());
    // The appended section adds a heading and a separator to the source's list.
    let list_after = size(&into_dir) + size(&from_dir) + SECTION_OVERHEAD;
    if size(&from_dir) > 0 && list_after > workspaces::MAX_CONTEXT - ATTACH_ROOM {
        return Err(format!(
            "Together their sources lists would be about {} KB, past the {} KB a workspace's SOURCES.md holds, so nothing more could be attached to “{}”. Trim one of the lists first. Nothing changed.",
            list_after / 1000,
            workspaces::MAX_CONTEXT / 1000,
            target.title
        ));
    }
    let sessions = sessions_of(root, &from_dir, &source)?;
    let mut plan = MergePlan {
        from: from.into(),
        from_title: source.title.clone(),
        into: into.into(),
        into_title: target.title.clone(),
        files,
        sessions,
        left,
        keeps_worktree: source.build.as_ref().is_some_and(|b| b.exists),
        hash: String::new(),
        blocked: None,
    };
    plan.hash = plan_hash(&plan);
    Ok(plan)
}

/// [`plan`] with `blocked` filled in: what the confirm shows. `allow_run`
/// as in [`merge`].
pub fn preview(
    root: &Path,
    from: &str,
    into: &str,
    allow_run: Option<&str>,
) -> Result<MergePlan, String> {
    let mut plan = plan(root, from, into)?;
    plan.blocked = busy(
        root,
        [
            (&plan.from, &plan.from_title),
            (&plan.into, &plan.into_title),
        ],
        &session_keys(&plan),
        allow_run,
    );
    Ok(plan)
}

fn session_keys(plan: &MergePlan) -> Vec<(String, String)> {
    plan.sessions
        .iter()
        .map(|s| (s.key.clone(), s.title.clone()))
        .collect()
}

fn from_dir_has_record(root: &Path, id: &str) -> bool {
    workspaces::folder(root, id).is_ok_and(|d| fs::symlink_metadata(d.join(RECORD)).is_ok())
}

fn is_archived(w: &Workspace) -> bool {
    w.status.trim().eq_ignore_ascii_case("archived")
}

fn too_many(source: &Workspace) -> String {
    format!(
        "“{}” has more than {MAX_FILES} files in sources/ and outputs/. Tidy it in Finder first.",
        source.title
    )
}

fn file_move(from: &str, into: &str, rel: &Path, to: &Path) -> MergeFile {
    MergeFile {
        from: format!("workspaces/{from}/{}", slash(rel)),
        to: format!("workspaces/{into}/{}", slash(to)),
        renamed: to != rel,
    }
}

fn plan_hash(plan: &MergePlan) -> String {
    let body = serde_json::json!({
        "from": plan.from,
        "into": plan.into,
        "files": plan.files,
        "sessions": plan.sessions.iter().map(|s| &s.key).collect::<Vec<_>>(),
        "left": plan.left,
    });
    content_hash(body.to_string().as_bytes())
}

fn slash(rel: &Path) -> String {
    rel.to_string_lossy().replace('\\', "/")
}

/// Every regular file under `dir`, as paths relative to the workspace folder
/// (`base` is `dir`'s). Symbolic links go to `left` (they never move);
/// `.DS_Store` is skipped. Stops early past [`MAX_FILES`].
fn walk(
    dir: &Path,
    base: &Path,
    out: &mut Vec<PathBuf>,
    left: &mut Vec<String>,
) -> Result<(), String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("read {}: {e}", base.display())),
    };
    let mut entries = entries
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("read {}: {e}", base.display()))?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name();
        if name == ".DS_Store" {
            continue;
        }
        let rel = base.join(&name);
        let kind = entry
            .file_type()
            .map_err(|e| format!("read {}: {e}", rel.display()))?;
        if kind.is_symlink() {
            left.push(slash(&rel));
        } else if kind.is_dir() {
            walk(&entry.path(), &rel, out, left)?;
        } else {
            out.push(rel);
        }
        if out.len() > MAX_FILES {
            return Ok(());
        }
    }
    Ok(())
}

/// The target-side names this merge has already handed out, compared
/// without case (APFS's default): files, and every folder above one.
#[derive(Default)]
struct Taken {
    files: HashSet<String>,
    dirs: HashSet<String>,
    /// Folders this merge pushed aside (`deep` → `deep-merged`), by their
    /// folded source path: every later file under one follows it, so a
    /// folder never splits across `-merged` and `-merged-2` (morning P1).
    moved: BTreeMap<String, PathBuf>,
}

fn fold(p: &Path) -> String {
    slash(p).to_lowercase()
}

fn ancestors(p: &Path) -> impl Iterator<Item = &Path> {
    p.ancestors().skip(1).filter(|a| !a.as_os_str().is_empty())
}

impl Taken {
    /// Whether ancestor `a` can't hold files: it is a file (or a link) in
    /// the target, or a file this merge already placed.
    fn ancestor_blocked(&self, into_dir: &Path, a: &Path) -> bool {
        self.files.contains(&fold(a))
            || fs::symlink_metadata(into_dir.join(a)).is_ok_and(|m| !m.is_dir())
    }

    fn blocked(&self, into_dir: &Path, p: &Path) -> bool {
        let key = fold(p);
        self.files.contains(&key)
            || self.dirs.contains(&key)
            || fs::symlink_metadata(into_dir.join(p)).is_ok()
            || ancestors(p).any(|a| self.ancestor_blocked(into_dir, a))
    }

    fn take(&mut self, p: &Path) {
        self.files.insert(fold(p));
        for a in ancestors(p) {
            self.dirs.insert(fold(a));
        }
    }
}

/// `rel` in the target, or the first `<stem>-merged[-N]<ext>` that is free
/// there and not handed out by this merge. When a folder on the path is the
/// clash (it exists as a file in the target), that folder is renamed
/// instead, so the files under it stay together.
fn free_name(into_dir: &Path, rel: &Path, taken: &mut Taken) -> Result<PathBuf, String> {
    // Under a folder already pushed aside: follow it, and if the name is
    // taken inside, suffix the file only.
    let parts: Vec<std::ffi::OsString> = rel.iter().map(|p| p.to_os_string()).collect();
    for i in 1..parts.len().saturating_sub(1) {
        let prefix: PathBuf = parts[..=i].iter().collect();
        if let Some(moved) = taken.moved.get(&fold(&prefix)).cloned() {
            let mut inside = moved;
            inside.extend(&parts[i + 1..]);
            // A folder inside it that clashes too (a file in the target):
            // only a new folder name can help, so the plain search runs.
            if ancestors(&inside).any(|a| taken.ancestor_blocked(into_dir, a)) {
                break;
            }
            return suffixed(into_dir, &inside, taken, true);
        }
    }
    suffixed(into_dir, rel, taken, false)
}

/// [`free_name`]'s search from `rel`; `file_only` keeps every folder as it
/// is (a file under a pushed-aside folder).
fn suffixed(
    into_dir: &Path,
    rel: &Path,
    taken: &mut Taken,
    file_only: bool,
) -> Result<PathBuf, String> {
    if !taken.blocked(into_dir, rel) {
        taken.take(rel);
        return Ok(rel.to_path_buf());
    }
    let mut parts: Vec<std::ffi::OsString> = rel.iter().map(|p| p.to_os_string()).collect();
    // The shallowest blocked folder below sources/ or outputs/ (which plan
    // checked are real folders), else the file itself.
    let mut at = parts.len() - 1;
    for i in 1..parts.len().saturating_sub(1) {
        let prefix: PathBuf = parts[..=i].iter().collect();
        if !file_only && taken.ancestor_blocked(into_dir, &prefix) {
            at = i;
            break;
        }
    }
    let folder: PathBuf = parts[..=at].iter().collect();
    let original = PathBuf::from(&parts[at]);
    let is_file = at == parts.len() - 1;
    let (stem, ext) = match (is_file, original.file_stem(), original.extension()) {
        (true, Some(stem), Some(ext)) if !stem.is_empty() => (
            stem.to_string_lossy().to_string(),
            format!(".{}", ext.to_string_lossy()),
        ),
        _ => (original.to_string_lossy().to_string(), String::new()),
    };
    for n in 1..=10_000 {
        let suffix = if n == 1 {
            "-merged".to_string()
        } else {
            format!("-merged-{n}")
        };
        parts[at] = format!("{stem}{suffix}{ext}").into();
        let candidate: PathBuf = parts.iter().collect();
        if !taken.blocked(into_dir, &candidate) {
            taken.take(&candidate);
            if !is_file {
                let moved: PathBuf = parts[..=at].iter().collect();
                // The first name a folder got stays its name.
                taken.moved.entry(fold(&folder)).or_insert(moved);
            }
            return Ok(candidate);
        }
    }
    Err(format!(
        "No free name for {} in the target. Nothing changed.",
        slash(rel)
    ))
}

/// Every conversation (archived too) whose folder is the workspace's own or
/// its build worktree, oldest first.
fn sessions_of(
    root: &Path,
    dir: &Path,
    workspace: &Workspace,
) -> Result<Vec<MergeSession>, String> {
    let dirs = workspaces::session_dirs(dir, workspace);
    let rows = session_store::with_read(root, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT key, title, cwd, archived FROM sessions ORDER BY created_at ASC, key ASC",
            )
            .map_err(|e| format!("merge sessions: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                ))
            })
            .map_err(|e| format!("merge sessions: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("merge sessions: {e}"))?;
        Ok(rows)
    })?;
    Ok(rows
        .into_iter()
        .filter(|(_, _, cwd, _)| dirs.contains(&workspaces::session_dir(cwd)))
        .map(|(key, title, _, archived)| MergeSession {
            key,
            title: title.unwrap_or_default(),
            archived,
        })
        .collect())
}

/// Why a merge or undo between `a` and `b` can't run right now (a
/// conversation is working), or `None`. `allow_run` is the calling agent's
/// own run, which may be running in either folder; every other run refuses.
fn busy(
    root: &Path,
    folders: [(&str, &str); 2],
    keys: &[(String, String)],
    allow_run: Option<&str>,
) -> Option<String> {
    for (id, title) in folders {
        let others = crate::automated_history::scope_holders(root, &format!("workspaces/{id}"))
            .into_iter()
            .any(|run| Some(run.as_str()) != allow_run);
        if others {
            return Some(format!(
                "A conversation in “{title}” is still running. Try again when the turn finishes."
            ));
        }
    }
    for (key, title) in keys {
        if crate::agent_sessions::live_for_key(key).is_some() || queued(root, key) {
            return Some(format!(
                "The conversation “{}” has a turn running or queued. Try again when it finishes.",
                if title.is_empty() { key } else { title }
            ));
        }
    }
    None
}

fn queued(root: &Path, key: &str) -> bool {
    session_store::with_read(root, |conn| {
        conn.query_row(
            "SELECT COUNT(*) FROM run_queue WHERE source = 'chat' AND source_id = ?1 \
             AND status IN ('queued', 'claimed', 'running')",
            [key],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|e| e.to_string())
    })
    .map_or(true, |n| n > 0)
}

/// Merge `from` into `into`, refusing unless the current plan's hash is
/// `expected` (what the person approved). `allow_run` is the calling
/// agent's run id (it may be running in the target), `None` from the rail.
pub fn merge(
    root: &Path,
    from: &str,
    into: &str,
    expected: &str,
    allow_run: Option<&str>,
) -> Result<MergeOutcome, String> {
    let plan = plan(root, from, into)?;
    if plan.hash != expected.trim() {
        return Err("What this merge would move changed since it was shown. Nothing changed; look at it again.".into());
    }
    if let Some(why) = busy(
        root,
        [
            (&plan.from, &plan.from_title),
            (&plan.into, &plan.into_title),
        ],
        &session_keys(&plan),
        allow_run,
    ) {
        return Err(why);
    }
    let source = workspaces::summary(root, &plan.from)?;
    let mut record = Record {
        version: 1,
        from: plan.from.clone(),
        into: plan.into.clone(),
        at: chrono::Local::now().to_rfc3339(),
        previous_status: source.status.clone(),
        ..Record::default()
    };
    let mut warnings = Vec::new();
    let mut expired_cards = 0;
    // The record exists (and the source names its target) before anything
    // moves, so even a merge killed part-way can be undone.
    begin_record(root, &plan, &record)?;
    // Conversations first: a move is the step most likely to refuse.
    for s in &plan.sessions {
        match session_store::move_agent_session_with(root, &s.key, &plan.into, || Ok(())) {
            Ok(moved) => {
                if moved.moved {
                    record.sessions.push(s.key.clone());
                }
                expired_cards += moved.expired_cards;
                warnings.extend(moved.warnings);
            }
            Err(error) => {
                return Err(stopped(
                    root,
                    &plan,
                    &record,
                    &format!("Couldn't move “{}”: {error}.", label(s)),
                ))
            }
        }
    }
    if let Err(error) = write_record(root, &plan, &record) {
        return Err(stopped(
            root,
            &plan,
            &record,
            &format!("The merge record could not be saved: {error}."),
        ));
    }
    for (i, f) in plan.files.iter().enumerate() {
        if let Err(error) = move_file(root, &f.from, &f.to) {
            return Err(stopped(
                root,
                &plan,
                &record,
                &format!("Moving {} stopped: {error}.", f.from),
            ));
        }
        record.files.push(f.clone());
        if (i + 1) % 200 == 0 {
            if let Err(error) = write_record(root, &plan, &record) {
                return Err(stopped(
                    root,
                    &plan,
                    &record,
                    &format!("The merge record could not be saved: {error}."),
                ));
            }
        }
    }
    if let Ok(dir) = workspaces::folder(root, &plan.from) {
        for top in MOVED_DIRS {
            prune_empty(&dir.join(top), false);
        }
    }
    let (appended, created) = append_docs(root, &plan, &mut warnings);
    record.appended = appended;
    record.created = created;
    record.complete = true;
    if let Err(error) = write_record(root, &plan, &record) {
        return Err(stopped(
            root,
            &plan,
            &record,
            &format!("The merge record could not be saved: {error}."),
        ));
    }
    archive_source(root, &plan, &record.previous_status)?;
    vault::invalidate_walk_cache();
    vault::invalidate_index();
    Ok(MergeOutcome {
        plan,
        expired_cards,
        warnings,
    })
}

/// Write the empty record and mark the source with `merged_into` before
/// anything moves.
fn begin_record(root: &Path, plan: &MergePlan, record: &Record) -> Result<(), String> {
    write_record(root, plan, record)?;
    let marked = vault::mutate_note_preserve_mtime(
        root,
        &format!("workspaces/{}/WORKSPACE.md", plan.from),
        |text| {
            frontmatter::patch(
                text,
                &[("merged_into", frontmatter::KeyEdit::Set(plan.into.clone()))],
                None,
            )
        },
    );
    if let Err(error) = marked {
        clear_record(root, &plan.from);
        return Err(format!(
            "“{}” could not be marked for the merge: {error}. Nothing changed.",
            plan.from_title
        ));
    }
    Ok(())
}

/// Remove the record and the `merged_into` mark (best effort).
fn clear_record(root: &Path, from: &str) {
    if let Ok(dir) = workspaces::folder(root, from) {
        let _ = fs::remove_file(dir.join(RECORD));
        let _ = fs::remove_file(dir.join(RECORD_NOTE));
    }
    let _ = vault::mutate_note_preserve_mtime(
        root,
        &format!("workspaces/{from}/WORKSPACE.md"),
        |text| frontmatter::patch(text, &[("merged_into", frontmatter::KeyEdit::Remove)], None),
    );
    vault::invalidate_walk_cache();
}

/// A merge that stopped part-way: when nothing moved, take the record back;
/// otherwise keep it current (so Undo merge puts back what moved) and say so.
fn stopped(root: &Path, plan: &MergePlan, record: &Record, why: &str) -> String {
    if record.sessions.is_empty() && record.files.is_empty() {
        clear_record(root, &plan.from);
        return format!("{why} Nothing changed.");
    }
    let kept = write_record(root, plan, record);
    vault::invalidate_walk_cache();
    match kept {
        Ok(()) => format!(
            "{why} {} conversation(s) and {} file(s) had already moved. “Undo merge” in “{}”'s menu puts them back.",
            record.sessions.len(),
            record.files.len(),
            plan.from_title
        ),
        Err(error) => format!(
            "{why} {} conversation(s) and {} file(s) had already moved, and the merge record could not be updated ({error}). “Undo merge” in “{}”'s menu puts back what it lists.",
            record.sessions.len(),
            record.files.len(),
            plan.from_title
        ),
    }
}

/// Whether a moved file belongs in the search indexes (what `move_note`
/// would index): visible Markdown notes up to 2 MB.
fn indexed(rel: &str, size: u64) -> bool {
    rel.to_lowercase().ends_with(".md")
        && size <= 2 * 1024 * 1024
        && !rel.split('/').any(|part| part.starts_with('.'))
}

/// Move one planned file: never replaces (`rename_no_replace`), never
/// follows a link at either name (only the parents go through
/// `safe_join`), and reads the file only when it is a note the search
/// indexes keep.
fn move_file(root: &Path, from_rel: &str, to_rel: &str) -> Result<(), String> {
    let split = |rel: &str| -> Result<(String, std::ffi::OsString), String> {
        let path = Path::new(rel);
        match (path.parent(), path.file_name()) {
            (Some(parent), Some(name)) => Ok((slash(parent), name.to_os_string())),
            _ => Err(format!("{rel} is not a file path")),
        }
    };
    let (from_parent, from_name) = split(from_rel)?;
    let (to_parent, to_name) = split(to_rel)?;
    let from = vault::safe_join(root, &from_parent)?.join(&from_name);
    let meta = fs::symlink_metadata(&from).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("it is no longer a plain file".into());
    }
    let to_dir = vault::safe_join(root, &to_parent)?;
    fs::create_dir_all(&to_dir).map_err(|e| format!("mkdir: {e}"))?;
    let to = to_dir.join(&to_name);
    vault::rename_no_replace(&from, &to).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            "its new name was taken meanwhile".to_string()
        } else {
            e.to_string()
        }
    })?;
    crate::vault_search_index::on_remove(root, from_rel);
    crate::vault_embed_index::on_remove(root, from_rel);
    if indexed(to_rel, meta.len()) {
        if let Ok(content) = fs::read_to_string(&to) {
            let modified = to
                .metadata()
                .and_then(|m| m.modified())
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
                .unwrap_or_default();
            crate::vault_search_index::on_write(root, to_rel, &content, &modified);
            crate::vault_embed_index::on_write(root, to_rel, &content, &modified, meta.len());
        }
    }
    Ok(())
}

fn label(s: &MergeSession) -> &str {
    if s.title.is_empty() {
        &s.key
    } else {
        &s.title
    }
}

/// Remove empty folders under `dir` (and `dir` itself when `remove_self`).
fn prune_empty(dir: &Path, remove_self: bool) {
    if fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink()) {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            prune_empty(&entry.path(), true);
        }
    }
    if remove_self {
        // Only an empty folder goes (remove_dir refuses anything else); a
        // leftover .DS_Store keeps its folder.
        let _ = fs::remove_dir(dir);
    }
}

/// `text`'s body (frontmatter dropped), its Markdown headings two levels
/// deeper so they nest under the merge heading; fenced code is untouched.
pub(crate) fn nested_body(text: &str) -> String {
    let body = if text.starts_with("---\n") || text.starts_with("---\r\n") {
        frontmatter::parse(text).body
    } else {
        text.to_string()
    };
    let mut fenced = false;
    let mut out = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
        }
        if !fenced && line.starts_with('#') {
            let level = line.chars().take_while(|c| *c == '#').count();
            if line[level..].starts_with(' ') && level + 2 <= 6 {
                out.push(format!("##{line}"));
                continue;
            }
            if line[level..].starts_with(' ') {
                // Deeper than h6: keep the text, bold, so nothing is lost.
                out.push(format!("**{}**", line[level..].trim()));
                continue;
            }
        }
        out.push(line.to_string());
    }
    out.join("\n").trim().to_string()
}

/// Characters that make a match part of a longer path or name.
fn path_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | '/' | '%')
}

/// Replace each `old` path with its `new` in one pass (so a replacement is
/// never replaced again), only where `old` stands as a whole relative path:
/// not inside a longer name and not after another path (`../x/outputs/a.md`,
/// `https://…/outputs/a.md`), though a leading `./` is fine.
fn replace_paths(text: &str, pairs: &[(String, String)]) -> String {
    let mut pairs: Vec<&(String, String)> = pairs
        .iter()
        .filter(|(old, new)| !old.is_empty() && old != new)
        .collect();
    pairs.sort_by_key(|p| std::cmp::Reverse(p.0.len()));
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    'scan: while i < text.len() {
        let head = &text[..i];
        let starts_path = match head.chars().next_back() {
            None => true,
            // `./old` is the same relative path; `../old` or `x/old` is not.
            Some('/') => {
                head.ends_with("./")
                    && !head.ends_with("../")
                    && head[..head.len() - 2]
                        .chars()
                        .next_back()
                        .is_none_or(|c| !path_char(c))
            }
            Some(c) => !path_char(c),
        };
        if starts_path {
            for (old, new) in &pairs {
                if text[i..].starts_with(old.as_str())
                    && !text[i + old.len()..].chars().next().is_some_and(path_char)
                {
                    out.push_str(new);
                    i += old.len();
                    continue 'scan;
                }
            }
        }
        let c = text[i..].chars().next().unwrap_or(' ');
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// The renamed files' links, so appended text points at the moved copies
/// and not at the target's own file of the old name.
fn relink(text: &str, plan: &MergePlan) -> String {
    let prefix_from = format!("workspaces/{}/", plan.from);
    let prefix_into = format!("workspaces/{}/", plan.into);
    let mut pairs = Vec::new();
    let mut titles = BTreeMap::new();
    for f in plan.files.iter().filter(|f| f.renamed) {
        let old = f.from.trim_start_matches(&prefix_from);
        let new = f.to.trim_start_matches(&prefix_into);
        pairs.push((old.to_string(), new.to_string()));
        // SOURCES.md links URL-encode the name, and titles each entry with it.
        if let (Some(old_name), Some(new_name)) = (
            old.strip_prefix("sources/").filter(|n| !n.contains('/')),
            new.strip_prefix("sources/").filter(|n| !n.contains('/')),
        ) {
            pairs.push((
                format!("sources/{}", urlencoding::encode(old_name)),
                format!("sources/{}", urlencoding::encode(new_name)),
            ));
            titles.insert(format!("#### {old_name}"), format!("#### {new_name}"));
        }
    }
    replace_paths(text, &pairs)
        .lines()
        .map(|line| {
            titles
                .get(line)
                .cloned()
                .unwrap_or_else(|| line.to_string())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Append the source's brief, plan and sources list to the target's.
/// Returns each appended section exactly (for undo).
fn append_docs(
    root: &Path,
    plan: &MergePlan,
    warnings: &mut Vec<String>,
) -> (BTreeMap<String, String>, Vec<String>) {
    let mut appended = BTreeMap::new();
    let mut created = Vec::new();
    let day = chrono::Local::now().format("%Y-%m-%d");
    let heading = format!(
        "{MERGED_HEADING}{}” ({}), {day}",
        plan.from_title, plan.from
    );
    let read = |name: &str| -> Option<String> {
        let path = vault::safe_join(root, &format!("workspaces/{}/{name}", plan.from)).ok()?;
        fs::read_to_string(path).ok()
    };
    let prefix_from = format!("workspaces/{}/", plan.from);
    let prefix_into = format!("workspaces/{}/", plan.into);
    let mut notes = Vec::new();
    let renamed: Vec<&MergeFile> = plan.files.iter().filter(|f| f.renamed).collect();
    if !renamed.is_empty() {
        notes.push("Renamed on the way in (the name was taken here):".to_string());
        for f in &renamed {
            notes.push(format!(
                "- `{}` → `{}`",
                f.from.trim_start_matches(&prefix_from),
                f.to.trim_start_matches(&prefix_into)
            ));
        }
    }
    if !plan.sessions.is_empty() {
        notes.push(format!(
            "{} conversation(s) moved here.",
            plan.sessions.len()
        ));
    }
    for (name, seed) in [
        ("WORKSPACE.md", None),
        ("PLAN.md", Some("# Plan\n")),
        ("SOURCES.md", Some("# Sources\n")),
    ] {
        let Some(text) = read(name) else { continue };
        let mut body = nested_body(&text);
        // PLAN.md and SOURCES.md open with their own title; it's implied.
        for title in ["### Plan", "### Sources"] {
            if let Some(rest) = body.strip_prefix(title) {
                if rest.is_empty() || rest.starts_with('\n') {
                    body = rest.trim().to_string();
                }
            }
        }
        let body = relink(&body, plan);
        let mut section = format!("{heading}\n");
        if name == "WORKSPACE.md" && !notes.is_empty() {
            section.push_str(&format!("\n{}\n", notes.join("\n")));
        }
        if !body.is_empty() {
            section.push_str(&format!("\n{body}\n"));
        } else if name != "WORKSPACE.md" {
            continue;
        }
        // The target's text is kept byte for byte; only a separator is
        // added, and the record holds separator + section, so undo removes
        // exactly what was added.
        let joined = |current: &str| {
            let sep = if current.is_empty() || current.ends_with("\n\n") {
                ""
            } else if current.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            format!("{sep}{section}")
        };
        let rel = format!("workspaces/{}/{name}", plan.into);
        let exists = vault::safe_join(root, &rel).is_ok_and(|p| p.exists());
        let mut added = String::new();
        let result = if exists {
            vault::mutate_note(root, &rel, |current| {
                added = joined(current);
                Ok(format!("{current}{added}"))
            })
            .map(|_| ())
        } else {
            let seed = seed.unwrap_or("");
            added = joined(seed);
            vault::write_note(root, &rel, &format!("{seed}{added}")).map(|_| ())
        };
        match result {
            Ok(()) => {
                appended.insert(name.to_string(), added);
                if !exists {
                    created.push(name.to_string());
                }
            }
            Err(error) => warnings.push(format!(
                "The merged text wasn't added to {name}: {error}. It is still in the archived workspace."
            )),
        }
    }
    (appended, created)
}

fn write_record(root: &Path, plan: &MergePlan, record: &Record) -> Result<(), String> {
    let json = serde_json::to_string_pretty(record).map_err(|e| e.to_string())?;
    let dir = workspaces::folder(root, &plan.from)?;
    vault::atomic_write(&dir.join(RECORD), json.as_bytes())?;
    let mut note = format!(
        "# Merged into “{}”\n\nOn {}, this workspace was merged into `{}`{}. Undo merge (this workspace's menu in Hollow) puts back everything listed here.\n",
        plan.into_title,
        record.at,
        plan.into,
        if record.complete { "" } else { " (the merge stopped part-way)" }
    );
    if !record.files.is_empty() {
        note.push_str("\n## Files\n\n");
        for f in &record.files {
            note.push_str(&format!("- `{}` → `{}`\n", f.from, f.to));
        }
    }
    if !record.sessions.is_empty() {
        note.push_str("\n## Conversations\n\n");
        for key in &record.sessions {
            note.push_str(&format!("- `{key}`\n"));
        }
    }
    if !plan.left.is_empty() {
        note.push_str("\n## Left here (symbolic links never move)\n\n");
        for path in &plan.left {
            note.push_str(&format!("- `{path}`\n"));
        }
    }
    vault::write_note(
        root,
        &format!("workspaces/{}/{RECORD_NOTE}", plan.from),
        &note,
    )
    .map(|_| ())
}

fn archive_source(root: &Path, plan: &MergePlan, previous: &str) -> Result<(), String> {
    let rel = format!("workspaces/{}/WORKSPACE.md", plan.from);
    // The status it had, as `update_meta` records on any archive (P1-38).
    let archived_from = match previous.trim() {
        "" => frontmatter::KeyEdit::Remove,
        status => frontmatter::KeyEdit::Set(status.to_string()),
    };
    vault::mutate_note_preserve_mtime(root, &rel, |text| {
        frontmatter::patch(
            text,
            &[
                ("status", frontmatter::KeyEdit::Set("archived".into())),
                ("merged_into", frontmatter::KeyEdit::Set(plan.into.clone())),
                // Archived workspaces are never pinned.
                ("pinned", frontmatter::KeyEdit::Remove),
                ("archived_from", archived_from.clone()),
            ],
            None,
        )
    })
    .map(|_| ())
    .map_err(|error| {
        format!(
            "Everything moved, but “{}” could not be archived: {error}. Archive it from the rail.",
            plan.from_title
        )
    })
}

/// The merge record in workspace `from`, or why there is none.
pub(crate) fn read_record(root: &Path, from: &str) -> Result<Record, String> {
    let dir = workspaces::folder(root, from)?;
    let text = match fs::read_to_string(dir.join(RECORD)) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err("This workspace has no merge to undo.".into())
        }
        Err(e) => return Err(format!("The merge record can't be read: {e}")),
    };
    let record: Record = serde_json::from_str(&text)
        .map_err(|e| format!("The merge record is damaged ({e}); see {RECORD_NOTE}."))?;
    if record.version != 1 || record.from != from || workspaces::folder(root, &record.into).is_err()
    {
        return Err(format!(
            "The merge record doesn't belong to this workspace; see {RECORD_NOTE}."
        ));
    }
    // Undo only ever moves files between the two workspaces' sources/ and
    // outputs/, whatever the record says.
    let inside = |path: &str, id: &str| {
        MOVED_DIRS
            .iter()
            .any(|dir| path.starts_with(&format!("workspaces/{id}/{dir}/")))
            && !path
                .split('/')
                .any(|part| part == ".." || part == "." || part.is_empty())
    };
    if record
        .files
        .iter()
        .any(|f| !inside(&f.from, &record.from) || !inside(&f.to, &record.into))
        || record
            .appended
            .keys()
            .any(|name| !DOCS.contains(&name.as_str()))
        || record
            .created
            .iter()
            .any(|name| !DOCS.contains(&name.as_str()))
    {
        return Err(format!(
            "The merge record names files outside the two workspaces, so nothing was undone; see {RECORD_NOTE}."
        ));
    }
    Ok(record)
}

/// Undo the merge recorded in workspace `from` (see the module docs).
/// Anything that can't go back (a file whose old name is taken again, a
/// conversation moved on since) stays where it is and is named in a warning.
pub fn undo(root: &Path, from: &str, allow_run: Option<&str>) -> Result<UnmergeOutcome, String> {
    let from = from.trim();
    let record = read_record(root, from)?;
    let source = workspaces::summary(root, from)?;
    let target_title = workspaces::summary(root, &record.into)
        .map(|w| w.title)
        .unwrap_or_else(|_| record.into.clone());
    let keys: Vec<(String, String)> = record
        .sessions
        .iter()
        .map(|k| (k.clone(), String::new()))
        .collect();
    if let Some(why) = busy(
        root,
        [(from, &source.title), (&record.into, &target_title)],
        &keys,
        allow_run,
    ) {
        return Err(why);
    }
    let mut warnings = Vec::new();
    let mut files = 0;
    for f in record.files.iter().rev() {
        if fs::symlink_metadata(root.join(&f.from)).is_ok() {
            warnings.push(format!(
                "{} stays at {}: its old name is taken again.",
                f.from, f.to
            ));
            continue;
        }
        if !fs::symlink_metadata(root.join(&f.to)).is_ok_and(|m| m.is_file()) {
            warnings.push(format!(
                "{} is gone or no longer a plain file, so it stays as it is.",
                f.to
            ));
            continue;
        }
        match move_file(root, &f.to, &f.from) {
            Ok(()) => files += 1,
            Err(error) => warnings.push(format!("{} stays where it is: {error}.", f.to)),
        }
    }
    if let Ok(dir) = workspaces::folder(root, &record.into) {
        for top in MOVED_DIRS {
            prune_empty(&dir.join(top), false);
        }
    }
    let mut sessions = 0;
    let mut session_keys = Vec::new();
    let into_cwd = workspaces::session_dir_for(root, &record.into)
        .map(|p| workspaces::session_dir(&p.to_string_lossy()))
        .ok();
    for key in &record.sessions {
        let here = session_store::read_session(root, key)
            .map(|s| Some(workspaces::session_dir(&s.summary.cwd)) == into_cwd)
            .unwrap_or(false);
        if !here {
            warnings.push(format!(
                "The conversation {key} has moved on since, so it stays where it is."
            ));
            continue;
        }
        match session_store::move_agent_session_with(root, key, from, || Ok(())) {
            Ok(moved) => {
                sessions += usize::from(moved.moved);
                if moved.moved {
                    session_keys.push(key.clone());
                }
                warnings.extend(moved.warnings);
            }
            Err(error) => warnings.push(format!("The conversation {key} stays: {error}.")),
        }
    }
    for name in DOCS {
        let Some(section) = record.appended.get(name) else {
            continue;
        };
        let rel = format!("workspaces/{}/{name}", record.into);
        let removed = vault::mutate_note(root, &rel, |current| {
            match current.rfind(section.as_str()) {
                Some(at) => Ok(format!(
                    "{}{}",
                    &current[..at],
                    &current[at + section.len()..]
                )),
                None => Err("edited since".into()),
            }
        });
        if removed.is_ok() && record.created.iter().any(|c| c == name) {
            let seed = if name == "PLAN.md" {
                "# Plan\n"
            } else {
                "# Sources\n"
            };
            if let Ok(path) = vault::safe_join(root, &rel) {
                if fs::read_to_string(&path).is_ok_and(|t| t == seed) {
                    let _ = fs::remove_file(path);
                }
            }
        }
        if removed.is_err() {
            warnings.push(format!(
                "The merged section in “{target_title}”'s {name} was edited since, so it stays; remove it by hand if you like."
            ));
        }
    }
    let status = if record.previous_status.trim().is_empty() {
        "active".to_string()
    } else {
        record.previous_status.clone()
    };
    vault::mutate_note_preserve_mtime(root, &format!("workspaces/{from}/WORKSPACE.md"), |text| {
        frontmatter::patch(
            text,
            &[
                ("status", frontmatter::KeyEdit::Set(status.clone())),
                ("merged_into", frontmatter::KeyEdit::Remove),
                ("archived_from", frontmatter::KeyEdit::Remove),
            ],
            None,
        )
    })?;
    let dir = workspaces::folder(root, from)?;
    let _ = fs::remove_file(dir.join(RECORD));
    let _ = fs::remove_file(dir.join(RECORD_NOTE));
    vault::invalidate_walk_cache();
    vault::invalidate_index();
    Ok(UnmergeOutcome {
        from: from.into(),
        into: record.into,
        files,
        sessions,
        session_keys,
        warnings,
    })
}

/// The confirm dialog's or card's lines: every file and conversation that
/// moves, one per line.
pub(crate) fn describe(plan: &MergePlan) -> Vec<String> {
    let mut lines = vec![format!(
        "Merge “{}” ({}) into “{}” ({}). “{}” is archived afterwards; nothing is deleted, and Undo merge (its menu) puts everything back.",
        plan.from_title, plan.from, plan.into_title, plan.into, plan.from_title
    )];
    lines.push(format!(
        "Its brief, plan and sources list are added to “{}”'s under a dated heading.",
        plan.into_title
    ));
    if plan.keeps_worktree {
        lines.push("Its git worktree stays; remove it separately if you no longer need it.".into());
    }
    let prefix = format!("workspaces/{}/", plan.from);
    let into_prefix = format!("workspaces/{}/", plan.into);
    lines.push(format!("Files that move ({}):", plan.files.len()));
    for f in &plan.files {
        let from = f.from.trim_start_matches(&prefix);
        if f.renamed {
            lines.push(format!(
                "- {from} → {} (name taken)",
                f.to.trim_start_matches(&into_prefix)
            ));
        } else {
            lines.push(format!("- {from}"));
        }
    }
    lines.push(format!(
        "Conversations that move ({}):",
        plan.sessions.len()
    ));
    for s in &plan.sessions {
        lines.push(format!(
            "- {}{}",
            label(s),
            if s.archived { " (archived)" } else { "" }
        ));
    }
    if !plan.left.is_empty() {
        lines.push(format!(
            "Left in “{}” (symbolic links never move): {}",
            plan.from_title,
            plan.left.join(", ")
        ));
    }
    lines
}

/// The undo card's lines.
pub(crate) fn describe_undo(root: &Path, from: &str) -> Result<Vec<String>, String> {
    let record = read_record(root, from.trim())?;
    let mut lines = vec![format!(
        "Undo the merge of {} into {}: {} file(s) and {} conversation(s) go back, the merged sections come out of {}'s brief, plan and sources list, and {} returns to “{}”.",
        record.from,
        record.into,
        record.files.len(),
        record.sessions.len(),
        record.into,
        record.from,
        if record.previous_status.is_empty() { "active" } else { &record.previous_status }
    )];
    for f in &record.files {
        lines.push(format!("- {} → {}", f.to, f.from));
    }
    for key in &record.sessions {
        lines.push(format!("- conversation {key}"));
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn two(root: &Path) -> (Workspace, Workspace) {
        let a = workspaces::create(root, "Old research", "Find the numbers").unwrap();
        let b = workspaces::create(root, "Launch", "Ship it").unwrap();
        (a, b)
    }

    fn add_session(root: &Path, key: &str, cwd: &str, archived: bool) {
        let ord = session_store::start_turn(
            root,
            key,
            "claude",
            cwd,
            &format!("run-{key}"),
            "Test session",
        )
        .unwrap();
        session_store::finish_turn(root, key, ord, "success").unwrap();
        if archived {
            session_store::archive_session(root, key, true).unwrap();
        }
    }

    fn tos(plan: &MergePlan) -> Vec<(String, bool)> {
        let prefix = format!("workspaces/{}/", plan.into);
        plan.files
            .iter()
            .map(|f| (f.to.trim_start_matches(&prefix).to_string(), f.renamed))
            .collect()
    }

    fn has(tos: &[(String, bool)], path: &str, renamed: bool) -> bool {
        tos.iter().any(|(p, r)| p == path && *r == renamed)
    }

    /// Morning P1: a folder pushed aside stays one folder, even when a name
    /// inside its new name is taken (it used to split into `-merged-2`).
    #[test]
    fn a_pushed_aside_folder_never_splits() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b) = two(root);
        write(root, &format!("{}/outputs/deep/x.csv", a.path), "x");
        write(root, &format!("{}/outputs/deep/y.csv", a.path), "y");
        write(root, &format!("{}/outputs/deep/sub/z.csv", a.path), "z");
        write(
            root,
            &format!("{}/outputs/deep", b.path),
            "a file named deep",
        );
        write(
            root,
            &format!("{}/outputs/deep-merged/y.csv", b.path),
            "theirs",
        );
        let first = plan(root, &a.id, &b.id).unwrap();
        let t = tos(&first);
        assert!(has(&t, "outputs/deep-merged/x.csv", true), "{t:?}");
        assert!(has(&t, "outputs/deep-merged/y-merged.csv", true), "{t:?}");
        assert!(has(&t, "outputs/deep-merged/sub/z.csv", true), "{t:?}");
        assert!(!t.iter().any(|(p, _)| p.contains("-merged-2")), "{t:?}");
        // A folder inside the moved one that clashes too (a file there):
        // the plan still succeeds, with a new folder name for that part.
        write(
            root,
            &format!("{}/outputs/deep-merged/sub", b.path),
            "a file named sub",
        );
        let again = plan(root, &a.id, &b.id).unwrap();
        let t = tos(&again);
        // `sub/z.csv` sorts first and takes `deep-merged-2`; the rest follow.
        for f in ["x.csv", "y.csv", "sub/z.csv"] {
            assert!(
                has(&t, &format!("outputs/deep-merged-2/{f}"), true),
                "{t:?}"
            );
        }
        // A file sorting before that clash keeps `deep-merged`; the clashing
        // part needs its own folder, but the plan still succeeds.
        write(root, &format!("{}/outputs/deep/a.csv", a.path), "a");
        let last = plan(root, &a.id, &b.id).unwrap();
        let t = tos(&last);
        assert!(has(&t, "outputs/deep-merged/a.csv", true), "{t:?}");
        assert!(t.iter().any(|(p, _)| p.ends_with("/sub/z.csv")), "{t:?}");
    }

    /// Morning P1: a merge that would leave the target unable to take a new
    /// source (over 100 files, or a sources list past 48 KB) is refused.
    #[test]
    fn a_merge_keeps_room_for_later_attachments() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b) = two(root);
        for i in 0..60 {
            write(root, &format!("{}/sources/a{i}.md", a.path), "a");
            write(root, &format!("{}/sources/b{i}.md", b.path), "b");
        }
        let err = plan(root, &a.id, &b.id).unwrap_err();
        assert!(err.contains("120 source files"), "{err}");
        // The boundary is attach's own: at 100 it takes no more.
        for i in 0..20 {
            fs::remove_file(root.join(&a.path).join(format!("sources/a{i}.md"))).unwrap();
        }
        assert!(plan(root, &a.id, &b.id)
            .unwrap_err()
            .contains("100 source files"));
        fs::remove_file(root.join(&a.path).join("sources/a20.md")).unwrap();
        assert!(plan(root, &a.id, &b.id).is_ok(), "99 leaves room for one");
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b) = two(root);
        let big = format!("# Sources\n\n{}", "- an entry\n".repeat(2_400));
        write(root, &format!("{}/SOURCES.md", a.path), &big);
        write(root, &format!("{}/SOURCES.md", b.path), &big);
        let err = plan(root, &a.id, &b.id).unwrap_err();
        assert!(err.contains("past the 48 KB"), "{err}");
        // Small lists merge.
        write(
            root,
            &format!("{}/SOURCES.md", a.path),
            "# Sources\n\n- one\n",
        );
        assert!(plan(root, &a.id, &b.id).is_ok());
    }

    /// Morning P1: plan rows merged into another workspace keep the tasks
    /// they already became, instead of coming back as new candidates.
    #[test]
    fn merged_plan_rows_keep_their_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b) = two(root);
        write(
            root,
            &format!("{}/PLAN.md", a.path),
            "# Plan\n\n- [ ] Pull the Q3 numbers\n- [ ] Draft the memo\n",
        );
        let before = workspaces::plan_task_candidates(root, &a.id).unwrap();
        let pulled = before
            .iter()
            .find(|c| c.text == "Pull the Q3 numbers")
            .unwrap();
        let task = crate::vault_tasks::create_from_input(
            root,
            &crate::types::CreateTaskInput {
                title: pulled.text.clone(),
                idempotency_key: Some(pulled.idempotency_key.clone()),
                origin: Some(pulled.origin.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        let p = plan(root, &a.id, &b.id).unwrap();
        merge(root, &a.id, &b.id, &p.hash, None).unwrap();
        let after = workspaces::plan_task_candidates(root, &b.id).unwrap();
        let moved = after
            .iter()
            .find(|c| c.text == "Pull the Q3 numbers")
            .unwrap();
        assert_eq!(moved.existing_url, Some(format!("vault:{}", task.rel)));
        assert_eq!(
            moved.idempotency_key, pulled.idempotency_key,
            "a create dedupes to it"
        );
        let open = after.iter().find(|c| c.text == "Draft the memo").unwrap();
        assert_eq!(
            open.existing_url, None,
            "a row that never became a task is still offered"
        );
        assert_eq!(
            merged_from_ids("## Merged from “Q (3) (draft)” (old-research), 2026-10-10\n## Merged from “x” (../bad), 2026-10-10\n#### Merged from “C” (chained), 2026-10-09\nMerged from “no” (heading), 2026-10-09"),
            ["old-research", "chained"]
        );
    }

    #[test]
    fn conflicting_names_get_suffixes_and_nothing_is_lost() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b) = two(root);
        write(root, &format!("{}/outputs/report.md", a.path), "A report");
        write(root, &format!("{}/outputs/deep/x.csv", a.path), "a,b");
        write(root, &format!("{}/outputs/deep/y.csv", a.path), "c,d");
        write(root, &format!("{}/outputs/notes", a.path), "no extension");
        write(root, &format!("{}/sources/memo.pdf", a.path), "pdf");
        write(root, &format!("{}/outputs/report.md", b.path), "B report");
        write(
            root,
            &format!("{}/outputs/report-merged.md", b.path),
            "taken too",
        );
        write(root, &format!("{}/outputs/notes", b.path), "B notes");
        // A file where the source has a folder.
        write(
            root,
            &format!("{}/outputs/deep", b.path),
            "a file named deep",
        );

        let plan = plan(root, &a.id, &b.id).unwrap();
        let t = tos(&plan);
        assert!(has(&t, "outputs/report-merged-2.md", true), "{t:?}");
        assert!(has(&t, "outputs/notes-merged", true), "{t:?}");
        assert!(has(&t, "outputs/deep-merged/x.csv", true), "{t:?}");
        assert!(has(&t, "outputs/deep-merged/y.csv", true), "{t:?}");
        assert!(has(&t, "sources/memo.pdf", false), "{t:?}");

        let out = merge(root, &a.id, &b.id, &plan.hash, None).unwrap();
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        // Every byte arrived, and the target's own files are untouched.
        let read = |rel: &str| fs::read_to_string(root.join(&b.path).join(rel)).unwrap();
        assert_eq!(read("outputs/report.md"), "B report");
        assert_eq!(read("outputs/report-merged.md"), "taken too");
        assert_eq!(read("outputs/report-merged-2.md"), "A report");
        assert_eq!(read("outputs/notes"), "B notes");
        assert_eq!(read("outputs/notes-merged"), "no extension");
        assert_eq!(read("outputs/deep"), "a file named deep");
        assert_eq!(read("outputs/deep-merged/x.csv"), "a,b");
        assert_eq!(read("outputs/deep-merged/y.csv"), "c,d");
        assert_eq!(read("sources/memo.pdf"), "pdf");
        // The source keeps its (now empty) folders and is archived, not gone.
        let src = root.join(&a.path);
        assert!(src.join("outputs").is_dir() && !src.join("outputs/deep").exists());
        let source = workspaces::summary(root, &a.id).unwrap();
        assert_eq!(source.status, "archived");
        assert_eq!(source.merged_into.as_deref(), Some(b.id.as_str()));
        assert!(src.join(RECORD).is_file() && src.join(RECORD_NOTE).is_file());
        // The renamed files are listed in the target's brief.
        let into_brief = read("WORKSPACE.md");
        assert!(
            into_brief.contains("`outputs/report.md` → `outputs/report-merged-2.md`"),
            "{into_brief}"
        );
    }

    #[test]
    fn names_clash_without_case_and_a_planned_folder_blocks_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b) = two(root);
        // Two source files that would both become report-merged.md once
        // case is ignored, and a source file named like a pushed-aside folder.
        write(root, &format!("{}/outputs/Report-Merged.md", a.path), "1");
        write(root, &format!("{}/outputs/report.md", a.path), "2");
        write(root, &format!("{}/outputs/report.md", b.path), "B");
        write(root, &format!("{}/outputs/deep-merged", a.path), "file");
        write(root, &format!("{}/outputs/deep/x.csv", a.path), "x");
        write(root, &format!("{}/outputs/deep", b.path), "B file");
        let plan = plan(root, &a.id, &b.id).unwrap();
        let mut folded: Vec<String> = tos(&plan).iter().map(|(p, _)| p.to_lowercase()).collect();
        let n = folded.len();
        folded.sort();
        folded.dedup();
        assert_eq!(folded.len(), n, "two files got one name: {:?}", tos(&plan));
        merge(root, &a.id, &b.id, &plan.hash, None).unwrap();
        for f in &plan.files {
            assert!(root.join(&f.to).is_file(), "{}", f.to);
        }
    }

    #[test]
    fn provenance_follows_a_renamed_source_and_links_point_at_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b) = two(root);
        write(root, &format!("{}/sources/memo.pdf", a.path), "A memo");
        write(
            root,
            &format!("{}/sources/.provenance/memo.pdf.json", a.path),
            "{\"a\":1}",
        );
        write(root, &format!("{}/sources/.DS_Store", a.path), "finder");
        write(
            root,
            &format!("{}/SOURCES.md", a.path),
            "# Sources\n\n## memo.pdf\n\n- Snapshot: [memo.pdf](sources/memo.pdf)\n",
        );
        write(root, &format!("{}/sources/memo.pdf", b.path), "B memo");
        let plan = plan(root, &a.id, &b.id).unwrap();
        let t = tos(&plan);
        assert!(has(&t, "sources/memo-merged.pdf", true), "{t:?}");
        assert!(
            has(&t, "sources/.provenance/memo-merged.pdf.json", true),
            "{t:?}"
        );
        assert!(!t.iter().any(|(p, _)| p.contains("DS_Store")), "{t:?}");
        merge(root, &a.id, &b.id, &plan.hash, None).unwrap();
        let into = root.join(&b.path);
        assert_eq!(
            fs::read_to_string(into.join("sources/.provenance/memo-merged.pdf.json")).unwrap(),
            "{\"a\":1}"
        );
        assert!(!into.join("sources/.provenance/memo.pdf.json").exists());
        let sources = fs::read_to_string(into.join("SOURCES.md")).unwrap();
        assert!(
            sources.contains(
                "#### memo-merged.pdf\n\n- Snapshot: [memo.pdf](sources/memo-merged.pdf)"
            ),
            "{sources}"
        );
        // Finder's file stayed behind, harmlessly.
        assert!(root.join(&a.path).join("sources/.DS_Store").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_never_move_and_never_drag_their_target() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b) = two(root);
        write(root, "notes/real.md", "real");
        fs::create_dir_all(root.join(&a.path).join("outputs")).unwrap();
        std::os::unix::fs::symlink(
            root.join("notes/real.md"),
            root.join(&a.path).join("outputs/link.md"),
        )
        .unwrap();
        std::os::unix::fs::symlink(root.join("notes"), root.join(&a.path).join("outputs/dir"))
            .unwrap();
        let plan = plan(root, &a.id, &b.id).unwrap();
        assert!(plan.files.is_empty(), "{:?}", plan.files);
        assert_eq!(plan.left, ["outputs/dir", "outputs/link.md"]);
        merge(root, &a.id, &b.id, &plan.hash, None).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("notes/real.md")).unwrap(),
            "real"
        );
        assert!(
            fs::symlink_metadata(root.join(&a.path).join("outputs/link.md"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        // A target whose outputs/ is a link is refused before anything.
        let c = workspaces::create(root, "Linked", "x").unwrap();
        let c_out = root.join(&c.path).join("outputs");
        fs::remove_dir(&c_out).unwrap();
        std::os::unix::fs::symlink(root.join("notes"), &c_out).unwrap();
        let d = workspaces::create(root, "Donor", "x").unwrap();
        assert!(super::plan(root, &d.id, &c.id)
            .unwrap_err()
            .contains("not a plain folder"));
    }

    #[test]
    fn brief_plan_and_sources_are_appended_under_a_dated_heading() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b) = two(root);
        write(
            root,
            &format!("{}/PLAN.md", a.path),
            "# Plan\n\n- [ ] Get the Q3 numbers\n\n## Later\n\n```\n# not a heading\n```\n",
        );
        let plan = plan(root, &a.id, &b.id).unwrap();
        // The merge stamps its own local day: one across midnight may write
        // either (overnight D2).
        let today = || chrono::Local::now().format("%Y-%m-%d").to_string();
        let before = today();
        merge(root, &a.id, &b.id, &plan.hash, None).unwrap();
        let into_plan = fs::read_to_string(root.join(&b.path).join("PLAN.md")).unwrap();
        let day = if into_plan.contains(&format!("), {before}\n")) {
            before
        } else {
            today()
        };
        let heading = format!("## Merged from “Old research” ({}), {day}", a.id);
        assert!(
            into_plan.starts_with("# Plan\n\n- [ ] Define what"),
            "{into_plan}"
        );
        assert!(
            into_plan.contains(&format!("{heading}\n\n- [ ] Get the Q3 numbers")),
            "{into_plan}"
        );
        assert!(into_plan.contains("#### Later"), "{into_plan}");
        assert!(
            into_plan.contains("```\n# not a heading\n```"),
            "{into_plan}"
        );
        assert!(!into_plan.contains("### Plan"), "{into_plan}");
        let into_brief = fs::read_to_string(root.join(&b.path).join("WORKSPACE.md")).unwrap();
        assert!(into_brief.contains(&heading), "{into_brief}");
        assert!(
            into_brief.contains("### Old research")
                && into_brief.contains("#### Outcome\n\nFind the numbers"),
            "{into_brief}"
        );
        assert!(
            !into_brief[into_brief.find(&heading).unwrap()..].contains("hollow_workspace"),
            "frontmatter must not be copied"
        );
        assert_eq!(workspaces::summary(root, &b.id).unwrap().title, "Launch");
        let into_sources = fs::read_to_string(root.join(&b.path).join("SOURCES.md")).unwrap();
        assert!(into_sources.contains(&heading), "{into_sources}");
    }

    #[test]
    fn conversations_move_including_archived_ones_and_records_follow() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b) = two(root);
        let other = workspaces::create(root, "Elsewhere", "Unrelated").unwrap();
        let a_cwd = workspaces::folder(root, &a.id).unwrap();
        let a_cwd = a_cwd.canonicalize().unwrap().to_string_lossy().to_string();
        let other_cwd = workspaces::folder(root, &other.id)
            .unwrap()
            .to_string_lossy()
            .to_string();
        add_session(root, "s-20261010-aaaa0001", &a_cwd, false);
        add_session(root, "s-20261010-aaaa0002", &a_cwd, true);
        add_session(root, "s-20261010-bbbb0001", &other_cwd, false);
        write(
            root,
            &format!("{}/sessions/s-20261010-aaaa0001.md", a.path),
            "---\nhollow_session: 1\nsession_key: \"s-20261010-aaaa0001\"\nagent: claude\n---\n",
        );
        let plan = plan(root, &a.id, &b.id).unwrap();
        let keys: Vec<&str> = plan.sessions.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["s-20261010-aaaa0001", "s-20261010-aaaa0002"]);
        assert!(plan.sessions[1].archived);
        merge(root, &a.id, &b.id, &plan.hash, None).unwrap();
        let b_cwd = workspaces::session_dir_for(root, &b.id).unwrap();
        for key in keys {
            let s = session_store::read_session(root, key).unwrap();
            assert_eq!(
                workspaces::session_dir(&s.summary.cwd),
                workspaces::session_dir(&b_cwd.to_string_lossy()),
                "{key}"
            );
        }
        let untouched = session_store::read_session(root, "s-20261010-bbbb0001").unwrap();
        assert_eq!(untouched.summary.cwd, other_cwd);
        assert!(!root
            .join(&a.path)
            .join("sessions/s-20261010-aaaa0001.md")
            .exists());
        assert!(root
            .join(&b.path)
            .join("sessions/s-20261010-aaaa0001.md")
            .exists());
    }

    #[test]
    fn undo_puts_back_files_conversations_sections_and_status() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b) = two(root);
        workspaces::update_meta(root, &a.id, None, Some("planning")).unwrap();
        write(root, &format!("{}/outputs/report.md", a.path), "A report");
        write(root, &format!("{}/sources/memo.pdf", a.path), "pdf");
        write(
            root,
            &format!("{}/sources/.provenance/memo.pdf.json", a.path),
            "{}",
        );
        write(root, &format!("{}/outputs/report.md", b.path), "B report");
        let a_cwd = workspaces::folder(root, &a.id)
            .unwrap()
            .to_string_lossy()
            .to_string();
        add_session(root, "s-20261010-eeee0001", &a_cwd, false);
        let before_into: Vec<String> = DOCS
            .iter()
            .map(|d| fs::read_to_string(root.join(&b.path).join(d)).unwrap())
            .collect();
        let plan = plan(root, &a.id, &b.id).unwrap();
        merge(root, &a.id, &b.id, &plan.hash, None).unwrap();
        // The merged source records the status it had, as any archive does (P1-38).
        let merged = workspaces::summary(root, &a.id).unwrap();
        assert_eq!(merged.archived_from.as_deref(), Some("planning"));

        let out = undo(root, &a.id, None).unwrap();
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert_eq!((out.files, out.sessions), (3, 1));
        assert_eq!(out.session_keys, ["s-20261010-eeee0001"]);
        let src = root.join(&a.path);
        assert_eq!(
            fs::read_to_string(src.join("outputs/report.md")).unwrap(),
            "A report"
        );
        assert_eq!(
            fs::read_to_string(src.join("sources/.provenance/memo.pdf.json")).unwrap(),
            "{}"
        );
        assert_eq!(
            fs::read_to_string(root.join(&b.path).join("outputs/report.md")).unwrap(),
            "B report"
        );
        assert!(!root.join(&b.path).join("outputs/report-merged.md").exists());
        let after_into: Vec<String> = DOCS
            .iter()
            .map(|d| fs::read_to_string(root.join(&b.path).join(d)).unwrap())
            .collect();
        assert_eq!(
            before_into, after_into,
            "the appended sections come out exactly"
        );
        let s = session_store::read_session(root, "s-20261010-eeee0001").unwrap();
        assert_eq!(
            workspaces::session_dir(&s.summary.cwd),
            workspaces::session_dir(&a_cwd)
        );
        let source = workspaces::summary(root, &a.id).unwrap();
        assert_eq!(
            (source.status.as_str(), source.merged_into),
            ("planning", None)
        );
        assert!(!fs::read_to_string(src.join("WORKSPACE.md"))
            .unwrap()
            .contains("archived_from"));
        assert!(!src.join(RECORD).exists() && !src.join(RECORD_NOTE).exists());
        assert!(undo(root, &a.id, None)
            .unwrap_err()
            .contains("no merge to undo"));
    }

    #[test]
    fn undo_leaves_what_changed_since_and_says_where() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b) = two(root);
        write(root, &format!("{}/outputs/one.md", a.path), "1");
        let plan = plan(root, &a.id, &b.id).unwrap();
        merge(root, &a.id, &b.id, &plan.hash, None).unwrap();
        // Someone edits the merged section and recreates the old file name.
        let brief = root.join(&b.path).join("WORKSPACE.md");
        let text = fs::read_to_string(&brief)
            .unwrap()
            .replace("Find the numbers", "Find the numbers (edited)");
        fs::write(&brief, text).unwrap();
        write(root, &format!("{}/outputs/one.md", a.path), "new");
        let out = undo(root, &a.id, None).unwrap();
        assert_eq!(out.files, 0);
        assert!(
            out.warnings
                .iter()
                .any(|w| w.contains("old name is taken again")),
            "{:?}",
            out.warnings
        );
        assert!(
            out.warnings.iter().any(|w| w.contains("edited since")),
            "{:?}",
            out.warnings
        );
        assert_eq!(
            fs::read_to_string(root.join(&b.path).join("outputs/one.md")).unwrap(),
            "1"
        );
        assert_eq!(
            fs::read_to_string(root.join(&a.path).join("outputs/one.md")).unwrap(),
            "new"
        );
    }

    #[test]
    fn a_changed_plan_or_a_bad_pair_is_refused_and_nothing_moves() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b) = two(root);
        write(root, &format!("{}/outputs/one.md", a.path), "1");
        let shown = plan(root, &a.id, &b.id).unwrap();
        write(root, &format!("{}/outputs/two.md", a.path), "2");
        let err = merge(root, &a.id, &b.id, &shown.hash, None).unwrap_err();
        assert!(err.contains("changed since it was shown"), "{err}");
        assert!(root.join(&a.path).join("outputs/one.md").exists());
        assert_eq!(workspaces::summary(root, &a.id).unwrap().status, "active");

        assert!(plan(root, &a.id, &a.id).unwrap_err().contains("itself"));
        assert!(plan(root, "quick-chats", &b.id)
            .unwrap_err()
            .contains("built in"));
        assert!(plan(root, &a.id, "main")
            .unwrap_err()
            .contains("can't receive"));
        assert!(plan(root, "../x", &b.id).is_err());
        let notion = workspaces::ensure_named(
            root,
            "notion-abc12300000000000000000000000000",
            "Mirror",
            "From Notion",
        )
        .unwrap();
        // A legacy Notion workspace merges either way since Notion two-way
        // sync was removed (2026-11-10).
        assert!(plan(root, &notion.id, &b.id).is_ok());
        assert!(plan(root, &a.id, &notion.id).is_ok());
        workspaces::update_meta(root, &b.id, None, Some("archived")).unwrap();
        assert!(plan(root, &a.id, &b.id)
            .unwrap_err()
            .contains("merge into it"));
        workspaces::update_meta(root, &a.id, None, Some("archived")).unwrap();
        assert!(plan(root, &a.id, &notion.id)
            .unwrap_err()
            .contains("archived"));
    }

    #[test]
    fn a_running_conversation_blocks_the_merge() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b) = two(root);
        let a_cwd = workspaces::folder(root, &a.id)
            .unwrap()
            .to_string_lossy()
            .to_string();
        add_session(root, "s-20261010-cccc0001", &a_cwd, false);
        crate::run_queue::enqueue(
            root,
            crate::run_queue::NewRow::new(
                crate::run_queue::RunSource::Chat,
                Some("s-20261010-cccc0001"),
                "Chat turn",
                "chat",
            ),
            chrono::Utc::now(),
        )
        .unwrap();
        let shown = plan(root, &a.id, &b.id).unwrap();
        let err = merge(root, &a.id, &b.id, &shown.hash, None).unwrap_err();
        assert!(err.contains("running or queued"), "{err}");
        let s = session_store::read_session(root, "s-20261010-cccc0001").unwrap();
        assert_eq!(s.summary.cwd, a_cwd);
    }

    /// P1-38: the preview says a merge is blocked before the confirm, and
    /// the hash doesn't change with it.
    #[test]
    fn the_preview_names_a_running_conversation_before_the_confirm() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b) = two(root);
        let a_cwd = workspaces::folder(root, &a.id)
            .unwrap()
            .to_string_lossy()
            .to_string();
        add_session(root, "s-20261010-cccc0002", &a_cwd, false);
        let idle = preview(root, &a.id, &b.id, None).unwrap();
        assert_eq!(idle.blocked, None);
        crate::run_queue::enqueue(
            root,
            crate::run_queue::NewRow::new(
                crate::run_queue::RunSource::Chat,
                Some("s-20261010-cccc0002"),
                "Chat turn",
                "chat",
            ),
            chrono::Utc::now(),
        )
        .unwrap();
        let busy = preview(root, &a.id, &b.id, None).unwrap();
        let why = busy.blocked.clone().unwrap_or_default();
        assert!(why.contains("running or queued"), "{why}");
        assert_eq!(busy.hash, idle.hash);
        assert_eq!(plan(root, &a.id, &b.id).unwrap().blocked, None);
    }

    #[test]
    fn nested_body_demotes_headings_outside_fences_only() {
        let text = "---\ntitle: x\n---\n\n# Top\n\n#### Four\n\n##### Five\n\n```\n# code\n```\n#hashtag\n";
        let out = nested_body(text);
        assert_eq!(
            out,
            "### Top\n\n###### Four\n\n**Five**\n\n```\n# code\n```\n#hashtag"
        );
    }

    #[test]
    fn replace_paths_matches_whole_relative_paths_once() {
        let pairs = |list: &[(&str, &str)]| {
            list.iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect::<Vec<_>>()
        };
        let text = "see sources/a.md, ./sources/a.md, sources/a.md.bak, xsources/a.md, \
                    ../other/sources/a.md, https://x.test/sources/a.md (sources/a.md)";
        assert_eq!(
            replace_paths(text, &pairs(&[("sources/a.md", "sources/a-merged.md")])),
            "see sources/a-merged.md, ./sources/a-merged.md, sources/a.md.bak, xsources/a.md, \
                    ../other/sources/a.md, https://x.test/sources/a.md (sources/a-merged.md)"
        );
        // One pass: a replacement is never replaced again.
        let chain = pairs(&[
            ("outputs/deep/x.csv", "outputs/deep-merged/x.csv"),
            (
                "outputs/deep-merged/x.csv",
                "outputs/deep-merged/x-merged.csv",
            ),
        ]);
        assert_eq!(
            replace_paths("a outputs/deep/x.csv b outputs/deep-merged/x.csv", &chain),
            "a outputs/deep-merged/x.csv b outputs/deep-merged/x-merged.csv"
        );
    }

    #[test]
    fn describe_lists_every_file_and_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b) = two(root);
        write(root, &format!("{}/outputs/a.md", a.path), "a");
        write(root, &format!("{}/outputs/a.md", b.path), "b");
        write(root, &format!("{}/sources/s.txt", a.path), "s");
        let a_cwd = workspaces::folder(root, &a.id)
            .unwrap()
            .to_string_lossy()
            .to_string();
        add_session(root, "s-20261010-dddd0001", &a_cwd, true);
        let lines = describe(&plan(root, &a.id, &b.id).unwrap());
        let text = lines.join("\n");
        assert!(text.contains("Files that move (2):"), "{text}");
        assert!(
            text.contains("- outputs/a.md → outputs/a-merged.md (name taken)"),
            "{text}"
        );
        assert!(text.contains("- sources/s.txt"), "{text}");
        assert!(
            text.contains("Conversations that move (1):\n- Test session (archived)"),
            "{text}"
        );
    }

    #[test]
    fn a_recorded_merge_must_be_undone_before_another() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b) = two(root);
        let c = workspaces::create(root, "Third", "x").unwrap();
        write(root, &format!("{}/outputs/one.md", a.path), "1");
        // A merge that stopped part-way leaves its record and merged_into,
        // but not the archived status.
        let plan = plan(root, &a.id, &b.id).unwrap();
        merge(root, &a.id, &b.id, &plan.hash, None).unwrap();
        workspaces::update_meta(root, &a.id, None, Some("active")).unwrap();
        let err = super::plan(root, &a.id, &c.id).unwrap_err();
        assert!(err.contains("Undo merge first"), "{err}");
        undo(root, &a.id, None).unwrap();
        assert!(super::plan(root, &a.id, &c.id).is_ok());
    }

    #[test]
    fn a_tampered_record_moves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b) = two(root);
        write(root, &format!("{}/outputs/one.md", a.path), "1");
        write(root, "notes/private.md", "private");
        let plan = plan(root, &a.id, &b.id).unwrap();
        merge(root, &a.id, &b.id, &plan.hash, None).unwrap();
        let path = root.join(&a.path).join(RECORD);
        let mut record: Record = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        record.files.push(MergeFile {
            from: format!("workspaces/{}/outputs/stolen.md", a.id),
            to: "notes/private.md".into(),
            renamed: true,
        });
        fs::write(&path, serde_json::to_string(&record).unwrap()).unwrap();
        let err = undo(root, &a.id, None).unwrap_err();
        assert!(err.contains("outside the two workspaces"), "{err}");
        assert_eq!(
            fs::read_to_string(root.join("notes/private.md")).unwrap(),
            "private"
        );
        assert!(root.join(&b.path).join("outputs/one.md").exists());
    }

    #[test]
    fn undo_removes_docs_the_merge_created_and_folders_it_made() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b) = two(root);
        fs::remove_file(root.join(&b.path).join("PLAN.md")).unwrap();
        write(root, &format!("{}/outputs/deep/x.csv", a.path), "x");
        write(root, &format!("{}/outputs/deep", b.path), "B file");
        let plan = plan(root, &a.id, &b.id).unwrap();
        merge(root, &a.id, &b.id, &plan.hash, None).unwrap();
        assert!(root.join(&b.path).join("PLAN.md").exists());
        assert!(root.join(&b.path).join("outputs/deep-merged").is_dir());
        let out = undo(root, &a.id, None).unwrap();
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert!(!root.join(&b.path).join("PLAN.md").exists());
        assert!(!root.join(&b.path).join("outputs/deep-merged").exists());
        assert_eq!(
            fs::read_to_string(root.join(&a.path).join("outputs/deep/x.csv")).unwrap(),
            "x"
        );
    }
}
