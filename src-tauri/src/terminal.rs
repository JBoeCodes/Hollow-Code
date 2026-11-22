//! The terminal drawer's shells (t3code parity 7.2: D2, D7).
//!
//! A person-driven shell on a pseudo-terminal, shown as a tab of the Preview
//! pane (`src/components/TerminalPanel.tsx`). It is the person's own click and
//! typing, never an agent action: the commands below are `plumbing` in the
//! command map and appear in no agent tool table, so an agent can neither open
//! a terminal nor type into one. The sandbox story, in the order it is applied:
//!
//! 1. **The chat's mode decides whether a shell may start** ([`gate`]). The
//!    terminal is not confined to a folder the way a sandboxed agent's shell
//!    is, so it starts only where the chat's agent itself could run commands
//!    beyond a folder sandbox (`agent::mode_capability`: commands and reach at
//!    least `Checked`). Plan, Claude's Default and Accept edits, Codex's Read
//!    only and Ask for approval, Grok's Default, Accept edits and Plan and
//!    Antigravity's non-auto modes are refused with the reason in words. The
//!    drawer re-asks on every mode change (`terminal_gate`) and a mode that no
//!    longer allows a shell ends the chat's terminals.
//! 2. **The folder** is the chat's own working folder (or the vault when it has
//!    none), absolute, canonicalized, and never a system or shared root
//!    (`agent::unsafe_working_dir`, the check agent runs get).
//! 3. **The environment is an allowlist** ([`scrubbed_env`]): HOME, USER,
//!    LOGNAME, LANG, LC_*, TZ, TMPDIR, a PATH, and what the terminal sets
//!    itself (TERM, COLORTERM, TERM_PROGRAM, SHELL). Nothing else is inherited:
//!    no API key, token, Hollow variable (`HOLLOW_*`, run tags, broker proofs)
//!    or agent socket. The login shell then reads the person's own rc files,
//!    which may export what the person put there; Hollow adds nothing.
//! 4. **Ending.** Every shell leads its own session and process group (the pty
//!    crate calls `setsid`), so closing one signals the whole group: SIGHUP,
//!    a grace period, SIGKILL ([`end_groups`]). Archiving, settling or deleting
//!    the chat ([`stop_chat`]), closing a tab, a mode that forbids shells and
//!    quitting Hollow (`lib.rs` `stop_children_once`) all end the terminals the
//!    same way. A shell that ends by itself has its group hung up once, when it
//!    is reaped (its jobs get SIGHUP, as a shell sends at exit), and is never
//!    signalled again, because its id may be reused. A crash closes the master
//!    side of each pty, which hangs up the shells (a job that left its group
//!    with `setsid`, or ignores SIGHUP after its shell ended, is not reached).
//!
//! Output is read on a thread per shell and sent to the window as
//! [`OUTPUT_EVENT`] (`{id, data}`, UTF-8; a character split across two reads is
//! held until its tail arrives); an exit is [`EXIT_EVENT`] (`{id, code}`).
//! The window keeps its own screen buffer per shell, so the tab can be left
//! and opened again with the output intact. At most [`MAX_PER_CHAT`] shells a
//! chat and [`MAX_TOTAL`] in all.

use serde::Serialize;
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

pub const OUTPUT_EVENT: &str = "terminal-output";
pub const EXIT_EVENT: &str = "terminal-exit";
/// Shells one chat may keep open.
pub(crate) const MAX_PER_CHAT: usize = 4;
/// Shells open at once, in every chat.
pub(crate) const MAX_TOTAL: usize = 12;
/// The most one write may carry (a large paste).
const WRITE_MAX: usize = 1024 * 1024;
const ROWS: std::ops::RangeInclusive<u16> = 2..=300;
const COLS: std::ops::RangeInclusive<u16> = 10..=600;
/// One read from a pty, and so one event at most.
const READ_CHUNK: usize = 32 * 1024;
/// SIGHUP, then this long before SIGKILL.
const HANGUP_GRACE: Duration = Duration::from_millis(1200);
/// After SIGKILL, how long a close waits for the group to be gone.
const KILL_WAIT: Duration = Duration::from_millis(1500);

/// Why a chat in `perm` may not start or keep a terminal, or `None` when it
/// may. `agent` is the chat's agent id; an unknown or missing agent is read
/// as the most restrictive of the agents that run that mode
/// (`agent::parent_capability`), so a name nobody knows never widens it.
pub(crate) fn gate(agent: Option<&str>, perm: &str) -> Option<String> {
    use crate::agent::{parent_capability, AgentKind, ModeReach};
    let kind = agent.and_then(AgentKind::from_opt);
    let cap = parent_capability(kind, perm);
    if cap.commands && cap.reach >= ModeReach::Checked {
        return None;
    }
    let mode = crate::app_mcp::normalize_perm(perm);
    let why = if mode == "plan" {
        "Plan mode reads and changes nothing, so a shell would do more than the chat is allowed to."
    } else if !cap.commands {
        "In this mode the agent does not run commands, so a shell would do more than the chat is allowed to."
    } else {
        "In this mode the agent's commands are held to the working folder, which a terminal cannot be."
    };
    Some(format!(
        "The terminal is off for this chat. {why} Switch the chat to a mode that runs commands without a folder sandbox (Auto, Approve for me or Bypass) to use it."
    ))
}

/// The environment names a terminal inherits from Hollow, besides `LC_*`.
/// Everything else is dropped; see the module doc.
const ENV_ALLOW: &[&str] = &[
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LANGUAGE",
    "TZ",
    "TMPDIR",
    "PATH",
    "__CF_USER_TEXT_ENCODING",
];

/// PATH when Hollow has none (a Finder launch has the system's short one).
const FALLBACK_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// The shell's whole environment: `base` (Hollow's) filtered through
/// [`ENV_ALLOW`] (UTF-8 names and values without NUL), then what the terminal
/// sets itself. Pure, so a test hands it a polluted environment.
pub(crate) fn scrubbed_env(
    base: impl IntoIterator<Item = (OsString, OsString)>,
    shell: &Path,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (name, value) in base {
        let (Some(name), Some(value)) = (name.to_str(), value.to_str()) else {
            continue;
        };
        if name.is_empty() || name.contains('=') || value.contains('\0') {
            continue;
        }
        if ENV_ALLOW.contains(&name) || name.starts_with("LC_") {
            out.push((name.to_string(), value.to_string()));
        }
    }
    if !out.iter().any(|(k, v)| k == "PATH" && !v.is_empty()) {
        out.retain(|(k, _)| k != "PATH");
        out.push(("PATH".into(), FALLBACK_PATH.into()));
    }
    for (name, value) in [
        ("TERM", "xterm-256color"),
        ("COLORTERM", "truecolor"),
        ("TERM_PROGRAM", "Hollow"),
    ] {
        out.retain(|(k, _)| k != name);
        out.push((name.into(), value.into()));
    }
    out.retain(|(k, _)| k != "SHELL");
    out.push(("SHELL".into(), shell.to_string_lossy().into_owned()));
    out
}

/// The person's login shell when it is a real, listed one, else zsh.
pub(crate) fn pick_shell(from_env: Option<&str>, shells_file: &str) -> PathBuf {
    let listed = |path: &str| {
        shells_file
            .lines()
            .map(str::trim)
            .any(|line| !line.starts_with('#') && line == path)
    };
    if let Some(path) = from_env {
        if path.starts_with('/') && listed(path) && is_executable(Path::new(path)) {
            return PathBuf::from(path);
        }
    }
    PathBuf::from("/bin/zsh")
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Take the longest valid UTF-8 prefix off `pending` as a string. An invalid
/// byte becomes U+FFFD; an incomplete character at the end (up to three bytes)
/// stays in `pending` for the next read.
pub(crate) fn take_utf8(pending: &mut Vec<u8>) -> String {
    let mut out = String::new();
    loop {
        match std::str::from_utf8(pending) {
            Ok(text) => {
                out.push_str(text);
                pending.clear();
                return out;
            }
            Err(error) => {
                let good = error.valid_up_to();
                out.push_str(std::str::from_utf8(&pending[..good]).unwrap_or(""));
                match error.error_len() {
                    // A truncated character at the end: wait for the rest.
                    None => {
                        pending.drain(..good);
                        return out;
                    }
                    Some(bad) => {
                        out.push('\u{FFFD}');
                        pending.drain(..good + bad);
                    }
                }
            }
        }
    }
}

/// What the window is told. Tests collect these; production emits events.
#[derive(Clone)]
pub(crate) struct Sink {
    pub(crate) output: Arc<dyn Fn(&str, String) + Send + Sync>,
    pub(crate) exit: Arc<dyn Fn(&str, Option<i32>) + Send + Sync>,
}

/// How to start one shell.
pub(crate) struct Spec {
    pub(crate) cwd: PathBuf,
    pub(crate) program: PathBuf,
    pub(crate) args: Vec<String>,
    pub(crate) env: Vec<(String, String)>,
    pub(crate) rows: u16,
    pub(crate) cols: u16,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Opened {
    pub id: String,
    pub cwd: String,
    pub shell: String,
    pub pid: u32,
}

struct Session {
    chat: Mutex<String>,
    pid: u32,
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Box<dyn portable_pty::MasterPty + Send>>,
    exited: AtomicBool,
}

/// The shells of one Hollow process. Production uses [`registry`]; tests build
/// their own.
#[derive(Default)]
pub(crate) struct Registry {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    next: AtomicU64,
}

fn clamp_size(rows: u16, cols: u16) -> (u16, u16) {
    (
        rows.clamp(*ROWS.start(), *ROWS.end()),
        cols.clamp(*COLS.start(), *COLS.end()),
    )
}

impl Registry {
    /// A helper thread could not start: forget the shell and end its group,
    /// so no running shell is left that the screen never heard of.
    fn abandon(&self, id: &str, pid: u32) {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        end_groups(&[pid]);
    }

    /// Start a shell for `chat`. Refuses beyond [`MAX_PER_CHAT`] and
    /// [`MAX_TOTAL`]. The caller has already applied the mode gate and
    /// checked the folder.
    pub(crate) fn open(&self, chat: &str, spec: Spec, sink: Sink) -> Result<Opened, String> {
        use portable_pty::{native_pty_system, CommandBuilder, PtySize};
        {
            let map = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            if map.len() >= MAX_TOTAL {
                return Err(format!(
                    "Hollow keeps at most {MAX_TOTAL} terminals open at once. Close one first."
                ));
            }
            let mine = map
                .values()
                .filter(|s| *s.chat.lock().unwrap_or_else(|e| e.into_inner()) == chat)
                .count();
            if mine >= MAX_PER_CHAT {
                return Err(format!(
                    "A chat keeps at most {MAX_PER_CHAT} terminals. Close one first."
                ));
            }
        }
        let (rows, cols) = clamp_size(spec.rows, spec.cols);
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("Could not open a terminal: {e}"))?;
        let mut cmd = CommandBuilder::new(&spec.program);
        // The crate seeds a builder with this process's whole environment;
        // the allowlist replaces it, never adds to it.
        cmd.env_clear();
        for (name, value) in &spec.env {
            cmd.env(name, value);
        }
        cmd.args(&spec.args);
        cmd.cwd(&spec.cwd);
        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("Could not start {}: {e}", spec.program.display()))?;
        // The child holds the only slave end it needs: dropping ours lets the
        // read side see the hang-up when the group is gone.
        drop(pair.slave);
        let pid = child.process_id().unwrap_or(0);
        // Anything that fails from here on must not leave the shell running.
        let ends = |child: &mut Box<dyn portable_pty::Child + Send + Sync>| {
            let _ = child.kill();
            end_groups(&[pid]);
        };
        if pid <= 1 {
            ends(&mut child);
            return Err("Could not start the terminal's shell.".into());
        }
        let reader = match pair.master.try_clone_reader() {
            Ok(reader) => reader,
            Err(e) => {
                ends(&mut child);
                return Err(format!("Could not read the terminal: {e}"));
            }
        };
        let writer = match pair.master.take_writer() {
            Ok(writer) => writer,
            Err(e) => {
                ends(&mut child);
                return Err(format!("Could not write to the terminal: {e}"));
            }
        };
        let id = format!("term-{}", self.next.fetch_add(1, Ordering::Relaxed) + 1);
        let session = Arc::new(Session {
            chat: Mutex::new(chat.to_string()),
            pid,
            writer: Mutex::new(writer),
            master: Mutex::new(pair.master),
            exited: AtomicBool::new(false),
        });
        {
            // The caps again, now that the lock is held across the insert: two
            // opens that raced past the first check cannot both fit.
            let mut map = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            let mine = map
                .values()
                .filter(|s| *s.chat.lock().unwrap_or_else(|e| e.into_inner()) == chat)
                .count();
            if map.len() >= MAX_TOTAL || mine >= MAX_PER_CHAT {
                drop(map);
                ends(&mut child);
                return Err("Too many terminals are open. Close one first.".into());
            }
            map.insert(id.clone(), Arc::clone(&session));
        }
        let read_id = id.clone();
        let read_sink = sink.clone();
        std::thread::Builder::new()
            .name(format!("{id}-read"))
            .spawn(move || pump(reader, &read_id, &read_sink))
            .map_err(|e| {
                self.abandon(&id, pid);
                format!("Could not read the terminal: {e}")
            })?;
        let wait_id = id.clone();
        let wait_session = Arc::clone(&session);
        std::thread::Builder::new()
            .name(format!("{id}-wait"))
            .spawn(move || {
                let code = child.wait().ok().map(|status| status.exit_code() as i32);
                wait_session.exited.store(true, Ordering::SeqCst);
                // The leader was just reaped, so the group id is still the shell's
                // while any job is in it: hang those up once, as a shell does at
                // exit. After this the id is never signalled again (it could be
                // reused), so a later close of an ended shell sends nothing.
                #[cfg(unix)]
                signal_group(wait_session.pid as i32, libc::SIGHUP);
                (sink.exit)(&wait_id, code);
            })
            .map_err(|e| {
                self.abandon(&id, pid);
                format!("Could not watch the terminal: {e}")
            })?;
        Ok(Opened {
            id,
            cwd: spec.cwd.to_string_lossy().into_owned(),
            shell: spec.program.to_string_lossy().into_owned(),
            pid,
        })
    }

    fn get(&self, id: &str) -> Result<Arc<Session>, String> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
            .ok_or_else(|| "That terminal is closed.".to_string())
    }

    /// Type into a shell (the window's key presses and pastes).
    pub(crate) fn write(&self, id: &str, data: &[u8]) -> Result<(), String> {
        if data.len() > WRITE_MAX {
            return Err("That paste is too large for the terminal.".into());
        }
        let session = self.get(id)?;
        if session.exited.load(Ordering::SeqCst) {
            return Err("That terminal's shell has ended. Close it and open a new one.".into());
        }
        let mut writer = session.writer.lock().unwrap_or_else(|e| e.into_inner());
        writer
            .write_all(data)
            .and_then(|()| writer.flush())
            .map_err(|e| format!("Could not type into the terminal: {e}"))
    }

    pub(crate) fn resize(&self, id: &str, rows: u16, cols: u16) -> Result<(), String> {
        use portable_pty::PtySize;
        let (rows, cols) = clamp_size(rows, cols);
        let session = self.get(id)?;
        let master = session.master.lock().unwrap_or_else(|e| e.into_inner());
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("Could not resize the terminal: {e}"))
    }

    /// Take `ids` out of the registry and end the live ones' groups together
    /// (one grace period for all). A shell that already ended had its group
    /// hung up when it was reaped and is never signalled again: its id may
    /// have been reused since.
    fn end(&self, ids: &[String]) {
        let mut pids = Vec::new();
        {
            let mut map = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            for id in ids {
                if let Some(session) = map.remove(id) {
                    if !session.exited.load(Ordering::SeqCst) {
                        pids.push(session.pid);
                    }
                }
            }
        }
        end_groups(&pids);
    }

    /// End one shell and everything in its group. Idempotent.
    pub(crate) fn close(&self, id: &str) {
        self.end(&[id.to_string()]);
    }

    /// End every shell of `chat`; returns how many.
    pub(crate) fn close_chat(&self, chat: &str) -> usize {
        let ids: Vec<String> = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(_, s)| *s.chat.lock().unwrap_or_else(|e| e.into_inner()) == chat)
            .map(|(id, _)| id.clone())
            .collect();
        self.end(&ids);
        ids.len()
    }

    /// A chat that had no key yet got one: its shells follow it.
    pub(crate) fn adopt_chat(&self, from: &str, to: &str) {
        for session in self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
        {
            let mut chat = session.chat.lock().unwrap_or_else(|e| e.into_inner());
            if *chat == from {
                *chat = to.to_string();
            }
        }
    }

    /// End every shell (Hollow is quitting).
    pub(crate) fn close_all(&self) {
        let ids: Vec<String> = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect();
        self.end(&ids);
    }

    #[cfg(test)]
    fn count(&self) -> usize {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }
}

/// Read a pty to its end, sending what arrives.
fn pump(mut reader: Box<dyn Read + Send>, id: &str, sink: &Sink) {
    let mut chunk = vec![0u8; READ_CHUNK];
    let mut pending: Vec<u8> = Vec::new();
    loop {
        match reader.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                pending.extend_from_slice(&chunk[..n]);
                let text = take_utf8(&mut pending);
                if !text.is_empty() {
                    (sink.output)(id, text);
                }
            }
        }
    }
    if !pending.is_empty() {
        (sink.output)(id, String::from_utf8_lossy(&pending).into_owned());
    }
}

/// Whether any process still sits in group `pgid`.
#[cfg(unix)]
fn group_alive(pgid: i32) -> bool {
    if pgid <= 1 {
        return false;
    }
    // SAFETY: signal 0 only probes for existence.
    let rc = unsafe { libc::kill(-pgid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

/// Signal group `pgid`; refuses ids no shell of ours can have (0, 1, and this
/// process's own group, which a bad pid could otherwise name).
#[cfg(unix)]
fn signal_group(pgid: i32, signal: i32) {
    // SAFETY: getpgrp has no preconditions.
    let own = unsafe { libc::getpgrp() };
    if pgid <= 1 || pgid == own {
        return;
    }
    // SAFETY: killpg only delivers a signal; the group is a shell's own
    // (the pty crate made it a session leader), still unreaped or non-empty.
    unsafe {
        libc::killpg(pgid, signal);
    }
}

/// Hang up each live shell's group, wait out one grace period for all of
/// them, then kill what is left. The ids are shells that were not reaped when
/// the caller looked, so they are still the shells' own.
#[cfg(unix)]
pub(crate) fn end_groups(pids: &[u32]) {
    let live: Vec<i32> = pids
        .iter()
        .map(|&p| p as i32)
        .filter(|&p| group_alive(p))
        .collect();
    if live.is_empty() {
        return;
    }
    for &pgid in &live {
        signal_group(pgid, libc::SIGHUP);
        // A stopped job ignores SIGHUP until it is continued.
        signal_group(pgid, libc::SIGCONT);
    }
    let wait_until = |limit: Duration, left: &mut Vec<i32>| {
        let start = std::time::Instant::now();
        while !left.is_empty() && start.elapsed() < limit {
            left.retain(|&p| group_alive(p));
            if !left.is_empty() {
                std::thread::sleep(Duration::from_millis(25));
            }
        }
        left.retain(|&p| group_alive(p));
    };
    let mut left = live;
    wait_until(HANGUP_GRACE, &mut left);
    for &pgid in &left {
        signal_group(pgid, libc::SIGKILL);
    }
    wait_until(KILL_WAIT, &mut left);
}

#[cfg(not(unix))]
pub(crate) fn end_groups(_pids: &[u32]) {}

static REGISTRY: OnceLock<Registry> = OnceLock::new();

pub(crate) fn registry() -> &'static Registry {
    REGISTRY.get_or_init(Registry::default)
}

/// A chat was archived, settled or deleted: its shells end. Runs on a thread
/// (a close waits for the group to go).
pub(crate) fn stop_chat(chat: &str) {
    if REGISTRY.get().is_none() {
        return;
    }
    let chat = chat.to_string();
    std::thread::spawn(move || {
        registry().close_chat(&chat);
    });
}

/// lib.rs exit handler: end every shell.
pub(crate) fn stop_all_on_exit() {
    if let Some(reg) = REGISTRY.get() {
        reg.close_all();
    }
}

fn app_sink(app: tauri::AppHandle) -> Sink {
    use tauri::Emitter;
    let out = app.clone();
    Sink {
        output: Arc::new(move |id, data| {
            let _ = out.emit(OUTPUT_EVENT, serde_json::json!({ "id": id, "data": data }));
        }),
        exit: Arc::new(move |id, code| {
            let _ = app.emit(EXIT_EVENT, serde_json::json!({ "id": id, "code": code }));
        }),
    }
}

/// The folder a terminal starts in: absolute, an existing folder, resolved
/// through symlinks, and not a system or shared root.
pub(crate) fn resolve_cwd(cwd: &str, vault: Option<&Path>) -> Result<PathBuf, String> {
    let path = Path::new(cwd);
    if !path.is_absolute() {
        return Err("The terminal's folder must be an absolute path.".into());
    }
    let canonical = std::fs::canonicalize(path)
        .map_err(|_| format!("{cwd} is not a folder this Mac can open."))?;
    if !canonical.is_dir() {
        return Err(format!("{cwd} is not a folder."));
    }
    if let Some(reason) = crate::agent::unsafe_working_dir(&canonical, vault) {
        return Err(format!(
            "The terminal won't start in {reason}. Choose a project folder, or use the vault."
        ));
    }
    Ok(canonical)
}

/// Start a terminal for a chat. The mode gate, the folder check and the
/// environment allowlist all run here, before any process exists.
#[tauri::command]
pub async fn terminal_open(
    chat: String,
    agent: Option<String>,
    perm: String,
    cwd: String,
    rows: u16,
    cols: u16,
    state: tauri::State<'_, crate::commands::AppState>,
    app: tauri::AppHandle,
) -> Result<Opened, String> {
    if chat.trim().is_empty() {
        return Err("Open the terminal from a chat.".into());
    }
    if let Some(reason) = gate(agent.as_deref(), &perm) {
        return Err(reason);
    }
    let vault = state.config.vault_path.clone();
    let cwd = resolve_cwd(&cwd, Some(&vault))?;
    let shells = std::fs::read_to_string("/etc/shells").unwrap_or_default();
    let shell = pick_shell(std::env::var("SHELL").ok().as_deref(), &shells);
    let env = scrubbed_env(std::env::vars_os(), &shell);
    let spec = Spec {
        cwd,
        program: shell,
        args: vec!["-l".into()],
        env,
        rows,
        cols,
    };
    let sink = app_sink(app);
    tokio::task::spawn_blocking(move || registry().open(&chat, spec, sink))
        .await
        .map_err(|e| e.to_string())?
}

/// The chat's mode changed: ends the chat's terminals and says why when the
/// new mode forbids a shell, else returns nothing.
#[tauri::command]
pub async fn terminal_gate(
    chat: String,
    agent: Option<String>,
    perm: String,
) -> Result<Option<String>, String> {
    let Some(reason) = gate(agent.as_deref(), &perm) else {
        return Ok(None);
    };
    if REGISTRY.get().is_some() {
        let _ = tokio::task::spawn_blocking(move || registry().close_chat(&chat)).await;
    }
    Ok(Some(reason))
}

#[tauri::command]
pub async fn terminal_write(id: String, data: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || registry().write(&id, data.as_bytes()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn terminal_resize(id: String, rows: u16, cols: u16) -> Result<(), String> {
    registry().resize(&id, rows, cols)
}

#[tauri::command]
pub async fn terminal_close(id: String) -> Result<(), String> {
    if REGISTRY.get().is_none() {
        return Ok(());
    }
    tokio::task::spawn_blocking(move || registry().close(&id))
        .await
        .map_err(|e| e.to_string())
}

/// Hand a chat's terminals to the key it just got.
#[tauri::command]
pub async fn terminal_adopt_chat(from: String, to: String) -> Result<(), String> {
    if REGISTRY.get().is_some() {
        registry().adopt_chat(&from, &to);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn os(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(k, v)| (OsString::from(k), OsString::from(v)))
            .collect()
    }

    /// A sink that keeps every output chunk and exit in one channel.
    fn collecting() -> (Sink, mpsc::Receiver<Event>) {
        let (tx, rx) = mpsc::channel();
        let out = tx.clone();
        (
            Sink {
                output: Arc::new(move |_id, data| {
                    let _ = out.send(Event::Out(data));
                }),
                exit: Arc::new(move |_id, code| {
                    let _ = tx.send(Event::Exit(code));
                }),
            },
            rx,
        )
    }

    #[derive(Debug)]
    enum Event {
        Out(String),
        Exit(Option<i32>),
    }

    fn spec(program: &str, args: &[&str], cwd: &Path) -> Spec {
        Spec {
            cwd: cwd.to_path_buf(),
            program: PathBuf::from(program),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: scrubbed_env(
                os(&[("HOME", "/Users/x"), ("PATH", "/usr/bin:/bin")]),
                Path::new(program),
            ),
            rows: 24,
            cols: 80,
        }
    }

    /// Collect output until `done` says so, the shell exits or `wait` passes.
    fn read_until(
        rx: &mpsc::Receiver<Event>,
        wait: Duration,
        done: impl Fn(&str) -> bool,
    ) -> (String, Option<Option<i32>>) {
        let mut text = String::new();
        let mut exit = None;
        let deadline = std::time::Instant::now() + wait;
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Event::Out(data)) => {
                    text.push_str(&data);
                    if done(&text) {
                        break;
                    }
                }
                Ok(Event::Exit(code)) => {
                    exit = Some(code);
                    if done(&text) {
                        break;
                    }
                }
                Err(_) => {}
            }
        }
        (text, exit)
    }

    fn pid_gone(pid: i32) -> bool {
        // SAFETY: signal 0 only probes for existence.
        let rc = unsafe { libc::kill(pid, 0) };
        rc != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }

    fn wait_gone(pid: i32, wait: Duration) -> bool {
        let start = std::time::Instant::now();
        while start.elapsed() < wait {
            if pid_gone(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50)); // sleep-ok: polling a pid the OS reaps
        }
        pid_gone(pid)
    }

    // ---- the mode gate ----

    #[test]
    fn a_shell_needs_a_mode_that_runs_commands_beyond_a_folder_sandbox() {
        for agent in ["claude", "codex", "grok", "antigravity"] {
            assert!(gate(Some(agent), "plan").is_some(), "{agent} plan");
            assert!(gate(Some(agent), "auto").is_none(), "{agent} auto");
            assert!(
                gate(Some(agent), "bypassPermissions").is_none(),
                "{agent} bypass"
            );
        }
        // Commands held to a folder (Codex's workspace sandbox, Grok's
        // workspace profile) or declined (Claude's headless default and
        // acceptEdits) are not a shell's reach.
        assert!(gate(Some("codex"), "default").is_some());
        assert!(gate(Some("codex"), "acceptEdits").is_some());
        assert!(gate(Some("claude"), "default").is_some());
        assert!(gate(Some("claude"), "acceptEdits").is_some());
        assert!(gate(Some("grok"), "acceptEdits").is_some());
        assert!(gate(Some("antigravity"), "acceptEdits").is_some());
    }

    #[test]
    fn grok_auto_runs_unsandboxed_so_it_may_start_a_shell() {
        assert!(gate(Some("grok"), "auto").is_none());
    }

    #[test]
    fn an_unknown_or_missing_agent_never_widens_the_gate() {
        for agent in [None, Some("nobody"), Some("")] {
            assert!(gate(agent, "plan").is_some(), "{agent:?}");
            assert!(gate(agent, "acceptEdits").is_some(), "{agent:?}");
            // Every agent that runs auto and bypass reaches that far.
            assert!(gate(agent, "auto").is_none(), "{agent:?}");
            assert!(gate(agent, "bypassPermissions").is_none(), "{agent:?}");
        }
        // An unknown mode normalizes to acceptEdits, which is refused.
        assert!(gate(Some("claude"), "anything-else").is_some());
    }

    #[test]
    fn a_refusal_names_the_way_out_without_a_dash() {
        let text = gate(Some("claude"), "plan").unwrap();
        assert!(text.contains("Auto"), "{text}");
        assert!(
            !text.contains('\u{2014}') && !text.contains('\u{2013}'),
            "{text}"
        );
        let folder = gate(Some("codex"), "default").unwrap();
        assert!(folder.contains("folder"), "{folder}");
    }

    // ---- the environment ----

    #[test]
    fn the_environment_keeps_only_the_allowlist() {
        let base = os(&[
            ("HOME", "/Users/x"),
            ("USER", "x"),
            ("LANG", "en_US.UTF-8"),
            ("LC_ALL", "en_US.UTF-8"),
            ("LC_CTYPE", "UTF-8"),
            ("TZ", "America/New_York"),
            ("TMPDIR", "/var/folders/aa/T/"),
            ("PATH", "/opt/homebrew/bin:/usr/bin"),
            ("ANTHROPIC_API_KEY", "sk-ant-secret"),
            ("OPENAI_API_KEY", "sk-secret"),
            ("GITHUB_TOKEN", "ghp_secret"),
            ("GH_TOKEN", "gho_secret"),
            ("AWS_SECRET_ACCESS_KEY", "aws"),
            ("GOOGLE_APPLICATION_CREDENTIALS", "/k.json"),
            ("LINEAR_API_KEY", "lin"),
            ("HOLLOW_RUN_TAG", "abcd"),
            ("HOLLOW_BROKER_PROOF", "proof"),
            ("HOLLOW_APP_DATA_DIR", "/data"),
            ("SSH_AUTH_SOCK", "/tmp/agent.sock"),
            ("DATABASE_URL", "postgres://u:p@h/db"),
            ("MY_SERVICE_PASSWORD", "pw"),
            ("npm_config_registry", "https://r"),
            ("CLAUDECODE", "1"),
        ]);
        let env = scrubbed_env(base, Path::new("/bin/zsh"));
        let names: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        for kept in [
            "HOME", "USER", "LANG", "LC_ALL", "LC_CTYPE", "TZ", "TMPDIR", "PATH",
        ] {
            assert!(names.contains(&kept), "{kept} kept: {names:?}");
        }
        for dropped in [
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "GITHUB_TOKEN",
            "GH_TOKEN",
            "AWS_SECRET_ACCESS_KEY",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "LINEAR_API_KEY",
            "HOLLOW_RUN_TAG",
            "HOLLOW_BROKER_PROOF",
            "HOLLOW_APP_DATA_DIR",
            "SSH_AUTH_SOCK",
            "DATABASE_URL",
            "MY_SERVICE_PASSWORD",
            "npm_config_registry",
            "CLAUDECODE",
        ] {
            assert!(!names.contains(&dropped), "{dropped} dropped: {names:?}");
        }
        // Nothing left that even looks like a credential, by name or by value.
        for (name, value) in &env {
            let upper = name.to_ascii_uppercase();
            for word in [
                "KEY",
                "TOKEN",
                "SECRET",
                "PASSWORD",
                "CREDENTIAL",
                "HOLLOW_",
                "AUTH",
            ] {
                assert!(!upper.contains(word), "{name} looks secret");
            }
            assert!(
                !value.contains("secret") && !value.contains("sk-"),
                "{name}={value}"
            );
        }
        let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("TERM"), Some("xterm-256color"));
        assert_eq!(get("TERM_PROGRAM"), Some("Hollow"));
        assert_eq!(get("SHELL"), Some("/bin/zsh"));
        assert_eq!(get("PATH"), Some("/opt/homebrew/bin:/usr/bin"));
    }

    #[test]
    fn a_missing_path_gets_the_system_one_and_the_terminal_names_win() {
        let env = scrubbed_env(
            os(&[("TERM", "dumb"), ("SHELL", "/bin/evil"), ("PATH", "")]),
            Path::new("/bin/zsh"),
        );
        let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("PATH"), Some(FALLBACK_PATH));
        assert_eq!(get("TERM"), Some("xterm-256color"));
        assert_eq!(get("SHELL"), Some("/bin/zsh"));
        assert_eq!(env.iter().filter(|(k, _)| k == "TERM").count(), 1);
    }

    #[test]
    fn odd_names_and_values_are_dropped() {
        let env = scrubbed_env(
            vec![
                (OsString::from("A=B"), OsString::from("x")),
                (OsString::from("HOME"), OsString::from("/a\0b")),
                (OsString::from(""), OsString::from("x")),
            ],
            Path::new("/bin/zsh"),
        );
        assert!(env
            .iter()
            .all(|(k, _)| k != "HOME" && k != "A=B" && !k.is_empty()));
    }

    #[test]
    fn the_login_shell_is_used_only_when_listed_and_runnable() {
        let shells = "# comment\n/bin/bash\n/bin/zsh\n/bin/sh\n";
        assert_eq!(
            pick_shell(Some("/bin/bash"), shells),
            PathBuf::from("/bin/bash")
        );
        assert_eq!(
            pick_shell(Some("/tmp/evil"), shells),
            PathBuf::from("/bin/zsh")
        );
        assert_eq!(pick_shell(Some("bash"), shells), PathBuf::from("/bin/zsh"));
        assert_eq!(
            pick_shell(Some("/bin/fish"), shells),
            PathBuf::from("/bin/zsh")
        );
        assert_eq!(pick_shell(None, shells), PathBuf::from("/bin/zsh"));
    }

    // ---- the folder ----

    #[test]
    fn the_folder_must_be_a_real_project_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("project");
        std::fs::create_dir(&dir).unwrap();
        let ok = resolve_cwd(dir.to_str().unwrap(), None).unwrap();
        assert_eq!(ok, std::fs::canonicalize(&dir).unwrap());
        assert!(resolve_cwd("relative/dir", None).is_err());
        assert!(resolve_cwd(tmp.path().join("missing").to_str().unwrap(), None).is_err());
        let file = tmp.path().join("a.txt");
        std::fs::write(&file, "x").unwrap();
        assert!(resolve_cwd(file.to_str().unwrap(), None).is_err());
        for root in ["/", "/tmp", "/usr", "/System", "/Library"] {
            if Path::new(root).exists() {
                assert!(resolve_cwd(root, None).is_err(), "{root}");
            }
        }
        // A symlink is judged by where it leads.
        let link = tmp.path().join("to-root");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/", &link).unwrap();
            assert!(resolve_cwd(link.to_str().unwrap(), None).is_err());
        }
    }

    // ---- reading ----

    #[test]
    fn a_character_split_across_reads_is_held_until_it_is_whole() {
        let bytes = "a\u{00e9}\u{20ac}\u{1f600}b".as_bytes().to_vec();
        let mut pending = Vec::new();
        let mut out = String::new();
        for b in bytes {
            pending.push(b);
            out.push_str(&take_utf8(&mut pending));
        }
        assert_eq!(out, "a\u{00e9}\u{20ac}\u{1f600}b");
        assert!(pending.is_empty());
        let mut bad = vec![b'a', 0xff, b'b'];
        assert_eq!(take_utf8(&mut bad), "a\u{FFFD}b");
        assert!(bad.is_empty());
    }

    // ---- real shells ----

    #[test]
    fn a_shell_starts_in_the_given_folder_with_a_scrubbed_environment() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = std::fs::canonicalize(tmp.path()).unwrap();
        let (sink, rx) = collecting();
        let reg = Registry::default();
        // `env` prints what the shell would inherit; `pwd -P` where it is.
        let opened = reg
            .open(
                "chat-a",
                spec(
                    "/bin/sh",
                    &["-c", "pwd -P; /usr/bin/env; echo END_OF_ENV"],
                    &dir,
                ),
                sink,
            )
            .unwrap();
        assert_eq!(opened.cwd, dir.to_string_lossy());
        assert!(opened.pid > 1);
        let (text, _) = read_until(&rx, Duration::from_secs(10), |t| t.contains("END_OF_ENV"));
        let text = text.replace('\r', "");
        assert_eq!(
            text.lines().next().unwrap(),
            dir.to_string_lossy(),
            "{text}"
        );
        let names: Vec<&str> = text
            .lines()
            .skip(1)
            .filter_map(|l| l.split_once('=').map(|(k, _)| k))
            .collect();
        for secret in [
            "ANTHROPIC_API_KEY",
            "GITHUB_TOKEN",
            "HOLLOW_RUN_TAG",
            "SSH_AUTH_SOCK",
        ] {
            assert!(
                !names.contains(&secret),
                "{secret} reached the shell: {names:?}"
            );
        }
        assert!(
            names.contains(&"HOME") && names.contains(&"TERM"),
            "{names:?}"
        );
        reg.close_all();
    }

    #[test]
    fn the_shell_does_not_inherit_a_variable_set_in_this_process() {
        // The spec's env is the whole environment: a variable the test
        // process holds must not reach the child even though the pty crate
        // seeds its builder from it.
        let name = "HOLLOW_TERMINAL_TEST_SECRET";
        std::env::set_var(name, "leaked");
        let tmp = tempfile::tempdir().unwrap();
        let (sink, rx) = collecting();
        let reg = Registry::default();
        reg.open(
            "chat-a",
            spec(
                "/bin/sh",
                &["-c", "/usr/bin/env; echo END-OF-ENV"],
                tmp.path(),
            ),
            sink,
        )
        .unwrap();
        let (text, _) = read_until(&rx, Duration::from_secs(10), |t| t.contains("END-OF-ENV"));
        std::env::remove_var(name);
        assert!(text.contains("END-OF-ENV"), "{text}");
        assert!(!text.contains(name) && !text.contains("leaked"), "{text}");
        reg.close_all();
    }

    #[test]
    fn typing_reaches_the_shell_and_resize_is_accepted() {
        let tmp = tempfile::tempdir().unwrap();
        let (sink, rx) = collecting();
        let reg = Registry::default();
        let opened = reg
            .open("chat-a", spec("/bin/sh", &[], tmp.path()), sink)
            .unwrap();
        reg.write(&opened.id, b"echo hello-$((20+22))\n").unwrap();
        let (text, _) = read_until(&rx, Duration::from_secs(10), |t| t.contains("hello-42"));
        assert!(text.contains("hello-42"), "{text}");
        reg.resize(&opened.id, 40, 120).unwrap();
        reg.write(&opened.id, b"stty size\n").unwrap();
        let (text, _) = read_until(&rx, Duration::from_secs(10), |t| t.contains("40 120"));
        assert!(text.contains("40 120"), "{text}");
        assert!(reg.write("term-nope", b"x").is_err());
        reg.close_all();
    }

    #[test]
    fn an_exit_is_reported_with_its_code_and_further_typing_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let (sink, rx) = collecting();
        let reg = Registry::default();
        let opened = reg
            .open(
                "chat-a",
                spec("/bin/sh", &["-c", "exit 7"], tmp.path()),
                sink,
            )
            .unwrap();
        let (_, exit) = read_until(&rx, Duration::from_secs(10), |_| false);
        assert_eq!(exit, Some(Some(7)));
        assert!(reg.write(&opened.id, b"x").is_err());
        reg.close(&opened.id);
        assert_eq!(reg.count(), 0);
    }

    // ---- limits and cleanup ----

    #[test]
    fn a_chat_and_the_whole_app_are_capped() {
        let tmp = tempfile::tempdir().unwrap();
        let (sink, _rx) = collecting();
        let reg = Registry::default();
        for _ in 0..MAX_PER_CHAT {
            reg.open(
                "chat-a",
                spec("/bin/sh", &["-c", "sleep 60"], tmp.path()),
                sink.clone(),
            )
            .unwrap();
        }
        let err = reg
            .open(
                "chat-a",
                spec("/bin/sh", &["-c", "sleep 60"], tmp.path()),
                sink.clone(),
            )
            .unwrap_err();
        assert!(err.contains("at most"), "{err}");
        // Another chat still can, until the total is reached.
        for chat in ["chat-b", "chat-c"] {
            for _ in 0..MAX_PER_CHAT {
                if reg.count() >= MAX_TOTAL {
                    break;
                }
                reg.open(
                    chat,
                    spec("/bin/sh", &["-c", "sleep 60"], tmp.path()),
                    sink.clone(),
                )
                .unwrap();
            }
        }
        assert_eq!(reg.count(), MAX_TOTAL);
        assert!(reg
            .open(
                "chat-d",
                spec("/bin/sh", &["-c", "sleep 60"], tmp.path()),
                sink
            )
            .is_err());
        reg.close_all();
        assert_eq!(reg.count(), 0);
    }

    /// A shell that starts a background job, and the job's pid.
    fn start_with_background_job(
        reg: &Registry,
        chat: &str,
        dir: &Path,
    ) -> (Opened, i32, mpsc::Receiver<Event>) {
        let (sink, rx) = collecting();
        let opened = reg
            .open(
                chat,
                spec("/bin/sh", &["-c", "sleep 300 & echo JOB-$!-JOB; wait"], dir),
                sink,
            )
            .unwrap();
        let (text, _) = read_until(&rx, Duration::from_secs(10), |t| {
            t.contains("JOB-") && t.matches("-JOB").count() >= 1
        });
        let job = text
            .split("JOB-")
            .nth(1)
            .and_then(|rest| rest.split("-JOB").next())
            .and_then(|n| n.trim().parse::<i32>().ok())
            .unwrap_or_else(|| panic!("no job pid in {text:?}"));
        assert!(!pid_gone(job), "the job should be running");
        (opened, job, rx)
    }

    #[test]
    fn closing_a_terminal_ends_the_shell_and_what_it_started() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = Registry::default();
        let (opened, job, _rx) = start_with_background_job(&reg, "chat-a", tmp.path());
        reg.close(&opened.id);
        assert_eq!(reg.count(), 0);
        assert!(
            wait_gone(opened.pid as i32, Duration::from_secs(5)),
            "shell still alive"
        );
        assert!(
            wait_gone(job, Duration::from_secs(5)),
            "background job still alive"
        );
        // Closing again is harmless.
        reg.close(&opened.id);
    }

    #[test]
    fn closing_a_chat_ends_only_that_chats_terminals() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = Registry::default();
        let (a1, job_a, _rx_a) = start_with_background_job(&reg, "chat-a", tmp.path());
        let (b1, job_b, _rx_b) = start_with_background_job(&reg, "chat-b", tmp.path());
        assert_eq!(reg.close_chat("chat-a"), 1);
        assert!(wait_gone(a1.pid as i32, Duration::from_secs(5)));
        assert!(wait_gone(job_a, Duration::from_secs(5)));
        assert!(
            !pid_gone(b1.pid as i32) && !pid_gone(job_b),
            "chat-b's shell was ended"
        );
        assert_eq!(reg.count(), 1);
        reg.close_all();
        assert!(wait_gone(job_b, Duration::from_secs(5)));
    }

    #[test]
    fn a_chat_that_gets_a_key_keeps_its_terminals_under_the_new_key() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = Registry::default();
        let (opened, job, _rx) = start_with_background_job(&reg, "draft", tmp.path());
        reg.adopt_chat("draft", "s-123");
        assert_eq!(reg.close_chat("draft"), 0);
        assert!(!pid_gone(opened.pid as i32));
        assert_eq!(reg.close_chat("s-123"), 1);
        assert!(wait_gone(job, Duration::from_secs(5)));
    }

    #[test]
    fn a_shell_that_ends_by_itself_hangs_up_the_jobs_it_left_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let (sink, rx) = collecting();
        let reg = Registry::default();
        let opened = reg
            .open(
                "chat-a",
                spec(
                    "/bin/sh",
                    &["-c", "sleep 300 & echo JOB-$!-JOB; sleep 1"],
                    tmp.path(),
                ),
                sink,
            )
            .unwrap();
        let (text, exit) = read_until(&rx, Duration::from_secs(15), |t| t.contains("-JOB"));
        let job = text
            .split("JOB-")
            .nth(1)
            .and_then(|rest| rest.split("-JOB").next())
            .and_then(|n| n.trim().parse::<i32>().ok())
            .unwrap_or_else(|| panic!("no job pid in {text:?}"));
        // Wait for the shell's own end, then the hang-up reaches the job.
        let exit = exit.or_else(|| read_until(&rx, Duration::from_secs(15), |_| false).1);
        assert_eq!(exit, Some(Some(0)));
        assert!(
            wait_gone(job, Duration::from_secs(5)),
            "the job outlived its shell"
        );
        // Closing the ended shell sends nothing and just forgets it.
        reg.close(&opened.id);
        assert_eq!(reg.count(), 0);
    }

    #[test]
    fn a_job_that_ignores_hangup_is_killed_after_the_grace() {
        let tmp = tempfile::tempdir().unwrap();
        let (sink, rx) = collecting();
        let reg = Registry::default();
        let opened = reg
            .open(
                "chat-a",
                spec(
                    "/bin/sh",
                    &["-c", "trap '' HUP; echo READY; while :; do sleep 1; done"],
                    tmp.path(),
                ),
                sink,
            )
            .unwrap();
        let (text, _) = read_until(&rx, Duration::from_secs(10), |t| t.contains("READY"));
        assert!(text.contains("READY"), "{text}");
        reg.close(&opened.id);
        assert!(
            wait_gone(opened.pid as i32, Duration::from_secs(5)),
            "survived SIGHUP and SIGKILL"
        );
    }
}
