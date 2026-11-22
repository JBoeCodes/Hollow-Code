//! Move Hollow home: staging, the journaled run, verify and rollback.
//!
//! The run executes exactly `plan.json` as a list of steps ([`ops`]). Each
//! step writes `started` then `done` (or `failed`) to `journal.jsonl` in the
//! backup folder, is idempotent (it checks the state it would leave before
//! acting), and has an undo that reads the file system, not memory, so a
//! rollback works from any crash point. A step that fails rolls back
//! everything done so far, then the helper relaunches Hollow.
//!
//! Before the first change the run snapshots (APFS clones, else copies)
//! every file it rewrites, the git worktree pointer files of every repaired
//! repository and `git worktree list --porcelain` per repository. The
//! Sessions database is never edited in place: a clone is checkpointed,
//! rewritten, checked (`quick_check`, row counts) and swapped in under the
//! new stem; the original trio moves into the backup folder, never deleted.

use crate::move_home::{Check, Plan, Probes, Roots, Status, Step, METHOD_COPY, SF_DATALESS};
use crate::move_home_icloud as icloud;
use crate::move_home_plan::{
    self as mp, current_froms, read_checked, s, sessions_db, sidecar, FILE_CLAUDE_DESKTOP,
    FILE_CLAUDE_JSON, FILE_CODEX, FILE_CONFIG, FILE_GEMINI, FILE_LAUNCH_AGENT, FILE_NOTE,
    FILE_OBSIDIAN, FILE_SCRIPT, FILE_STORE, WHAT_SESSIONS,
};
use crate::move_home_rekey as rekey;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

const PLAN_FILE: &str = "plan.json";
const ROOTS_FILE: &str = "roots.json";
const MANIFEST_FILE: &str = "manifest.json";
const JOURNAL_FILE: &str = "journal.jsonl";
const SNAPSHOT_FILE: &str = "snapshot.json";
const VERIFY_FILE: &str = "verify.json";
const LOCK_FILE: &str = "helper.lock";
const TMP_SUFFIX: &str = ".move-home-tmp";

/// Test hooks: stop the run as if killed.
#[derive(Debug, Clone, Default)]
pub struct RunHooks {
    /// Stop after this many steps completed (no final record).
    pub stop_after: Option<usize>,
    /// With `stop_after`: also write `started` for the next step, as if
    /// killed inside it before it changed anything.
    pub crash_mid: bool,
    /// Kill the Sessions step between moving the original trio out and
    /// moving the rewritten copy in.
    pub kill_in_sessions_swap: bool,
    /// Kill a rollback after it undid the steps, before it restored files.
    pub kill_before_restore: bool,
    /// Kill the Sessions step after only the original main file moved out
    /// (its `-wal` and `-shm` still in place).
    pub kill_after_first_original: bool,
    /// Kill a rollback's Sessions undo after the copy's `-wal`/`-shm` went
    /// aside, before its main file did.
    pub kill_rollback_before_copy_main: bool,
    /// Kill a rollback's Sessions undo after the original `-wal`/`-shm`
    /// came back, before the original main file did.
    pub kill_rollback_before_original_main: bool,
    /// Kill file step `n` right after its write landed, before `done`.
    pub kill_after_write: Option<usize>,
    /// Kill a failed run after it journaled the failure, before its
    /// automatic rollback began.
    pub kill_before_auto_rollback: bool,
    /// Kill a copy move right after its copy went into place, before `done`.
    pub kill_after_copy_rename: bool,
    /// Kill the retire step after it removed part of the original.
    pub kill_in_retire: bool,
    /// Kill a rollback of a copy move after it rebuilt the original, before
    /// it removed the copy.
    pub kill_in_rollback_copy: bool,
    /// Fail (a real failure, rolled back) inside the Sessions re-key, after
    /// the first column was rewritten in the copy: what run 2 of 11-13 hit.
    pub fail_in_sessions_rekey: bool,
    /// Fail the step with this name (a real failure, rolled back) before it
    /// runs.
    pub fail_step: Option<String>,
}

/// What a test hook's kill returns (never a real failure: nothing is undone).
pub const KILLED: &str = "stopped (test)";

/// sha256 of `bytes`, hex.
fn sha_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(bytes))
}

fn sha_file(p: &Path) -> Option<String> {
    std::fs::read(p).ok().map(|b| sha_hex(&b))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    version: u32,
    created: String,
    plan_hash: String,
    caller: Option<String>,
    options: crate::move_home::Options,
    backup_dir: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapEntry {
    /// Original path.
    path: String,
    /// The copy in the backup folder.
    copy: String,
    mode: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    files: Vec<SnapEntry>,
    /// Per repo (plan order): its worktree folders (original paths).
    worktrees: Vec<Vec<String>>,
    /// Per repo: `git worktree list --porcelain` before.
    lists: Vec<String>,
    /// File count per final root before the move.
    files_before: u64,
    bytes_before: u64,
    counted: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Verified {
    at_nanos: u128,
    at: String,
    sessions: i64,
    turns_max: i64,
    new_db: String,
    vault: String,
}

pub(crate) struct Staged {
    pub backup_dir: PathBuf,
    pub plan_file: PathBuf,
}

fn now() -> String {
    chrono::Local::now().to_rfc3339()
}

fn nanos(t: SystemTime) -> u128 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    write_atomic(path, text.as_bytes(), None)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Write through a sibling temp file, fsync, keep `mode` (or the old file's),
/// rename over.
fn write_atomic(path: &Path, bytes: &[u8], mode: Option<u32>) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let tmp = sidecar(path, TMP_SUFFIX);
    let mode = mode.or_else(|| {
        std::fs::metadata(path)
            .ok()
            .map(|m| m.permissions().mode() & 0o7777)
    });
    let mut f = std::fs::File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    f.write_all(bytes)
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    if let Some(m) = mode {
        f.set_permissions(std::fs::Permissions::from_mode(m))
            .map_err(|e| e.to_string())?;
    }
    f.sync_all().map_err(|e| e.to_string())?;
    drop(f);
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// [`clone_file`] for the planner.
pub(crate) fn clone_or_copy(src: &Path, dst: &Path) -> Result<(), String> {
    clone_file(src, dst)
}

/// An APFS clone of `src` at `dst`, else a copy.
fn clone_file(src: &Path, dst: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        if let (Ok(a), Ok(b)) = (
            CString::new(src.as_os_str().as_bytes()),
            CString::new(dst.as_os_str().as_bytes()),
        ) {
            // SAFETY: two valid NUL-terminated paths.
            if unsafe { libc::clonefile(a.as_ptr(), b.as_ptr(), 0) } == 0 {
                return Ok(());
            }
        }
    }
    std::fs::copy(src, dst)
        .map(|_| ())
        .map_err(|e| format!("copy {} to {}: {e}", src.display(), dst.display()))
}

fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::symlink_metadata(path)
        .map(|m| m.permissions().mode() & 0o7777)
        .unwrap_or(0o644)
}

fn exists(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

fn is_link_to(p: &Path, target: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink())
        && std::fs::read_link(p).is_ok_and(|t| t == target)
}

fn rename(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::rename(from, to)
        .map_err(|e| format!("rename {} to {}: {e}", from.display(), to.display()))
}

// ── Journal ───────────────────────────────────────────────────────────────

struct Journal {
    path: PathBuf,
}

impl Journal {
    fn new(dir: &Path) -> Journal {
        Journal {
            path: dir.join(JOURNAL_FILE),
        }
    }

    fn read(&self) -> Vec<Step> {
        std::fs::read_to_string(&self.path)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    fn append(&self, step: &str, state: &str, detail: &str) -> Result<(), String> {
        use std::io::Write;
        let line = serde_json::to_string(&Step {
            step: step.into(),
            state: state.into(),
            detail: detail.into(),
            at: now(),
        })
        .map_err(|e| e.to_string())?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| format!("journal: {e}"))?;
        writeln!(f, "{line}").map_err(|e| format!("journal: {e}"))?;
        f.sync_all().map_err(|e| format!("journal: {e}"))
    }

    /// Every step a record ever showed as started, writing, done or failed
    /// (an `undone` later doesn't erase it: a rollback killed after marking
    /// steps undone must still restore their files on the next try).
    fn touched(&self) -> std::collections::BTreeSet<String> {
        self.read()
            .into_iter()
            .filter(|s| matches!(s.state.as_str(), "started" | "writing" | "done" | "failed"))
            .map(|s| s.step)
            .collect()
    }

    /// The detail of a step's last record that did something (not `undone`
    /// or `writing`): what its undo needs.
    fn last_detail(&self, step: &str) -> String {
        self.read()
            .into_iter()
            .rev()
            .filter(|s| s.step == step && matches!(s.state.as_str(), "done" | "failed" | "started"))
            .find(|s| !s.detail.is_empty())
            .map(|s| s.detail)
            .unwrap_or_default()
    }

    /// A file step's recorded write: (sha256 of the bytes written, the copy
    /// of the bytes it replaced, their mode). Read from any record of it.
    fn file_write(&self, step: &str) -> (Option<String>, Option<String>, Option<u32>) {
        let (mut sha, mut pre, mut mode) = (None, None, None);
        for s in self.read().into_iter().filter(|s| s.step == step) {
            let v: serde_json::Value = serde_json::from_str(&s.detail).unwrap_or_default();
            if let Some(x) = v["sha"].as_str() {
                sha = Some(x.to_string());
            }
            if let Some(x) = v["pre"].as_str() {
                pre = Some(x.to_string());
            }
            if let Some(x) = v["mode"].as_u64() {
                mode = Some(x as u32);
            }
        }
        (sha, pre, mode)
    }

    /// Whether a `writing` record of the step carries `key` (a copy move
    /// marks that its copy went into place).
    fn marked(&self, step: &str, key: &str) -> bool {
        self.marker(step, key).is_some()
    }

    /// The value of `key` in the last `writing` record of the step that has it.
    fn marker(&self, step: &str, key: &str) -> Option<serde_json::Value> {
        self.read()
            .iter()
            .rev()
            .filter(|s| s.step == step && s.state == "writing")
            .find_map(|s| {
                serde_json::from_str::<serde_json::Value>(&s.detail)
                    .ok()
                    .and_then(|v| v.get(key).cloned())
            })
    }

    /// Whether any record shows the step finished.
    fn completed(&self, step: &str) -> bool {
        self.read()
            .iter()
            .any(|s| s.step == step && s.state == "done")
    }

    /// Whether the step's last record says a rollback already undid it.
    fn undone(&self, step: &str) -> bool {
        self.read()
            .into_iter()
            .rev()
            .find(|s| s.step == step)
            .is_some_and(|s| s.state == "undone")
    }

    /// The last `run` or `rollback` record that settles the state (a refused
    /// rollback changed nothing and doesn't).
    fn last_outcome(&self) -> Option<Step> {
        self.read()
            .into_iter()
            .rev()
            .find(|s| s.step == "run" || (s.step == "rollback" && s.state != "refused"))
    }

    /// The last state and detail of each step.
    fn states(&self) -> BTreeMap<String, (String, String)> {
        let mut out = BTreeMap::new();
        for s in self.read() {
            out.insert(s.step.clone(), (s.state.clone(), s.detail.clone()));
        }
        out
    }
}

// ── Steps ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Op {
    Preflight,
    Snapshot,
    Probe,
    Bootout(usize),
    Mkdir(usize),
    Move(usize),
    Workspace(usize),
    Store(usize),
    Sessions,
    File(usize),
    Git(usize),
    Bootstrap(usize),
    Verify,
    /// Remove the original of a folder that moved by copy, last.
    Retire(usize),
}

impl Op {
    pub(crate) fn name(&self) -> String {
        match self {
            Op::Preflight => "preflight".into(),
            Op::Snapshot => "snapshot".into(),
            Op::Probe => "probe".into(),
            Op::Bootout(i) => format!("bootout:{i}"),
            Op::Mkdir(i) => format!("mkdir:{i}"),
            Op::Move(i) => format!("move:{i}"),
            Op::Workspace(i) => format!("workspace:{i}"),
            Op::Store(i) => format!("store:{i}"),
            Op::Sessions => "sessions".into(),
            Op::File(i) => format!("file:{i}"),
            Op::Git(i) => format!("git:{i}"),
            Op::Bootstrap(i) => format!("bootstrap:{i}"),
            Op::Verify => "verify".into(),
            Op::Retire(i) => format!("retire:{i}"),
        }
    }
}

/// The plan's steps in run order. A folder to make runs just before the
/// first move into it (so `Projects/Personal` comes after the Projects
/// folder moved); the rest after the moves.
pub(crate) fn ops(plan: &Plan) -> Vec<Op> {
    let mut out = vec![Op::Preflight, Op::Snapshot, Op::Probe];
    for (i, l) in plan.launch_agents.iter().enumerate() {
        if l.on_plan && !l.mentions.is_empty() {
            out.push(Op::Bootout(i));
        }
    }
    let mut made = vec![false; plan.mkdirs.len()];
    for (i, m) in plan.moves.iter().enumerate() {
        let parent = Path::new(&m.to)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        for (j, d) in plan.mkdirs.iter().enumerate() {
            if !made[j] && parent.starts_with(d) {
                made[j] = true;
                out.push(Op::Mkdir(j));
            }
        }
        out.push(Op::Move(i));
    }
    for (j, done) in made.iter().enumerate() {
        if !done {
            out.push(Op::Mkdir(j));
        }
    }
    for (i, id) in plan.ids.iter().enumerate() {
        if id.has_workspace {
            out.push(Op::Workspace(i));
        }
    }
    for (i, st) in plan.stores.iter().enumerate() {
        if st.what != WHAT_SESSIONS {
            out.push(Op::Store(i));
        }
    }
    // The Sessions database is rewritten when it is renamed to the new stem,
    // and also in place when the stem stays (the vault was already home) but
    // its path or workspace id columns change.
    if plan.stores.iter().any(|s| s.what == WHAT_SESSIONS)
        || plan.columns.iter().any(|c| c.rows > 0)
        || !plan.ids.is_empty()
    {
        out.push(Op::Sessions);
    }
    for i in 0..plan.files.len() {
        out.push(Op::File(i));
    }
    for (i, r) in plan.repos.iter().enumerate() {
        if r.skipped.is_none() {
            out.push(Op::Git(i));
        }
    }
    for (i, l) in plan.launch_agents.iter().enumerate() {
        if l.on_plan && !l.mentions.is_empty() {
            out.push(Op::Bootstrap(i));
        }
    }
    out.push(Op::Verify);
    // A folder moved by copy keeps its original until everything else, the
    // checks included, went through: a failure before this point rolls back
    // by removing the copy, never by rebuilding the original in iCloud.
    for (i, m) in plan.moves.iter().enumerate() {
        if m.method == METHOD_COPY {
            out.push(Op::Retire(i));
        }
    }
    out
}

struct Run<'a> {
    dir: PathBuf,
    plan: Plan,
    roots: Roots,
    manifest: Manifest,
    probes: &'a dyn Probes,
    journal: Journal,
    maps: Vec<(String, String)>,
    ids: BTreeMap<String, String>,
    hooks: RunHooks,
}

impl Run<'_> {
    fn load<'p>(dir: &Path, probes: &'p dyn Probes) -> Result<Run<'p>, String> {
        let plan: Plan = read_json(&dir.join(PLAN_FILE))?;
        let roots: Roots = read_json(&dir.join(ROOTS_FILE))?;
        let manifest: Manifest = read_json(&dir.join(MANIFEST_FILE))?;
        if manifest.plan_hash != plan.hash {
            return Err("The plan file doesn't match its manifest; nothing was changed.".into());
        }
        let maps = rekey::sorted_mappings(&plan.mappings);
        let ids = plan
            .ids
            .iter()
            .map(|i| (i.old.clone(), i.new.clone()))
            .collect();
        Ok(Run {
            dir: dir.to_path_buf(),
            journal: Journal::new(dir),
            plan,
            roots,
            manifest,
            probes,
            maps,
            ids,
            hooks: RunHooks::default(),
        })
    }

    /// Where an original path is after the whole move.
    fn final_path(&self, original: &str) -> PathBuf {
        PathBuf::from(rekey::whole_or_same(original, &self.maps))
    }

    fn new_vault(&self) -> PathBuf {
        PathBuf::from(&self.plan.vault_to)
    }

    fn vault_now(&self) -> PathBuf {
        if self
            .plan
            .moves
            .iter()
            .any(|m| m.from == self.plan.vault_from)
        {
            self.new_vault()
        } else {
            PathBuf::from(&self.plan.vault_from)
        }
    }

    // ── do ────────────────────────────────────────────────────────────

    fn exec(&self, op: &Op, wait_pid: Option<i32>) -> Result<String, String> {
        match op {
            Op::Preflight => self.preflight(wait_pid),
            Op::Snapshot => self.snapshot(),
            Op::Probe => self.rename_probe(),
            Op::Bootout(i) => {
                let l = &self.plan.launch_agents[*i];
                self.probes.bootout(&l.label).map(|_| String::new())
            }
            Op::Mkdir(i) => {
                let d = Path::new(&self.plan.mkdirs[*i]);
                if exists(d) {
                    Ok("existed".into())
                } else {
                    std::fs::create_dir(d).map_err(|e| format!("make {}: {e}", d.display()))?;
                    Ok("created".into())
                }
            }
            Op::Move(i) => self.do_move(*i),
            Op::Retire(i) => self.retire(*i),
            Op::Workspace(i) => {
                let id = &self.plan.ids[*i];
                let ws = self.vault_now().join("workspaces");
                let (from, to) = (ws.join(&id.old), ws.join(&id.new));
                if !exists(&from) && exists(&to) {
                    return Ok("already renamed".into());
                }
                if exists(&to) {
                    return Err(format!("{} already exists", to.display()));
                }
                rename(&from, &to).map(|_| String::new())
            }
            Op::Store(i) => {
                let st = &self.plan.stores[*i];
                let (from, to) = (Path::new(&st.from), Path::new(&st.to));
                if !exists(from) && exists(to) {
                    return Ok("already renamed".into());
                }
                if !exists(from) {
                    return Ok("gone before the move; skipped".into());
                }
                if exists(to) {
                    return Err(format!("{} already exists", to.display()));
                }
                rename(from, to).map(|_| String::new())
            }
            Op::Sessions => self.sessions(),
            Op::File(i) => self.rewrite_file(*i),
            Op::Git(i) => self.git_repair(*i),
            Op::Bootstrap(i) => {
                let l = &self.plan.launch_agents[*i];
                self.probes
                    .bootstrap(Path::new(&l.path))
                    .map(|_| String::new())
            }
            Op::Verify => self.verify(),
        }
    }

    fn preflight(&self, wait_pid: Option<i32>) -> Result<String, String> {
        if let Some(pid) = wait_pid {
            wait_for_exit(pid, Duration::from_secs(180))?;
        }
        let db = sessions_db(&self.roots, &self.plan.old_stem);
        let deadline = Instant::now() + Duration::from_secs(60);
        while db.exists() && crate::hollow_databases::shm_in_use(&db) {
            if Instant::now() > deadline {
                return Err("Hollow's Sessions database is still open in another process; nothing was changed.".into());
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        let live = mp::live_queue(&db, self.manifest.caller.as_deref())?;
        // A turn Hollow ended at quit may still read `running` until the next
        // launch's recovery; only queued or claimed rows of other chats, or
        // rows the plan didn't see, matter. Any live row refuses.
        if !live.is_empty() {
            let names: Vec<String> = live.iter().map(|(l, st)| format!("{l} ({st})")).collect();
            return Err(format!(
                "Chats are still running or queued: {}. Nothing was changed.",
                names.join(", ")
            ));
        }
        // Claude Code and Codex rewrite their own settings files
        // (`~/.claude.json`, `~/.codex/config.toml`) at any moment; none may
        // run while the move rewrites them. Hollow's own children end with it.
        // After a quit, Hollow's children may take a moment to end.
        let wait = if wait_pid.is_some() { 30 } else { 0 };
        let deadline = Instant::now() + Duration::from_secs(wait);
        loop {
            let clis = self.probes.agent_clis(wait_pid.map(|p| p as u32));
            if clis.is_empty() {
                break;
            }
            if Instant::now() > deadline {
                return Err(format!(
                    "Quit these first, they rewrite their own settings: {}. Nothing was changed.",
                    clis.join(", ")
                ));
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        // Top-level targets are free (a link or an empty folder is replaced).
        let froms = current_froms(&self.plan.moves);
        for (i, m) in self.plan.moves.iter().enumerate() {
            let nested = self
                .plan
                .moves
                .iter()
                .any(|o| o != m && m.to.starts_with(&format!("{}/", o.to)));
            if nested || self.move_done(i, &froms) {
                continue;
            }
            let to = Path::new(&m.to);
            if let Ok(meta) = std::fs::symlink_metadata(to) {
                let empty =
                    meta.is_dir() && std::fs::read_dir(to).is_ok_and(|mut d| d.next().is_none());
                if !meta.file_type().is_symlink() && !empty && !case_only(&froms[i], &m.to) {
                    return Err(format!("{} is taken; nothing was changed.", to.display()));
                }
            }
            if mp::dev_of(Path::new(&froms[i])) != mp::dev_of(to.parent().unwrap_or(to)) {
                return Err(format!(
                    "{} is on another volume; nothing was changed.",
                    m.from
                ));
            }
        }
        let new_db = sessions_db(&self.roots, &self.plan.new_stem);
        if self.plan.old_stem != self.plan.new_stem
            && exists(&new_db)
            && !exists(&self.dir.join("sessions-original"))
        {
            return Err(format!(
                "{} already exists; nothing was changed.",
                new_db.display()
            ));
        }
        Ok(format!(
            "pid {} gone; Sessions database closed",
            wait_pid.unwrap_or(0)
        ))
    }

    /// Clone every file the plan rewrites, the git pointer files and lists.
    fn snapshot(&self) -> Result<String, String> {
        let snap_dir = self.dir.join("snapshot");
        std::fs::create_dir_all(&snap_dir).map_err(|e| e.to_string())?;
        let mut snap = Snapshot::default();
        let mut paths: Vec<String> = self.plan.files.iter().map(|f| f.path.clone()).collect();
        // Git pointer files (originals), and each repo's worktree list.
        for r in &self.plan.repos {
            if r.skipped.is_some() {
                snap.worktrees.push(Vec::new());
                snap.lists.push(String::new());
                continue;
            }
            let main = Path::new(&r.repo_from);
            let wts = mp::worktrees_of(main, self.probes).unwrap_or_default();
            for (folder, gitdir) in &wts {
                paths.push(s(gitdir));
                let dotgit = folder.join(".git");
                if std::fs::symlink_metadata(&dotgit).is_ok_and(|m| m.is_file()) {
                    paths.push(s(&dotgit));
                }
            }
            snap.worktrees.push(wts.iter().map(|(f, _)| s(f)).collect());
            snap.lists
                .push(git_list(main, &self.roots.home).unwrap_or_default());
        }
        paths.sort();
        paths.dedup();
        for (n, p) in paths.iter().enumerate() {
            let path = Path::new(p);
            let Ok(meta) = std::fs::symlink_metadata(path) else {
                continue;
            };
            if !meta.file_type().is_file() || self.probes.flags(path, &meta) & SF_DATALESS != 0 {
                continue;
            }
            let copy = snap_dir.join(format!("{n:05}"));
            if !exists(&copy) {
                clone_file(path, &copy)?;
            }
            snap.files.push(SnapEntry {
                path: p.clone(),
                copy: s(&copy),
                mode: mode_of(path),
            });
        }
        // File counts for verify.
        let deadline = Instant::now() + Duration::from_secs(600);
        let mut partial = false;
        for m in top_level_froms(&self.plan) {
            let w = mp::walk(Path::new(&m), self.probes, deadline);
            snap.files_before += w.files;
            snap.bytes_before += w.bytes;
            partial |= w.partial;
        }
        snap.counted = !partial;
        write_json(&self.dir.join(SNAPSHOT_FILE), &snap)?;
        write_json(&self.dir.join("ids.json"), &self.plan.ids)?;
        let links: Vec<(String, Option<String>)> = self
            .plan
            .moves
            .iter()
            .map(|m| (m.to.clone(), m.replaces_symlink.clone()))
            .collect();
        write_json(&self.dir.join("symlinks.json"), &links)?;
        Ok(format!("{} files", snap.files.len()))
    }

    /// The same kind of move the run will do, on a small tree in each
    /// source folder: a folder in iCloud is cloned out, compared and removed
    /// (a file rename passed on 2026-11-12 and the real folder rename then
    /// failed after 9 minutes); other folders get a file rename. A folder
    /// that moves by copy must also have no iCloud-only file left.
    fn rename_probe(&self) -> Result<String, String> {
        let hollow = Path::new(&self.plan.hollow_home);
        let pid = std::process::id();
        let mut n = 0;
        let mut sources: Vec<(&str, bool)> = self
            .plan
            .moves
            .iter()
            .map(|m| (m.from.as_str(), m.method == METHOD_COPY))
            .collect();
        sources.sort();
        sources.dedup_by(|a, b| {
            if a.0 == b.0 {
                b.1 |= a.1;
                true
            } else {
                false
            }
        });
        for (k, (src, copy)) in sources.iter().enumerate() {
            let dir = Path::new(src);
            if !std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir()) {
                continue;
            }
            if *copy {
                let ev = icloud::count_evicted(dir, self.probes);
                if ev.files + ev.dirs > 0 {
                    return Err(format!(
                        "{src} still has {} iCloud-only files ({}) and {} iCloud-only folders; copying one downloads it alone and waits. Download them first with `Hollow move-home --download`, then start the move again. Nothing was changed.",
                        ev.files,
                        mp::size(ev.bytes),
                        ev.dirs
                    ));
                }
                let special = icloud::special_files(dir)
                    .map_err(|e| format!("Couldn't list {src} ({e}); nothing was changed."))?;
                if let Some(why) = special.refusal() {
                    return Err(format!("{why}. Nothing was changed."));
                }
                icloud::clone_probe(dir, hollow, &format!("{pid}-{k}")).map_err(|e| {
                    format!("A copy out of {src} failed ({e}); nothing was changed.")
                })?;
                n += 1;
                continue;
            }
            let a = dir.join(format!(".hollow-move-probe-{pid}-{k}"));
            let b = hollow.join(format!(".hollow-move-probe-{pid}-{k}"));
            std::fs::write(&a, b"probe").map_err(|e| {
                format!("Couldn't write a probe file in {src}: {e}. Nothing was changed.")
            })?;
            let result = rename(&a, &b).and_then(|_| rename(&b, &a));
            let _ = std::fs::remove_file(&a);
            let _ = std::fs::remove_file(&b);
            result
                .map_err(|e| format!("A rename out of {src} failed ({e}); nothing was changed."))?;
            n += 1;
        }
        Ok(format!("{n} folders"))
    }

    /// Whether move `i` already happened.
    fn move_done(&self, i: usize, froms: &[String]) -> bool {
        let m = &self.plan.moves[i];
        let (from, to) = (Path::new(&froms[i]), Path::new(&m.to));
        if case_only(&froms[i], &m.to) {
            return actual_name(to).as_deref() == to.file_name().and_then(|n| n.to_str());
        }
        let to_real = std::fs::symlink_metadata(to).is_ok_and(|m| !m.file_type().is_symlink());
        if m.method == METHOD_COPY {
            // The copy is in place and was compared; the original stays
            // until the retire step.
            return to_real && self.journal.marked(&Op::Move(i).name(), "copied");
        }
        to_real && (!exists(from) || is_link_to(from, to))
    }

    fn do_move(&self, i: usize) -> Result<String, String> {
        let froms = current_froms(&self.plan.moves);
        let m = &self.plan.moves[i];
        let (from, to) = (PathBuf::from(&froms[i]), PathBuf::from(&m.to));
        let nested = froms[i] != m.from;
        let mut detail = serde_json::Map::new();
        if !self.move_done(i, &froms) {
            if m.method == METHOD_COPY {
                self.refuse_evicted(&from)?;
            }
            if !case_only(&froms[i], &m.to) {
                if let Ok(meta) = std::fs::symlink_metadata(&to) {
                    if meta.file_type().is_symlink() {
                        let target = std::fs::read_link(&to).map(|t| s(&t)).unwrap_or_default();
                        std::fs::remove_file(&to)
                            .map_err(|e| format!("remove the link {}: {e}", to.display()))?;
                        detail.insert("replacedLink".into(), serde_json::json!(target));
                    } else if meta.is_dir()
                        && std::fs::read_dir(&to).is_ok_and(|mut d| d.next().is_none())
                    {
                        std::fs::remove_dir(&to).map_err(|e| {
                            format!("replace the empty folder {}: {e}", to.display())
                        })?;
                        detail.insert("replacedEmpty".into(), serde_json::json!(true));
                    } else {
                        return Err(format!("{} is taken", to.display()));
                    }
                }
            }
            if m.method == METHOD_COPY {
                self.copy_into_place(i, &from, &to, &mut detail)?;
            } else {
                rename(&from, &to)?;
            }
        }
        if m.compat_symlink
            && m.method != METHOD_COPY
            && !nested
            && !case_only(&froms[i], &m.to)
            && !exists(&from)
        {
            std::os::unix::fs::symlink(&to, &from)
                .map_err(|e| format!("leave a link at {}: {e}", from.display()))?;
            detail.insert("linked".into(), serde_json::json!(true));
        }
        Ok(serde_json::Value::Object(detail).to_string())
    }

    /// Clone `from` beside `to`, compare the clone with the original entry by
    /// entry, rename it into place and mark that in the journal. The original
    /// is untouched (the retire step removes it last).
    fn copy_into_place(
        &self,
        i: usize,
        from: &Path,
        to: &Path,
        detail: &mut serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), String> {
        let tmp = sidecar(to, TMP_SUFFIX);
        // Special files first (the probe checked too; one may have appeared
        // since): a known throwaway one is left out, any other stops the
        // copy before it starts. The source is only read.
        let special = icloud::special_files(from)?;
        if let Some(why) = special.refusal() {
            return Err(why);
        }
        icloud::remove_tree(&tmp)?;
        let started = nanos(SystemTime::now());
        let how = icloud::clone_tree(from, &tmp)?;
        let dropped = match icloud::drop_throwaway_specials(&tmp) {
            Ok(n) => n,
            Err(e) => {
                let _ = icloud::remove_tree(&tmp);
                return Err(e);
            }
        };
        let stats = match icloud::compare_trees(from, &tmp, false) {
            Ok(st) => st,
            Err(e) => {
                let _ = icloud::remove_tree(&tmp);
                return Err(format!(
                    "the copy of {} doesn't match the original ({e})",
                    from.display()
                ));
            }
        };
        if let Err(e) = rename(&tmp, to) {
            let _ = icloud::remove_tree(&tmp);
            return Err(e);
        }
        let note = serde_json::json!({
            "copied": true, "how": how, "files": stats.files, "dirs": stats.dirs,
            "links": stats.links, "bytes": stats.bytes, "atNanos": started.to_string(),
            "throwawaySpecial": special.throwaway.len(), "droppedFromCopy": dropped
        });
        self.journal
            .append(&Op::Move(i).name(), "writing", &note.to_string())?;
        detail.insert("copied".into(), note);
        if self.hooks.kill_after_copy_rename {
            return Err(KILLED.into());
        }
        Ok(())
    }

    /// When the move's checks finished (nanoseconds), if they did.
    fn verified_at(&self) -> Option<u128> {
        read_json::<Verified>(&self.dir.join("verified.json"))
            .ok()
            .map(|v| v.at_nanos)
    }

    /// A copy folder may not hold an iCloud-only file or folder: cloning one
    /// downloads it alone and waits.
    fn refuse_evicted(&self, from: &Path) -> Result<(), String> {
        let ev = icloud::count_evicted(from, self.probes);
        if ev.files + ev.dirs > 0 {
            return Err(format!(
                "{} has {} iCloud-only files and {} iCloud-only folders; copying one downloads it alone and waits. Run `Hollow move-home --download` first (with the options of the check)",
                from.display(),
                ev.files,
                ev.dirs
            ));
        }
        Ok(())
    }

    /// Remove the original of a folder that moved by copy, then leave the
    /// link at its old path. Never fatal: when the checks failed, when
    /// something was written to the original after the copy was made (an
    /// iCloud change from another Mac, a script the plan didn't rewrite), or
    /// when the removal fails partway, the step is done with a `kept` or
    /// `error` detail and the run ends with a warning, so one folder's
    /// problem doesn't roll back folders already finished.
    fn retire(&self, i: usize) -> Result<String, String> {
        let froms = current_froms(&self.plan.moves);
        let m = &self.plan.moves[i];
        let (from, to) = (PathBuf::from(&froms[i]), PathBuf::from(&m.to));
        let step = Op::Move(i).name();
        let kept = |why: String| Ok(serde_json::json!({ "kept": why }).to_string());
        let verify_failed = self
            .journal
            .states()
            .get("verify")
            .is_some_and(|(_, d)| d.contains("failed"));
        if verify_failed {
            return kept("the checks failed".into());
        }
        if is_link_to(&from, &to) {
            return Ok(serde_json::json!({ "removed": true, "already": true }).to_string());
        }
        let to_real = std::fs::symlink_metadata(&to).is_ok_and(|x| !x.file_type().is_symlink());
        if !to_real || !self.journal.marked(&step, "copied") {
            return kept(format!("the copy at {} isn't in place", to.display()));
        }
        if let Some(at) = self
            .journal
            .marker(&step, "atNanos")
            .and_then(|v| v.as_str().and_then(|s| s.parse::<u128>().ok()))
        {
            if let Some(p) = icloud::modified_since(&from, at) {
                return kept(format!(
                    "{} was written after the copy was made, so the original may hold newer work than the copy",
                    p.display()
                ));
            }
        }
        // Something that arrived after the clone with an old modification time
        // (iCloud delivering another Mac's file, a `mv`) is not in the copy:
        // the original must hold exactly what the clone held.
        let held = |k: &str| self.journal.marker(&step, k).and_then(|v| v.as_u64());
        match icloud::tree_stats(&from) {
            Ok(now)
                if Some(now.files) == held("files")
                    && Some(now.dirs) == held("dirs")
                    && Some(now.links) == held("links")
                    && Some(now.bytes) == held("bytes") => {}
            Ok(now) => {
                return kept(format!(
                    "{} holds {} files, {} folders, {} links and {} bytes now, not what was copied ({} files, {} folders, {} links, {} bytes)",
                    from.display(),
                    now.files,
                    now.dirs,
                    now.links,
                    now.bytes,
                    held("files").unwrap_or(0),
                    held("dirs").unwrap_or(0),
                    held("links").unwrap_or(0),
                    held("bytes").unwrap_or(0)
                ))
            }
            Err(e) => return kept(format!("the original couldn't be read in full ({e})")),
        }
        // From here on the original may be partly gone: a rollback rebuilds it
        // from the copy.
        self.journal.append(
            &step,
            "writing",
            &serde_json::json!({ "removing": true }).to_string(),
        )?;
        if self.hooks.kill_in_retire {
            if let Some(Ok(e)) = std::fs::read_dir(&from).ok().and_then(|mut d| d.next()) {
                let _ = icloud::remove_tree(&e.path());
            }
            return Err(KILLED.into());
        }
        if let Err(e) = icloud::remove_tree(&from) {
            return Ok(serde_json::json!({ "removed": false, "error": e }).to_string());
        }
        let mut linked = false;
        if m.compat_symlink && !exists(&from) {
            std::os::unix::fs::symlink(&to, &from)
                .map_err(|e| format!("leave a link at {}: {e}", from.display()))?;
            linked = true;
        }
        Ok(serde_json::json!({ "removed": true, "linked": linked }).to_string())
    }

    /// Undo a copy move, from what the journal and the folders say:
    /// - no `copied` marker: the copy is removed only if it equals the original
    ///   (killed between the rename and the marker); anything else at `to` is
    ///   not ours and stays;
    /// - the original intact (no `removing` marker), or already rebuilt: the
    ///   copy is removed and iCloud is not touched;
    /// - the original removed or partly removed: it is rebuilt from the copy
    ///   (a clone straight to its path, compared, marked `rebuilt`), then the
    ///   copy is removed. A partial leftover is deleted first, but only when
    ///   every entry of it is in the copy (it is what the retire left, not
    ///   something else that appeared there).
    fn unwind_copy(&self, i: usize, from: &Path, to: &Path) -> Result<(), String> {
        let step = Op::Move(i).name();
        let _ = icloud::remove_tree(&sidecar(to, TMP_SUFFIX));
        let to_real = std::fs::symlink_metadata(to).is_ok_and(|x| !x.file_type().is_symlink());
        if !to_real {
            return Ok(());
        }
        let from_dir = std::fs::symlink_metadata(from).is_ok_and(|x| x.is_dir());
        if exists(from) && !from_dir {
            return Err(format!(
                "{} is in the way: something else is at that path now, while {} holds the copied folder. Move it aside and roll back again.",
                from.display(),
                to.display()
            ));
        }
        // Another move's folder inside this copy that wasn't put back (its undo
        // failed or hasn't run) would be deleted with it.
        let (me, froms) = (&self.plan.moves[i], current_froms(&self.plan.moves));
        let inside = format!("{}/", me.to);
        for (j, mj) in self.plan.moves.iter().enumerate() {
            let name = Op::Move(j).name();
            if j != i
                && (mj.to.starts_with(&inside) || froms[j].starts_with(&inside))
                && self.journal.touched().contains(&name)
                && !self.journal.undone(&name)
            {
                return Err(format!(
                    "{} is still inside the copy at {}: its move wasn't undone, so the copy stays until it is. Roll back again",
                    mj.to,
                    to.display()
                ));
            }
        }
        if !self.journal.marked(&step, "copied") {
            if from_dir && icloud::compare_trees(from, to, false).is_ok() {
                return icloud::remove_tree(to);
            }
            return Err(format!(
                "{} holds something that isn't the original of {} (the copy never finished or something else is there). Move it aside and roll back again.",
                to.display(),
                from.display()
            ));
        }
        let rebuilt = self.journal.marked(&step, "rebuilt");
        let removing = self.journal.marked(&step, "removing");
        if from_dir && (rebuilt || !removing) {
            // The original was kept: the copy may hold the only later work
            // (Hollow ran on it after the move). Then it is parked in the backup
            // folder, never deleted: the same volume, outside iCloud, instant.
            if !rebuilt {
                if let Some(p) = self
                    .verified_at()
                    .and_then(|at| icloud::modified_since(to, at))
                {
                    // A free name: a backup folder never deletes what is in it.
                    let mut parked = self.dir.join(format!("kept-copy-{i}"));
                    let mut n = 1;
                    while exists(&parked) {
                        n += 1;
                        parked = self.dir.join(format!("kept-copy-{i}-{n}"));
                    }
                    rename(to, &parked)?;
                    self.journal.append(
                        &step,
                        "writing",
                        &serde_json::json!({ "parked": s(&parked), "because": s(&p) }).to_string(),
                    )?;
                    return Ok(());
                }
            }
            return icloud::remove_tree(to);
        }
        if from_dir {
            icloud::names_subset(from, to).map_err(|e| {
                format!(
                    "{} is in the way: it holds something that isn't in {} ({e}). Move it aside and roll back again.",
                    from.display(),
                    to.display()
                )
            })?;
            icloud::remove_tree(from)?;
        }
        icloud::clone_tree(to, from)?;
        if let Err(e) = icloud::compare_trees(to, from, false) {
            let _ = icloud::remove_tree(from);
            return Err(format!(
                "the rebuilt {} doesn't match the copy ({e})",
                from.display()
            ));
        }
        self.journal.append(
            &step,
            "writing",
            &serde_json::json!({ "rebuilt": true }).to_string(),
        )?;
        if self.hooks.kill_in_rollback_copy {
            return Err(KILLED.into());
        }
        icloud::remove_tree(to)
    }

    fn sessions(&self) -> Result<String, String> {
        let old = sessions_db(&self.roots, &self.plan.old_stem);
        let new = sessions_db(&self.roots, &self.plan.new_stem);
        let orig = self.dir.join("sessions-original");
        let work = self.dir.join("sessions-work");
        let old_name = old
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let work_db = work.join(new.file_name().unwrap_or_default());
        if exists(&orig.join(&old_name)) {
            // The originals are already in the backup folder. Killed before
            // the rewritten copy went in: finish the swap from the work copy.
            if !exists(&new) {
                // Killed inside the first loop: an original `-wal`/`-shm` may
                // still sit at the old name. Move it out first, or SQLite would
                // replay the original WAL over the rewritten copy.
                for suffix in ["-wal", "-shm"] {
                    let p = sidecar(&old, suffix);
                    let o = orig.join(format!("{old_name}{suffix}"));
                    if exists(&p) && !exists(&o) {
                        rename(&p, &o)?;
                    }
                }
                if !exists(&work_db) {
                    return Err(format!(
                        "The original Sessions database is in {} but its rewritten copy is missing; roll back to put the original back.",
                        orig.display()
                    ));
                }
                for suffix in ["", "-wal", "-shm"] {
                    let p = sidecar(&work_db, suffix);
                    if exists(&p) {
                        rename(&p, &sidecar(&new, suffix))?;
                    }
                }
            }
            return self.sessions_detail(&new, &BTreeMap::new());
        }
        if !exists(&old) {
            return Ok(serde_json::json!({ "none": true, "counts": {} }).to_string());
        }
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
        clone_file(&old, &work_db)?;
        if exists(&sidecar(&old, "-wal")) {
            clone_file(&sidecar(&old, "-wal"), &sidecar(&work_db, "-wal"))?;
        }
        let mut rows: BTreeMap<String, u64> = BTreeMap::new();
        let (counts_before, counts_after);
        {
            let mut conn = Connection::open(&work_db).map_err(|e| format!("open the copy: {e}"))?;
            conn.busy_timeout(Duration::from_secs(5))
                .map_err(|e| e.to_string())?;
            conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
                .map_err(|e| format!("checkpoint the copy: {e}"))?;
            counts_before = table_counts(&conn)?;
            let audit_before = crate::audit::verify_in(&conn).map(|v| (v.ok, v.rows, v.first_bad));
            let (cols, _) = mp::rewrite_columns(&conn);
            let tx = conn.transaction().map_err(|e| e.to_string())?;
            for (i, (table, column, kind)) in cols.iter().enumerate() {
                let n = rewrite_column(&tx, table, column, *kind, &self.maps, &self.ids)?;
                if n > 0 {
                    rows.insert(format!("{table}.{column}"), n);
                }
                if i == 0 && self.hooks.fail_in_sessions_rekey {
                    return Err("injected failure inside the Sessions re-key (test)".into());
                }
            }
            // Append-only tables keep their ids; the old-to-new pairs go in
            // the same transaction, so they exist exactly when the re-key does.
            let move_id = self
                .dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let aliases = crate::workspace_aliases::record(&tx, &self.ids, &move_id, &now())?;
            if aliases > 0 {
                rows.insert("workspace_aliases".into(), aliases);
            }
            tx.commit().map_err(|e| format!("commit the copy: {e}"))?;
            let check: String = conn
                .query_row("PRAGMA quick_check", [], |r| r.get(0))
                .map_err(|e| format!("quick_check: {e}"))?;
            if check != "ok" {
                return Err(format!("The rewritten copy failed quick_check: {check}"));
            }
            // The audit record is never rewritten: its hash chain reads the
            // same after the re-key as before it.
            let audit_after = crate::audit::verify_in(&conn).map(|v| (v.ok, v.rows, v.first_bad));
            if audit_after != audit_before {
                return Err(format!(
                    "The audit record changed in the rewritten copy ({audit_before:?} before, {audit_after:?} after)"
                ));
            }
            counts_after = table_counts(&conn)?;
            // Only `workspace_aliases` may grow, by the rows just recorded.
            let without = |c: &BTreeMap<String, u64>| {
                let mut c = c.clone();
                c.remove("workspace_aliases");
                c
            };
            let alias_rows =
                |c: &BTreeMap<String, u64>| c.get("workspace_aliases").copied().unwrap_or(0);
            if without(&counts_before) != without(&counts_after)
                || alias_rows(&counts_after)
                    != alias_rows(&counts_before)
                        + rows.get("workspace_aliases").copied().unwrap_or(0)
            {
                return Err("Row counts changed in the rewritten copy".into());
            }
            conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
                .map_err(|e| format!("checkpoint the copy: {e}"))?;
        }
        // Swap: the original trio into the backup folder, the copy in. With
        // the stem unchanged both have the same name; the same steps apply.
        std::fs::create_dir_all(&orig).map_err(|e| e.to_string())?;
        for suffix in ["", "-wal", "-shm"] {
            let p = sidecar(&old, suffix);
            if exists(&p) {
                rename(&p, &orig.join(format!("{old_name}{suffix}")))?;
            }
            if suffix.is_empty() && self.hooks.kill_after_first_original {
                return Err(KILLED.into());
            }
        }
        if self.hooks.kill_in_sessions_swap {
            return Err(KILLED.into());
        }
        for suffix in ["", "-wal", "-shm"] {
            let p = sidecar(&work_db, suffix);
            if exists(&p) {
                rename(&p, &sidecar(&new, suffix))?;
            }
        }
        let _ = counts_after;
        self.sessions_detail(&new, &rows)
    }

    /// The Sessions step's journal detail, read from the swapped-in file:
    /// rows rewritten, every table's count (verify compares them), sessions
    /// and the newest turn.
    fn sessions_detail(&self, db: &Path, rows: &BTreeMap<String, u64>) -> Result<String, String> {
        let counts = mp::open_read_only(db).and_then(|c| table_counts(&c))?;
        let sessions = counts.get("sessions").copied().unwrap_or(0);
        Ok(serde_json::json!({ "rows": rows, "sessions": sessions, "turnsMax": max_rowid(db, "turns"), "counts": counts }).to_string())
    }

    /// A file kind's rewrite, forward (old to new) or back (new to old).
    fn transform(&self, what: &str, text: &str, forward: bool) -> Result<Option<String>, String> {
        let rev: Vec<(String, String)>;
        let rev_ids: BTreeMap<String, String>;
        let (maps, ids, from_stem, to_stem) = if forward {
            (
                &self.maps,
                &self.ids,
                &self.plan.old_stem,
                &self.plan.new_stem,
            )
        } else {
            rev = rekey::sorted_mappings(
                &self
                    .maps
                    .iter()
                    .map(|(a, b)| (b.clone(), a.clone()))
                    .collect::<Vec<_>>(),
            );
            rev_ids = self
                .ids
                .iter()
                .map(|(a, b)| (b.clone(), a.clone()))
                .collect();
            (&rev, &rev_ids, &self.plan.new_stem, &self.plan.old_stem)
        };
        Ok(match what {
            FILE_STORE => {
                rekey::rewrite_store_json(text, from_stem, to_stem, maps, ids).map(|(t, _)| t)
            }
            FILE_NOTE => Some(rekey::rewrite_text(text, maps, ids).0),
            FILE_CLAUDE_JSON => rekey::rewrite_claude_json(text, maps).map(|(t, _, _)| t),
            FILE_CODEX => Some(rekey::rewrite_codex_headers(text, maps).0),
            FILE_GEMINI | FILE_CLAUDE_DESKTOP | FILE_OBSIDIAN => {
                rekey::rewrite_json_paths(text, maps).map(|(t, _)| t)
            }
            FILE_LAUNCH_AGENT | FILE_SCRIPT => {
                if what == FILE_LAUNCH_AGENT && !text.trim_start().starts_with('<') {
                    return Ok(Some(text.to_string()));
                }
                Some(rekey::rewrite_paths(text, &rekey::home_forms(maps, &self.roots.home)).0)
            }
            other => return Err(format!("unknown file kind {other}")),
        })
    }

    /// Rewrite one file. Before writing, the step journals the sha256 of the
    /// bytes it is about to write and keeps a copy of the bytes they replace;
    /// a resumed step whose file already holds exactly those bytes is done,
    /// and a rollback restores the copy only while the file still holds them.
    fn rewrite_file(&self, i: usize) -> Result<String, String> {
        let f = &self.plan.files[i];
        let step = Op::File(i).name();
        let cur = self.final_path(&f.path);
        let meta = match std::fs::symlink_metadata(&cur) {
            Ok(m) => m,
            Err(_) => return Ok(serde_json::json!({ "skipped": "gone" }).to_string()),
        };
        let before = meta.len();
        if let (Some(sha), pre, _) = self.journal.file_write(&step) {
            if sha_file(&cur).as_deref() == Some(sha.as_str()) {
                // The write landed before a kill: the size before it is the
                // copy's, so verify's byte count stays right.
                let after = before;
                let before = pre
                    .and_then(|p| std::fs::metadata(p).ok())
                    .map_or(after, |m| m.len());
                return Ok(serde_json::json!({ "before": before, "after": after, "sha": sha, "already": true }).to_string());
            }
        }
        if meta.file_type().is_symlink() && f.what != FILE_CONFIG {
            return Ok(serde_json::json!({ "skipped": "a link" }).to_string());
        }
        let pre = self.dir.join("snapshot").join(format!("pre-file-{i}"));
        let mode = mode_of(&cur);
        if f.what == FILE_CONFIG {
            let mut map = crate::setup::load_config_map(&cur)?;
            // The first pass's copy holds the original bytes; keep it.
            if !exists(&pre) {
                clone_file(&cur, &pre)?;
            }
            self.journal.append(
                &step,
                "writing",
                &serde_json::json!({ "pre": s(&pre), "mode": mode }).to_string(),
            )?;
            for (k, v) in mp::config_changes(&self.plan, &map) {
                map.insert(k, serde_json::json!(v));
            }
            crate::setup::publish_config(&cur, &map)?;
            let sha = sha_file(&cur).unwrap_or_default();
            let after = std::fs::metadata(&cur).map(|m| m.len()).unwrap_or(0);
            return Ok(
                serde_json::json!({ "before": before, "after": after, "sha": sha }).to_string(),
            );
        }
        let Some(text) = read_checked(&cur, self.probes)? else {
            return Ok(serde_json::json!({ "skipped": "iCloud-evicted" }).to_string());
        };
        let Some(new_text) = self.transform(&f.what, &text, true)? else {
            return Err(format!("{} isn't readable JSON any more", cur.display()));
        };
        let sha = sha_hex(new_text.as_bytes());
        if new_text != text {
            // The first pass's copy holds the original bytes; keep it.
            if !exists(&pre) {
                clone_file(&cur, &pre)?;
            }
            self.journal.append(
                &step,
                "writing",
                &serde_json::json!({ "sha": sha, "pre": s(&pre), "mode": mode }).to_string(),
            )?;
            write_atomic(&cur, new_text.as_bytes(), None)?;
            if self.hooks.kill_after_write == Some(i) {
                return Err(KILLED.into());
            }
        }
        Ok(
            serde_json::json!({ "before": before, "after": new_text.len(), "sha": sha })
                .to_string(),
        )
    }

    fn git_repair(&self, i: usize) -> Result<String, String> {
        let r = &self.plan.repos[i];
        let snap: Snapshot = read_json(&self.dir.join(SNAPSHOT_FILE))?;
        let folders = snap.worktrees.get(i).cloned().unwrap_or_default();
        let main = PathBuf::from(&r.repo_to);
        let new_folders: Vec<PathBuf> = folders
            .iter()
            .map(|f| self.final_path(f))
            .filter(|f| exists(&f.join(".git")))
            .collect();
        let size_of = |p: &Path| std::fs::metadata(p).map(|m| m.len() as i64).unwrap_or(0);
        let roots_from = top_level_froms(&self.plan);
        let tracked = |snap: &Snapshot| -> i64 {
            snap.files
                .iter()
                .filter(|e| e.path.contains("/.git"))
                .filter(|e| {
                    roots_from
                        .iter()
                        .any(|r| e.path.starts_with(&format!("{r}/")))
                })
                .map(|e| size_of(&self.final_path(&e.path)))
                .sum()
        };
        let before = tracked(&snap);
        // Point each worktree's `.git` file and the main's `gitdir` files at
        // the new places. `git worktree repair` alone leaves them: through
        // the link left at the old path they still resolve, so git sees
        // nothing to fix, and they break the day that link goes.
        let mut pointers: Vec<PathBuf> = new_folders.iter().map(|f| f.join(".git")).collect();
        for e in std::fs::read_dir(main.join(".git").join("worktrees"))
            .into_iter()
            .flatten()
            .flatten()
        {
            pointers.push(e.path().join("gitdir"));
        }
        for p in pointers {
            if !std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_file()) {
                continue;
            }
            let Some(text) = read_checked(&p, self.probes)? else {
                continue;
            };
            let (new_text, n) = rekey::rewrite_git_pointer(&text, &self.maps);
            if n > 0 {
                write_atomic(&p, new_text.as_bytes(), None)?;
            }
        }
        if !new_folders.is_empty() {
            let mut cmd = git(&main, &self.roots.home);
            cmd.args(["worktree", "repair"]).args(&new_folders);
            let out = cmd.output().map_err(|e| format!("git: {e}"))?;
            if !out.status.success() {
                return Err(format!(
                    "git worktree repair in {} failed: {}",
                    main.display(),
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
        }
        let after = tracked(&snap);
        Ok(
            serde_json::json!({ "repaired": new_folders.len(), "delta": after - before })
                .to_string(),
        )
    }

    fn verify(&self) -> Result<String, String> {
        let mut checks: Vec<Check> = Vec::new();
        let states = self.journal.states();
        let snap: Snapshot = read_json(&self.dir.join(SNAPSHOT_FILE)).unwrap_or_default();
        // Files and bytes in the moved folders.
        if snap.counted {
            let deadline = Instant::now() + Duration::from_secs(600);
            let (mut files, mut bytes) = (0u64, 0u64);
            for root in top_level_tos(&self.plan) {
                let w = mp::walk(Path::new(&root), self.probes, deadline);
                files += w.files;
                bytes += w.bytes;
            }
            let mut delta: i64 = 0;
            for (i, f) in self.plan.files.iter().enumerate() {
                if !top_level_froms(&self.plan)
                    .iter()
                    .any(|r| f.path.starts_with(&format!("{r}/")))
                {
                    continue;
                }
                if let Some((_, d)) = states.get(&Op::File(i).name()) {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(d) {
                        delta +=
                            v["after"].as_i64().unwrap_or(0) - v["before"].as_i64().unwrap_or(0);
                    }
                }
            }
            for i in 0..self.plan.repos.len() {
                if let Some((_, d)) = states.get(&Op::Git(i).name()) {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(d) {
                        delta += v["delta"].as_i64().unwrap_or(0);
                    }
                }
            }
            let expected = snap.bytes_before as i64 + delta;
            let ok = files == snap.files_before && bytes as i64 == expected;
            checks.push(Check {
                name: "files".into(),
                ok,
                detail: format!(
                    "{files} files, {bytes} bytes after; {} files, {expected} bytes expected",
                    snap.files_before
                ),
            });
        } else {
            checks.push(Check {
                name: "files".into(),
                ok: true,
                detail: "too many files to count within 10 minutes; skipped".into(),
            });
        }
        // Sessions database.
        let new_db = sessions_db(&self.roots, &self.plan.new_stem);
        let mut sessions_now = 0;
        let sessions_detail = states
            .get("sessions")
            .map(|(_, d)| serde_json::from_str::<serde_json::Value>(d).unwrap_or_default())
            .filter(|v| v["none"].as_bool() != Some(true));
        if let Some(v) = sessions_detail {
            let expected = v["counts"].clone();
            let now = mp::open_read_only(&new_db).and_then(|c| table_counts(&c));
            let ok = match &now {
                Ok(c) => {
                    sessions_now = c.get("sessions").copied().unwrap_or(0) as i64;
                    serde_json::to_value(c).unwrap_or_default() == expected
                }
                Err(_) => false,
            };
            // The original is in the backup folder; under a new stem its old
            // name is gone, under the same stem the rewritten copy took it.
            let old_db = sessions_db(&self.roots, &self.plan.old_stem);
            let old_name = old_db.file_name().unwrap_or_default();
            let old_gone = exists(&self.dir.join("sessions-original").join(old_name))
                && (self.plan.old_stem == self.plan.new_stem || !exists(&old_db));
            checks.push(Check {
                name: "sessions".into(),
                ok: ok && old_gone,
                detail: match now {
                    Ok(c) => format!(
                        "{} sessions, {} external sessions under {}; the old file is {}",
                        c.get("sessions").copied().unwrap_or(0),
                        c.get("external_sessions").copied().unwrap_or(0),
                        self.plan.new_stem,
                        if old_gone {
                            "in the backup folder"
                        } else {
                            "still there"
                        }
                    ),
                    Err(e) => format!("couldn't read {}: {e}", new_db.display()),
                },
            });
            // Every moved workspace id resolves to its new id, so rows that
            // keep the old one (append-only `audit_log`) still read under it.
            let aliases = mp::open_read_only(&new_db)
                .map(|c| crate::workspace_aliases::Aliases::load(&c))
                .unwrap_or_default();
            let moved: Vec<(&String, &String)> = self.ids.iter().filter(|(a, b)| a != b).collect();
            let wrong: Vec<String> = moved
                .iter()
                .filter(|(old, new)| aliases.current(old) != aliases.current(new))
                .map(|(old, _)| (*old).clone())
                .collect();
            checks.push(Check {
                name: "workspace aliases".into(),
                ok: wrong.is_empty(),
                detail: if wrong.is_empty() {
                    format!(
                        "{} moved workspace ids resolve to their new ids",
                        moved.len()
                    )
                } else {
                    format!("not resolved: {}", wrong.join(", "))
                },
            });
            // No second, empty set under the new stem beside it.
            let strays: Vec<String> = mp::stem_entries(&self.roots, &self.plan.new_stem)
                .into_iter()
                .filter(|p| {
                    p.extension().is_some_and(|x| x == "db")
                        && *p != new_db
                        && p.parent() == new_db.parent()
                })
                .map(|p| s(&p))
                .collect();
            checks.push(Check {
                name: "single store set".into(),
                ok: strays.is_empty(),
                detail: if strays.is_empty() {
                    "one set under the new stem".into()
                } else {
                    strays.join(", ")
                },
            });
        }
        // Workspaces.
        let ws = self.vault_now().join("workspaces");
        let missing: Vec<String> = self
            .plan
            .ids
            .iter()
            .filter(|i| i.has_workspace && (exists(&ws.join(&i.old)) || !ws.join(&i.new).is_dir()))
            .map(|i| i.new.clone())
            .collect();
        checks.push(Check {
            name: "workspaces".into(),
            ok: missing.is_empty(),
            detail: if missing.is_empty() {
                format!(
                    "{} renamed; {} without a connected project (reported, untouched)",
                    self.plan.ids.iter().filter(|i| i.has_workspace).count(),
                    self.plan.orphans.len()
                )
            } else {
                format!("not renamed: {}", missing.join(", "))
            },
        });
        // Git.
        for (i, r) in self.plan.repos.iter().enumerate() {
            if r.skipped.is_some() {
                continue;
            }
            let before = snap.lists.get(i).cloned().unwrap_or_default();
            let after = git_list(Path::new(&r.repo_to), &self.roots.home).unwrap_or_default();
            let (bw, bp) = parse_list(&before);
            let (aw, ap) = parse_list(&after);
            let expected: std::collections::BTreeSet<String> = bw
                .iter()
                .map(|w| rekey::whole_or_same(w, &self.maps))
                .collect();
            let new_prunable = ap
                .iter()
                .filter(|p| {
                    !bp.iter()
                        .any(|b| rekey::whole_or_same(b, &self.maps) == **p)
                })
                .count();
            let ok = expected == aw && new_prunable == 0;
            checks.push(Check {
                name: format!("git {}", r.repo_to),
                ok,
                detail: format!("{} worktrees, {new_prunable} newly prunable", aw.len()),
            });
        }
        // Config.
        if let Ok(map) = crate::setup::load_config_map(&self.roots.config_path) {
            let left = mp::config_changes(&self.plan, &map);
            checks.push(Check {
                name: "config".into(),
                ok: left.is_empty(),
                detail: if left.is_empty() {
                    "vaultPath, projectsPath and hollowHome point at the new folders".into()
                } else {
                    format!(
                        "still to change: {}",
                        left.iter()
                            .map(|(k, _)| k.clone())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                },
            });
        }
        // launchd can read the new vault.
        match self.probes.launchd_probe(&self.vault_now(), &self.dir) {
            Ok(Some(head)) => checks.push(Check {
                name: "launchd".into(),
                ok: true,
                detail: format!("a launchd job listed the vault: {head}"),
            }),
            Ok(None) => checks.push(Check {
                name: "launchd".into(),
                ok: true,
                detail: "launchctl isn't available here; skipped".into(),
            }),
            Err(e) => checks.push(Check {
                name: "launchd".into(),
                ok: false,
                detail: e,
            }),
        }
        let at = SystemTime::now();
        write_json(&self.dir.join(VERIFY_FILE), &checks)?;
        write_json(
            &self.dir.join("verified.json"),
            &Verified {
                at_nanos: nanos(at),
                at: now(),
                sessions: sessions_now,
                turns_max: max_rowid(&new_db, "turns"),
                new_db: s(&new_db),
                vault: s(&self.vault_now()),
            },
        )?;
        let failed: Vec<&Check> = checks.iter().filter(|c| !c.ok).collect();
        if failed.is_empty() {
            Ok(format!("{} checks passed", checks.len()))
        } else {
            Ok(format!(
                "{} of {} checks failed: {}",
                failed.len(),
                checks.len(),
                failed
                    .iter()
                    .map(|c| c.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        }
    }

    // ── undo ──────────────────────────────────────────────────────────

    fn undo(&self, op: &Op, detail: &str) -> Result<(), String> {
        match op {
            Op::Preflight | Op::Snapshot | Op::Probe | Op::Verify | Op::File(_) | Op::Git(_) => {
                Ok(())
            }
            // The original comes back with its move's undo, which sees from
            // the journal whether it was removed.
            Op::Retire(_) => Ok(()),
            Op::Bootout(_) => Ok(()), // bootstrapped again after files are restored
            Op::Bootstrap(i) => {
                let l = &self.plan.launch_agents[*i];
                self.probes.bootout(&l.label)
            }
            Op::Mkdir(i) => {
                let d = Path::new(&self.plan.mkdirs[*i]);
                if detail == "created" && std::fs::read_dir(d).is_ok_and(|mut e| e.next().is_none())
                {
                    std::fs::remove_dir(d).map_err(|e| format!("remove {}: {e}", d.display()))?;
                }
                Ok(())
            }
            Op::Move(i) => self.undo_move(*i, detail),
            Op::Workspace(i) => {
                let id = &self.plan.ids[*i];
                let ws = self.vault_now_for_undo(*i).join("workspaces");
                let (from, to) = (ws.join(&id.old), ws.join(&id.new));
                if exists(&to) && exists(&from) {
                    if !self.journal.completed(&op.name()) {
                        return Ok(()); // the rename never happened
                    }
                    return Err(format!(
                        "{} exists again, so {} can't take its name back",
                        from.display(),
                        to.display()
                    ));
                }
                if exists(&to) {
                    rename(&to, &from)?;
                }
                Ok(())
            }
            Op::Store(i) => {
                let st = &self.plan.stores[*i];
                let (from, to) = (Path::new(&st.from), Path::new(&st.to));
                if exists(to) && exists(from) {
                    if !self.journal.completed(&op.name()) {
                        return Ok(()); // the rename never happened
                    }
                    return Err(format!(
                        "{} exists again, so {} can't take its name back",
                        from.display(),
                        to.display()
                    ));
                }
                if exists(to) {
                    rename(to, from)?;
                }
                Ok(())
            }
            Op::Sessions => {
                let old = sessions_db(&self.roots, &self.plan.old_stem);
                let new = sessions_db(&self.roots, &self.plan.new_stem);
                let orig = self.dir.join("sessions-original");
                let old_name = old
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                // Nothing to undo only when none of the original trio is left
                // in the backup (a retry after a partial restore finds some).
                if !["", "-wal", "-shm"]
                    .iter()
                    .any(|s| exists(&orig.join(format!("{old_name}{s}"))))
                {
                    return Ok(());
                }
                let aside = self.dir.join("sessions-rolled-back");
                std::fs::create_dir_all(&aside).map_err(|e| e.to_string())?;
                let new_name = new
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                // The copy's trio goes aside only when the copy went in: with
                // the stem unchanged, sidecars at that name with no main file
                // are originals the swap never moved, and they stay.
                // Sidecars go first: while the main file is still here, so are
                // they, so a rollback killed in this loop resumes correctly.
                if exists(&new) {
                    for suffix in ["-wal", "-shm", ""] {
                        if suffix.is_empty() && self.hooks.kill_rollback_before_copy_main {
                            return Err(KILLED.into());
                        }
                        let p = sidecar(&new, suffix);
                        if exists(&p) {
                            rename(&p, &aside.join(format!("{new_name}{suffix}")))?;
                        }
                    }
                }
                // Every conflict is found before anything moves, and the
                // sidecars go back before the main file: while the original
                // main file is still in the backup, so are its sidecars.
                let order = ["-wal", "-shm", ""];
                for suffix in order {
                    let p = orig.join(format!("{old_name}{suffix}"));
                    let back = sidecar(&old, suffix);
                    if exists(&p) && exists(&back) {
                        return Err(format!(
                            "Both {} and the original in {} exist; move one aside, then roll back again.",
                            back.display(),
                            p.display()
                        ));
                    }
                }
                for suffix in order {
                    if suffix.is_empty() && self.hooks.kill_rollback_before_original_main {
                        return Err(KILLED.into());
                    }
                    let p = orig.join(format!("{old_name}{suffix}"));
                    if exists(&p) {
                        rename(&p, &sidecar(&old, suffix))?;
                    }
                }
                Ok(())
            }
        }
    }

    /// The vault's folder while undoing workspace step `_i`: moves are
    /// undone after it (reverse order), so it is still the new one.
    fn vault_now_for_undo(&self, _i: usize) -> PathBuf {
        let p = self.vault_now();
        if exists(&p) && !std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()) {
            p
        } else {
            PathBuf::from(&self.plan.vault_from)
        }
    }

    fn undo_move(&self, i: usize, detail: &str) -> Result<(), String> {
        let froms = current_froms(&self.plan.moves);
        let m = &self.plan.moves[i];
        let (from, to) = (PathBuf::from(&froms[i]), PathBuf::from(&m.to));
        let d: serde_json::Value = serde_json::from_str(detail).unwrap_or_default();
        // Check before touching anything: with the moved folder at `to` and
        // something real (or a link elsewhere) at `from`, there is nowhere to
        // put it back.
        let to_real_now = std::fs::symlink_metadata(&to).is_ok_and(|x| !x.file_type().is_symlink());
        let copy = m.method == METHOD_COPY;
        if !copy
            && !case_only(&froms[i], &m.to)
            && to_real_now
            && exists(&from)
            && !is_link_to(&from, &to)
        {
            return Err(format!(
                "{} is in the way: something else is at that path now, while {} holds the moved folder. Move it aside and roll back again.",
                from.display(),
                to.display()
            ));
        }
        if is_link_to(&from, &to) {
            std::fs::remove_file(&from)
                .map_err(|e| format!("remove the link {}: {e}", from.display()))?;
        }
        if case_only(&froms[i], &m.to) {
            if actual_name(&to).as_deref() == to.file_name().and_then(|n| n.to_str()) {
                rename(&to, &from)?;
            }
            return Ok(());
        }
        let to_real = std::fs::symlink_metadata(&to).is_ok_and(|x| !x.file_type().is_symlink());
        if copy {
            self.unwind_copy(i, &from, &to)?;
        } else if to_real && !exists(&from) {
            rename(&to, &from)?;
        }
        if !exists(&to) {
            let link = d["replacedLink"]
                .as_str()
                .map(str::to_string)
                .or_else(|| m.replaces_symlink.clone());
            if let Some(target) = link {
                std::os::unix::fs::symlink(&target, &to)
                    .map_err(|e| format!("put back the link {}: {e}", to.display()))?;
            } else if d["replacedEmpty"].as_bool() == Some(true) {
                std::fs::create_dir(&to)
                    .map_err(|e| format!("make {} again: {e}", to.display()))?;
            }
        }
        Ok(())
    }

    /// Put back every file a touched step wrote. A file that still holds
    /// exactly the bytes the run wrote gets its old bytes back; one changed
    /// since (an agent rewrote its settings) is mapped back new to old with
    /// the same rewriter, keeping the later change, and is listed. Safe to
    /// run again. Git pointer files come back from the snapshot.
    fn restore_files(
        &self,
        touched: &std::collections::BTreeSet<String>,
    ) -> Result<Vec<String>, String> {
        let mut mapped_back = Vec::new();
        for (i, f) in self.plan.files.iter().enumerate() {
            let step = Op::File(i).name();
            if !touched.contains(&step) {
                continue;
            }
            let path = Path::new(&f.path);
            let _ = std::fs::remove_file(sidecar(path, TMP_SUFFIX));
            let (sha, pre, mode) = self.journal.file_write(&step);
            let Some(pre) = pre else {
                continue; // never wrote
            };
            let Ok(old_bytes) = std::fs::read(&pre) else {
                return Err(format!("the copy {pre} of {} is missing", f.path));
            };
            let Ok(cur) = std::fs::read(path) else {
                continue;
            };
            if cur == old_bytes {
                continue;
            }
            let mode = mode.unwrap_or_else(|| mode_of(path));
            if sha.as_deref() == Some(sha_hex(&cur).as_str()) {
                write_atomic(path, &old_bytes, Some(mode))?;
                continue;
            }
            // Changed since the move wrote it.
            if f.what == FILE_CONFIG {
                let old_map: BTreeMap<String, serde_json::Value> =
                    serde_json::from_slice(&old_bytes).unwrap_or_default();
                let mut map = crate::setup::load_config_map(path)?;
                for k in ["vaultPath", "projectsPath", "hollowHome", "gitReposPath"] {
                    match old_map.get(k) {
                        Some(v) => {
                            map.insert(k.into(), v.clone());
                        }
                        None => {
                            map.remove(k);
                        }
                    }
                }
                crate::setup::publish_config(path, &map)?;
                mapped_back.push(f.path.clone());
                continue;
            }
            let text = String::from_utf8_lossy(&cur).into_owned();
            if let Some(back) = self.transform(&f.what, &text, false)? {
                if back != text {
                    write_atomic(path, back.as_bytes(), Some(mode))?;
                }
                mapped_back.push(f.path.clone());
            }
        }
        if (0..self.plan.repos.len()).any(|i| touched.contains(&Op::Git(i).name())) {
            if let Ok(snap) = read_json::<Snapshot>(&self.dir.join(SNAPSHOT_FILE)) {
                for e in snap.files.iter().filter(|e| e.path.contains("/.git")) {
                    let bytes =
                        std::fs::read(&e.copy).map_err(|err| format!("{}: {err}", e.copy))?;
                    let path = Path::new(&e.path);
                    if std::fs::read(path).ok().as_deref() != Some(bytes.as_slice())
                        || mode_of(path) != e.mode
                    {
                        write_atomic(path, &bytes, Some(e.mode))?;
                    }
                }
            }
        }
        Ok(mapped_back)
    }
}

fn case_only(a: &str, b: &str) -> bool {
    a != b && a.to_lowercase() == b.to_lowercase() && Path::new(a).parent() == Path::new(b).parent()
}

/// The name a folder entry has on disk (case as stored).
fn actual_name(p: &Path) -> Option<String> {
    let want = p.file_name()?.to_string_lossy().to_lowercase();
    std::fs::read_dir(p.parent()?)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.to_lowercase() == want)
}

/// The moves whose source isn't inside another move's source.
fn top_level_froms(plan: &Plan) -> Vec<String> {
    plan.moves
        .iter()
        .filter(|m| {
            !plan
                .moves
                .iter()
                .any(|o| o != *m && m.from.starts_with(&format!("{}/", o.from)))
        })
        .map(|m| m.from.clone())
        .collect()
}

/// The final folders that aren't inside another move's target.
fn top_level_tos(plan: &Plan) -> Vec<String> {
    plan.moves
        .iter()
        .filter(|m| {
            !plan
                .moves
                .iter()
                .any(|o| o != *m && m.to.starts_with(&format!("{}/", o.to)))
        })
        .map(|m| m.to.clone())
        .collect()
}

pub(crate) fn table_counts(conn: &Connection) -> Result<BTreeMap<String, u64>, String> {
    let mut out = BTreeMap::new();
    for t in mp::tables(conn) {
        let n: i64 = conn
            .query_row(
                &format!("SELECT count(*) FROM \"{}\"", t.replace('"', "\"\"")),
                [],
                |r| r.get(0),
            )
            .map_err(|e| format!("count {t}: {e}"))?;
        out.insert(t, n.max(0) as u64);
    }
    Ok(out)
}

fn max_rowid(db: &Path, table: &str) -> i64 {
    mp::open_read_only(db)
        .ok()
        .and_then(|c| {
            c.query_row(
                &format!("SELECT COALESCE(max(rowid), 0) FROM \"{table}\""),
                [],
                |r| r.get(0),
            )
            .ok()
        })
        .unwrap_or(0)
}

pub(crate) fn rewrite_column(
    conn: &Connection,
    table: &str,
    column: &str,
    kind: mp::ColKind,
    maps: &[(String, String)],
    ids: &BTreeMap<String, String>,
) -> Result<u64, String> {
    let (t, c) = (table.replace('"', "\"\""), column.replace('"', "\"\""));
    let mut select = match conn.prepare(&format!(
        "SELECT rowid, \"{c}\" FROM \"{t}\" WHERE typeof(\"{c}\") = 'text'"
    )) {
        Ok(s) => s,
        // A WITHOUT ROWID table has no rowid: left as it is.
        Err(_) => return Ok(0),
    };
    let changes: Vec<(i64, String)> = select
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .filter_map(|(id, v)| mp::rewrite_value(&v, kind, maps, ids).map(|n| (id, n)))
        .collect();
    let mut update = conn
        .prepare(&format!("UPDATE \"{t}\" SET \"{c}\" = ?1 WHERE rowid = ?2"))
        .map_err(|e| e.to_string())?;
    for (id, v) in &changes {
        update
            .execute(rusqlite::params![v, id])
            .map_err(|e| format!("{table}.{column}: {e}"))?;
    }
    Ok(changes.len() as u64)
}

/// The user's git in `dir`, with `HOME` set to the roots' home, `GIT_*`
/// stripped and no prompt.
fn git(dir: &Path, home: &Path) -> std::process::Command {
    let mut cmd = std::process::Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(&key);
        }
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .env("HOME", home)
        .arg("-C")
        .arg(dir);
    #[cfg(test)]
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    cmd
}

fn git_list(main: &Path, home: &Path) -> Option<String> {
    let out = git(main, home)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// (worktree paths, prunable worktree paths) from `--porcelain`.
fn parse_list(text: &str) -> (std::collections::BTreeSet<String>, Vec<String>) {
    let mut all = std::collections::BTreeSet::new();
    let mut prunable = Vec::new();
    let mut cur = String::new();
    for line in text.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            cur = p.to_string();
            all.insert(cur.clone());
        } else if line.starts_with("prunable") {
            prunable.push(cur.clone());
        }
    }
    (all, prunable)
}

fn pid_alive(pid: i32) -> bool {
    // SAFETY: kill with signal 0 only checks the process exists.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn wait_for_exit(pid: i32, limit: Duration) -> Result<(), String> {
    let deadline = Instant::now() + limit;
    while pid > 1 && pid_alive(pid) {
        if Instant::now() > deadline {
            return Err(format!(
                "Hollow (pid {pid}) didn't quit within {} s; nothing was changed.",
                limit.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}

/// An exclusive lock on the backup folder's lock file while a helper works.
struct Lock(#[allow(dead_code)] std::fs::File);

fn take_lock(dir: &Path) -> Result<Lock, String> {
    use std::os::unix::io::AsRawFd;
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(LOCK_FILE))
        .map_err(|e| e.to_string())?;
    // A process forked by another thread holds a copy of this descriptor
    // until it execs (O_CLOEXEC closes it only then), so a lock just released
    // can read as held for a moment: try for up to 3 s before calling it busy.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        // SAFETY: a valid open descriptor.
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(Lock(f));
        }
        if std::time::Instant::now() >= deadline {
            return Err("Another move helper is working on this backup folder.".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub(crate) fn locked(dir: &Path) -> bool {
    use std::os::unix::io::AsRawFd;
    let Ok(f) = std::fs::File::open(dir.join(LOCK_FILE)) else {
        return false;
    };
    // SAFETY: a valid open descriptor; a shared probe released at once.
    let rc = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) };
    if rc == 0 {
        // SAFETY: as above.
        unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_UN) };
        false
    } else {
        true
    }
}

// ── Staging ───────────────────────────────────────────────────────────────

/// Re-plan, check `hash` and readiness, and write the backup folder.
pub(crate) fn stage(
    roots: &Roots,
    options: &crate::move_home::Options,
    hash: &str,
    caller: Option<&str>,
    probes: &dyn Probes,
) -> Result<Staged, String> {
    let plan = mp::plan_with(roots, options, caller, probes)?;
    if plan.hash != hash {
        return Err("The plan changed since it was shown. Nothing was changed; run the dry run again and confirm the new plan.".into());
    }
    if plan.is_noop() {
        return Err("Already home: nothing to move or re-key.".into());
    }
    if !plan.ready {
        let why: Vec<String> = plan
            .blockers
            .iter()
            .filter(|b| b.blocking)
            .map(|b| b.detail.clone())
            .collect();
        return Err(format!("The move can't start: {}", why.join("; ")));
    }
    if let Some(prev) = latest_backup(roots) {
        let st = status_in(&prev, roots);
        if st.state == "running" || half_state(&prev).is_some() {
            // A staged plan whose helper never started holds no journal and
            // doesn't block; an unfinished run does.
            return Err(format!(
                "A move is unfinished in {}; run it again (move-home --run) or roll it back first.",
                prev.display()
            ));
        }
    }
    let backups = roots.backups();
    std::fs::create_dir_all(&backups).map_err(|e| format!("make {}: {e}", backups.display()))?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let mut dir = backups.join(format!("move-home-{stamp}"));
    let mut n = 2;
    while exists(&dir) {
        dir = backups.join(format!("move-home-{stamp}-{n}"));
        n += 1;
    }
    std::fs::create_dir(&dir).map_err(|e| format!("make {}: {e}", dir.display()))?;
    let plan_file = dir.join(PLAN_FILE);
    write_json(&plan_file, &plan)?;
    write_json(&dir.join(ROOTS_FILE), roots)?;
    write_json(
        &dir.join(MANIFEST_FILE),
        &Manifest {
            version: crate::move_home::PLAN_VERSION,
            created: now(),
            plan_hash: plan.hash.clone(),
            caller: caller.map(str::to_string),
            options: options.clone(),
            backup_dir: s(&dir),
        },
    )?;
    std::fs::write(dir.join("plan.txt"), plan.text.join("\n")).map_err(|e| e.to_string())?;
    Ok(Staged {
        backup_dir: dir,
        plan_file,
    })
}

// ── Run and rollback ──────────────────────────────────────────────────────

/// Execute the staged plan at `plan_file`, resuming after the last completed
/// step. A failing step rolls everything back.
pub fn run_with(
    plan_file: &Path,
    probes: &dyn Probes,
    wait_pid: Option<i32>,
    hooks: &RunHooks,
) -> Result<String, String> {
    let dir = plan_file
        .parent()
        .ok_or("The plan file has no folder.")?
        .to_path_buf();
    let _lock = take_lock(&dir)?;
    let mut run = Run::load(&dir, probes)?;
    run.hooks = hooks.clone();
    let states = run.journal.states();
    match run.journal.last_outcome() {
        Some(s) if s.step == "run" && s.state == "done" => {
            return Ok("The move already finished.".into())
        }
        Some(s) if s.step == "rollback" => {
            return Err("This move was rolled back (or a rollback of it began); finish that with move-home --rollback, then stage a new move.".into())
        }
        _ => {}
    }
    let ops = ops(&run.plan);
    let mut completed = 0;
    for op in &ops {
        let name = op.name();
        if states.get(&name).is_some_and(|(st, _)| st == "done") {
            completed += 1;
            continue;
        }
        if hooks.stop_after == Some(completed) {
            if hooks.crash_mid {
                run.journal.append(&name, "started", "")?;
            }
            return Err(KILLED.into());
        }
        run.journal.append(&name, "started", "")?;
        let result = if hooks.fail_step.as_deref() == Some(name.as_str()) {
            Err(format!("injected failure in {name} (test)"))
        } else {
            run.exec(op, wait_pid)
        };
        match result {
            Ok(detail) => {
                run.journal.append(&name, "done", &detail)?;
                completed += 1;
            }
            Err(e) if e == KILLED => return Err(e),
            Err(e) => {
                run.journal.append(&name, "failed", &e)?;
                let nothing = matches!(op, Op::Preflight | Op::Snapshot | Op::Probe)
                    && !run.journal.read().iter().any(|s| {
                        s.state == "done"
                            && !matches!(s.step.as_str(), "preflight" | "snapshot" | "probe")
                    });
                if nothing {
                    run.journal.append("run", "failed", &e)?;
                    return Err(e);
                }
                run.journal.append("run", "failed", &e)?;
                if run.hooks.kill_before_auto_rollback {
                    return Err(KILLED.into());
                }
                run.journal
                    .append("rollback", "started", "automatic, after a failed step")?;
                let msg = match undo_all(&run) {
                    Ok(done) => {
                        run.journal.append("rollback", "done", &done)?;
                        format!("{e}. Everything done so far was rolled back.")
                    }
                    Err(u) if u == KILLED => return Err(u),
                    Err(u) => {
                        run.journal.append("rollback", "failed", &u)?;
                        format!(
                            "{e}. Rolling back also failed: {u}. The journal in {} says what is done.",
                            dir.display()
                        )
                    }
                };
                return Err(msg);
            }
        }
    }
    if hooks.stop_after == Some(completed) && hooks.crash_mid {
        return Err(KILLED.into());
    }
    let summary = run
        .journal
        .states()
        .get("verify")
        .map(|(_, d)| d.clone())
        .unwrap_or_default();
    let failed = summary.contains("failed");
    // A retire that kept an original (or failed partway) is a warning, not a
    // failure: the move is whole and rolling back stays possible.
    let states = run.journal.states();
    let warnings: Vec<String> = ops
        .iter()
        .filter_map(|op| match op {
            Op::Retire(i) => states.get(&op.name()).and_then(|(_, d)| {
                let v: serde_json::Value = serde_json::from_str(d).ok()?;
                let from = &run.plan.moves[*i].from;
                if let Some(why) = v["kept"].as_str() {
                    return Some(format!("{from} was kept in iCloud ({why})"));
                }
                let e = v["error"].as_str()?;
                Some(format!(
                    "{from} was only partly removed from iCloud ({e}), so that old path is incomplete now; roll back to rebuild it, or delete the rest by hand"
                ))
            }),
            _ => None,
        })
        .collect();
    let summary = if warnings.is_empty() {
        summary
    } else {
        format!(
            "{summary}. Warning: {}. The moved folder is whole; remove the original by hand once you have checked it, or roll back",
            warnings.join("; ")
        )
    };
    run.journal
        .append("run", if failed { "failed" } else { "done" }, &summary)?;
    if failed {
        return Err(format!("The move finished but verify found problems: {summary}. Roll back from Settings, Files & storage, or run move-home --rollback."));
    }
    Ok(format!(
        "Moved. {summary}. Backup and journal: {}",
        dir.display()
    ))
}

/// Whether each moved top-level folder can go back: nothing else may sit
/// at its old path (other than the link the move left).
fn undo_precheck(run: &Run<'_>) -> Result<(), String> {
    let touched = run.journal.touched();
    let froms = current_froms(&run.plan.moves);
    for (i, m) in run.plan.moves.iter().enumerate() {
        let name = Op::Move(i).name();
        if !touched.contains(&name)
            || run.journal.undone(&name)
            || froms[i] != m.from
            || case_only(&froms[i], &m.to)
        {
            continue;
        }
        let (from, to) = (Path::new(&m.from), Path::new(&m.to));
        if m.method == METHOD_COPY {
            // The original, or what a retire left, is a folder; a file or
            // anything else there is not ours. (What a partial leftover
            // holds is checked once the later steps are undone.)
            if exists(from)
                && !is_link_to(from, to)
                && !std::fs::symlink_metadata(from).is_ok_and(|x| x.is_dir())
            {
                return Err(format!(
                    "Nothing was rolled back: {} is in the way (something else is at that path now, while {} holds the copied folder). Move it aside and roll back again.",
                    from.display(),
                    to.display()
                ));
            }
            continue;
        }
        let to_real = std::fs::symlink_metadata(to).is_ok_and(|x| !x.file_type().is_symlink());
        if to_real && exists(from) && !is_link_to(from, to) {
            return Err(format!(
                "Nothing was rolled back: {} is in the way (something else is at that path now, while {} holds the moved folder). Move it aside and roll back again.",
                from.display(),
                to.display()
            ));
        }
    }
    Ok(())
}

/// Undo every step any record shows as touched, in reverse order, then put
/// back the files they wrote, then bootstrap again the launch agents booted
/// out. Every part is safe to repeat, so a rollback killed anywhere (even
/// after marking steps undone) finishes on the next try.
fn undo_all(run: &Run<'_>) -> Result<String, String> {
    let ops = ops(&run.plan);
    let touched = run.journal.touched();
    let mut errors = Vec::new();
    let mut n = 0;
    for op in ops.iter().rev() {
        let name = op.name();
        // A step a killed rollback already undid stays undone (its files are
        // restored below whatever its last record says).
        if !touched.contains(&name) || run.journal.undone(&name) {
            continue;
        }
        let detail = run.journal.last_detail(&name);
        match run.undo(op, &detail) {
            Ok(()) => {
                run.journal.append(&name, "undone", "")?;
                n += 1;
            }
            Err(e) if e == KILLED => return Err(e),
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    if run.hooks.kill_before_restore {
        return Err(KILLED.into());
    }
    let mut mapped_back = Vec::new();
    match run.restore_files(&touched) {
        Ok(m) => mapped_back = m,
        Err(e) => errors.push(format!("restore files: {e}")),
    }
    for op in &ops {
        if let Op::Bootout(i) = op {
            if touched.contains(&op.name()) {
                // Out first (a no-op when it isn't loaded), so a repeat
                // never fails on "already loaded".
                let plist = Path::new(&run.plan.launch_agents[*i].path);
                let _ = run.probes.bootout(&run.plan.launch_agents[*i].label);
                if let Err(e) = run.probes.bootstrap(plist) {
                    errors.push(e);
                }
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    let mut msg = format!("{n} steps undone");
    // Parked copies are found by name too: a kill between the rename and its
    // journal marker must still tell the person where the later work is.
    for (i, m) in run.plan.moves.iter().enumerate() {
        let prefix = format!("kept-copy-{i}");
        for e in std::fs::read_dir(&run.dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == prefix || name.starts_with(&format!("{prefix}-")) {
                msg.push_str(&format!(
                    "; the copy of {} held work written after the move, so it was parked at {} instead of deleted (the old original in iCloud was kept)",
                    m.from,
                    s(&e.path())
                ));
            }
        }
    }
    if !mapped_back.is_empty() {
        msg.push_str(&format!(
            "; changed since the move, so mapped back to the old paths instead of restored (later changes kept): {}",
            mapped_back.join(", ")
        ));
    }
    Ok(msg)
}

/// Why a rollback of `dir` would lose work, if it would.
pub(crate) fn rollback_refusal(dir: &Path) -> Option<String> {
    // Once a rollback began (or failed partway), the person already chose
    // it: what it restored must not read as new work and refuse the retry.
    if Journal::new(dir)
        .read()
        .iter()
        .any(|s| s.step == "rollback" && matches!(s.state.as_str(), "started" | "failed"))
    {
        return None;
    }
    let verified: Verified = read_json(&dir.join("verified.json")).ok()?;
    let db = Path::new(&verified.new_db);
    let mut reasons = Vec::new();
    if db.exists() {
        if let Ok(conn) = mp::open_read_only(db) {
            let sessions: i64 = conn
                .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
                .unwrap_or(0);
            let turns: i64 = conn
                .query_row("SELECT COALESCE(max(rowid), 0) FROM turns", [], |r| {
                    r.get(0)
                })
                .unwrap_or(0);
            if sessions != verified.sessions || turns > verified.turns_max {
                reasons.push(format!(
                    "the Sessions database has {} new conversations and {} new turns since the move",
                    (sessions - verified.sessions).max(0),
                    (turns - verified.turns_max).max(0)
                ));
            }
        }
    }
    let mut changed = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(20);
    let root = PathBuf::from(&verified.vault);
    let mut stack = vec![root.clone()];
    'walk: while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            if Instant::now() > deadline {
                break 'walk;
            }
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(m) = std::fs::symlink_metadata(e.path()) else {
                continue;
            };
            if m.is_dir() {
                stack.push(e.path());
            } else if m.is_file() && m.modified().is_ok_and(|t| nanos(t) > verified.at_nanos) {
                changed.push(s(&e.path()));
            }
        }
    }
    if !changed.is_empty() {
        changed.sort();
        let sample: Vec<String> = changed.iter().take(10).cloned().collect();
        reasons.push(format!(
            "{} vault files changed since the move (e.g. {})",
            changed.len(),
            sample.join(", ")
        ));
    }
    if reasons.is_empty() {
        return None;
    }
    Some(format!(
        "Rolling back now would lose new work: {}. Copy what you need out of {} first (the database is {}), or keep the move.",
        reasons.join("; "),
        verified.vault,
        verified.new_db
    ))
}

/// Roll the move in `dir` back (waiting for Hollow to quit first).
pub fn rollback_with(
    dir: &Path,
    probes: &dyn Probes,
    wait_pid: Option<i32>,
) -> Result<String, String> {
    rollback_with_hooks(dir, probes, wait_pid, &RunHooks::default())
}

/// [`rollback_with`] with test hooks.
pub fn rollback_with_hooks(
    dir: &Path,
    probes: &dyn Probes,
    wait_pid: Option<i32>,
    hooks: &RunHooks,
) -> Result<String, String> {
    let _lock = take_lock(dir)?;
    let mut run = Run::load(dir, probes)?;
    run.hooks = hooks.clone();
    if run
        .journal
        .last_outcome()
        .is_some_and(|s| s.step == "rollback" && s.state == "done")
    {
        return Ok("Already rolled back.".into());
    }
    if let Some(pid) = wait_pid {
        wait_for_exit(pid, Duration::from_secs(180))?;
    }
    let new_db = sessions_db(&run.roots, &run.plan.new_stem);
    let deadline = Instant::now() + Duration::from_secs(60);
    while new_db.exists() && crate::hollow_databases::shm_in_use(&new_db) {
        if Instant::now() > deadline {
            return Err(
                "Hollow's Sessions database is still open; nothing was rolled back.".into(),
            );
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    if let Some(why) = rollback_refusal(dir) {
        // Refused: nothing was undone, the move stands.
        run.journal.append("rollback", "refused", &why)?;
        return Err(why);
    }
    // Every moved folder must have a free place to go back to; checked
    // before anything is undone, so a blocked rollback changes nothing.
    if let Err(why) = undo_precheck(&run) {
        run.journal.append("rollback", "refused", &why)?;
        return Err(why);
    }
    run.journal.append("rollback", "started", "")?;
    match undo_all(&run) {
        Err(e) if e == KILLED => Err(e),
        Ok(msg) => {
            run.journal.append("rollback", "done", &msg)?;
            Ok(format!("Rolled back: {msg}."))
        }
        Err(e) => {
            run.journal.append("rollback", "failed", &e)?;
            Err(format!("Rollback stopped: {e}. The journal in {} says what is undone; run it again after fixing the cause.", dir.display()))
        }
    }
}

// ── Status ────────────────────────────────────────────────────────────────

/// The newest `move-home-*` folder with a plan in the Hollow folder's Backups.
pub(crate) fn latest_backup(roots: &Roots) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(roots.backups())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("move-home-"))
        .map(|e| e.path())
        .filter(|p| p.join(PLAN_FILE).is_file())
        .collect();
    dirs.sort();
    dirs.pop()
}

pub(crate) fn status(roots: &Roots) -> Status {
    match latest_backup(roots) {
        Some(dir) => status_in(&dir, roots),
        None => Status {
            state: "none".into(),
            backup_dir: None,
            steps: Vec::new(),
            checks: Vec::new(),
            last_result: None,
            can_roll_back: false,
            rollback_refusal: None,
        },
    }
}

/// The state of the move in `dir` from its journal (and, with
/// `consider_lock`, whether a helper holds it now).
fn classify(dir: &Path, consider_lock: bool) -> &'static str {
    let journal = Journal::new(dir);
    if consider_lock && locked(dir) {
        return "running";
    }
    if journal.read().is_empty() {
        return "staged";
    }
    match journal.last_outcome() {
        // Steps written, no outcome: a run killed partway.
        None => "running",
        Some(s) => match (s.step.as_str(), s.state.as_str()) {
            ("rollback", "done") => "rolledBack",
            // A rollback killed partway: the layout is half undone.
            ("rollback", "started") => "running",
            ("rollback", _) => "failed",
            ("run", "done") => "done",
            _ => "failed",
        },
    }
}

/// Whether the move in `dir` left a half-moved layout that Hollow must not
/// open on: a run or rollback killed partway, or a rollback (or the undo
/// after a failed step) that failed. `Some(reason)`.
pub(crate) fn half_state(dir: &Path) -> Option<String> {
    let journal = Journal::new(dir);
    match classify(dir, false) {
        "running" => Some("a move or rollback was stopped partway".into()),
        "failed" => match journal.last_outcome() {
            Some(s) if s.step == "rollback" => {
                Some(format!("undoing the move failed: {}", s.detail))
            }
            // A run that failed partway and was stopped before its automatic
            // rollback finished: steps are done and not undone (a run whose
            // verify failed is whole, and one that failed in preflight
            // changed nothing).
            _ if !journal.completed("verify") && changed_not_undone(&journal) => {
                Some("a failed move was stopped before it was rolled back".into())
            }
            _ => None,
        },
        _ => None,
    }
}

/// Whether any step that changes something was started or done and not
/// undone since.
fn changed_not_undone(journal: &Journal) -> bool {
    let mut last: BTreeMap<String, String> = BTreeMap::new();
    for s in journal.read() {
        if !matches!(
            s.step.as_str(),
            "preflight" | "snapshot" | "probe" | "verify" | "run" | "rollback"
        ) {
            last.insert(s.step, s.state);
        }
    }
    last.values().any(|st| st != "undone")
}

/// Whether the helper may open Hollow after it ends: only on a finished
/// move, a full rollback, or a run that changed nothing (never on a half
/// state).
pub(crate) fn should_relaunch(dir: &Path) -> bool {
    half_state(dir).is_none()
}

pub(crate) fn status_in(dir: &Path, _roots: &Roots) -> Status {
    let journal = Journal::new(dir);
    let steps = journal.read();
    let checks: Vec<Check> = read_json(&dir.join(VERIFY_FILE)).unwrap_or_default();
    let touched = journal.touched().iter().any(|s| {
        !matches!(
            s.as_str(),
            "preflight" | "snapshot" | "probe" | "run" | "rollback" | "verify"
        )
    });
    let state = classify(dir, true);
    let last_result = journal
        .read()
        .into_iter()
        .rev()
        .find(|s| (s.step == "run" || s.step == "rollback") && s.state != "started")
        .map(|s| s.detail)
        .filter(|d| !d.is_empty());
    let refusal = if matches!(state, "done" | "failed" | "running") && touched {
        rollback_refusal(dir)
    } else {
        None
    };
    let can = matches!(state, "done" | "failed" | "running")
        && touched
        && refusal.is_none()
        && !locked(dir);
    Status {
        state: state.into(),
        backup_dir: Some(s(dir)),
        steps,
        checks,
        last_result,
        can_roll_back: can,
        rollback_refusal: refusal,
    }
}

/// The backup folder a rollback would use, or why it can't run.
pub(crate) fn check_rollback(
    roots: &Roots,
    backup_dir: Option<PathBuf>,
) -> Result<PathBuf, String> {
    let dir = match backup_dir {
        Some(d) => d,
        None => latest_backup(roots).ok_or("There is no move to roll back.")?,
    };
    if !dir.join(PLAN_FILE).is_file() {
        return Err(format!("{} holds no move.", dir.display()));
    }
    let st = status_in(&dir, roots);
    match st.state.as_str() {
        "rolledBack" => Err("That move was already rolled back.".into()),
        "staged" => Err("That move hasn't run yet; there is nothing to roll back.".into()),
        _ if locked(&dir) => Err("The move helper is still working; wait for it to finish.".into()),
        _ if st.rollback_refusal.is_some() => Err(st.rollback_refusal.unwrap_or_default()),
        _ if !st.can_roll_back => {
            Err("Nothing was changed by that move, so there is nothing to roll back.".into())
        }
        _ => Ok(dir),
    }
}

/// What a launch finds of the last move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unfinished {
    /// A run or rollback stopped partway (no helper holds it now, unless
    /// `locked`): the launch rolls it back first.
    Interrupted { dir: PathBuf, locked: bool },
    /// Undoing it failed: Hollow must not open until a person fixes it.
    Failed { dir: PathBuf, reason: String },
}

/// A move left half done, for the launch guard.
pub fn unfinished(roots: &Roots) -> Option<Unfinished> {
    let dir = latest_backup(roots)?;
    if locked(&dir) {
        return (classify(&dir, false) == "running" || half_state(&dir).is_some())
            .then_some(Unfinished::Interrupted { dir, locked: true });
    }
    if classify(&dir, false) == "running" {
        return Some(Unfinished::Interrupted { dir, locked: false });
    }
    let reason = half_state(&dir)?;
    let rollback_failed = Journal::new(&dir)
        .last_outcome()
        .is_some_and(|s| s.step == "rollback");
    Some(if rollback_failed {
        Unfinished::Failed { dir, reason }
    } else {
        // Stopped before its automatic rollback: the launch rolls it back.
        Unfinished::Interrupted { dir, locked: false }
    })
}

/// Every step name of the plan in `dir` (tests count them).
#[cfg(test)]
pub(crate) fn op_count(plan: &Plan) -> usize {
    ops(plan).len()
}

#[cfg(test)]
pub(crate) fn op_names(plan: &Plan) -> Vec<String> {
    ops(plan).iter().map(Op::name).collect()
}
