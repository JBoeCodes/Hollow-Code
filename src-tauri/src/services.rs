//! Services that outlive a chat turn (Hollow build programme, PR B6): the GUI
//! half of `hollow_services`.
//!
//! A run's finish sweep ends every process the agent started (see
//! `process_group`), so an agent's `npm run dev &` dies with its turn. A
//! service is started by the GUI instead, on the agent's request through the
//! `hollow services-mcp` subprocess (which only forwards), and belongs to the
//! build worktree (its **scope**), not to the turn:
//!
//! - **Bounded unless the chat is in Bypass** (merged plan decision 1,
//!   2026-10-02; review A·S2, B·F11). [`bounded`]: in every mode but
//!   `bypassPermissions` the service records its chat (`owner`) and stops
//!   when that chat is archived ([`stop_chat`]), as well as on an explicit
//!   stop, when the worktree is removed, and when Hollow quits. Its folder
//!   must resolve (canonicalized, symlinks followed) inside the worktree in
//!   every mode ([`parse_start`]). Its command and folder go on the starting
//!   turn's evidence receipt ([`take_started_by_run`]). Only bypass starts an
//!   unrestricted service: one that outlives its chat (the pre-decision
//!   behaviour), with no card.
//!
//! - It is spawned here, as `/bin/sh -c "exec 2>&1\n<command>"` in its own
//!   process group, with the GUI's environment minus every `HOLLOW_*`
//!   variable (the requesting run's `HOLLOW_RUN_TAG` above all), `TMPDIR`
//!   reset to the system's (not the turn's scratch folder, which is deleted
//!   at turn end), and a PATH with the usual tool folders. It is not a
//!   descendant of the agent and carries its own run tag, so no run's sweep
//!   can match it. `process_group::drive` supervises it (sampling slowly),
//!   and a stop is its standard settle: SIGTERM to the group, 3 s, SIGKILL.
//! - Output goes to `~/Library/Logs/Hollow/services/<scope hash>/<name>.log`
//!   (rotated at 4 MiB).
//! - At most 4 services, and one `dev-server`, machine-wide (this Mac allows
//!   one dev server at a time).
//! - Running services are recorded in `services.json` (app data folder) with
//!   pid and process start time. A worktree's services stop when the worktree
//!   is removed; all stop when Hollow quits. At the next launch, a recorded
//!   service still alive with the same start time (Hollow died without
//!   cleaning up) is stopped; a pid whose start time differs is never
//!   signalled.
//!
//! The broker listens on `~/Library/Caches/hollow/services-<pid>.sock` and
//! checks a proof (`sha256(secret ‖ scope ‖ run id ‖ chat ‖ mode)`, the
//! secret in this process's memory only), that the run is still live
//! (`automated_history::run_is_live`: the grant expires when the turn ends),
//! and that the scope is a live build worktree of this vault.

use crate::evidence::ReceiptService;
use crate::process_group::{self, GroupGuard, StderrPolicy, StopSignal, Supervision, Timings};
use crate::services_mcp::Identity;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

pub const EVENT: &str = "services-changed";
const MAX_SERVICES: usize = 4;
const MAX_DEV_SERVERS: usize = 1;
const NAME_MAX: usize = 40;
const COMMAND_MAX: usize = 4000;
const LOG_ROTATE_BYTES: u64 = 4 * 1024 * 1024;
const LOG_LINES_DEFAULT: usize = 80;
const LOG_LINES_MAX: usize = 400;
const STOP_WAIT: Duration = Duration::from_secs(20);
/// Receipt rows kept for turns that haven't finished yet (a turn takes its
/// own at its finish; this only caps a leak from a turn that never does).
const STARTED_KEEP: usize = 64;

/// Whether a service started from a run in canonical permission mode `perm`
/// is bounded to its chat (merged plan decision 1): every mode but bypass.
pub(crate) fn bounded(perm: &str) -> bool {
    perm != "bypassPermissions"
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    DevServer,
    Other,
}

/// What `list_services` and the UI see.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServiceView {
    pub scope: String,
    pub name: String,
    pub command: String,
    pub cwd: String,
    pub port: Option<u16>,
    pub kind: Kind,
    /// `running` or `exited`.
    pub status: String,
    pub exit_code: Option<i32>,
    pub pid: u32,
    pub started_at: String,
    pub log: String,
}

/// One persisted record (running services only).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Record {
    pub scope: String,
    pub name: String,
    pub command: String,
    pub cwd: String,
    pub port: Option<u16>,
    pub kind: Kind,
    pub pid: u32,
    pub start_micros: Option<u64>,
    pub log: String,
    pub started_at: String,
    /// The chat session key whose archiving stops this service (decision 1);
    /// `None` for a bypass (unrestricted) service and records written before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// The history run id of the turn that started it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub run_id: String,
}

struct Entry {
    record: Record,
    running: bool,
    exit_code: Option<i32>,
    stop: Arc<StopSignal>,
    /// Flips to true when supervision returned (the process group is gone).
    done: tokio::sync::watch::Receiver<bool>,
}

/// The services of one Hollow process. Production uses [`registry`]; tests
/// build their own with temporary folders.
pub(crate) struct Registry {
    store: Option<PathBuf>,
    log_root: PathBuf,
    entries: Mutex<Vec<Entry>>,
    /// Called after every change (the GUI emits [`EVENT`]).
    on_change: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// (run id, receipt row) for each service a still-unfinished turn
    /// started, oldest first; the turn takes its rows at its finish.
    started: Mutex<Vec<(String, ReceiptService)>>,
}

static REGISTRY: OnceLock<Arc<Registry>> = OnceLock::new();

/// The process-wide registry (GUI only).
pub(crate) fn registry() -> Arc<Registry> {
    Arc::clone(REGISTRY.get_or_init(|| {
        let store = crate::agent_settings::app_data_dir()
            .ok()
            .map(|d| d.join("services.json"));
        let log_root = dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("Library/Logs")
            .join(crate::instance::current().support_name())
            .join("services");
        Arc::new(Registry::new(store, log_root))
    }))
}

/// Canonical scope string of `cwd`, or a readable refusal.
pub(crate) fn scope_of(cwd: &Path) -> Result<String, String> {
    cwd.canonicalize()
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|_| format!("{} does not exist", cwd.display()))
}

fn scope_hash(scope: &str) -> String {
    let digest = Sha256::digest(scope.as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// Names: 1-40 of `a-z A-Z 0-9 - _`.
pub(crate) fn valid_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty()
        || n.len() > NAME_MAX
        || !n
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "name must be 1-{NAME_MAX} letters, digits, - or _ (got {n:?})"
        ));
    }
    Ok(n.to_string())
}

/// The system temp folder (`confstr(_CS_DARWIN_USER_TEMP_DIR)` on macOS), not
/// a turn's scratch folder.
fn system_tmpdir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        let mut buf = vec![0u8; 1024];
        // SAFETY: confstr writes at most `buf.len()` bytes into `buf`.
        let n = unsafe {
            libc::confstr(
                libc::_CS_DARWIN_USER_TEMP_DIR,
                buf.as_mut_ptr() as *mut libc::c_char,
                buf.len(),
            )
        };
        if n > 1 && n <= buf.len() {
            buf.truncate(n - 1);
            if let Ok(s) = String::from_utf8(buf) {
                return PathBuf::from(s);
            }
        }
    }
    PathBuf::from("/tmp/")
}

/// The service's environment: `env` minus every `HOLLOW_*` variable, with
/// `TMPDIR` reset and PATH extended with the usual tool folders.
pub(crate) fn service_env(
    env: impl Iterator<Item = (OsString, OsString)>,
    home: &Path,
) -> Vec<(OsString, OsString)> {
    let mut out: Vec<(OsString, OsString)> = Vec::new();
    let mut inherited_path = None;
    for (k, v) in env {
        let key = k.to_string_lossy();
        if key.starts_with("HOLLOW_") || key == "TMPDIR" || key == "TEMP" || key == "TMP" {
            continue;
        }
        if key == "PATH" {
            inherited_path = Some(v.to_string_lossy().into_owned());
            continue;
        }
        out.push((k, v));
    }
    out.push((
        "PATH".into(),
        crate::self_install::tool_path(inherited_path.as_deref(), home).into(),
    ));
    out.push(("TMPDIR".into(), system_tmpdir().into_os_string()));
    out
}

/// A start request, validated.
#[derive(Clone, Debug)]
pub(crate) struct StartSpec {
    pub scope: String,
    pub name: String,
    pub command: String,
    pub cwd: PathBuf,
    pub port: Option<u16>,
    pub kind: Kind,
    /// The history run id of the requesting turn (empty in unit tests that
    /// start one directly).
    pub run_id: String,
    /// The chat whose archiving stops it, or `None` for an unrestricted
    /// (bypass) service. [`dispatch`] fills both from the proven identity.
    pub owner: Option<String>,
}

/// Validate `start_service` arguments for `scope`. The folder must resolve
/// inside the worktree: canonicalized, so `..` and a symlink are judged by
/// where they lead, in every mode.
pub(crate) fn parse_start(scope: &str, args: &Value) -> Result<StartSpec, String> {
    let name = valid_name(args.get("name").and_then(Value::as_str).unwrap_or(""))?;
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if command.is_empty() || command.len() > COMMAND_MAX || command.contains('\0') {
        return Err(format!("command must be 1-{COMMAND_MAX} characters"));
    }
    let root = PathBuf::from(scope);
    let cwd = match args
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        None => root.clone(),
        Some(c) => {
            let p = if Path::new(c).is_absolute() {
                PathBuf::from(c)
            } else {
                root.join(c)
            };
            let canonical = p
                .canonicalize()
                .map_err(|_| format!("cwd {c} does not exist"))?;
            if !canonical.starts_with(&root) {
                return Err(format!(
                    "cwd {c} resolves to {}, outside this worktree; a service runs only in a folder inside the worktree. Nothing was started.",
                    canonical.display()
                ));
            }
            if !canonical.is_dir() {
                return Err(format!("cwd {c} must be a folder inside the worktree"));
            }
            canonical
        }
    };
    let port = match args.get("port") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_u64()
                .filter(|p| (1..=65535).contains(p))
                .ok_or("port must be 1-65535")? as u16,
        ),
    };
    let kind = match args.get("kind").and_then(Value::as_str).unwrap_or("other") {
        "dev-server" => Kind::DevServer,
        "other" | "" => Kind::Other,
        other => return Err(format!("kind must be dev-server or other (got {other})")),
    };
    Ok(StartSpec {
        scope: scope.to_string(),
        name,
        command,
        cwd,
        port,
        kind,
        run_id: String::new(),
        owner: None,
    })
}

/// Append each line to `log`, rotating it to `<log>.1` past 4 MiB.
struct LogWriter {
    path: PathBuf,
    file: Option<std::fs::File>,
    written: u64,
}

impl LogWriter {
    fn open(path: &Path) -> Result<Self, String> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| format!("open {}: {e}", path.display()))?;
        let written = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(Self {
            path: path.to_path_buf(),
            file: Some(file),
            written,
        })
    }
    fn line(&mut self, line: &str) {
        if self.written > LOG_ROTATE_BYTES {
            let _ = std::fs::rename(&self.path, self.path.with_extension("log.1"));
            self.file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
                .ok();
            self.written = 0;
        }
        if let Some(f) = self.file.as_mut() {
            if writeln!(f, "{line}").is_ok() {
                self.written += line.len() as u64 + 1;
            }
        }
    }
}

impl Registry {
    pub(crate) fn new(store: Option<PathBuf>, log_root: PathBuf) -> Self {
        Self {
            store,
            log_root,
            entries: Mutex::new(Vec::new()),
            on_change: Mutex::new(None),
            started: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn set_on_change(&self, f: Arc<dyn Fn() + Send + Sync>) {
        *self.on_change.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }

    fn changed(&self) {
        self.persist();
        let f = self
            .on_change
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(f) = f {
            f();
        }
    }

    fn persist(&self) {
        let Some(store) = &self.store else { return };
        let records: Vec<Record> = self
            .lock()
            .iter()
            .filter(|e| e.running)
            .map(|e| e.record.clone())
            .collect();
        let text =
            serde_json::to_string_pretty(&json!({ "schemaVersion": 1, "services": records }))
                .unwrap_or_default();
        if let Err(e) = crate::vault::atomic_write_private(store, text.as_bytes()) {
            eprintln!("[services] could not record services: {e}");
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn view(e: &Entry) -> ServiceView {
        ServiceView {
            scope: e.record.scope.clone(),
            name: e.record.name.clone(),
            command: e.record.command.clone(),
            cwd: e.record.cwd.clone(),
            port: e.record.port,
            kind: e.record.kind,
            status: if e.running {
                "running".into()
            } else {
                "exited".into()
            },
            exit_code: e.exit_code,
            pid: e.record.pid,
            started_at: e.record.started_at.clone(),
            log: e.record.log.clone(),
        }
    }

    /// Services, all or one scope's, oldest first.
    pub(crate) fn list(&self, scope: Option<&str>) -> Vec<ServiceView> {
        self.lock()
            .iter()
            .filter(|e| scope.is_none_or(|s| e.record.scope == s))
            .map(Self::view)
            .collect()
    }

    /// Start a service. `env` is the GUI's environment (tests pass their own).
    #[cfg(test)]
    pub(crate) async fn start(
        self: &Arc<Self>,
        spec: StartSpec,
        env: impl Iterator<Item = (OsString, OsString)>,
    ) -> Result<ServiceView, String> {
        self.start_checked(spec, env, &crate::shared_mcp::request_current)
            .await
    }

    /// Start a service because the person asked (a project script's Run or chord): there is no
    /// agent run whose end could cancel it, and no owner chat, so it lasts until Stop, its exit
    /// or Hollow quitting.
    pub(crate) async fn start_for_user(
        self: &Arc<Self>,
        spec: StartSpec,
        env: impl Iterator<Item = (OsString, OsString)>,
    ) -> Result<ServiceView, String> {
        self.start_checked(spec, env, &|| true).await
    }

    async fn start_checked(
        self: &Arc<Self>,
        spec: StartSpec,
        env: impl Iterator<Item = (OsString, OsString)>,
        valid: &(dyn Fn() -> bool + Sync),
    ) -> Result<ServiceView, String> {
        {
            let mut entries = self.lock();
            if !valid() {
                return Err(RUN_ENDED.into());
            }
            entries.retain(|e| {
                e.running || !(e.record.scope == spec.scope && e.record.name == spec.name)
            });
            if entries
                .iter()
                .any(|e| e.record.scope == spec.scope && e.record.name == spec.name)
            {
                return Err(format!(
                    "A service named {} is already running here; stop it first",
                    spec.name
                ));
            }
            let running: Vec<&Entry> = entries.iter().filter(|e| e.running).collect();
            if running.len() >= MAX_SERVICES {
                return Err(format!(
                    "{MAX_SERVICES} services are already running; stop one first"
                ));
            }
            if spec.kind == Kind::DevServer {
                if let Some(other) = running
                    .iter()
                    .filter(|e| e.record.kind == Kind::DevServer)
                    .nth(MAX_DEV_SERVERS - 1)
                {
                    return Err(format!(
                        "A dev server is already running ({} in {}); this Mac runs one at a time. Stop it first",
                        other.record.name, other.record.scope
                    ));
                }
            }
        }
        let dir = self.log_root.join(scope_hash(&spec.scope));
        std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for d in [&self.log_root, &dir] {
                let _ = std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700));
            }
        }
        let log_path = dir.join(format!("{}.log", spec.name));
        let mut log = LogWriter::open(&log_path)?;
        log.line(&format!(
            "[hollow] {} starting: {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            spec.command
        ));

        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let mut cmd = tokio::process::Command::new("/bin/sh");
        cmd.arg("-c").arg(format!("exec 2>&1\n{}", spec.command));
        cmd.env_clear()
            .envs(service_env(env, &home))
            .current_dir(&spec.cwd);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // Its own group and its own tag: never the requesting run's.
        process_group::isolate(&mut cmd);
        if !valid() {
            return Err(RUN_ENDED.into());
        }
        let child = cmd
            .spawn()
            .map_err(|e| format!("could not start {}: {e}", spec.name))?;
        let pid = child.id().unwrap_or(0);
        // Not in the run ledger: services.json (pid + start time) already
        // ends what a previous Hollow left running (`clean_up_stale`).
        let guard = GroupGuard::adopt_unrecorded(&cmd, &child);
        let record = Record {
            scope: spec.scope.clone(),
            name: spec.name.clone(),
            command: spec.command.clone(),
            cwd: spec.cwd.to_string_lossy().into(),
            port: spec.port,
            kind: spec.kind,
            pid,
            start_micros: crate::proc_info::process_start_micros(pid as i32),
            log: log_path.to_string_lossy().into(),
            started_at: chrono::Utc::now().to_rfc3339(),
            owner: spec.owner.clone(),
            run_id: spec.run_id.clone(),
        };
        if !spec.run_id.is_empty() {
            let mut started = self.started.lock().unwrap_or_else(|e| e.into_inner());
            started.push((
                spec.run_id.clone(),
                ReceiptService {
                    name: record.name.clone(),
                    command: record.command.clone(),
                    cwd: record.cwd.clone(),
                    port: record.port,
                    kind: match record.kind {
                        Kind::DevServer => "dev-server".into(),
                        Kind::Other => "other".into(),
                    },
                    ends_with_chat: record.owner.is_some(),
                },
            ));
            while started.len() > STARTED_KEEP {
                started.remove(0);
            }
        }
        let stop = Arc::new(StopSignal::new());
        let (done_tx, done_rx) = tokio::sync::watch::channel(false);
        self.lock().push(Entry {
            record,
            running: true,
            exit_code: None,
            stop: Arc::clone(&stop),
            done: done_rx,
        });
        self.changed();

        let me = Arc::clone(self);
        let (scope, name) = (spec.scope.clone(), spec.name.clone());
        tokio::spawn(async move {
            let sup = Supervision {
                stop: Some(&stop),
                limit: None,
                deadline: None,
                stderr: StderrPolicy::Silent,
                timings: Timings {
                    // Long-lived: sample descendants rarely, and never end a
                    // service just because it closed its stdout.
                    sample_every: Duration::from_secs(15),
                    post_eof_exit: Duration::from_secs(366 * 24 * 60 * 60),
                    ..Timings::default()
                },
                watch: None,
            };
            let driven = process_group::drive(child, guard, sup, |line| log.line(line)).await;
            let code = driven.status.and_then(|s| s.ok()).and_then(|s| s.code());
            log.line(&format!(
                "[hollow] {} {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                match driven.ended {
                    process_group::Ended::Stopped => "stopped".to_string(),
                    _ => format!(
                        "exited ({})",
                        code.map(|c| c.to_string())
                            .unwrap_or_else(|| "signal".into())
                    ),
                }
            ));
            {
                let mut entries = me.lock();
                if let Some(e) = entries.iter_mut().find(|e| {
                    e.running
                        && e.record.scope == scope
                        && e.record.name == name
                        && e.record.pid == pid
                }) {
                    e.running = false;
                    e.exit_code = code;
                }
            }
            let _ = done_tx.send(true);
            me.changed();
        });
        let view = self
            .lock()
            .iter()
            .find(|e| e.record.scope == spec.scope && e.record.name == spec.name)
            .map(Self::view);
        view.ok_or_else(|| format!("{} ended at once", spec.name))
    }

    /// Stop `name` in `scope` (SIGTERM its group, 3 s, SIGKILL) and wait for
    /// it; an exited one is forgotten.
    pub(crate) async fn stop(&self, scope: &str, name: &str) -> Result<(), String> {
        self.stop_checked(scope, name, &crate::shared_mcp::request_current)
            .await
    }

    async fn stop_checked(
        &self,
        scope: &str,
        name: &str,
        valid: &(dyn Fn() -> bool + Sync),
    ) -> Result<(), String> {
        let (stop, mut done) = {
            let mut entries = self.lock();
            if !valid() {
                return Err(RUN_ENDED.into());
            }
            let Some(i) = entries
                .iter()
                .position(|e| e.record.scope == scope && e.record.name == name)
            else {
                return Err(format!("No service named {name} here"));
            };
            if !entries[i].running {
                entries.remove(i);
                drop(entries);
                self.changed();
                return Ok(());
            }
            (Arc::clone(&entries[i].stop), entries[i].done.clone())
        };
        if !valid() {
            return Err(RUN_ENDED.into());
        }
        stop.request();
        let waited = tokio::time::timeout(STOP_WAIT, done.wait_for(|d| *d)).await;
        self.lock()
            .retain(|e| !(e.record.scope == scope && e.record.name == name && !e.running));
        self.changed();
        match waited {
            Ok(_) => Ok(()),
            Err(_) => Err(format!(
                "{name} did not stop within {}s",
                STOP_WAIT.as_secs()
            )),
        }
    }

    /// Stop every service of `scope` (its worktree is being removed).
    pub(crate) async fn stop_scope(&self, scope: &str) {
        let names: Vec<String> = self
            .lock()
            .iter()
            .filter(|e| e.record.scope == scope)
            .map(|e| e.record.name.clone())
            .collect();
        for name in names {
            if let Err(e) = self.stop(scope, &name).await {
                eprintln!("[services] {e}");
            }
        }
    }

    /// Stop every service `chat` owns (that chat was archived), in any scope.
    /// Unrestricted (bypass) services have no owner and are left alone.
    pub(crate) async fn stop_owner(&self, chat: &str) {
        let owned: Vec<(String, String)> = self
            .lock()
            .iter()
            .filter(|e| e.record.owner.as_deref() == Some(chat))
            .map(|e| (e.record.scope.clone(), e.record.name.clone()))
            .collect();
        for (scope, name) in owned {
            if let Err(e) = self.stop(&scope, &name).await {
                eprintln!("[services] {e}");
            }
        }
    }

    /// The receipt rows for the services `run_id` started, removed from the
    /// registry (the turn's finish puts them on its receipt).
    pub(crate) fn take_started(&self, run_id: &str) -> Vec<ReceiptService> {
        let mut started = self.started.lock().unwrap_or_else(|e| e.into_inner());
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *started)
            .into_iter()
            .partition(|(id, _)| id == run_id);
        *started = rest;
        mine.into_iter().map(|(_, row)| row).collect()
    }

    /// Hollow is quitting: end every running group now, synchronously (the
    /// async runtime may already be going away).
    pub(crate) fn stop_all_blocking(&self) {
        let pids: Vec<u32> = self
            .lock()
            .iter()
            .filter(|e| e.running)
            .map(|e| e.record.pid)
            .collect();
        for pid in &pids {
            end_group_blocking(*pid, None);
        }
        for e in self.lock().iter_mut() {
            e.running = false;
        }
        self.persist();
    }

    /// The newest `lines` lines of a service's log, redacted.
    pub(crate) fn logs(
        &self,
        scope: &str,
        name: &str,
        lines: Option<usize>,
    ) -> Result<String, String> {
        let log = self
            .lock()
            .iter()
            .find(|e| e.record.scope == scope && e.record.name == name)
            .map(|e| e.record.log.clone())
            .ok_or_else(|| format!("No service named {name} here"))?;
        let n = lines.unwrap_or(LOG_LINES_DEFAULT).clamp(1, LOG_LINES_MAX);
        let text = read_tail(Path::new(&log), 256 * 1024);
        let tail: Vec<&str> = text.lines().collect();
        let start = tail.len().saturating_sub(n);
        Ok(crate::claude_runner::redact(&tail[start..].join("\n")))
    }

    /// Startup (GUI only): stop services a previous Hollow left running.
    /// Returns how many were signalled.
    pub(crate) fn clean_up_stale(&self) -> usize {
        let Some(store) = &self.store else { return 0 };
        let Ok(text) = std::fs::read_to_string(store) else {
            return 0;
        };
        let records: Vec<Record> = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.get("services").cloned())
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let mut signalled = 0;
        for r in &records {
            if end_group_blocking(r.pid, Some(r.start_micros)) {
                signalled += 1;
                eprintln!(
                    "[services] stopped {} ({}) left running by a previous Hollow",
                    r.name, r.scope
                );
            }
        }
        let _ = std::fs::remove_file(store);
        signalled
    }
}

/// End the process group led by `pid`: SIGTERM, then SIGKILL after 3 s.
/// With `expect_start = Some(start)`, only when the pid is alive, still leads
/// its group, and started exactly then (never a reused pid). Returns whether
/// it signalled.
fn end_group_blocking(pid: u32, expect_start: Option<Option<u64>>) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    if pid <= 1 {
        return false;
    }
    if crate::proc_info::pid_alive(pid) != Some(true) {
        return false;
    }
    if let Some(expected) = expect_start {
        match (expected, crate::proc_info::process_start_micros(pid)) {
            (Some(want), Some(have)) if want == have => {}
            _ => return false,
        }
    }
    #[cfg(unix)]
    {
        // SAFETY: getpgid and killpg only read the process table / deliver a
        // signal to the group this pid leads.
        if unsafe { libc::getpgid(pid) } != pid {
            return false;
        }
        unsafe { libc::killpg(pid, libc::SIGTERM) };
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if unsafe { libc::killpg(pid, 0) } != 0 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        unsafe { libc::killpg(pid, libc::SIGKILL) };
    }
    true
}

fn read_tail(path: &Path, max: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else {
        return String::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if len > max {
        let _ = f.seek(SeekFrom::Start(len - max));
    }
    let mut buf = Vec::new();
    let _ = f.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf).into_owned();
    if len > max {
        text.split_once('\n')
            .map(|(_, rest)| rest.to_string())
            .unwrap_or(text)
    } else {
        text
    }
}

// ------------------------------------------------------------------ broker

static SECRET: OnceLock<[u8; 32]> = OnceLock::new();
static SOCKET_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// The proof a run's server carries: over the scope, the run id, the chat
/// and the mode, so a proof minted for one run is refused for another, and a
/// bounded (auto) grant can't claim bypass.
pub(crate) fn proof_for(id: &Identity) -> String {
    let secret = SECRET.get_or_init(rand::random::<[u8; 32]>);
    let mut h = Sha256::new();
    h.update(secret);
    h.update([0u8]);
    h.update(b"services/2");
    for part in [&id.scope, &id.run_id, &id.chat, &id.perm] {
        h.update([0u8]);
        h.update(part.trim().as_bytes());
    }
    format!("{:x}", h.finalize())
}

fn proof_matches(expected: &str, given: &str) -> bool {
    let (a, b) = (expected.as_bytes(), given.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The `hollow_services` entry for a chat turn in `cwd` (`run_id`, `chat`,
/// canonical mode `perm`), when the broker listens. None without a run id
/// or chat: the grant is always tied to one run.
pub(crate) fn server_entry(
    cwd: &Path,
    run_id: &str,
    chat: &str,
    perm: &str,
) -> Option<(&'static str, Value)> {
    let sock = attach_socket()?;
    let scope = scope_of(cwd).ok()?;
    if run_id.trim().is_empty() || chat.trim().is_empty() || perm.trim().is_empty() {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let id = Identity {
        scope,
        run_id: run_id.trim().into(),
        chat: chat.trim().into(),
        perm: perm.trim().into(),
    };
    Some(crate::services_mcp::entry_for(
        &exe,
        &sock,
        &id,
        &proof_for(&id),
    ))
}

fn attach_socket() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(sock) = ATTACH_OVERRIDE.with(|o| o.borrow().clone()) {
        return sock;
    }
    SOCKET_PATH
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

#[cfg(test)]
thread_local! {
    static ATTACH_OVERRIDE: std::cell::RefCell<Option<Option<PathBuf>>> = const { std::cell::RefCell::new(None) };
}

/// Run `f` with this thread's broker socket replaced (test only).
#[cfg(test)]
pub(crate) fn with_attach_override<R>(sock: Option<PathBuf>, f: impl FnOnce() -> R) -> R {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            ATTACH_OVERRIDE.with(|o| *o.borrow_mut() = None);
        }
    }
    ATTACH_OVERRIDE.with(|o| *o.borrow_mut() = Some(sock));
    let _reset = Reset;
    f()
}

#[derive(Debug, Deserialize)]
pub(crate) struct BrokerRequest {
    #[serde(default)]
    pub v: u32,
    pub op: String,
    pub scope: String,
    #[serde(default, rename = "runId")]
    pub run_id: String,
    #[serde(default)]
    pub chat: String,
    #[serde(default)]
    pub perm: String,
    pub proof: String,
    #[serde(default)]
    pub args: Value,
}

impl BrokerRequest {
    fn identity(&self) -> Identity {
        Identity {
            scope: self.scope.trim().into(),
            run_id: self.run_id.trim().into(),
            chat: self.chat.trim().into(),
            perm: self.perm.trim().into(),
        }
    }
}

/// Why a request from an ended turn is refused.
pub(crate) const RUN_ENDED: &str =
    "The turn this service permission was granted to has ended, so it has expired; nothing was started or stopped.";

/// One forwarded call: proof (scope, run, chat, mode), the run still live,
/// the scope, then the operation. `is_live_scope` says whether the scope is
/// still a live build worktree (production:
/// `build_workspaces::workspace_for_worktree`), `is_live_run` whether the
/// run is still in flight (production: `automated_history::run_is_live`).
pub(crate) async fn dispatch(
    reg: &Arc<Registry>,
    req: &BrokerRequest,
    is_live_scope: impl Fn(&str) -> bool,
    is_live_run: impl Fn(&str) -> bool + Sync,
    env: impl Iterator<Item = (OsString, OsString)>,
) -> Result<Value, String> {
    if req.v != 2 {
        return Err(format!("unsupported service request version {}", req.v));
    }
    let id = req.identity();
    let incomplete = [&id.scope, &id.run_id, &id.chat, &id.perm]
        .iter()
        .any(|p| p.is_empty());
    if incomplete || !proof_matches(&proof_for(&id), req.proof.trim()) {
        return Err(
            "This request doesn't carry a valid Hollow identity; nothing was started or stopped."
                .into(),
        );
    }
    if !is_live_run(id.run_id.as_str()) {
        return Err(RUN_ENDED.into());
    }
    let scope = id.scope.as_str();
    if !is_live_scope(scope) {
        return Err(
            "Services run only in a build workspace's worktree, and this one is no longer bound."
                .into(),
        );
    }
    let name = || {
        req.args
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    match req.op.as_str() {
        "start_service" => {
            let mut spec = parse_start(scope, &req.args)?;
            spec.run_id = id.run_id.clone();
            spec.owner = bounded(&id.perm).then(|| id.chat.clone());
            let note = if spec.owner.is_some() {
                "It keeps running after this turn and stops when this conversation is archived or someone stops it. Read its output with service_logs."
            } else {
                "It keeps running after this turn, until someone stops it (Bypass mode: archiving this conversation doesn't stop it). Read its output with service_logs."
            };
            let view = reg
                .start_checked(spec, env, &|| is_live_run(id.run_id.as_str()))
                .await?;
            Ok(json!({ "started": view, "note": note }))
        }
        "list_services" => Ok(json!({ "services": reg.list(Some(scope)) })),
        "stop_service" => {
            let name = valid_name(&name())?;
            reg.stop_checked(scope, &name, &|| is_live_run(id.run_id.as_str()))
                .await?;
            Ok(json!({ "stopped": name }))
        }
        "service_logs" => {
            let name = valid_name(&name())?;
            let lines = req
                .args
                .get("lines")
                .and_then(Value::as_u64)
                .map(|n| n as usize);
            Ok(json!({ "name": name, "log": reg.logs(scope, &name, lines)? }))
        }
        other => Err(format!("unknown service operation {other}")),
    }
}

/// Bind the broker socket and serve (lib.rs setup, configured vaults only).
/// Also stops services a previous Hollow left running, and wires the change
/// event. Non-fatal: without a socket nothing attaches the server.
#[cfg(unix)]
pub(crate) fn start(app: impl Into<crate::core_host::Core>, vault: PathBuf) {
    let app = app.into();
    let reg = registry();
    let stale = reg.clean_up_stale();
    if stale > 0 {
        eprintln!("[services] stopped {stale} service(s) left by a previous Hollow");
    }
    let emitter = app.clone();
    reg.set_on_change(Arc::new(move || {
        let _ = emitter.emit(EVENT, ());
    }));
    let dir = crate::instance::cache_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join(format!("services-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = match std::os::unix::net::UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[services] bind {} failed: {e}", path.display());
            return;
        }
    };
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    *SOCKET_PATH.lock().unwrap_or_else(|e| e.into_inner()) = Some(path.clone());
    let _ = std::thread::Builder::new()
        .name("hollow-services-broker".into())
        .spawn(move || {
            let mut errors = 0u32;
            for conn in listener.incoming() {
                match conn {
                    Ok(stream) => {
                        errors = 0;
                        let (reg, vault) = (Arc::clone(&reg), vault.clone());
                        let peers = crate::run_auth::global();
                        let _ = std::thread::Builder::new()
                            .name("hollow-services-request".into())
                            .spawn(move || handle_conn(stream, &reg, &vault, peers));
                    }
                    Err(e) => {
                        errors += 1;
                        if errors >= 20 {
                            eprintln!("[services] accept: {e}; stopping");
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(50 * u64::from(errors)));
                    }
                }
            }
            *SOCKET_PATH.lock().unwrap_or_else(|e| e.into_inner()) = None;
            let _ = std::fs::remove_file(&path);
        });
}

#[cfg(not(unix))]
pub(crate) fn start(_app: impl Into<crate::core_host::Core>, _vault: PathBuf) {}

/// The request on a connection, once its peer is inside the tree of the run
/// its proof names (`run_auth::check_peer`, merged plan slice 6): a proof
/// read out of another run's `hollow services-mcp` environment is refused.
#[cfg(unix)]
fn admitted_request(
    peers: &crate::run_auth::Peers,
    stream: &std::os::unix::net::UnixStream,
    line: &str,
) -> Result<BrokerRequest, String> {
    use std::os::fd::AsRawFd;
    let req = serde_json::from_str::<BrokerRequest>(line.trim())
        .map_err(|e| format!("unreadable service request ({e})"))?;
    let fd = stream.as_raw_fd();
    crate::run_auth::check_peer(peers, fd, "service broker", Some(&req.run_id))?;
    Ok(req)
}

#[cfg(unix)]
fn handle_conn(
    stream: std::os::unix::net::UnixStream,
    reg: &Arc<Registry>,
    vault: &Path,
    peers: &crate::run_auth::Peers,
) {
    use std::io::{BufRead, BufReader, Read};
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut line = String::new();
    if BufReader::new((&stream).take(256 * 1024))
        .read_line(&mut line)
        .is_err()
    {
        return;
    }
    let answer = match admitted_request(peers, &stream, &line) {
        Ok(req) => {
            let live = |scope: &str| {
                crate::build_workspaces::workspace_for_worktree(vault, Path::new(scope)).is_some()
            };
            let live_run = |run: &str| crate::automated_history::run_is_live(vault, run);
            match tauri::async_runtime::block_on(dispatch(
                reg,
                &req,
                live,
                live_run,
                std::env::vars_os(),
            )) {
                Ok(result) => json!({ "ok": true, "result": result }),
                Err(error) => json!({ "ok": false, "error": error }),
            }
        }
        Err(error) => json!({ "ok": false, "error": error }),
    };
    let mut out = answer.to_string();
    out.push('\n');
    let mut stream = stream;
    let _ = stream.write_all(out.as_bytes());
}

// ------------------------------------------------------------------ hooks

/// `build_workspaces::remove`: stop the worktree's services first.
pub(crate) async fn stop_worktree(worktree: &Path) {
    if let Some(reg) = REGISTRY.get() {
        if let Ok(scope) = scope_of(worktree) {
            reg.stop_scope(&scope).await;
        }
    }
}

/// A chat was archived (`sessions_api::archive_agent_session`): stop the
/// bounded services it started, in the background.
pub(crate) fn stop_chat(chat: &str) {
    if let Some(reg) = REGISTRY.get() {
        let (reg, chat) = (Arc::clone(reg), chat.to_string());
        tauri::async_runtime::spawn(async move { reg.stop_owner(&chat).await });
    }
}

/// The services `run_id` started, for its receipt (`chat::finish_observed_run`).
pub(crate) fn take_started_by_run(run_id: &str) -> Vec<ReceiptService> {
    REGISTRY
        .get()
        .map(|reg| reg.take_started(run_id))
        .unwrap_or_default()
}

/// lib.rs exit handler: end every service.
pub(crate) fn stop_all_on_exit() {
    if let Some(reg) = REGISTRY.get() {
        reg.stop_all_blocking();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::Builder::new()
            .prefix("services-")
            .tempdir()
            .unwrap();
        let scope = tmp
            .path()
            .join("work tree")
            .canonicalize()
            .unwrap_or_else(|_| {
                std::fs::create_dir_all(tmp.path().join("work tree")).unwrap();
                tmp.path().join("work tree").canonicalize().unwrap()
            });
        (tmp, scope)
    }

    fn reg(tmp: &Path) -> Arc<Registry> {
        Arc::new(Registry::new(
            Some(tmp.join("services.json")),
            tmp.join("logs"),
        ))
    }

    fn env() -> impl Iterator<Item = (OsString, OsString)> {
        vec![
            ("PATH".into(), "/usr/bin:/bin".into()),
            (
                "HOLLOW_RUN_TAG".into(),
                "feedfacefeedfacefeedfacefeedface".into(),
            ),
            ("HOLLOW_VAULT".into(), "/v".into()),
            (
                "TMPDIR".into(),
                "/private/tmp/hollow-run-501/scratch/".into(),
            ),
            ("KEEP".into(), "1".into()),
        ]
        .into_iter()
    }

    fn start_args(name: &str, command: &str, kind: &str) -> Value {
        json!({ "name": name, "command": command, "kind": kind, "port": 5173 })
    }

    async fn wait_for(mut f: impl FnMut() -> bool) -> bool {
        for _ in 0..200 {
            if f() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        false
    }

    #[test]
    fn start_arguments_are_validated() {
        let (_tmp, scope) = temp();
        let s = scope.to_string_lossy();
        std::fs::create_dir_all(scope.join("web")).unwrap();
        let ok = parse_start(&s, &json!({ "name": "web", "command": "npm run dev", "cwd": "web", "port": 3000, "kind": "dev-server" })).unwrap();
        assert_eq!(ok.cwd, scope.join("web"));
        assert_eq!((ok.port, ok.kind), (Some(3000), Kind::DevServer));
        for bad in [
            json!({ "name": "../x", "command": "a" }),
            json!({ "name": "web", "command": "" }),
            json!({ "name": "web", "command": "a", "cwd": "/etc" }),
            json!({ "name": "web", "command": "a", "cwd": ".." }),
            json!({ "name": "web", "command": "a", "cwd": "missing" }),
            json!({ "name": "web", "command": "a", "port": 70000 }),
            json!({ "name": "web", "command": "a", "kind": "daemon" }),
        ] {
            assert!(parse_start(&s, &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn service_environment_drops_hollow_variables_and_the_turn_tmpdir() {
        let out = service_env(env(), Path::new("/Users/me"));
        let get = |k: &str| {
            out.iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.to_string_lossy().into_owned())
        };
        assert!(get("HOLLOW_RUN_TAG").is_none() && get("HOLLOW_VAULT").is_none());
        assert_eq!(get("KEEP").as_deref(), Some("1"));
        assert!(!get("TMPDIR").unwrap().contains("hollow-run"));
        assert!(get("PATH").unwrap().starts_with("/Users/me/.cargo/bin:"));
    }

    #[tokio::test]
    async fn a_service_outlives_the_run_logs_and_stops_with_its_group() {
        let (tmp, scope) = temp();
        let s = scope.to_string_lossy().into_owned();
        let reg = reg(tmp.path());
        let marker = scope.join("env.txt");
        let command = format!(
            "env > '{}'; echo started; echo oops >&2; exec /bin/sleep 60",
            marker.display()
        );
        let view = reg
            .start(
                parse_start(&s, &start_args("web", &command, "dev-server")).unwrap(),
                env(),
            )
            .await
            .unwrap();
        assert_eq!(view.status, "running");
        let pid = view.pid as i32;
        assert!(
            wait_for(
                || std::fs::read_to_string(&view.log).is_ok_and(|l| l.lines().any(|x| x == "oops"))
            )
            .await,
            "stderr joins the log"
        );
        let seen = std::fs::read_to_string(&marker).unwrap();
        assert!(!seen.contains("feedface"), "never the run's tag: {seen}");
        assert!(!seen.contains("HOLLOW_VAULT"), "{seen}");
        assert!(
            !seen.contains("hollow-run"),
            "not the turn's scratch TMPDIR"
        );
        // Its own process group, recorded with its start time.
        assert_eq!(unsafe { libc::getpgid(pid) }, pid);
        let stored: Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.path().join("services.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(stored["services"][0]["name"], json!("web"));
        assert!(stored["services"][0]["startMicros"].as_u64().is_some());

        // A requesting "run" finishing (a GUI-owned run settled with its own
        // tag) leaves the service alone.
        let mut run = tokio::process::Command::new("/bin/sh");
        run.arg("-c").arg("sleep 0.2");
        let out = process_group::run_to_completion(&mut run, None, Duration::from_secs(10), None)
            .await
            .unwrap();
        assert!(out.status.unwrap().success());
        assert_eq!(
            crate::proc_info::pid_alive(pid),
            Some(true),
            "the service survived the run's finish sweep"
        );

        assert_eq!(reg.list(Some(&s)).len(), 1);
        assert!(reg
            .logs(&s, "web", Some(5))
            .unwrap()
            .lines()
            .any(|x| x == "started"));
        reg.stop(&s, "web").await.unwrap();
        assert_eq!(crate::proc_info::pid_alive(pid), Some(false));
        assert!(reg.list(Some(&s)).is_empty());
        let stored: Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.path().join("services.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(stored["services"], json!([]));
    }

    #[tokio::test]
    async fn one_dev_server_and_four_services_at_most() {
        let (tmp, scope) = temp();
        let s = scope.to_string_lossy().into_owned();
        let reg = reg(tmp.path());
        let start = |name: &str, kind: &str| {
            parse_start(&s, &start_args(name, "exec /bin/sleep 60", kind)).unwrap()
        };
        reg.start(start("web", "dev-server"), env()).await.unwrap();
        let err = reg
            .start(start("web2", "dev-server"), env())
            .await
            .unwrap_err();
        assert!(err.contains("one at a time"), "{err}");
        assert!(reg
            .start(start("web", "other"), env())
            .await
            .unwrap_err()
            .contains("already running"));
        for n in ["a", "b", "c"] {
            reg.start(start(n, "other"), env()).await.unwrap();
        }
        assert!(reg
            .start(start("d", "other"), env())
            .await
            .unwrap_err()
            .contains("4 services"));
        reg.stop_scope(&s).await;
        assert!(reg.list(None).is_empty());
    }

    #[tokio::test]
    async fn a_term_ignoring_service_is_killed_and_an_exited_one_is_listed() {
        let (tmp, scope) = temp();
        let s = scope.to_string_lossy().into_owned();
        let reg = reg(tmp.path());
        let stubborn = reg
            .start(
                parse_start(
                    &s,
                    &start_args(
                        "stubborn",
                        "trap '' TERM; while :; do sleep 1; done",
                        "other",
                    ),
                )
                .unwrap(),
                env(),
            )
            .await
            .unwrap();
        let started = std::time::Instant::now();
        reg.stop(&s, "stubborn").await.unwrap();
        assert!(
            started.elapsed() >= Duration::from_secs(2),
            "waited out the grace before SIGKILL"
        );
        assert_eq!(
            crate::proc_info::pid_alive(stubborn.pid as i32),
            Some(false)
        );

        reg.start(
            parse_start(&s, &start_args("once", "echo done; exit 3", "other")).unwrap(),
            env(),
        )
        .await
        .unwrap();
        assert!(
            wait_for(|| reg
                .list(Some(&s))
                .first()
                .is_some_and(|v| v.status == "exited"))
            .await
        );
        assert_eq!(reg.list(Some(&s))[0].exit_code, Some(3));
        // Starting the same name again replaces the exited record.
        reg.start(
            parse_start(&s, &start_args("once", "exec /bin/sleep 60", "other")).unwrap(),
            env(),
        )
        .await
        .unwrap();
        assert_eq!(reg.list(Some(&s)).len(), 1);
        reg.stop_all_blocking();
        assert!(reg.list(Some(&s)).iter().all(|v| v.status == "exited"));
    }

    fn identity(scope: &str, run_id: &str, chat: &str, perm: &str) -> Identity {
        Identity {
            scope: scope.into(),
            run_id: run_id.into(),
            chat: chat.into(),
            perm: perm.into(),
        }
    }

    /// A request carrying `id` and the proof minted for `minted`.
    fn request(id: &Identity, minted: &Identity, op: &str, args: Value) -> BrokerRequest {
        BrokerRequest {
            v: 2,
            op: op.into(),
            scope: id.scope.clone(),
            run_id: id.run_id.clone(),
            chat: id.chat.clone(),
            perm: id.perm.clone(),
            proof: proof_for(minted),
            args,
        }
    }

    async fn call(
        reg: &Arc<Registry>,
        id: &Identity,
        op: &str,
        args: Value,
    ) -> Result<Value, String> {
        dispatch(reg, &request(id, id, op, args), |_| true, |_| true, env()).await
    }

    /// Merged plan slice 6 (reviews A·S5, B·F11): wave 2 left the services
    /// proof in the `hollow services-mcp` environment, which any process of
    /// this user can read. A request carrying it is refused from outside
    /// every run Hollow started and from inside a different run; from inside
    /// the run it names it is admitted.
    #[cfg(unix)]
    #[test]
    fn a_valid_proof_from_outside_its_run_tree_is_refused() {
        let id = identity("/tmp/wt", "R1", "s-1", "bypassPermissions");
        let wire = crate::services_mcp::request("list_services", &id, &proof_for(&id), &json!({}));
        let line = wire.to_string();
        let (server, _client) = std::os::unix::net::UnixStream::pair().unwrap();
        let pid = std::process::id() as i32;

        let outside = crate::run_auth::Peers::new();
        let refused = admitted_request(&outside, &server, &line).unwrap_err();
        assert!(refused.contains("Hollow refused"), "{refused}");

        let other_run = crate::run_auth::Peers::new();
        other_run.register("other", pid, vec!["R0".into()]);
        let replay = admitted_request(&other_run, &server, &line).unwrap_err();
        assert!(replay.contains("R1"), "{replay}");

        let own_run = crate::run_auth::Peers::new();
        own_run.register("own", pid, vec!["R1".into()]);
        let admitted = admitted_request(&own_run, &server, &line).unwrap();
        assert_eq!(admitted.identity(), id);
    }

    #[tokio::test]
    async fn the_broker_checks_the_proof_and_the_scope() {
        let (tmp, scope) = temp();
        let s = scope.to_string_lossy().into_owned();
        let reg = reg(tmp.path());
        let id = identity(&s, "R1", "s-1", "bypassPermissions");
        let mut forged = request(&id, &id, "list_services", json!({}));
        forged.proof = "0".repeat(64);
        let forged = dispatch(&reg, &forged, |_| true, |_| true, env())
            .await
            .unwrap_err();
        assert!(forged.contains("valid Hollow identity"));
        let elsewhere = identity("/elsewhere", "R1", "s-1", "bypassPermissions");
        assert!(
            dispatch(
                &reg,
                &request(&elsewhere, &id, "list_services", json!({})),
                |_| true,
                |_| true,
                env()
            )
            .await
            .is_err(),
            "a proof is bound to its scope"
        );
        let unbound = dispatch(
            &reg,
            &request(&id, &id, "list_services", json!({})),
            |_| false,
            |_| true,
            env(),
        )
        .await
        .unwrap_err();
        assert!(unbound.contains("no longer bound"));
        let started = call(
            &reg,
            &id,
            "start_service",
            start_args("api", "exec /bin/sleep 60", "other"),
        )
        .await
        .unwrap();
        assert_eq!(started["started"]["name"], json!("api"));
        let listed = call(&reg, &id, "list_services", json!({})).await.unwrap();
        assert_eq!(listed["services"].as_array().unwrap().len(), 1);
        call(&reg, &id, "stop_service", json!({ "name": "api" }))
            .await
            .unwrap();
        assert!(reg.list(None).is_empty());
    }

    /// Merged plan decision 1 (review B·F11): the grant names its run, chat
    /// and mode. A proof minted for another run, chat or mode is refused,
    /// and every proof expires when its run ends.
    #[tokio::test]
    async fn a_proof_is_tied_to_its_run_and_expires_with_it() {
        let (tmp, scope) = temp();
        let s = scope.to_string_lossy().into_owned();
        let reg = reg(tmp.path());
        let auto = identity(&s, "R1", "s-1", "auto");
        for (claimed, why) in [
            (identity(&s, "R2", "s-1", "auto"), "another run"),
            (identity(&s, "R1", "s-2", "auto"), "another chat"),
            (
                identity(&s, "R1", "s-1", "bypassPermissions"),
                "a stronger mode",
            ),
            (identity(&s, "", "s-1", "auto"), "no run"),
        ] {
            let refused = dispatch(
                &reg,
                &request(
                    &claimed,
                    &auto,
                    "start_service",
                    start_args("web", "exec /bin/sleep 60", "other"),
                ),
                |_| true,
                |_| true,
                env(),
            )
            .await;
            assert!(
                refused.unwrap_err().contains("valid Hollow identity"),
                "{why}"
            );
        }
        // The run ended: its proof is still well-formed, and still refused.
        let ended = dispatch(
            &reg,
            &request(
                &auto,
                &auto,
                "start_service",
                start_args("web", "exec /bin/sleep 60", "other"),
            ),
            |_| true,
            |run| run != "R1",
            env(),
        )
        .await;
        assert_eq!(ended.unwrap_err(), RUN_ENDED);
        let old = BrokerRequest {
            v: 1,
            ..request(&auto, &auto, "list_services", json!({}))
        };
        assert!(
            dispatch(&reg, &old, |_| true, |_| true, env())
                .await
                .unwrap_err()
                .contains("version"),
            "a version-1 request carries no run"
        );
        assert!(reg.list(None).is_empty(), "nothing was started");
    }

    /// Decision 1: in auto (and every mode but bypass) a service's folder
    /// must resolve inside the worktree, a symlink judged by where it leads.
    #[tokio::test]
    async fn auto_refuses_a_folder_outside_the_worktree_even_through_a_symlink() {
        let (tmp, scope) = temp();
        let s = scope.to_string_lossy().into_owned();
        let reg = reg(tmp.path());
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, scope.join("escape")).unwrap();
        std::fs::create_dir_all(scope.join("web")).unwrap();
        let auto = identity(&s, "R1", "s-1", "auto");
        for cwd in [
            outside.to_string_lossy().into_owned(),
            "..".into(),
            "escape".into(),
            scope.join("escape").to_string_lossy().into_owned(),
        ] {
            let args = json!({ "name": "web", "command": "exec /bin/sleep 60", "cwd": cwd });
            let refused = call(&reg, &auto, "start_service", args).await.unwrap_err();
            assert!(
                refused.contains("outside this worktree")
                    && refused.contains("Nothing was started"),
                "{cwd}: {refused}"
            );
        }
        assert!(reg.list(None).is_empty(), "nothing was started");
        let inside = call(
            &reg,
            &auto,
            "start_service",
            json!({ "name": "web", "command": "exec /bin/sleep 60", "cwd": "web" }),
        )
        .await
        .unwrap();
        assert_eq!(
            inside["started"]["cwd"],
            json!(scope.join("web").to_string_lossy())
        );
        reg.stop_scope(&s).await;
    }

    /// Decision 1: an auto service ends with its chat; a bypass one is
    /// unrestricted (started with no card, not tied to its chat).
    #[tokio::test]
    async fn an_auto_service_stops_when_its_chat_ends_and_a_bypass_one_does_not() {
        let (tmp, scope) = temp();
        let s = scope.to_string_lossy().into_owned();
        let reg = reg(tmp.path());
        let auto = identity(&s, "R1", "s-1", "auto");
        let bypass = identity(&s, "R2", "s-1", "bypassPermissions");
        let bounded = call(
            &reg,
            &auto,
            "start_service",
            start_args("watch", "exec /bin/sleep 60", "other"),
        )
        .await
        .unwrap();
        assert!(bounded["note"].as_str().unwrap().contains("archived"));
        let free = call(
            &reg,
            &bypass,
            "start_service",
            start_args("api", "exec /bin/sleep 60", "other"),
        )
        .await
        .unwrap();
        assert!(free["note"].as_str().unwrap().contains("Bypass"));
        let pid = |v: &Value| v["started"]["pid"].as_u64().unwrap() as i32;
        let stored: Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.path().join("services.json")).unwrap(),
        )
        .unwrap();
        let owners: Vec<Value> = stored["services"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["owner"].clone())
            .collect();
        assert_eq!(
            owners,
            vec![json!("s-1"), Value::Null],
            "the owner is recorded for the bounded one only"
        );

        reg.stop_owner("s-2").await;
        assert_eq!(reg.list(None).len(), 2, "another chat's end stops nothing");
        reg.stop_owner("s-1").await;
        assert_eq!(
            crate::proc_info::pid_alive(pid(&bounded)),
            Some(false),
            "the auto service ended with its chat"
        );
        assert_eq!(
            crate::proc_info::pid_alive(pid(&free)),
            Some(true),
            "the bypass service is unrestricted"
        );
        assert_eq!(
            reg.list(None)
                .iter()
                .map(|v| v.name.as_str())
                .collect::<Vec<_>>(),
            ["api"]
        );
        reg.stop_scope(&s).await;
        assert!(reg.list(None).is_empty());
    }

    /// Decision 1: the command and folder of each service a turn started go
    /// on that turn's receipt, and only that turn's.
    #[tokio::test]
    async fn the_command_is_recorded_on_the_starting_turns_receipt() {
        let (tmp, scope) = temp();
        let s = scope.to_string_lossy().into_owned();
        let reg = reg(tmp.path());
        let auto = identity(&s, "R1", "s-1", "auto");
        call(
            &reg,
            &auto,
            "start_service",
            start_args("web", "exec /bin/sleep 60", "dev-server"),
        )
        .await
        .unwrap();
        call(
            &reg,
            &identity(&s, "R2", "s-1", "bypassPermissions"),
            "start_service",
            start_args("api", "exec /bin/sleep 61", "other"),
        )
        .await
        .unwrap();
        let rows = reg.take_started("R1");
        assert_eq!(
            rows,
            vec![ReceiptService {
                name: "web".into(),
                command: "exec /bin/sleep 60".into(),
                cwd: s.clone(),
                port: Some(5173),
                kind: "dev-server".into(),
                ends_with_chat: true,
            }]
        );
        assert!(reg.take_started("R1").is_empty(), "taken once");
        assert!(!reg.take_started("R2")[0].ends_with_chat);

        let vault = tempfile::tempdir().unwrap();
        let mut collector = crate::evidence::EvidenceCollector::new("chat", None, "R1");
        collector.set_services(rows);
        let receipt = collector.finish(vault.path(), "success", vec![]).unwrap();
        assert_eq!(receipt.services[0].command, "exec /bin/sleep 60");
        let wire = serde_json::to_value(&receipt).unwrap();
        assert_eq!(wire["services"][0]["cwd"], json!(s));
        assert_eq!(wire["services"][0]["endsWithChat"], json!(true));
        reg.stop_scope(&s).await;
    }

    /// The hooks are wired where chats end and turns finish.
    #[test]
    fn archiving_a_chat_and_finishing_a_turn_reach_the_registry() {
        let archive = include_str!("commands/sessions_api.rs");
        assert!(archive.contains(concat!("services::", "stop_chat(&key)")));
        let chat = include_str!("chat.rs");
        assert!(chat.contains(concat!(
            "collector.set_services(crate::services::",
            "take_started_by_run(&run_id))"
        )));
    }

    #[test]
    fn stale_records_are_stopped_only_when_the_start_time_matches() {
        let tmp = tempfile::Builder::new()
            .prefix("services-stale-")
            .tempdir()
            .unwrap();
        let spawn = || {
            use std::os::unix::process::CommandExt;
            let mut c = std::process::Command::new("/bin/sleep");
            c.arg("60").process_group(0);
            c.spawn().unwrap()
        };
        let mut matching = spawn();
        let mut reused = spawn();
        let record = |pid: u32, start: Option<u64>| Record {
            scope: "/w".into(),
            name: format!("s{pid}"),
            command: "sleep".into(),
            cwd: "/w".into(),
            port: None,
            kind: Kind::Other,
            pid,
            start_micros: start,
            log: "/dev/null".into(),
            started_at: "t".into(),
            owner: None,
            run_id: String::new(),
        };
        let start_of = |pid: u32| crate::proc_info::process_start_micros(pid as i32);
        let records = vec![
            record(matching.id(), start_of(matching.id())),
            record(reused.id(), start_of(reused.id()).map(|s| s + 1)),
        ];
        std::fs::write(
            tmp.path().join("services.json"),
            serde_json::to_string(&json!({ "schemaVersion": 1, "services": records })).unwrap(),
        )
        .unwrap();
        let reg = Registry::new(
            Some(tmp.path().join("services.json")),
            tmp.path().join("logs"),
        );
        assert_eq!(reg.clean_up_stale(), 1);
        let _ = matching.wait();
        assert!(
            reused.try_wait().unwrap().is_none(),
            "a pid whose start time differs is never signalled"
        );
        let _ = reused.kill();
        let _ = reused.wait();
        assert!(!tmp.path().join("services.json").exists());
    }
}
