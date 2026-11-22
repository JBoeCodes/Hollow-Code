//! Build workspaces: a workspace that owns a git worktree (Hollow build
//! programme, PR B1; see docs/history/HOLLOW_BUILD_PLAN.md).
//!
//! Creating one runs `git worktree add -b <branch> <path> <base>` in a repo
//! the user picked, then creates an ordinary outcome workspace and binds the
//! two. While bound, the workspace's `cwd` IS the worktree, so every
//! conversation in it runs there (the agent loads the repo's own
//! `CLAUDE.md` / `AGENTS.md`), and PR 16's repository receipts pick the
//! worktree up as the run's repo target. The workspace's brief, plan and
//! outputs stay in the vault; the chat adds that folder with `--add-dir`.
//!
//! **The binding lives outside the vault** (`build-workspaces.json` in the
//! app data folder, one map per vault). It decides where the user's next
//! agent run executes, so an agent that can write the vault must not be able
//! to repoint it (the same reason permission stamps live outside it).
//!
//! Worktrees go under `<hollowHome>/Worktrees/<repo>/<name>` (default
//! `~/Hollow/Worktrees`): outside iCloud-synced `~/Documents`, where a
//! bundle build's code signature fails on synced xattrs.
//!
//! Git runs as the user's normal git (hooks, LFS and filters stay on: this
//! is the user's own repository, created on their request), with `GIT_*`
//! stripped and no terminal prompt, through `process_group` like every other
//! GUI-owned spawn.

use crate::{process_group, session_store, workspaces};
use chrono::Utc;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use tokio::process::Command;

/// What the workspace list and the UI see.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct BuildInfo {
    /// The repository the worktree was created from (its toplevel).
    pub repo: String,
    pub worktree: String,
    pub branch: String,
    /// What the branch was created from (`origin/main`, or a local branch).
    pub base: String,
    /// The worktree folder is still there. False after "Remove worktree",
    /// when the workspace falls back to its vault folder.
    pub exists: bool,
    /// Runs on (the remote runner, 2026-11-01): `this-mac` or `runner`
    /// chosen for this workspace (a record saved before M6 with `mac-mini`
    /// reads as `runner`), or absent to follow Settings, Devices, Runs on.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub runs_on: Option<String>,
    /// This worktree's own `PORT` / `PW_PORT` (F6 part B), assigned once
    /// when it was bound; absent on a binding from before that, which keeps
    /// `build_slots::port_for`'s hash. Backend only: not sent to the UI.
    #[serde(skip)]
    #[cfg_attr(test, ts(skip))]
    pub port: Option<u16>,
}

/// The primary worktree's lane name (F6 part B): the binding's own
/// worktree, the workspace's `cwd`.
pub const MAIN_LANE: &str = "main";

/// Most extra worktrees one build workspace may have (each first build is
/// cold and a Hollow target is several GB).
pub const MAX_LANES: usize = 8;

/// The ports bound worktrees get, 1500-1899 (`build_slots::port_for`).
const BOUND_PORTS: std::ops::Range<u16> = 1500..1900;

/// An extra worktree of a build workspace (several worktrees per build
/// workspace, F6 part B; docs/BUILD_WORKSPACES_MULTI.md): of the same
/// repository as the primary, with its own branch, folder, build target
/// and port. Kept after Remove worktree, like the primary, with `removedAt`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Lane {
    worktree: String,
    branch: String,
    base: String,
    created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    removed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Binding {
    repo: String,
    worktree: String,
    branch: String,
    base: String,
    created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    removed_at: Option<String>,
    /// Where this workspace's turns run (`this-mac` or `runner`; records
    /// from before M6 say `mac-mini`, read as `runner` and rewritten only
    /// when the person sets Runs on again); absent
    /// follows Settings, Devices, Runs on. Here, not in the brief, for the same reason
    /// as the worktree: an agent that writes the vault must not move where
    /// runs execute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runs_on: Option<String>,
    /// The primary worktree's port (see [`BuildInfo::port`]). Optional and
    /// left out when absent, so a v1 file keeps its hashed port and an
    /// older Hollow ignores it (no schema bump, F6 part B).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    /// Extra worktrees by lane name ([`Lane`]). Left out when empty, so a
    /// single-worktree binding is written exactly as before; an older
    /// Hollow reading a file with lanes ignores them and keeps the primary.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    lanes: BTreeMap<String, Lane>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Store {
    #[serde(default = "schema_v1")]
    schema_version: u32,
    /// Vault file stem (`session_store::vault_file_stem`) → workspace id →
    /// binding.
    #[serde(default)]
    vaults: BTreeMap<String, BTreeMap<String, Binding>>,
}
fn schema_v1() -> u32 {
    1
}

fn store_path() -> Result<PathBuf, String> {
    Ok(crate::agent_settings::app_data_dir()?.join("build-workspaces.json"))
}

/// The parsed store, reloaded when the file's mtime changes. Only the GUI
/// writes it; a missing file is an empty store.
static CACHE: Lazy<
    Mutex<
        Option<(
            Option<SystemTime>,
            BTreeMap<String, BTreeMap<String, Binding>>,
        )>,
    >,
> = Lazy::new(|| Mutex::new(None));
static WRITE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

fn load() -> Result<BTreeMap<String, BTreeMap<String, Binding>>, String> {
    let path = store_path()?;
    let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((seen, map)) = cache.as_ref() {
        if *seen == mtime {
            return Ok(map.clone());
        }
    }
    let map = match std::fs::read_to_string(&path) {
        Ok(text) => {
            let store: Store = serde_json::from_str(&text).map_err(|e| {
                format!(
                    "Build workspace records are unreadable ({e}); move {} aside to reset them",
                    path.display()
                )
            })?;
            if store.schema_version != 1 {
                return Err(format!(
                    "Build workspace records use an unknown schema ({})",
                    store.schema_version
                ));
            }
            store.vaults
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
        Err(e) => return Err(format!("read build workspace records: {e}")),
    };
    *cache = Some((mtime, map.clone()));
    Ok(map)
}

fn save(map: &BTreeMap<String, BTreeMap<String, Binding>>) -> Result<(), String> {
    let path = store_path()?;
    let text = serde_json::to_string_pretty(&Store {
        schema_version: 1,
        vaults: map.clone(),
    })
    .map_err(|e| e.to_string())?;
    let result = crate::vault::atomic_write_private(&path, text.as_bytes());
    // Drop the cache: two writes inside one mtime tick must not read stale.
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    result
}

fn info(binding: &Binding) -> BuildInfo {
    BuildInfo {
        repo: binding.repo.clone(),
        worktree: binding.worktree.clone(),
        branch: binding.branch.clone(),
        base: binding.base.clone(),
        exists: binding.removed_at.is_none() && Path::new(&binding.worktree).is_dir(),
        runs_on: binding
            .runs_on
            .as_deref()
            .map(crate::agent_settings::canonical_place),
        port: binding.port,
    }
}

/// Lane `lane` of `binding` as a [`BuildInfo`]: the lane's own worktree,
/// branch, base and port, the workspace's repository and Runs on.
fn lane_info(binding: &Binding, lane: &Lane) -> BuildInfo {
    BuildInfo {
        repo: binding.repo.clone(),
        worktree: lane.worktree.clone(),
        branch: lane.branch.clone(),
        base: lane.base.clone(),
        exists: lane.removed_at.is_none() && Path::new(&lane.worktree).is_dir(),
        port: lane.port,
        ..info(binding)
    }
}

/// The exclusive lock key a turn in worktree `path` holds
/// (`automated_history::ScopeMode::Worktree`): one turn per worktree.
pub(crate) fn lock_key(path: &Path) -> String {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    format!("worktree:{}", path.display())
}

/// One worktree of a build workspace, as `workspaces.build_worktrees` shows
/// it: its lane name (`main` for the primary) and binding.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LaneView {
    pub name: String,
    pub worktree: String,
    pub branch: String,
    pub base: String,
    pub exists: bool,
    pub port: u16,
}

fn lane_view(name: &str, info: &BuildInfo) -> LaneView {
    LaneView {
        name: name.to_string(),
        worktree: info.worktree.clone(),
        branch: info.branch.clone(),
        base: info.base.clone(),
        exists: info.exists,
        port: crate::build_slots::port_for(info),
    }
}

/// Every worktree of build workspace `id`, the primary (`main`) first, then
/// its lanes by name, removed ones included. Empty when it isn't one.
pub(crate) fn lanes_for(vault: &Path, id: &str) -> Vec<LaneView> {
    let Some(binding) = stored(vault, id) else {
        return Vec::new();
    };
    let mut out = vec![lane_view(MAIN_LANE, &info(&binding))];
    out.extend(
        binding
            .lanes
            .iter()
            .map(|(name, lane)| lane_view(name, &lane_info(&binding, lane))),
    );
    out
}

/// Build workspace `id`'s stored binding, if any (errors read as none).
fn stored(vault: &Path, id: &str) -> Option<Binding> {
    load()
        .ok()?
        .get(&session_store::vault_file_stem(vault))?
        .get(id)
        .cloned()
}

/// How many of build workspace `id`'s extra worktrees are still there
/// (a trash and the primary's removal wait for them).
pub(crate) fn live_lane_count(vault: &Path, id: &str) -> usize {
    stored(vault, id).map_or(0, |b| {
        b.lanes.values().filter(|l| l.removed_at.is_none()).count()
    })
}

/// How a turn in worktree `worktree` of build workspace `id` is admitted
/// (F6 part B): with no extra worktree live, alone in the workspace's
/// folder, as before (a folder chat there is told to work in the worktree,
/// so the two never run at once); with one, alone on that worktree's lock,
/// so the workspace's worktrees and its folder's conversations run at once.
/// A commit, an update or a removal of that worktree waits in the same mode.
/// The primary's is [`ScopeMode::PrimaryWorktree`], so it also waits for a
/// folder chat admitted before the first lane, which was given the primary
/// (delta review: a mode is decided at admission and kept).
pub(crate) fn turn_mode(
    vault: &Path,
    id: &str,
    worktree: &str,
) -> crate::automated_history::ScopeMode {
    use crate::automated_history::ScopeMode;
    if live_lane_count(vault, id) == 0 {
        return ScopeMode::Exclusive;
    }
    let key = lock_key(Path::new(worktree));
    let primary = stored(vault, id).is_some_and(|b| lock_key(Path::new(&b.worktree)) == key);
    if primary {
        ScopeMode::PrimaryWorktree(key)
    } else {
        ScopeMode::Worktree(key)
    }
}

/// How a conversation in build workspace `id`'s vault folder is admitted:
/// while the workspace's primary worktree is there and it has no lane, the
/// chat is told to work in that worktree and given its build folder, so
/// [`ScopeMode::PrimaryFolder`] (it waits for a worktree turn a lane's
/// existence let in, and a later primary turn waits for it); otherwise an
/// ordinary shared run. `None` when `id` isn't a build workspace.
pub(crate) fn folder_mode(vault: &Path, id: &str) -> Option<crate::automated_history::ScopeMode> {
    use crate::automated_history::ScopeMode;
    let binding = stored(vault, id)?;
    let given = binding.removed_at.is_none()
        && Path::new(&binding.worktree).is_dir()
        && binding.lanes.values().all(|l| l.removed_at.is_some());
    Some(if given {
        ScopeMode::PrimaryFolder
    } else {
        ScopeMode::Shared
    })
}

/// Every live worktree of this vault's build workspaces, primaries and
/// lanes, as (workspace id, lane name, info): what must count as bound
/// (worktree hygiene, port owners, media folders).
pub(crate) fn all_worktrees(vault: &Path) -> Vec<(String, String, BuildInfo)> {
    let Some(map) = load()
        .ok()
        .and_then(|map| map.get(&session_store::vault_file_stem(vault)).cloned())
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (id, binding) in &map {
        out.push((id.clone(), MAIN_LANE.to_string(), info(binding)));
        for (name, lane) in &binding.lanes {
            out.push((id.clone(), name.clone(), lane_info(binding, lane)));
        }
    }
    out
}

/// The build binding of workspace `id`, if any. Errors reading the store
/// are logged and read as "no binding", so a damaged file never hides the
/// workspace list (the workspace falls back to its vault folder).
pub fn binding_for(vault: &Path, id: &str) -> Option<BuildInfo> {
    match load() {
        Ok(map) => map
            .get(&session_store::vault_file_stem(vault))?
            .get(id)
            .map(info),
        Err(error) => {
            eprintln!("[build-workspaces] {error}");
            None
        }
    }
}

/// Every build binding of this vault (removed ones too), as (workspace id,
/// info). Errors reading the store read as none, like [`binding_for`].
pub(crate) fn all_bindings(vault: &Path) -> Vec<(String, BuildInfo)> {
    load()
        .ok()
        .and_then(|map| map.get(&session_store::vault_file_stem(vault)).cloned())
        .map(|m| m.iter().map(|(id, b)| (id.clone(), info(b))).collect())
        .unwrap_or_default()
}

/// The repositories this vault's build workspaces came from, each once:
/// repositories with a live binding first, then those with only removed
/// ones, each group newest binding first. Never path order (fix chat 10-20:
/// an old checkout sorted first and answered for the installed app).
pub(crate) fn repos_newest_first(vault: &Path) -> Vec<String> {
    let mut rows: Vec<(bool, String, String)> = load()
        .ok()
        .and_then(|map| map.get(&session_store::vault_file_stem(vault)).cloned())
        .map(|m| {
            m.values()
                .map(|b| (info(b).exists, b.created_at.clone(), b.repo.clone()))
                .collect()
        })
        .unwrap_or_default();
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    let mut out: Vec<String> = Vec::new();
    for (_, _, repo) in rows {
        if !out.contains(&repo) {
            out.push(repo);
        }
    }
    out
}

/// The workspace whose live worktree is exactly `cwd` (canonical paths):
/// its primary or one of its lanes (F6 part B).
pub fn workspace_for_worktree(vault: &Path, cwd: &Path) -> Option<String> {
    worktree_lane(vault, cwd).map(|(id, _)| id)
}

/// The build workspace and lane whose live worktree is exactly `cwd`, as
/// (workspace id, lane name; `main` for the primary).
pub(crate) fn worktree_lane(vault: &Path, cwd: &Path) -> Option<(String, String)> {
    worktree_build(vault, cwd).map(|(id, lane, _)| (id, lane))
}

/// [`worktree_lane`] with that worktree's own [`BuildInfo`] (a lane's
/// branch, folder and port; the workspace's repository and Runs on).
pub(crate) fn worktree_build(vault: &Path, cwd: &Path) -> Option<(String, String, BuildInfo)> {
    let map = load().ok()?;
    let bindings = map.get(&session_store::vault_file_stem(vault))?;
    let cwd = cwd.canonicalize().ok()?;
    let here = |path: &str| Path::new(path).canonicalize().ok().as_ref() == Some(&cwd);
    bindings.iter().find_map(|(id, b)| {
        if b.removed_at.is_none() && here(&b.worktree) {
            return Some((id.clone(), MAIN_LANE.to_string(), info(b)));
        }
        b.lanes.iter().find_map(|(name, lane)| {
            (lane.removed_at.is_none() && here(&lane.worktree))
                .then(|| (id.clone(), name.clone(), lane_info(b, lane)))
        })
    })
}

/// The Runs on choice of the build workspace whose live worktree is `cwd`:
/// `Some(choice)` for one (`None` inside: follow Settings, Devices, Runs on), `None`
/// when `cwd` isn't a build workspace's worktree.
pub(crate) fn runs_on_for_cwd(vault: &Path, cwd: &Path) -> Option<Option<String>> {
    let id = workspace_for_worktree(vault, cwd)?;
    let map = load().ok()?;
    let binding = map.get(&session_store::vault_file_stem(vault))?.get(&id)?;
    Some(
        binding
            .runs_on
            .as_deref()
            .map(crate::agent_settings::canonical_place),
    )
}

/// Set build workspace `id`'s own Runs on choice (`this-mac` or `runner`;
/// `mac-mini`, its old name, is accepted and saved as `runner`), or clear it (`None` or `default`: follow Settings,
/// Agents). Returns the binding as the UI sees it.
pub fn set_runs_on(vault: &Path, id: &str, runs_on: Option<&str>) -> Result<BuildInfo, String> {
    store_runs_on(vault, id, runs_on, false)
}

/// Build workspace `id`'s own Runs on exactly as stored (an old `mac-mini`
/// kept), for [`restore_runs_on`].
pub(crate) fn stored_runs_on(vault: &Path, id: &str) -> Option<String> {
    load()
        .ok()?
        .get(&session_store::vault_file_stem(vault))?
        .get(id)?
        .runs_on
        .clone()
}

/// Put back a value [`stored_runs_on`] read (the phone's failed-send undo):
/// an old `mac-mini` goes back as it was, so an undo never rewrites a
/// choice the person didn't change (M6 review).
pub(crate) fn restore_runs_on(
    vault: &Path,
    id: &str,
    stored: Option<&str>,
) -> Result<BuildInfo, String> {
    store_runs_on(vault, id, stored, true)
}

fn store_runs_on(
    vault: &Path,
    id: &str,
    runs_on: Option<&str>,
    keep_old_name: bool,
) -> Result<BuildInfo, String> {
    let value = match runs_on.map(str::trim) {
        None | Some("") | Some("default") => None,
        Some(v) => match crate::agent_settings::place_choice(v) {
            Some(place) => Some(place.to_string()),
            None => return Err(format!("Runs on is this-mac or runner, not {v}")),
        },
    };
    let old = crate::agent_settings::LEGACY_RUNNER;
    let kept = keep_old_name && runs_on.map(str::trim) == Some(old);
    // Under the write lock (F6 part B review): a load-then-put could drop a
    // lane added meanwhile.
    modify(vault, id, |binding, _| {
        binding.runs_on = if kept { Some(old.into()) } else { value };
        Ok(info(binding))
    })
}

/// Whether a workspace other than `id` has, or had before Remove worktree,
/// its worktree at `path`. Reads the bindings rather than the briefs, so a
/// workspace whose brief can't be read still counts; unreadable bindings
/// count as yes.
pub(crate) fn worktree_of_another(vault: &Path, id: &str, path: &str) -> bool {
    let Ok(map) = load() else { return true };
    map.get(&session_store::vault_file_stem(vault))
        .is_some_and(|bindings| {
            bindings.iter().any(|(other, b)| {
                other != id && (b.worktree == path || b.lanes.values().any(|l| l.worktree == path))
            })
        })
}

/// Change build workspace `id`'s stored binding in place under the write
/// lock, `f` seeing it and every binding of this vault as they were.
fn modify<T>(
    vault: &Path,
    id: &str,
    f: impl FnOnce(&mut Binding, &BTreeMap<String, Binding>) -> Result<T, String>,
) -> Result<T, String> {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut map = load()?;
    let stem = session_store::vault_file_stem(vault);
    let all = map.get(&stem).cloned().unwrap_or_default();
    let binding = map
        .get_mut(&stem)
        .and_then(|m| m.get_mut(id))
        .ok_or_else(|| format!("{id} is not a build workspace"))?;
    let out = f(binding, &all)?;
    save(&map)?;
    Ok(out)
}

/// A port for a new worktree at `worktree` (F6 part B): its hashed port
/// (`build_slots::hashed_port`) when no other live worktree bound in this
/// vault uses it, else the next free one in 1500-1899; `None` (the hash
/// then) when all 400 are taken. Ends the hash's chance of two build
/// workspaces landing on one port.
fn assign_port(bindings: &BTreeMap<String, Binding>, worktree: &str) -> Option<u16> {
    let effective = |port: Option<u16>, tree: &str| {
        port.unwrap_or_else(|| crate::build_slots::hashed_port(tree))
    };
    let mut taken = std::collections::BTreeSet::new();
    for b in bindings.values() {
        if b.removed_at.is_none() {
            taken.insert(effective(b.port, &b.worktree));
        }
        for lane in b.lanes.values().filter(|l| l.removed_at.is_none()) {
            taken.insert(effective(lane.port, &lane.worktree));
        }
    }
    let len = BOUND_PORTS.end - BOUND_PORTS.start;
    let start = crate::build_slots::hashed_port(worktree).saturating_sub(BOUND_PORTS.start) % len;
    (0..len)
        .map(|i| BOUND_PORTS.start + (start + i) % len)
        .find(|port| !taken.contains(port))
}

/// Record a new build workspace's binding with its own port, assigned under
/// the write lock so two creates never pick one port.
fn put_new(vault: &Path, id: &str, mut binding: Binding) -> Result<(), String> {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut map = load()?;
    let stem = session_store::vault_file_stem(vault);
    let bindings = map.entry(stem).or_default();
    binding.port = assign_port(bindings, &binding.worktree);
    bindings.insert(id.to_string(), binding);
    save(&map)
}

fn put(vault: &Path, id: &str, binding: Option<Binding>) -> Result<(), String> {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut map = load()?;
    let stem = session_store::vault_file_stem(vault);
    match binding {
        Some(binding) => {
            map.entry(stem).or_default().insert(id.to_string(), binding);
        }
        None => {
            if let Some(v) = map.get_mut(&stem) {
                v.remove(id);
            }
        }
    }
    save(&map)
}

// ------------------------------------------------------------------ git

pub(crate) static GIT: Lazy<PathBuf> = Lazy::new(|| {
    ["/opt/homebrew/bin/git", "/usr/local/bin/git"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("git"))
});

/// What one git call printed. `stdout` is untrimmed (diff text needs its
/// exact bytes); `truncated` says the read stopped at the caller's cap.
pub(crate) struct GitOutput {
    pub stdout: Vec<u8>,
    pub stderr: String,
    /// The exit code; `None` when git was killed by a signal (a capped read
    /// usually ends in SIGPIPE, so check `truncated` first).
    pub code: Option<i32>,
    pub truncated: bool,
}

/// Config every Hollow git in a build worktree runs with: no fsmonitor
/// program, and no look inside submodules (`git add -A` still stages a
/// submodule's new commit; the commit check asks for it explicitly).
pub(crate) const SUBMODULE_SAFE: [&str; 8] = [
    "-c",
    "core.fsmonitor=false",
    "-c",
    "diff.ignoreSubmodules=all",
    "-c",
    "submodule.recurse=false",
    "-c",
    "status.submoduleSummary=false",
];

/// The user's git as a command: `GIT_*` stripped, no terminal prompt, C
/// locale, `-C dir`. Tests never read the developer's own git config.
pub(crate) fn git_command(dir: &Path) -> Command {
    let mut cmd = Command::new(&*GIT);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(&key);
        }
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .arg("-C")
        .arg(dir);
    // A Codex auto turn can write the worktree's index and add a submodule
    // whose `.git` names a folder with a `core.fsmonitor` program; status
    // and diff would start a child git there, unsandboxed. These are
    // forwarded to any child git too (review 2026-11-12). Hooks stay on.
    cmd.args(SUBMODULE_SAFE);
    // A worktree a Codex auto run could write: its `commondir` is put back
    // and git pinned to the recorded folders, never that file.
    if let Some((git_dir, common)) = crate::agent::restore_codex_worktree_git(dir) {
        cmd.env("GIT_DIR", git_dir).env("GIT_COMMON_DIR", common);
        if let Some(top) = crate::agent::codex_guarded_worktree(dir) {
            cmd.env("GIT_WORK_TREE", top);
        }
    } else {
        // No snapshot (no auto turn since Hollow started): a linked
        // worktree is still pinned from its `.git` file alone, and one whose
        // layout doesn't hold gets no git (a git folder that can't exist).
        match crate::agent::worktree_git_pins(dir) {
            crate::agent::WorktreeGitPins::NotLinked => {}
            crate::agent::WorktreeGitPins::Pinned(git_dir, common, top) => {
                cmd.env("GIT_DIR", git_dir)
                    .env("GIT_COMMON_DIR", common)
                    .env("GIT_WORK_TREE", top);
            }
            crate::agent::WorktreeGitPins::Refused => {
                let refused = "/nonexistent/hollow-refused-worktree-git";
                cmd.env("GIT_DIR", refused).env("GIT_COMMON_DIR", refused);
            }
        }
    }
    #[cfg(test)]
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    cmd
}

/// Run the user's git in `dir` through `process_group`, keeping at most
/// `cap` bytes of stdout. Any exit code is returned; only a spawn failure or
/// a timeout is an error.
pub(crate) async fn git_raw(
    dir: &Path,
    args: &[&str],
    limit: Duration,
    cap: usize,
) -> Result<GitOutput, String> {
    let mut cmd = git_command(dir);
    cmd.args(args);
    let out = process_group::run_to_completion_capped(&mut cmd, None, limit, None, cap)
        .await
        .map_err(|e| format!("could not run git: {e}"))?;
    if out.timed_out {
        return Err(format!(
            "git {} took longer than {}s",
            args.first().unwrap_or(&""),
            limit.as_secs()
        ));
    }
    Ok(GitOutput {
        code: out.status.and_then(|s| s.code()),
        stdout: out.stdout,
        stderr: out.stderr,
        truncated: out.stdout_truncated,
    })
}

/// The last line of git's stderr, or a generic message.
pub(crate) fn git_error(args: &[&str], stderr: &str) -> String {
    let detail = stderr.trim();
    if detail.is_empty() {
        format!("git {} failed", args.join(" "))
    } else {
        detail.lines().last().unwrap_or(detail).to_string()
    }
}

/// Run the user's git in `dir`; trimmed stdout on success, a readable error
/// otherwise.
pub(crate) async fn git(dir: &Path, args: &[&str], limit: Duration) -> Result<String, String> {
    let out = git_raw(dir, args, limit, usize::MAX).await?;
    if out.code != Some(0) {
        return Err(git_error(args, &out.stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub(crate) const QUICK: Duration = Duration::from_secs(20);

/// A branch-safe slug: lowercase ASCII letters, digits and single dashes.
pub(crate) fn slug(text: &str, max: usize) -> String {
    let mut out = String::new();
    for c in text.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= max {
            break;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "work".into()
    } else {
        out
    }
}

/// `refs/remotes/origin/main` → `origin/main`.
pub(crate) fn remote_head_to_base(symbolic: &str) -> Option<String> {
    symbolic
        .trim()
        .strip_prefix("refs/remotes/")
        .filter(|b| b.contains('/'))
        .map(str::to_string)
}

/// The repository toplevel of `path`, which must BE that toplevel.
async fn repo_toplevel(path: &Path) -> Result<PathBuf, String> {
    let path = path
        .canonicalize()
        .map_err(|_| format!("{} does not exist", path.display()))?;
    let top = git(&path, &["rev-parse", "--show-toplevel"], QUICK)
        .await
        .map_err(|_| format!("{} is not a git repository", path.display()))?;
    let top = PathBuf::from(top)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if top != path {
        return Err(format!(
            "Pick the repository's top folder ({}), not a folder inside it",
            top.display()
        ));
    }
    Ok(top)
}

async fn branch_exists(repo: &Path, branch: &str) -> bool {
    git(
        repo,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
        QUICK,
    )
    .await
    .is_ok()
}

/// What a new build workspace needs.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateInput {
    pub title: String,
    pub goal: String,
    pub repo: String,
    /// Empty: `hollow/<title slug>`, numbered when taken.
    #[serde(default)]
    pub branch: Option<String>,
    /// Empty: origin's default branch (after a fetch), else the repo's
    /// current branch.
    #[serde(default)]
    pub base: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Created {
    pub workspace: workspaces::Workspace,
    /// Non-fatal problems (a failed fetch: the base may be behind origin).
    pub warnings: Vec<String>,
}

/// Error-message prefix for a create refused because its base is too far
/// behind main (OC5).
pub const STALE_BASE_ERROR: &str = "STALE_BASE";
/// Default [`BasePolicy::limit`]: Orca's number. Hollow has no data of its
/// own yet on how far behind a lane's base can be before its merge hurts.
pub const STALE_BASE_LIMIT: u32 = 20;

/// How far behind main a new build workspace's base may be (OC5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BasePolicy {
    /// A base this many commits behind main, or more, is refused.
    pub limit: u32,
    /// Branch from a stale base anyway.
    pub allow_stale: bool,
}

impl Default for BasePolicy {
    fn default() -> Self {
        Self {
            limit: STALE_BASE_LIMIT,
            allow_stale: false,
        }
    }
}

/// The refusal for a base `behind` commits behind `main`, or `None` when it
/// is fresh enough or the policy allows it.
pub(crate) fn stale_base_refusal(
    base: &str,
    main: &str,
    behind: u32,
    policy: BasePolicy,
) -> Option<String> {
    (!policy.allow_stale && policy.limit > 0 && behind >= policy.limit).then(|| {
        format!(
            "{STALE_BASE_ERROR}: {base} is {behind} commits behind {main} (the limit is {}). Nothing was created; merge {main} into {base} first, or pass allowStaleBase to branch from it anyway.",
            policy.limit
        )
    })
}

/// The repo's main line as this Mac last saw it: origin's default branch,
/// else `origin/main` or `origin/master`, else a local `main` or `master`.
async fn main_ref(repo: &Path) -> Option<String> {
    if let Some(base) = git(
        repo,
        &["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
        QUICK,
    )
    .await
    .ok()
    .and_then(|s| remote_head_to_base(&s))
    {
        return Some(base);
    }
    for guess in ["origin/main", "origin/master", "main", "master"] {
        if git(repo, &["rev-parse", "--verify", "--quiet", guess], QUICK)
            .await
            .is_ok()
        {
            return Some(guess.to_string());
        }
    }
    None
}

/// Commits on `main` that `base` lacks; `None` when git can't say.
async fn commits_behind(repo: &Path, base: &str, main: &str) -> Option<u32> {
    git(
        repo,
        &["rev-list", "--count", &format!("{base}..{main}")],
        QUICK,
    )
    .await
    .ok()
    .and_then(|n| n.trim().parse().ok())
}

/// Create the worktree, then the workspace, then bind them. Any failure
/// after the worktree exists removes it again (and the new branch), so a
/// refused create leaves nothing behind. The base must be fresh by the
/// default [`BasePolicy`]; [`create_with`] takes another.
#[cfg_attr(not(test), allow(dead_code))] // callers use create_with since OC5
pub async fn create(
    vault: &Path,
    worktrees_root: &Path,
    input: CreateInput,
) -> Result<Created, String> {
    create_with(vault, worktrees_root, input, BasePolicy::default()).await
}

/// [`create`] with a [`BasePolicy`]: a base `policy.limit` or more commits
/// behind main is refused (`STALE_BASE: …`, naming the fix) before anything
/// is made, unless `policy.allow_stale`.
pub async fn create_with(
    vault: &Path,
    worktrees_root: &Path,
    input: CreateInput,
    policy: BasePolicy,
) -> Result<Created, String> {
    let title = input.title.trim().to_string();
    let repo = repo_toplevel(Path::new(input.repo.trim())).await?;
    if repo.starts_with(vault.canonicalize().unwrap_or_else(|_| vault.to_path_buf())) {
        return Err("The repository is inside the vault; pick a project folder outside it".into());
    }
    let mut warnings = Vec::new();
    // Base: explicit, else origin's default branch (fetched), else HEAD's branch.
    let base = match input
        .base
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
    {
        Some(base) => base.to_string(),
        None => {
            let origin = git(&repo, &["remote", "get-url", "origin"], QUICK)
                .await
                .is_ok();
            let mut base = None;
            if origin {
                if let Err(e) = git(
                    &repo,
                    &["fetch", "--quiet", "origin"],
                    Duration::from_secs(90),
                )
                .await
                {
                    warnings.push(format!("Could not fetch origin ({e}); the branch starts from what this Mac last saw."));
                }
                base = git(
                    &repo,
                    &["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
                    QUICK,
                )
                .await
                .ok()
                .and_then(|s| remote_head_to_base(&s));
                if base.is_none() {
                    for guess in ["origin/main", "origin/master"] {
                        if git(&repo, &["rev-parse", "--verify", "--quiet", guess], QUICK)
                            .await
                            .is_ok()
                        {
                            base = Some(guess.to_string());
                            break;
                        }
                    }
                }
            }
            match base {
                Some(base) => base,
                None => git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"], QUICK).await?,
            }
        }
    };
    git(
        &repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{base}^{{commit}}"),
        ],
        QUICK,
    )
    .await
    .map_err(|_| format!("{base} is not a branch or commit in this repository"))?;
    // OC5: refuse a base too far behind main before anything is made.
    if let Some(main) = main_ref(&repo).await {
        if let Some(behind) = commits_behind(&repo, &base, &main).await {
            if let Some(refusal) = stale_base_refusal(&base, &main, behind, policy) {
                return Err(refusal);
            }
        }
    }
    // BI7: the repository's fingerprint before the connect touches anything,
    // for the first-connect draft's "nothing changed" line.
    let before = crate::first_connect::fingerprint_async(repo.clone()).await;
    // Branch: explicit (must be new), else hollow/<slug>, numbered when taken.
    let branch = match input
        .branch
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
    {
        Some(branch) => {
            git(&repo, &["check-ref-format", "--branch", branch], QUICK)
                .await
                .map_err(|_| format!("{branch} is not a valid branch name"))?;
            if branch_exists(&repo, branch).await {
                return Err(format!("Branch {branch} already exists; pick another name"));
            }
            branch.to_string()
        }
        None => {
            let stem = format!("hollow/{}", slug(&title, 40));
            let mut branch = stem.clone();
            let mut n = 2;
            while branch_exists(&repo, &branch).await {
                branch = format!("{stem}-{n}");
                n += 1;
            }
            branch
        }
    };
    let path = new_worktree_path(worktrees_root, &repo, &branch)?;
    let path_arg = path.to_string_lossy().to_string();
    let worktree = add_worktree(worktrees_root, &repo, &branch, &path, &base).await?;
    let vault_owned = vault.to_path_buf();
    let (t, g) = (title.clone(), input.goal.clone());
    let workspace =
        match tokio::task::spawn_blocking(move || workspaces::create(&vault_owned, &t, &g)).await {
            Ok(Ok(w)) => w,
            Ok(Err(e)) => {
                undo(repo.clone(), path_arg, branch).await;
                return Err(e);
            }
            Err(e) => {
                undo(repo.clone(), path_arg, branch).await;
                return Err(e.to_string());
            }
        };
    let binding = Binding {
        repo: repo.to_string_lossy().into(),
        worktree: worktree.to_string_lossy().into(),
        branch: branch.clone(),
        base,
        created_at: Utc::now().to_rfc3339(),
        removed_at: None,
        runs_on: None,
        port: None,
        lanes: BTreeMap::new(),
    };
    if let Err(e) = put_new(vault, &workspace.id, binding) {
        undo(repo.clone(), path_arg, branch).await;
        let _ = std::fs::remove_dir_all(vault.join(&workspace.path));
        return Err(format!("Could not record the build workspace: {e}"));
    }
    // BI7: describe-only first connect. Best effort: a failure is a warning,
    // and a repository that changed meanwhile is one too.
    match crate::first_connect::write_draft(vault, &workspace.id, &repo, before).await {
        Ok(w) => {
            eprintln!(
                "[build-workspaces] first-connect draft {} ({} attention item(s))",
                w.path.display(),
                w.attention
            );
            if let crate::first_connect::Proof::Changed { total, .. } = w.proof {
                warnings.push(format!(
                    "{total} path(s) in the repository changed during the connect (possibly its own hooks or filters, run by git worktree add); see {}.",
                    w.path.display()
                ));
            }
        }
        Err(e) => warnings.push(format!("Could not write the first-connect draft: {e}")),
    }
    // OC15: the repo's setup hook starts in the background only in a version
    // the person approved (by content hash), its result in the workspace's
    // outputs; otherwise a warning. After the draft, so the draft describes
    // the connect alone.
    if let Some(warning) = crate::setup_hook::on_create(vault, &workspace.id, &worktree).await {
        warnings.push(warning);
    }
    let workspace = tokio::task::spawn_blocking({
        let vault = vault.to_path_buf();
        let id = workspace.id.clone();
        move || workspaces::read(&vault, &id).map(|d| d.workspace)
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(Created {
        workspace,
        warnings,
    })
}

/// Where a new worktree for `branch` of `repo` goes:
/// `<worktrees root>/<repo>/<branch leaf>`, numbered when taken.
fn new_worktree_path(worktrees_root: &Path, repo: &Path, branch: &str) -> Result<PathBuf, String> {
    let repo_name = slug(
        &repo
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        60,
    );
    let parent = worktrees_root.join(&repo_name);
    std::fs::create_dir_all(&parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    let leaf = slug(branch.rsplit('/').next().unwrap_or(branch), 60);
    let mut path = parent.join(&leaf);
    let mut n = 2;
    while path.exists() {
        path = parent.join(format!("{leaf}-{n}"));
        n += 1;
    }
    Ok(path)
}

/// `git worktree add -b <branch> <path> <base>` in `repo`, then a sibling's
/// `node_modules` cloned in (OC7). Returns the worktree's canonical path.
async fn add_worktree(
    worktrees_root: &Path,
    repo: &Path,
    branch: &str,
    path: &Path,
    base: &str,
) -> Result<PathBuf, String> {
    let path_arg = path.to_string_lossy().to_string();
    // `--no-track`: a branch started from `origin/main` would otherwise track
    // it, and look already pushed (B2 found this).
    git(
        repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "--no-track",
            "-b",
            branch,
            &path_arg,
            base,
        ],
        Duration::from_secs(600),
    )
    .await
    .map_err(|e| format!("Could not create the worktree: {e}"))?;
    let worktree = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    // OC7: a sibling's install of the same lock file, cloned (best effort; Build
    // app's release build keeps its own clean `npm ci`).
    match seed_new_worktree(
        &worktree,
        &[worktrees_root.to_path_buf(), repo.to_path_buf()],
    )
    .await
    {
        Seed::Cloned(from) => eprintln!(
            "[build-workspaces] node_modules cloned from {} (same package-lock.json)",
            from.display()
        ),
        Seed::Skipped(why) => eprintln!("[build-workspaces] node_modules not seeded: {why}"),
    }
    Ok(worktree)
}

/// Take back a worktree [`add_worktree`] made, and its new branch.
async fn undo(repo: PathBuf, path: String, branch: String) {
    crate::agent::forget_codex_worktree_guard(Path::new(&path));
    let _ = git(
        &repo,
        &["worktree", "remove", "--force", &path],
        Duration::from_secs(60),
    )
    .await;
    let _ = git(&repo, &["branch", "-D", &branch], QUICK).await;
}

/// What "Add worktree" asks for (F6 part B): a lane name, unique in the
/// workspace, and optionally its new branch and base.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaneInput {
    pub name: String,
    pub branch: Option<String>,
    /// Default: the primary's base.
    pub base: Option<String>,
}

/// A lane name: a slug unique in its workspace, never `main` (the primary).
fn lane_name(name: &str) -> Result<String, String> {
    // `slug` reads an empty name as `work`: ask for a real one instead.
    if !name.chars().any(|c| c.is_ascii_alphanumeric()) {
        return Err("Name the worktree with letters or digits".into());
    }
    let name = slug(name.trim(), 40);
    if name == MAIN_LANE {
        return Err("main is the workspace's own worktree; pick another name".into());
    }
    Ok(name)
}

/// Add worktree (several worktrees per build workspace, F6 part B): a new
/// branch and worktree of build workspace `id`'s repository, bound as lane
/// `input.name` with a port of its own. Its turns hold it alone and run
/// beside the primary's and the other lanes' (`chat::run_scope_mode`). The
/// workspace's primary must still be there. Nothing is recorded when git
/// fails, and the worktree and branch are taken back when recording does.
pub(crate) async fn bind_lane(
    vault: &Path,
    worktrees_root: &Path,
    id: &str,
    input: LaneInput,
) -> Result<LaneView, String> {
    let binding = stored(vault, id).ok_or_else(|| format!("{id} is not a build workspace"))?;
    if binding.removed_at.is_some() || !Path::new(&binding.worktree).is_dir() {
        return Err(
            "This workspace's own worktree was removed; a worktree is added beside it".into(),
        );
    }
    let name = lane_name(&input.name)?;
    let live = |b: &Binding, name: &str| b.lanes.get(name).is_some_and(|l| l.removed_at.is_none());
    if live(&binding, &name) {
        return Err(format!(
            "This workspace already has a worktree named {name}"
        ));
    }
    if binding
        .lanes
        .values()
        .filter(|l| l.removed_at.is_none())
        .count()
        >= MAX_LANES
    {
        return Err(format!(
            "A build workspace has at most {MAX_LANES} extra worktrees; remove one first"
        ));
    }
    let repo = PathBuf::from(&binding.repo);
    let base = input
        .base
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .unwrap_or(&binding.base)
        .to_string();
    git(
        &repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{base}^{{commit}}"),
        ],
        QUICK,
    )
    .await
    .map_err(|_| format!("{base} is not a branch or commit in this repository"))?;
    if let Some(main) = main_ref(&repo).await {
        if let Some(behind) = commits_behind(&repo, &base, &main).await {
            if let Some(refusal) = stale_base_refusal(&base, &main, behind, BasePolicy::default()) {
                return Err(refusal);
            }
        }
    }
    let branch = match input
        .branch
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
    {
        Some(branch) => {
            git(&repo, &["check-ref-format", "--branch", branch], QUICK)
                .await
                .map_err(|_| format!("{branch} is not a valid branch name"))?;
            if branch_exists(&repo, branch).await {
                return Err(format!("Branch {branch} already exists; pick another name"));
            }
            branch.to_string()
        }
        None => {
            let stem = format!("{}-{name}", binding.branch);
            let mut branch = stem.clone();
            let mut n = 2;
            while branch_exists(&repo, &branch).await {
                branch = format!("{stem}-{n}");
                n += 1;
            }
            branch
        }
    };
    let path = new_worktree_path(worktrees_root, &repo, &branch)?;
    let path_arg = path.to_string_lossy().to_string();
    let worktree = add_worktree(worktrees_root, &repo, &branch, &path, &base).await?;
    let lane = Lane {
        worktree: worktree.to_string_lossy().into(),
        branch: branch.clone(),
        base,
        created_at: Utc::now().to_rfc3339(),
        removed_at: None,
        port: None,
    };
    let recorded = modify(vault, id, |b, all| {
        // Again under the lock: the store may have changed during git.
        if b.removed_at.is_some() {
            return Err("This workspace's own worktree was removed meanwhile".to_string());
        }
        if live(b, &name) {
            return Err(format!(
                "This workspace already has a worktree named {name}"
            ));
        }
        if b.lanes.values().filter(|l| l.removed_at.is_none()).count() >= MAX_LANES {
            return Err(format!(
                "A build workspace has at most {MAX_LANES} extra worktrees; remove one first"
            ));
        }
        let lane = Lane {
            port: assign_port(all, &lane.worktree),
            ..lane
        };
        let view = lane_view(&name, &lane_info(b, &lane));
        b.lanes.insert(name.clone(), lane);
        Ok(view)
    });
    if recorded.is_err() {
        undo(repo, path_arg, branch).await;
    }
    recorded
}

/// Bind `worktree` to build workspace `id` as lane `name` without git
/// (tests of the run paths), with a port assigned as [`bind_lane`] does.
#[cfg(test)]
pub(crate) fn bind_lane_for_test(vault: &Path, id: &str, name: &str, worktree: &Path) {
    let worktree = worktree
        .canonicalize()
        .unwrap_or_else(|_| worktree.to_path_buf());
    let name = lane_name(name).unwrap();
    modify(vault, id, |b, all| {
        let tree = worktree.to_string_lossy().to_string();
        let port = assign_port(all, &tree);
        b.lanes.insert(
            name,
            Lane {
                worktree: tree,
                branch: "hollow/test-lane".into(),
                base: "main".into(),
                created_at: Utc::now().to_rfc3339(),
                removed_at: None,
                port,
            },
        );
        Ok(())
    })
    .unwrap();
}

/// Mark lane `name` of build workspace `id` removed, without git (tests).
#[cfg(test)]
pub(crate) fn mark_lane_removed_for_test(vault: &Path, id: &str, name: &str) {
    modify(vault, id, |b, _| {
        b.lanes.get_mut(name).expect("a lane").removed_at = Some(Utc::now().to_rfc3339());
        Ok(())
    })
    .unwrap();
}

/// Remove worktree `lane` of build workspace `id` (`main`: the primary,
/// [`remove_for`]). Like the primary's: the branch is kept, unsaved work
/// refuses without `force` (`WORKTREE_UNSAVED`), and it waits for a turn
/// holding that worktree or a run holding the vault (but `caller`'s own);
/// a conversation in the workspace's folder or another worktree doesn't
/// stop it. Its build target goes with it.
pub(crate) async fn remove_lane(
    vault: &Path,
    id: &str,
    lane: &str,
    force: bool,
    caller: Option<&str>,
) -> Result<(), String> {
    if lane.trim().is_empty() || lane.trim() == MAIN_LANE {
        return remove_for(vault, id, force, caller).await;
    }
    // Named as Add worktree stored it.
    let lane = lane_name(lane)?;
    let lane = lane.as_str();
    let binding = stored(vault, id).ok_or("This workspace has no worktree")?;
    let entry = binding
        .lanes
        .get(lane)
        .cloned()
        .ok_or_else(|| format!("This workspace has no worktree named {lane}"))?;
    if entry.removed_at.is_some() {
        return Err(format!("Worktree {lane} was already removed"));
    }
    let mode = crate::automated_history::ScopeMode::Worktree(lock_key(Path::new(&entry.worktree)));
    refuse_removal_while_busy(vault, id, &mode, caller)?;
    // Held for the whole removal (review): no turn starts in the worktree
    // while git removes it.
    let reservation = reserve_for_removal(vault, id, mode)?;
    let removed = remove_tree(
        &binding.repo,
        &entry.worktree,
        &entry.base,
        force,
        || async { None },
    )
    .await;
    let _ = reservation.finish(if removed.is_ok() { "success" } else { "error" });
    removed?;
    modify(vault, id, |b, _| {
        if let Some(l) = b.lanes.get_mut(lane) {
            l.removed_at = Some(Utc::now().to_rfc3339());
        }
        Ok(())
    })?;
    let (repo, worktree) = (binding.repo.clone(), entry.worktree.clone());
    tokio::task::spawn_blocking(move || crate::build_slots::forget_target(&repo, &worktree));
    Ok(())
}

/// Reserve worktree `mode` of workspace `id` while it is removed, so no
/// turn starts in it meanwhile.
fn reserve_for_removal(
    vault: &Path,
    id: &str,
    mode: crate::automated_history::ScopeMode,
) -> Result<crate::automated_history::RunLease, String> {
    crate::automated_history::try_begin_scoped_mode(
        vault,
        &format!("workspaces/{id}"),
        mode,
        REMOVE_RESERVATION_SOURCE,
        None,
    )
    .map_err(|e| match e {
        crate::automated_history::BeginRunError::Busy(_) => LANE_BUSY.to_string(),
        crate::automated_history::BeginRunError::Failed(why) => {
            format!("Hollow couldn't reserve this worktree to remove it: {why}")
        }
    })
}

/// The History source of the reservation [`remove_lane`] and the primary's
/// removal hold.
pub(crate) const REMOVE_RESERVATION_SOURCE: &str = "buildRemoveWorktree";

/// Why a worktree's removal waits for a turn in it.
const LANE_BUSY: &str =
    "A conversation is working in this worktree; remove it when its turn finishes";

/// A worktree's removal waits for a turn holding it (in `mode`, the mode
/// that worktree's turns take: [`turn_mode`]) and for a run holding the
/// vault but `caller`'s.
fn refuse_removal_while_busy(
    vault: &Path,
    id: &str,
    mode: &crate::automated_history::ScopeMode,
    caller: Option<&str>,
) -> Result<(), String> {
    let callers_own = caller.is_some_and(|run| crate::automated_history::lease_is_run(vault, run));
    if crate::automated_history::lease_holder(vault).is_some() && !callers_own {
        return Err("An agent run is using the vault; remove the worktree when it finishes".into());
    }
    if crate::automated_history::mode_holder(vault, &format!("workspaces/{id}"), mode).is_some() {
        return Err(LANE_BUSY.into());
    }
    Ok(())
}

/// Why a worktree can't be removed without `force`.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoveRefusal {
    pub uncommitted: usize,
    pub unpushed: usize,
}

/// Remove workspace `id`'s worktree. Without `force`, refuses (with the
/// counts, as `WORKTREE_UNSAVED: {json}`) when it has uncommitted changes or
/// commits that neither its base nor any remote has. The branch is kept; the workspace falls back to
/// its vault folder and its conversations stay in history.
pub async fn remove(vault: &Path, id: &str, force: bool) -> Result<(), String> {
    remove_for(vault, id, force, None).await
}

/// [`remove`], also allowed while `caller` (a Hollow app caller's run id,
/// PR A3) is the run holding the vault.
pub(crate) async fn remove_for(
    vault: &Path,
    id: &str,
    force: bool,
    caller: Option<&str>,
) -> Result<(), String> {
    remove_for_unless(vault, id, force, caller, || async { None }).await
}

/// [`remove_for`], asking `kept` once more right before `git worktree
/// remove`, after the slow steps (stopping its services can take up to
/// `services::STOP_WAIT` each, then `git status` and `rev-list`). A reason
/// from it is the error and nothing is removed: the worktree, its binding
/// and its build caches all stay (overnight R4: a Build or Install pressed
/// during those steps needs the worktree; Remove finished worktrees and the
/// gardener pass `worktree_hygiene::held_now`). No build-workspaces lock is
/// held while `kept` runs (`load` drops `CACHE` when it returns and `put`
/// comes after), so it may take `self_install`'s job lock.
pub(crate) async fn remove_for_unless<F, Fut>(
    vault: &Path,
    id: &str,
    force: bool,
    caller: Option<&str>,
    kept: F,
) -> Result<(), String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Option<String>>,
{
    let map = load()?;
    let binding = map
        .get(&session_store::vault_file_stem(vault))
        .and_then(|m| m.get(id))
        .cloned()
        .ok_or("This workspace has no worktree")?;
    if binding.removed_at.is_some() {
        return Err("This workspace's worktree was already removed".into());
    }
    // Its lanes are of its repository and follow its binding: they go first.
    let lanes = binding
        .lanes
        .values()
        .filter(|l| l.removed_at.is_none())
        .count();
    if lanes > 0 {
        return Err(format!(
            "This workspace has {lanes} other worktree(s); remove them first"
        ));
    }
    let mode = turn_mode(vault, id, &binding.worktree);
    refuse_removal_while_busy(vault, id, &mode, caller)?;
    // Held for the whole removal, as for a lane (delta review). A workspace
    // with no vault folder admits no turn at all, so it needs none.
    let reservation = match reserve_for_removal(vault, id, mode) {
        Ok(lease) => Some(lease),
        Err(_) if !vault.join("workspaces").join(id).is_dir() => None,
        Err(why) => return Err(why),
    };
    let removed = remove_tree(&binding.repo, &binding.worktree, &binding.base, force, kept).await;
    if let Some(lease) = reservation {
        let _ = lease.finish(if removed.is_ok() { "success" } else { "error" });
    }
    removed?;
    let (owner, repo, worktree) = (
        id.to_string(),
        binding.repo.clone(),
        binding.worktree.clone(),
    );
    // Through `modify` (review): the binding read above is minutes old, and
    // writing it back would erase a lane added meanwhile. Such a lane stays
    // bound and usable beside the removed primary (it is matched on its own,
    // and Remove worktree with its name takes it).
    modify(vault, id, |b, _| {
        b.removed_at = Some(Utc::now().to_rfc3339());
        let added = b.lanes.values().filter(|l| l.removed_at.is_none()).count();
        if added > 0 {
            eprintln!("[build-workspaces] {id}: {added} worktree(s) added during the primary's removal stay bound");
        }
        Ok(())
    })?;
    // Nothing can build from its caches any more (night ST7). Gigabytes:
    // deleted off the async workers, without holding up the reply.
    tokio::task::spawn_blocking(move || {
        crate::self_install::forget_build_output(&owner);
        crate::build_slots::forget_target(&repo, &worktree);
    });
    Ok(())
}

/// The git half of a worktree's removal: stop its services, refuse unsaved
/// work without `force`, ask `kept` last, then `git worktree remove` (or,
/// when the folder is already gone, prune git's record of it).
async fn remove_tree<F, Fut>(
    repo: &str,
    worktree_path: &str,
    base: &str,
    force: bool,
    kept: F,
) -> Result<(), String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Option<String>>,
{
    let worktree = PathBuf::from(worktree_path);
    if worktree.is_dir() {
        // Its services (Build PR B6) run in it: stop them first.
        crate::services::stop_worktree(&worktree).await;
        if !force {
            let status = git(&worktree, &["status", "--porcelain"], QUICK).await?;
            // Commits made here that neither the base nor any remote has.
            let unpushed = git(
                &worktree,
                &["rev-list", "--count", "HEAD", "--not", base, "--remotes"],
                QUICK,
            )
            .await
            .ok()
            .and_then(|n| n.parse::<usize>().ok())
            .unwrap_or(0);
            let refusal = RemoveRefusal {
                uncommitted: status.lines().filter(|l| !l.is_empty()).count(),
                unpushed,
            };
            if refusal.uncommitted > 0 || refusal.unpushed > 0 {
                return Err(format!(
                    "WORKTREE_UNSAVED: {}",
                    serde_json::to_string(&refusal).unwrap()
                ));
            }
        }
        // Needed since the list was shown (overnight R4). Its services stay
        // stopped: a Build or Install doesn't use them (it runs its own
        // build), and they are started again on request.
        if let Some(why) = kept().await {
            return Err(why);
        }
        // OC15: a setup hook still running there ends before the worktree
        // goes (after every refusal above, so a refused remove leaves it).
        crate::setup_hook::stop(&worktree).await;
        let mut args = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        args.push(worktree_path);
        // The Codex auto guard must not recreate files git is deleting.
        crate::agent::forget_codex_worktree_guard(Path::new(worktree_path));
        git(Path::new(repo), &args, Duration::from_secs(120)).await?;
    } else {
        // Gone already (removed by hand): just forget git's record of it.
        crate::setup_hook::stop(&worktree).await;
        crate::agent::forget_codex_worktree_guard(Path::new(worktree_path));
        crate::agent::forget_codex_worktree_guard(Path::new(repo));
        let _ = git(Path::new(repo), &["worktree", "prune"], QUICK).await;
    }
    Ok(())
}

/// Bind workspace `id` to `worktree` without git (tests of the run paths
/// that only need a live binding).
#[cfg(test)]
pub(crate) fn bind_for_test(vault: &Path, id: &str, worktree: &Path) {
    let worktree = worktree
        .canonicalize()
        .unwrap_or_else(|_| worktree.to_path_buf());
    put(
        vault,
        id,
        Some(Binding {
            repo: worktree.to_string_lossy().into(),
            worktree: worktree.to_string_lossy().into(),
            branch: "hollow/test".into(),
            base: "main".into(),
            created_at: Utc::now().to_rfc3339(),
            removed_at: None,
            runs_on: None,
            port: None,
            lanes: BTreeMap::new(),
        }),
    )
    .unwrap();
}

/// Bind workspace `id` to `worktree`, created from repository `repo`,
/// without git (retrieval scope tests: an arbitrarily named worktree of a
/// connected project).
#[cfg(test)]
pub(crate) fn bind_repo_for_test(vault: &Path, id: &str, repo: &Path, worktree: &Path) {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    put(
        vault,
        id,
        Some(Binding {
            repo: canon(repo).to_string_lossy().into(),
            worktree: canon(worktree).to_string_lossy().into(),
            branch: "hollow/test".into(),
            base: "main".into(),
            created_at: Utc::now().to_rfc3339(),
            removed_at: None,
            runs_on: None,
            port: None,
            lanes: BTreeMap::new(),
        }),
    )
    .unwrap();
}

/// Mark workspace `id`'s binding removed, as "Remove worktree" does, without git.
#[cfg(test)]
pub(crate) fn mark_removed_for_test(vault: &Path, id: &str) {
    let mut binding = load()
        .unwrap()
        .get(&session_store::vault_file_stem(vault))
        .and_then(|m| m.get(id).cloned())
        .expect("bound");
    binding.removed_at = Some(Utc::now().to_rfc3339());
    put(vault, id, Some(binding)).unwrap();
}

/// The file a trashed build workspace keeps its binding in, inside its
/// trashed folder (P1-41), so a restore brings back its worktree
/// conversations and its branch line.
const TRASHED_BINDING: &str = ".hollow-build-binding.json";

/// Keep workspace `id`'s binding in its trashed folder `dir`, before
/// [`forget`] drops it from the store (P1-41). Logged, never fatal.
pub(crate) fn stash(vault: &Path, id: &str, dir: &Path) {
    let Some(binding) = load().ok().and_then(|map| {
        map.get(&session_store::vault_file_stem(vault))?
            .get(id)
            .cloned()
    }) else {
        return;
    };
    let written = serde_json::to_vec_pretty(&binding)
        .map_err(|e| e.to_string())
        .and_then(|bytes| {
            std::fs::write(dir.join(TRASHED_BINDING), bytes).map_err(|e| e.to_string())
        });
    if let Err(e) = written {
        eprintln!("[build-workspaces] keep {id}'s binding in the trash: {e}");
    }
}

/// Put back the binding [`stash`] kept in restored folder `dir`, as removed
/// (trash required its worktree gone), and drop the file (P1-41).
pub(crate) fn unstash(vault: &Path, id: &str, dir: &Path) {
    let file = dir.join(TRASHED_BINDING);
    let Ok(text) = std::fs::read_to_string(&file) else {
        return;
    };
    let put_back = serde_json::from_str::<Binding>(&text)
        .map_err(|e| e.to_string())
        .and_then(|mut binding| {
            let now = Utc::now().to_rfc3339();
            binding.removed_at.get_or_insert_with(|| now.clone());
            for lane in binding.lanes.values_mut() {
                lane.removed_at.get_or_insert_with(|| now.clone());
            }
            put(vault, id, Some(binding))
        });
    match put_back {
        Ok(()) => {
            let _ = std::fs::remove_file(&file);
        }
        Err(e) => eprintln!("[build-workspaces] restore {id}'s binding: {e}"),
    }
}

/// Forget a workspace's binding entirely (its workspace was trashed).
pub fn forget(vault: &Path, id: &str) {
    // Its build caches go with it (night ST7), on a thread of their own: a
    // trash can run inside a live run, under the workspace's edit lease or on
    // an async worker, and the delete is gigabytes. A worktree already removed
    // queued its own delete in `remove_for` (batch review).
    let removed = load()
        .ok()
        .and_then(|map| {
            map.get(&session_store::vault_file_stem(vault))?
                .get(id)
                .map(|b| b.removed_at.is_some())
        })
        .unwrap_or(false);
    if let Some(build) = binding_for(vault, id).filter(|_| !removed) {
        let owner = id.to_string();
        std::thread::spawn(move || {
            crate::self_install::forget_build_output(&owner);
            crate::build_slots::forget_target(&build.repo, &build.worktree);
        });
    }
    if let Err(e) = put(vault, id, None) {
        eprintln!("[build-workspaces] forget {id}: {e}");
    }
}

/// A repository the new-workspace form offers. Beyond the folder name, the
/// form shows where it lives, its current branch, whether it is itself a
/// linked worktree and its origin, so two copies of one repository can be
/// told apart (audit H05). All of it is read from files, never a git spawn.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoCandidate {
    pub name: String,
    pub path: String,
    /// The containing folder relative to the Projects folder ("" at the top).
    pub parent: String,
    /// The checked-out branch, or `None` for a detached or unreadable HEAD.
    pub branch: Option<String>,
    /// `.git` is a file: this checkout is a linked worktree of another repo.
    pub worktree: bool,
    /// origin's URL reduced to `host/owner/name`, when configured.
    pub remote: Option<String>,
}

/// Folders scanned at most, so a huge Projects folder stays a cheap listing.
const MAX_CANDIDATE_DIRS: usize = 600;

/// Git repositories inside `projects` (the connected Projects folder): each
/// top-level folder, and one level inside top-level folders that are not
/// repositories themselves (`projects/personal/hollow`), sorted by name then
/// location. Cheap: a few small file reads per repository, no git spawn.
pub fn repo_candidates(projects: &Path) -> Result<Vec<RepoCandidate>, String> {
    let root = projects
        .canonicalize()
        .unwrap_or_else(|_| projects.to_path_buf());
    let mut out = Vec::new();
    let mut budget = MAX_CANDIDATE_DIRS;
    let visible = |e: &std::fs::DirEntry| {
        let n = e.file_name().to_string_lossy().to_string();
        !n.starts_with('.') && n != "node_modules" && n != "target"
    };
    let entries = std::fs::read_dir(&root)
        .map_err(|e| format!("Couldn't read {}: {e}", projects.display()))?;
    for e in entries.filter_map(Result::ok).filter(visible) {
        if budget == 0 {
            break;
        }
        budget -= 1;
        let Ok(path) = e.path().canonicalize() else {
            continue;
        };
        if !path.is_dir() {
            continue;
        }
        if path.join(".git").exists() {
            out.push(candidate(&root, &path));
            continue;
        }
        let Ok(children) = std::fs::read_dir(&path) else {
            continue;
        };
        for c in children.filter_map(Result::ok).filter(visible) {
            if budget == 0 {
                break;
            }
            budget -= 1;
            let Ok(child) = c.path().canonicalize() else {
                continue;
            };
            if child.is_dir() && child.join(".git").exists() {
                out.push(candidate(&root, &child));
            }
        }
    }
    out.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.parent.cmp(&b.parent))
    });
    Ok(out)
}

fn candidate(root: &Path, path: &Path) -> RepoCandidate {
    let dot_git = path.join(".git");
    let worktree = dot_git.is_file();
    // A linked worktree's `.git` file points at its own gitdir (HEAD lives
    // there); its config and remotes live in the main repository's commondir.
    let gitdir = if worktree {
        std::fs::read_to_string(&dot_git)
            .ok()
            .and_then(|t| {
                t.lines()
                    .find_map(|l| l.strip_prefix("gitdir:").map(|g| g.trim().to_string()))
            })
            .map(|g| {
                let g = PathBuf::from(g);
                if g.is_absolute() {
                    g
                } else {
                    path.join(g)
                }
            })
    } else {
        Some(dot_git.clone())
    };
    let branch = gitdir
        .as_ref()
        .and_then(|g| std::fs::read_to_string(g.join("HEAD")).ok())
        .and_then(|h| {
            h.trim()
                .strip_prefix("ref: refs/heads/")
                .map(str::to_string)
        });
    let common = gitdir
        .as_ref()
        .map(|g| match std::fs::read_to_string(g.join("commondir")) {
            Ok(c) => {
                let c = PathBuf::from(c.trim());
                if c.is_absolute() {
                    c
                } else {
                    g.join(c)
                }
            }
            Err(_) => g.clone(),
        });
    let remote = common
        .and_then(|c| std::fs::read_to_string(c.join("config")).ok())
        .and_then(|cfg| origin_url(&cfg));
    let parent = path
        .parent()
        .and_then(|p| p.strip_prefix(root).ok())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    RepoCandidate {
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        path: path.to_string_lossy().into(),
        parent,
        branch,
        worktree,
        remote,
    }
}

/// A small regular file's text (a git config or `.git` file): never a FIFO
/// or a huge file an agent pointed at.
fn read_small(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    (meta.is_file() && meta.len() <= 1024 * 1024)
        .then(|| std::fs::read_to_string(path).ok())
        .flatten()
}

/// The remote `gh` would use in the checkout holding `dir` (`dir` or a
/// parent), as `host/owner/name`, from its git config with no process (a
/// linked worktree's config is its main repository's; morning N8): the
/// remote marked `gh-resolved` (`gh repo set-default`), else `origin` when it
/// is the only GitHub remote. `None` when that can't be told (a fork with
/// `upstream` and `origin`, say).
pub(crate) fn gh_remote_of(dir: &Path) -> Option<String> {
    let dir = dir.canonicalize().ok()?;
    let root = dir.ancestors().find(|p| p.join(".git").exists())?;
    let dot_git = root.join(".git");
    let gitdir = if dot_git.is_file() {
        let text = read_small(&dot_git)?;
        let g = PathBuf::from(text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim());
        if g.is_absolute() {
            g
        } else {
            root.join(g)
        }
    } else {
        dot_git
    };
    let common = match read_small(&gitdir.join("commondir")) {
        Some(c) => {
            let c = PathBuf::from(c.trim());
            if c.is_absolute() {
                c
            } else {
                gitdir.join(c)
            }
        }
        None => gitdir,
    };
    gh_remote(&read_small(&common.join("config"))?)
}

/// [`gh_remote_of`] for a git config's text.
fn gh_remote(config: &str) -> Option<String> {
    let mut remotes: Vec<(String, Option<String>, Option<String>)> = Vec::new();
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            let name = line
                .strip_prefix("[remote \"")
                .and_then(|r| r.strip_suffix("\"]"))
                .map(str::to_string);
            remotes.push((name.unwrap_or_default(), None, None));
            continue;
        }
        let Some(last) = remotes.last_mut().filter(|r| !r.0.is_empty()) else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "url" => last.1 = remote_url(value.trim()),
            "gh-resolved" => last.2 = Some(value.trim().to_string()),
            _ => {}
        }
    }
    let remotes: Vec<(String, String, Option<String>)> = remotes
        .into_iter()
        .filter_map(|(n, u, r)| u.map(|u| (n, u, r)))
        .collect();
    // `gh repo set-default`: `base` means this remote; `owner/name` names the
    // base repository on this remote's host.
    if let Some((_, url, Some(resolved))) = remotes.iter().find(|r| r.2.is_some()) {
        if resolved == "base" {
            return Some(url.clone());
        }
        let host = url.split('/').next().unwrap_or("");
        return (resolved.matches('/').count() == 1).then(|| format!("{host}/{resolved}"));
    }
    let github = remotes
        .iter()
        .filter(|r| r.1.starts_with("github.com/"))
        .count();
    if github > 1 {
        return None;
    }
    remotes.into_iter().find(|r| r.0 == "origin").map(|r| r.1)
}

/// A remote URL as `host/owner/name`: credentials, a port, a `?query` and
/// `.git` dropped; GitHub's SSH-over-443 host is github.com.
fn remote_url(url: &str) -> Option<String> {
    let url = url.trim().trim_matches('"');
    let rest = if let Some(r) = url.split_once("://").map(|(_, r)| r) {
        r.rsplit_once('@').map(|(_, h)| h).unwrap_or(r).to_string()
    } else if let Some((user_host, path)) = url.split_once(':') {
        format!(
            "{}/{}",
            user_host
                .rsplit_once('@')
                .map(|(_, h)| h)
                .unwrap_or(user_host),
            path
        )
    } else {
        return None;
    };
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let (host, path) = rest.split_once('/')?;
    let host = host.split(':').next().unwrap_or(host);
    let host = if host == "ssh.github.com" {
        "github.com"
    } else {
        host
    };
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    (!path.is_empty()).then(|| format!("{host}/{path}"))
}

/// `[remote "origin"] url = …` from a git config, as `host/owner/name`.
/// Credentials in https URLs are dropped; nothing else is interpreted.
fn origin_url(config: &str) -> Option<String> {
    let mut in_origin = false;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            in_origin = line == "[remote \"origin\"]";
            continue;
        }
        if !in_origin {
            continue;
        }
        let Some(url) = line
            .strip_prefix("url")
            .map(str::trim_start)
            .and_then(|l| l.strip_prefix('='))
        else {
            continue;
        };
        let url = url.trim();
        let rest = if let Some(r) = url.split_once("://").map(|(_, r)| r) {
            r.rsplit_once('@').map(|(_, h)| h).unwrap_or(r).to_string()
        } else if let Some((user_host, path)) = url.split_once(':') {
            format!(
                "{}/{}",
                user_host
                    .rsplit_once('@')
                    .map(|(_, h)| h)
                    .unwrap_or(user_host),
                path
            )
        } else {
            return None;
        };
        let rest = rest
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .to_string();
        return (!rest.is_empty()).then_some(rest);
    }
    None
}

/// `<hollowHome>/Worktrees`, else `~/Hollow/Worktrees`.
pub fn worktrees_root(hollow_home: Option<&str>) -> Result<PathBuf, String> {
    let home = match hollow_home.map(str::trim).filter(|h| !h.is_empty()) {
        Some(home) => PathBuf::from(home),
        None => dirs::home_dir()
            .ok_or("No home folder")?
            .join(crate::instance::current().home_name()),
    };
    Ok(home.join("Worktrees"))
}

// ---- OC7: clone-seeded node_modules for a new worktree ----
//
// A new agent-lane worktree used to start with no `node_modules`: a cold
// `npm ci` in every lane, while a sibling checkout of the same repository
// often already held an install of the identical `package-lock.json`. When one
// does, an APFS clone (`cp -c -R`, copy-on-write) of it is ready in seconds.
// Only `create` seeds; Build app's release build keeps its clean `npm ci`.
//
// Never a symlink or hard link: a link would let one worktree's install change
// another's. `cp -c` is NOT fail-closed: on a volume that cannot clone, BSD cp
// quietly falls back to a whole copy, which for `node_modules` is minutes of
// disk. So the clone is only tried when the worktree and the source both sit
// on APFS and on one device (st_dev), and the child runs under a deadline.
// Any doubt (no sibling, a changed lock, a stale or torn clone, a timeout, an
// error) means no seed and the normal `npm ci`, never a failure.
//
// Trust: a sibling's installed tree is trusted only as far as its marker. The
// marker is the lock's sha256, written by Build app after a successful `npm
// ci`; the sibling's lock must still hash to it, and nothing shallow in
// `node_modules` (top-level files, `.bin`) may be newer than the marker. An
// agent in that lane could still have altered a package's own files, or left
// a symlink inside the tree; a clone copies those as they are.

/// Marker inside `node_modules` holding the sha256 of the `package-lock.json`
/// the install was made from.
pub(crate) const LOCK_MARK: &str = ".hollow-lock-sha256";

/// The most of a marker file that is read.
const MARK_MAX_BYTES: u64 = 128;

/// How long a clone may run, and how often it is checked.
pub(crate) struct SeedLimits {
    pub deadline: Duration,
    pub poll: Duration,
}

impl SeedLimits {
    pub(crate) const NORMAL: SeedLimits = SeedLimits {
        deadline: Duration::from_secs(90),
        poll: Duration::from_millis(50),
    };
}

/// sha256 (lowercase hex) of `dir/package-lock.json`.
pub(crate) fn lock_hash(dir: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(dir.join("package-lock.json")).ok()?;
    Some(format!("{:x}", Sha256::digest(&bytes)))
}

/// The marker in `node_modules` folder `nm`: a plain file (never a symlink or
/// a FIFO), at most [`MARK_MAX_BYTES`] bytes read.
fn read_mark(nm: &Path) -> Option<String> {
    use std::io::Read;
    let path = nm.join(LOCK_MARK);
    if !std::fs::symlink_metadata(&path).ok()?.file_type().is_file() {
        return None;
    }
    let mut text = String::new();
    std::fs::File::open(&path)
        .ok()?
        .take(MARK_MAX_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    let hash = text.trim();
    (!hash.is_empty()).then(|| hash.to_string())
}

/// Record that `worktree`'s `node_modules` was installed from its current
/// lock file. Called after a successful install by Build app. Refuses a
/// marker path that is not a plain file.
pub(crate) fn write_lock_mark(worktree: &Path) -> Result<(), String> {
    let hash = lock_hash(worktree).ok_or("no package-lock.json to record")?;
    let nm = worktree.join("node_modules");
    if !nm.is_dir() {
        return Err("no node_modules to mark".into());
    }
    let mark = nm.join(LOCK_MARK);
    if let Ok(meta) = std::fs::symlink_metadata(&mark) {
        if !meta.file_type().is_file() {
            return Err(format!("{LOCK_MARK} is not a plain file"));
        }
    }
    std::fs::write(mark, format!("{hash}\n")).map_err(|e| format!("write {LOCK_MARK}: {e}"))
}

/// Whether nothing shallow in `nm` changed after the marker: no top-level
/// file and no `.bin` entry is newer than it. (Package folders are not
/// compared: a clone resets a folder's own mtime, and files keep theirs.) Any
/// read error counts as changed.
fn untouched_since_mark(nm: &Path) -> bool {
    let Some(mark) = std::fs::symlink_metadata(nm.join(LOCK_MARK))
        .and_then(|m| m.modified())
        .ok()
    else {
        return false;
    };
    let newer = |path: &Path| {
        std::fs::symlink_metadata(path)
            .and_then(|m| m.modified())
            .map(|t| t > mark)
            .unwrap_or(true)
    };
    let Ok(top) = std::fs::read_dir(nm) else {
        return false;
    };
    for entry in top {
        let Ok(entry) = entry else { return false };
        let Ok(kind) = entry.file_type() else {
            return false;
        };
        if kind.is_dir() || entry.file_name() == LOCK_MARK {
            continue;
        }
        if newer(&entry.path()) {
            return false;
        }
    }
    let bin = nm.join(".bin");
    if std::fs::symlink_metadata(&bin).is_ok_and(|m| m.file_type().is_dir()) {
        let Ok(entries) = std::fs::read_dir(&bin) else {
            return false;
        };
        for entry in entries {
            let Ok(entry) = entry else { return false };
            if newer(&entry.path()) {
                return false;
            }
        }
    }
    true
}

/// The checkout among `candidates` to clone `node_modules` from. It must sit
/// under one of `roots` (the worktrees root or the repo root), have a real
/// (not symlinked) `node_modules` whose marker AND whose own current lock file
/// both equal this worktree's lock hash (a sibling that ran `npm install`
/// since is dropped), and have nothing shallow in `node_modules` newer than
/// the marker. The newest marker wins, ties by path, so the choice is stable.
pub(crate) fn pick_seed(
    worktree: &Path,
    candidates: &[PathBuf],
    roots: &[PathBuf],
) -> Option<PathBuf> {
    let want = lock_hash(worktree)?;
    let me = worktree.canonicalize().ok();
    let roots: Vec<PathBuf> = roots.iter().filter_map(|r| r.canonicalize().ok()).collect();
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for dir in candidates {
        let Ok(real) = dir.canonicalize() else {
            continue;
        };
        if Some(&real) == me.as_ref() || !roots.iter().any(|r| real.starts_with(r)) {
            continue;
        }
        let nm = dir.join("node_modules");
        let is_real_dir = std::fs::symlink_metadata(&nm).is_ok_and(|m| m.file_type().is_dir());
        if !is_real_dir
            || read_mark(&nm).as_deref() != Some(want.as_str())
            || lock_hash(dir).as_deref() != Some(want.as_str())
            || !untouched_since_mark(&nm)
        {
            continue;
        }
        let stamp = std::fs::symlink_metadata(nm.join(LOCK_MARK))
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        let better = match &best {
            None => true,
            Some((t, p)) => stamp > *t || (stamp == *t && dir < p),
        };
        if better {
            best = Some((stamp, dir.clone()));
        }
    }
    best.map(|(_, dir)| dir)
}

/// The arguments of the clone: an APFS copy-on-write copy of `src` to `dst`.
pub(crate) fn clone_args(src: &Path, dst: &Path) -> Vec<String> {
    vec![
        "-c".into(),
        "-R".into(),
        src.to_string_lossy().into(),
        dst.to_string_lossy().into(),
    ]
}

/// Whether every device id is the same (a clone cannot cross volumes).
pub(crate) fn same_volume(devs: &[u64]) -> bool {
    devs.windows(2).all(|w| w[0] == w[1])
}

fn dev_of(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| m.dev())
}

/// Whether `path` is on an APFS volume (the only place `cp -c` clones).
#[cfg(target_os = "macos")]
pub(crate) fn is_apfs(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mut st = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `c` is a NUL-terminated path and `st` is a valid out pointer;
    // it is read only after statfs reports success.
    if unsafe { libc::statfs(c.as_ptr(), st.as_mut_ptr()) } != 0 {
        return false;
    }
    // SAFETY: statfs succeeded, so it filled the struct.
    let st = unsafe { st.assume_init() };
    let name: Vec<u8> = st
        .f_fstypename
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8)
        .collect();
    name == b"apfs"
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn is_apfs(_path: &Path) -> bool {
    false
}

/// Checkout paths from `git worktree list --porcelain` (the main checkout
/// first).
pub(crate) fn parse_worktree_list(porcelain: &str) -> Vec<PathBuf> {
    porcelain
        .lines()
        .filter_map(|l| l.strip_prefix("worktree "))
        .map(PathBuf::from)
        .collect()
}

/// What [`seed_node_modules_with`] did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Seed {
    /// `node_modules` was cloned from this checkout.
    Cloned(PathBuf),
    /// Nothing was seeded, and why: the caller falls back to `npm ci`.
    Skipped(&'static str),
}

fn remove_any(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_dir() => {
            let _ = std::fs::remove_dir_all(path);
        }
        Ok(_) => {
            let _ = std::fs::remove_file(path);
        }
        Err(_) => {}
    }
}

/// Clone a matching sibling's `node_modules` into `worktree` with `cp` (a
/// parameter so a test can swap it). The clone lands in `staging` (a fixed
/// name, cleared first, that the caller keeps out of the worktree's own
/// status) under a deadline, and is renamed into place only when it carries
/// this lock's marker and the sibling's lock and marker still match: a torn,
/// slow or stale clone is discarded, never left for `npm ci` to skip.
pub(crate) fn seed_node_modules_with(
    worktree: &Path,
    candidates: &[PathBuf],
    roots: &[PathBuf],
    staging: &Path,
    cp: &Path,
    limits: &SeedLimits,
) -> Seed {
    let dest = worktree.join("node_modules");
    if std::fs::symlink_metadata(&dest).is_ok() {
        return Seed::Skipped("node_modules is already there");
    }
    let Some(want) = lock_hash(worktree) else {
        return Seed::Skipped("no package-lock.json");
    };
    let Some(from) = pick_seed(worktree, candidates, roots) else {
        return Seed::Skipped("no sibling has an install of this lock file");
    };
    let src = from.join("node_modules");
    let Some(staging_parent) = staging.parent() else {
        return Seed::Skipped("no staging folder");
    };
    let devs = [dev_of(&src), dev_of(worktree), dev_of(staging_parent)];
    if devs.iter().any(Option::is_none)
        || !same_volume(&devs.iter().flatten().copied().collect::<Vec<_>>())
    {
        return Seed::Skipped("the sibling is on another volume");
    }
    remove_any(staging);
    let Ok(mut child) = std::process::Command::new(cp)
        .args(clone_args(&src, staging))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return Seed::Skipped("the clone did not work");
    };
    let started = std::time::Instant::now();
    let finished = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status.success()),
            Ok(None) if started.elapsed() < limits.deadline => std::thread::sleep(limits.poll),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let verdict = match finished {
        None => Err("the clone took too long"),
        Some(false) => Err("the clone did not work"),
        Some(true) => {
            let carried = read_mark(staging).as_deref() == Some(want.as_str());
            let sibling_same = lock_hash(&from).as_deref() == Some(want.as_str())
                && read_mark(&src).as_deref() == Some(want.as_str());
            if carried && sibling_same {
                Ok(())
            } else {
                Err("the clone did not match the lock file")
            }
        }
    };
    match verdict {
        Ok(()) if std::fs::rename(staging, &dest).is_ok() => Seed::Cloned(from),
        Ok(()) => {
            remove_any(staging);
            Seed::Skipped("the clone did not work")
        }
        Err(why) => {
            remove_any(staging);
            Seed::Skipped(why)
        }
    }
}

/// Seed `worktree`'s `node_modules` from a sibling worktree of the same
/// repository (or the main checkout) under `roots` when one holds an install
/// of the same lock file. Best effort: any error is a skip. The staging folder
/// lives in the worktree's own git directory, so it is never in `git status`
/// and goes away with the worktree.
pub(crate) async fn seed_new_worktree(worktree: &Path, roots: &[PathBuf]) -> Seed {
    if !is_apfs(worktree) {
        return Seed::Skipped("the worktree is not on an APFS volume");
    }
    let Ok(list) = git(worktree, &["worktree", "list", "--porcelain"], QUICK).await else {
        return Seed::Skipped("could not list the worktrees");
    };
    let Ok(git_dir) = git(worktree, &["rev-parse", "--absolute-git-dir"], QUICK).await else {
        return Seed::Skipped("could not find the git directory");
    };
    let candidates: Vec<PathBuf> = parse_worktree_list(&list)
        .into_iter()
        .filter(|c| is_apfs(&c.join("node_modules")))
        .collect();
    let staging = Path::new(&git_dir).join("hollow-node-modules-seed");
    let (worktree, roots) = (worktree.to_path_buf(), roots.to_vec());
    tokio::task::spawn_blocking(move || {
        seed_node_modules_with(
            &worktree,
            &candidates,
            &roots,
            &staging,
            Path::new("/bin/cp"),
            &SeedLimits::NORMAL,
        )
    })
    .await
    .unwrap_or(Seed::Skipped("the clone task failed"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn slugs_are_branch_safe() {
        assert_eq!(slug("Fix the About page!", 40), "fix-the-about-page");
        assert_eq!(slug("  ---  ", 40), "work");
        assert_eq!(slug("Ünïcode + ops", 40), "n-code-ops");
        assert!(slug(&"x".repeat(100), 40).len() <= 40);
    }

    #[test]
    fn origin_head_maps_to_a_base() {
        assert_eq!(
            remote_head_to_base("refs/remotes/origin/main\n").as_deref(),
            Some("origin/main")
        );
        assert_eq!(remote_head_to_base("refs/heads/main"), None);
        assert_eq!(remote_head_to_base("refs/remotes/origin"), None);
    }

    pub(crate) fn sh(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new(&*GIT)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "init.defaultBranch=main",
            ])
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    /// A repo with one commit on `main` (no remote), a vault, and a
    /// worktrees root, all under one temp dir.
    pub(crate) fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let tmp = tempfile::Builder::new()
            .prefix("build-ws-")
            .tempdir()
            .unwrap();
        let repo = tmp.path().join("my-app");
        std::fs::create_dir_all(&repo).unwrap();
        sh(&repo, &["init", "-q"]);
        std::fs::write(repo.join("README.md"), "hi\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-q", "-m", "first"]);
        let vault = tmp.path().join("vault");
        std::fs::create_dir_all(&vault).unwrap();
        let roots = tmp.path().join("Worktrees");
        (tmp, repo, vault, roots)
    }

    #[tokio::test]
    async fn create_binds_a_worktree_and_the_workspace_runs_there() {
        let (_tmp, repo, vault, roots) = fixture();
        let made = create(
            &vault,
            &roots,
            CreateInput {
                title: "Fix the About page".into(),
                goal: "Ship it".into(),
                repo: repo.to_string_lossy().into(),
                branch: None,
                base: None,
            },
        )
        .await
        .unwrap();
        let w = made.workspace;
        let build = binding_for(&vault, &w.id).expect("bound");
        assert_eq!(build.branch, "hollow/fix-the-about-page");
        assert_eq!(build.base, "main");
        assert!(build.exists);
        assert_eq!(
            Path::new(&w.cwd).canonicalize().unwrap(),
            Path::new(&build.worktree).canonicalize().unwrap()
        );
        assert!(Path::new(&build.worktree).join("README.md").exists());
        assert!(Path::new(&build.worktree).starts_with(roots.canonicalize().unwrap()));
        // The worktree cwd maps back to the workspace: its context applies.
        let found = workspaces::workspace_for_cwd(&vault, Path::new(&w.cwd))
            .unwrap()
            .unwrap();
        assert_eq!(found.id, w.id);
        let context = workspaces::context(&vault, Path::new(&w.cwd))
            .unwrap()
            .unwrap();
        assert!(context.contains("git worktree"), "{context}");
        assert!(context.contains(&build.branch));
        // A second one with the same title gets a numbered branch and folder.
        let again = create(
            &vault,
            &roots,
            CreateInput {
                title: "Fix the About page".into(),
                goal: "Again".into(),
                repo: repo.to_string_lossy().into(),
                branch: None,
                base: None,
            },
        )
        .await
        .unwrap();
        let b2 = binding_for(&vault, &again.workspace.id).unwrap();
        assert_eq!(b2.branch, "hollow/fix-the-about-page-2");
        assert_ne!(b2.worktree, build.worktree);
    }

    #[tokio::test]
    async fn create_refuses_bad_input_and_leaves_nothing_behind() {
        let (_tmp, repo, vault, roots) = fixture();
        let input = |branch: Option<&str>, base: Option<&str>, repo: &Path| CreateInput {
            title: "T".into(),
            goal: "G".into(),
            repo: repo.to_string_lossy().into(),
            branch: branch.map(str::to_string),
            base: base.map(str::to_string),
        };
        assert!(create(&vault, &roots, input(Some("main"), None, &repo))
            .await
            .unwrap_err()
            .contains("already exists"));
        assert!(
            create(&vault, &roots, input(Some("bad..name"), None, &repo))
                .await
                .unwrap_err()
                .contains("not a valid")
        );
        assert!(create(&vault, &roots, input(None, Some("nope"), &repo))
            .await
            .unwrap_err()
            .contains("not a branch"));
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        assert!(create(&vault, &roots, input(None, None, &repo.join("sub")))
            .await
            .unwrap_err()
            .contains("top folder"));
        assert!(create(&vault, &roots, input(None, None, &vault))
            .await
            .is_err());
        // A title the workspace refuses undoes the worktree and the branch.
        let long = CreateInput {
            title: "x".repeat(101),
            ..input(Some("feature/x"), None, &repo)
        };
        assert!(create(&vault, &roots, long).await.is_err());
        assert!(!branch_exists(&repo, "feature/x").await);
        assert_eq!(std::fs::read_dir(roots.join("my-app")).unwrap().count(), 0);
        assert!(workspaces::list(&vault).unwrap().is_empty());
    }

    /// OC5: a base N or more commits behind main is refused with the fix
    /// named; the override and a higher limit let it through.
    #[tokio::test]
    async fn a_stale_base_is_refused_unless_overridden() {
        let (_tmp, repo, vault, roots) = fixture();
        sh(&repo, &["branch", "old"]);
        for n in 0..STALE_BASE_LIMIT {
            std::fs::write(repo.join("README.md"), format!("{n}\n")).unwrap();
            sh(&repo, &["commit", "-q", "-am", &format!("c{n}")]);
        }
        let input = |title: &str| CreateInput {
            title: title.into(),
            goal: "G".into(),
            repo: repo.to_string_lossy().into(),
            branch: None,
            base: Some("old".into()),
        };
        let refused = create(&vault, &roots, input("Stale")).await.unwrap_err();
        assert!(refused.starts_with(STALE_BASE_ERROR), "{refused}");
        assert!(refused.contains("20 commits behind main"), "{refused}");
        assert!(refused.contains("merge main"), "{refused}");
        assert!(workspaces::list(&vault).unwrap().is_empty());
        assert!(!branch_exists(&repo, "hollow/stale").await);
        // The override branches from it anyway.
        let forced = BasePolicy {
            allow_stale: true,
            ..BasePolicy::default()
        };
        create_with(&vault, &roots, input("Forced"), forced)
            .await
            .unwrap();
        // So does a limit above the gap.
        let roomy = BasePolicy {
            limit: STALE_BASE_LIMIT + 1,
            ..BasePolicy::default()
        };
        create_with(&vault, &roots, input("Roomy"), roomy)
            .await
            .unwrap();
        // Branching from main itself is never stale.
        let fresh = CreateInput {
            base: Some("main".into()),
            ..input("Fresh")
        };
        create(&vault, &roots, fresh).await.unwrap();
    }

    #[tokio::test]
    async fn remove_refuses_unsaved_work_then_forces_and_falls_back() {
        let (_tmp, repo, vault, roots) = fixture();
        let w = create(
            &vault,
            &roots,
            CreateInput {
                title: "Work".into(),
                goal: "G".into(),
                repo: repo.to_string_lossy().into(),
                branch: None,
                base: None,
            },
        )
        .await
        .unwrap()
        .workspace;
        let tree = PathBuf::from(binding_for(&vault, &w.id).unwrap().worktree);
        std::fs::write(tree.join("new.txt"), "x").unwrap();
        let refused = remove(&vault, &w.id, false).await.unwrap_err();
        assert!(refused.starts_with("WORKTREE_UNSAVED: "), "{refused}");
        let counts: serde_json::Value =
            serde_json::from_str(refused.trim_start_matches("WORKTREE_UNSAVED: ")).unwrap();
        assert_eq!(counts["uncommitted"], 1);
        // A local commit with no remote counts as unpushed.
        sh(&tree, &["add", "."]);
        sh(&tree, &["commit", "-q", "-m", "wip"]);
        let refused = remove(&vault, &w.id, false).await.unwrap_err();
        assert!(refused.contains("\"unpushed\":1"), "{refused}");
        remove(&vault, &w.id, true).await.unwrap();
        assert!(!tree.exists());
        assert!(
            branch_exists(&repo, "hollow/work").await,
            "the branch is kept"
        );
        let info = binding_for(&vault, &w.id).unwrap();
        assert!(!info.exists);
        // The workspace runs in its vault folder again.
        let back = workspaces::list(&vault)
            .unwrap()
            .into_iter()
            .find(|x| x.id == w.id)
            .unwrap();
        assert!(
            back.cwd.ends_with(&format!("workspaces/{}", w.id)),
            "{}",
            back.cwd
        );
        assert!(remove(&vault, &w.id, true)
            .await
            .unwrap_err()
            .contains("already removed"));
    }

    /// F6 part B (docs/BUILD_WORKSPACES_MULTI.md): Add worktree makes a
    /// branch and worktree of the same repository, bound as a lane the
    /// workspace's runs find, with its own port; the primary waits for its
    /// lanes before it goes; Remove keeps the lane's branch and asks about
    /// unsaved work like the primary's.
    #[tokio::test]
    async fn a_lane_is_added_found_and_removed_beside_the_primary() {
        // The turn registry is process-wide; `held` below is a real lease.
        let (_tmp, repo, vault, roots) = fixture();
        let w = create(
            &vault,
            &roots,
            CreateInput {
                title: "Lanes".into(),
                goal: "G".into(),
                repo: repo.to_string_lossy().into(),
                branch: None,
                base: None,
            },
        )
        .await
        .unwrap()
        .workspace;
        let primary = binding_for(&vault, &w.id).unwrap();
        assert!(primary.port.is_some(), "a new binding gets its own port");
        let lane = |name: &str| LaneInput {
            name: name.into(),
            ..Default::default()
        };
        for bad in ["main", "  ", "!!"] {
            assert!(
                bind_lane(&vault, &roots, &w.id, lane(bad)).await.is_err(),
                "{bad}"
            );
        }
        let b = bind_lane(&vault, &roots, &w.id, lane("Lane B"))
            .await
            .unwrap();
        assert_eq!(b.name, "lane-b");
        assert_eq!(b.branch, "hollow/lanes-lane-b");
        assert_eq!(b.base, primary.base);
        assert!(b.exists && Path::new(&b.worktree).join("README.md").exists());
        assert_ne!(
            b.port,
            crate::build_slots::port_for(&primary),
            "ports differ"
        );
        assert!(bind_lane(&vault, &roots, &w.id, lane("lane-b"))
            .await
            .unwrap_err()
            .contains("already has"));
        // Its runs find the workspace and the lane; the primary is `main`.
        let tree = PathBuf::from(&b.worktree);
        assert_eq!(
            worktree_lane(&vault, &tree),
            Some((w.id.clone(), "lane-b".to_string()))
        );
        assert_eq!(
            worktree_lane(&vault, Path::new(&primary.worktree)),
            Some((w.id.clone(), MAIN_LANE.to_string()))
        );
        assert_eq!(
            workspaces::workspace_for_cwd(&vault, &tree)
                .unwrap()
                .unwrap()
                .id,
            w.id
        );
        assert_eq!(
            binding_for(&vault, &w.id).unwrap(),
            primary,
            "the primary is unchanged"
        );
        let names: Vec<String> = lanes_for(&vault, &w.id)
            .into_iter()
            .map(|l| l.name)
            .collect();
        assert_eq!(names, ["main", "lane-b"]);
        // The primary waits for its lanes, before any git.
        let refused = remove(&vault, &w.id, true).await.unwrap_err();
        assert!(refused.contains("other worktree"), "{refused}");
        assert!(Path::new(&primary.worktree).is_dir());
        // A lane turn keeps its own removal out, not the primary's lock.
        let held = crate::automated_history::try_begin_scoped_mode(
            &vault,
            &w.path,
            crate::automated_history::ScopeMode::Worktree(lock_key(&tree)),
            "chat",
            None,
        )
        .unwrap();
        let busy = remove_lane(&vault, &w.id, "Lane B", true, None)
            .await
            .unwrap_err();
        assert!(busy.contains("working in this worktree"), "{busy}");
        // A lane turn's agent can't commit the primary's work through
        // Hollow's build actions (review of #1599), even with the primary idle.
        std::fs::write(Path::new(&primary.worktree).join("wip.txt"), "primary's").unwrap();
        let refused = crate::build_changes::commit_for(&vault, &w.id, "x", Some(held.id()))
            .await
            .unwrap_err();
        assert!(refused.contains("worktree lane-b"), "{refused}");
        // Each turn is told its own worktree; the folder's chats none.
        let told = workspaces::context(&vault, &tree).unwrap().unwrap();
        assert!(
            told.contains(&format!(
                "git worktree {} on branch hollow/lanes-lane-b",
                b.worktree
            )),
            "{told}"
        );
        assert!(
            !told.contains(&format!("git worktree {} on", primary.worktree)),
            "{told}"
        );
        assert!(told.contains("It is worktree lane-b"), "{told}");
        let folder = vault.join(&w.path);
        let told = workspaces::context(&vault, &folder).unwrap().unwrap();
        assert!(
            told.contains(workspaces::LANES_FOLDER_NOTE.trim()),
            "{told}"
        );
        assert!(!told.contains("git worktree /"), "{told}");
        held.finish("success").unwrap();
        std::fs::write(tree.join("new.txt"), "x").unwrap();
        let refused = remove_lane(&vault, &w.id, "lane-b", false, None)
            .await
            .unwrap_err();
        assert!(refused.starts_with("WORKTREE_UNSAVED: "), "{refused}");
        remove_lane(&vault, &w.id, "lane-b", true, None)
            .await
            .unwrap();
        assert!(!tree.exists());
        assert!(
            branch_exists(&repo, "hollow/lanes-lane-b").await,
            "the branch is kept"
        );
        assert_eq!(worktree_lane(&vault, &tree), None);
        assert_eq!(live_lane_count(&vault, &w.id), 0);
        assert!(remove_lane(&vault, &w.id, "lane-b", true, None)
            .await
            .unwrap_err()
            .contains("already removed"));
        // Its name is free again, and the primary can go once lanes are gone.
        bind_lane(&vault, &roots, &w.id, lane("lane-b"))
            .await
            .unwrap();
        remove_lane(&vault, &w.id, "lane-b", true, None)
            .await
            .unwrap();
        remove(&vault, &w.id, true).await.unwrap();
        assert!(bind_lane(&vault, &roots, &w.id, lane("late"))
            .await
            .unwrap_err()
            .contains("was removed"));
    }

    /// F6 part B: no schema bump. A v1 binding parses with no lanes and no
    /// port and is written back exactly as it was; a binding with lanes
    /// still reads, primary intact, as the struct an older Hollow has.
    #[test]
    fn a_v1_binding_reads_unchanged_and_an_older_hollow_keeps_the_primary() {
        let v1 = r#"{"repo":"/r","worktree":"/w/a","branch":"hollow/a","base":"origin/main","createdAt":"2026-10-01T00:00:00Z"}"#;
        let binding: Binding = serde_json::from_str(v1).unwrap();
        assert!(binding.lanes.is_empty() && binding.port.is_none());
        let back: serde_json::Value = serde_json::to_value(&binding).unwrap();
        assert_eq!(back, serde_json::from_str::<serde_json::Value>(v1).unwrap());
        assert_eq!(
            crate::build_slots::port_for(&info(&binding)),
            crate::build_slots::hashed_port("/w/a"),
            "a v1 port doesn't move"
        );
        let mut with_lanes = binding.clone();
        with_lanes.port = Some(1600);
        with_lanes.lanes.insert(
            "lane-b".into(),
            Lane {
                worktree: "/w/b".into(),
                branch: "hollow/a-lane-b".into(),
                base: "origin/main".into(),
                created_at: "2026-11-18T00:00:00Z".into(),
                removed_at: None,
                port: Some(1601),
            },
        );
        let text = serde_json::to_string(&Store {
            schema_version: 1,
            vaults: [(
                "v".to_string(),
                [("w1".to_string(), with_lanes.clone())].into(),
            )]
            .into(),
        })
        .unwrap();
        // The binding as Hollow knew it before F6 part B.
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct OldBinding {
            repo: String,
            worktree: String,
            branch: String,
            base: String,
            removed_at: Option<String>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct OldStore {
            schema_version: u32,
            vaults: BTreeMap<String, BTreeMap<String, OldBinding>>,
        }
        let old: OldStore = serde_json::from_str(&text).unwrap();
        assert_eq!(old.schema_version, 1);
        let primary = &old.vaults["v"]["w1"];
        assert_eq!(
            (
                primary.repo.as_str(),
                primary.worktree.as_str(),
                primary.branch.as_str()
            ),
            ("/r", "/w/a", "hollow/a")
        );
        assert_eq!(primary.base, "origin/main");
        assert!(primary.removed_at.is_none());
        let store: Store = serde_json::from_str(&text).unwrap();
        assert_eq!(store.vaults["v"]["w1"], with_lanes, "round trip");
    }

    /// F6 part B: a new worktree's port is its hash when free, else the
    /// next one no live worktree of the vault uses, wrapping within
    /// 1500-1899; none when every one is taken.
    #[test]
    fn a_new_worktrees_port_skips_every_bound_one() {
        let binding = |tree: &str, port: Option<u16>| Binding {
            repo: "/r".into(),
            worktree: tree.into(),
            branch: "b".into(),
            base: "main".into(),
            created_at: String::new(),
            removed_at: None,
            runs_on: None,
            port,
            lanes: BTreeMap::new(),
        };
        let hash = crate::build_slots::hashed_port("/w/new");
        let mut all = BTreeMap::new();
        assert_eq!(assign_port(&all, "/w/new"), Some(hash), "free: its hash");
        all.insert("a".to_string(), binding("/w/a", Some(hash)));
        let next = if hash == 1899 { 1500 } else { hash + 1 };
        assert_eq!(assign_port(&all, "/w/new"), Some(next));
        // A removed binding's port is free again.
        all.get_mut("a").unwrap().removed_at = Some("x".into());
        assert_eq!(assign_port(&all, "/w/new"), Some(hash));
        // Every port taken, by bindings and lanes: none.
        let mut full = BTreeMap::new();
        for port in BOUND_PORTS {
            full.insert(
                format!("w{port}"),
                binding(&format!("/w/{port}"), Some(port)),
            );
        }
        assert_eq!(assign_port(&full, "/w/new"), None);
    }

    /// #1067 review: the project's main agent stays sendable when its
    /// worktree goes, without moving anything; one the person moved to
    /// another project is still refused, and so is a removed path that a
    /// newer workspace reuses (ambiguous).
    #[tokio::test]
    async fn a_project_main_agent_still_sends_after_its_worktree_is_removed() {
        let (_tmp, repo, vault, roots) = fixture();
        let input = |title: &str| CreateInput {
            title: title.into(),
            goal: "G".into(),
            repo: repo.to_string_lossy().into(),
            branch: None,
            base: None,
        };
        let w = create(&vault, &roots, input("Work"))
            .await
            .unwrap()
            .workspace;
        let moved = create(&vault, &roots, input("Moved"))
            .await
            .unwrap()
            .workspace;
        let other = workspaces::create(&vault, "Other", "G").unwrap();
        let main = crate::project_agents::key(&w.id).unwrap();
        let elsewhere = crate::project_agents::key(&moved.id).unwrap();
        assert_eq!(w.cwd, binding_for(&vault, &w.id).unwrap().worktree);
        session_store::start_turn(&vault, &main, "claude", &w.cwd, "r-1", "Plan it").unwrap();
        session_store::start_turn(&vault, &elsewhere, "codex", &other.cwd, "r-2", "Moved").unwrap();
        remove(&vault, &w.id, true).await.unwrap();
        remove(&vault, &moved.id, true).await.unwrap();
        let home = |id: &str| {
            workspaces::list(&vault)
                .unwrap()
                .into_iter()
                .find(|x| x.id == id)
                .unwrap()
                .cwd
        };
        let target = workspaces::session_target(&vault, &main).unwrap().unwrap();
        assert_eq!(target.workspace_id.as_deref(), Some(w.id.as_str()));
        assert!(crate::project_agents::validate(&vault, Path::new(&home(&w.id)), &main).is_ok());
        // History is untouched: same folder record, same provider.
        assert_eq!(target.session.cwd, w.cwd);
        assert_eq!(target.session.agent, "claude");
        assert!(
            crate::project_agents::validate(&vault, Path::new(&home(&moved.id)), &elsewhere)
                .is_err()
        );
        let stayed = session_store::session_by_key(&vault, &elsewhere)
            .unwrap()
            .unwrap();
        assert_eq!(stayed.cwd, other.cwd);
        // A new workspace reusing the removed path makes it ambiguous: refused.
        let reuse = create(
            &vault,
            &roots,
            CreateInput {
                branch: Some("new/work".into()),
                ..input("Reuse")
            },
        )
        .await
        .unwrap()
        .workspace;
        assert_eq!(reuse.cwd, w.cwd);
        assert!(crate::project_agents::validate(&vault, Path::new(&home(&w.id)), &main).is_err());
        // Its binding still claims the path when its brief can't be read.
        std::fs::write(
            vault
                .join("workspaces")
                .join(&reuse.id)
                .join("WORKSPACE.md"),
            "x",
        )
        .unwrap();
        assert!(workspaces::list(&vault)
            .unwrap()
            .iter()
            .all(|x| x.id != reuse.id));
        assert!(crate::project_agents::validate(&vault, Path::new(&home(&w.id)), &main).is_err());
    }

    /// Overnight R4: a reason from `kept`, asked once after the services
    /// stopped and the unsaved-work checks, refuses the removal with it; the
    /// worktree and its binding stay; asked nothing, it goes as before.
    #[tokio::test]
    async fn remove_for_unless_keeps_the_worktree_and_binding_when_kept_says_why() {
        let (_tmp, repo, vault, roots) = fixture();
        let w = create(
            &vault,
            &roots,
            CreateInput {
                title: "Busy".into(),
                goal: "G".into(),
                repo: repo.to_string_lossy().into(),
                branch: None,
                base: None,
            },
        )
        .await
        .unwrap()
        .workspace;
        let tree = PathBuf::from(binding_for(&vault, &w.id).unwrap().worktree);
        let why = "Kept: a build running now, since the list was shown";
        let calls = std::cell::Cell::new(0);
        let calls_ref = &calls;
        let kept = move || async move {
            calls_ref.set(calls_ref.get() + 1);
            Some(why.to_string())
        };
        let refused = remove_for_unless(&vault, &w.id, false, None, kept)
            .await
            .unwrap_err();
        assert_eq!(refused, why);
        assert_eq!(calls.get(), 1);
        assert!(tree.is_dir(), "the worktree stays");
        assert!(
            binding_for(&vault, &w.id).unwrap().exists,
            "its binding stays"
        );
        // Then nothing keeps it: it goes as before.
        remove_for_unless(&vault, &w.id, false, None, || async { None })
            .await
            .unwrap();
        assert!(!tree.exists());
        assert!(!binding_for(&vault, &w.id).unwrap().exists);
    }

    #[test]
    fn repo_candidates_list_only_git_folders() {
        let tmp = tempfile::Builder::new().prefix("cands-").tempdir().unwrap();
        for name in ["beta", "Alpha", ".hidden", "plain"] {
            std::fs::create_dir_all(tmp.path().join(name)).unwrap();
        }
        for name in ["beta", "Alpha", ".hidden"] {
            std::fs::create_dir_all(tmp.path().join(name).join(".git")).unwrap();
        }
        // A worktree's `.git` is a file.
        std::fs::create_dir_all(tmp.path().join("tree")).unwrap();
        std::fs::write(tmp.path().join("tree/.git"), "gitdir: /x").unwrap();
        let names: Vec<String> = repo_candidates(tmp.path())
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["Alpha", "beta", "tree"]);
    }

    #[test]
    fn repo_candidates_find_nested_copies_and_identify_them() {
        let tmp = tempfile::Builder::new().prefix("cands-").tempdir().unwrap();
        let main = tmp.path().join("personal/hollow");
        std::fs::create_dir_all(main.join(".git/worktrees/wt")).unwrap();
        std::fs::write(main.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(main.join(".git/config"), "[core]\n\tbare = false\n[remote \"origin\"]\n\turl = https://user:tok@github.com/example-org/hollow-code.git\n").unwrap();
        // A linked worktree of the same repository, one level down elsewhere.
        let wt = tmp.path().join("work/hollow");
        std::fs::create_dir_all(&wt).unwrap();
        let gitdir = main.join(".git/worktrees/wt");
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
        std::fs::write(gitdir.join("HEAD"), "ref: refs/heads/feature/x\n").unwrap();
        std::fs::write(gitdir.join("commondir"), "../..\n").unwrap();
        // Two levels down is out of scope; node_modules is never scanned.
        std::fs::create_dir_all(tmp.path().join("a/b/deep/.git")).unwrap();
        std::fs::create_dir_all(tmp.path().join("node_modules/pkg/.git")).unwrap();
        let c = repo_candidates(tmp.path()).unwrap();
        assert_eq!(c.len(), 2, "{c:?}");
        assert_eq!(
            (c[0].parent.as_str(), c[0].branch.as_deref(), c[0].worktree),
            ("personal", Some("main"), false)
        );
        assert_eq!(
            c[0].remote.as_deref(),
            Some("github.com/example-org/hollow-code")
        );
        assert_eq!(
            (c[1].parent.as_str(), c[1].branch.as_deref(), c[1].worktree),
            ("work", Some("feature/x"), true)
        );
        assert_eq!(
            c[1].remote.as_deref(),
            Some("github.com/example-org/hollow-code")
        );
    }

    #[test]
    fn gh_uses_its_resolved_remote_or_a_lone_github_origin() {
        let fork = "[remote \"origin\"]\n\turl = git@github.com:me/r.git\n[remote \"upstream\"]\n\turl = https://github.com/them/r\n";
        assert_eq!(
            gh_remote(fork),
            None,
            "a fork: gh would ask, so guess nothing"
        );
        let set = format!("{fork}\tgh-resolved = base\n");
        assert_eq!(gh_remote(&set).as_deref(), Some("github.com/them/r"));
        let named =
            "[remote \"origin\"]\n\turl = git@github.com:me/r.git\n\tgh-resolved = them/r\n";
        assert_eq!(
            gh_remote(named).as_deref(),
            Some("github.com/them/r"),
            "a set-default to the parent"
        );
        assert_eq!(
            gh_remote("[remote \"origin\"]\n\turl = ssh://git@ssh.github.com:443/o/r.git\n")
                .as_deref(),
            Some("github.com/o/r")
        );
        assert_eq!(
            gh_remote("[remote \"origin\"]\n\turl = https://u:tok@github.com/o/r.git?x=1\n")
                .as_deref(),
            Some("github.com/o/r")
        );
        assert_eq!(gh_remote("[core]\n\tbare = false\n"), None);
    }

    #[test]
    fn origin_urls_reduce_to_host_owner_name() {
        assert_eq!(
            origin_url("[remote \"origin\"]\n\turl = git@github.com:o/r.git").as_deref(),
            Some("github.com/o/r")
        );
        assert_eq!(
            origin_url("[remote \"upstream\"]\n\turl = git@x:o/r").as_deref(),
            None
        );
        assert_eq!(
            origin_url("[remote \"origin\"]\nurl=ssh://git@host:22/o/r/").as_deref(),
            Some("host:22/o/r")
        );
    }

    /// M6 (PER-20): a build workspace record saved before the runner's
    /// neutral name says `mac-mini`. It reads as `runner` everywhere (the UI's
    /// binding, the turn's placement), reading leaves the record as it is,
    /// and only setting Runs on again writes `runner`.
    #[test]
    fn an_old_mac_mini_record_reads_as_the_runner_and_is_rewritten_only_on_a_save() {
        const LEGACY: &str = "mac-mini";
        let vault = tempfile::tempdir().unwrap();
        let worktree = tempfile::tempdir().unwrap();
        bind_for_test(vault.path(), "bw-old", worktree.path());
        // The old shape, as an earlier Hollow wrote it.
        let mut old =
            load().unwrap()[&session_store::vault_file_stem(vault.path())]["bw-old"].clone();
        old.runs_on = Some(LEGACY.into());
        put(vault.path(), "bw-old", Some(old)).unwrap();
        let stored = || -> Option<String> {
            let text = std::fs::read_to_string(store_path().unwrap()).unwrap();
            let store: Store = serde_json::from_str(&text).unwrap();
            store.vaults[&session_store::vault_file_stem(vault.path())]["bw-old"]
                .runs_on
                .clone()
        };
        assert_eq!(stored().as_deref(), Some("mac-mini"));
        for _ in 0..2 {
            assert_eq!(
                binding_for(vault.path(), "bw-old")
                    .unwrap()
                    .runs_on
                    .as_deref(),
                Some("runner")
            );
            assert_eq!(
                runs_on_for_cwd(vault.path(), worktree.path()),
                Some(Some("runner".into()))
            );
        }
        assert_eq!(
            stored().as_deref(),
            Some("mac-mini"),
            "reading never rewrites it"
        );
        assert_eq!(
            stored_runs_on(vault.path(), "bw-old").as_deref(),
            Some("mac-mini")
        );
        // The phone's failed-send undo puts the old value back as it was.
        set_runs_on(vault.path(), "bw-old", Some("this-mac")).unwrap();
        let info = restore_runs_on(vault.path(), "bw-old", Some("mac-mini")).unwrap();
        assert_eq!(info.runs_on.as_deref(), Some("runner"));
        assert_eq!(stored().as_deref(), Some("mac-mini"));
        // A save of either name writes `runner`; this Mac and default as before.
        let info = set_runs_on(vault.path(), "bw-old", Some("mac-mini")).unwrap();
        assert_eq!(info.runs_on.as_deref(), Some("runner"));
        assert_eq!(stored().as_deref(), Some("runner"));
        set_runs_on(vault.path(), "bw-old", Some(" runner ")).unwrap();
        assert_eq!(stored().as_deref(), Some("runner"));
        set_runs_on(vault.path(), "bw-old", Some("this-mac")).unwrap();
        assert_eq!(stored().as_deref(), Some("this-mac"));
        set_runs_on(vault.path(), "bw-old", Some("default")).unwrap();
        assert_eq!(stored(), None);
        assert!(set_runs_on(vault.path(), "bw-old", Some("moon")).is_err());
    }

    // ---- OC7: clone-seeded node_modules ----

    /// A checkout with a lock file and (optionally) an installed
    /// `node_modules` (a package folder, a top-level `.package-lock.json`, a
    /// `.bin` entry) carrying a recorded lock hash. `installed` is the mark
    /// text, or "" for the checkout's own lock hash.
    fn checkout(root: &Path, name: &str, lock: &str, installed: Option<&str>) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("package-lock.json"), lock).unwrap();
        if let Some(mark) = installed {
            let nm = dir.join("node_modules");
            std::fs::create_dir_all(nm.join("pkg")).unwrap();
            std::fs::create_dir_all(nm.join(".bin")).unwrap();
            std::fs::write(nm.join("pkg/index.js"), "module.exports = 1;\n").unwrap();
            std::fs::write(nm.join(".package-lock.json"), "{}").unwrap();
            std::fs::write(nm.join(".bin/tool"), "#!/bin/sh\n").unwrap();
            let hash = if mark.is_empty() {
                lock_hash(&dir).unwrap()
            } else {
                mark.to_string()
            };
            std::fs::write(nm.join(LOCK_MARK), format!("{hash}\n")).unwrap();
        }
        dir
    }

    /// Make `path` newer than its checkout's marker.
    fn touch_after_mark(checkout: &Path, path: &Path) {
        let mark = std::fs::metadata(checkout.join("node_modules").join(LOCK_MARK))
            .unwrap()
            .modified()
            .unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(mark + Duration::from_secs(10))
            .unwrap();
    }

    fn limits() -> SeedLimits {
        SeedLimits {
            deadline: Duration::from_secs(30),
            poll: Duration::from_millis(5),
        }
    }

    /// An executable stand-in for `cp` (its arguments are `-c -R <src> <dst>`).
    fn fake_cp(dir: &Path, body: &str) -> PathBuf {
        use std::hash::{Hash, Hasher};
        use std::os::unix::fs::PermissionsExt;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        body.hash(&mut h);
        let path = dir.join(format!("fake-cp-{:x}", h.finish()));
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn lock_hash_is_the_sha256_of_the_lock_file() {
        let tmp = tempfile::tempdir().unwrap();
        let a = checkout(tmp.path(), "a", "{}", None);
        // sha256("{}")
        assert_eq!(
            lock_hash(&a).as_deref(),
            Some("44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a")
        );
        assert_eq!(lock_hash(&tmp.path().join("none")), None);
    }

    #[test]
    fn pick_seed_takes_only_a_sibling_whose_install_matches_this_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let roots = [tmp.path().to_path_buf()];
        let me = checkout(tmp.path(), "me", "{\"v\":1}", None);
        let same = checkout(tmp.path(), "same", "{\"v\":1}", Some(""));
        let other_lock = checkout(tmp.path(), "other", "{\"v\":2}", Some(""));
        let no_install = checkout(tmp.path(), "bare", "{\"v\":1}", None);
        let stale_mark = checkout(tmp.path(), "stale", "{\"v\":1}", Some("deadbeef"));
        // Installed from the same lock, but its own lock file changed since
        // (an `npm install` there): its node_modules no longer matches.
        let drifted = checkout(tmp.path(), "drifted", "{\"v\":1}", Some(""));
        std::fs::write(drifted.join("package-lock.json"), "{\"v\":3}").unwrap();
        let all = [
            me.clone(),
            other_lock,
            no_install,
            stale_mark,
            drifted,
            same.clone(),
        ];
        assert_eq!(pick_seed(&me, &all, &roots), Some(same));
        assert_eq!(pick_seed(&me, &all[..5], &roots), None);
        // This worktree never seeds from itself.
        let installed_me = checkout(tmp.path(), "me2", "{\"v\":1}", Some(""));
        assert_eq!(
            pick_seed(&installed_me, std::slice::from_ref(&installed_me), &roots),
            None
        );
        // A worktree with no lock file has nothing to match.
        let nolock = tmp.path().join("nolock");
        std::fs::create_dir_all(&nolock).unwrap();
        assert_eq!(pick_seed(&nolock, &all, &roots), None);
    }

    #[test]
    fn pick_seed_only_trusts_siblings_under_the_given_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let me = checkout(tmp.path(), "me", "{}", None);
        let outside = checkout(elsewhere.path(), "outside", "{}", Some(""));
        let roots = [tmp.path().to_path_buf()];
        assert_eq!(pick_seed(&me, std::slice::from_ref(&outside), &roots), None);
        let roots = [tmp.path().to_path_buf(), elsewhere.path().to_path_buf()];
        assert_eq!(
            pick_seed(&me, std::slice::from_ref(&outside), &roots),
            Some(outside)
        );
        assert_eq!(pick_seed(&me, &[], &[]), None);
    }

    #[test]
    fn pick_seed_never_takes_a_symlinked_node_modules() {
        let tmp = tempfile::tempdir().unwrap();
        let roots = [tmp.path().to_path_buf()];
        let me = checkout(tmp.path(), "me", "{}", None);
        let real = checkout(tmp.path(), "real", "{}", Some(""));
        let linked = checkout(tmp.path(), "linked", "{}", None);
        std::os::unix::fs::symlink(real.join("node_modules"), linked.join("node_modules")).unwrap();
        assert_eq!(pick_seed(&me, &[linked], &roots), None);
    }

    #[test]
    fn a_marker_that_is_a_symlink_or_a_fifo_or_huge_is_never_trusted() {
        let tmp = tempfile::tempdir().unwrap();
        let roots = [tmp.path().to_path_buf()];
        let me = checkout(tmp.path(), "me", "{}", None);
        let hash = lock_hash(&me).unwrap();
        // A symlink to a file holding the right hash.
        let linked = checkout(tmp.path(), "linked", "{}", Some(""));
        let target = tmp.path().join("hash.txt");
        std::fs::write(&target, &hash).unwrap();
        let mark = linked.join("node_modules").join(LOCK_MARK);
        std::fs::remove_file(&mark).unwrap();
        std::os::unix::fs::symlink(&target, &mark).unwrap();
        assert_eq!(pick_seed(&me, std::slice::from_ref(&linked), &roots), None);
        assert!(write_lock_mark(&linked).is_err());
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            hash,
            "the link's target was not written through"
        );
        // A FIFO: refused without blocking on a read.
        let fifo = checkout(tmp.path(), "fifo", "{}", Some(""));
        let mark = fifo.join("node_modules").join(LOCK_MARK);
        std::fs::remove_file(&mark).unwrap();
        let c = std::ffi::CString::new(mark.to_string_lossy().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        assert_eq!(pick_seed(&me, std::slice::from_ref(&fifo), &roots), None);
        assert!(write_lock_mark(&fifo).is_err());
        // Far more than the read limit of junk is not a match.
        let junk = "a".repeat(100_000);
        let huge = checkout(tmp.path(), "huge", "{}", Some(junk.as_str()));
        assert_eq!(pick_seed(&me, std::slice::from_ref(&huge), &roots), None);
    }

    #[test]
    fn a_shallow_change_after_the_marker_drops_the_sibling() {
        let tmp = tempfile::tempdir().unwrap();
        let roots = [tmp.path().to_path_buf()];
        let me = checkout(tmp.path(), "me", "{}", None);
        let clean = checkout(tmp.path(), "clean", "{}", Some(""));
        assert_eq!(
            pick_seed(&me, std::slice::from_ref(&clean), &roots),
            Some(clean.clone())
        );
        // A top-level file newer than the marker.
        let top = checkout(tmp.path(), "top", "{}", Some(""));
        touch_after_mark(&top, &top.join("node_modules/.package-lock.json"));
        assert_eq!(pick_seed(&me, std::slice::from_ref(&top), &roots), None);
        // A `.bin` entry newer than the marker.
        let bin = checkout(tmp.path(), "bin", "{}", Some(""));
        touch_after_mark(&bin, &bin.join("node_modules/.bin/tool"));
        assert_eq!(pick_seed(&me, std::slice::from_ref(&bin), &roots), None);
        // A package folder's own mtime is not compared (a clone resets it).
        let dir = checkout(tmp.path(), "dir", "{}", Some(""));
        let mark = std::fs::metadata(dir.join("node_modules").join(LOCK_MARK))
            .unwrap()
            .modified()
            .unwrap();
        std::fs::File::open(dir.join("node_modules/pkg"))
            .unwrap()
            .set_modified(mark + Duration::from_secs(10))
            .unwrap();
        assert_eq!(
            pick_seed(&me, std::slice::from_ref(&dir), &roots),
            Some(dir.clone())
        );
    }

    #[test]
    fn a_clone_never_crosses_volumes() {
        assert!(same_volume(&[]));
        assert!(same_volume(&[7]));
        assert!(same_volume(&[7, 7, 7]));
        assert!(!same_volume(&[7, 8, 7]));
        assert!(!same_volume(&[7, 7, 9]));
    }

    #[test]
    fn clone_args_are_an_apfs_clone_never_a_link() {
        let args = clone_args(Path::new("/a/node_modules"), Path::new("/b/tmp"));
        assert_eq!(args, ["-c", "-R", "/a/node_modules", "/b/tmp"]);
        assert!(!args.iter().any(|a| a == "-l" || a == "-s"));
    }

    #[test]
    fn worktree_list_gives_every_checkout_path() {
        let porcelain = "worktree /r/main\nHEAD abc\nbranch refs/heads/main\n\nworktree /r/wt one\nHEAD def\ndetached\n\n";
        assert_eq!(
            parse_worktree_list(porcelain),
            [PathBuf::from("/r/main"), PathBuf::from("/r/wt one")]
        );
        assert!(parse_worktree_list("").is_empty());
    }

    #[test]
    fn a_second_worktree_with_the_same_lock_is_seeded_by_a_real_clone() {
        let tmp = tempfile::tempdir().unwrap();
        let roots = [tmp.path().to_path_buf()];
        let first = checkout(tmp.path(), "first", "{\"v\":1}", Some(""));
        let second = checkout(tmp.path(), "second", "{\"v\":1}", None);
        let staging = tmp.path().join("staging");
        let seeded = seed_node_modules_with(
            &second,
            std::slice::from_ref(&first),
            &roots,
            &staging,
            Path::new("/bin/cp"),
            &limits(),
        );
        if !is_apfs(tmp.path()) {
            // Not APFS: production never calls this (seed_new_worktree checks), and
            // cp may have copied instead; only the no-link rule below is asserted.
            return;
        }
        assert_eq!(seeded, Seed::Cloned(first.clone()));
        let nm = second.join("node_modules");
        assert_eq!(
            std::fs::read_to_string(nm.join("pkg/index.js")).unwrap(),
            "module.exports = 1;\n"
        );
        assert_eq!(
            std::fs::read_to_string(nm.join(LOCK_MARK)).unwrap().trim(),
            lock_hash(&second).unwrap()
        );
        // A clone, not a link: no symlink, and a write in one never shows in the other.
        assert!(!std::fs::symlink_metadata(&nm)
            .unwrap()
            .file_type()
            .is_symlink());
        std::fs::write(nm.join("pkg/index.js"), "changed").unwrap();
        assert_eq!(
            std::fs::read_to_string(first.join("node_modules/pkg/index.js")).unwrap(),
            "module.exports = 1;\n"
        );
        assert!(!staging.exists(), "no staging folder is left");
    }

    #[test]
    fn a_changed_lock_or_a_failed_clone_falls_back_without_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let roots = [tmp.path().to_path_buf()];
        let staging = tmp.path().join("staging");
        let first = checkout(tmp.path(), "first", "{\"v\":1}", Some(""));
        let from = std::slice::from_ref(&first);
        let changed = checkout(tmp.path(), "changed", "{\"v\":2}", None);
        assert_eq!(
            seed_node_modules_with(
                &changed,
                from,
                &roots,
                &staging,
                Path::new("/bin/cp"),
                &limits()
            ),
            Seed::Skipped("no sibling has an install of this lock file")
        );
        assert!(!changed.join("node_modules").exists());
        // cp failing leaves nothing behind.
        let same = checkout(tmp.path(), "same", "{\"v\":1}", None);
        assert_eq!(
            seed_node_modules_with(
                &same,
                from,
                &roots,
                &staging,
                Path::new("/usr/bin/false"),
                &limits()
            ),
            Seed::Skipped("the clone did not work")
        );
        assert!(!same.join("node_modules").exists());
        assert!(!staging.exists());
        assert_eq!(std::fs::read_dir(&same).unwrap().count(), 1);
        // A node_modules already there is left alone.
        let there = checkout(tmp.path(), "there", "{\"v\":1}", Some("beef"));
        assert_eq!(
            seed_node_modules_with(
                &there,
                from,
                &roots,
                &staging,
                Path::new("/bin/cp"),
                &limits()
            ),
            Seed::Skipped("node_modules is already there")
        );
    }

    #[test]
    fn a_torn_or_stale_clone_is_discarded_and_a_slow_one_is_killed() {
        let tmp = tempfile::tempdir().unwrap();
        let roots = [tmp.path().to_path_buf()];
        let staging = tmp.path().join("staging");
        let first = checkout(tmp.path(), "first", "{\"v\":1}", Some(""));
        let from = std::slice::from_ref(&first);
        let target = checkout(tmp.path(), "target", "{\"v\":1}", None);
        let run = |cp: &Path, limits: &SeedLimits| {
            seed_node_modules_with(&target, from, &roots, &staging, cp, limits)
        };
        // A torn clone: cp exits 0 having made part of the tree and no marker.
        let torn = fake_cp(
            tmp.path(),
            "mkdir -p \"$4/pkg\"; echo x > \"$4/pkg/index.js\"",
        );
        assert_eq!(
            run(&torn, &limits()),
            Seed::Skipped("the clone did not match the lock file")
        );
        assert!(!staging.exists(), "the torn clone is removed");
        assert!(!target.join("node_modules").exists());
        // The sibling's lock changed while the copy ran.
        let lock = first.join("package-lock.json");
        let racing = fake_cp(
            tmp.path(),
            &format!(
                "/bin/cp -c -R \"$3\" \"$4\"; echo '{{\"v\":9}}' > '{}'",
                lock.display()
            ),
        );
        assert_eq!(
            run(&racing, &limits()),
            Seed::Skipped("the clone did not match the lock file")
        );
        assert!(!staging.exists());
        assert!(!target.join("node_modules").exists());
        std::fs::write(&lock, "{\"v\":1}").unwrap();
        // A clone that never ends is killed at the deadline, and its staging removed.
        // sleep-ok: the fake cp has to outlast the 300 ms test deadline to be killed
        let slow = fake_cp(tmp.path(), "mkdir -p \"$4\"; exec /bin/sleep 60");
        let started = std::time::Instant::now();
        let short = SeedLimits {
            deadline: Duration::from_millis(300),
            poll: Duration::from_millis(10),
        };
        assert_eq!(run(&slow, &short), Seed::Skipped("the clone took too long"));
        assert!(started.elapsed() < Duration::from_secs(20));
        assert!(!staging.exists());
        assert!(!target.join("node_modules").exists());
        // A stale staging folder from an earlier run is cleared first.
        std::fs::create_dir_all(staging.join("junk")).unwrap();
        let ok = run(Path::new("/bin/cp"), &limits());
        assert!(!staging.exists(), "{ok:?}");
    }

    #[tokio::test]
    async fn create_seeds_node_modules_from_the_main_checkout_with_the_same_lock() {
        let (_tmp, repo, vault, roots) = fixture();
        std::fs::write(repo.join("package-lock.json"), "{\"lockfileVersion\":3}").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-q", "-m", "lock"]);
        std::fs::create_dir_all(repo.join("node_modules/pkg")).unwrap();
        std::fs::write(repo.join("node_modules/pkg/index.js"), "1").unwrap();
        write_lock_mark(&repo).unwrap();
        let input = |title: &str| CreateInput {
            title: title.into(),
            goal: "g".into(),
            repo: repo.to_string_lossy().into(),
            branch: None,
            base: None,
        };
        let made = create(&vault, &roots, input("Seeded")).await.unwrap();
        let wt = PathBuf::from(&binding_for(&vault, &made.workspace.id).unwrap().worktree);
        if !is_apfs(&wt) {
            // Only a volume that cannot clone may skip the seed.
            assert!(!wt.join("node_modules").exists());
            return;
        }
        assert!(wt.join("node_modules/pkg/index.js").exists());
        assert_eq!(
            lock_hash(&wt),
            read_mark(&wt.join("node_modules")),
            "the clone carries the lock marker"
        );
        // The staging folder is neither in the worktree nor in `git status`.
        let status = git(&wt, &["status", "--porcelain"], QUICK).await.unwrap();
        assert!(!status.contains("seed"), "{status}");
        assert!(!wt.join(".node_modules-hollow-seed").exists());
        // A changed lock in the repo: no seed, no failure.
        std::fs::write(repo.join("package-lock.json"), "{\"lockfileVersion\":2}").unwrap();
        sh(&repo, &["commit", "-q", "-am", "newer lock"]);
        let made = create(&vault, &roots, input("Changed lock")).await.unwrap();
        let wt = PathBuf::from(&binding_for(&vault, &made.workspace.id).unwrap().worktree);
        assert!(wt.join("package-lock.json").exists());
        assert!(!wt.join("node_modules").exists());
    }

    /// OC15: an approved setup hook runs silently on create; an edited one
    /// doesn't run and leaves a warning naming the approve action.
    #[tokio::test]
    async fn create_runs_an_approved_setup_hook_and_warns_for_an_edited_one() {
        let (_tmp, repo, vault, roots) = fixture();
        std::fs::create_dir_all(repo.join(".hollow")).unwrap();
        std::fs::write(repo.join(".hollow/setup.sh"), "touch made-by-hook\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-q", "-m", "hook"]);
        let hash = crate::setup_hook::sha256_hex(b"touch made-by-hook\n");
        crate::repo_trust::approve_setup_hook_in(
            &crate::repo_trust::store_path().unwrap(),
            &crate::setup_hook::repo_of(&repo),
            &hash,
            "t",
        )
        .unwrap();
        let input = |title: &str| CreateInput {
            title: title.into(),
            goal: "g".into(),
            repo: repo.to_string_lossy().into(),
            branch: None,
            base: None,
        };
        let made = create(&vault, &roots, input("Approved")).await.unwrap();
        // The create returns at once with a note saying where the result goes.
        assert_eq!(made.warnings.len(), 1, "{:?}", made.warnings);
        assert!(made.warnings[0].contains("running in the background"));
        let result = crate::setup_hook::result_path(&vault, &made.workspace.id).unwrap();
        assert!(made.warnings[0].contains(&result.display().to_string()));
        let wt = PathBuf::from(&binding_for(&vault, &made.workspace.id).unwrap().worktree);
        let mut text = String::new();
        for _ in 0..300 {
            text = std::fs::read_to_string(&result).unwrap_or_default();
            if text.contains("- Status: ok") {
                break;
            }
            // sleep-ok: waits for the background hook run to write its result (bounded at 30 s).
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(text.contains("- Status: ok"), "{text}");
        assert!(wt.join("made-by-hook").exists(), "the approved hook ran");
        // An edited hook asks again: not run, a warning.
        std::fs::write(
            repo.join(".hollow/setup.sh"),
            "touch made-by-hook\ncurl x\n",
        )
        .unwrap();
        sh(&repo, &["commit", "-q", "-am", "edit"]);
        let made = create(&vault, &roots, input("Edited")).await.unwrap();
        let wt = PathBuf::from(&binding_for(&vault, &made.workspace.id).unwrap().worktree);
        assert!(!wt.join("made-by-hook").exists());
        assert_eq!(made.warnings.len(), 1, "{:?}", made.warnings);
        assert!(made.warnings[0].contains("build.approve_setup_hook"));
    }

    #[test]
    fn writing_the_mark_needs_an_install_and_a_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let bare = checkout(tmp.path(), "bare", "{}", None);
        assert!(write_lock_mark(&bare).is_err());
        let installed = checkout(tmp.path(), "installed", "{}", Some("old"));
        write_lock_mark(&installed).unwrap();
        assert_eq!(
            read_mark(&installed.join("node_modules")),
            lock_hash(&installed)
        );
    }
}
