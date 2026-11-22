//! Build and install Hollow itself from a build workspace (Hollow build
//! programme, PR B5; see docs/history/HOLLOW_BUILD_PLAN.md).
//!
//! Two steps, both from the Changes pane of a build workspace whose repository
//! is Hollow (`src-tauri/tauri.conf.json` with identifier `com.hollow.app`):
//!
//! 1. **Build app** runs `npm run tauri:bundle` in the worktree (or, with
//!    Settings, App builds: Remote runner, on the runner and signs the
//!    result here, falling back to this Mac: [`RemoteAppBuild`]; after
//!    `npm ci` when `node_modules` is missing, and `xattr -cr src-tauri/models`)
//!    as ONE GUI-owned background job through `process_group::drive`: its own
//!    `CARGO_TARGET_DIR` under `~/Library/Caches/hollow-install-target-<id>`
//!    (outside iCloud), `CARGO_BUILD_JOBS=8`, a 45-minute deadline and a stop
//!    signal. At most one job app-wide, and none starts while memory is short
//!    (free < 30 % or macOS pressure at warning): this machine allows two
//!    bounded toolchain slots and the build is one. The log goes to
//!    `~/Library/Logs/Hollow/self-install/<id>-<stamp>.log`.
//! 2. **Installing** is the update client's (row U10, `update_client::local`):
//!    a finished build is offered in the top bar's Update button as "Local
//!    build of <branch>" and installed by `hollow install-update` under the
//!    restart checks, with the same no-expiry "Update when chats finish"
//!    wait. This module keeps the build, its receipt, the finished build
//!    saved across a restart and the install record (`last-result.json`).
//!    The old Install and restart card, its helper script and the 30-minute
//!    waiter are gone.
//!
//! `self_install_result` reads `last-result.json` (written for a Local
//! build by the update client after the restart, or by older helpers) on
//! each call and returns it until the user acknowledges it (a launch-time snapshot missed results
//! written after the app started, and the reopen check's rewrite). Its
//! `state` and the receipt's say `installed`, `failed` (the previous app is
//! verifiably still there) or `unknown`: never "kept" without the
//! installer saying so (merged plan slice 6, review B·F02).

use crate::process_group::{
    self, Ended, GroupGuard, StderrPolicy, StopSignal, Supervision, Timings,
};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const EVENT: &str = "self-install-event";
const BUILD_DEADLINE: Duration = Duration::from_secs(45 * 60);
const TAIL_LINES: usize = 60;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BuildState {
    Idle,
    Building,
    Built,
    Failed,
}

/// What the Changes pane shows (and `self-install-event` carries).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppBuildStatus {
    /// Authoritative reads include the install guard; job events are hints only.
    pub blocked: Option<String>,
    pub workspace_id: String,
    /// The worktree is Hollow's own repository.
    pub is_hollow: bool,
    pub state: BuildState,
    /// A job for another workspace is using the build slot.
    pub other_job: Option<String>,
    pub log: Option<String>,
    /// The newest log lines, redacted.
    pub log_tail: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub error: Option<String>,
    pub commit: Option<String>,
    pub branch: Option<String>,
    /// The finished build's receipt (merged plan slice 6, review A·S4/B·F12):
    /// what the install card shows and its approval binds.
    pub receipt: Option<BuildReceipt>,
}

/// What a finished build is, recorded once when it finishes (merged plan
/// slice 6, review A·S4/B·F12). The install card shows it, the approval binds
/// it, and an install re-hashes the bundle and refuses anything else.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct BuildReceipt {
    /// Unique per finished build: a rebuild of the same workspace gets a new one.
    pub build_id: String,
    pub workspace_id: String,
    /// The checkout it was built from (the card's diff runs there).
    pub worktree: String,
    /// The built `Hollow.app`, the one the installer takes.
    pub bundle: String,
    /// sha256 over every entry of the bundle: relative path, kind, the
    /// executable bit, and each file's bytes or each link's target.
    pub bundle_digest: String,
    /// From the bundle's embedded `build-provenance.json`.
    pub source_fingerprint: String,
    pub commit: String,
    pub dirty: bool,
    /// The installed app's commit when the build finished: what the card's
    /// diff is measured against. Empty when unknown.
    pub base_commit: String,
    pub recorded_at: String,
}

struct Job {
    workspace_id: String,
    state: BuildState,
    worktree: PathBuf,
    target_dir: PathBuf,
    log: PathBuf,
    tail: VecDeque<String>,
    started_at: String,
    finished_at: Option<String>,
    error: Option<String>,
    commit: String,
    branch: String,
    stop: Arc<StopSignal>,
    /// Set when the build finished and its receipt was recorded.
    receipt: Option<BuildReceipt>,
}

static JOB: Lazy<Mutex<Option<Job>>> = Lazy::new(|| Mutex::new(None));

/// True when `worktree` is Hollow's repository.
pub fn is_hollow_repo(worktree: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(worktree.join("src-tauri/tauri.conf.json")) else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| {
            v.get("identifier")
                .and_then(|i| i.as_str())
                .map(|i| i == "com.hollow.app")
        })
        .unwrap_or(false)
}

/// Why a build must not start now, from the free-memory percentage and the
/// macOS pressure level (1 normal, 2 warning, 4 critical). Unreadable probes
/// never refuse.
pub(crate) fn memory_refusal(free_pct: Option<u8>, pressure: Option<i32>) -> Option<String> {
    if let Some(level) = pressure.filter(|l| *l >= 2) {
        let name = if level >= 4 { "critical" } else { "warning" };
        return Some(format!("macOS reports memory pressure at {name}; wait for other builds to finish, then try again"));
    }
    if let Some(free) = free_pct.filter(|f| *f < 30) {
        return Some(format!("Only {free}% of memory is free (a build needs at least 30%); wait for other builds to finish, then try again"));
    }
    None
}

fn status_of(id: &str, is_hollow: bool, job: Option<&Job>) -> AppBuildStatus {
    match job {
        Some(job) if job.workspace_id == id => AppBuildStatus {
            blocked: None,
            workspace_id: id.to_string(),
            is_hollow,
            state: job.state,
            other_job: None,
            log: Some(job.log.to_string_lossy().into()),
            log_tail: crate::claude_runner::redact(
                &job.tail.iter().cloned().collect::<Vec<_>>().join("\n"),
            ),
            started_at: Some(job.started_at.clone()),
            finished_at: job.finished_at.clone(),
            error: job.error.clone(),
            commit: Some(job.commit.clone()),
            branch: Some(job.branch.clone()),
            receipt: job.receipt.clone(),
        },
        other => AppBuildStatus {
            blocked: None,
            workspace_id: id.to_string(),
            is_hollow,
            state: BuildState::Idle,
            other_job: other
                .filter(|j| j.state == BuildState::Building)
                .map(|j| j.workspace_id.clone()),
            log: None,
            log_tail: String::new(),
            started_at: None,
            finished_at: None,
            error: None,
            commit: None,
            branch: None,
            receipt: None,
        },
    }
}

fn lock() -> std::sync::MutexGuard<'static, Option<Job>> {
    JOB.lock().unwrap_or_else(|e| e.into_inner())
}

/// The cargo targets a build still holds, with why: the current job's
/// (running, or finished and not yet installed) and a finished build saved
/// for after a restart. `install_targets` never prunes these.
pub(crate) fn held_targets() -> Vec<(PathBuf, String)> {
    held_builds()
        .into_iter()
        .map(|b| (b.target_dir, b.why.to_string()))
        .collect()
}

/// The worktrees a build still needs, with why: the current job's while it
/// builds, waits to install or installs (the install step runs
/// `npm run tauri:install` there), and a finished build saved for after a
/// restart. `worktree_hygiene` never removes these (overnight S01a: a
/// release checkout that moved on looked finished and went, and Install and
/// restart then said "The worktree is gone"). A failed build holds none.
pub(crate) fn held_worktrees() -> Vec<(PathBuf, String)> {
    held_builds()
        .into_iter()
        .filter(|b| b.needs_worktree)
        .map(|b| (b.worktree, b.why.to_string()))
        .collect()
}

/// A build [`held_targets`] and [`held_worktrees`] keep things for.
struct HeldBuild {
    target_dir: PathBuf,
    worktree: PathBuf,
    why: &'static str,
    /// Building, waiting to install or installing: it still reads its
    /// worktree. A failed build's target stays; its worktree needn't.
    needs_worktree: bool,
}

/// The current job (when it holds anything) and the saved finished build.
fn held_builds() -> Vec<HeldBuild> {
    let mut held = Vec::new();
    if let Some(job) = lock().as_ref() {
        let why = match job.state {
            BuildState::Idle => None,
            BuildState::Building => Some(("a build running now", true)),
            BuildState::Built => Some(("a finished build waiting to install", true)),
            BuildState::Failed => Some(("the last build (failed)", false)),
        };
        if let Some((why, needs_worktree)) = why {
            held.push(HeldBuild {
                target_dir: job.target_dir.clone(),
                worktree: job.worktree.clone(),
                why,
                needs_worktree,
            });
        }
    }
    if let Some(saved) = state_dir()
        .ok()
        .and_then(|dir| std::fs::read_to_string(dir.join(SAVED_BUILD)).ok())
        .and_then(|text| serde_json::from_str::<SavedBuild>(&text).ok())
    {
        held.push(HeldBuild {
            target_dir: saved.target_dir,
            worktree: saved.worktree,
            why: "a finished build waiting to install",
            needs_worktree: true,
        });
    }
    held
}

/// The build status of workspace `id`.
pub fn status(vault: &Path, id: &str) -> AppBuildStatus {
    let is_hollow = crate::build_workspaces::binding_for(vault, id)
        .filter(|b| b.exists)
        .is_some_and(|b| is_hollow_repo(Path::new(&b.worktree)));
    let mut status = status_of(id, is_hollow, lock().as_ref());
    // Never query running work while holding the build job lock.
    status.blocked = busy_refusal(vault);
    status
}

// ------------------------------------------------------------ paths & tools

fn home() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "No home folder".into())
}

fn private_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

fn log_dir() -> Result<PathBuf, String> {
    Ok(crate::instance::logs_dir(&home()?).join("self-install"))
}

pub(crate) fn state_dir() -> Result<PathBuf, String> {
    Ok(crate::agent_settings::app_data_dir()?.join("self-install"))
}

/// Remove `dir`, a build cache folder, only when it is a real folder (not a
/// link) directly inside `root` (night ST7, 2026-10-14). Errors are ignored:
/// a cache left behind costs disk, never data. True when it was removed.
pub(crate) fn remove_cache_dir(dir: &Path, root: &Path) -> bool {
    if dir.parent() != Some(root) {
        return false;
    }
    let real = std::fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_dir());
    real && std::fs::remove_dir_all(dir).is_ok()
}

/// Delete workspace `id`'s install target once nothing can build from it: its
/// worktree was removed or the workspace trashed (night ST7). Each was 2–4 GB
/// and stayed in `~/Library/Caches` for good. Not while `id`'s build job runs.
pub(crate) fn forget_build_output(id: &str) -> bool {
    // Tests never touch this Mac's real caches.
    if cfg!(test) {
        return false;
    }
    let job_holds = lock().as_ref().is_some_and(|j| j.workspace_id == id);
    let Ok(dir) = target_dir(id) else {
        return false;
    };
    let Some(root) = dir.parent().map(Path::to_path_buf) else {
        return false;
    };
    let held: Vec<PathBuf> = held_targets().into_iter().map(|(p, _)| p).collect();
    let prunable = crate::install_targets::plan(0)
        .map(|plan| plan.remove)
        .unwrap_or_default();
    may_forget(&dir, job_holds, &held, &prunable) && remove_cache_dir(&dir, &root)
}

/// Whether a removed workspace's install target may go: no build job is on
/// that workspace (in any state: building, built, installing), no build holds
/// it, and the confirmed prune would remove it with no "keep newest" (so not
/// the installed app's or the rollback copy's target).
fn may_forget(dir: &Path, job_holds: bool, held: &[PathBuf], prunable: &[String]) -> bool {
    !job_holds && !held.iter().any(|p| p == dir) && prunable.iter().any(|p| Path::new(p) == dir)
}

/// The build's `CARGO_TARGET_DIR`: per workspace, outside iCloud.
pub(crate) fn target_dir(id: &str) -> Result<PathBuf, String> {
    Ok(home()?.join("Library/Caches").join(format!(
        "{}-install-target-{}",
        crate::instance::current().cache_name(),
        crate::build_workspaces::slug(id, 80)
    )))
}

/// The system folders every PATH Hollow builds ends with.
pub(crate) const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// A PATH for the build: a Finder-launched app inherits almost none, and
/// npm, node, cargo and rustc live in the user's tool folders.
pub(crate) fn tool_path(inherited: Option<&str>, home: &Path) -> String {
    let mut dirs: Vec<String> = vec![
        home.join(".cargo/bin").to_string_lossy().into(),
        home.join(".local/bin").to_string_lossy().into(),
        "/opt/homebrew/bin".into(),
        "/usr/local/bin".into(),
    ];
    for d in inherited
        .unwrap_or_default()
        .split(':')
        .filter(|d| !d.is_empty())
    {
        dirs.push(d.to_string());
    }
    for d in SYSTEM_PATH.split(':') {
        dirs.push(d.into());
    }
    let mut seen = std::collections::BTreeSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    dirs.join(":")
}

fn find_npm(path: &str) -> Option<PathBuf> {
    path.split(':')
        .map(|d| Path::new(d).join("npm"))
        .find(|p| p.is_file())
}

fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

// ------------------------------------------------------------ the build job

/// Everything one build run needs (the seams tests replace).
pub(crate) struct BuildSpec {
    pub worktree: PathBuf,
    pub target_dir: PathBuf,
    pub npm: PathBuf,
    pub path_env: String,
    pub log: PathBuf,
    pub deadline: Duration,
    /// Smart routing 7 (2026-11-03): compile on the remote runner and sign
    /// here (Settings, Advanced, App builds: Mac mini); `None`
    /// builds here, as before.
    pub remote: Option<RemoteAppBuild>,
}

/// How an app build compiles on the remote runner: through `hollow-slot
/// --on <host>` (`slot_tool`), with ad-hoc signing there (the runner has no
/// signing key), the zipped bundle fetched back, unpacked where the install
/// step looks for it ([`built_bundle`]) and signed again here with
/// `identity` and `entitlements` (the worktree's `tauri.conf.json`), the
/// hardened runtime on, as Tauri signs a build made here. Any failure falls
/// back to building here ([`run_build`]). What comes back is signed with
/// this Mac's identity and so gets Hollow's macOS permissions: it is for a
/// runner this person controls (Settings says so).
pub(crate) struct RemoteAppBuild {
    pub slot_tool: PathBuf,
    pub host: String,
    pub codesign: PathBuf,
    pub identity: String,
    pub entitlements: PathBuf,
}

/// The worktree's signing identity and entitlements file from
/// `src-tauri/tauri.conf.json` (`bundle.macOS`), for [`RemoteAppBuild`].
pub(crate) fn signing_of(worktree: &Path) -> Option<(String, PathBuf)> {
    let text = std::fs::read_to_string(worktree.join("src-tauri/tauri.conf.json")).ok()?;
    let conf: serde_json::Value = serde_json::from_str(&text).ok()?;
    let mac = conf.get("bundle")?.get("macOS")?;
    let identity = mac.get("signingIdentity")?.as_str()?.trim().to_string();
    let entitlements = mac.get("entitlements")?.as_str()?.trim().to_string();
    if identity.is_empty() || identity == "-" || entitlements.is_empty() {
        return None;
    }
    Some((identity, worktree.join("src-tauri").join(entitlements)))
}

/// Where a remote app build's zipped bundle comes back to: inside the
/// worktree (a `--fetch` path must be) and under `src-tauri/target/`, which
/// git ignores, so the tree stays clean.
const REMOTE_APP_OUT: &str = "src-tauri/target/hollow-remote-app";
/// The longest the runner may take: the build here, if it comes to that,
/// gets a whole deadline of its own (review of #1124).
const REMOTE_BUILD_DEADLINE: Duration = Duration::from_secs(30 * 60);

/// The script the runner runs: it refuses to run anywhere but a macOS
/// runner (`HOLLOW_REMOTE_HEAD` is set only there, so hollow-slot's own
/// fallback here becomes a failure and the normal build here runs), builds
/// with ad-hoc signing and this Mac's checkout named in the provenance
/// (`scripts/source-provenance.mjs`), and zips the bundle where `--fetch`
/// brings it back.
fn remote_build_script(worktree: &Path) -> String {
    format!(
        "set -e; [ -n \"${{HOLLOW_REMOTE_HEAD:-}}\" ] || {{ echo 'hollow-slot ran this on this Mac, not the runner'; exit 3; }}; [ \"$(uname -s)\" = Darwin ] || {{ echo 'the runner is not a Mac, so it cannot build the macOS app'; exit 3; }}; rm -rf {REMOTE_APP_OUT}; env CI=true APPLE_SIGNING_IDENTITY=- HOLLOW_PROVENANCE_CHECKOUT={} npm run tauri:bundle; mkdir -p {REMOTE_APP_OUT}; ditto -c -k --keepParent \"$CARGO_TARGET_DIR/release/bundle/macos/Hollow.app\" {REMOTE_APP_OUT}/Hollow.zip",
        sh_quote(&worktree.to_string_lossy())
    )
}

/// The build on the remote runner ([`RemoteAppBuild`]), its fetched folder
/// removed however it ends.
async fn run_remote_build(
    spec: &BuildSpec,
    remote: &RemoteAppBuild,
    stop: &StopSignal,
    both: &mut (dyn FnMut(&str) + Send),
) -> Result<PathBuf, String> {
    let out = spec.worktree.join(REMOTE_APP_OUT);
    let _ = std::fs::remove_dir_all(&out);
    let result = remote_build_steps(spec, remote, stop, &out, both).await;
    let _ = std::fs::remove_dir_all(&out);
    result
}

async fn remote_build_steps(
    spec: &BuildSpec,
    remote: &RemoteAppBuild,
    stop: &StopSignal,
    out: &Path,
    both: &mut (dyn FnMut(&str) + Send),
) -> Result<PathBuf, String> {
    both(&format!(
        "[self-install] App builds: Remote runner: compiling on {}, signing here",
        remote.host
    ));
    let script = remote_build_script(&spec.worktree);
    run_step(
        spec,
        &remote.slot_tool,
        &[
            "--on",
            &remote.host,
            "--fetch",
            REMOTE_APP_OUT,
            "/bin/sh",
            "-c",
            &script,
        ],
        stop,
        spec.deadline.min(REMOTE_BUILD_DEADLINE),
        both,
    )
    .await?;
    let zip = out.join("Hollow.zip");
    if !zip.is_file() {
        return Err(format!("no bundle came back to {}", zip.display()));
    }
    let bundle = built_bundle(&spec.target_dir);
    let folder = bundle.parent().expect("the bundle has a folder");
    std::fs::create_dir_all(folder).map_err(|e| format!("create {}: {e}", folder.display()))?;
    // Unpacked beside it and checked (one Hollow.app, a real folder) before
    // the old bundle goes.
    let unpack = folder.join(".hollow-remote-unpack");
    let _ = std::fs::remove_dir_all(&unpack);
    std::fs::create_dir_all(&unpack).map_err(|e| format!("create {}: {e}", unpack.display()))?;
    let short = Duration::from_secs(300);
    let zip_s = zip.to_string_lossy().into_owned();
    let unpack_s = unpack.to_string_lossy().into_owned();
    let unpacked = run_step(
        spec,
        Path::new("/usr/bin/ditto"),
        &["-x", "-k", &zip_s, &unpack_s],
        stop,
        short,
        both,
    )
    .await;
    let entries: Vec<String> = std::fs::read_dir(&unpack)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    let app = unpack.join("Hollow.app");
    let fine = unpacked.is_ok()
        && entries == ["Hollow.app"]
        && std::fs::symlink_metadata(&app).is_ok_and(|m| m.is_dir())
        && app.join("Contents/MacOS/hollow").is_file();
    if !fine {
        let _ = std::fs::remove_dir_all(&unpack);
        unpacked?;
        return Err(format!(
            "the zip that came back doesn't hold one Hollow.app (it holds {entries:?})"
        ));
    }
    if bundle.exists() {
        std::fs::remove_dir_all(&bundle)
            .map_err(|e| format!("remove the old {}: {e}", bundle.display()))?;
    }
    std::fs::rename(&app, &bundle)
        .map_err(|e| format!("move the bundle into {}: {e}", bundle.display()))?;
    let _ = std::fs::remove_dir_all(&unpack);
    let bundle_s = bundle.to_string_lossy().into_owned();
    let entitlements_s = remote.entitlements.to_string_lossy().into_owned();
    both(&format!(
        "[self-install] signing it here as {}",
        remote.identity
    ));
    run_step(
        spec,
        &remote.codesign,
        &[
            "--force",
            "--options",
            "runtime",
            "--timestamp=none",
            "--entitlements",
            &entitlements_s,
            "--sign",
            &remote.identity,
            &bundle_s,
        ],
        stop,
        short,
        both,
    )
    .await?;
    run_step(
        spec,
        &remote.codesign,
        &["--verify", "--deep", "--strict", &bundle_s],
        stop,
        short,
        both,
    )
    .await?;
    Ok(bundle)
}

/// The bundle a successful `tauri:bundle` leaves.
pub(crate) fn built_bundle(target_dir: &Path) -> PathBuf {
    target_dir.join("release/bundle/macos/Hollow.app")
}

/// Run one step (`sh -c 'exec "$@" 2>&1' sh <program> <args>`, so stderr joins
/// stdout in order) under `process_group::drive`, each line to `on_line`.
async fn run_step(
    spec: &BuildSpec,
    program: &Path,
    args: &[&str],
    stop: &StopSignal,
    deadline: Duration,
    on_line: &mut (dyn FnMut(&str) + Send),
) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new("/bin/sh");
    cmd.arg("-c")
        .arg("exec \"$@\" 2>&1")
        .arg("sh")
        .arg(program)
        .args(args);
    for (key, _) in std::env::vars_os() {
        let k = key.to_string_lossy();
        if k.starts_with("HOLLOW_") || k.starts_with("GIT_") {
            cmd.env_remove(&key);
        }
    }
    cmd.current_dir(&spec.worktree)
        .env("PATH", &spec.path_env)
        .env("CARGO_TARGET_DIR", &spec.target_dir)
        .env("CARGO_BUILD_JOBS", "8")
        .env("CI", "true")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    process_group::isolate(&mut cmd);
    let child = cmd
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", program.display()))?;
    let guard = GroupGuard::adopt(&cmd, &child);
    let sup = Supervision {
        stop: Some(stop),
        limit: None,
        deadline: Some(deadline),
        stderr: StderrPolicy::Silent,
        timings: Timings::default(),
        watch: None,
    };
    let driven = process_group::drive(child, guard, sup, |line| on_line(line)).await;
    let name = format!(
        "{} {}",
        program
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        args.join(" ")
    );
    match driven.ended {
        Ended::Stopped => Err("Stopped".into()),
        Ended::TimedOut => Err(format!(
            "{name} took longer than {} minutes",
            deadline.as_secs() / 60
        )),
        _ => match driven.status {
            Some(Ok(s)) if s.success() => Ok(()),
            Some(Ok(s)) => Err(format!("{name} failed ({s})")),
            Some(Err(e)) => Err(format!("{name}: {e}")),
            None => Err(format!("{name} ended without a status")),
        },
    }
}

/// The build: `xattr -cr src-tauri/models` (when present), `npm ci` (when
/// `node_modules` is missing), then `npm run tauri:bundle`. Every output line
/// goes to the log file and `on_line`.
pub(crate) async fn run_build(
    spec: &BuildSpec,
    stop: &StopSignal,
    on_line: &mut (dyn FnMut(&str) + Send),
) -> Result<PathBuf, String> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&spec.log)
        .map_err(|e| format!("open the log {}: {e}", spec.log.display()))?;
    let mut both = |line: &str| {
        let _ = writeln!(file, "{line}");
        on_line(line);
    };
    let mut started = Instant::now();
    both(&format!(
        "[self-install] building in {}",
        spec.worktree.display()
    ));
    both(&format!(
        "[self-install] CARGO_TARGET_DIR={}",
        spec.target_dir.display()
    ));
    if let Some(remote) = &spec.remote {
        match run_remote_build(spec, remote, stop, &mut both).await {
            Ok(bundle) => {
                both(&format!(
                    "[self-install] built {} on {} and signed it here in {}s",
                    bundle.display(),
                    remote.host,
                    started.elapsed().as_secs()
                ));
                return Ok(bundle);
            }
            Err(e) if e == "Stopped" => return Err(e),
            Err(e) => {
                both(&format!(
                    "[self-install] the remote build didn't work ({e}); building on this Mac instead"
                ));
                // The build here gets a whole deadline of its own.
                started = Instant::now();
            }
        }
    }
    let models = spec.worktree.join("src-tauri/models");
    if models.is_dir() {
        both("[self-install] xattr -cr src-tauri/models");
        run_step(
            spec,
            Path::new("/usr/bin/xattr"),
            &["-cr", "src-tauri/models"],
            stop,
            Duration::from_secs(60),
            &mut both,
        )
        .await?;
    }
    if !spec.worktree.join("node_modules").is_dir() {
        both("[self-install] npm ci (node_modules is missing)");
        run_step(spec, &spec.npm, &["ci"], stop, spec.deadline, &mut both).await?;
        // OC7: record which lock this install is from, for lane worktrees that
        // seed from it. This build itself never seeds: it keeps a clean `npm ci`.
        if let Err(e) = crate::build_workspaces::write_lock_mark(&spec.worktree) {
            both(&format!("[self-install] no lock mark written: {e}"));
        }
    }
    both("[self-install] npm run tauri:bundle");
    let left = spec
        .deadline
        .saturating_sub(started.elapsed())
        .max(Duration::from_secs(1));
    run_step(
        spec,
        &spec.npm,
        &["run", "tauri:bundle"],
        stop,
        left,
        &mut both,
    )
    .await?;
    let bundle = built_bundle(&spec.target_dir);
    if !bundle.join("Contents/MacOS/hollow").is_file() {
        return Err(format!(
            "the build finished but {} has no app",
            bundle.display()
        ));
    }
    both(&format!(
        "[self-install] built {} in {}s",
        bundle.display(),
        started.elapsed().as_secs()
    ));
    Ok(bundle)
}

pub type Emit = Arc<dyn Fn(AppBuildStatus) + Send + Sync>;

fn emit_current(emit: &Emit, id: &str) {
    let snapshot = status_of(id, true, lock().as_ref());
    emit(snapshot);
}

/// `start_build` while the Update button installs a Local build.
pub(crate) const LOCAL_INSTALLING: &str =
    "Hollow is installing a local build and will restart. Build again after it reopens.";

/// Start building the app from workspace `id`'s worktree.
pub async fn start_build(vault: &Path, id: &str, emit: Emit) -> Result<AppBuildStatus, String> {
    let info = crate::build_workspaces::binding_for(vault, id)
        .filter(|b| b.exists)
        .ok_or("This workspace has no worktree")?;
    let worktree = PathBuf::from(&info.worktree);
    if !is_hollow_repo(&worktree) {
        return Err("This worktree is not Hollow's repository".into());
    }
    if crate::update_client::installing_local_build() {
        return Err(LOCAL_INSTALLING.into());
    }
    if let Some(why) = memory_refusal(
        crate::proc_info::free_memory_pct(),
        crate::proc_info::vm_pressure_level(),
    ) {
        return Err(why);
    }
    let home = home()?;
    let path_env = tool_path(std::env::var("PATH").ok().as_deref(), &home);
    let npm = find_npm(&path_env).ok_or(
        "npm was not found (looked in ~/.local/bin, /opt/homebrew/bin, /usr/local/bin and PATH)",
    )?;
    let logs = log_dir()?;
    private_dir(&logs)?;
    let log = logs.join(format!(
        "{}-{}.log",
        crate::build_workspaces::slug(id, 80),
        stamp()
    ));
    let target = target_dir(id)?;
    let commit = crate::build_workspaces::git(
        &worktree,
        &["rev-parse", "--short=12", "HEAD"],
        crate::build_workspaces::QUICK,
    )
    .await
    .unwrap_or_default();
    let branch = crate::build_workspaces::git(
        &worktree,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        crate::build_workspaces::QUICK,
    )
    .await
    .unwrap_or_else(|_| info.branch.clone());
    let stop = Arc::new(StopSignal::new());
    {
        let mut job = lock();
        if let Some(j) = job.as_ref().filter(|j| j.state == BuildState::Building) {
            return Err(format!(
                "An app build is already running (workspace {}); wait for it or stop it",
                j.workspace_id
            ));
        }
        // A new build replaces the finished one, on disk as in memory.
        if let Ok(dir) = state_dir() {
            forget_finished_in(&dir);
        }
        *job = Some(Job {
            workspace_id: id.to_string(),
            state: BuildState::Building,
            worktree: worktree.clone(),
            target_dir: target.clone(),
            log: log.clone(),
            tail: VecDeque::new(),
            started_at: chrono::Utc::now().to_rfc3339(),
            finished_at: None,
            error: None,
            commit,
            branch,
            stop: Arc::clone(&stop),
            receipt: None,
        });
    }
    // A new build replaces the finished one in the Update button too.
    crate::update_client::withdraw_local_build(None);
    // Smart routing 7: App builds: Remote runner (off by default) with a
    // runner set up, a slot tool and a signing identity to sign with here.
    let wanted = crate::agent_settings::get_settings()
        .ok()
        .and_then(|s| crate::agent_settings::app_builds_on_choice(s.app_builds_on.as_deref()))
        .is_some();
    let remote = wanted
        .then(|| {
            let host = crate::remote_session::configured_runner()?;
            let tool = crate::build_slots::ensure_tool().ok()?.join("hollow-slot");
            let (identity, entitlements) = signing_of(&worktree)?;
            Some(RemoteAppBuild {
                slot_tool: tool,
                host,
                codesign: PathBuf::from("/usr/bin/codesign"),
                identity,
                entitlements,
            })
        })
        .flatten();
    if wanted && remote.is_none() {
        eprintln!("[self-install] App builds is Remote runner, but no runner is set up or the worktree has no signing identity; building here");
    }
    let spec = BuildSpec {
        worktree,
        target_dir: target,
        npm,
        path_env,
        log,
        deadline: BUILD_DEADLINE,
        remote,
    };
    let id_owned = id.to_string();
    let vault_owned = vault.to_path_buf();
    let emit_task = Arc::clone(&emit);
    tauri::async_runtime::spawn(async move {
        let mut last_emit = Instant::now();
        let mut on_line = |line: &str| {
            {
                let mut job = lock();
                if let Some(j) = job.as_mut().filter(|j| j.workspace_id == id_owned) {
                    j.tail.push_back(line.chars().take(400).collect());
                    while j.tail.len() > TAIL_LINES {
                        j.tail.pop_front();
                    }
                }
            }
            if last_emit.elapsed() > Duration::from_millis(700) {
                last_emit = Instant::now();
                emit_current(&emit_task, &id_owned);
            }
        };
        let result = match run_build(&spec, &stop, &mut on_line).await {
            // The receipt the install card shows and its approval binds
            // (review A·S4/B·F12); hashing a bundle takes a moment.
            Ok(bundle) => {
                let (ws, tree) = (id_owned.clone(), spec.worktree.clone());
                tauri::async_runtime::spawn_blocking(move || {
                    record_receipt(&ws, &tree, &bundle, &installed_commit())
                })
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r)
                .map_err(|e| {
                    format!("the build finished but its build record could not be saved: {e}")
                })
            }
            Err(e) => Err(e),
        };
        if let Ok(r) = &result {
            if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&spec.log) {
                let _ = writeln!(f, "[self-install] {}", receipt_line(r));
            }
        }
        {
            let mut job = lock();
            if let Some(j) = job.as_mut().filter(|j| j.workspace_id == id_owned) {
                j.finished_at = Some(chrono::Utc::now().to_rfc3339());
                match &result {
                    Ok(receipt) => {
                        j.state = BuildState::Built;
                        j.receipt = Some(receipt.clone());
                    }
                    Err(e) => {
                        j.state = BuildState::Failed;
                        j.error = Some(crate::claude_runner::redact(e));
                    }
                }
                // Kept across a restart (a finished build) or not at all.
                if let Ok(dir) = state_dir() {
                    match j.state {
                        BuildState::Built => {
                            if let Err(e) = save_finished_in(&dir, j) {
                                eprintln!("[self-install] could not save the finished build: {e}");
                            }
                        }
                        _ => forget_finished_in(&dir),
                    }
                }
            }
        }
        if let Err(e) = &result {
            if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&spec.log) {
                let _ = writeln!(f, "[self-install] build failed: {e}");
            }
        }
        emit_current(&emit_task, &id_owned);
        // Row U10: the finished build waits in the top bar's Update button
        // as "Local build of <branch>" (hashing and codesign take a moment).
        if result.is_ok() {
            let _ = tauri::async_runtime::spawn_blocking(move || {
                if let Err(why) =
                    crate::update_client::offer_local_build(&vault_owned, &id_owned, false)
                {
                    eprintln!(
                        "[self-install] the finished build isn't in the Update button: {why}"
                    );
                }
            })
            .await;
        }
    });
    let status = status_of(id, true, lock().as_ref());
    emit(status.clone());
    Ok(status)
}

/// Stop workspace `id`'s running build.
pub fn cancel_build(id: &str) -> Result<(), String> {
    let job = lock();
    match job.as_ref() {
        Some(j) if j.workspace_id == id && j.state == BuildState::Building => {
            j.stop.request();
            Ok(())
        }
        _ => Err("No app build is running for this workspace".into()),
    }
}

// ------------------------------------------------------------ the build receipt

/// The installed everyday app; its embedded provenance names the commit a
/// new build's diff is measured against.
const INSTALLED_PROVENANCE: &str =
    "/Applications/Hollow.app/Contents/Resources/build-provenance.json";

/// The installed app's commit, else the running app's (normally the same app).
fn installed_commit() -> String {
    installed_field(|id| id.commit)
}

/// When the installed app was built (its provenance `generatedAt`), else
/// the running app's: what L2's lesson offers measure a Done issue against.
pub(crate) fn installed_built_at() -> String {
    installed_field(|id| id.generated_at)
}

fn installed_field(field: impl Fn(crate::app_menu::BuildIdentity) -> String) -> String {
    std::fs::read_to_string(INSTALLED_PROVENANCE)
        .ok()
        .map(|text| field(crate::app_menu::parse_identity(&text, "")))
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| field(crate::app_menu::build_identity()))
}

fn hash_file(hash: &mut sha2::Sha256, path: &Path) -> Result<(), String> {
    use sha2::Digest;
    use std::io::Read;
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        if n == 0 {
            return Ok(());
        }
        hash.update(&buf[..n]);
    }
}

/// sha256 over every entry of `bundle`, in name order: its relative path,
/// its kind, the executable bit, and a file's bytes or a link's target. Any
/// change to the app (a swapped binary, an edited resource, a rebuild that
/// produced different bytes) changes it.
pub(crate) fn bundle_digest(bundle: &Path) -> Result<String, String> {
    use sha2::Digest;
    if !bundle.is_dir() {
        return Err(format!("{} is not there", bundle.display()));
    }
    let mut hash = sha2::Sha256::new();
    for entry in walkdir::WalkDir::new(bundle)
        .follow_links(false)
        .sort_by_file_name()
    {
        let entry = entry.map_err(|e| format!("walk {}: {e}", bundle.display()))?;
        let rel = entry
            .path()
            .strip_prefix(bundle)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .to_string();
        let kind = entry.file_type();
        let meta = entry
            .path()
            .symlink_metadata()
            .map_err(|e| format!("stat {}: {e}", entry.path().display()))?;
        #[cfg(unix)]
        let exec = {
            use std::os::unix::fs::PermissionsExt;
            meta.permissions().mode() & 0o111 != 0
        };
        #[cfg(not(unix))]
        let exec = false;
        hash.update(rel.as_bytes());
        hash.update([0]);
        if kind.is_symlink() {
            let target = std::fs::read_link(entry.path())
                .map_err(|e| format!("read link {}: {e}", entry.path().display()))?;
            hash.update(b"l\0");
            hash.update(target.to_string_lossy().as_bytes());
        } else if kind.is_dir() {
            hash.update(b"d\0");
        } else {
            hash.update(if exec { b"x\0" } else { b"f\0" });
            hash.update(meta.len().to_le_bytes());
            hash_file(&mut hash, entry.path())?;
        }
        hash.update([0]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// The receipt of the build in `bundle`: its digest and embedded provenance,
/// and `base_commit` (the installed app's commit) for the diff.
pub(crate) fn record_receipt(
    workspace_id: &str,
    worktree: &Path,
    bundle: &Path,
    base_commit: &str,
) -> Result<BuildReceipt, String> {
    let provenance = bundle.join("Contents/Resources/build-provenance.json");
    let text = std::fs::read_to_string(&provenance)
        .map_err(|e| format!("the built app has no build provenance ({e})"))?;
    let identity = crate::app_menu::parse_identity(&text, "");
    if identity.commit.is_empty() || identity.source_fingerprint.is_empty() {
        return Err("the built app's build provenance names no commit or fingerprint".into());
    }
    Ok(BuildReceipt {
        build_id: format!("{}-{:08x}", stamp(), rand::random::<u32>()),
        workspace_id: workspace_id.to_string(),
        worktree: worktree.to_string_lossy().into(),
        bundle: bundle.to_string_lossy().into(),
        bundle_digest: bundle_digest(bundle)?,
        source_fingerprint: identity.source_fingerprint,
        commit: identity.commit,
        dirty: identity.dirty,
        base_commit: base_commit.trim().to_string(),
        recorded_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// `text`'s first `n` characters.
pub(crate) fn short(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

/// One line for the build log.
fn receipt_line(r: &BuildReceipt) -> String {
    let base = if r.base_commit.is_empty() {
        "unknown"
    } else {
        &r.base_commit
    };
    format!(
        "receipt {}: commit {}{}, bundle {}, source {}, against installed {base}",
        r.build_id,
        r.commit,
        if r.dirty { " (dirty)" } else { "" },
        r.bundle_digest,
        r.source_fingerprint,
    )
}

/// What a build changes against the installed commit, for the install card.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct BuildDiff {
    /// `git diff --shortstat`, for example "3 files changed, 10 insertions(+)".
    pub stat: String,
    /// `<status> <path>` per changed file, every one (the card caps the list).
    pub files: Vec<String>,
    /// Why there is no diff (the installed commit is unknown or not in this
    /// repository).
    pub error: Option<String>,
}

fn git_out(worktree: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = std::process::Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(&key);
        }
    }
    let out = cmd
        .arg("-C")
        .arg(worktree)
        .arg("--no-optional-locks")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(short(err.trim(), 200));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The committed changes from `base` to `commit` in `worktree`.
pub(crate) fn build_diff(worktree: &Path, base: &str, commit: &str) -> BuildDiff {
    if base.trim().is_empty() {
        return BuildDiff {
            error: Some("the installed app's commit is unknown".into()),
            ..BuildDiff::default()
        };
    }
    let diff = |mode: &str| {
        let args = ["diff", mode, "--no-renames", base, commit, "--"];
        git_out(worktree, &args)
    };
    match (diff("--shortstat"), diff("--name-status")) {
        (Ok(stat), Ok(names)) => BuildDiff {
            stat: if stat.trim().is_empty() {
                "no committed changes".into()
            } else {
                stat.trim().to_string()
            },
            files: names
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.replacen('\t', " ", 1))
                .collect(),
            error: None,
        },
        (Err(e), _) | (_, Err(e)) => BuildDiff {
            error: Some(format!(
                "no diff against the installed {} ({e})",
                short(base, 12)
            )),
            ..BuildDiff::default()
        },
    }
}

/// Whether `approved` may be installed: it must still be the current build
/// (`current`, the job's receipt) and its bundle must hash to what was
/// approved. Re-hashes the bundle, so a swapped or rebuilt app is refused.
pub(crate) fn verify_receipt(
    approved: &BuildReceipt,
    current: &BuildReceipt,
) -> Result<(), String> {
    if approved.build_id != current.build_id {
        return Err(format!(
            "A newer build (commit {}, receipt {}) replaced the one that was approved (commit {}, receipt {}); nothing was installed. Approve the new build to install it.",
            short(&current.commit, 12),
            current.build_id,
            short(&approved.commit, 12),
            approved.build_id
        ));
    }
    if approved != current {
        return Err(format!(
            "The approval does not match build {}'s receipt; nothing was installed.",
            current.build_id
        ));
    }
    let now = bundle_digest(Path::new(&approved.bundle)).map_err(|e| {
        format!("The approved build can't be checked ({e}); nothing was installed. Build it again.")
    })?;
    if now != approved.bundle_digest {
        return Err(format!(
            "The built app changed after it was approved (bundle fingerprint {} now, {} approved); nothing was installed. Build it again and approve the new build.",
            short(&now, 16),
            short(&approved.bundle_digest, 16)
        ));
    }
    Ok(())
}

/// A finished build an install may take, read under the job lock: its
/// receipt and branch.
pub(crate) struct Ready {
    pub receipt: BuildReceipt,
    pub branch: String,
}

/// Workspace `id`'s finished build in `slot`, with its receipt.
fn ready_build(slot: &Mutex<Option<Job>>, id: &str) -> Result<Ready, String> {
    let job = slot.lock().unwrap_or_else(|e| e.into_inner());
    let j = job
        .as_ref()
        .filter(|j| j.workspace_id == id && j.state == BuildState::Built)
        .ok_or("Build the app first")?;
    let receipt = j
        .receipt
        .clone()
        .ok_or("Hollow has no record of this build; build it again. Nothing was installed.")?;
    Ok(Ready {
        receipt,
        branch: j.branch.clone(),
    })
}

/// Workspace `id`'s finished build, re-hashed against its own receipt
/// (row U10: what the update client offers as a Local build).
pub(crate) fn finished_build(id: &str) -> Result<Ready, String> {
    let ready = ready_build(&JOB, id)?;
    check_in(&JOB, id, &ready.receipt)
}

/// The workspace whose build finished and waits to install, if any (one
/// build job app-wide).
pub(crate) fn finished_workspace() -> Option<String> {
    lock()
        .as_ref()
        .filter(|j| j.state == BuildState::Built && j.receipt.is_some())
        .map(|j| j.workspace_id.clone())
}

/// Check `approved` against workspace `id`'s current build now (hashing
/// outside the job lock), for the agent's install call before it is offered.
pub(crate) fn check_approved(id: &str, approved: &BuildReceipt) -> Result<Ready, String> {
    check_in(&JOB, id, approved)
}

fn check_in(slot: &Mutex<Option<Job>>, id: &str, approved: &BuildReceipt) -> Result<Ready, String> {
    let ready = ready_build(slot, id)?;
    verify_receipt(approved, &ready.receipt)?;
    Ok(ready)
}

/// Write `result` as `dir`'s `last-result.json` (0600, atomic replace): the
/// update client settles a Local build's install here (row U10).
pub(crate) fn write_result_in(dir: &Path, result: &InstallResult) -> Result<(), String> {
    let text = serde_json::to_string(result).map_err(|e| e.to_string())?;
    private_dir(dir)?;
    let tmp = dir.join("last-result.json.tmp");
    write_private(&tmp, &format!("{text}\n"), 0o600)?;
    std::fs::rename(&tmp, dir.join("last-result.json"))
        .map_err(|e| format!("write {}: {e}", dir.display()))
}

/// The id of `dir`'s `last-result.json`, acknowledged or not.
pub(crate) fn result_id_in(dir: &Path) -> Option<String> {
    parse_result(&std::fs::read_to_string(dir.join("last-result.json")).ok()?).map(|r| r.id)
}

/// Tests that put a build into the app-wide job hold this.
#[cfg(test)]
pub(crate) static TEST_JOB_GUARD: Mutex<()> = Mutex::new(());

/// Put a finished build with `receipt` into the app-wide job (tests).
#[cfg(test)]
pub(crate) fn set_test_job(receipt: Option<BuildReceipt>, workspace_id: &str) {
    *lock() = Some(Job {
        workspace_id: workspace_id.into(),
        state: BuildState::Built,
        worktree: PathBuf::from("/w"),
        target_dir: PathBuf::from("/t"),
        log: PathBuf::from("/l.log"),
        tail: VecDeque::new(),
        started_at: "s".into(),
        finished_at: Some("f".into()),
        error: None,
        commit: "c".into(),
        branch: "b".into(),
        stop: Arc::new(StopSignal::new()),
        receipt,
    });
}

#[cfg(test)]
pub(crate) fn clear_test_job() {
    *lock() = None;
}

// ------------------------------------------------------------ a finished build across a restart

/// The finished build in the self-install folder: the job an install needs,
/// with the receipt its approval binds. A build is in memory only while
/// this app runs, so without it a restart (an update, a crash, a quit)
/// left "no finished build to install" and a rebuild. Written when a build
/// finishes, removed when the next build starts or one fails, and restored
/// at launch unless the installed app is already that build or the built
/// app is gone. Restoring changes nothing an install checks: the approval
/// still binds this receipt and the bundle is re-hashed before the swap.
const SAVED_BUILD: &str = "finished-build.json";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SavedBuild {
    workspace_id: String,
    worktree: PathBuf,
    target_dir: PathBuf,
    log: PathBuf,
    started_at: String,
    finished_at: Option<String>,
    commit: String,
    branch: String,
    receipt: BuildReceipt,
}

/// Save `job`, a finished build with its receipt, to `dir`.
fn save_finished_in(dir: &Path, job: &Job) -> Result<(), String> {
    let receipt = job
        .receipt
        .clone()
        .ok_or("a build without a receipt is not saved")?;
    let saved = SavedBuild {
        workspace_id: job.workspace_id.clone(),
        worktree: job.worktree.clone(),
        target_dir: job.target_dir.clone(),
        log: job.log.clone(),
        started_at: job.started_at.clone(),
        finished_at: job.finished_at.clone(),
        commit: job.commit.clone(),
        branch: job.branch.clone(),
        receipt,
    };
    let text = serde_json::to_string_pretty(&saved).map_err(|e| e.to_string())?;
    private_dir(dir)?;
    let tmp = dir.join(format!("{SAVED_BUILD}.tmp"));
    write_private(&tmp, &format!("{text}\n"), 0o600)?;
    std::fs::rename(&tmp, dir.join(SAVED_BUILD))
        .map_err(|e| format!("write {}: {e}", dir.display()))
}

/// Remove the saved build from `dir`, if there is one.
fn forget_finished_in(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(SAVED_BUILD));
}

/// Put the build saved in `dir` back into an empty `slot` as finished.
/// `installed` is the installed app's (commit, source fingerprint). Returns
/// what happened, for the log; None when there was nothing to do.
fn restore_in(slot: &Mutex<Option<Job>>, dir: &Path, installed: (&str, &str)) -> Option<String> {
    let mut job = slot.lock().unwrap_or_else(|e| e.into_inner());
    if job.is_some() {
        return None;
    }
    let text = std::fs::read_to_string(dir.join(SAVED_BUILD)).ok()?;
    let drop_it = |why: String| {
        forget_finished_in(dir);
        Some(why)
    };
    let saved = match serde_json::from_str::<SavedBuild>(&text) {
        Ok(s) if s.receipt.workspace_id == s.workspace_id => s,
        _ => return drop_it("the saved finished build was unreadable; dropped it".into()),
    };
    let r = &saved.receipt;
    let (installed_commit, installed_fingerprint) = installed;
    if !installed_commit.is_empty()
        && installed_commit == r.commit
        && installed_fingerprint == r.source_fingerprint
    {
        return drop_it(format!(
            "build {} ({}) is already installed; dropped the saved build",
            r.build_id,
            short(&r.commit, 12)
        ));
    }
    if !Path::new(&r.bundle).is_dir() {
        return drop_it(format!(
            "build {}'s app is gone ({}); dropped the saved build",
            r.build_id, r.bundle
        ));
    }
    let note = format!(
        "restored workspace {}'s finished build after a restart: {}",
        saved.workspace_id,
        receipt_line(r)
    );
    *job = Some(Job {
        workspace_id: saved.workspace_id,
        state: BuildState::Built,
        worktree: saved.worktree,
        target_dir: saved.target_dir,
        log: saved.log,
        tail: VecDeque::from([format!("[self-install] {note}")]),
        started_at: saved.started_at,
        finished_at: saved.finished_at,
        error: None,
        commit: saved.commit,
        branch: saved.branch,
        stop: Arc::new(StopSignal::new()),
        receipt: Some(saved.receipt),
    });
    Some(note)
}

/// Setup (GUI only): restore the finished build an earlier run left, so an
/// install approved after a restart still finds it.
pub fn restore_finished_build() {
    let Ok(dir) = state_dir() else { return };
    let identity = std::fs::read_to_string(INSTALLED_PROVENANCE)
        .ok()
        .map(|text| crate::app_menu::parse_identity(&text, ""));
    let (commit, fingerprint) = identity
        .map(|i| (i.commit, i.source_fingerprint))
        .unwrap_or_default();
    if let Some(note) = restore_in(&JOB, &dir, (&commit, &fingerprint)) {
        eprintln!("[self-install] {note}");
    }
}

// ------------------------------------------------------------ install

/// `'…'` for /bin/sh: every character literal, embedded `'` as `'\''`.
pub(crate) fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Why an install can't start now (a run holds the vault, a build
/// workspace turn or an automation is running), or `None`. Hollow app tools
/// wait on it before an install the user allowed from a chat (PR A5).
pub(crate) fn install_blocked(vault: &Path) -> Option<String> {
    install_refusal(vault)
}

fn install_refusal(vault: &Path) -> Option<String> {
    candidate_refusal(crate::instance::current()).or_else(|| busy_refusal(vault))
}

/// A candidate build (merged plan step 0) runs beside the everyday app; its
/// installer would replace `/Applications/Hollow.app` and quit the wrong app.
pub(crate) fn candidate_refusal(instance: &crate::instance::Instance) -> Option<String> {
    instance.is_candidate().then(|| {
        format!(
            "This is a candidate build ({}). Install from the everyday Hollow, not from a candidate.",
            instance.identifier
        )
    })
}

fn busy_refusal(vault: &Path) -> Option<String> {
    if let Some(holder) = crate::agent_sessions::busy_reason(vault) {
        return Some(format!(
            "Wait for {holder} to finish: installing quits Hollow"
        ));
    }
    // A build workspace's turn holds no vault lease (build programme PR B3).
    if crate::automated_history::scoped_run_live(vault) {
        return Some(
            "Wait for the running build workspace turns to finish: installing quits Hollow".into(),
        );
    }
    match crate::run_queue::live_rows(vault) {
        Ok(rows)
            if rows
                .iter()
                .any(|r| r.status == crate::run_queue::RowStatus::Running) =>
        {
            Some("An automation is running; install when it finishes".into())
        }
        _ => None,
    }
}

fn write_private(path: &Path, text: &str, mode: u32) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    let mut f = options
        .open(path)
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    f.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    }
    Ok(())
}

// ------------------------------------------------------------ the result

/// What the helper recorded (`last-result.json`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstallResult {
    pub id: String,
    pub ok: bool,
    #[serde(default)]
    pub started_at: String,
    pub finished_at: String,
    #[serde(default)]
    pub commit: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub log: String,
    #[serde(default)]
    pub message: String,
    /// The helper started the installer (`false`: Hollow never quit or the
    /// worktree was gone, so nothing was touched). `None` from older helpers.
    #[serde(default)]
    pub installer_ran: Option<bool>,
    /// The installer's own report (`--report`), when the helper is new enough
    /// to ask for one (review B·F02).
    #[serde(default)]
    pub installer: Option<InstallerReport>,
    /// What is installed, as far as the evidence goes; derived on read.
    #[serde(default)]
    pub state: InstallState,
    /// The build receipt the install was approved for (review A·S4/B·F12);
    /// on a refusal, the one that was refused. `None` from older helpers.
    #[serde(default)]
    pub build: Option<BuildReceipt>,
}

/// What an install left, as far as the evidence shows (merged plan slice 6,
/// review B·F02). `Failed` means the previous app is verifiably still
/// installed; when nobody can say, it is `Unknown`, never "kept".
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InstallState {
    Installed,
    Failed,
    #[default]
    Unknown,
}

/// What `scripts/install-macos-app.mjs --report` wrote: whether the install
/// went through and why not, then (`--confirm-running`) whether the app
/// running after the reopen is the installed one.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct InstallerReport {
    pub ok: bool,
    /// Why the install failed (for example Hollow was opened before the swap).
    pub reason: String,
    pub installed_commit: String,
    /// `None` until the reopen check ran.
    pub running_matches: Option<bool>,
    /// The check quit a pre-swap app and reopened it once.
    pub relaunched: bool,
    /// Why the running app is not the installed one.
    pub running_reason: String,
    /// What is on disk after the installer: "yes" (the new app verified in
    /// place), "no" (nothing moved, or every rename was undone), "unknown"
    /// (putting the apps back failed too). Empty from older installers.
    pub installed: String,
    /// The install id, which names the installer's journal.
    pub install_id: String,
    pub journal: Option<String>,
    /// The journal steps reached: prepared, promoted, verified, healthy…
    pub steps: Vec<String>,
}

/// The install's state from the helper's result and the installer's report.
fn state_of(result: &InstallResult) -> InstallState {
    match &result.installer {
        // `ok` from an installer before the journal also came only after it
        // verified the installed app.
        Some(r) if r.installed == "yes" || (r.ok && r.installed.is_empty()) => {
            InstallState::Installed
        }
        Some(r) if r.installed == "no" && !result.ok => InstallState::Failed,
        Some(_) => InstallState::Unknown,
        // The installer never ran: nothing was touched.
        None if !result.ok && result.installer_ran == Some(false) => InstallState::Failed,
        None => InstallState::Unknown,
    }
}

/// Parse `last-result.json`; the message carries the installer's reason
/// when it gave one, so the notice and the receipt say why.
fn parse_result(text: &str) -> Option<InstallResult> {
    let mut result: InstallResult = serde_json::from_str(text.trim()).ok()?;
    if let Some(report) = &result.installer {
        let why = if !result.ok {
            report.reason.trim()
        } else if report.running_matches == Some(false) {
            report.running_reason.trim()
        } else {
            ""
        };
        if !why.is_empty() {
            result.message = why.to_string();
        }
    }
    result.state = state_of(&result);
    Some(result)
}

/// The result in `dir` that the user has not acknowledged yet.
pub(crate) fn unacknowledged_in(dir: &Path) -> Option<InstallResult> {
    let result = parse_result(&std::fs::read_to_string(dir.join("last-result.json")).ok()?)?;
    let acked = std::fs::read_to_string(dir.join("acknowledged")).unwrap_or_default();
    (acked.trim() != result.id).then_some(result)
}

pub(crate) fn acknowledge_in(dir: &Path, id: &str) -> Result<(), String> {
    private_dir(dir)?;
    write_private(&dir.join("acknowledged"), id, 0o600)
}

/// Setup (GUI only). Nothing to load any more: `pending_result` reads the
/// file on each call (slice 6). Kept so setup's call site still says where
/// the install notice comes from.
pub fn load_result_at_startup() {}

/// The unacknowledged install result, read from `last-result.json` now. It
/// used to be a snapshot taken at launch, so a result written after the app
/// started (the helper's rewrite after the reopen check, or an install that
/// gave up while this app still ran) read as null.
pub fn pending_result() -> Option<InstallResult> {
    unacknowledged_in(&state_dir().ok()?)
}

/// The user saw the notice: don't show it again.
pub fn acknowledge_result() -> Result<(), String> {
    let dir = state_dir()?;
    match unacknowledged_in(&dir) {
        Some(result) => acknowledge_in(&dir, &result.id),
        None => Ok(()),
    }
}

/// The persistent install receipt and what it does and doesn't prove
/// (next-level P00/P20, 2026-10-01). `self_install_result` returns a result
/// only until its notice is dismissed, so after a dismissal it answers null
/// even though `last-result.json` still records the install. Agents read this
/// instead: the receipt, whether it was acknowledged, and whether its commit is
/// the running app's embedded provenance.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstallReceipt {
    pub result: InstallResult,
    /// The user dismissed its notice (`acknowledged` holds this id).
    pub acknowledged: bool,
    /// The running app's commit, from its embedded `build-provenance.json`.
    pub running_commit: String,
    pub running_dirty: bool,
    /// The receipt's commit is a prefix of (or equal to) the running commit.
    pub matches_running: bool,
    /// Why a successful install is not what is running: the helper's check
    /// after reopening (review B·F02), or the running app's commit.
    pub mismatch_reason: Option<String>,
    /// `installed` only when the running app reports the receipt's commit;
    /// `failed` only when the installer said the previous app is back in
    /// place; otherwise `unknown` (merged plan slice 6, review B·F02).
    pub state: InstallState,
    /// One plain sentence: what this receipt establishes.
    pub summary: String,
}

/// [`read_receipt_in`] with an unreadable receipt as `None` too.
pub(crate) fn receipt_in(
    dir: &Path,
    running: &crate::app_menu::BuildIdentity,
) -> Option<InstallReceipt> {
    read_receipt_in(dir, running).ok().flatten()
}

/// The receipt in `dir`: `Ok(None)` when no install was ever recorded, an
/// error when `last-result.json` exists but can't be read or parsed, so a
/// broken receipt never reads as "no install recorded" (P1-46).
pub(crate) fn read_receipt_in(
    dir: &Path,
    running: &crate::app_menu::BuildIdentity,
) -> Result<Option<InstallReceipt>, String> {
    let text = match std::fs::read_to_string(dir.join("last-result.json")) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("The install record couldn't be read: {e}")),
    };
    let result = parse_result(&text).ok_or(
        "The install record couldn't be read: last-result.json is not a valid install result",
    )?;
    Ok(Some(receipt_from(dir, result, running)))
}

fn receipt_from(
    dir: &Path,
    result: InstallResult,
    running: &crate::app_menu::BuildIdentity,
) -> InstallReceipt {
    let acked = std::fs::read_to_string(dir.join("acknowledged")).unwrap_or_default();
    let acknowledged = acked.trim() == result.id;
    let commit = result.commit.trim();
    let matches_running =
        !commit.is_empty() && !running.commit.is_empty() && running.commit.starts_with(commit);
    let short: String = commit.chars().take(8).collect();
    let helper_reason = result
        .installer
        .as_ref()
        .filter(|r| r.running_matches == Some(false))
        .map(|r| r.running_reason.trim().to_string())
        .filter(|r| !r.is_empty());
    let mismatch_reason = (result.ok && !matches_running).then(|| {
        helper_reason.clone().unwrap_or_else(|| {
            format!(
                "The running app reports {}, not {short}.",
                running.short_commit
            )
        })
    });
    let swapped = result.ok || result.state == InstallState::Installed;
    let state = match result.state {
        InstallState::Failed => InstallState::Failed,
        _ if swapped && matches_running => InstallState::Installed,
        _ => InstallState::Unknown,
    };
    let message = if result.message.is_empty() {
        String::new()
    } else {
        format!(" {}", result.message)
    };
    let summary = match state {
        InstallState::Installed => format!(
            "Installed {short} at {}; the running app reports the same commit{}.{}",
            result.finished_at,
            if running.dirty { " (built with uncommitted changes)" } else { "" },
            if result.ok { String::new() } else { message.clone() }
        ),
        InstallState::Failed => format!("The install of {short} failed at {}; the previous app was kept.{message}", result.finished_at),
        InstallState::Unknown if swapped => format!(
            "The installer reported success for {short} at {}, but the running app is {}.{} Restart Hollow or check /Applications/Hollow.app before treating it as installed.",
            result.finished_at,
            running.short_commit,
            helper_reason.map(|r| format!(" After reopening: {r}")).unwrap_or_default()
        ),
        InstallState::Unknown => format!(
            "The install of {short} failed at {}, and which app is installed now is unknown: the installer did not confirm the previous app was put back.{message} Check /Applications/Hollow.app before relying on it.",
            result.finished_at
        ),
    };
    InstallReceipt {
        result,
        acknowledged,
        running_commit: running.commit.clone(),
        running_dirty: running.dirty,
        matches_running,
        mismatch_reason,
        state,
        summary,
    }
}

/// The last install's persistent receipt, acknowledged or not.
pub fn last_receipt() -> Option<InstallReceipt> {
    receipt_in(&state_dir().ok()?, &crate::app_menu::build_identity())
}

/// [`last_receipt`], with an unreadable receipt as an error (P1-46).
pub fn read_last_receipt() -> Result<Option<InstallReceipt>, String> {
    let dir = state_dir().map_err(|e| format!("The install record couldn't be read: {e}"))?;
    read_receipt_in(&dir, &crate::app_menu::build_identity())
}

/// [`receipt_in`] when the build it installed (or refused) came from
/// workspace `id` (a workspace's Activity, Canvas C5b · Q3). Hollow keeps
/// only the last install's `last-result.json`, and a result from a helper
/// older than build receipts names no workspace, so it is left out.
pub(crate) fn receipt_for_workspace_in(
    dir: &Path,
    running: &crate::app_menu::BuildIdentity,
    id: &str,
) -> Option<InstallReceipt> {
    receipt_in(dir, running).filter(|r| {
        r.result
            .build
            .as_ref()
            .is_some_and(|b| b.workspace_id == id)
    })
}

/// [`receipt_for_workspace_in`] for this app's install state.
pub(crate) fn last_receipt_for_workspace(id: &str) -> Option<InstallReceipt> {
    receipt_for_workspace_in(&state_dir().ok()?, &crate::app_menu::build_identity(), id)
}

/// The app build workspace `id` started in this app run, when there is
/// one: builds live in memory only, so a build from before the last
/// restart is gone (its install, if any, is [`last_receipt_for_workspace`]).
pub(crate) fn build_job_for_workspace(id: &str) -> Option<AppBuildStatus> {
    let job = lock();
    job.as_ref()
        .filter(|j| j.workspace_id == id)
        .map(|j| status_of(id, true, Some(j)))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Night ST7 (2026-10-14): build caches are removed only when they are real
    // folders directly in the caches root; never a link, never elsewhere.
    #[test]
    fn a_build_cache_is_removed_only_as_a_real_folder_inside_its_root() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("Caches");
        let keep = base.path().join("elsewhere");
        std::fs::create_dir_all(root.join("hollow-install-target-w1/release")).unwrap();
        std::fs::write(root.join("hollow-install-target-w1/release/app"), "x").unwrap();
        std::fs::create_dir_all(&keep).unwrap();
        std::fs::write(keep.join("precious"), "y").unwrap();
        std::os::unix::fs::symlink(&keep, root.join("hollow-install-target-link")).unwrap();

        assert!(!remove_cache_dir(
            &root.join("hollow-install-target-link"),
            &root
        ));
        assert!(
            keep.join("precious").exists(),
            "a link's target is never touched"
        );
        assert!(!remove_cache_dir(
            &root.join("hollow-install-target-w1/release"),
            &root
        ));
        assert!(!remove_cache_dir(&keep, &root));
        assert!(keep.exists());
        assert!(!remove_cache_dir(&root.join("missing"), &root));
        assert!(remove_cache_dir(
            &root.join("hollow-install-target-w1"),
            &root
        ));
        assert!(!root.join("hollow-install-target-w1").exists());
    }

    #[test]
    fn a_removed_workspaces_target_goes_only_when_nothing_keeps_it() {
        let dir = Path::new("/c/hollow-install-target-w1");
        let prunable = vec!["/c/hollow-install-target-w1".to_string()];
        assert!(may_forget(dir, false, &[], &prunable));
        // Its build job (building, built or installing) keeps it.
        assert!(!may_forget(dir, true, &[], &prunable));
        // A build that holds it keeps it.
        assert!(!may_forget(dir, false, &[dir.to_path_buf()], &prunable));
        // The prune keeps it (it built the installed app or the rollback copy).
        assert!(!may_forget(
            dir,
            false,
            &[],
            &["/c/hollow-install-target-w2".to_string()]
        ));
    }

    #[test]
    fn receipt_survives_acknowledgement_and_compares_with_the_running_app() {
        let tmp = tempfile::Builder::new()
            .prefix("self-install-")
            .tempdir()
            .unwrap();
        let running = crate::app_menu::parse_identity(
            r#"{"commit":"63c79a889a214c368b1f52b057d20c4e2f1ad519","dirty":false,"version":"0.5.3"}"#,
            "0.0.0",
        );
        assert!(receipt_in(tmp.path(), &running).is_none());
        std::fs::write(tmp.path().join("last-result.json"), r#"{"id":"20261001-144445-47e5bd3a","ok":true,"startedAt":"s","finishedAt":"2026-10-01T18:44:47Z","commit":"63c79a889a21","branch":"release/x","log":"/l","message":""}"#).unwrap();
        let fresh = receipt_in(tmp.path(), &running).unwrap();
        assert!(!fresh.acknowledged && fresh.matches_running, "{fresh:?}");
        assert!(fresh.summary.starts_with("Installed 63c79a88 at 2026-10-01T18:44:47Z; the running app reports the same commit"), "{}", fresh.summary);
        // The audit's case: the notice was dismissed, so the old read is null…
        std::fs::write(tmp.path().join("acknowledged"), "20261001-144445-47e5bd3a").unwrap();
        assert!(unacknowledged_in(tmp.path()).is_none());
        // …but the receipt is still there, and says so.
        let acked = receipt_in(tmp.path(), &running).unwrap();
        assert!(acked.acknowledged && acked.matches_running && acked.result.ok);
        // A different running commit never confirms the install.
        let other = crate::app_menu::parse_identity(
            r#"{"commit":"7a325ac2576d3f595d84e652c02b4580d7edb07a"}"#,
            "0.0.0",
        );
        let stale = receipt_in(tmp.path(), &other).unwrap();
        assert!(!stale.matches_running);
        assert!(
            stale.summary.contains("but the running app is 7a325ac"),
            "{}",
            stale.summary
        );
        // A failed install says the old app was kept only when the installer
        // said so (slice 6); this one did not, so it is unknown.
        std::fs::write(tmp.path().join("last-result.json"), r#"{"id":"b","ok":false,"finishedAt":"t","commit":"63c79a88","message":"signature check failed"}"#).unwrap();
        let failed = receipt_in(tmp.path(), &running).unwrap();
        assert_eq!(failed.state, InstallState::Unknown);
        assert!(
            failed.summary.starts_with(
                "The install of 63c79a88 failed at t, and which app is installed now is unknown"
            ),
            "{}",
            failed.summary
        );
        std::fs::write(tmp.path().join("last-result.json"), r#"{"id":"c","ok":false,"finishedAt":"t","commit":"63c79a88","installer":{"ok":false,"reason":"signature check failed","installed":"no"}}"#).unwrap();
        let kept = receipt_in(tmp.path(), &running).unwrap();
        assert_eq!(kept.state, InstallState::Failed);
        assert!(kept.summary.starts_with("The install of 63c79a88 failed at t; the previous app was kept. signature check failed"), "{}", kept.summary);
    }

    /// P1-46 (XDIALOG-03): a receipt that exists but can't be read is an
    /// error, never "no install recorded".
    #[test]
    fn an_unreadable_receipt_is_an_error_and_a_missing_one_is_none() {
        let tmp = tempfile::Builder::new()
            .prefix("self-install-")
            .tempdir()
            .unwrap();
        let running = crate::app_menu::parse_identity(r#"{"commit":"63c79a88"}"#, "0.0.0");
        assert_eq!(read_receipt_in(tmp.path(), &running), Ok(None));
        std::fs::write(tmp.path().join("last-result.json"), "{\"id\": trunc").unwrap();
        let why = read_receipt_in(tmp.path(), &running).unwrap_err();
        assert!(why.contains("couldn't be read"), "{why}");
        assert!(receipt_in(tmp.path(), &running).is_none());
        std::fs::remove_file(tmp.path().join("last-result.json")).unwrap();
        std::fs::create_dir(tmp.path().join("last-result.json")).unwrap();
        assert!(read_receipt_in(tmp.path(), &running).is_err());
    }

    #[test]
    fn a_candidate_build_never_installs_over_the_everyday_app() {
        let candidate = crate::instance::Instance::from_identifier("com.hollow.app.candidate");
        let why = candidate_refusal(&candidate).expect("a candidate is refused");
        assert!(why.contains("com.hollow.app.candidate"), "{why}");
        assert!(candidate_refusal(&crate::instance::Instance::everyday()).is_none());
    }

    #[test]
    fn hollow_repo_detection_reads_the_tauri_identifier() {
        let tmp = tempfile::Builder::new()
            .prefix("self-install-")
            .tempdir()
            .unwrap();
        assert!(!is_hollow_repo(tmp.path()));
        std::fs::create_dir_all(tmp.path().join("src-tauri")).unwrap();
        std::fs::write(
            tmp.path().join("src-tauri/tauri.conf.json"),
            r#"{"identifier":"com.other.app"}"#,
        )
        .unwrap();
        assert!(!is_hollow_repo(tmp.path()));
        std::fs::write(
            tmp.path().join("src-tauri/tauri.conf.json"),
            r#"{"productName":"Hollow","identifier":"com.hollow.app"}"#,
        )
        .unwrap();
        assert!(is_hollow_repo(tmp.path()));
    }

    #[test]
    fn memory_gate() {
        assert!(memory_refusal(Some(80), Some(1)).is_none());
        assert!(memory_refusal(None, None).is_none());
        assert!(memory_refusal(Some(29), Some(1)).unwrap().contains("29%"));
        assert!(memory_refusal(Some(90), Some(2))
            .unwrap()
            .contains("warning"));
        assert!(memory_refusal(Some(90), Some(4))
            .unwrap()
            .contains("critical"));
    }

    #[test]
    fn tool_path_puts_user_tool_folders_first_once() {
        let p = tool_path(
            Some("/usr/bin:/opt/homebrew/bin:/custom"),
            Path::new("/Users/me"),
        );
        assert!(p.starts_with("/Users/me/.cargo/bin:/Users/me/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/custom:"), "{p}");
        assert_eq!(
            p.split(':').filter(|d| *d == "/usr/bin").count(),
            1,
            "deduplicated: {p}"
        );
        assert!(p.ends_with(":/bin:/usr/sbin:/sbin"));
    }

    #[test]
    fn shell_quoting_survives_spaces_and_apostrophes() {
        assert_eq!(sh_quote("plain"), "'plain'");
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
        let tricky = "/Users/dev/Documents/Documents - Alex’s MacBook Pro/it's $HOME `x`";
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("printf '%s' {}", sh_quote(tricky)))
            .output()
            .unwrap();
        assert_eq!(String::from_utf8(out.stdout).unwrap(), tricky);
    }

    fn write_exec(path: &Path, body: &str) {
        std::fs::write(path, body).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A temp folder whose name has a space and a curly apostrophe, like the
    /// user's Documents folder.
    fn tricky_tmp() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::Builder::new()
            .prefix("self-install-")
            .tempdir()
            .unwrap();
        let dir = tmp.path().join("Alex’s Mac 'repo'");
        std::fs::create_dir_all(&dir).unwrap();
        (tmp, dir)
    }

    #[tokio::test]
    async fn the_build_job_logs_every_line_and_finds_the_bundle() {
        let (_tmp, dir) = tricky_tmp();
        let tree = dir.join("worktree");
        std::fs::create_dir_all(tree.join("node_modules")).unwrap();
        let npm = dir.join("npm");
        write_exec(&npm, "#!/bin/sh\necho \"npm $*\"\n[ \"$CI\" = true ] || exit 42\necho 'to stderr' >&2\nmkdir -p \"$CARGO_TARGET_DIR/release/bundle/macos/Hollow.app/Contents/MacOS\"\ntouch \"$CARGO_TARGET_DIR/release/bundle/macos/Hollow.app/Contents/MacOS/hollow\"\n");
        let spec = BuildSpec {
            worktree: tree.clone(),
            target_dir: dir.join("target"),
            npm: npm.clone(),
            path_env: tool_path(Some("/usr/bin:/bin"), &dir),
            log: dir.join("build.log"),
            deadline: Duration::from_secs(30),
            remote: None,
        };
        let mut lines = Vec::new();
        let stop = StopSignal::new();
        let bundle = run_build(&spec, &stop, &mut |l: &str| lines.push(l.to_string()))
            .await
            .unwrap();
        assert_eq!(bundle, built_bundle(&dir.join("target")));
        assert!(
            lines.iter().any(|l| l == "npm run tauri:bundle"),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l == "to stderr"),
            "stderr joins the log: {lines:?}"
        );
        assert!(
            !lines.iter().any(|l| l == "npm ci"),
            "node_modules exists: no npm ci"
        );
        let log = std::fs::read_to_string(dir.join("build.log")).unwrap();
        assert!(log.contains("npm run tauri:bundle") && log.contains("to stderr"));

        // Without node_modules it runs `npm ci` first; a failing step fails the build.
        std::fs::remove_dir_all(tree.join("node_modules")).unwrap();
        write_exec(
            &npm,
            "#!/bin/sh\necho \"npm $*\"\n[ \"$1\" = ci ] && exit 0\nexit 2\n",
        );
        let mut lines = Vec::new();
        let err = run_build(&spec, &stop, &mut |l: &str| lines.push(l.to_string()))
            .await
            .unwrap_err();
        assert!(err.contains("failed"), "{err}");
        assert!(lines.iter().any(|l| l == "npm ci"), "{lines:?}");

        // A stop ends the step.
        write_exec(&npm, "#!/bin/sh\nexec /bin/sleep 30\n");
        let stop = Arc::new(StopSignal::new());
        let stopper = Arc::clone(&stop);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            stopper.request();
        });
        let started = Instant::now();
        std::fs::create_dir_all(tree.join("node_modules")).unwrap();
        let err = run_build(&spec, &stop, &mut |_l: &str| {})
            .await
            .unwrap_err();
        assert_eq!(err, "Stopped");
        assert!(started.elapsed() < Duration::from_secs(15));
    }

    /// Smart routing 7: App builds on the remote runner. A stand-in
    /// `hollow-slot --on <host>` runs the real script as the host would
    /// (`HOLLOW_REMOTE_HEAD` set, its own `CARGO_TARGET_DIR`), a stand-in npm
    /// "builds" the bundle there and records what the script gave it; the
    /// zip comes back, is unpacked where the install step looks, signed here
    /// with the identity and entitlements and verified, and the fetch folder
    /// goes. A runner that fails, a signing that fails, or hollow-slot's own
    /// fallback to this Mac all build here instead.
    #[tokio::test]
    async fn an_app_build_on_the_runner_comes_back_signed_here_or_builds_here() {
        let (_tmp, dir) = tricky_tmp();
        let tree = dir.join("worktree");
        std::fs::create_dir_all(tree.join("node_modules")).unwrap();
        let runner_target = dir.join("runner-target");
        let slot = dir.join("hollow-slot");
        let slot_as_host = |head: &str| {
            format!(
                "#!/bin/sh\necho \"slot $1 $2 $3 $4\"\n[ \"$1\" = --on ] && [ \"$3\" = --fetch ] || exit 9\nshift 4\nexport CARGO_TARGET_DIR={} HOLLOW_REMOTE_HEAD={head} PATH={}:\"$PATH\"\nexec \"$@\"\n",
                sh_quote(&runner_target.to_string_lossy()),
                sh_quote(&dir.to_string_lossy())
            )
        };
        write_exec(
            &slot,
            &slot_as_host("0e3a0efab5016201d62f12f9d80764e737c64658"),
        );
        // The log sits beside it: the test folder's name has quotes in it.
        let codesign = dir.join("codesign");
        write_exec(
            &codesign,
            "#!/bin/sh\necho \"$*\" >> \"$(dirname \"$0\")/signed.txt\"\n",
        );
        let signed = dir.join("signed.txt");
        let npm = dir.join("npm");
        write_exec(
            &npm,
            "#!/bin/sh\necho \"npm $* identity=$APPLE_SIGNING_IDENTITY ci=$CI\"\nprintf '%s' \"$HOLLOW_PROVENANCE_CHECKOUT\" > \"$(dirname \"$0\")/checkout.txt\"\nmkdir -p \"$CARGO_TARGET_DIR/release/bundle/macos/Hollow.app/Contents/MacOS\"\ntouch \"$CARGO_TARGET_DIR/release/bundle/macos/Hollow.app/Contents/MacOS/hollow\"\n",
        );
        let entitlements = tree.join("src-tauri/Entitlements.plist");
        let spec = BuildSpec {
            worktree: tree.clone(),
            target_dir: dir.join("target"),
            npm: npm.clone(),
            path_env: tool_path(Some("/usr/bin:/bin"), &dir),
            log: dir.join("build.log"),
            deadline: Duration::from_secs(60),
            remote: Some(RemoteAppBuild {
                slot_tool: slot.clone(),
                host: "testhost".into(),
                codesign: codesign.clone(),
                identity: "Test Identity".into(),
                entitlements: entitlements.clone(),
            }),
        };
        let stop = StopSignal::new();
        let mut lines = Vec::new();
        let bundle = run_build(&spec, &stop, &mut |l: &str| lines.push(l.to_string()))
            .await
            .unwrap();
        assert_eq!(bundle, built_bundle(&dir.join("target")));
        assert!(bundle.join("Contents/MacOS/hollow").is_file());
        assert!(
            lines
                .iter()
                .any(|l| l.contains("App builds: Remote runner: compiling on testhost")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l == "slot --on testhost --fetch src-tauri/target/hollow-remote-app"),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l == "npm run tauri:bundle identity=- ci=true"),
            "ad hoc there: {lines:?}"
        );
        assert!(
            runner_target
                .join("release/bundle/macos/Hollow.app")
                .is_dir(),
            "built in the runner's own folder"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("checkout.txt")).unwrap(),
            tree.to_string_lossy(),
            "the provenance names this Mac's checkout"
        );
        let calls = std::fs::read_to_string(&signed).unwrap();
        let b = bundle.display().to_string();
        assert!(calls.contains(&format!("--force --options runtime --timestamp=none --entitlements {} --sign Test Identity {b}", entitlements.display())), "{calls}");
        assert!(
            calls.contains(&format!("--verify --deep --strict {b}")),
            "{calls}"
        );
        assert!(!tree.join(REMOTE_APP_OUT).exists(), "the fetch folder goes");
        assert!(!bundle
            .parent()
            .unwrap()
            .join(".hollow-remote-unpack")
            .exists());

        // Each way it can fail builds here, says so, and leaves no fetch folder.
        let local_build = |lines: &[String]| {
            lines
                .iter()
                .any(|l| l == "[self-install] npm run tauri:bundle")
        };
        for (case, slot_body, sign_body) in [
            (
                "the runner fails",
                "#!/bin/sh\necho 'the runner said no'\nexit 1\n".to_string(),
                None,
            ),
            ("hollow-slot ran it here", slot_as_host(""), None),
            (
                "signing fails",
                slot_as_host("0e3a0efab5016201d62f12f9d80764e737c64658"),
                Some("#!/bin/sh\nexit 1\n"),
            ),
        ] {
            write_exec(&slot, &slot_body);
            if let Some(body) = sign_body {
                write_exec(&codesign, body);
            }
            std::fs::remove_dir_all(dir.join("target")).unwrap();
            let mut lines = Vec::new();
            let bundle = run_build(&spec, &stop, &mut |l: &str| lines.push(l.to_string()))
                .await
                .unwrap();
            assert!(bundle.join("Contents/MacOS/hollow").is_file(), "{case}");
            assert!(
                lines
                    .iter()
                    .any(|l| l.contains("the remote build didn't work")
                        && l.contains("building on this Mac instead")),
                "{case}: {lines:?}"
            );
            assert!(local_build(&lines), "{case}: {lines:?}");
            assert!(
                !tree.join(REMOTE_APP_OUT).exists(),
                "{case}: the fetch folder goes"
            );
        }
    }

    /// The identity and entitlements come from the worktree's
    /// tauri.conf.json; none (or ad hoc) means no remote build.
    #[test]
    fn signing_comes_from_the_worktrees_tauri_conf() {
        let (_tmp, dir) = tricky_tmp();
        let conf = dir.join("src-tauri");
        std::fs::create_dir_all(&conf).unwrap();
        assert_eq!(signing_of(&dir), None);
        std::fs::write(conf.join("tauri.conf.json"), r#"{"bundle":{"macOS":{"signingIdentity":"Hollow Dev Signing","entitlements":"Entitlements.plist"}}}"#).unwrap();
        assert_eq!(
            signing_of(&dir),
            Some((
                "Hollow Dev Signing".to_string(),
                dir.join("src-tauri/Entitlements.plist")
            ))
        );
        std::fs::write(
            conf.join("tauri.conf.json"),
            r#"{"bundle":{"macOS":{"signingIdentity":"-","entitlements":"Entitlements.plist"}}}"#,
        )
        .unwrap();
        assert_eq!(signing_of(&dir), None);
        // The real one has both.
        let here = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        assert!(signing_of(here).is_some());
    }

    #[test]
    fn results_from_older_helpers_still_parse_without_an_installer_report() {
        let tmp = tempfile::Builder::new()
            .prefix("self-install-")
            .tempdir()
            .unwrap();
        std::fs::write(
            tmp.path().join("last-result.json"),
            r#"{"id":"a","ok":true,"finishedAt":"t","commit":"abc123","message":""}"#,
        )
        .unwrap();
        let result = unacknowledged_in(tmp.path()).unwrap();
        assert!(result.ok && result.installer.is_none() && result.message.is_empty());
        let other = crate::app_menu::parse_identity(r#"{"commit":"7a325ac2576d"}"#, "0.0.0");
        assert_eq!(
            receipt_in(tmp.path(), &other)
                .unwrap()
                .mismatch_reason
                .as_deref(),
            Some("The running app reports 7a325ac, not abc123.")
        );
    }

    // ---- the build receipt (merged plan slice 6, review A·S4/B·F12)

    /// A built `Hollow.app` under `target` whose provenance names `commit`.
    fn fake_bundle(target: &Path, commit: &str) -> PathBuf {
        let bundle = built_bundle(target);
        std::fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        std::fs::create_dir_all(bundle.join("Contents/Resources")).unwrap();
        write_exec(
            &bundle.join("Contents/MacOS/hollow"),
            "#!/bin/sh\necho built\n",
        );
        std::fs::write(bundle.join("Contents/Info.plist"), "<plist/>").unwrap();
        std::fs::write(
            bundle.join("Contents/Resources/build-provenance.json"),
            format!(
                r#"{{"commit":"{commit}","dirty":false,"sourceFingerprint":"{}","version":"0.5.3"}}"#,
                "f".repeat(64)
            ),
        )
        .unwrap();
        bundle
    }

    fn built_job(receipt: BuildReceipt, target: &Path) -> Job {
        Job {
            workspace_id: "ws".into(),
            state: BuildState::Built,
            worktree: PathBuf::from("/w"),
            target_dir: target.to_path_buf(),
            log: PathBuf::from("/l.log"),
            tail: VecDeque::new(),
            started_at: "s".into(),
            finished_at: Some("f".into()),
            error: None,
            commit: short(&receipt.commit, 12),
            branch: "feat/x".into(),
            stop: Arc::new(StopSignal::new()),
            receipt: Some(receipt),
        }
    }

    const COMMIT_A: &str = "abc123def4567890abc123def4567890abc12345";

    // ---- a finished build across a restart

    fn saved_slot(dir: &Path, receipt: &BuildReceipt, target: &Path) -> Mutex<Option<Job>> {
        save_finished_in(dir, &built_job(receipt.clone(), target)).unwrap();
        Mutex::new(None)
    }

    #[test]
    fn a_finished_build_survives_a_restart_and_its_approval_still_checks() {
        let (_tmp, dir) = tricky_tmp();
        let target = dir.join("target");
        let bundle = fake_bundle(&target, COMMIT_B);
        let receipt = record_receipt("ws", &dir.join("worktree"), &bundle, COMMIT_A).unwrap();
        let state = dir.join("state");
        let slot = saved_slot(&state, &receipt, &target);
        // A new app run: nothing in memory until the saved build is restored.
        assert!(ready_build(&slot, "ws").is_err());
        let note = restore_in(&slot, &state, (COMMIT_A, &"a".repeat(64))).expect("restored");
        assert!(note.contains("restored"), "{note}");
        let status = status_of("ws", true, slot.lock().unwrap().as_ref());
        assert_eq!(status.state, BuildState::Built);
        assert_eq!(status.receipt.as_ref(), Some(&receipt));
        assert_eq!(status.branch.as_deref(), Some("feat/x"));
        // The install card's approval binds the same receipt, re-hashed.
        let ready = check_in(&slot, "ws", &receipt).expect("installable after a restart");
        assert_eq!(ready.receipt, receipt);
        // Another workspace still has nothing to install.
        assert!(ready_build(&slot, "other").is_err());
    }

    #[test]
    fn a_restored_build_that_changed_on_disk_is_still_refused() {
        let (_tmp, dir) = tricky_tmp();
        let target = dir.join("target");
        let bundle = fake_bundle(&target, COMMIT_B);
        let receipt = record_receipt("ws", &dir.join("worktree"), &bundle, COMMIT_A).unwrap();
        let state = dir.join("state");
        let slot = saved_slot(&state, &receipt, &target);
        std::fs::write(bundle.join("Contents/Info.plist"), "<plist>swapped</plist>").unwrap();
        restore_in(&slot, &state, (COMMIT_A, "")).expect("restored");
        let refused = check_in(&slot, "ws", &receipt).err().expect("refused");
        assert!(
            refused.contains("changed after it was approved"),
            "{refused}"
        );
    }

    #[test]
    fn an_installed_gone_or_unreadable_saved_build_is_dropped() {
        let (_tmp, dir) = tricky_tmp();
        let target = dir.join("target");
        let bundle = fake_bundle(&target, COMMIT_B);
        let receipt = record_receipt("ws", &dir.join("worktree"), &bundle, COMMIT_A).unwrap();
        let state = dir.join("state");
        let file = state.join(SAVED_BUILD);

        // Already installed (the install that followed it succeeded): dropped.
        let slot = saved_slot(&state, &receipt, &target);
        let note = restore_in(
            &slot,
            &state,
            (&receipt.commit, &receipt.source_fingerprint),
        )
        .expect("a note");
        assert!(note.contains("already installed"), "{note}");
        assert!(slot.lock().unwrap().is_none());
        assert!(!file.exists());

        // The built app was deleted: dropped.
        let slot = saved_slot(&state, &receipt, &target);
        std::fs::rename(&bundle, dir.join("moved.app")).unwrap();
        let note = restore_in(&slot, &state, (COMMIT_A, "")).expect("a note");
        assert!(note.contains("gone"), "{note}");
        assert!(slot.lock().unwrap().is_none());
        assert!(!file.exists());

        // Unreadable: dropped.
        std::fs::write(&file, "{not json").unwrap();
        let slot = Mutex::new(None);
        let note = restore_in(&slot, &state, (COMMIT_A, "")).expect("a note");
        assert!(note.contains("unreadable"), "{note}");
        assert!(slot.lock().unwrap().is_none());
        assert!(!file.exists());

        // Nothing saved: nothing to say.
        assert_eq!(restore_in(&Mutex::new(None), &state, (COMMIT_A, "")), None);
    }

    #[test]
    fn a_job_in_memory_wins_and_forgetting_removes_the_saved_build() {
        let (_tmp, dir) = tricky_tmp();
        let target = dir.join("target");
        let bundle = fake_bundle(&target, COMMIT_B);
        let receipt = record_receipt("ws", &dir.join("worktree"), &bundle, COMMIT_A).unwrap();
        let state = dir.join("state");
        let slot = saved_slot(&state, &receipt, &target);
        let mut running = built_job(receipt.clone(), &target);
        running.state = BuildState::Building;
        *slot.lock().unwrap() = Some(running);
        assert_eq!(restore_in(&slot, &state, (COMMIT_A, "")), None);
        assert_eq!(
            slot.lock().unwrap().as_ref().unwrap().state,
            BuildState::Building
        );
        forget_finished_in(&state);
        assert!(!state.join(SAVED_BUILD).exists());
        forget_finished_in(&state); // already gone: fine
    }

    #[test]
    fn the_saved_build_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let (_tmp, dir) = tricky_tmp();
        let target = dir.join("target");
        let bundle = fake_bundle(&target, COMMIT_B);
        let receipt = record_receipt("ws", &dir.join("worktree"), &bundle, COMMIT_A).unwrap();
        let state = dir.join("state");
        let _slot = saved_slot(&state, &receipt, &target);
        let mode = std::fs::metadata(state.join(SAVED_BUILD))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    const COMMIT_B: &str = "bbb123def4567890abc123def4567890abc12345";

    #[test]
    fn a_swapped_or_rebuilt_bundle_after_approval_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let (_tmp, dir) = tricky_tmp();
        let bundle = fake_bundle(&dir.join("target"), COMMIT_A);
        let tree = dir.join("worktree");
        let approved = record_receipt("ws", &tree, &bundle, "0011223344").unwrap();
        assert_eq!(
            (approved.commit.as_str(), approved.base_commit.as_str()),
            (COMMIT_A, "0011223344")
        );
        assert_eq!(approved.source_fingerprint, "f".repeat(64));
        assert_eq!(approved.bundle_digest.len(), 64);
        verify_receipt(&approved, &approved).expect("the approved bytes install");

        // The binary swapped after the approval.
        let exe = bundle.join("Contents/MacOS/hollow");
        write_exec(&exe, "#!/bin/sh\necho swapped\n");
        let why = verify_receipt(&approved, &approved).unwrap_err();
        assert!(why.contains("changed after it was approved"), "{why}");
        assert!(why.contains("nothing was installed"), "{why}");
        write_exec(&exe, "#!/bin/sh\necho built\n");
        verify_receipt(&approved, &approved).expect("the same bytes again");

        // A file added, or the executable bit dropped, is a change too.
        let extra = bundle.join("Contents/Resources/extra.dylib");
        std::fs::write(&extra, "x").unwrap();
        assert!(verify_receipt(&approved, &approved).is_err());
        std::fs::remove_file(&extra).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(verify_receipt(&approved, &approved).is_err());
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        verify_receipt(&approved, &approved).unwrap();

        // A rebuild is a new receipt, even with the same bytes: an approval
        // names one build.
        let rebuilt = record_receipt("ws", &tree, &bundle, "0011223344").unwrap();
        assert_ne!(rebuilt.build_id, approved.build_id);
        assert_eq!(rebuilt.bundle_digest, approved.bundle_digest);
        let why = verify_receipt(&approved, &rebuilt).unwrap_err();
        assert!(why.contains("A newer build"), "{why}");
        // An approval that names the current build with other contents.
        let forged = BuildReceipt {
            bundle_digest: "0".repeat(64),
            ..rebuilt.clone()
        };
        let why = verify_receipt(&forged, &rebuilt).unwrap_err();
        assert!(why.contains("does not match"), "{why}");
        // The bundle gone.
        std::fs::remove_dir_all(&bundle).unwrap();
        let why = verify_receipt(&approved, &approved).unwrap_err();
        assert!(why.contains("can't be checked"), "{why}");
        // No provenance, no receipt.
        let bare = dir.join("bare.app");
        std::fs::create_dir_all(&bare).unwrap();
        assert!(record_receipt("ws", &tree, &bare, "").is_err());
    }

    #[test]
    fn a_deferred_install_takes_the_approved_build_not_the_current_one() {
        let (_tmp, dir) = tricky_tmp();
        let target = dir.join("target");
        let tree = dir.join("worktree");
        let bundle = fake_bundle(&target, COMMIT_A);
        let approved = record_receipt("ws", &tree, &bundle, "0011223344").unwrap();
        let slot = Mutex::new(Some(built_job(approved.clone(), &target)));
        let ready = check_in(&slot, "ws", &approved).expect("the approved build is current");
        assert_eq!(ready.receipt, approved);
        assert_eq!(ready.branch, "feat/x");

        // While the turn ran, a newer build of the same workspace finished
        // into the same folder: the approval never carries over to it.
        fake_bundle(&target, COMMIT_B);
        let newer = record_receipt("ws", &tree, &bundle, "0011223344").unwrap();
        *slot.lock().unwrap() = Some(built_job(newer.clone(), &target));
        let why = check_in(&slot, "ws", &approved)
            .err()
            .expect("the approved build was replaced");
        assert!(
            why.contains("A newer build") && why.contains(&newer.build_id),
            "{why}"
        );
        assert_eq!(check_in(&slot, "ws", &newer).unwrap().receipt, newer);
        // A rebuild still running: nothing to install.
        slot.lock().unwrap().as_mut().unwrap().state = BuildState::Building;
        assert_eq!(
            check_in(&slot, "ws", &newer).err().as_deref(),
            Some("Build the app first")
        );

        // A local build's result written by the update client (row U10)
        // is what the receipt reads, and its id is found again.
        let result = InstallResult {
            id: "20261115-010203-0badf00d".into(),
            ok: false,
            started_at: "t".into(),
            finished_at: "t".into(),
            commit: short(COMMIT_A, 12),
            branch: "feat/x".into(),
            log: String::new(),
            message: why.clone(),
            installer_ran: Some(false),
            installer: None,
            state: InstallState::Failed,
            build: Some(approved.clone()),
        };
        write_result_in(&dir, &result).unwrap();
        assert_eq!(result_id_in(&dir).as_deref(), Some(result.id.as_str()));
        let running = crate::app_menu::parse_identity(r#"{"commit":"0011223344"}"#, "0.0.0");
        let receipt = receipt_in(&dir, &running).unwrap();
        assert_eq!(receipt.state, InstallState::Failed);
        assert_eq!(receipt.result.build.as_ref(), Some(&approved));
        assert_eq!(unacknowledged_in(&dir).unwrap().message, why);
    }

    #[test]
    fn job_status_belongs_to_one_workspace() {
        let job = Job {
            workspace_id: "a".into(),
            state: BuildState::Building,
            worktree: PathBuf::from("/w"),
            target_dir: PathBuf::from("/t"),
            log: PathBuf::from("/l.log"),
            tail: VecDeque::from(vec![
                "line with sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789".to_string(),
            ]),
            started_at: "s".into(),
            finished_at: None,
            error: None,
            commit: "c".into(),
            branch: "b".into(),
            stop: Arc::new(StopSignal::new()),
            receipt: None,
        };
        let mine = status_of("a", true, Some(&job));
        assert_eq!(mine.state, BuildState::Building);
        assert!(!mine.log_tail.contains("sk-ant-api03-abcdef"), "redacted");
        let other = status_of("b", true, Some(&job));
        assert_eq!(other.state, BuildState::Idle);
        assert_eq!(other.other_job.as_deref(), Some("a"));
    }
}
