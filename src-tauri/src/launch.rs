//! What the `hollow` binary does with its command line, and with a second
//! launch (2026-11-11).
//!
//! Three crashes on 2026-11-11 were child processes of the installed app
//! whose command lines an agent had typed:
//!
//! 1. `Hollow.app/Contents/MacOS/hollow --version` was not a command, so it
//!    started a second full app. Its `shell-path` thread then wrote a log line
//!    with `eprintln!` after the agent's shell had gone: stderr was a pipe
//!    with no reader, `eprintln!` panics on that error, and the release
//!    profile is `panic = "abort"`, so the whole process aborted (SIGABRT 0.4 s
//!    after launch; the same shape at 12:33 PM). [`elog`] never panics.
//! 2. The same binary run from zsh ran as a second app against the running
//!    app's data and aborted in `applicationDidFinishLaunching` 42 s later.
//! 3. (Not this binary: Apple's Tk `Wish.app`, see `docs/AGENT_CONTEXT.md`.)
//!
//! So now: `--version` and `--help` answer; any other command or option
//! prints one message to stderr and exits 2 without starting anything; and a
//! plain launch takes a per-instance lock first (`gui.lock` in the app data
//! folder, so a candidate or `HOLLOW_APP_DATA_DIR` build has its own). A
//! launch that finds the lock held, after a short grace for an app that is
//! quitting for a relaunch, hands off to the running app (`open -b <id>` from
//! inside a bundle) and exits 0. Nothing in this module can abort.

use crate::flock::HeldLock;
use std::fs::{OpenOptions, TryLockError};
use std::io::Write as _;
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The hidden subcommands `main.rs` dispatches before the app starts, each
/// with a one-line purpose for `--help`. A test keeps this list and
/// `main.rs`'s arms equal.
pub(crate) const COMMANDS: &[(&str, &str)] = &[
    (
        "app-mcp",
        "MCP server: Hollow's own app tools, for agent runs",
    ),
    (
        "claude-defer-hook",
        "PreToolUse hook of an ask-first Claude turn: defers a tool call until its approval card is answered",
    ),
    (
        "code-context",
        "the code-context tools on the command line (hollow code-context --help)",
    ),
    (
        "code-context-mcp",
        "MCP server: code context (callers, impact) for agent runs",
    ),
    (
        "companion-apply",
        "debug builds: one companion applier pass over a temp vault",
    ),
    ("companion-bridge-once", "one companion bridge exchange"),
    (
        "companion-fixtures",
        "write the companion apps' golden fixtures into a folder",
    ),
    (
        "computer",
        "the desktop tools on the command line (hollow computer --help)",
    ),
    (
        "computer-mcp",
        "MCP server: vetted desktop verbs, for agent runs",
    ),
    ("codex-relay", "private inherited-fd relay for a shared agent server"),
    (
        "embed-selftest",
        "embed a probe with the bundled model and exit",
    ),
    (
        "install-update",
        "install a staged Hollow update, or go back to the previous one (run detached)",
    ),
    ("jev-study", "the agent-effectiveness study runner"),
    (
        "m365-mcp",
        "MCP server: the built-in Microsoft 365 connector",
    ),
    (
        "mcp-launch",
        "start a Codex run's MCP server from its private config file",
    ),
    (
        "move-home",
        "Move Hollow home: the dry run (default), or the helper that runs or rolls back a staged move",
    ),
    (
        "questions-mcp",
        "MCP server: question cards, for agent runs",
    ),
    (
        "reaper",
        "ends a run's leftovers when Hollow exits (started by the app)",
    ),
    (
        "remote-mcp",
        "MCP server: a remote connector on Hollow's own sign-in",
    ),
    (
        "serve",
        "Hollow's core with no window: queue, automations, COO, phone API (hollow serve --help)",
    ),
    ("services-mcp", "MCP server: services, for agent runs"),
    ("tasks-mcp", "MCP server: tasks, for agent runs"),
    ("team-mcp", "MCP server: sub-agents, for agent runs"),
    (
        "tracker-import",
        "the one-time Notion-to-vault importer and tracker rekey",
    ),
    (
        "vault-mcp",
        "MCP server: vault search and source reads, for agent runs",
    ),
];

/// How long a launch waits for the lock before treating the holder as a
/// running app: an app that is quitting for a relaunch lets go within this.
const GRACE: Duration = Duration::from_secs(8);
pub(crate) const LOCK_FILE: &str = "gui.lock";

/// What a command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Launch {
    /// A plain app launch (no arguments, or only the flags the app itself
    /// and macOS pass: `--hidden`, `-psn_…`, `-NS…`).
    Gui,
    Version,
    Help,
    /// One of [`COMMANDS`] (`main.rs` handles these before it gets here).
    Command,
    /// Anything else, with the message to print.
    Reject(String),
}

/// Classify the arguments after the program name.
pub(crate) fn classify(args: &[String]) -> Launch {
    if let Some(first) = args.first() {
        if COMMANDS.iter().any(|(name, _)| name == first) {
            return Launch::Command;
        }
    }
    // `--help` and `--version` win anywhere, so `hollow --hidden --version`
    // answers instead of starting the app.
    if args.iter().any(|a| a == "--help" || a == "-h") || args.first().is_some_and(|a| a == "help")
    {
        return Launch::Help;
    }
    if args.iter().any(|a| a == "--version" || a == "-V")
        || args.first().is_some_and(|a| a == "version")
    {
        return Launch::Version;
    }
    let mut rest = args.iter().peekable();
    while let Some(arg) = rest.next() {
        if arg == "--hidden" {
            continue;
        }
        // macOS and Cocoa's own single-dash flags, with an optional value.
        if arg.starts_with('-') && !arg.starts_with("--") {
            if rest.peek().is_some_and(|next| !next.starts_with('-')) {
                rest.next();
            }
            continue;
        }
        return Launch::Reject(format!(
            "hollow: unknown command or option `{arg}`.\n\
             Run `hollow --help` to list the commands. Nothing was started."
        ));
    }
    Launch::Gui
}

/// The text of `hollow --version`: one line, the same identity the About
/// panel and the tray show.
pub(crate) fn version_text() -> String {
    crate::app_menu::build_identity().text
}

/// The text of `hollow --help`.
pub(crate) fn help_text() -> String {
    let mut text = String::from(
        "Hollow: a local workspace for projects, tasks, notes, recordings and Calendar.\n\n\
         Usage:\n  \
         hollow                 start the app (one at a time: a second launch brings the running one forward)\n  \
         hollow --version       print the build and exit\n  \
         hollow --help          print this and exit\n  \
         hollow <command> ...   run one command below without starting the app\n\n\
         The commands are Hollow's own processes and tools, started by the app and its\n\
         agents; most are not meant to be typed. Any other command or option is an error:\n\
         one message on stderr, exit status 2, and the app is not started.\n\n\
         Commands:\n",
    );
    for (name, purpose) in COMMANDS {
        text.push_str(&format!("  {name:<22} {purpose}\n"));
    }
    text
}

/// Write one line to stderr and never panic. `eprintln!` panics when stderr
/// is a pipe nobody reads any more (an agent's shell that has exited), and
/// the release profile turns that panic into an abort of the whole app:
/// background threads and launch messages log through here instead.
pub(crate) fn elog(message: &str) {
    let _ = writeln!(std::io::stderr().lock(), "{message}");
}

/// A line on stdout; a closed pipe (`hollow --version | head -0`) is not an
/// error.
fn say(text: &str) {
    let _ = writeln!(std::io::stdout().lock(), "{}", text.trim_end());
}

/// The app's lock, held for the life of the process.
static GUI_LOCK: OnceLock<HeldLock> = OnceLock::new();

/// Whether this launch may go on to start the app. `main.rs` calls it once
/// no command matched: `Some(code)` is the exit status to leave with (the
/// answer or the refusal was already printed); `None` means start the app.
pub(crate) fn gate(args: &[String]) -> Option<i32> {
    match classify(args) {
        Launch::Version => {
            say(&version_text());
            Some(0)
        }
        Launch::Help => {
            say(&help_text());
            Some(0)
        }
        Launch::Reject(message) => {
            elog(&message);
            Some(2)
        }
        // `main.rs` dispatches these first; reaching here means a command is
        // listed but not wired, so refuse rather than start the app.
        Launch::Command => {
            elog(
                "hollow: that command is listed but not wired in this build. Nothing was started.",
            );
            Some(2)
        }
        Launch::Gui => single_instance(),
    }
}

/// `Some(0)` when another app already holds this instance's lock.
fn single_instance() -> Option<i32> {
    if std::env::var_os("HOLLOW_ALLOW_SECOND_INSTANCE").is_some_and(|v| v == "1") {
        return None;
    }
    let dir = match crate::agent_settings::app_data_dir() {
        Ok(dir) => dir,
        Err(why) => {
            elog(&format!(
                "[launch] no app data folder for the instance lock: {why}"
            ));
            return None;
        }
    };
    match acquire(&dir.join(LOCK_FILE), GRACE) {
        Acquire::First(lock) => {
            let _ = GUI_LOCK.set(lock);
            // Holding the lock means no serve runs here, so a `serve.json`
            // is a crashed or killed serve's: gone, or a later second launch
            // would read a reused pid as a serve and refuse to hand off.
            let _ = std::fs::remove_file(dir.join(crate::serve::MARKER_FILE));
            None
        }
        Acquire::Running => {
            // `hollow serve` holds the same lock (M1 PR 2) but has no window
            // to bring forward, and `open -b` would only start this app again.
            if let Some(pid) = crate::serve::running_in(&dir) {
                elog(&format!(
                    "hollow: `hollow serve` (pid {pid}) is running Hollow's core on this data folder ({}), so the app was not started. Stop it first.",
                    dir.display()
                ));
                return Some(2);
            }
            hand_off();
            Some(0)
        }
        // A folder we can't write must not stop the app from starting.
        Acquire::Unavailable(why) => {
            elog(&format!(
                "[launch] instance lock unavailable, starting anyway: {why}"
            ));
            None
        }
    }
}

pub(crate) enum Acquire {
    First(HeldLock),
    Running,
    Unavailable(String),
}

/// Take the instance lock at `path`, retrying for `grace` while another
/// process holds it (an app that is quitting for a relaunch).
pub(crate) fn acquire(path: &Path, grace: Duration) -> Acquire {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let deadline = Instant::now() + grace;
    loop {
        let file = match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
        {
            Ok(file) => file,
            Err(error) => return Acquire::Unavailable(error.to_string()),
        };
        match HeldLock::try_take(file) {
            Ok(lock) => return Acquire::First(lock),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(TryLockError::WouldBlock) => return Acquire::Running,
            Err(TryLockError::Error(error)) => return Acquire::Unavailable(error.to_string()),
        }
    }
}

/// Tell the person, and bring the running app forward when this binary sits
/// in a bundle (`open -b` asks LaunchServices, which activates the one that
/// is running instead of starting another).
fn hand_off() {
    elog("hollow: Hollow is already running, so a second copy was not started.");
    #[cfg(target_os = "macos")]
    {
        // A custom data folder (a review run) is not the everyday app, whose
        // bundle id `open -b` would bring forward, or start.
        let identifier = std::env::current_exe()
            .ok()
            .filter(|_| std::env::var_os("HOLLOW_APP_DATA_DIR").is_none())
            .and_then(|exe| crate::instance::bundle_identifier(&exe));
        if let Some(identifier) = identifier {
            let _ = std::process::Command::new("/usr/bin/open")
                .args(["-b", &identifier])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
}

#[cfg(test)]
#[path = "launch_tests.rs"]
mod tests;
