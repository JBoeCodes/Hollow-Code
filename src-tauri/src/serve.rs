//! `hollow serve` (M1 PR 2): Hollow's core with no window server.
//!
//! M0 found that the stock app never starts its core without a window
//! server: Tauri's `.setup` runs from `applicationDidFinishLaunching`, which
//! never fires for a LaunchDaemon or a background ssh session. Serve boots
//! the same core (`core_boot`) on a plain Tokio runtime instead: no
//! `tauri::Builder`, no tao/AppKit loop, no window, tray, menu, shortcuts,
//! autostart, window state or dialog plugin. Its host is [`ServeHost`]:
//! events go nowhere (or to stderr with `HOLLOW_SERVE_LOG_EVENTS=1`), a
//! native banner is a log line, and there is no main window.
//!
//! One Hollow per app data folder: serve takes the same `gui.lock` the app
//! takes at launch (`launch.rs`) and refuses to start while it is held, and
//! it leaves `serve.json` (its pid) beside the lock so an app launched
//! while serve runs says so and exits instead of trying to bring a window
//! forward ([`running_in`]). It also refuses a vault whose run queue another
//! Hollow process owns.
//!
//! M1 PR 3: serve keeps Hollow's secrets in a private file in the data
//! folder instead of the Keychain (`secret_store::use_file`), reads no
//! Keychain item at all ([`serving`]), and answers its commands (`hollow
//! serve pair`, `enable-phone` and the rest, `serve_control.rs`) on a 0600
//! control socket.
//!
//! Exit statuses: 0 after a SIGTERM or SIGINT settled every live run; 2 for
//! a bad command line; 75 when another Hollow owns the data folder or the
//! vault's run queue; 78 without a usable config.json or vault (serve has no
//! setup mode); 1 when the runtime cannot start. A second signal while
//! settling exits at once with 128 + the signal. Everything is logged to
//! stderr, which launchd captures.

use crate::commands::AppState;
use crate::core_host::{Core, CoreHost};
use crate::launch::elog;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub(crate) const EXIT_USAGE: i32 = 2;
pub(crate) const EXIT_BUSY: i32 = 75;
pub(crate) const EXIT_CONFIG: i32 = 78;
pub(crate) const EXIT_RUNTIME: i32 = 1;

/// Beside `gui.lock` in the app data folder while serve runs.
pub(crate) const MARKER_FILE: &str = "serve.json";

/// Logs every event serve's host drops, by name and size, when `1` (or
/// with `--log-events`).
const LOG_EVENTS_ENV: &str = "HOLLOW_SERVE_LOG_EVENTS";

/// Set once this process is `hollow serve`, before any thread starts.
static SERVING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether this process is `hollow serve`: code that would reach for the
/// login Keychain or a window skips it (`claude_quota`).
pub(crate) fn serving() -> bool {
    SERVING.load(std::sync::atomic::Ordering::SeqCst)
}

// ── The host ─────────────────────────────────────────────────────────────

/// The core host of `hollow serve`: its own [`AppState`], no window, no
/// webview listening for events. The state sits in a
/// [`StateShell`](crate::serve_actions::StateShell) so Hollow app tools run
/// against it too (M1 PR 5).
#[derive(Clone)]
pub(crate) struct ServeHost(Arc<ServeInner>);

struct ServeInner {
    shell: crate::serve_actions::StateShell,
    log_events: bool,
}

impl ServeHost {
    pub(crate) fn try_new(state: AppState, log_events: bool) -> Result<Self, String> {
        let shell = crate::serve_actions::StateShell::new(state)?;
        Ok(Self(Arc::new(ServeInner { shell, log_events })))
    }

    #[cfg(test)]
    pub(crate) fn new(state: AppState, log_events: bool) -> Self {
        Self::try_new(state, log_events).expect("serve's state shell")
    }
}

impl CoreHost for ServeHost {
    fn emit_json(&self, event: &str, json: String) -> Result<(), String> {
        if self.0.log_events {
            elog(&format!("[serve] event {event} ({} bytes)", json.len()));
        }
        Ok(())
    }
    fn emit_json_to(&self, target: &str, event: &str, json: String) -> Result<(), String> {
        if self.0.log_events {
            elog(&format!(
                "[serve] event {event} to {target} ({} bytes)",
                json.len()
            ));
        }
        Ok(())
    }
    fn core_state(&self) -> Option<&AppState> {
        Some(self.0.shell.state())
    }
    fn main_window(&self) -> Option<(bool, bool)> {
        None
    }
    fn show_native(&self, title: &str, body: &str) -> Result<(), String> {
        // The title only: a body can quote a chat. `--log-events` adds it.
        if self.0.log_events {
            elog(&format!("[serve] notification: {title}: {body}"));
        } else {
            elog(&format!("[serve] notification: {title}"));
        }
        Ok(())
    }
    fn gui(&self) -> Option<&tauri::AppHandle> {
        None
    }
    fn core(&self) -> Core {
        Core::new(self.clone())
    }
    fn action_host(&self) -> Option<Box<dyn crate::app_actions::Host>> {
        Some(Box::new(self.0.shell.action_host(Core::new(self.clone()))))
    }
}

// ── The command line ─────────────────────────────────────────────────────

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Options {
    pub data_dir: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub phone_port: Option<u16>,
    pub log_events: bool,
    pub help: bool,
}

pub(crate) fn help_text() -> &'static str {
    "Usage: hollow serve [--data-dir <folder>] [--config <config.json>] [--phone-port <port>] [--log-events]\n\
     \x20      hollow serve <command> [--data-dir <folder>] ...\n\n\
     Runs Hollow's core with no window: the run queue, automations, heartbeat, the\n\
     runner COO (when runner-coo is on), the phone API (when phone access is on) and\n\
     the sockets agents' tools call back to. Stop it with SIGTERM or SIGINT; live runs settle first.\n\
     Secrets live in secrets.json in the data folder (0600), not the Keychain.\n\n\
     --data-dir    a folder of serve's own (absolute): app data, plus its Support and\n\
                   Caches folders, so it shares no store with the app (default: the\n\
                   app's own folders)\n\
     --config      config.json to read (same as HOLLOW_CONFIG; it must exist); no setup mode\n\
     --phone-port  the phone API's loopback port (default 47680)\n\
     --log-events  log each event serve drops, by name and size (HOLLOW_SERVE_LOG_EVENTS=1)\n\n\
     Commands (the Settings a headless Hollow has no window for):\n\
     secret-set <key>         save a secret, its value read from stdin (never argv);\n\
                              keys: claude:<account id>, tracker:<provider>,\n\
                              runner.github.token, apns, jev\n\
     secret-list              the saved keys, never their values\n\
     secret-delete <key>      remove one\n\
     pair [--machine] [--json] open a pairing window in the running serve; prints the code;\n\
                              --machine: for another Mac (the peer link), --json: one line\n\
     enable-phone             turn the phone API on (in the running serve, or for its next start)\n\
     disable-phone            turn it off\n\
     set-default-mode <mode> [--agent <agent>]\n\
                              the permission mode a phone's new chat starts in\n\
                              (plan, default, acceptEdits, auto, bypassPermissions;\n\
                              Plan when none is set); without --agent it is read the\n\
                              most restrictive way for every agent, and the command\n\
                              prints what each agent's new chat gets\n\
     runner-coo on|off|status this Mac is the runner: its COO is woken by its chats'\n\
                              ends and its own follow-ups (no check-ins); status\n\
                              prints the role and the runner COO's chat key\n\n\
     Exit status: 0 stopped cleanly (or the command worked), 2 bad arguments, 75 another\n\
     Hollow owns the data folder or the vault's run queue, 78 no usable config.json or\n\
     vault, 1 the runtime could not start (or the command failed).\n"
}

/// The options after `hollow serve`, or the message for a bad command line.
pub(crate) fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_string())),
            _ => (arg.as_str(), None),
        };
        let mut value = |name: &str| {
            inline
                .clone()
                .or_else(|| rest.next().cloned())
                .filter(|v| !v.trim().is_empty() && !v.starts_with("--"))
                .ok_or_else(|| format!("hollow serve: {name} needs a value."))
        };
        match flag {
            "-h" | "--help" => options.help = true,
            "--data-dir" => {
                let dir = PathBuf::from(value("--data-dir")?);
                if !dir.is_absolute() {
                    return Err("hollow serve: --data-dir must be an absolute path.".into());
                }
                options.data_dir = Some(dir);
            }
            "--config" => options.config = Some(PathBuf::from(value("--config")?)),
            "--log-events" => options.log_events = true,
            "--phone-port" => {
                let raw = value("--phone-port")?;
                let port = raw
                    .parse::<u16>()
                    .ok()
                    .filter(|p| *p >= 1024)
                    .ok_or_else(|| {
                        format!("hollow serve: --phone-port wants a port from 1024 to 65535, not `{raw}`.")
                    })?;
                options.phone_port = Some(port);
            }
            other => {
                return Err(format!(
                    "hollow serve: unknown option `{other}`. Run `hollow serve --help`."
                ))
            }
        }
    }
    Ok(options)
}

// ── One Hollow per data folder ───────────────────────────────────────────

/// The pid a running serve left in `dir`, when that process is alive. The
/// app reads this when its own launch lock is held (`launch.rs`).
pub(crate) fn running_in(dir: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(dir.join(MARKER_FILE)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let pid = u32::try_from(value.get("pid")?.as_u64()?).ok()?;
    alive(pid).then_some(pid)
}

fn alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    if pid <= 1 {
        return false;
    }
    // SAFETY: signal 0 only checks that the process exists.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// The data folder's lock and marker, held for the life of serve.
pub(crate) struct Owner {
    _lock: crate::flock::HeldLock,
    marker: PathBuf,
}

impl Owner {
    /// Take `dir`'s launch lock without waiting and write the marker.
    pub(crate) fn take(dir: &Path) -> Result<Self, (i32, String)> {
        match crate::launch::acquire(&dir.join(crate::launch::LOCK_FILE), Duration::ZERO) {
            crate::launch::Acquire::First(lock) => {
                let marker = dir.join(MARKER_FILE);
                let text = serde_json::json!({
                    "pid": std::process::id(),
                    "startedAt": chrono::Utc::now().to_rfc3339(),
                })
                .to_string();
                std::fs::write(&marker, text + "\n")
                    .map_err(|e| (EXIT_RUNTIME, format!("write {}: {e}", marker.display())))?;
                Ok(Self { _lock: lock, marker })
            }
            crate::launch::Acquire::Running => Err((
                EXIT_BUSY,
                format!(
                    "another Hollow (the app, or another hollow serve) is running on the data folder {}; nothing was started. Quit it first.",
                    dir.display()
                ),
            )),
            crate::launch::Acquire::Unavailable(why) => Err((
                EXIT_RUNTIME,
                format!("cannot take the data folder's lock in {}: {why}", dir.display()),
            )),
        }
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.marker);
    }
}

/// `Err` when another live process owns `vault`'s run queue. Takes the
/// owner lock and lets it go at once; `core_boot` claims it for good.
fn queue_free(vault: &Path) -> Result<(), String> {
    let Some(path) = crate::session_store::pump_lock_path(vault) else {
        return Ok(());
    };
    match crate::launch::acquire(&path, Duration::ZERO) {
        crate::launch::Acquire::Running => Err(busy_vault(vault)),
        _ => Ok(()),
    }
}

fn busy_vault(vault: &Path) -> String {
    format!(
        "another Hollow process owns the run queue of the vault {}; nothing was started. Quit it first, or point serve at its own vault.",
        vault.display()
    )
}

/// Under `--data-dir` serve's stores, its queue lock included, live in its
/// own folder, so it also holds the vault's queue lock in the app's own
/// support folder for its whole life: an app on the same vault then finds
/// the queue owned elsewhere, and serve refuses a vault the app runs.
fn hold_app_queue_lock(vault: &Path) -> Result<Option<crate::flock::HeldLock>, String> {
    let Some(support) = crate::instance::default_support_dir() else {
        return Ok(None);
    };
    let path = crate::session_store::pump_lock_path_under(&support, vault);
    match crate::launch::acquire(&path, Duration::ZERO) {
        crate::launch::Acquire::First(lock) => Ok(Some(lock)),
        crate::launch::Acquire::Running => Err(busy_vault(vault)),
        crate::launch::Acquire::Unavailable(_) => Ok(None),
    }
}

// ── Boot and lifecycle ───────────────────────────────────────────────────

/// `hollow serve <args>`: the exit status.
pub(crate) fn main(args: Vec<String>) -> i32 {
    if let Some(command) = args
        .first()
        .filter(|c| crate::serve_control::COMMANDS.contains(&c.as_str()))
    {
        return crate::serve_control::main(command, &args[1..]);
    }
    let options = match parse(&args) {
        Ok(options) => options,
        Err(message) => {
            elog(&message);
            return EXIT_USAGE;
        }
    };
    if options.help {
        let _ = std::io::Write::write_all(&mut std::io::stdout(), help_text().as_bytes());
        return 0;
    }
    // Before any thread starts: every store reads these when it opens.
    // `--data-dir` moves the support and cache roots under it too, so a
    // serve beside the app shares none of its stores.
    if let Some(dir) = &options.data_dir {
        std::env::set_var("HOLLOW_APP_DATA_DIR", dir);
        crate::instance::isolate_under(dir);
    }
    if let Some(config) = &options.config {
        std::env::set_var("HOLLOW_CONFIG", config);
    }
    // A named config.json must exist: the app's lookup would otherwise fall
    // through to ~/.config/Hollow, Application Support and the working
    // folder, and a daemon would quietly serve whichever vault it found
    // there (seen in the M1 PR 4 smoke on the mini).
    if let Err(message) = named_config(std::env::var("HOLLOW_CONFIG").ok().as_deref()) {
        elog(&format!("[serve] {}", no_config(message)));
        return EXIT_CONFIG;
    }
    let dir = match crate::agent_settings::app_data_dir() {
        Ok(dir) => dir,
        Err(why) => {
            elog(&format!("[serve] no app data folder: {why}"));
            return EXIT_CONFIG;
        }
    };
    if let Err(error) = std::fs::create_dir_all(&dir) {
        elog(&format!("[serve] cannot create {}: {error}", dir.display()));
        return EXIT_RUNTIME;
    }
    // A folder of serve's own (secrets, sockets, the sessions DB) is the
    // user's alone.
    if options.data_dir.is_some() {
        private_folder(&dir);
    }
    let owner = match Owner::take(&dir) {
        Ok(owner) => owner,
        Err((code, message)) => {
            elog(&format!("[serve] {message}"));
            return code;
        }
    };
    SERVING.store(true, std::sync::atomic::Ordering::SeqCst);
    // Before the core boots: whether this Mac is the runner (M3b).
    crate::runner_coo::load(&dir);
    // Before anything reads a secret: the file, never the Keychain.
    crate::secret_store::use_file(dir.join(crate::secret_store::FILE_NAME));
    let state = match crate::core_boot::load_state()
        .map_err(no_config)
        .and_then(usable)
    {
        Ok(state) => state,
        Err(message) => {
            elog(&format!("[serve] {message}"));
            return EXIT_CONFIG;
        }
    };
    let vault = state.config.vault_path.clone();
    if let Err(message) = queue_free(&vault) {
        elog(&format!("[serve] {message}"));
        return EXIT_BUSY;
    }
    let _app_queue = if options.data_dir.is_some() {
        match hold_app_queue_lock(&vault) {
            Ok(lock) => lock,
            Err(message) => {
                elog(&format!("[serve] {message}"));
                return EXIT_BUSY;
            }
        }
    } else {
        None
    };
    crate::instance::write_manifest(&state.config_path, Some(&vault));
    if let Some(port) = options.phone_port {
        crate::phone_access::set_port(port);
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("hollow-serve")
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            elog(&format!("[serve] cannot start the runtime: {error}"));
            return EXIT_RUNTIME;
        }
    };
    // Every `tauri::async_runtime::spawn` in the core lands on this runtime.
    tauri::async_runtime::set(runtime.handle().clone());
    let _entered = runtime.enter();
    let log_events =
        options.log_events || std::env::var(LOG_EVENTS_ENV).is_ok_and(|v| v.trim() == "1");
    let host = match ServeHost::try_new(state, log_events) {
        Ok(host) => host,
        Err(message) => {
            elog(&format!("[serve] {message}"));
            return EXIT_RUNTIME;
        }
    };
    let code = boot_and_wait(&runtime, Core::new(host), &dir);
    // Never wait on a service loop's task: the process exits right after.
    drop(_entered);
    runtime.shutdown_timeout(Duration::from_secs(2));
    // Only now: with the runtime down the pump can claim nothing more.
    crate::run_pump::release_ownership();
    drop(owner);
    code
}

/// `--config` (or `HOLLOW_CONFIG`) names a file that is there; nothing named
/// is fine (the usual lookup runs).
fn named_config(named: Option<&str>) -> Result<(), String> {
    match named.filter(|p| !p.trim().is_empty()) {
        Some(path) if !Path::new(path).is_file() => Err(format!("{path} is not a file")),
        _ => Ok(()),
    }
}

fn no_config(e: String) -> String {
    format!("no usable config.json ({e}). hollow serve has no setup mode: set Hollow up in the app first, or pass --config <file>.")
}

/// `dir` to 0700 when it is this user's and others can open it.
pub(crate) fn private_folder(dir: &Path) {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let Ok(meta) = std::fs::metadata(dir) else {
        return;
    };
    // SAFETY: geteuid has no preconditions.
    let me = unsafe { libc::geteuid() };
    if meta.uid() == me && meta.mode() & 0o077 != 0 {
        match std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)) {
            Ok(()) => elog(&format!("[serve] {} is now 0700", dir.display())),
            Err(error) => elog(&format!("[serve] cannot chmod {}: {error}", dir.display())),
        }
    }
}

/// The config serve runs: a loaded config.json with its vault on disk
/// (else exit 78).
fn usable(state: AppState) -> Result<AppState, String> {
    if !state.config.vault_path.is_dir() {
        return Err(format!(
            "the vault {} in {} is not a folder on this Mac; nothing was started.",
            state.config.vault_path.display(),
            state.config_path.display()
        ));
    }
    Ok(state)
}

/// Boot the core the way the app's setup does, minus the window, then
/// wait for a quit signal and settle.
fn boot_and_wait(runtime: &tokio::runtime::Runtime, core: Core, dir: &Path) -> i32 {
    let boot_at = chrono::Utc::now();
    // The boot sweep first (R2), as in the app's setup.
    let stopped_leftovers = crate::process_group::orphans::start_at_boot(boot_at);
    let signals = match crate::process_group::orphans::QuitSignals::install() {
        Ok(signals) => signals,
        Err(error) => {
            elog(&format!("[serve] cannot watch quit signals: {error}"));
            return EXIT_RUNTIME;
        }
    };
    crate::core_boot::process_services(&core, true);
    crate::core_boot::account_services(boot_at);
    crate::core_boot::vault_services(&core, true, boot_at, stopped_leftovers);
    // A core that doesn't own its queue would idle "ready" and run nothing.
    if !crate::run_pump::queue_state().owned() {
        elog(&format!(
            "[serve] the run queue is not owned here ({:?}); stopping",
            crate::run_pump::queue_state()
        ));
        return EXIT_BUSY;
    }
    for (service, why) in crate::core_boot::SKIPPED_IN_SERVE {
        elog(&format!("[serve] not started: {service} ({why})"));
    }
    let vault_path = core
        .try_state::<AppState>()
        .map(|s| s.config.vault_path.clone());
    // Before the socket: `runner-coo on|off` needs the folder and vault.
    if let Some(vault) = &vault_path {
        crate::runner_coo::place(dir, vault);
    }
    // The commands' way in, once the phone API is up (`serve_control`).
    let control = match crate::serve_control::bind(dir) {
        Ok((listener, control)) => {
            match crate::serve_control::start(listener, crate::serve_control::core_handler()) {
                Ok(()) => Some(control),
                Err(error) => {
                    elog(&format!("[serve] no control socket: {error}"));
                    None
                }
            }
        }
        Err(error) => {
            elog(&format!("[serve] no control socket: {error}"));
            None
        }
    };
    let vault = core
        .try_state::<AppState>()
        .map(|s| s.config.vault_path.display().to_string())
        .unwrap_or_default();
    elog(&format!(
        "[serve] ready: pid {}, vault {vault}, app data {}, control {}, run queue {:?}",
        std::process::id(),
        dir.display(),
        if control.is_some() {
            crate::serve_control::SOCKET_FILE
        } else {
            "none"
        },
        crate::run_pump::queue_state()
    ));
    // The runner COO's line (M3b): it may create `workspaces/main`, so off
    // the runtime's threads, once the socket is up.
    if let Some(vault) = vault_path {
        runtime.spawn_blocking(move || {
            elog(&crate::runner_coo::line(crate::runner_coo::role(), &vault))
        });
    }
    let signal = std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0));
    let seen = signal.clone();
    runtime.block_on(crate::process_group::orphans::settle_on_signal(
        signals,
        crate::stop_children_once,
        move |s| seen.store(s, std::sync::atomic::Ordering::SeqCst),
    ));
    elog(&format!(
        "[serve] signal {} settled; stopped",
        signal.load(std::sync::atomic::Ordering::SeqCst)
    ));
    0
}

#[cfg(test)]
#[path = "serve_tests.rs"]
mod tests;
