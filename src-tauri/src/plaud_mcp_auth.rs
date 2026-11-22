//! The built-in Plaud connector's sign-in (overnight 2026-10-13 S8).
//!
//! The built-in Plaud row (`builtin_connectors::PLAUD`) runs Plaud's own
//! installed MCP server, `plaud-mcp` (`@plaud-ai/mcp`). That server keeps
//! its **own** OAuth sign-in in `~/.plaud/tokens-mcp.json`, separate from the
//! `plaud` CLI's `~/.plaud/tokens.json` that Recordings use (`plaud.rs`), and
//! it has no login command: `plaud-mcp install` / `setup` rewrite other
//! apps' agent configs, so Hollow never runs them. Sign-in happens through
//! the server's MCP `login` tool (it opens the browser and waits up to two
//! minutes for the OAuth callback on port 8199), sign-out through `logout`.
//!
//! So Hollow speaks just enough MCP over stdio to call one tool: start the
//! server in its own process group, `initialize`, `notifications/initialized`,
//! one `tools/call`, read the answer, end the group. Bounded by a wait; on
//! a timeout the answer says so in words.
//!
//! Ready = the tokens file holds an access token and `plaud-mcp` is on the
//! tool PATH a connector gets (`self_install::tool_path`). No network: an
//! expired access token is the server's to refresh.
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// The server's program name.
pub const PROGRAM: &str = "plaud-mcp";
/// Its npm package, for a connector that runs it through `npx`.
pub const PACKAGE: &str = "@plaud-ai/mcp";
/// How long Hollow waits for a sign-in: the person is in the browser. The
/// server's own limit (two minutes) usually answers first.
pub const SIGN_IN_WAIT: Duration = Duration::from_secs(5 * 60);
const SIGN_OUT_WAIT: Duration = Duration::from_secs(30);
/// What to install, in the words the row and errors use.
pub const INSTALL_HINT: &str = "npm install -g @plaud-ai/mcp";

/// `~/.plaud/tokens-mcp.json`.
pub fn tokens_path(home: &Path) -> PathBuf {
    home.join(".plaud/tokens-mcp.json")
}

/// Whether the server's tokens file holds an access token.
pub fn signed_in_at(home: &Path) -> bool {
    std::fs::read_to_string(tokens_path(home))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|v| {
            v.get("access_token")
                .and_then(Value::as_str)
                .map(|t| !t.is_empty())
        })
        .unwrap_or(false)
}

/// Where `plaud-mcp` is on the tool PATH (a Finder-launched Hollow has only
/// the system's).
pub fn find_program(inherited: Option<&str>, home: &Path) -> Option<PathBuf> {
    find_program_on(&crate::self_install::tool_path(inherited, home))
}

/// `plaud-mcp` on exactly this PATH (tests give one without the real
/// Homebrew folder, so they can never start the installed server).
pub fn find_program_on(path: &str) -> Option<PathBuf> {
    path.split(':')
        .filter(|d| !d.is_empty())
        .map(|d| Path::new(d).join(PROGRAM))
        .find(|p| p.is_file())
}

/// Signed in and installed (GUI only: reads the real home folder and PATH).
pub fn ready() -> bool {
    let home = dirs::home_dir().unwrap_or_default();
    signed_in_at(&home) && find_program(std::env::var("PATH").ok().as_deref(), &home).is_some()
}

/// Whether the server is installed (GUI only).
pub fn installed() -> bool {
    let home = dirs::home_dir().unwrap_or_default();
    find_program(std::env::var("PATH").ok().as_deref(), &home).is_some()
}

/// What sign-in and sign-out return.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub signed_in: bool,
    pub installed: bool,
    /// The server was signed in already ("Already logged in."): nothing
    /// opened in the browser.
    pub already: bool,
}

/// Whether the login tool answered that it was signed in already.
fn already(answer: &ToolAnswer) -> bool {
    !answer.is_error
        && answer
            .text
            .to_ascii_lowercase()
            .contains("already logged in")
}

/// A `tools/call` answer: its text parts, and whether the tool said it failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolAnswer {
    pub is_error: bool,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallError {
    /// The wait ran out; the server was stopped.
    TimedOut,
    /// It didn't start, stopped early, or answered something else.
    Failed(String),
}

/// Start `program` with `path` as its PATH, call `tool` with no arguments,
/// and stop it, all within `wait`.
pub async fn call_tool(
    program: &Path,
    path: &str,
    tool: &str,
    wait: Duration,
) -> Result<ToolAnswer, CallError> {
    use std::process::Stdio;
    let mut cmd = tokio::process::Command::new(program);
    cmd.env("PATH", path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // Its own group, so ending it also ends anything it started.
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd
        .spawn()
        .map_err(|e| CallError::Failed(format!("couldn't start it: {e}")))?;
    let pid = child.id();
    let answer = tokio::time::timeout(wait, talk(&mut child, tool)).await;
    if let Some(pid) = pid {
        crate::process_group::end_probe_group(pid);
    }
    let _ = child.kill().await;
    answer.unwrap_or(Err(CallError::TimedOut))
}

/// The conversation: `initialize`, `notifications/initialized`, one
/// `tools/call`. Anything that isn't the awaited answer (a notification, a
/// line too long to be one) is skipped.
async fn talk(child: &mut tokio::process::Child, tool: &str) -> Result<ToolAnswer, CallError> {
    use tokio::io::AsyncWriteExt;
    let early = || CallError::Failed("it stopped before it answered".into());
    let mut stdin = child.stdin.take().ok_or_else(early)?;
    let stdout = child.stdout.take().ok_or_else(early)?;
    let mut lines = crate::process_group::BoundedLines::new(stdout, 1 << 20);
    let messages = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "hollow", "version": env!("CARGO_PKG_VERSION")}}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": tool, "arguments": {}}}),
    ];
    for (i, message) in messages.iter().enumerate() {
        let mut line = message.to_string();
        line.push('\n');
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|_| early())?;
        stdin.flush().await.map_err(|_| early())?;
        let Some(id) = message.get("id").and_then(Value::as_u64) else {
            continue;
        };
        let reply = loop {
            match lines.next_checked().await {
                Ok(Some(crate::process_group::BoundedLine::Line(l))) => {
                    let Ok(v) = serde_json::from_str::<Value>(&l) else {
                        continue;
                    };
                    if v.get("id").and_then(Value::as_u64) == Some(id) && v.get("method").is_none()
                    {
                        break v;
                    }
                }
                Ok(Some(_)) => continue,
                Ok(None) | Err(_) => return Err(early()),
            }
        };
        if let Some(e) = reply.get("error") {
            let msg = e.get("message").and_then(Value::as_str).unwrap_or("");
            return Err(CallError::Failed(format!("it refused the request: {msg}")));
        }
        if i == messages.len() - 1 {
            let result = reply.get("result").cloned().unwrap_or(Value::Null);
            let text = result
                .get("content")
                .and_then(Value::as_array)
                .map(|parts| {
                    parts
                        .iter()
                        .filter_map(|p| p.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            let is_error = result
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            return Ok(ToolAnswer { is_error, text });
        }
    }
    Err(early())
}

/// How long `wait` is, in words.
fn in_words(wait: Duration) -> String {
    let s = wait.as_secs();
    if s >= 120 && s.is_multiple_of(60) {
        format!("{} minutes", s / 60)
    } else if s == 60 {
        "a minute".into()
    } else {
        let s = s.max(1);
        format!("{s} second{}", if s == 1 { "" } else { "s" })
    }
}

/// The `login` tool's answer, in the row's words.
fn sign_in_words(answer: &ToolAnswer) -> Result<(), String> {
    if !answer.is_error {
        return Ok(());
    }
    let low = answer.text.to_ascii_lowercase();
    Err(if low.contains("timed out") {
        // "Authentication timed out after 2 minutes. …<url>": keep the limit, drop the rest.
        let limit = answer
            .text
            .split("after ")
            .nth(1)
            .and_then(|r| r.split(['.', '\n']).next())
            .map(str::trim)
            .filter(|l| !l.is_empty() && l.len() < 30)
            .unwrap_or("its time limit");
        format!("The Plaud sign-in wasn't finished in the browser within {limit}. Sign in again when you're ready.")
    } else if low.contains("denied") {
        "The Plaud sign-in was declined in the browser, so nothing changed.".into()
    } else if low.contains("eaddrinuse") || low.contains("address already in use") {
        "Plaud couldn't start its sign-in: port 8199 on this Mac is busy (another Plaud sign-in may still be open). Close it, then sign in again.".into()
    } else {
        "Plaud couldn't finish signing in. Sign in again; if it keeps failing, check that plaud-mcp runs in Terminal.".into()
    })
}

/// A failed call, in words. `what` is "sign-in" or "sign-out".
fn call_words(e: CallError, what: &str, wait: Duration) -> String {
    match e {
        CallError::TimedOut => format!(
            "The Plaud {what} wasn't finished within {}, so Hollow stopped waiting. Try again when you're ready.",
            in_words(wait)
        ),
        CallError::Failed(why) if why.contains("stopped before") => format!(
            "Plaud's MCP server stopped before it answered, so the {what} didn't happen. Check that plaud-mcp runs in Terminal, then try again."
        ),
        CallError::Failed(_) => format!(
            "Hollow couldn't talk to Plaud's MCP server, so the {what} didn't happen. Check that plaud-mcp runs in Terminal, then try again."
        ),
    }
}

/// The program, or what to install.
fn program_or_hint(path: &str) -> Result<PathBuf, String> {
    find_program_on(path).ok_or_else(|| {
        format!("Plaud's MCP server (plaud-mcp) isn't on this Mac. Install it with `{INSTALL_HINT}`, then try again.")
    })
}

/// One Plaud browser sign-in at a time, across Hollow: this server's
/// `login` and Recordings' `plaud login` (`plaud.rs`) both listen on port
/// 8199, and a sign-out is refused while a sign-in is open (S8 review).
static PLAUD_SIGN_IN: AtomicBool = AtomicBool::new(false);

/// Holding the Plaud sign-in slot; freed however the holder ends.
#[derive(Debug)]
pub struct SignInSlot(&'static AtomicBool);

impl SignInSlot {
    /// Take the shared slot, or say why not.
    pub fn take() -> Result<Self, String> {
        Self::take_in(&PLAUD_SIGN_IN)
    }

    fn take_in(flag: &'static AtomicBool) -> Result<Self, String> {
        if flag.swap(true, Ordering::SeqCst) {
            return Err("A Plaud sign-in is already open (here or in Recordings). Finish or close it in your browser, then try again.".into());
        }
        Ok(Self(flag))
    }
}

impl Drop for SignInSlot {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Sign the server in: call its `login` tool and check the tokens file.
/// `path` is the whole PATH the server gets (and is found on).
pub async fn sign_in_with(home: &Path, path: &str, wait: Duration) -> Result<Status, String> {
    let program = program_or_hint(path)?;
    let answer = call_tool(&program, path, "login", wait)
        .await
        .map_err(|e| call_words(e, "sign-in", wait))?;
    sign_in_words(&answer)?;
    if !signed_in_at(home) {
        return Err("Plaud said the sign-in worked, but its sign-in file (~/.plaud/tokens-mcp.json) isn't there. Sign in again.".into());
    }
    Ok(Status {
        signed_in: true,
        installed: true,
        already: already(&answer),
    })
}

/// Sign the server out: call its `logout` tool (it also revokes the grant).
pub async fn sign_out_with(home: &Path, path: &str, wait: Duration) -> Result<Status, String> {
    let program = program_or_hint(path)?;
    let answer = call_tool(&program, path, "logout", wait)
        .await
        .map_err(|e| call_words(e, "sign-out", wait))?;
    if signed_in_at(home) {
        return Err(if answer.is_error {
            "Plaud couldn't sign out. Try again.".into()
        } else {
            "Plaud said it signed out, but its sign-in file (~/.plaud/tokens-mcp.json) is still there. Try again.".into()
        });
    }
    Ok(Status {
        signed_in: false,
        installed: true,
        already: false,
    })
}

// ── IPC ────────────────────────────────────────────────────────────────────

/// Sign the built-in Plaud connector in, in the browser (waits up to five
/// minutes; Plaud's own limit is two).
#[tauri::command]
pub async fn plaud_mcp_sign_in() -> Result<Status, String> {
    let home = dirs::home_dir().ok_or("No home folder")?;
    let _slot = SignInSlot::take()?;
    let path = crate::self_install::tool_path(std::env::var("PATH").ok().as_deref(), &home);
    sign_in_with(&home, &path, SIGN_IN_WAIT).await
}

/// Sign the built-in Plaud connector out.
#[tauri::command]
pub async fn plaud_mcp_sign_out() -> Result<Status, String> {
    let home = dirs::home_dir().ok_or("No home folder")?;
    let _slot = SignInSlot::take()?;
    let path = crate::self_install::tool_path(std::env::var("PATH").ok().as_deref(), &home);
    sign_out_with(&home, &path, SIGN_OUT_WAIT).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Server requests whose ids collide with the client's: only a response
    /// (no `method`) counts as the answer (S8 review).
    const COLLIDE_1: &str = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
    const INIT_REPLY: &str = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"plaud","version":"0"}}}"#;

    /// A fake `plaud-mcp` in `home/.local/bin` (on the tool PATH): it logs
    /// its arguments and what it is sent to `home/log`, answers
    /// `initialize`, then runs `after_call` once the tool call has arrived.
    fn fake_server(home: &Path, after_call: &str) -> PathBuf {
        let bin = home.join(".local/bin");
        fs::create_dir_all(&bin).unwrap();
        let log = home.join("log");
        let path = bin.join(PROGRAM);
        let script = format!(
            "#!/bin/sh\nLOG='{log}'\necho \"args:$*\" > \"$LOG\"\nread -r init\necho \"$init\" >> \"$LOG\"\nprintf '%s\\n' '{COLLIDE_1}'\nprintf '%s\\n' '{INIT_REPLY}'\nread -r initialized\necho \"$initialized\" >> \"$LOG\"\nread -r call\necho \"$call\" >> \"$LOG\"\n{after_call}\n",
            log = log.display()
        );
        fs::write(&path, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn reply(text: &str, is_error: bool) -> String {
        let body = json!({"jsonrpc": "2.0", "id": 2, "result": {
            "content": [{"type": "text", "text": text}], "isError": is_error}});
        // A log notification first: the client skips what isn't its answer.
        format!(
            "printf '%s\\n' '{{\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{{}}}}'\nprintf '%s\\n' '{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"roots/list\"}}'\nprintf '%s\\n' '{}'",
            body.to_string().replace('\'', "")
        )
    }

    /// The PATH a test's server gets: its own bin folder and the system's,
    /// never `/opt/homebrew/bin`, where the real `plaud-mcp` may be.
    fn test_path(home: &Path) -> String {
        format!("{}:/usr/bin:/bin", home.join(".local/bin").display())
    }

    fn write_tokens(home: &Path, body: &str) {
        fs::create_dir_all(home.join(".plaud")).unwrap();
        fs::write(tokens_path(home), body).unwrap();
    }

    #[test]
    fn ready_needs_an_access_token_in_its_own_file_and_the_program_on_the_tool_path() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        assert!(!signed_in_at(h), "no file");
        // The CLI's file is not the server's sign-in.
        write_tokens(h, "");
        fs::write(h.join(".plaud/tokens.json"), r#"{"access_token":"cli"}"#).unwrap();
        assert!(!signed_in_at(h), "empty file");
        for bad in ["{}", "not json", r#"{"access_token":""}"#, "[]"] {
            write_tokens(h, bad);
            assert!(!signed_in_at(h), "{bad}");
        }
        write_tokens(
            h,
            r#"{"access_token":"a","refresh_token":"r","expires_at":1,"token_type":"Bearer"}"#,
        );
        assert!(
            signed_in_at(h),
            "an expired token still counts: the server refreshes it"
        );

        assert_eq!(find_program_on(&test_path(h)), None);
        let program = fake_server(h, "");
        assert_eq!(find_program_on(&test_path(h)), Some(program));
        assert!(
            crate::self_install::tool_path(None, h).contains("/.local/bin"),
            "the real search covers the tool folders"
        );
    }

    #[tokio::test]
    async fn sign_in_calls_the_login_tool_over_stdio_and_checks_the_tokens_file() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        let tokens = tokens_path(h);
        fake_server(
            h,
            &format!(
                "mkdir -p '{dir}'\nprintf '%s' '{{\"access_token\":\"a\"}}' > '{file}'\n{reply}",
                dir = tokens.parent().unwrap().display(),
                file = tokens.display(),
                reply = reply("Successfully authenticated with Plaud!", false)
            ),
        );
        let status = sign_in_with(h, &test_path(h), Duration::from_secs(20))
            .await
            .unwrap();
        assert_eq!(
            status,
            Status {
                signed_in: true,
                installed: true,
                already: false,
            }
        );
        let log = fs::read_to_string(h.join("log")).unwrap();
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(
            lines[0], "args:",
            "never `install` or `setup`: no arguments at all"
        );
        assert!(lines[1].contains(r#""method":"initialize""#), "{log}");
        assert!(lines[2].contains("notifications/initialized"), "{log}");
        assert!(lines[3].contains(r#""method":"tools/call""#), "{log}");
        assert!(lines[3].contains(r#""name":"login""#), "{log}");
    }

    #[tokio::test]
    async fn sign_out_calls_the_logout_tool() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        write_tokens(h, r#"{"access_token":"a"}"#);
        fake_server(
            h,
            &format!(
                "rm -f '{}'\n{}",
                tokens_path(h).display(),
                reply("Logged out and revoked authorization.", false)
            ),
        );
        let status = sign_out_with(h, &test_path(h), Duration::from_secs(20))
            .await
            .unwrap();
        assert!(!status.signed_in);
        let log = fs::read_to_string(h.join("log")).unwrap();
        assert!(log.contains(r#""name":"logout""#), "{log}");
    }

    #[tokio::test]
    async fn a_sign_in_that_outlasts_the_wait_says_so_in_words_and_stops_the_server() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        fake_server(h, "sleep 30");
        let started = std::time::Instant::now();
        let err = sign_in_with(h, &test_path(h), Duration::from_millis(1500))
            .await
            .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(10), "bounded");
        assert!(err.contains("wasn't finished within"), "{err}");
        assert!(err.contains("stopped waiting"), "{err}");
        assert!(!err.contains("Error"), "words, not an error string: {err}");
    }

    #[tokio::test]
    async fn without_the_program_sign_in_says_what_to_install() {
        let home = tempfile::tempdir().unwrap();
        let err = sign_in_with(home.path(), "/nowhere", Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(err.contains(INSTALL_HINT), "{err}");
    }

    #[tokio::test]
    async fn a_server_that_quits_before_answering_is_said_in_words() {
        let home = tempfile::tempdir().unwrap();
        fake_server(home.path(), "exit 3");
        let err = sign_in_with(
            home.path(),
            &test_path(home.path()),
            Duration::from_secs(20),
        )
        .await
        .unwrap_err();
        assert!(err.contains("stopped before it answered"), "{err}");
    }

    #[test]
    fn the_login_tools_answers_become_plain_words() {
        let a = |text: &str, is_error| ToolAnswer {
            is_error,
            text: text.into(),
        };
        assert_eq!(
            sign_in_words(&a("Successfully authenticated with Plaud!", false)),
            Ok(())
        );
        assert_eq!(sign_in_words(&a("Already logged in.", false)), Ok(()));
        let timed_out = sign_in_words(&a("Authentication timed out after 2 minutes. If no browser opened, open this URL on the same machine and retry:\nhttps://web.plaud.ai/x", true)).unwrap_err();
        assert!(timed_out.contains("within 2 minutes"), "{timed_out}");
        assert!(!timed_out.contains("http"), "{timed_out}");
        let denied = sign_in_words(&a("Authentication denied: access_denied", true)).unwrap_err();
        assert!(denied.contains("declined"), "{denied}");
        let port = sign_in_words(&a(
            "Failed to start callback server: listen EADDRINUSE",
            true,
        ))
        .unwrap_err();
        assert!(port.contains("8199"), "{port}");
        assert!(!port.contains("EADDRINUSE"), "{port}");
        let other = sign_in_words(&a("Authentication failed: boom", true)).unwrap_err();
        assert!(other.contains("couldn't finish signing in"), "{other}");
        assert!(!other.contains("boom"), "{other}");
        assert_eq!(in_words(SIGN_IN_WAIT), "5 minutes");
    }

    #[tokio::test]
    async fn already_signed_in_is_said_plainly() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        write_tokens(h, r#"{"access_token":"a"}"#);
        fake_server(h, &reply("Already logged in.", false));
        let status = sign_in_with(h, &test_path(h), Duration::from_secs(20))
            .await
            .unwrap();
        assert!(status.signed_in && status.already, "{status:?}");
    }

    /// One Plaud browser sign-in at a time, shared with Recordings' `plaud
    /// login` (same port 8199), and a sign-out waits for it (S8 review).
    #[test]
    fn one_plaud_sign_in_at_a_time() {
        static FLAG: AtomicBool = AtomicBool::new(false);
        let first = SignInSlot::take_in(&FLAG).unwrap();
        let busy = SignInSlot::take_in(&FLAG).unwrap_err();
        assert!(busy.contains("already open"), "{busy}");
        assert!(busy.contains("Recordings"), "{busy}");
        drop(first);
        assert!(SignInSlot::take_in(&FLAG).is_ok(), "freed when it ends");
    }

    #[test]
    fn port_busy_words_only_for_a_busy_port() {
        let a = |text: &str| ToolAnswer {
            is_error: true,
            text: text.into(),
        };
        let other =
            sign_in_words(&a("Failed to start callback server: permission denied")).unwrap_err();
        assert!(!other.contains("8199"), "{other}");
        let busy = sign_in_words(&a(
            "Failed to start callback server: listen EADDRINUSE: address already in use :::8199",
        ))
        .unwrap_err();
        assert!(busy.contains("8199"), "{busy}");
    }
}
