//! `hollow serve`'s commands and its control socket (M1 PR 3).
//!
//! A headless Hollow has no window, so the Settings clicks it still needs
//! are commands run by the person at the serving Mac's terminal:
//!
//! - `secret-set <key>` reads a value from stdin (a prompt with echo off at
//!   a terminal; never argv, so `ps` never shows it) into serve's private
//!   secrets file (`secret_store::SecretFile`); `secret-list` prints the
//!   key names only; `secret-delete <key>` removes one. These work whether
//!   serve runs or not: the file is shared under a lock and serve reads it
//!   again on every use.
//! - `pair [--machine] [--json]` opens a pairing window in the running serve
//!   and prints its code once, to stdout only. `--machine` opens the machine
//!   window (M2: another Mac pairs with `POST /v1/machines/pair`, never a
//!   phone), listening first if the iPhone app is off; `--json` prints one
//!   line, `{"code","expiresAt"}` and for a machine also `machineId`, `slug`
//!   and `name`, which the other Mac reads over SSH.
//! - `enable-phone`, `disable-phone` and `set-default-mode <mode> [--agent
//!   <agent>]` change the phone record: through the running serve, else on
//!   disk while nothing runs on the data folder (the app's own `gui.lock`
//!   is held for the edit; a running app refuses, since it keeps the record
//!   in memory and its Settings is the place).
//!
//! **The control socket.** `<app data>/serve.sock`, bound by serve once its
//! core is up and removed when it stops. Mode 0600, and every connection's
//! peer user (`getpeereid`) must be serve's own user, so only that user
//! can drive it (not even root: an admin stops serve and edits the record,
//! or runs the command as that user). One JSON line in, one JSON line out. A pairing
//! code goes back to the caller only: serve logs that a window opened, never
//! the code.

use crate::launch::elog;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::io::{BufRead as _, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

/// The control socket's name in the app data folder.
pub(crate) const SOCKET_FILE: &str = "serve.sock";
/// The 0700 folder it is bound in before it is moved into place.
const STAGE_PREFIX: &str = ".ss";
/// `sockaddr_un`'s path limit on macOS, with its NUL.
const SUN_PATH_MAX: usize = 104;
/// The longest request line read.
const REQUEST_MAX: u64 = 4096;
/// How long a connection may take to send its request (shorter in tests).
const READ_WAIT: Duration = if cfg!(test) {
    Duration::from_millis(300)
} else {
    Duration::from_secs(5)
};
/// How long a command waits for serve's answer (enabling may bind a port).
const ANSWER_WAIT: Duration = Duration::from_secs(30);
/// The longest secret value read from stdin.
const VALUE_MAX: u64 = 64 * 1024;

const EXIT_FAILED: i32 = 1;

/// `hollow serve`'s commands, as `serve::main` dispatches them.
pub(crate) const COMMANDS: &[&str] = &[
    "secret-set",
    "secret-list",
    "secret-delete",
    "pair",
    "enable-phone",
    "disable-phone",
    "set-default-mode",
    "runner-coo",
];

// ── The protocol ─────────────────────────────────────────────────────────

/// What a command asks of a running serve.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub(crate) enum Request {
    Pair {
        #[serde(default)]
        machine: bool,
    },
    EnablePhone,
    DisablePhone,
    SetDefaultMode {
        mode: String,
        #[serde(default)]
        agent: Option<String>,
    },
    /// `runner-coo on|off|status` (M3b): `None` asks for the status.
    RunnerCoo {
        #[serde(default)]
        on: Option<bool>,
    },
}

/// Serve's answer: `ok` and what the command prints, or why not.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Answer {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// `pair`: the code, `XXXX-XXXX`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// `pair`: when the window closes (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// `pair --machine`: the window pairs another Mac (M2).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub machine: bool,
    /// `pair --machine`: this Mac's `machineId`, slug and name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `enable-phone`/`disable-phone`: whether the phone API listens now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listening: Option<bool>,
    /// `runner-coo`: the role (`runner`, `off`) and the runner COO's chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_key: Option<String>,
    /// `runner-coo`: why the record could not be read (the role is off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_error: Option<String>,
}

impl Answer {
    fn done() -> Self {
        Self {
            ok: true,
            ..Self::default()
        }
    }
    fn failed(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(error.into()),
            ..Self::default()
        }
    }
}

pub(crate) type Handler =
    Arc<dyn Fn(Request) -> Pin<Box<dyn Future<Output = Answer> + Send>> + Send + Sync>;

/// Serve's own handler: the phone API's serve entry points.
pub(crate) fn core_handler() -> Handler {
    Arc::new(|request| Box::pin(handle(request)))
}

async fn handle(request: Request) -> Answer {
    match request {
        Request::Pair { machine: true } => {
            match crate::phone_access::serve_machine_pairing_start().await {
                Ok(window) => {
                    // Never the code: the caller prints it once.
                    elog(
                        "[serve] control: a machine pairing window opened (10 minutes or one use)",
                    );
                    Answer {
                        code: Some(window.code),
                        expires_at: Some(window.expires_at),
                        machine: true,
                        machine_id: Some(window.machine_id),
                        slug: window.slug,
                        name: Some(window.name),
                        ..Answer::done()
                    }
                }
                Err(error) => Answer::failed(error),
            }
        }
        Request::Pair { machine: false } => {
            match crate::phone_access::serve_pairing_start().await {
                Ok((code, expires_at)) => {
                    // Never the code: the caller prints it once.
                    elog("[serve] control: a pairing window opened (10 minutes or one use)");
                    Answer {
                        code: Some(code),
                        expires_at: Some(expires_at),
                        ..Answer::done()
                    }
                }
                Err(error) => Answer::failed(error),
            }
        }
        Request::EnablePhone | Request::DisablePhone => {
            let enabled = request == Request::EnablePhone;
            match crate::phone_access::serve_set_enabled(enabled).await {
                Ok((listening, bind_error)) => {
                    elog(&format!(
                        "[serve] control: phone API {}",
                        if enabled { "on" } else { "off" }
                    ));
                    match bind_error.filter(|_| enabled && !listening) {
                        Some(error) => Answer {
                            listening: Some(false),
                            ..Answer::failed(format!(
                                "turned on, but the phone API could not listen: {error}"
                            ))
                        },
                        None => Answer {
                            listening: Some(listening),
                            ..Answer::done()
                        },
                    }
                }
                Err(error) => Answer::failed(error),
            }
        }
        Request::SetDefaultMode { mode, agent } => {
            let checked = crate::phone_access::checked_default_mode(&mode, agent.as_deref());
            let (mode, agent) = match checked {
                Ok(checked) => checked,
                Err(error) => return Answer::failed(error),
            };
            match crate::phone_access::serve_set_default_mode(mode.clone(), agent).await {
                Ok(()) => {
                    elog(&format!("[serve] control: new phone chats start in {mode}"));
                    Answer::done()
                }
                Err(error) => Answer::failed(error),
            }
        }
        Request::RunnerCoo { on } => {
            let status = match on {
                Some(on) => {
                    let switched = crate::runner_coo::switch(on);
                    if switched.is_ok() {
                        elog(&format!(
                            "[serve] control: runner COO {}",
                            if on { "on" } else { "off" }
                        ));
                    }
                    switched
                }
                None => Ok(crate::runner_coo::status()),
            };
            match status {
                Ok(status) => Answer {
                    role: Some(status.role.as_str().into()),
                    chat_key: status.chat_key,
                    record_error: status.record_error,
                    ..Answer::done()
                },
                Err(error) => Answer::failed(error),
            }
        }
    }
}

// ── The socket ───────────────────────────────────────────────────────────

/// The bound control socket; its file goes when this drops.
pub(crate) struct Control {
    path: PathBuf,
}

impl Drop for Control {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(crate) fn socket_path(dir: &Path) -> PathBuf {
    dir.join(SOCKET_FILE)
}

/// Bind `<dir>/serve.sock` (the caller owns the data folder, so a file
/// there is a stale socket of its own) with mode 0600. It is never wider:
/// bound inside a fresh 0700 folder of its own, set to 0600 there, then
/// renamed into place. (Not a `umask`: that is process wide, and serve's
/// worker threads are already making files and folders.)
pub(crate) fn bind(dir: &Path) -> Result<(std::os::unix::net::UnixListener, Control), String> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let path = socket_path(dir);
    let stage = dir.join(format!("{STAGE_PREFIX}{}", std::process::id()));
    let staged = stage.join(SOCKET_FILE);
    // The staging folder's name adds its own bytes to the limit.
    let extra = staged.as_os_str().len() - path.as_os_str().len();
    if path.as_os_str().as_bytes().len() + extra >= SUN_PATH_MAX {
        return Err(format!(
            "the control socket path {} is too long for a Unix socket (at most {} bytes here); use a shorter --data-dir",
            path.display(),
            SUN_PATH_MAX - 1 - extra
        ));
    }
    // Every stage a killed serve left, whatever its pid: this one owns the
    // folder.
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(STAGE_PREFIX)
            {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&stage)
        .map_err(|e| format!("create {}: {e}", stage.display()))?;
    let bound = (|| -> Result<std::os::unix::net::UnixListener, String> {
        let listener = std::os::unix::net::UnixListener::bind(&staged)
            .map_err(|e| format!("bind {}: {e}", staged.display()))?;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("chmod {}: {e}", staged.display()))?;
        // Over any stale socket of this folder's.
        std::fs::rename(&staged, &path).map_err(|e| format!("place {}: {e}", path.display()))?;
        Ok(listener)
    })();
    let _ = std::fs::remove_dir_all(&stage);
    Ok((bound?, Control { path }))
}

/// Answer requests on `listener` with `handler`, on the current runtime,
/// for the life of the process.
pub(crate) fn start(
    listener: std::os::unix::net::UnixListener,
    handler: Handler,
) -> Result<(), String> {
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("control socket: {e}"))?;
    let listener =
        tokio::net::UnixListener::from_std(listener).map_err(|e| format!("control socket: {e}"))?;
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            };
            let handler = handler.clone();
            tokio::spawn(async move {
                let _ = answer_one(stream, handler).await;
            });
        }
    });
    Ok(())
}

/// Whether the connecting process runs as this process's user.
fn same_user(stream: &tokio::net::UnixStream) -> bool {
    // SAFETY: geteuid has no preconditions.
    let me = unsafe { libc::geteuid() };
    stream.peer_cred().is_ok_and(|cred| cred.uid() == me)
}

async fn answer_one(mut stream: tokio::net::UnixStream, handler: Handler) -> std::io::Result<()> {
    use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _};
    if !same_user(&stream) {
        elog("[serve] control: refused a connection from another user");
        return Ok(());
    }
    let mut line = String::new();
    {
        let (read, _) = stream.split();
        let mut limited = tokio::io::BufReader::new(read.take(REQUEST_MAX));
        match tokio::time::timeout(READ_WAIT, limited.read_line(&mut line)).await {
            Ok(Ok(_)) => {}
            _ => return Ok(()),
        }
    }
    let answer = match serde_json::from_str::<Request>(line.trim()) {
        Ok(request) => handler(request).await,
        Err(_) => Answer::failed("serve did not understand that request"),
    };
    let mut text = serde_json::to_string(&answer).unwrap_or_else(|_| "{}".into());
    text.push('\n');
    stream.write_all(text.as_bytes()).await?;
    stream.shutdown().await
}

/// Send `request` to the serve whose socket is at `path`.
pub(crate) fn ask(path: &Path, request: &Request) -> Result<Answer, String> {
    let mut stream = std::os::unix::net::UnixStream::connect(path)
        .map_err(|e| format!("could not reach hollow serve at {}: {e}", path.display()))?;
    let _ = stream.set_read_timeout(Some(ANSWER_WAIT));
    let _ = stream.set_write_timeout(Some(ANSWER_WAIT));
    let mut text = serde_json::to_string(request).map_err(|e| e.to_string())?;
    text.push('\n');
    stream
        .write_all(text.as_bytes())
        .map_err(|e| format!("hollow serve: {e}"))?;
    let mut reply = String::new();
    std::io::BufReader::new(stream)
        .take(REQUEST_MAX * 4)
        .read_line(&mut reply)
        .map_err(|e| format!("hollow serve did not answer: {e}"))?;
    serde_json::from_str(reply.trim()).map_err(|_| "hollow serve's answer was not readable".into())
}

// ── The commands ─────────────────────────────────────────────────────────

/// A command line after `hollow serve <command>`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Command {
    pub data_dir: Option<PathBuf>,
    pub what: What,
    /// `pair --json`: one JSON line on stdout.
    pub json: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum What {
    SecretSet(String),
    SecretList,
    SecretDelete(String),
    Phone(Request),
}

/// Parse `hollow serve <command> <args>`.
pub(crate) fn parse(command: &str, args: &[String]) -> Result<Command, String> {
    let usage = |text: &str| format!("hollow serve {command}: {text}. Run `hollow serve --help`.");
    let mut data_dir = None;
    let mut agent = None;
    let mut machine = false;
    let mut json = false;
    let mut words = Vec::new();
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
                .ok_or_else(|| usage(&format!("{name} needs a value")))
        };
        match flag {
            "--data-dir" => {
                let dir = PathBuf::from(value("--data-dir")?);
                if !dir.is_absolute() {
                    return Err(usage("--data-dir must be an absolute path"));
                }
                data_dir = Some(dir);
            }
            "--agent" if command == "set-default-mode" => agent = Some(value("--agent")?),
            "--machine" if command == "pair" => machine = true,
            "--json" if command == "pair" => json = true,
            other if other.starts_with('-') => {
                return Err(usage(&format!("unknown option `{other}`")))
            }
            word => words.push(word.to_string()),
        }
    }
    let one = |words: &[String], what: &str| match words {
        [word] => Ok(word.clone()),
        [] => Err(usage(&format!("name the {what}"))),
        _ => Err(usage(&format!("takes one {what}"))),
    };
    let none = |words: &[String]| {
        if words.is_empty() {
            Ok(())
        } else {
            Err(usage("takes no arguments"))
        }
    };
    let what = match command {
        "secret-set" => {
            if words.len() > 1 {
                return Err(usage(
                    "takes the key only; it reads the value from stdin, never the command line",
                ));
            }
            What::SecretSet(checked_key(&one(&words, "key")?).map_err(|e| usage(&e))?)
        }
        "secret-list" => none(&words).map(|_| What::SecretList)?,
        "secret-delete" => {
            What::SecretDelete(checked_key(&one(&words, "key")?).map_err(|e| usage(&e))?)
        }
        "pair" => none(&words).map(|_| What::Phone(Request::Pair { machine }))?,
        "enable-phone" => none(&words).map(|_| What::Phone(Request::EnablePhone))?,
        "disable-phone" => none(&words).map(|_| What::Phone(Request::DisablePhone))?,
        "runner-coo" => {
            let on = match one(&words, "on, off or status")?.as_str() {
                "on" => Some(true),
                "off" => Some(false),
                "status" => None,
                other => return Err(usage(&format!("takes on, off or status, not `{other}`"))),
            };
            What::Phone(Request::RunnerCoo { on })
        }
        "set-default-mode" => {
            let mode = one(&words, "mode")?;
            let (mode, agent) = crate::phone_access::checked_default_mode(&mode, agent.as_deref())
                .map_err(|e| usage(e.trim_end_matches('.')))?;
            What::Phone(Request::SetDefaultMode { mode, agent })
        }
        other => return Err(format!("hollow serve: unknown command `{other}`.")),
    };
    Ok(Command {
        data_dir,
        what,
        json,
    })
}

/// A secret's key: printable ASCII without spaces, as Hollow names them
/// (`claude:<account id>`, `tracker:<provider>`, `runner.github.token`).
fn checked_key(key: &str) -> Result<String, String> {
    if key.is_empty() || key.len() > 200 || !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("a key is printable ASCII with no spaces".into());
    }
    Ok(key.to_string())
}

/// `hollow serve <command> <args>`: the exit status.
pub(crate) fn main(command: &str, args: &[String]) -> i32 {
    let parsed = match parse(command, args) {
        Ok(parsed) => parsed,
        Err(message) => {
            elog(&message);
            return crate::serve::EXIT_USAGE;
        }
    };
    // The same folders serve uses with the same flag (`serve::main`).
    if let Some(dir) = &parsed.data_dir {
        std::env::set_var("HOLLOW_APP_DATA_DIR", dir);
        crate::instance::isolate_under(dir);
    }
    let dir = match crate::agent_settings::app_data_dir() {
        Ok(dir) => dir,
        Err(why) => {
            elog(&format!("hollow serve: no app data folder: {why}"));
            return crate::serve::EXIT_CONFIG;
        }
    };
    let mut out = std::io::stdout();
    let result = match parsed.what {
        What::SecretSet(key) => {
            crate::secret_store::use_file(dir.join(crate::secret_store::FILE_NAME));
            read_secret(&key).and_then(|value| save_secret(&key, value, &mut out))
        }
        What::SecretList => {
            crate::secret_store::use_file(dir.join(crate::secret_store::FILE_NAME));
            list_secrets(&mut out)
        }
        What::SecretDelete(key) => {
            crate::secret_store::use_file(dir.join(crate::secret_store::FILE_NAME));
            crate::secret_store::delete(&key)
                .map_err(|e| Failure::Failed(e.to_string()))
                .and_then(|()| say(&mut out, &format!("Removed {key}.")))
        }
        What::Phone(request) => phone(&dir, &request, parsed.json, &mut out),
    };
    match result {
        Ok(()) => 0,
        Err(Failure::Busy(message)) => {
            elog(&format!("hollow serve {command}: {message}"));
            crate::serve::EXIT_BUSY
        }
        Err(Failure::Failed(message)) => {
            elog(&format!("hollow serve {command}: {message}"));
            EXIT_FAILED
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// The app owns the data folder (exit 75).
    Busy(String),
    Failed(String),
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Failure::Failed(message)
    }
}

fn say(out: &mut impl std::io::Write, text: &str) -> Result<(), Failure> {
    writeln!(out, "{text}").map_err(|e| Failure::Failed(format!("stdout: {e}")))
}

/// Read a secret's value: a prompt with echo off at a terminal, else all of
/// stdin. One trailing line break goes; an empty value is refused.
fn read_secret(key: &str) -> Result<String, Failure> {
    let stdin = std::io::stdin();
    // SAFETY: isatty only inspects descriptor 0.
    let terminal = unsafe { libc::isatty(0) } == 1;
    let raw = if terminal {
        // Never read a secret with echo on.
        let _quiet = EchoOff::new().ok_or_else(|| {
            Failure::Failed(
                "could not turn the terminal's echo off; pipe the value in instead".into(),
            )
        })?;
        elog(&format!("Value for {key} (not shown), then Return:"));
        let mut line = String::new();
        stdin
            .lock()
            .take(VALUE_MAX)
            .read_line(&mut line)
            .map_err(|e| Failure::Failed(format!("stdin: {e}")))?;
        line
    } else {
        read_value(stdin.lock())?
    };
    Ok(raw)
}

/// All of `input` (at most [`VALUE_MAX`] bytes), as text.
pub(crate) fn read_value(input: impl std::io::Read) -> Result<String, Failure> {
    let mut bytes = Vec::new();
    input
        .take(VALUE_MAX + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Failure::Failed(format!("stdin: {e}")))?;
    if bytes.len() as u64 > VALUE_MAX {
        return Err(Failure::Failed("the value is over 64 KiB".into()));
    }
    String::from_utf8(bytes).map_err(|_| Failure::Failed("the value is not UTF-8 text".into()))
}

/// Save `value` under `key` in the store in use. A Claude account token is
/// checked as Settings checks one.
pub(crate) fn save_secret(
    key: &str,
    value: String,
    out: &mut impl std::io::Write,
) -> Result<(), Failure> {
    save_secret_with(key, value, out, crate::secret_store::set)
}

/// [`save_secret`] into the store `set` writes.
pub(crate) fn save_secret_with(
    key: &str,
    value: String,
    out: &mut impl std::io::Write,
    mut set: impl FnMut(&str, &str) -> Result<(), crate::secret_store::StoreError>,
) -> Result<(), Failure> {
    let value = value
        .strip_suffix("\r\n")
        .or_else(|| value.strip_suffix('\n'))
        .unwrap_or(&value)
        .to_string();
    let value = if key.starts_with("claude:") {
        crate::claude_accounts::normalize_token(&value)?
    } else {
        value
    };
    if value.trim().is_empty() {
        return Err(Failure::Failed(
            "the value is empty; nothing was saved".into(),
        ));
    }
    set(key, &value).map_err(|e| Failure::Failed(e.to_string()))?;
    say(out, &format!("Saved {key}."))
}

fn list_secrets(out: &mut impl std::io::Write) -> Result<(), Failure> {
    let keys = crate::secret_store::keys().map_err(|e| Failure::Failed(e.to_string()))?;
    if keys.is_empty() {
        return say(out, "No secrets saved.");
    }
    for key in keys {
        say(out, &key)?;
    }
    Ok(())
}

/// The terminal settings [`EchoOff`] restores, for its SIGINT handler too.
static SAVED_TERMINAL: std::sync::OnceLock<libc::termios> = std::sync::OnceLock::new();

/// Ctrl-C at the secret prompt: put echo back, then die of SIGINT as usual.
extern "C" fn restore_and_reraise(signal: libc::c_int) {
    // SAFETY: tcsetattr, signal and raise are async-signal-safe; the
    // settings were stored before this handler was installed.
    unsafe {
        if let Some(saved) = SAVED_TERMINAL.get() {
            libc::tcsetattr(0, libc::TCSANOW, saved);
        }
        libc::signal(signal, libc::SIG_DFL);
        libc::raise(signal);
    }
}

/// Terminal echo off until dropped (the secret prompt), with SIGINT
/// restoring it first. `None` when echo could not be turned off.
struct EchoOff {
    saved: libc::termios,
    previous: libc::sigaction,
}

impl EchoOff {
    fn new() -> Option<Self> {
        // SAFETY: tcgetattr/tcsetattr on descriptor 0 with a zeroed struct
        // they fill, and sigaction with a handler that only restores it.
        unsafe {
            let mut saved: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(0, &mut saved) != 0 {
                return None;
            }
            let _ = SAVED_TERMINAL.set(saved);
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = restore_and_reraise as *const () as usize;
            let mut previous: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(libc::SIGINT, &action, &mut previous) != 0 {
                return None;
            }
            let mut quiet = saved;
            quiet.c_lflag &= !libc::ECHO;
            if libc::tcsetattr(0, libc::TCSANOW, &quiet) != 0 {
                libc::sigaction(libc::SIGINT, &previous, std::ptr::null_mut());
                return None;
            }
            Some(Self { saved, previous })
        }
    }
}

impl Drop for EchoOff {
    fn drop(&mut self) {
        // SAFETY: restores the settings and handler read in `new`.
        unsafe {
            libc::tcsetattr(0, libc::TCSANOW, &self.saved);
            libc::sigaction(libc::SIGINT, &self.previous, std::ptr::null_mut());
        }
        elog("");
    }
}

/// A phone command: through the running serve, else on disk.
///
/// The folder's `gui.lock` decides, not the marker: free means nothing runs
/// there whatever a stale `serve.json` says (the lock is then held for the
/// edit, so nothing starts meanwhile); held with a live serve's marker
/// means that serve; held without one means the app (or another Hollow).
fn phone(
    dir: &Path,
    request: &Request,
    json: bool,
    out: &mut impl std::io::Write,
) -> Result<(), Failure> {
    let _lock = match crate::launch::acquire(&dir.join(crate::launch::LOCK_FILE), Duration::ZERO) {
        crate::launch::Acquire::First(lock) => lock,
        crate::launch::Acquire::Running => {
            if let Some(pid) = crate::serve::running_in(dir) {
                let answer = ask(&socket_path(dir), request).map_err(|e| {
                    Failure::Failed(format!("hollow serve (pid {pid}) runs here, but {e}"))
                })?;
                return print_answer_as(request, &answer, json, out);
            }
            return Err(Failure::Busy(format!(
                "another Hollow process holds the folder {} (the app, or a hollow serve that is starting or stopping); try again in a moment, or change this in the app's Settings (Devices, Companion devices, iPhone app)",
                dir.display()
            )));
        }
        crate::launch::Acquire::Unavailable(why) => return Err(Failure::Failed(why)),
    };
    if matches!(request, Request::Pair { .. }) {
        return Err(Failure::Failed(format!(
            "hollow serve is not running on {}; pairing needs it (start it, then run this again)",
            dir.display()
        )));
    }
    match request {
        Request::EnablePhone | Request::DisablePhone => {
            let on = *request == Request::EnablePhone;
            crate::phone_access::offline_set_enabled(on)?;
            say(
                out,
                &format!(
                    "The phone API is {}; hollow serve {} it when it starts.",
                    if on { "on" } else { "off" },
                    if on { "starts" } else { "leaves" }
                ),
            )
        }
        Request::SetDefaultMode { mode, agent } => {
            crate::phone_access::offline_set_default_mode(mode, agent.as_deref())?;
            say(out, &default_mode_line(mode, agent.as_deref()))
        }
        Request::RunnerCoo { on } => runner_offline(dir, *on, out),
        Request::Pair { .. } => unreachable!("handled above"),
    }
}

/// `runner-coo` while nothing runs on the data folder (its lock held): the
/// record on disk, which serve reads when it starts.
pub(crate) fn runner_offline(
    dir: &Path,
    on: Option<bool>,
    out: &mut impl std::io::Write,
) -> Result<(), Failure> {
    match on {
        Some(on) => {
            crate::runner_coo::write(dir, on, chrono::Utc::now())?;
            say(
                out,
                &format!(
                    "The runner COO is {}; hollow serve {} when it starts.",
                    if on { "on" } else { "off" },
                    if on { "runs it" } else { "leaves it off" }
                ),
            )
        }
        None => match crate::runner_coo::read(dir) {
            Ok(record) => say(
                out,
                &format!(
                    "Runner COO: {} (hollow serve is not running here).",
                    if record.on { "on" } else { "off" }
                ),
            ),
            Err(error) => say(
                out,
                &format!("Runner COO: off, {error}; `runner-coo on` or `off` rewrites it (hollow serve is not running here)."),
            ),
        },
    }
}

/// What `set-default-mode` stored, and what each agent's new chat gets from
/// it (`agent::inherited_mode`, as the phone's new chat reads it): without
/// `--agent` the mode is read the most restrictive way for every agent.
pub(crate) fn default_mode_line(mode: &str, agent: Option<&str>) -> String {
    let from = agent.and_then(crate::agent::AgentKind::from_opt);
    let each: Vec<String> = crate::agent::AgentKind::all()
        .into_iter()
        .map(|kind| {
            format!(
                "{} {}",
                kind.as_str(),
                crate::agent::inherited_mode(from, kind, mode)
            )
        })
        .collect();
    let set = match agent {
        Some(agent) => format!("Stored {mode} for new phone chats, as set for {agent}."),
        None => format!(
            "Stored {mode} for new phone chats, with no --agent: read the most restrictive way for every agent."
        ),
    };
    format!(
        "{set}\nEach agent's new chat starts in: {}.",
        each.join(", ")
    )
}

/// What a command prints for serve's answer. The pairing code goes to
/// `out` once and nowhere else.
#[cfg(test)]
pub(crate) fn print_answer(
    request: &Request,
    answer: &Answer,
    out: &mut impl std::io::Write,
) -> Result<(), Failure> {
    print_answer_as(request, answer, false, out)
}

/// `pair --json`'s line: the code and when it closes, and for a machine
/// window which Mac this is.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PairLine<'a> {
    code: &'a str,
    expires_at: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    machine_id: Option<&'a str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    machine: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    slug: Option<Option<&'a str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
}

/// [`print_answer`], as one JSON line for `pair --json`.
pub(crate) fn print_answer_as(
    request: &Request,
    answer: &Answer,
    json: bool,
    out: &mut impl std::io::Write,
) -> Result<(), Failure> {
    if !answer.ok {
        return Err(Failure::Failed(
            answer
                .error
                .clone()
                .unwrap_or_else(|| "hollow serve refused that".into()),
        ));
    }
    match request {
        Request::Pair { machine } => {
            let code = answer
                .code
                .as_deref()
                .ok_or_else(|| Failure::Failed("hollow serve sent no code".into()))?;
            if *machine && !answer.machine {
                return Err(Failure::Failed(
                    "the running hollow serve opened no machine window (it is older than the peer protocol); update it, then run this again".into(),
                ));
            }
            if json {
                let line = PairLine {
                    code,
                    expires_at: answer.expires_at.as_deref(),
                    machine: *machine,
                    machine_id: answer.machine_id.as_deref().filter(|_| *machine),
                    slug: machine.then_some(answer.slug.as_deref()),
                    name: answer.name.as_deref().filter(|_| *machine),
                };
                let text =
                    serde_json::to_string(&line).map_err(|e| Failure::Failed(e.to_string()))?;
                return say(out, &text);
            }
            if *machine {
                say(
                    out,
                    &format!(
                        "Machine pairing code for {}: {code}",
                        answer.name.as_deref().unwrap_or("this Mac")
                    ),
                )?;
            } else {
                say(out, &format!("Pairing code: {code}"))?;
            }
            say(
                out,
                &format!(
                    "Open until {} (10 minutes or one use).",
                    answer
                        .expires_at
                        .as_deref()
                        .unwrap_or("10 minutes from now")
                ),
            )
        }
        Request::EnablePhone | Request::DisablePhone => say(
            out,
            if answer.listening == Some(true) {
                "The phone API is on and listening."
            } else {
                "The phone API is off."
            },
        ),
        Request::SetDefaultMode { mode, agent } => {
            say(out, &default_mode_line(mode, agent.as_deref()))
        }
        Request::RunnerCoo { .. } => say(out, &runner_line(answer)),
    }
}

/// The running serve's `runner-coo` answer, for the terminal.
pub(crate) fn runner_line(answer: &Answer) -> String {
    match (
        answer.role.as_deref(),
        answer.chat_key.as_deref(),
        answer.record_error.as_deref(),
    ) {
        (Some("runner"), Some(key), _) => format!("Runner COO: on, chat {key}."),
        (Some("runner"), None, _) => {
            "Runner COO: on, no thread yet (its first wake or message starts one).".into()
        }
        (_, _, Some(error)) => {
            format!("Runner COO: off, {error}; `runner-coo on` or `off` rewrites it.")
        }
        _ => "Runner COO: off.".into(),
    }
}

#[cfg(test)]
#[path = "serve_control_tests.rs"]
mod tests;
