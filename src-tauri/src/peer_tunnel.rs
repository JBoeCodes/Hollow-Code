//! The SSH tunnel to a paired Mac (M2 PR 2, `docs/phone-api/machines.md`).
//!
//! One `ssh -N -L <socket>:127.0.0.1:<port> <alias>` per paired Mac, opened on
//! demand and never more than one: the other Mac's Hollow answers on its own
//! loopback, so the request lands there as a local one and is judged by its
//! token. The ssh options are `remote_session::SSH_OPTS` (no agent or X11
//! forwarding, never a prompt, a bounded connect, a dead link noticed within
//! a minute, no shared master) plus `ExitOnForwardFailure`,
//! `StreamLocalBindUnlink` and `PermitLocalCommand=no`. Its arguments are the
//! alias, the socket path and the port: nothing secret.
//!
//! **This Mac's end is a Unix socket**, `$TMPDIR/hollow-peer/<machineId>.sock`
//! in a 0700 folder this user owns ([`socket_dir`]): no free-port race, no
//! clash with this Mac's own 47680, and no other user on this Mac can reach
//! it. Its path must fit `sun_path` (104 bytes).
//!
//! **Opening** ([`Tunnel::ensure`]): an earlier socket file is removed, ssh
//! starts, and the socket is polled every 100 ms until it accepts, for at
//! most [`READY_WITHIN`] (ssh's own 10-second connect timeout plus margin).
//! An ssh that exits first failed (`ExitOnForwardFailure` makes a failed bind
//! exit at once); its stderr's last line says why. The aliases are tried in
//! order; when all fail, the next try waits up to 2, 5, 15, 30, then 60
//! seconds ([`BACKOFF`], each wait jittered between half the step and the
//! whole step: PL1, `peer_link`), reset by a success. A tunnel whose ssh died is opened again
//! by the next `ensure`.
//!
//! **Lifetime.** The ssh leads its own process group and carries a run tag
//! (`process_group::isolate`), and its guard records it in the run ledger
//! like every other child Hollow starts: dropping the tunnel (or
//! [`Tunnel::close`]) kills the group, Hollow's quit and quit signals settle
//! it, the crash reaper and the next launch's sweep end one a dead Hollow
//! left. Nothing is ever killed by name. [`Tunnel::close_if_idle`] closes
//! one nobody used for [`IDLE_CLOSE`].
//!
//! **Cancel** ([`Tunnel::stop_opening`], the strip's Cancel while a Mac
//! connects): an opening in progress ends its ssh within one poll, and no
//! new opening starts until the person asks again ([`Tunnel::retry_now`]:
//! Retry, Check now, an alias change or a new pairing). A tunnel that is up
//! stays up.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::time::Instant;

/// How long a new ssh may take to accept on its socket.
pub(crate) const READY_WITHIN: Duration = Duration::from_secs(12);
/// Waits before trying again after every alias failed, in seconds.
pub(crate) const BACKOFF: [u64; 5] = [2, 5, 15, 30, 60];
/// A tunnel nobody used for this long may be closed.
pub(crate) const IDLE_CLOSE: Duration = Duration::from_secs(10 * 60);
const POLL: Duration = Duration::from_millis(100);
/// `sun_path` is 104 bytes with its terminating NUL.
const SUN_PATH_MAX: usize = 103;
/// ssh's stderr kept, in bytes.
const TAIL_BYTES: usize = 2048;

/// Starts the ssh child (a test stand-in in tests).
pub(crate) trait Spawner: Send + Sync + 'static {
    fn open(
        &self,
        alias: &str,
        sock: &Path,
        remote_port: u16,
    ) -> Result<Box<dyn TunnelChild>, String>;
}

/// A running ssh. Dropping it ends it.
pub(crate) trait TunnelChild: Send {
    fn alive(&mut self) -> bool;
    /// The end of what it wrote to stderr.
    fn stderr_tail(&self) -> String;
    #[cfg_attr(not(test), allow(dead_code))]
    fn pid(&self) -> Option<u32>;
}

/// Why a tunnel isn't up. Every message is a plain sentence.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TunnelError {
    /// No SSH alias to try.
    NoAlias,
    /// The socket folder or path can't be used.
    Socket(String),
    /// ssh couldn't be started at all.
    Spawn(String),
    /// ssh exited: it couldn't connect, log in or forward.
    Unreachable { alias: String, ssh_error: String },
    /// ssh ran but its socket never accepted.
    Timeout { alias: String },
    /// Every alias failed a moment ago; the next try waits.
    Waiting {
        last: Box<TunnelError>,
        retry_in: Duration,
    },
    /// The person cancelled connecting; nothing opens until they retry.
    Stopped,
}

impl std::fmt::Display for TunnelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoAlias => f.write_str("No SSH name is set for that Mac. Add one in Settings."),
            Self::Socket(why) => write!(
                f,
                "Hollow couldn't make the link's socket on this Mac: {why}."
            ),
            Self::Spawn(why) => write!(f, "ssh couldn't start on this Mac ({why})."),
            Self::Unreachable { alias, ssh_error } => {
                let last = crate::peer_client::remote_text(ssh_error.lines().last().unwrap_or(""));
                let last = last.trim_end_matches('.');
                let causes = crate::remote_status::likely_causes(last);
                if last.is_empty() {
                    write!(f, "This Mac couldn't reach {alias} over SSH. {causes}")
                } else {
                    write!(
                        f,
                        "This Mac couldn't reach {alias} over SSH: {last}. {causes}"
                    )
                }
            }
            Self::Timeout { alias } => write!(
                f,
                "{alias} didn't answer over SSH within {} seconds.",
                READY_WITHIN.as_secs()
            ),
            Self::Waiting { last, retry_in } => write!(
                f,
                "{last} Hollow tries again in {} seconds.",
                retry_in.as_millis().div_ceil(1000).max(1)
            ),
            Self::Stopped => f.write_str("Connecting was cancelled. Retry connects again."),
        }
    }
}

/// The real ssh.
pub(crate) struct SshSpawner {
    program: OsString,
}

impl SshSpawner {
    pub(crate) fn new() -> Self {
        Self::with_program("ssh")
    }

    /// Another program in ssh's place (tests: a script that stands in).
    pub(crate) fn with_program(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
        }
    }
}

/// ssh's arguments for one tunnel: the hardened options, then the forward.
pub(crate) fn ssh_args(alias: &str, sock: &Path, remote_port: u16) -> Vec<OsString> {
    let mut args: Vec<OsString> = crate::remote_session::SSH_OPTS
        .iter()
        .map(OsString::from)
        .collect();
    for extra in [
        "-N",
        "-T",
        "-o",
        "ExitOnForwardFailure=yes",
        "-o",
        "StreamLocalBindUnlink=yes",
        "-o",
        "PermitLocalCommand=no",
        "-L",
    ] {
        args.push(extra.into());
    }
    let mut forward = sock.as_os_str().to_owned();
    forward.push(format!(":127.0.0.1:{remote_port}"));
    args.push(forward);
    // Never an option: `valid_host` refuses a leading dash, and `--` ends
    // ssh's options anyway.
    args.push("--".into());
    args.push(alias.into());
    args
}

struct SshChild {
    child: tokio::process::Child,
    _guard: crate::process_group::GroupGuard,
    tail: Arc<Mutex<VecDeque<u8>>>,
}

impl TunnelChild for SshChild {
    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
    fn stderr_tail(&self) -> String {
        let tail = self.tail.lock().unwrap_or_else(PoisonError::into_inner);
        let bytes: Vec<u8> = tail.iter().copied().collect();
        String::from_utf8_lossy(&bytes).trim().to_string()
    }
    fn pid(&self) -> Option<u32> {
        self.child.id()
    }
}

impl Drop for SshChild {
    fn drop(&mut self) {
        // The guard's drop kills the whole group; this is the direct child
        // for a run that couldn't be isolated.
        let _ = self.child.start_kill();
    }
}

impl Spawner for SshSpawner {
    fn open(
        &self,
        alias: &str,
        sock: &Path,
        remote_port: u16,
    ) -> Result<Box<dyn TunnelChild>, String> {
        use tokio::io::AsyncReadExt as _;
        if !crate::remote_session::valid_host(alias) {
            return Err(crate::peer_machines::host_refusal(alias));
        }
        let mut cmd = tokio::process::Command::new(&self.program);
        cmd.args(ssh_args(alias, sock, remote_port))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        crate::process_group::isolate(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        let guard = crate::process_group::GroupGuard::adopt(&cmd, &child);
        let tail = Arc::new(Mutex::new(VecDeque::new()));
        if let Some(mut stderr) = child.stderr.take() {
            let tail = tail.clone();
            tokio::spawn(async move {
                let mut chunk = [0u8; 1024];
                while let Ok(n) = stderr.read(&mut chunk).await {
                    if n == 0 {
                        break;
                    }
                    let mut tail = tail.lock().unwrap_or_else(PoisonError::into_inner);
                    tail.extend(&chunk[..n]);
                    while tail.len() > TAIL_BYTES {
                        tail.pop_front();
                    }
                }
            });
        }
        Ok(Box::new(SshChild {
            child,
            _guard: guard,
            tail,
        }))
    }
}

/// `$TMPDIR/hollow-peer`, made 0700 and checked to be this user's.
pub(crate) fn socket_dir() -> Result<PathBuf, TunnelError> {
    let dir = std::env::temp_dir().join("hollow-peer");
    private_dir(&dir)?;
    Ok(dir)
}

/// Make `dir` (0700) or check an existing one: a real folder of this user's,
/// which then gets 0700 again.
pub(crate) fn private_dir(dir: &Path) -> Result<(), TunnelError> {
    let bad = |why: String| TunnelError::Socket(why);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        match std::fs::DirBuilder::new().mode(0o700).create(dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(bad(format!("{} ({e})", dir.display()))),
        }
        let meta =
            std::fs::symlink_metadata(dir).map_err(|e| bad(format!("{} ({e})", dir.display())))?;
        // SAFETY: getuid has no preconditions and can't fail.
        let me = unsafe { libc::getuid() };
        if !meta.is_dir() || meta.uid() != me {
            return Err(bad(format!(
                "{} isn't a folder of this user's",
                dir.display()
            )));
        }
        if meta.permissions().mode() & 0o777 != 0o700 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| bad(format!("{} ({e})", dir.display())))?;
        }
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir).map_err(|e| bad(format!("{} ({e})", dir.display())))?;
    Ok(())
}

/// `<dir>/<machineId>.sock`, refused when it wouldn't fit `sun_path`.
pub(crate) fn socket_path(dir: &Path, machine_id: &str) -> Result<PathBuf, TunnelError> {
    if !crate::phone_access::valid_machine_id(machine_id) {
        return Err(TunnelError::Socket(
            "that Mac's id isn't one Hollow gives out".into(),
        ));
    }
    let path = dir.join(format!("{machine_id}.sock"));
    // `-L <socket>:127.0.0.1:<port>`: a colon in the path would split it.
    if path.as_os_str().to_string_lossy().contains(':') {
        return Err(TunnelError::Socket(format!(
            "{} has a colon, which ssh's forward can't take",
            path.display()
        )));
    }
    let len = path.as_os_str().len();
    if len > SUN_PATH_MAX {
        return Err(TunnelError::Socket(format!(
            "{} is {len} bytes, longer than the {SUN_PATH_MAX} a socket path may be",
            path.display()
        )));
    }
    Ok(path)
}

/// The wait after `failures` rounds in a row of every alias failing.
pub(crate) fn backoff(failures: u32) -> Duration {
    let i = (failures.max(1) as usize - 1).min(BACKOFF.len() - 1);
    Duration::from_secs(BACKOFF[i])
}

struct State {
    child: Option<Box<dyn TunnelChild>>,
    alias: Option<String>,
    failures: u32,
    last_error: Option<TunnelError>,
    /// When the running ssh came up (PL1: a retry closes only a link at
    /// least as old as the failed try).
    opened_at: Option<Instant>,
}

/// One paired Mac's tunnel.
pub(crate) struct Tunnel {
    machine_id: String,
    hosts: Mutex<Vec<String>>,
    remote_port: u16,
    sock: PathBuf,
    spawner: Arc<dyn Spawner>,
    ready_within: Duration,
    /// Its last use: an `ensure`, or a frame read through it ([`Tunnel::touch`]).
    last_used: Mutex<Instant>,
    /// When the next try may start after every alias failed. Apart from
    /// `state` (which an opening holds for up to 12 s per alias) so clearing
    /// it ([`Tunnel::retry_now`]) never waits.
    retry_at: Mutex<Option<Instant>>,
    /// Cancel was asked ([`Tunnel::stop_opening`]); cleared by
    /// [`Tunnel::retry_now`] and by an opening that succeeded anyway. Apart
    /// from `state` too, so Cancel never waits for the opening it ends.
    stopped: AtomicBool,
    /// Draws each backoff wait (PL1).
    jitter: crate::peer_link::Jitter,
    /// The one retry owner (PL1): an opening holds it, and every caller that
    /// finds the link down waits for that opening instead of starting its own.
    state: tokio::sync::Mutex<State>,
}

impl Tunnel {
    /// A closed tunnel to `machine_id` through `hosts`, its socket in `dir`
    /// (made 0700, or checked to be this user's, here).
    pub(crate) fn new(
        machine_id: &str,
        hosts: Vec<String>,
        remote_port: u16,
        dir: &Path,
        spawner: Arc<dyn Spawner>,
    ) -> Result<Self, TunnelError> {
        private_dir(dir)?;
        let sock = socket_path(dir, machine_id)?;
        Ok(Self {
            machine_id: machine_id.to_string(),
            hosts: Mutex::new(hosts),
            remote_port,
            sock,
            spawner,
            ready_within: READY_WITHIN,
            last_used: Mutex::new(Instant::now()),
            retry_at: Mutex::new(None),
            stopped: AtomicBool::new(false),
            jitter: crate::peer_link::Jitter::random(),
            state: tokio::sync::Mutex::new(State {
                child: None,
                alias: None,
                failures: 0,
                last_error: None,
                opened_at: None,
            }),
        })
    }

    /// A shorter readiness wait (tests).
    #[cfg(test)]
    pub(crate) fn with_ready_within(mut self, within: Duration) -> Self {
        self.ready_within = within;
        self
    }

    /// A fixed backoff draw (tests).
    #[cfg(test)]
    pub(crate) fn with_jitter(mut self, jitter: crate::peer_link::Jitter) -> Self {
        self.jitter = jitter;
        self
    }

    #[cfg(test)]
    pub(crate) fn socket(&self) -> &Path {
        &self.sock
    }

    pub(crate) fn remote_port(&self) -> u16 {
        self.remote_port
    }

    /// Note a use of the open socket (an event stream reads a frame), so a
    /// tunnel in use never looks idle.
    pub(crate) fn touch(&self) {
        *self
            .last_used
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }

    fn idle_for(&self) -> Duration {
        self.last_used
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .elapsed()
    }

    /// Its aliases, in the order they are tried.
    pub(crate) fn hosts(&self) -> Vec<String> {
        self.hosts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// New aliases, used from the next opening on.
    pub(crate) fn set_hosts(&self, hosts: Vec<String>) {
        *self.hosts.lock().unwrap_or_else(PoisonError::into_inner) = hosts;
    }

    /// The socket of a tunnel that is up: the running one, else a new one
    /// through the first alias that works. Callers wait for one opening.
    pub(crate) async fn ensure(&self) -> Result<PathBuf, TunnelError> {
        self.touch();
        let mut state = self.state.lock().await;
        if let Some(child) = state.child.as_mut() {
            if child.alive() {
                // A Cancel that reached a tunnel already up (it raced the
                // check in `stop_opening`) must not outlive it: a later
                // drop would read as cancelled.
                self.stopped.store(false, Ordering::SeqCst);
                return Ok(self.sock.clone());
            }
            let tail = child.stderr_tail();
            crate::launch::elog(&format!(
                "[peer] the link to {} closed; opening it again{}",
                self.machine_id,
                if tail.is_empty() {
                    String::new()
                } else {
                    format!(
                        " ({})",
                        crate::peer_client::remote_text(tail.lines().last().unwrap_or(""))
                    )
                }
            ));
            state.child = None;
            state.alias = None;
        }
        if self.stopped() {
            return Err(TunnelError::Stopped);
        }
        let retry_at = *self.retry_at.lock().unwrap_or_else(PoisonError::into_inner);
        if let (Some(at), Some(last)) = (retry_at, state.last_error.clone()) {
            let now = Instant::now();
            if at > now {
                return Err(TunnelError::Waiting {
                    last: Box::new(last),
                    retry_in: at - now,
                });
            }
        }
        let hosts = self
            .hosts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if hosts.is_empty() {
            return Err(TunnelError::NoAlias);
        }
        let mut last = TunnelError::NoAlias;
        for alias in &hosts {
            match self.open(alias).await {
                Ok(child) => {
                    state.child = Some(child);
                    state.alias = Some(alias.clone());
                    state.opened_at = Some(Instant::now());
                    state.failures = 0;
                    *self.retry_at.lock().unwrap_or_else(PoisonError::into_inner) = None;
                    state.last_error = None;
                    // A Cancel that came too late: the tunnel is up, and a
                    // later drop must not read as cancelled.
                    self.stopped.store(false, Ordering::SeqCst);
                    return Ok(self.sock.clone());
                }
                // Cancelled: no failure counted, no backoff, no next alias.
                Err(TunnelError::Stopped) => return Err(TunnelError::Stopped),
                Err(e) => last = e,
            }
        }
        state.failures += 1;
        *self.retry_at.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(Instant::now() + self.jitter.wait(backoff(state.failures)));
        state.last_error = Some(last.clone());
        Err(last)
    }

    async fn open(&self, alias: &str) -> Result<Box<dyn TunnelChild>, TunnelError> {
        // An earlier socket file (a dead ssh's) must not be taken for this
        // one's.
        let _ = std::fs::remove_file(&self.sock);
        let mut child = self
            .spawner
            .open(alias, &self.sock, self.remote_port)
            .map_err(TunnelError::Spawn)?;
        let deadline = Instant::now() + self.ready_within;
        loop {
            if self.stopped() {
                drop(child);
                let _ = std::fs::remove_file(&self.sock);
                return Err(TunnelError::Stopped);
            }
            if !child.alive() {
                return Err(TunnelError::Unreachable {
                    alias: alias.to_string(),
                    ssh_error: child.stderr_tail(),
                });
            }
            if tokio::net::UnixStream::connect(&self.sock).await.is_ok() {
                return Ok(child);
            }
            if Instant::now() >= deadline {
                drop(child);
                let _ = std::fs::remove_file(&self.sock);
                return Err(TunnelError::Timeout {
                    alias: alias.to_string(),
                });
            }
            tokio::time::sleep(POLL).await;
        }
    }

    /// The alias the running tunnel went through, when one is up (none while
    /// one opens; never waits for it).
    pub(crate) async fn alias(&self) -> Option<String> {
        // An opening in progress holds `state` for seconds: not up yet.
        let Ok(mut state) = self.state.try_lock() else {
            return None;
        };
        let alive = state.child.as_mut().is_some_and(|c| c.alive());
        alive.then(|| state.alias.clone()).flatten()
    }

    /// The running ssh's pid (tests).
    #[cfg(test)]
    pub(crate) async fn pid(&self) -> Option<u32> {
        self.state.lock().await.child.as_ref().and_then(|c| c.pid())
    }

    /// Try every alias at the next `ensure`, without waiting out the backoff
    /// (the person's Retry).
    /// Never waits for an opening in progress.
    pub(crate) fn retry_now(&self) {
        *self.retry_at.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.stopped.store(false, Ordering::SeqCst);
    }

    /// Cancel connecting: an opening in progress ends within one poll and no
    /// new one starts until [`Tunnel::retry_now`]. A tunnel that is up is
    /// left alone and not marked (its later drop or idle close reconnects as
    /// usual). Never waits for the opening: a busy `state` means one runs.
    pub(crate) fn stop_opening(&self) {
        if let Ok(mut state) = self.state.try_lock() {
            if state.child.as_mut().is_some_and(|c| c.alive()) {
                return;
            }
        }
        self.stopped.store(true, Ordering::SeqCst);
    }

    /// Whether Cancel holds it closed.
    pub(crate) fn stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    /// End the ssh now.
    pub(crate) async fn close(&self) {
        let mut state = self.state.lock().await;
        state.child = None;
        state.alias = None;
        let _ = std::fs::remove_file(&self.sock);
    }

    /// PL1: end a link that went quiet under a command. Closes the running
    /// ssh only when it came up no later than `since` (when the try failed),
    /// so a link another caller already opened again is left up.
    /// `true` when it closed. A quiet drop keeps ssh alive for up to a
    /// minute (`ServerAliveInterval`); without this, the retry would go over
    /// the same dead link.
    pub(crate) async fn close_if_opened_before(&self, since: Instant) -> bool {
        let mut state = self.state.lock().await;
        if state.child.is_none() || state.opened_at.is_some_and(|at| at > since) {
            return false;
        }
        state.child = None;
        state.alias = None;
        state.opened_at = None;
        let _ = std::fs::remove_file(&self.sock);
        true
    }

    /// Close it when nobody used it for `idle`. `true` when it closed.
    pub(crate) async fn close_if_idle(&self, idle: Duration) -> bool {
        let mut state = self.state.lock().await;
        if state.child.is_none() || self.idle_for() < idle {
            return false;
        }
        state.child = None;
        state.alias = None;
        let _ = std::fs::remove_file(&self.sock);
        true
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.try_lock() {
            state.child = None;
        }
        let _ = std::fs::remove_file(&self.sock);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A stand-in for ssh (a perl script in a temp folder): it logs its
    /// arguments, then by the alias's name fails like an unreachable host
    /// (`down`), never binds (`slow`), binds and exits after a second
    /// (`brief`), or binds the `-L` socket and forwards each connection to
    /// the TCP port named there. Without `-N` it prints `pair_line` (the
    /// `hollow serve pair --machine --json` answer), 0.8 s late for an
    /// alias with `lag` in it, and exits.
    pub(crate) fn fake_ssh(dir: &Path, pair_line: &str) -> PathBuf {
        let path = dir.join("ssh");
        let log = dir.join("ssh.args");
        let script = r#"#!/usr/bin/perl
use strict; use warnings;
use Socket qw(SOCK_STREAM);
use IO::Socket::UNIX; use IO::Socket::INET; use IO::Select;
my @a = @ARGV;
if (open(my $log, '>>', '__LOG__')) { print $log join(' ', @a), "\n"; close $log; }
my ($fwd, $n) = (undef, 0);
for my $i (0..$#a) { $fwd = $a[$i+1] if $a[$i] eq '-L'; $n = 1 if $a[$i] eq '-N'; }
my $alias = $n ? $a[-1] : $a[-2];
if ($alias =~ /down/) { print STDERR "ssh: connect to host $alias port 22: Connection refused\n"; exit 255; }
if ($alias =~ /slow/) { sleep 60; exit 0; }
if (!$n && $alias =~ /lag/) { select(undef, undef, undef, 0.8); }
if (!$n) { print "__PAIR__\n"; exit 0; }
my ($sock, $host, $port) = $fwd =~ /^(.*):([^:]+):(\d+)$/;
unlink $sock;
my $srv = IO::Socket::UNIX->new(Type => SOCK_STREAM, Local => $sock, Listen => 16) or die "bind: $!";
$SIG{CHLD} = 'IGNORE';
if ($alias =~ /brief/) { $SIG{ALRM} = sub { exit 0 }; alarm 1; }
while (1) {
  my $c = $srv->accept or next;
  my $pid = fork();
  if (!defined $pid) { close $c; next; }
  if ($pid == 0) {
    close $srv;
    my $t = IO::Socket::INET->new(PeerAddr => $host, PeerPort => $port, Proto => 'tcp') or exit 0;
    my $sel = IO::Select->new($c, $t);
    LOOP: while (my @r = $sel->can_read) {
      for my $h (@r) {
        my $buf; my $got = sysread($h, $buf, 65536);
        last LOOP unless $got;
        my $o = $h == $c ? $t : $c;
        my $off = 0;
        while ($off < $got) { my $w = syswrite($o, $buf, $got - $off, $off); last LOOP unless defined $w; $off += $w; }
      }
    }
    exit 0;
  }
  close $c;
}
"#
        .replace("__LOG__", &log.to_string_lossy())
        .replace("__PAIR__", &pair_line.replace('\\', "\\\\").replace('"', "\\\""));
        std::fs::write(&path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    /// A short folder for sockets (`sun_path`).
    pub(crate) fn short_dir() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("hp")
            .tempdir_in("/tmp")
            .unwrap()
    }

    const ID: &str = "m-8d21c4be90f17a3e";

    async fn gone_within(pid: u32, within: Duration) -> bool {
        crate::process_group::test_support::wait_gone(pid as i32, within).await
    }

    #[test]
    fn ssh_gets_the_hardened_options_the_forward_and_nothing_secret() {
        let args: Vec<String> = ssh_args("runner", Path::new("/tmp/hollow-peer/x.sock"), 47680)
            .into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let joined = args.join(" ");
        for opt in crate::remote_session::SSH_OPTS {
            assert!(args.iter().any(|a| a == opt), "{opt}");
        }
        assert!(joined.contains("-a -x"));
        assert!(joined.contains("BatchMode=yes"));
        assert!(joined.contains("ForwardAgent=no"));
        assert!(joined.contains("-N -T -o ExitOnForwardFailure=yes -o StreamLocalBindUnlink=yes -o PermitLocalCommand=no"));
        assert!(joined.ends_with("-L /tmp/hollow-peer/x.sock:127.0.0.1:47680 -- runner"));
    }

    #[test]
    fn the_socket_path_must_fit_and_its_folder_is_private() {
        let dir = short_dir();
        let sock = socket_path(dir.path(), ID).unwrap();
        assert!(sock.ends_with(format!("{ID}.sock")));
        let long = dir.path().join("x".repeat(90));
        assert!(matches!(
            socket_path(&long, ID),
            Err(TunnelError::Socket(_))
        ));
        assert!(socket_path(dir.path(), "../etc").is_err());
        assert!(socket_path(&dir.path().join("a:b"), ID).is_err());
        // A new tunnel makes its folder private itself.
        let made = dir.path().join("made");
        let spawner: Arc<dyn Spawner> = Arc::new(SshSpawner::new());
        Tunnel::new(ID, vec!["runner".into()], 1, &made, spawner).unwrap();
        assert!(made.is_dir());
        let peer = dir.path().join("hollow-peer");
        std::fs::create_dir(&peer).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&peer, std::fs::Permissions::from_mode(0o755)).unwrap();
            private_dir(&peer).unwrap();
            let mode = std::fs::metadata(&peer).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
            let file = dir.path().join("file");
            std::fs::write(&file, "").unwrap();
            assert!(private_dir(&file).is_err());
        }
        // The real one, under this test run's TMPDIR.
        let real = socket_dir().unwrap();
        assert!(real.ends_with("hollow-peer"));
    }

    #[test]
    fn backoff_steps_up_to_a_minute() {
        let secs: Vec<u64> = (1..=7).map(|n| backoff(n).as_secs()).collect();
        assert_eq!(secs, [2, 5, 15, 30, 60, 60, 60]);
        assert_eq!(backoff(0).as_secs(), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_tunnel_opens_through_the_first_alias_that_works_and_is_killed_on_drop() {
        let dir = short_dir();
        let ssh = fake_ssh(dir.path(), "{}");
        let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = target.local_addr().unwrap().port();
        let tunnel = Tunnel::new(
            ID,
            vec!["host-down".into(), "host-up".into()],
            port,
            dir.path(),
            Arc::new(SshSpawner::with_program(&ssh)),
        )
        .unwrap();
        let sock = tunnel.ensure().await.unwrap();
        assert_eq!(tunnel.alias().await.as_deref(), Some("host-up"));
        // Bytes cross to the target.
        let accept = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            // The readiness check's own connection comes first, empty.
            loop {
                let (mut s, _) = target.accept().await.unwrap();
                let mut buf = [0u8; 5];
                if s.read_exact(&mut buf).await.is_ok() {
                    return buf;
                }
            }
        });
        {
            use tokio::io::AsyncWriteExt;
            let mut s = tokio::net::UnixStream::connect(&sock).await.unwrap();
            s.write_all(b"hello").await.unwrap();
            assert_eq!(&accept.await.unwrap(), b"hello");
        }
        // Both aliases were tried, in order, with the forward and no secret.
        let log = std::fs::read_to_string(dir.path().join("ssh.args")).unwrap();
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with("-- host-down"));
        assert!(lines[1].contains(&format!(":127.0.0.1:{port} -- host-up")));
        // A second ensure reuses the running ssh.
        let pid = tunnel.pid().await.unwrap();
        tunnel.ensure().await.unwrap();
        assert_eq!(tunnel.pid().await, Some(pid));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("ssh.args"))
                .unwrap()
                .lines()
                .count(),
            2
        );
        drop(tunnel);
        assert!(gone_within(pid, Duration::from_secs(5)).await, "ssh ended");
        assert!(!sock.exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn every_alias_failing_says_why_then_waits_out_the_backoff() {
        let dir = short_dir();
        let ssh = fake_ssh(dir.path(), "{}");
        let tunnel = Tunnel::new(
            ID,
            vec!["one-down".into(), "two-down".into()],
            1,
            dir.path(),
            Arc::new(SshSpawner::with_program(&ssh)),
        )
        .unwrap();
        let err = tunnel.ensure().await.unwrap_err();
        match &err {
            TunnelError::Unreachable { alias, ssh_error } => {
                assert_eq!(alias, "two-down");
                assert!(ssh_error.contains("Connection refused"), "{ssh_error}");
            }
            other => panic!("{other:?}"),
        }
        let text = err.to_string();
        assert!(text.starts_with("This Mac couldn't reach two-down over SSH: ssh: connect to host two-down port 22: Connection refused. It is on but not taking SSH"), "{text}");
        // The next call doesn't start ssh again until the wait is over.
        let waiting = tunnel.ensure().await.unwrap_err();
        match &waiting {
            TunnelError::Waiting { retry_in, .. } => assert!(*retry_in <= Duration::from_secs(2)),
            other => panic!("{other:?}"),
        }
        assert!(
            waiting
                .to_string()
                .ends_with("Hollow tries again in 2 seconds."),
            "{waiting}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("ssh.args"))
                .unwrap()
                .lines()
                .count(),
            2
        );
        // Retry skips the wait; a second round of failures waits longer.
        tunnel.retry_now();
        assert!(matches!(
            tunnel.ensure().await,
            Err(TunnelError::Unreachable { .. })
        ));
        match tunnel.ensure().await {
            Err(TunnelError::Waiting { retry_in, .. }) => {
                assert!(retry_in > Duration::from_secs(2) && retry_in <= Duration::from_secs(5))
            }
            other => panic!("{other:?}"),
        }
        let none = Tunnel::new(
            ID,
            vec![],
            1,
            dir.path(),
            Arc::new(SshSpawner::with_program(&ssh)),
        )
        .unwrap();
        assert_eq!(none.ensure().await.unwrap_err(), TunnelError::NoAlias);
    }

    /// Review: clearing the backoff and asking for the alias never wait for
    /// an opening in progress (it holds `state` for up to 12 s per alias).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn clearing_the_backoff_and_asking_the_alias_never_wait_for_an_opening() {
        let dir = short_dir();
        let ssh = fake_ssh(dir.path(), "{}");
        let tunnel = Arc::new(
            Tunnel::new(
                ID,
                vec!["slow".into()],
                1,
                dir.path(),
                Arc::new(SshSpawner::with_program(&ssh)),
            )
            .unwrap()
            .with_ready_within(Duration::from_millis(1500)),
        );
        let opening = tokio::spawn({
            let tunnel = tunnel.clone();
            async move { tunnel.ensure().await }
        });
        tokio::time::sleep(Duration::from_millis(300)).await;
        tunnel.retry_now();
        assert!(tunnel.alias().await.is_none(), "not up while it opens");
        // Neither call waited for the opening: it is still in progress.
        assert!(!opening.is_finished(), "the calls waited for the opening");
        assert!(matches!(
            opening.await.unwrap(),
            Err(TunnelError::Timeout { .. })
        ));
        // That failure set a wait; clearing it lets the next try start.
        assert!(matches!(
            tunnel.ensure().await,
            Err(TunnelError::Waiting { .. })
        ));
        tunnel.retry_now();
        assert!(matches!(
            tunnel.ensure().await,
            Err(TunnelError::Timeout { .. })
        ));
    }

    /// M2 leftovers: Cancel ends an opening at once, starts nothing until
    /// Retry, counts no failure, and leaves a tunnel that is up alone.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_ends_an_opening_and_holds_it_closed_until_retry() {
        let dir = short_dir();
        let ssh = fake_ssh(dir.path(), "{}");
        let tunnel = Arc::new(
            Tunnel::new(
                ID,
                vec!["slow".into(), "also-slow".into()],
                1,
                dir.path(),
                Arc::new(SshSpawner::with_program(&ssh)),
            )
            .unwrap(),
        );
        let opening = tokio::spawn({
            let tunnel = tunnel.clone();
            async move { tunnel.ensure().await }
        });
        tokio::time::sleep(Duration::from_millis(300)).await;
        tunnel.stop_opening();
        // Ended within a poll, not after the 12 s wait (a bound, no clock assert).
        let ended = tokio::time::timeout(Duration::from_secs(6), opening)
            .await
            .expect("the opening ended within a poll");
        assert_eq!(ended.unwrap(), Err(TunnelError::Stopped));
        assert!(tunnel.pid().await.is_none());
        // Only the first alias was tried, and nothing starts again.
        let tried = || {
            std::fs::read_to_string(dir.path().join("ssh.args"))
                .unwrap()
                .lines()
                .count()
        };
        assert_eq!(tried(), 1);
        assert_eq!(tunnel.ensure().await, Err(TunnelError::Stopped));
        assert_eq!(tried(), 1);
        assert_eq!(
            TunnelError::Stopped.to_string(),
            "Connecting was cancelled. Retry connects again."
        );
        // Retry: tried again at once (no backoff from the cancel).
        tunnel.set_hosts(vec!["host-down".into()]);
        tunnel.retry_now();
        assert!(matches!(
            tunnel.ensure().await,
            Err(TunnelError::Unreachable { .. })
        ));
        // A tunnel that is up stays up through a Cancel.
        let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = target.local_addr().unwrap().port();
        let up = Tunnel::new(
            "m-8d21c4be90f17a3f",
            vec!["host-up".into()],
            port,
            dir.path(),
            Arc::new(SshSpawner::with_program(&ssh)),
        )
        .unwrap();
        up.ensure().await.unwrap();
        up.stop_opening();
        assert!(!up.stopped(), "a Cancel on a live tunnel marks nothing");
        assert!(up.ensure().await.is_ok());
        assert_eq!(up.alias().await.as_deref(), Some("host-up"));
        // Review: after that Cancel, a drop (here a close) reconnects on the
        // next ensure instead of reading as cancelled; so does an idle close.
        up.close().await;
        assert!(up.ensure().await.is_ok(), "reconnects after the drop");
        up.stop_opening();
        assert!(up.close_if_idle(Duration::ZERO).await);
        assert!(up.ensure().await.is_ok(), "reconnects after an idle close");
        // A stale flag (Cancel raced the alive check) is cleared by the next
        // ensure that finds the tunnel up.
        up.stopped.store(true, Ordering::SeqCst);
        assert!(up.ensure().await.is_ok());
        assert!(!up.stopped());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_ssh_that_never_binds_times_out_and_is_ended() {
        let dir = short_dir();
        let ssh = fake_ssh(dir.path(), "{}");
        let tunnel = Tunnel::new(
            ID,
            vec!["slow".into()],
            1,
            dir.path(),
            Arc::new(SshSpawner::with_program(&ssh)),
        )
        .unwrap()
        .with_ready_within(Duration::from_millis(600));
        let err = tunnel.ensure().await.unwrap_err();
        assert_eq!(
            err,
            TunnelError::Timeout {
                alias: "slow".into()
            }
        );
        assert!(err
            .to_string()
            .contains("didn't answer over SSH within 12 seconds"));
        assert!(tunnel.pid().await.is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_tunnel_whose_ssh_died_opens_again_and_an_idle_one_closes() {
        let dir = short_dir();
        let ssh = fake_ssh(dir.path(), "{}");
        let tunnel = Tunnel::new(
            ID,
            vec!["brief".into()],
            1,
            dir.path(),
            Arc::new(SshSpawner::with_program(&ssh)),
        )
        .unwrap();
        tunnel.ensure().await.unwrap();
        let first = tunnel.pid().await.unwrap();
        // It exits after a second; `alias` asks it (and reaps it).
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while tunnel.alias().await.is_some() {
            assert!(std::time::Instant::now() < deadline, "the brief ssh exits");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // Reaped by try_wait, then opened again.
        tunnel.ensure().await.unwrap();
        let second = tunnel.pid().await.unwrap();
        assert_ne!(first, second);
        assert!(!tunnel.close_if_idle(Duration::from_secs(60)).await);
        // A use of the open socket (a frame read) keeps it from looking idle.
        tokio::time::sleep(Duration::from_millis(300)).await;
        tunnel.touch();
        assert!(!tunnel.close_if_idle(Duration::from_millis(200)).await);
        assert!(tunnel.close_if_idle(Duration::ZERO).await);
        assert!(tunnel.pid().await.is_none());
        assert!(gone_within(second, Duration::from_secs(5)).await);
    }
}
