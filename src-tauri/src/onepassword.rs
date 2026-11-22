//! 1Password as an optional second secret path beside the Keychain (t3code
//! parity row 7.5, 2026-11-20). **Off by default.** The Keychain
//! ([`crate::secret_store`]) stays the default and the fallback.
//!
//! - **What is saved.** Settings, Accounts, 1Password holds a switch and, per
//!   secret slot (the GitHub runner token, the Linear API key, each Claude
//!   account's token), an `op://Vault/Item/field` reference, in
//!   `<app data>/onepassword.json` (0600, never the vault, so an agent that
//!   writes the vault can't point a secret anywhere). A reference names an
//!   item, it is not the secret; Hollow never saves, caches on disk or logs a
//!   value 1Password gave it.
//! - **When it is used.** [`crate::secret_store::get`] asks [`lookup`] first:
//!   with the switch on and a reference saved for that key, the value is read
//!   at that moment with `op read --no-newline <reference>`. With the switch
//!   off, or no reference, [`lookup`] says `None` and `op` is never run (an
//!   off Hollow does not even look for the program on a read path).
//! - **Fallback.** A reference that can't be read (`op` missing, 1Password
//!   locked, not signed in, no such item, no answer in 30 s) never blocks the
//!   Keychain: the Keychain value for the same key is used when there is one,
//!   and only when there is none does the caller get the clear error
//!   ([`OpError::message`], fixed words, never 1Password's own output). A
//!   failure is remembered for 2 minutes for that key and reference, or, when
//!   it is about 1Password itself (locked, not signed in, no answer), until
//!   Check, a setting change or a restart, so a stream of callers can't stack
//!   unlock prompts; Settings' Check button forgets it. Finding `op` looks only in
//!   `/opt/homebrew/bin` and `/usr/local/bin` (owner root or this user, not
//!   group or world writable), never `PATH` or a folder under the home.
//! - **What the child sees.** `op` runs with a cleared environment: `PATH`
//!   (with the usual Homebrew folders added), `HOME`, `USER`, `LOGNAME`,
//!   `TMPDIR`, locale and XDG folders, and the person's own `OP_*` variables
//!   (a service account or Connect server, if they use one). Not Hollow's, an
//!   agent's or a connector's tokens. Stdin is empty, the value comes back on
//!   a pipe (never a file, never argv), and its buffers are overwritten after
//!   it is copied out. **The reference is on `op`'s command line**: it names
//!   an item, so another process of this user can see which item Hollow
//!   reads, never what is in it.
//! - **Redaction.** [`scrub_resolved`] (called by `claude_runner::redact`, which
//!   every log, health line, transcript snapshot and error excerpt already
//!   goes through) removes any value read this run, by value, so a secret of
//!   any shape is masked wherever it would have been echoed. The values are
//!   kept in memory only, for this process, at most [`REGISTRY_MAX`] of them;
//!   a value shorter than [`MIN_SCRUB`] characters is not scrubbed by value
//!   (it would mask ordinary words). `redact` also masks `op://` references,
//!   1Password service-account tokens and Account Secret Keys by shape.
//! - **Errors** are one of [`OpError`]'s fixed messages. `op`'s stderr is read
//!   only to choose between them and is dropped.
//!
//! Settings, Accounts, 1Password is the person's control; the Hollow app
//! actions (`settings.accounts.onepassword_*`) change the same file only
//! behind a card that always asks.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

const FILE_NAME: &str = "onepassword.json";
/// How long one `op` call may take (the desktop app's approval prompt is
/// inside it).
const OP_TIMEOUT: Duration = Duration::from_secs(30);
/// `op --version` needs no sign-in.
const VERSION_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a failed read is remembered (like `secret_store`'s five minutes
/// for a denied Keychain read, shorter because Settings' Check forgets it).
const FAIL_MEMORY: Duration = Duration::from_secs(120);
/// A value larger than this is refused (a certificate bundle is far below it).
const MAX_VALUE: usize = 64 * 1024;
/// How long to wait for the output pipes after `op` has exited (a daemon
/// `op` started may keep them open).
const DRAIN_WAIT: Duration = Duration::from_millis(1500);
/// How many values [`scrub_resolved`] remembers.
pub(crate) const REGISTRY_MAX: usize = 32;
/// The shortest value [`scrub_resolved`] masks by value.
pub(crate) const MIN_SCRUB: usize = 6;

const REF_PREFIX: &str = "op://";

/// Why a reference could not be read. Each has one fixed message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpError {
    NotInstalled,
    Locked,
    NotSignedIn,
    ItemNotFound,
    BadReference,
    TimedOut,
    Failed,
}

impl OpError {
    /// What a person (or a caller's error) is told.
    pub(crate) fn message(self) -> &'static str {
        match self {
            OpError::NotInstalled => "The 1Password command line tool (op) isn't installed on this Mac, so Hollow is using the Keychain.",
            OpError::Locked => "1Password is locked, or its approval prompt was dismissed. Unlock 1Password and try again.",
            OpError::NotSignedIn => "1Password isn't signed in for the command line tool. Turn on Integrate with 1Password CLI in the 1Password app (Settings, Developer), or run op signin in Terminal.",
            OpError::ItemNotFound => "1Password couldn't find that reference. Check the vault, item and field names.",
            OpError::BadReference => "That isn't a 1Password reference. It looks like op://Vault/Item/field.",
            OpError::TimedOut => "1Password didn't answer within 30 seconds.",
            OpError::Failed => "1Password couldn't read that reference.",
        }
    }
}

// ---- Settings ---------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    /// Off until the person turns it on.
    #[serde(default)]
    enabled: bool,
    /// Secret key (`secret_store`'s) → `op://` reference.
    #[serde(default)]
    refs: BTreeMap<String, String>,
}

static SETTINGS: Mutex<Option<Settings>> = Mutex::new(None);
/// One change at a time (read, edit, write).
static WRITE: Mutex<()> = Mutex::new(());

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn settings_path() -> Result<PathBuf, String> {
    #[cfg(test)]
    {
        if let Some(dir) = lock(&tests::DIR).clone() {
            return Ok(dir.join(FILE_NAME));
        }
    }
    Ok(crate::agent_settings::app_data_dir()?.join(FILE_NAME))
}

fn read_file() -> Result<Settings, String> {
    let path = settings_path()?;
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// The saved settings. A file that can't be read or parsed counts as off, so
/// a damaged file sends every read back to the Keychain; it isn't cached, so
/// a repaired file is picked up.
fn settings() -> Settings {
    let mut cached = lock(&SETTINGS);
    if let Some(s) = cached.as_ref() {
        return s.clone();
    }
    match read_file() {
        Ok(s) => {
            *cached = Some(s.clone());
            s
        }
        Err(_) => Settings::default(),
    }
}

fn change(f: impl FnOnce(&mut Settings) -> Result<(), String>) -> Result<(), String> {
    let _held = lock(&WRITE);
    // A damaged file counts as off (`settings`), so a change replaces it.
    let mut next = read_file().unwrap_or_default();
    f(&mut next)?;
    let text = serde_json::to_string_pretty(&next).map_err(|e| e.to_string())?;
    crate::vault::atomic_write_private(&settings_path()?, text.as_bytes())?;
    *lock(&SETTINGS) = Some(next);
    lock(&FAILED).clear();
    lock(&LAST_ERROR).clear();
    Ok(())
}

/// Whether the person turned the 1Password path on.
pub(crate) fn enabled() -> bool {
    settings().enabled
}

/// Turn the 1Password path on or off. Saved references stay either way.
pub(crate) fn set_enabled(on: bool) -> Result<(), String> {
    change(|s| {
        s.enabled = on;
        Ok(())
    })
}

/// Whether `reference` is a well-formed `op://vault/item/field` (or
/// `op://vault/item/section/field`) reference: no control characters, no
/// empty or dot segments, a bounded length. A reference is not a secret, but
/// it is checked before it reaches a command line.
pub(crate) fn reference_ok(reference: &str) -> Result<(), OpError> {
    let Some(rest) = reference.strip_prefix(REF_PREFIX) else {
        return Err(OpError::BadReference);
    };
    // The optional `?attribute=otp` suffix belongs to the last segment.
    let path = rest.split('?').next().unwrap_or_default();
    let parts: Vec<&str> = path.split('/').collect();
    let shaped = (3..=4).contains(&parts.len())
        && parts
            .iter()
            .all(|p| !p.trim().is_empty() && *p != "." && *p != "..");
    if reference.len() > 512 || !shaped || reference.chars().any(|c| c.is_control() || c == '\\') {
        return Err(OpError::BadReference);
    }
    Ok(())
}

/// Save `reference` for secret `key` (a slot of [`slots`]; the caller checks).
pub(crate) fn set_reference(key: &str, reference: &str) -> Result<(), String> {
    let reference = reference.trim();
    reference_ok(reference).map_err(|e| e.message().to_string())?;
    change(|s| {
        s.refs.insert(key.to_string(), reference.to_string());
        Ok(())
    })
}

/// Forget the reference for `key` (the Keychain value, if any, is used again).
pub(crate) fn clear_reference(key: &str) -> Result<(), String> {
    change(|s| {
        s.refs.remove(key);
        Ok(())
    })
}

/// Whether reads of `key` go to 1Password now (the switch is on and a
/// reference is saved). Never runs `op`, so a presence check cannot prompt.
pub(crate) fn has_reference(key: &str) -> bool {
    let s = settings();
    s.enabled && s.refs.contains_key(key)
}

// ---- Finding and running op -------------------------------------------------

/// The folders `op` is looked for in besides `PATH` (Hollow opened from
/// Finder has a short one).
const EXTRA_DIRS: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/usr/bin",
    "~/.local/bin",
];

fn extra_dirs() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    EXTRA_DIRS
        .iter()
        .map(|d| match d.strip_prefix("~/") {
            Some(rel) => home.join(rel),
            None => PathBuf::from(d),
        })
        .collect()
}

/// The folders `op` is looked for in: where Homebrew and 1Password's own
/// installer put it. Not `PATH`, and not a folder under the home: a program
/// any process of this user could drop there would run outside an agent's
/// sandbox and be believed about a secret.
const OP_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin"];

/// The `op` program, when one is installed: a regular executable owned by
/// root or this user that no one else can write to.
fn find_op() -> Option<PathBuf> {
    #[cfg(test)]
    {
        if let Some(bin) = lock(&tests::BIN).clone() {
            return bin.is_file().then_some(bin);
        }
    }
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    // SAFETY: `geteuid` has no preconditions.
    let me = unsafe { libc::geteuid() };
    OP_DIRS.iter().map(|d| Path::new(d).join("op")).find(|p| {
        std::fs::metadata(p).is_ok_and(|m| {
            m.is_file()
                && m.permissions().mode() & 0o111 != 0
                && m.permissions().mode() & 0o022 == 0
                && (m.uid() == 0 || m.uid() == me)
        })
    })
}

/// Whether the 1Password desktop app is installed (`op` can then borrow its
/// sign-in: Settings, Developer, Integrate with 1Password CLI).
fn desktop_app() -> bool {
    Path::new("/Applications/1Password.app").is_dir()
}

/// What `op` is given: from `vars`, the few it needs and the person's own
/// `OP_*`, and a `PATH` that reaches the usual folders. Nothing else.
fn child_env(vars: impl Iterator<Item = (OsString, OsString)>) -> Vec<(OsString, OsString)> {
    const KEEP: &[&str] = &[
        "HOME",
        "USER",
        "LOGNAME",
        "TMPDIR",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "XDG_DATA_HOME",
        "XDG_RUNTIME_DIR",
    ];
    let mut kept = Vec::new();
    let mut path = Vec::new();
    for (k, v) in vars {
        let name = k.to_string_lossy();
        if name == "PATH" {
            path.extend(std::env::split_paths(&v));
        } else if KEEP.contains(&name.as_ref()) || name.starts_with("OP_") {
            kept.push((k, v));
        }
    }
    for d in extra_dirs() {
        if !path.contains(&d) {
            path.push(d);
        }
    }
    if let Ok(joined) = std::env::join_paths(path) {
        kept.push(("PATH".into(), joined));
    }
    kept
}

/// Overwrite `bytes` so a copy of a value doesn't linger in freed memory.
fn wipe(bytes: &mut [u8]) {
    for b in bytes.iter_mut() {
        // SAFETY: `b` is a valid, aligned, exclusive reference.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
}

struct Done {
    ok: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl Drop for Done {
    fn drop(&mut self) {
        wipe(&mut self.stdout);
        wipe(&mut self.stderr);
    }
}

/// Copy a pipe into a shared buffer as it arrives, up to `cap` bytes (then
/// drain and drop the rest so the child never blocks on a full pipe).
fn pump(mut pipe: impl std::io::Read + Send + 'static, cap: usize) -> Arc<Mutex<(Vec<u8>, bool)>> {
    let shared = Arc::new(Mutex::new((Vec::new(), false)));
    let sink = Arc::clone(&shared);
    std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut held = lock(&sink);
                    let room = cap.saturating_sub(held.0.len());
                    held.0.extend_from_slice(&chunk[..n.min(room)]);
                }
            }
        }
        lock(&sink).1 = true;
        wipe(&mut chunk);
    });
    shared
}

/// `op <args>` with the cleared environment, bounded by `limit`. A program
/// that isn't there is [`OpError::NotInstalled`], one that won't start
/// [`OpError::Failed`]; one that outruns `limit`
/// is killed (by its own PID) and is [`OpError::TimedOut`].
fn run_op(bin: &Path, args: &[&str], limit: Duration) -> Result<Done, OpError> {
    let spawn = || {
        Command::new(bin)
            .args(args)
            .env_clear()
            .envs(child_env(std::env::vars_os()))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    };
    let mut tries = 0;
    let mut child = loop {
        match spawn() {
            Ok(child) => break child,
            // A program another thread just finished writing can be busy
            // for a moment.
            Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) && tries < 5 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(OpError::NotInstalled)
            }
            Err(_) => return Err(OpError::Failed),
        }
    };
    let out = pump(child.stdout.take().expect("piped"), MAX_VALUE + 1);
    let err = pump(child.stderr.take().expect("piped"), 8 * 1024);
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < limit => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                for shared in [&out, &err] {
                    wipe(&mut lock(shared).0);
                }
                return Err(OpError::TimedOut);
            }
        }
    };
    let drained = Instant::now();
    while !(lock(&out).1 && lock(&err).1) && drained.elapsed() < DRAIN_WAIT {
        std::thread::sleep(Duration::from_millis(10));
    }
    let take = |shared: &Arc<Mutex<(Vec<u8>, bool)>>| std::mem::take(&mut lock(shared).0);
    Ok(Done {
        ok: status.success(),
        stdout: take(&out),
        stderr: take(&err),
    })
}

/// Which failure `op`'s stderr describes. The text is read here and dropped.
fn classify(stderr: &[u8]) -> OpError {
    let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| text.contains(w));
    if has(&[
        "not currently signed in",
        "not signed in",
        "no accounts configured",
        "no account found",
        "session expired",
    ]) {
        OpError::NotSignedIn
    } else if has(&[
        "isn't an item",
        "isn't a vault",
        "isn't a field",
        "isn't a section",
        "could not find",
        "couldn't find",
        "doesn't exist",
        "more than one item",
    ]) {
        // Before `Locked`: the stderr names the item and its vault, and a
        // vault called "Locked Box" must not read as a locked 1Password.
        OpError::ItemNotFound
    } else if has(&[
        "locked",
        "authorization prompt",
        "authorization timeout",
        "authorization denied",
        "prompt dismissed",
        "biometric",
    ]) {
        OpError::Locked
    } else {
        OpError::Failed
    }
}

/// Read `reference` now: `op read --no-newline <reference>`. The value is
/// remembered for [`scrub_resolved`] and returned; it goes nowhere else.
fn resolve(reference: &str) -> Result<String, OpError> {
    reference_ok(reference)?;
    let bin = find_op().ok_or(OpError::NotInstalled)?;
    let done = run_op(&bin, &["read", "--no-newline", reference], OP_TIMEOUT)?;
    if !done.ok {
        return Err(classify(&done.stderr));
    }
    if done.stdout.len() > MAX_VALUE {
        return Err(OpError::Failed);
    }
    let value = std::str::from_utf8(&done.stdout).map_err(|_| OpError::Failed)?;
    let value = value.trim_end_matches(['\r', '\n']);
    if value.is_empty() {
        return Err(OpError::Failed);
    }
    remember(value);
    Ok(value.to_string())
}

// ---- The read path ----------------------------------------------------------

/// A failed read, remembered for [`FAIL_MEMORY`]: `scope` is the key and
/// reference it was about, or `None` for one about 1Password itself (locked,
/// not signed in, no answer), which holds for every key so N references cost
/// one unlock prompt, not N. Not installed is never remembered: finding `op`
/// runs nothing.
struct Failure {
    scope: Option<(String, String)>,
    at: Instant,
    error: OpError,
}

impl Failure {
    /// Locked, not signed in and timed out (no scope) are about 1Password
    /// itself and stay remembered until the person presses Check, changes a
    /// setting or restarts Hollow: otherwise an agent calling a read-only
    /// action in a loop would raise an unlock prompt every
    /// [`FAIL_MEMORY`]. A failure of one reference lasts [`FAIL_MEMORY`].
    fn live(&self) -> bool {
        self.scope.is_none() || self.at.elapsed() < FAIL_MEMORY
    }
}

static FAILED: Mutex<Vec<Failure>> = Mutex::new(Vec::new());
/// The last read of each key: `None` when it worked.
static LAST_ERROR: Mutex<BTreeMap<String, Option<OpError>>> = Mutex::new(BTreeMap::new());
/// One `op` read at a time: callers stacked behind a failure meet the
/// remembered failure instead of each raising a prompt.
static RESOLVING: Mutex<()> = Mutex::new(());

/// What [`crate::secret_store::get`] asks first. `None`: this key is not read
/// through 1Password (the switch is off, or no reference is saved for it), so
/// the Keychain answers and `op` is never run. `Some`: the 1Password result.
pub(crate) fn lookup(key: &str) -> Option<Result<String, OpError>> {
    let s = settings();
    if !s.enabled {
        return None;
    }
    let reference = s.refs.get(key)?.clone();
    let _one = lock(&RESOLVING);
    let remembered = lock(&FAILED)
        .iter()
        .find(|f| {
            f.live()
                && f.scope
                    .as_ref()
                    .is_none_or(|(k, r)| k == key && *r == reference)
        })
        .map(|f| f.error);
    let result = match remembered {
        Some(e) => Err(e),
        None => resolve(&reference),
    };
    note(key, &reference, result.as_ref().err().copied());
    Some(result)
}

fn note(key: &str, reference: &str, error: Option<OpError>) {
    lock(&LAST_ERROR).insert(key.to_string(), error);
    let mut failed = lock(&FAILED);
    failed.retain(Failure::live);
    match error {
        // 1Password answered: nothing about it is wrong now, and nothing
        // about this key.
        None => failed.retain(|f| f.scope.as_ref().is_some_and(|(k, _)| k != key)),
        Some(OpError::NotInstalled) => {}
        Some(e @ (OpError::Locked | OpError::NotSignedIn | OpError::TimedOut)) => {
            failed.retain(|f| f.scope.is_some());
            failed.push(Failure {
                scope: None,
                at: Instant::now(),
                error: e,
            });
        }
        Some(e) => {
            failed.retain(|f| f.scope.as_ref().is_none_or(|(k, _)| k != key));
            failed.push(Failure {
                scope: Some((key.to_string(), reference.to_string())),
                at: Instant::now(),
                error: e,
            });
        }
    }
}

// ---- Redaction --------------------------------------------------------------

static RESOLVED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Keep `value` so [`scrub_resolved`] can mask it. In memory only.
fn remember(value: &str) {
    if value.chars().count() < MIN_SCRUB {
        return;
    }
    let mut held = lock(&RESOLVED);
    if held.iter().any(|v| v == value) {
        return;
    }
    if held.len() >= REGISTRY_MAX {
        let mut oldest = held.remove(0);
        // SAFETY: only zeroes the string's bytes, which stay valid UTF-8.
        wipe(unsafe { oldest.as_bytes_mut() });
    }
    held.push(value.to_string());
}

/// Mask every value read from 1Password in this process wherever `text`
/// holds it (a multi-line value also line by line, for its lines of
/// [`MIN_SCRUB`] characters or more).
pub(crate) fn scrub_resolved(text: &mut String) {
    let held = lock(&RESOLVED);
    for value in held.iter() {
        if text.contains(value.as_str()) {
            *text = text.replace(value.as_str(), "[redacted]");
        }
        // The same value inside a JSON string (quotes, backslashes, newlines
        // escaped), as a tool result or a log line carries it.
        if let Ok(quoted) = serde_json::to_string(value) {
            let escaped = &quoted[1..quoted.len() - 1];
            if escaped != value && text.contains(escaped) {
                *text = text.replace(escaped, "[redacted]");
            }
        }
        if value.contains('\n') {
            for line in value.lines().map(str::trim) {
                if line.chars().count() >= MIN_SCRUB && text.contains(line) {
                    *text = text.replace(line, "[redacted]");
                }
            }
        }
    }
}

// ---- Status (Settings) ------------------------------------------------------

/// A secret Settings can point at a reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Slot {
    pub key: String,
    pub label: String,
}

/// The secrets a reference can stand in for: the GitHub runner token, the
/// Linear API key and each added Claude account's token.
pub(crate) fn slots() -> Vec<Slot> {
    let mut out = vec![
        Slot {
            key: crate::runner_access::GITHUB_TOKEN_KEY.to_string(),
            label: "GitHub runner token".to_string(),
        },
        Slot {
            key: crate::tracker::settings::key_name("linear"),
            label: "Linear API key".to_string(),
        },
    ];
    if let Some(view) = crate::claude_accounts::accounts_view() {
        out.extend(view.accounts.into_iter().map(|a| Slot {
            key: crate::secret_store::claude_key(&a.id),
            label: format!("Claude account: {}", a.label),
        }));
    }
    out
}

/// The command line tool's state, as Settings words it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliView {
    /// `off`, `not-installed`, `unchecked`, `ready`, `locked`, `not-signed-in`,
    /// `timed-out` or `error`.
    pub state: &'static str,
    pub message: String,
    pub version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotView {
    pub key: String,
    pub label: String,
    pub reference: Option<String>,
    /// Why the last read through 1Password failed, `None` when it worked or
    /// hasn't been tried.
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub enabled: bool,
    pub installed: bool,
    pub desktop_app: bool,
    pub cli: CliView,
    pub slots: Vec<SlotView>,
}

fn view(enabled: bool, installed: bool, cli: CliView, slots: Vec<Slot>) -> Status {
    let saved = settings().refs;
    let last = lock(&LAST_ERROR).clone();
    Status {
        enabled,
        installed,
        desktop_app: desktop_app(),
        cli,
        slots: slots
            .into_iter()
            .map(|slot| SlotView {
                reference: saved.get(&slot.key).cloned(),
                last_error: last
                    .get(&slot.key)
                    .copied()
                    .flatten()
                    .map(|e| e.message().to_string()),
                key: slot.key,
                label: slot.label,
            })
            .collect(),
    }
}

fn cli_state(error: OpError) -> &'static str {
    match error {
        OpError::NotInstalled => "not-installed",
        OpError::Locked => "locked",
        OpError::NotSignedIn => "not-signed-in",
        OpError::TimedOut => "timed-out",
        _ => "error",
    }
}

/// Settings' status without running anything: whether the switch is on,
/// whether `op` and the desktop app are there, each slot's reference.
pub(crate) fn status() -> Status {
    let on = enabled();
    let installed = find_op().is_some();
    let cli = match (on, installed) {
        (false, _) => CliView {
            state: "off",
            message: "Off. Secrets come from the Keychain.".into(),
            version: None,
        },
        (true, false) => CliView {
            state: "not-installed",
            message: OpError::NotInstalled.message().into(),
            version: None,
        },
        (true, true) => CliView {
            state: "unchecked",
            message: "Press Check to see whether 1Password answers.".into(),
            version: None,
        },
    };
    view(on, installed, cli, slots())
}

/// `op --version` then `op whoami`, for Settings' Check button. Runs only
/// when the switch is on (an off Hollow runs nothing), forgets remembered
/// failures first, and reads no secret.
pub(crate) fn check() -> Status {
    let on = enabled();
    if !on {
        return status();
    }
    lock(&FAILED).clear();
    let installed = find_op();
    let (cli, found) = match installed {
        None => (
            CliView {
                state: "not-installed",
                message: OpError::NotInstalled.message().into(),
                version: None,
            },
            false,
        ),
        Some(bin) => (probe(&bin), true),
    };
    view(on, found, cli, slots())
}

fn probe(bin: &Path) -> CliView {
    let version = match run_op(bin, &["--version"], VERSION_TIMEOUT) {
        Ok(done) if done.ok => Some(clean_version(&done.stdout)).filter(|v| !v.is_empty()),
        Ok(done) => {
            let e = classify(&done.stderr);
            return failed_cli(e, None);
        }
        Err(e) => return failed_cli(e, None),
    };
    match run_op(bin, &["whoami"], OP_TIMEOUT) {
        Ok(done) if done.ok => CliView {
            state: "ready",
            message: "1Password answered: the command line tool is signed in.".into(),
            version,
        },
        Ok(done) => failed_cli(classify(&done.stderr), version),
        Err(e) => failed_cli(e, version),
    }
}

fn failed_cli(error: OpError, version: Option<String>) -> CliView {
    CliView {
        state: cli_state(error),
        message: error.message().into(),
        version,
    }
}

/// A version number as `op --version` prints it, nothing else.
fn clean_version(stdout: &[u8]) -> String {
    String::from_utf8_lossy(stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
        .take(32)
        .collect()
}

#[cfg(test)]
#[path = "onepassword_tests.rs"]
pub(crate) mod tests;
