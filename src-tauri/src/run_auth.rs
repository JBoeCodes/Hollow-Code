//! Who may use Hollow's broker sockets, and where a run's MCP config lives
//! (merged plan slice 6: reviews A·S5 and B·F11, plus review A's "the
//! approval socket has no authentication").
//!
//! **The threat.** Every process running as this user can read every other
//! such process's argv (`ps -ww`; argv is in fact readable by every user on
//! macOS) and environment (`ps -E`), and can connect to a 0600 socket.
//! Before this slice a run's MCP config, broker proofs and library-connector
//! secrets included, rode on the agent CLI's argv (Claude's inline
//! `--mcp-config`, Codex's `-c mcp_servers=…`), so any process could copy a
//! stronger run's proof and replay it to the app, task or services broker,
//! and any process could post a fake card to the approval socket.
//!
//! **The transport.** [`write_config`] puts the run's MCP config in a 0600
//! file in a 0700 folder ([`config_dir`]), named for the run tag. Claude gets
//! `--mcp-config <file>`; Codex gets each stdio server as
//! `hollow mcp-launch <file> <name> <sha256>` ([`launch_command`]), which
//! reads that server's command, arguments and environment from the file
//! (refusing it when its digest no longer matches) and execs it,
//! and each HTTP connector's header values through `env_http_headers` from
//! its own environment. Argv carries file paths and names only. The file is
//! removed when the run's process group is settled or dropped ([`Binding`]),
//! and anything a crash left behind is swept after [`STALE_AFTER`].
//!
//! **The peer check.** A file mode only stops other users, and a running MCP
//! server still holds its proof in its environment. So a proof alone is no
//! longer enough: every broker (app, tasks, services) and the approval
//! socket asks the kernel who connected (`LOCAL_PEERPID` and `getpeereid`
//! on macOS, `SO_PEERCRED` on Linux) and walks that pid up its parents until
//! it reaches the leader of a run Hollow registered ([`Peers::runs_of`]). A
//! run is registered by `process_group::GroupGuard` when it adopts a child
//! whose command carries [`BOUND_RUNS_ENV`] (set by `agent::build_command`
//! for every GUI-owned run with an MCP config), with the run ids its servers
//! were minted for, and released when the guard settles or drops. A broker
//! request is served only when the peer is inside the tree of the run whose
//! id it names ([`check_peer`]); the approval socket needs the peer inside
//! any registered run. Everything else is refused with a reason on stderr.
//! A proof copied out of another run's environment is therefore worthless:
//! from outside every run it is refused, and from inside a different run it
//! names the wrong run.
//!
//! **What this does not cover.** Processes inside a run's own tree (its
//! Bash tool, its other servers) can still read and use that run's proofs:
//! that is the run's own authority, not an escalation. Library connector
//! secrets (a stdio connector's env, a Codex HTTP connector's header values)
//! are bearer credentials for outside services: any process of this user
//! can still read them from the run file or the processes' environment and
//! use them from anywhere. They are off argv, but not bound to the run; the
//! peer check only makes Hollow's own broker proofs worthless on replay. The
//! peer check defeats replaying a copied proof; it does not stop a process of this
//! user that can *write* this user's files: rewriting the run file before
//! Claude reads it, or editing the CLIs' own config and hooks, gets its code
//! started inside a run's tree with that run's authority. Only the Codex
//! path checks the file (the digest on `mcp-launch`'s argv). The leader's start
//! time guards against a reused leader pid, but the peer pid is read at
//! connect time, so a peer that exits and whose pid is reused before the
//! walk is a (narrow) race. A Codex HTTP connector's URL stays on argv
//! (Codex has no per-run way to read one from elsewhere), so a secret
//! embedded in a URL is still visible there; its header values are not.

use serde_json::Value;
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

/// Env var on a GUI-owned agent command (never read from a process): the
/// JSON array of run ids its MCP servers' proofs were minted for. The
/// process-group guard reads it from the `Command` it spawned.
pub const BOUND_RUNS_ENV: &str = "HOLLOW_BOUND_RUN_IDS";
/// The hidden subcommand Codex launches a stdio MCP server through.
pub const LAUNCH_SUBCOMMAND: &str = "mcp-launch";
/// Prefix of the Codex env vars that carry an HTTP connector's header
/// values. Contains SECRET, so Codex's default shell environment policy
/// keeps them out of the agent's shell commands.
pub const CODEX_HEADER_ENV_PREFIX: &str = "HOLLOW_MCP_SECRET_HEADER_";
/// Server env keys that name the run a proof was minted for.
const RUN_ID_KEYS: [&str; 2] = [
    crate::usage_ledger::ENV_RUN_ID,
    crate::services_mcp::ENV_RUN,
];
/// How far up its parents a peer is followed before giving up.
const MAX_ANCESTORS: usize = 64;
/// A config file older than this is from a run that never cleaned up (a
/// failed spawn, a crash) and is removed by the next write.
pub const STALE_AFTER: Duration = Duration::from_secs(2 * 24 * 60 * 60);
/// Largest config [`launch_command`] reads.
const CONFIG_MAX_BYTES: u64 = 4 * 1024 * 1024;

// ── Registered runs ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct LiveRun {
    leader: i32,
    /// The leader's start stamp, so a reused pid never passes as the run.
    start: Option<u64>,
    run_ids: Vec<String>,
}

impl LiveRun {
    /// Whether `pid` is this run's leader, and not a reuse of its pid.
    fn led_by(&self, pid: i32) -> bool {
        if self.leader != pid {
            return false;
        }
        match self.start {
            None => true,
            Some(start) => crate::proc_info::process_start_micros(pid) == Some(start),
        }
    }
}

/// The live runs whose process trees may use the brokers, keyed by run tag.
/// Its mutex is a leaf: it is held only to copy or change the map.
#[derive(Debug, Default)]
pub(crate) struct Peers {
    runs: Mutex<HashMap<String, LiveRun>>,
}

impl Peers {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A registry that lives for the rest of the process (tests hand one to
    /// a broker thread).
    #[cfg(test)]
    pub(crate) fn leaked() -> &'static Self {
        Box::leak(Box::new(Self::new()))
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, LiveRun>> {
        self.runs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Register the run led by `leader` (its process-group leader) under
    /// `tag`, allowed to act for `run_ids`.
    pub(crate) fn register(&self, tag: &str, leader: i32, run_ids: Vec<String>) {
        let start = crate::proc_info::process_start_micros(leader);
        self.lock().insert(
            tag.to_string(),
            LiveRun {
                leader,
                start,
                run_ids,
            },
        );
    }

    pub(crate) fn release(&self, tag: &str) {
        self.lock().remove(tag);
    }

    /// The run ids the process `peer` may act for: those of the registered
    /// run whose leader is `peer` or one of its ancestors. An error names
    /// why it may act for none.
    pub(crate) fn runs_of(&self, peer: i32) -> Result<Vec<String>, String> {
        let live: Vec<LiveRun> = self.lock().values().cloned().collect();
        if live.is_empty() {
            return Err(format!("pid {peer}: no run Hollow started is live"));
        }
        let mut pid = peer;
        for _ in 0..MAX_ANCESTORS {
            if let Some(run) = live.iter().find(|run| run.led_by(pid)) {
                return Ok(run.run_ids.clone());
            }
            match crate::proc_info::parent_pid(pid) {
                Some(parent) if parent != pid => pid = parent,
                _ => break,
            }
        }
        Err(format!("pid {peer} is not inside a run Hollow started"))
    }
}

/// The registry the production brokers check.
pub(crate) fn global() -> &'static Peers {
    static GLOBAL: OnceLock<Peers> = OnceLock::new();
    GLOBAL.get_or_init(Peers::new)
}

/// A registered run: dropping it releases the registration and removes the
/// run's config file. Held by the run's `process_group::GroupGuard`.
#[derive(Debug)]
pub(crate) struct Binding {
    tag: String,
    peers: &'static Peers,
    shared: Option<std::sync::Arc<crate::shared_mcp_host::Attempt>>,
}

impl Binding {
    pub(crate) fn revoke(&self) {
        if let Some(shared) = &self.shared {
            shared.revoke();
        }
    }
}

impl Drop for Binding {
    fn drop(&mut self) {
        self.revoke();
        self.peers.release(&self.tag);
        remove_config(&self.tag);
    }
}

/// Register a freshly spawned run in the global registry, when its command
/// was built with [`BOUND_RUNS_ENV`] (a GUI-owned run with an MCP config).
/// Any other command (a service, a probe, the installer) is never a run the
/// brokers serve.
pub(crate) fn adopt(cmd: &std::process::Command, tag: &str, leader: i32) -> Option<Binding> {
    adopt_into(global(), cmd, tag, leader)
}

pub(crate) fn adopt_into(
    peers: &'static Peers,
    cmd: &std::process::Command,
    tag: &str,
    leader: i32,
) -> Option<Binding> {
    let raw = cmd
        .get_envs()
        .find(|(key, _)| *key == OsStr::new(BOUND_RUNS_ENV))
        .and_then(|(_, value)| value)?;
    // Unreadable means no run ids: still a run (for the approval socket),
    // never a wider one.
    let run_ids: Vec<String> = serde_json::from_str(&raw.to_string_lossy()).unwrap_or_default();
    peers.register(tag, leader, run_ids);
    let shared = crate::shared_mcp_host::attempt(tag);
    if let Some(shared) = &shared {
        shared.activate();
    }
    Some(Binding {
        tag: tag.to_string(),
        peers,
        shared,
    })
}

/// The run ids a config's servers were minted for (`HOLLOW_RUN_ID`,
/// `HOLLOW_SERVICES_RUN`), as the JSON array [`BOUND_RUNS_ENV`] carries.
pub(crate) fn bound_runs_value(config: &str) -> String {
    Value::from(run_ids_in(config)).to_string()
}

fn run_ids_in(config: &str) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let Ok(value) = serde_json::from_str::<Value>(config) else {
        return ids;
    };
    let Some(servers) = value.get("mcpServers").and_then(Value::as_object) else {
        return ids;
    };
    for server in servers.values() {
        let Some(env) = server.get("env").and_then(Value::as_object) else {
            continue;
        };
        for key in RUN_ID_KEYS {
            let id = env
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty());
            if let Some(id) = id {
                if !ids.iter().any(|known| known == id) {
                    ids.push(id.to_string());
                }
            }
        }
    }
    ids
}

// ── The peer check ───────────────────────────────────────────────────────

/// The pid of the process on the other end of a connected Unix socket, after
/// checking it runs as this user.
#[cfg(target_os = "macos")]
pub(crate) fn peer_pid(fd: std::os::fd::RawFd) -> Result<i32, String> {
    // <sys/un.h>: SOL_LOCAL and LOCAL_PEERPID (the pid that last used the
    // peer's end: the client that connected and wrote its request).
    const SOL_LOCAL: libc::c_int = 0;
    const LOCAL_PEERPID: libc::c_int = 0x002;
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: `pid` is a pid_t-sized buffer and `len` says so.
    let rc = unsafe {
        libc::getsockopt(
            fd,
            SOL_LOCAL,
            LOCAL_PEERPID,
            &mut pid as *mut libc::pid_t as *mut libc::c_void,
            &mut len,
        )
    };
    if rc != 0 || pid <= 0 {
        return Err(format!(
            "its process could not be identified ({})",
            std::io::Error::last_os_error()
        ));
    }
    let (mut uid, mut gid): (libc::uid_t, libc::gid_t) = (0, 0);
    // SAFETY: both out-pointers are valid for writes.
    let rc = unsafe { libc::getpeereid(fd, &mut uid, &mut gid) };
    // SAFETY: geteuid has no preconditions.
    if rc != 0 || uid != unsafe { libc::geteuid() } {
        return Err(format!("pid {pid} runs as another user"));
    }
    Ok(pid)
}

#[cfg(target_os = "linux")]
pub(crate) fn peer_pid(fd: std::os::fd::RawFd) -> Result<i32, String> {
    // SAFETY: `ucred` is a plain C struct; all-zero is valid.
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `cred` is a ucred-sized buffer and `len` says so.
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        )
    };
    if rc != 0 || cred.pid <= 0 {
        return Err(format!(
            "its process could not be identified ({})",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: geteuid has no preconditions.
    if cred.uid != unsafe { libc::geteuid() } {
        return Err(format!("pid {} runs as another user", cred.pid));
    }
    Ok(cred.pid)
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
pub(crate) fn peer_pid(_fd: std::os::fd::RawFd) -> Result<i32, String> {
    Err("this platform cannot identify a socket's peer".into())
}

/// Admit one broker connection, or refuse it with a logged reason. With
/// `run_id`, the peer must be inside the tree of the registered run with
/// that id (the run its proof names); without, inside any registered run
/// (the approval socket, whose requests name no run).
#[cfg(unix)]
pub(crate) fn check_peer(
    peers: &Peers,
    fd: std::os::fd::RawFd,
    broker: &str,
    run_id: Option<&str>,
) -> Result<(), String> {
    let verdict = peer_pid(fd).and_then(|pid| {
        let runs = peers.runs_of(pid)?;
        match run_id.map(str::trim) {
            None => Ok(()),
            Some(id) if !id.is_empty() && runs.iter().any(|run| run == id) => Ok(()),
            Some(id) => Err(format!(
                "pid {pid} is not inside run {id:?}, so it may not use that run's proof"
            )),
        }
    });
    verdict.map_err(|reason| {
        eprintln!("[run auth] {broker} refused a connection: {reason}");
        format!(
            "Hollow refused this request: {reason}. Only the tools of the run Hollow started may use its {broker}."
        )
    })
}

// ── The run's config file ────────────────────────────────────────────────

/// Where run config files live: a 0700 folder in Hollow's cache (tests use
/// a per-process one in the temp dir, so two worktrees' runs never share it).
pub(crate) fn config_dir() -> PathBuf {
    #[cfg(test)]
    {
        crate::test_temp::process_dir("hollow-run-mcp-test-")
    }
    #[cfg(not(test))]
    {
        crate::instance::cache_dir().join("run-mcp")
    }
}

/// Whether `name` is a run tag (32 lowercase hex digits), so a file name
/// built from it stays inside [`config_dir`].
fn is_tag(name: &str) -> bool {
    name.len() == 32 && name.bytes().all(is_lower_hex)
}

fn is_lower_hex(b: u8) -> bool {
    b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
}

/// The run tag `process_group::with_run_tag` put in the config's servers.
fn tag_in(config: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(config).ok()?;
    value
        .get("mcpServers")?
        .as_object()?
        .values()
        .filter_map(|server| server.get("env")?.get(crate::process_group::RUN_TAG_ENV))
        .filter_map(Value::as_str)
        .find(|tag| is_tag(tag))
        .map(str::to_string)
}

/// Write `config` to a fresh 0600 file in [`config_dir`] and return its
/// path. Named for the run tag in the config (a fresh random name when there
/// is none), so the run's [`Binding`] can remove it.
#[cfg(test)]
pub(crate) fn write_config(config: &str) -> Result<PathBuf, String> {
    write_config_as(config, None)
}

pub(crate) fn write_config_as(config: &str, tag: Option<&str>) -> Result<PathBuf, String> {
    use std::io::Write;
    let dir = config_dir();
    ensure_private_dir(&dir)?;
    sweep_stale(&dir, STALE_AFTER);
    let name = match tag
        .filter(|s| is_tag(s))
        .map(str::to_string)
        .or_else(|| tag_in(config))
    {
        Some(tag) => tag,
        None => crate::process_group::RunTag::new().as_str().to_string(),
    };
    let path = dir.join(format!("{name}.json"));
    let _ = std::fs::remove_file(&path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let fail = |error: std::io::Error| {
        format!("Hollow couldn't write this run's private tool configuration ({error}).")
    };
    let mut file = options.open(&path).map_err(fail)?;
    if let Err(error) = file.write_all(config.as_bytes()).and_then(|_| file.flush()) {
        let _ = std::fs::remove_file(&path);
        return Err(fail(error));
    }
    Ok(path)
}

/// Remove the config file of the run tagged `tag`, if there is one.
pub(crate) fn remove_config(tag: &str) {
    if is_tag(tag) {
        let _ = std::fs::remove_file(config_dir().join(format!("{tag}.json")));
    }
}

/// Create `dir` if needed and make sure it is a real folder only this user
/// can enter.
fn ensure_private_dir(dir: &Path) -> Result<(), String> {
    let fail = |what: String| {
        format!("Hollow couldn't prepare a private folder for this run's tools ({what}).")
    };
    std::fs::create_dir_all(dir).map_err(|e| fail(e.to_string()))?;
    let meta = std::fs::symlink_metadata(dir).map_err(|e| fail(e.to_string()))?;
    if !meta.is_dir() {
        return Err(fail(format!("{} is not a folder", dir.display())));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // SAFETY: geteuid has no preconditions.
        if meta.uid() != unsafe { libc::geteuid() } {
            return Err(fail(format!("{} belongs to another user", dir.display())));
        }
        if meta.mode() & 0o077 != 0 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| fail(e.to_string()))?;
        }
    }
    Ok(())
}

/// Remove config files last changed more than `age` ago.
fn sweep_stale(dir: &Path, age: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension() != Some(OsStr::new("json")) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|elapsed| elapsed > age);
        if old {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Read a run config file, refusing one another user could have written or
/// can read, a symlink, or one too large to be Hollow's.
fn read_private(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .map_err(|e| format!("cannot open {} ({e})", path.display()))?;
    let meta = file
        .metadata()
        .map_err(|e| format!("cannot read {} ({e})", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid has no preconditions.
        if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
            return Err(format!(
                "{} is not a private Hollow run file (owner or mode)",
                path.display()
            ));
        }
    }
    if meta.len() > CONFIG_MAX_BYTES {
        return Err(format!("{} is too large", path.display()));
    }
    let mut text = String::new();
    file.take(CONFIG_MAX_BYTES)
        .read_to_string(&mut text)
        .map_err(|e| format!("cannot read {} ({e})", path.display()))?;
    Ok(text)
}

// ── `hollow mcp-launch <file> <server>` ──────────────────────────────────

/// The SHA-256 (lower-case hex) of a run config's text. Codex's
/// `mcp-launch` arguments carry it, so [`launch_command`] refuses a file
/// rewritten after Hollow wrote it. Not a secret: the proofs inside are
/// random, so the digest reveals nothing about them.
pub(crate) fn config_digest(config: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(config.as_bytes()))
}

/// The command `hollow mcp-launch <file> <server> <digest>` execs: the named
/// server's `command`, `args` and `env` from the run's config file, its
/// environment layered over the one Codex gave the launcher. A file whose
/// [`config_digest`] is not `<digest>` (rewritten since Hollow wrote it, for
/// example by another process of this user) is refused.
pub(crate) fn launch_command(args: &[OsString]) -> Result<std::process::Command, String> {
    let [file, name, digest] = args else {
        return Err(format!(
            "usage: hollow {LAUNCH_SUBCOMMAND} <config-file> <server> <sha256>"
        ));
    };
    let text = read_private(Path::new(file))?;
    if config_digest(&text) != digest.to_string_lossy() {
        return Err(format!(
            "{} changed after Hollow wrote it; refusing to start a server from it",
            Path::new(file).display()
        ));
    }
    let value = serde_json::from_str::<Value>(&text);
    let value = value.map_err(|e| format!("unreadable run config ({e})"))?;
    let name = name.to_string_lossy();
    let server = value
        .get("mcpServers")
        .and_then(|servers| servers.get(&*name))
        .and_then(Value::as_object)
        .ok_or_else(|| format!("no server {name:?} in this run's config"))?;
    let program = server
        .get("command")
        .and_then(Value::as_str)
        .filter(|command| !command.is_empty())
        .ok_or_else(|| format!("server {name:?} has no command"))?;
    let mut cmd = std::process::Command::new(program);
    if let Some(list) = server.get("args") {
        let Some(list) = list.as_array() else {
            return Err(format!("server {name:?} has unreadable arguments"));
        };
        for arg in list {
            let Some(arg) = arg.as_str() else {
                return Err(format!("server {name:?} has a non-text argument"));
            };
            cmd.arg(arg);
        }
    }
    if let Some(env) = server.get("env") {
        let Some(env) = env.as_object() else {
            return Err(format!("server {name:?} has an unreadable environment"));
        };
        for (key, value) in env {
            let Some(value) = value.as_str() else {
                return Err(format!("server {name:?} has a non-text value for {key}"));
            };
            cmd.env(key, value);
        }
    }
    Ok(cmd)
}

/// Entry point for `hollow mcp-launch`: exec the server, or print why not
/// and exit non-zero (Codex reports the server as failed to start).
pub fn launch_main() -> i32 {
    let args: Vec<OsString> = std::env::args_os().skip(2).collect();
    match launch_command(&args) {
        #[cfg(unix)]
        Ok(mut cmd) => {
            use std::os::unix::process::CommandExt;
            let error = cmd.exec();
            eprintln!("[{LAUNCH_SUBCOMMAND}] could not start the server: {error}");
            127
        }
        #[cfg(not(unix))]
        Ok(mut cmd) => match cmd.status() {
            Ok(status) => status.code().unwrap_or(1),
            Err(error) => {
                eprintln!("[{LAUNCH_SUBCOMMAND}] could not start the server: {error}");
                127
            }
        },
        Err(error) => {
            eprintln!("[{LAUNCH_SUBCOMMAND}] {error}");
            2
        }
    }
}

/// The `--mcp-config` value a Claude command was given, as JSON: the
/// inline empty config, or the run file's contents (test only).
#[cfg(test)]
pub(crate) fn config_from_arg(arg: &str) -> Option<Value> {
    if arg.trim_start().starts_with('{') {
        return serde_json::from_str(arg).ok();
    }
    serde_json::from_str(&read_private(Path::new(arg)).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn own_pid() -> i32 {
        std::process::id() as i32
    }

    /// A child shell with a grandchild: the shell (a run leader in these
    /// tests), its pid, and the grandchild's pid (a process in its tree).
    #[cfg(unix)]
    fn tree() -> (std::process::Child, i32, i32) {
        use std::io::{BufRead, BufReader};
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", "/bin/sleep 30 & echo $!; wait"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("spawn a shell");
        let mut line = String::new();
        let stdout = child.stdout.as_mut().expect("piped stdout");
        BufReader::new(stdout).read_line(&mut line).unwrap();
        let grandchild: i32 = line.trim().parse().expect("the grandchild's pid");
        let leader = child.id() as i32;
        (child, leader, grandchild)
    }

    #[cfg(unix)]
    fn end(mut child: std::process::Child, grandchild: i32) {
        // SAFETY: a plain signal to a process this test started.
        unsafe {
            libc::kill(grandchild, libc::SIGKILL);
        }
        let _ = child.kill();
        let _ = child.wait();
    }

    /// A process is inside a run when the run's leader is it or one of its
    /// ancestors; this test process (the leader's parent) is not.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn only_processes_under_a_registered_leader_belong_to_its_run() {
        let peers = Peers::new();
        let (child, leader, grandchild) = tree();
        assert!(peers.runs_of(grandchild).is_err(), "nothing registered yet");
        peers.register("t1", leader, vec!["run-a".into()]);
        assert_eq!(peers.runs_of(grandchild).unwrap(), ["run-a"]);
        assert_eq!(peers.runs_of(leader).unwrap(), ["run-a"]);
        let outside = peers.runs_of(own_pid()).unwrap_err();
        assert!(outside.contains("not inside a run"), "{outside}");
        peers.release("t1");
        assert!(peers.runs_of(grandchild).is_err(), "released with the run");
        end(child, grandchild);
    }

    /// A leader pid that now belongs to another process (a different start
    /// time) is not the run.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn a_reused_leader_pid_is_not_the_run() {
        let peers = Peers::new();
        peers.register("t2", own_pid(), vec!["run-a".into()]);
        assert!(peers.runs_of(own_pid()).is_ok());
        peers.lock().get_mut("t2").unwrap().start = Some(1);
        assert!(peers.runs_of(own_pid()).is_err());
    }

    /// The broker gate over a real socket: refused from outside every run,
    /// refused for another run's id (a replayed proof), admitted for the
    /// run the peer belongs to.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn check_peer_admits_only_the_named_run_tree() {
        use std::os::fd::AsRawFd;
        let (server, _client) = std::os::unix::net::UnixStream::pair().unwrap();
        let fd = server.as_raw_fd();
        assert_eq!(peer_pid(fd).unwrap(), own_pid(), "a pair's peer is us");

        let empty = Peers::new();
        assert!(check_peer(&empty, fd, "test broker", None).is_err());
        assert!(check_peer(&empty, fd, "test broker", Some("run-a")).is_err());

        let peers = Peers::new();
        peers.register("t3", own_pid(), vec!["run-a".into()]);
        assert!(check_peer(&peers, fd, "test broker", None).is_ok());
        assert!(check_peer(&peers, fd, "test broker", Some("run-a")).is_ok());
        let replay = check_peer(&peers, fd, "test broker", Some("run-b")).unwrap_err();
        assert!(replay.contains("run-b"), "{replay}");
        assert!(check_peer(&peers, fd, "test broker", Some("")).is_err());
    }

    #[test]
    fn bound_run_ids_come_from_every_server_and_dedupe() {
        let value = json!({"mcpServers": {
            "hollow_app": {"command": "h", "env": {"HOLLOW_RUN_ID": "r1", "HOLLOW_APP_PROOF": "p"}},
            "hollow_tasks": {"command": "h", "env": {"HOLLOW_RUN_ID": "r1"}},
            "hollow_services": {"command": "h", "env": {"HOLLOW_SERVICES_RUN": "r2"}},
            "hollow_vault": {"command": "h"}
        }});
        let bound = bound_runs_value(&value.to_string());
        let ids: Vec<String> = serde_json::from_str(&bound).unwrap();
        assert_eq!(ids, ["r1", "r2"]);
        assert_eq!(bound_runs_value("not json"), "[]");
    }

    #[test]
    fn adoption_registers_only_bound_commands_and_the_binding_cleans_up() {
        let peers = Peers::leaked();
        let tag = crate::process_group::RunTag::new();
        let value = json!({"mcpServers": {"hollow_app": {"command": "h", "env": {
            "HOLLOW_RUN_ID": "r1",
            "HOLLOW_RUN_TAG": tag.as_str(),
        }}}});
        let config = value.to_string();
        let file = write_config(&config).unwrap();
        assert_eq!(file, config_dir().join(format!("{}.json", tag.as_str())));

        let plain = std::process::Command::new("/bin/true");
        assert!(adopt_into(peers, &plain, tag.as_str(), own_pid()).is_none());
        assert!(peers.runs_of(own_pid()).is_err());

        let mut bound = std::process::Command::new("/bin/true");
        bound.env(BOUND_RUNS_ENV, bound_runs_value(&config));
        let binding = adopt_into(peers, &bound, tag.as_str(), own_pid()).unwrap();
        assert_eq!(peers.runs_of(own_pid()).unwrap(), ["r1"]);
        assert!(file.exists());
        drop(binding);
        assert!(peers.runs_of(own_pid()).is_err(), "released");
        assert!(!file.exists(), "the run's config file goes with it");
    }

    /// The process-group guard registers a bound run while it lives, and
    /// settling it releases the registration and removes the run's file.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[tokio::test]
    async fn a_guarded_run_is_registered_until_it_settles() {
        use crate::process_group::{GroupGuard, RunTag, Timings};
        let tag = RunTag::new();
        let value = json!({"mcpServers": {"hollow_app": {"command": "h", "env": {
            "HOLLOW_RUN_ID": "run-guarded",
            "HOLLOW_RUN_TAG": tag.as_str(),
        }}}});
        let config = value.to_string();
        let file = write_config(&config).unwrap();
        let mut cmd = tokio::process::Command::new("/bin/sleep");
        cmd.arg("30");
        crate::process_group::isolate_with(&mut cmd, &tag);
        cmd.env(BOUND_RUNS_ENV, bound_runs_value(&config));
        let mut child = cmd.spawn().unwrap();
        let mut guard = GroupGuard::adopt_unrecorded(&cmd, &child);
        let pid = child.id().unwrap() as i32;
        assert_eq!(global().runs_of(pid).unwrap(), ["run-guarded"]);
        let _ = guard.settle(&mut child, &Timings::fast_for_tests()).await;
        assert!(!global().lock().contains_key(tag.as_str()), "released");
        assert!(!file.exists(), "the run's config file is removed");
    }

    #[cfg(unix)]
    #[test]
    fn the_config_file_is_private_and_named_for_the_run() {
        use std::os::unix::fs::PermissionsExt;
        let config = r#"{"mcpServers":{"x":{"command":"c","env":{"HOLLOW_APP_PROOF":"sp"}}}}"#;
        let file = write_config(config).unwrap();
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let dir = std::fs::metadata(config_dir())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(dir & 0o077, 0, "only this user can enter the folder");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), config);
        let stem = file.file_stem().unwrap().to_string_lossy().into_owned();
        assert!(is_tag(&stem), "a fresh random name without a run tag");
        let read = config_from_arg(&file.to_string_lossy()).unwrap();
        assert_eq!(read["mcpServers"]["x"]["command"], json!("c"));
        remove_config(&stem);
        assert!(!file.exists());
        // Never a path outside the folder.
        remove_config("../../etc/passwd");
    }

    #[test]
    fn stale_files_are_swept() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.json");
        std::fs::write(&old, "{}").unwrap();
        let keep = dir.path().join("keep.txt");
        std::fs::write(&keep, "").unwrap();
        sweep_stale(dir.path(), STALE_AFTER);
        assert!(old.exists(), "young enough to keep");
        std::thread::sleep(Duration::from_millis(20));
        sweep_stale(dir.path(), Duration::from_millis(1));
        assert!(!old.exists());
        assert!(keep.exists(), "only run config files");
    }

    #[test]
    fn the_launcher_subcommand_is_dispatched() {
        let main = include_str!("main.rs");
        let needle =
            format!("Some(\"{LAUNCH_SUBCOMMAND}\") => return hollow_lib::run_mcp_launch()");
        assert!(crate::source_pins::contains(main, &needle));
    }

    /// `hollow mcp-launch` execs exactly the server the run file describes,
    /// and refuses a file anyone else can read.
    #[cfg(unix)]
    #[test]
    fn the_launcher_runs_the_named_server_from_a_private_file_only() {
        use std::os::unix::fs::PermissionsExt;
        let value = json!({"mcpServers": {
            "hollow_app": {"command": "/x/hollow", "args": ["app-mcp"], "env": {"HOLLOW_APP_PROOF": "p1"}},
            "linear": {"command": "npx", "args": ["-y", "linear"], "env": {"API_KEY": "k"}}
        }});
        let text = value.to_string();
        let file = write_config(&text).unwrap();
        let digest = OsString::from(config_digest(&text));
        let args = |name: &str| {
            let name = OsString::from(name);
            vec![file.clone().into_os_string(), name, digest.clone()]
        };
        let cmd = launch_command(&args("linear")).unwrap();
        assert_eq!(cmd.get_program(), "npx");
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), ["-y", "linear"]);
        let envs: Vec<_> = cmd.get_envs().collect();
        assert_eq!(envs, [(OsStr::new("API_KEY"), Some(OsStr::new("k")))]);
        let app = launch_command(&args("hollow_app")).unwrap();
        assert_eq!(app.get_program(), "/x/hollow");
        assert!(launch_command(&args("missing")).is_err());
        assert!(launch_command(&[file.clone().into_os_string()]).is_err());
        let undigested = [file.clone().into_os_string(), OsString::from("linear")];
        assert!(launch_command(&undigested).is_err());

        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        let refused = launch_command(&args("linear")).unwrap_err();
        assert!(refused.contains("not a private"), "{refused}");
        let _ = std::fs::remove_file(&file);
    }

    /// A same-user process that rewrites a run's file before Codex starts a
    /// server (same owner, same 0600 mode) does not get its own program run
    /// inside the run's tree: the digest on Codex's argv no longer matches.
    #[cfg(unix)]
    #[test]
    fn the_launcher_refuses_a_run_file_rewritten_after_hollow_wrote_it() {
        use std::io::Write;
        let text = json!({"mcpServers": {
            "hollow_app": {"command": "/x/hollow", "args": ["app-mcp"], "env": {"HOLLOW_APP_PROOF": "p2"}}
        }})
        .to_string();
        let file = write_config(&text).unwrap();
        let args = [
            file.clone().into_os_string(),
            OsString::from("hollow_app"),
            OsString::from(config_digest(&text)),
        ];
        assert_eq!(launch_command(&args).unwrap().get_program(), "/x/hollow");

        let forged = text.replace("/x/hollow", "/tmp/evil");
        let mut handle = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&file)
            .unwrap();
        handle.write_all(forged.as_bytes()).unwrap();
        drop(handle);
        let refused = launch_command(&args).unwrap_err();
        assert!(
            refused.contains("changed after Hollow wrote it"),
            "{refused}"
        );
        let _ = std::fs::remove_file(&file);
    }
}
