//! Row U9, the publisher side: Settings, Updates, People on Alex's Mac
//! (updates design section 11). Add a person (a name and the channels they
//! may use), show their install link once, revoke them, change their
//! channels, list them.
//!
//! `people.json` lives in the R2 bucket, and the only R2 write credential is
//! the release builder's on the Mac mini, so every change runs
//! `scripts/release/people.mjs` there: one `ssh` to this Mac's remote runner
//! (`remote_session::configured_runner`, the same options as remote
//! sessions), which sources the builder's env file and runs the script from
//! the builder's clone. Every argument is a checked word (subcommand,
//! channel, id) or the shell-quoted name.
//!
//! **The token.** `update_people_add` makes 32 bytes with the OS random
//! source and encodes them as 43 base64url characters; the runner gets only
//! the SHA-256 hash, and only on ssh's stdin (never argv). The token goes
//! back to the window once, inside the install link, and is kept nowhere:
//! not in settings, state, an event, History or a log, and the answer's
//! `Debug` hides it. Lose it and the person is revoked and added again.
//!
//! Only on a publisher Mac (`UpdateSettings.publisher`); none of these is an
//! agent action (the person's own clicks).

use super::client::Client;
use super::settings::UpdateSettings;
use super::state::Channel;
use base64::Engine as _;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(crate) const NOT_PUBLISHER: &str = "People is only on the publisher's Mac: turn on \"This Mac publishes updates\" in Settings, Updates first.";
pub(crate) const NO_RUNNER: &str = "People changes run on the release builder, and no remote runner is set up on this Mac (Settings, Devices, Remote runner).";
/// The longest name (the script's own limit).
pub(crate) const MAX_NAME: usize = 80;
/// One runner call may take this long in all.
const RUN_TIMEOUT: Duration = Duration::from_secs(90);

/// One person, as the script lists them: never a token or a hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UpdatePerson {
    pub id: String,
    pub name: String,
    /// Allowed channels, Main first; empty means none.
    pub channels: Vec<Channel>,
    pub revoked: bool,
    /// RFC 3339, or empty for an entry written by hand without one.
    pub created_at: String,
}

/// `update_people_list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct UpdatePeopleList {
    pub people: Vec<UpdatePerson>,
}

/// `update_people_add`: the new person and their install link. The link
/// holds the token: show it once with a Copy button, never store it. Not
/// `Clone` (no second copy of the link), and the link's bytes are wiped
/// when the answer is dropped (after it is serialized to the window).
#[derive(PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UpdatePersonAdded {
    pub person: UpdatePerson,
    /// `https://<host>/install/<token>`.
    pub install_link: String,
}

impl std::fmt::Debug for UpdatePersonAdded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdatePersonAdded")
            .field("person", &self.person)
            .field("install_link", &"<redacted>")
            .finish()
    }
}

impl Drop for UpdatePersonAdded {
    fn drop(&mut self) {
        wipe(&mut self.install_link);
    }
}

/// Overwrite a secret string's bytes before its memory is freed (best
/// effort: an earlier reallocation may have left a copy).
pub(crate) fn wipe(secret: &mut String) {
    let mut bytes = std::mem::take(secret).into_bytes();
    bytes.fill(0);
    std::hint::black_box(&bytes);
}

/// Runs `people.mjs` on the release builder.
pub(crate) trait PeopleRunner: Send + Sync {
    /// The script's stdout, or the words for a person.
    fn run(&self, args: &[String], stdin: &[u8]) -> Result<String, String>;
}

/// A fresh token: 32 bytes from the OS random source, base64url without
/// padding (43 characters), the shape `token::valid_shape` and the Worker
/// accept.
pub(crate) fn new_token(
    fill: &mut dyn FnMut(&mut [u8]) -> Result<(), String>,
) -> Result<String, String> {
    let mut bytes = [0u8; 32];
    fill(&mut bytes)?;
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    bytes.fill(0);
    if !super::token::valid_shape(&token) {
        return Err("Hollow couldn't make an updates token.".into());
    }
    Ok(token)
}

/// The OS random source (`OsRng`), as an error rather than a panic.
pub(crate) fn os_fill(bytes: &mut [u8]) -> Result<(), String> {
    rand::RngCore::try_fill_bytes(&mut rand::rngs::OsRng, bytes)
        .map_err(|_| "Hollow couldn't read the system's random source.".to_string())
}

/// What the Worker stores: the lowercase hex SHA-256 of the token's text.
pub(crate) fn token_hash(token: &str) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `<host>/install/<token>`.
pub(crate) fn install_link(host: &Url, token: &str) -> String {
    let mut url = host.clone();
    url.set_path(&format!("/install/{token}"));
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

/// A name as the script takes it: trimmed, 1 to [`MAX_NAME`] characters, no
/// control, format (`\p{Cf}`, the BOM among them) or line-separator
/// characters anywhere, checked before trimming (JS's `trim` strips U+FEFF
/// and Rust's doesn't, so a name holding one is refused on both sides).
pub(crate) fn check_name(name: &str) -> Result<String, String> {
    if name.chars().any(super::people::hidden_char) {
        return Err(
            "A name can't hold line breaks, control characters or invisible formatting characters."
                .into(),
        );
    }
    let name = name.trim();
    if name.is_empty() {
        return Err("A person needs a name.".into());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!("A name is at most {MAX_NAME} characters."));
    }
    // A hash-shaped name would be refused by the script (a hash belongs on
    // stdin only).
    if name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("That name looks like a token hash; choose another.".into());
    }
    Ok(name.to_string())
}

/// A person id as the script made it (`p-<12 hex>`) or as written by hand.
pub(crate) fn check_id(id: &str) -> Result<&str, String> {
    let ok = (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && !(id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()));
    if ok {
        Ok(id)
    } else {
        Err("That isn't a person id.".into())
    }
}

/// Channels as the script takes them: no repeats, Main first.
pub(crate) fn channel_args(channels: &[Channel]) -> Vec<String> {
    super::people::ALL
        .into_iter()
        .filter(|c| channels.contains(c))
        .map(|c| c.as_str().to_string())
        .collect()
}

/// What a change asked for, which the runner's answer must match.
pub(crate) enum Expect<'a> {
    /// `add`: a new id (`p-<12 hex>`), the same name and channels, not revoked.
    Added {
        name: &'a str,
        channels: &'a [String],
    },
    /// `revoke`: the same id, revoked.
    Revoked { id: &'a str },
    /// `set-channels`: the same id with exactly these channels.
    Channels { id: &'a str, channels: &'a [String] },
}

pub(crate) const MISMATCH: &str = "The release builder's answer doesn't match the change asked for, so Hollow doesn't trust it. List people to see what people.json holds.";

/// An `add` whose answer didn't match: the builder may still have written the
/// person (the token's hash went to it before the answer came back), but its
/// answer can't be trusted and the link is never built from an answer that
/// failed the check, so the link is gone. Say what to do, not just that it
/// failed (U9 follow-up).
pub(crate) const MISMATCH_ADD: &str = "The release builder's answer doesn't match the person asked for, so Hollow doesn't trust it and made no install link. The person may still have been added: open People, and if they are listed, revoke that row and add them again for a new link.";

/// `p-` and 12 lowercase hex: the ids `people.mjs add` makes.
pub(crate) fn new_id_shape(id: &str) -> bool {
    id.len() == 14
        && id.starts_with("p-")
        && id[2..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The runner's answer, read as a person and checked against what was
/// asked: a runner (or anything between it and this Mac) that answers
/// success for a change it didn't make is refused rather than shown as done.
pub(crate) fn parse_person(out: &str, expect: Expect<'_>) -> Result<UpdatePerson, String> {
    let person: UpdatePerson = serde_json::from_str(out.trim())
        .map_err(|_| "The release builder's answer didn't read as a person.".to_string())?;
    let same_channels = |want: &[String]| {
        person
            .channels
            .iter()
            .map(|c| c.as_str())
            .eq(want.iter().map(String::as_str))
    };
    let ok = match expect {
        Expect::Added { name, channels } => {
            new_id_shape(&person.id)
                && !person.revoked
                && person.name == name
                && same_channels(channels)
        }
        Expect::Revoked { id } => person.id == id && person.revoked,
        Expect::Channels { id, channels } => person.id == id && same_channels(channels),
    };
    if ok {
        Ok(person)
    } else {
        Err(MISMATCH.into())
    }
}

fn require_publisher(settings: &UpdateSettings) -> Result<(), String> {
    if settings.publisher {
        Ok(())
    } else {
        Err(NOT_PUBLISHER.into())
    }
}

/// One People change at a time from this Mac (the script reads, changes and
/// writes the whole file; two at once could lose one).
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

pub(crate) fn list(
    settings: &UpdateSettings,
    runner: &dyn PeopleRunner,
) -> Result<UpdatePeopleList, String> {
    require_publisher(settings)?;
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let out = runner.run(&["list".to_string()], &[])?;
    serde_json::from_str(out.trim())
        .map_err(|_| "The release builder's answer didn't read as a list of people.".to_string())
}

/// Add a person: the token is made here, its hash goes to the runner on
/// stdin, the link comes back once.
pub(crate) fn add(
    settings: &UpdateSettings,
    host: &Url,
    runner: &dyn PeopleRunner,
    fill: &mut dyn FnMut(&mut [u8]) -> Result<(), String>,
    name: &str,
    channels: &[Channel],
) -> Result<UpdatePersonAdded, String> {
    require_publisher(settings)?;
    let name = check_name(name)?;
    let channels = channel_args(channels);
    if channels.is_empty() {
        return Err("Choose at least one channel for a new person.".into());
    }
    let mut token = new_token(fill)?;
    let hash = token_hash(&token);
    let mut args = vec!["add".to_string(), name.clone()];
    args.extend(channels.iter().cloned());
    let answer = {
        let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        runner
            .run(&args, format!("{hash}\n").as_bytes())
            .and_then(|out| {
                parse_person(
                    &out,
                    Expect::Added {
                        name: &name,
                        channels: &channels,
                    },
                )
            })
    };
    let added = match answer {
        Ok(person) => Ok(UpdatePersonAdded {
            install_link: install_link(host, &token),
            person,
        }),
        Err(m) if m == MISMATCH => Err(MISMATCH_ADD.to_string()),
        Err(m) => Err(m),
    };
    wipe(&mut token);
    added
}

pub(crate) fn revoke(
    settings: &UpdateSettings,
    runner: &dyn PeopleRunner,
    id: &str,
) -> Result<UpdatePerson, String> {
    require_publisher(settings)?;
    let id = check_id(id)?;
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    parse_person(
        &runner.run(&["revoke".into(), id.into()], &[])?,
        Expect::Revoked { id },
    )
}

pub(crate) fn set_channels(
    settings: &UpdateSettings,
    runner: &dyn PeopleRunner,
    id: &str,
    channels: &[Channel],
) -> Result<UpdatePerson, String> {
    require_publisher(settings)?;
    let id = check_id(id)?;
    let channels = channel_args(channels);
    let mut args = vec!["set-channels".to_string(), id.to_string()];
    args.extend(channels.iter().cloned());
    let _held = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    parse_person(
        &runner.run(&args, &[])?,
        Expect::Channels {
            id,
            channels: &channels,
        },
    )
}

// ── The runner over ssh ─────────────────────────────────────────────────

/// What runs on the release builder: the builder's env file (R2 settings,
/// `HOLLOW_NIGHTLY_REPO`), then `people.mjs` from its clone, with the
/// arguments as `"$@"`.
pub(crate) const REMOTE_SCRIPT: &str = r#"PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"; f="$HOME/.config/hollow-nightly/env"; if [ ! -r "$f" ]; then echo "people: the release builder's env file (~/.config/hollow-nightly/env) isn't on this runner" >&2; exit 1; fi; set -a; . "$f"; set +a; exec node "${HOLLOW_NIGHTLY_REPO:-$HOME/hollow-nightly/repo}/scripts/release/people.mjs" "$@""#;

/// `'…'` for a POSIX shell.
pub(crate) fn shell_quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// The one command line ssh sends: `/bin/sh -c '<script>' people '<arg>'…`.
pub(crate) fn remote_command(args: &[String]) -> String {
    let mut line = format!("exec /bin/sh -c {} people", shell_quote(REMOTE_SCRIPT));
    for arg in args {
        line.push(' ');
        line.push_str(&shell_quote(arg));
    }
    line
}

/// This Mac's runner over ssh (`remote_session::SSH_OPTS`: no prompts, no
/// agent or X11 forwarding, no shared connection), plus
/// `StrictHostKeyChecking=yes`: People writes the access list, so the runner
/// must already be a known host (an unknown or changed key is refused, never
/// trusted on first use).
pub(crate) struct SshRunner {
    pub(crate) ssh: String,
    pub(crate) host: Option<String>,
}

impl SshRunner {
    pub(crate) fn configured() -> Self {
        Self {
            ssh: "/usr/bin/ssh".into(),
            host: crate::remote_session::configured_runner(),
        }
    }
}

impl PeopleRunner for SshRunner {
    fn run(&self, args: &[String], stdin: &[u8]) -> Result<String, String> {
        use std::io::{Read, Write};
        use std::process::Stdio;
        let host = self.host.as_deref().ok_or(NO_RUNNER)?;
        let mut child = std::process::Command::new(&self.ssh)
            .args(ssh_args(host, args))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("ssh couldn't start ({e})"))?;
        if let Some(mut pipe) = child.stdin.take() {
            let _ = pipe.write_all(stdin);
        }
        let mut out_pipe = child.stdout.take();
        let mut err_pipe = child.stderr.take();
        let out = std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(p) = out_pipe.as_mut() {
                let _ = p.take(1024 * 1024).read_to_string(&mut s);
            }
            s
        });
        let err = std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(p) = err_pipe.as_mut() {
                let _ = p.take(64 * 1024).read_to_string(&mut s);
            }
            s
        });
        let began = std::time::Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if began.elapsed() < RUN_TIMEOUT => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
        };
        let out = out.join().unwrap_or_default();
        let err = err.join().unwrap_or_default();
        runner_result(host, status.and_then(|s| s.code()), &out, &err)
    }
}

/// ssh's arguments for one People call.
pub(crate) fn ssh_args(host: &str, args: &[String]) -> Vec<String> {
    let mut out: Vec<String> = crate::remote_session::SSH_OPTS
        .iter()
        .map(|s| s.to_string())
        .collect();
    out.extend(["-o".into(), "StrictHostKeyChecking=yes".into()]);
    out.extend(["--".into(), host.into(), remote_command(args)]);
    out
}

/// What an ssh call's exit means. `None`: it ran past the time limit or
/// was killed.
pub(crate) fn runner_result(
    host: &str,
    code: Option<i32>,
    out: &str,
    err: &str,
) -> Result<String, String> {
    match code {
        Some(0) => Ok(out.to_string()),
        None => Err(format!(
            "The release builder ({host}) didn't finish within {} seconds; nothing is known to have changed. List people to see.",
            RUN_TIMEOUT.as_secs()
        )),
        Some(255) => Err(crate::remote_session::unreachable_message(host, err)),
        Some(_) => {
            // Only the script's own `people:` line is repeated: it carries no
            // hash, token or credential.
            let words = err
                .lines()
                .rev()
                .find_map(|l| l.strip_prefix("people: "))
                .map(|w| w.trim().to_string());
            Err(match words {
                Some(w) => format!("The release builder said: {w}."),
                None => format!(
                    "The release builder ({host}) couldn't run People (scripts/release/people.mjs)."
                ),
            })
        }
    }
}

// ── Commands ────────────────────────────────────────────────────────────

/// The client, or why not.
fn the_client() -> Result<Arc<Client>, String> {
    super::client().ok_or_else(|| "Hollow's updater isn't running in this window.".to_string())
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|_| "People stopped before it answered.".to_string())?
}

/// The host the install link names: the same as the client checks.
fn link_host() -> Result<Url, String> {
    super::config::host_from(super::config::host_override(), !cfg!(debug_assertions))
}

#[tauri::command]
pub async fn update_people_list() -> Result<UpdatePeopleList, String> {
    let client = the_client()?;
    blocking(move || list(&client.settings(), &SshRunner::configured())).await
}

/// Add a person; the answer's `installLink` holds their token, shown once.
#[tauri::command]
pub async fn update_people_add(
    name: String,
    channels: Vec<Channel>,
) -> Result<UpdatePersonAdded, String> {
    let client = the_client()?;
    blocking(move || {
        add(
            &client.settings(),
            &link_host()?,
            &SshRunner::configured(),
            &mut os_fill,
            &name,
            &channels,
        )
    })
    .await
}

#[tauri::command]
pub async fn update_people_revoke(id: String) -> Result<UpdatePerson, String> {
    let client = the_client()?;
    blocking(move || revoke(&client.settings(), &SshRunner::configured(), &id)).await
}

/// Replace a person's allowed channels (none is allowed: they keep their
/// token but get no channel).
#[tauri::command]
pub async fn update_people_set_channels(
    id: String,
    channels: Vec<Channel>,
) -> Result<UpdatePerson, String> {
    let client = the_client()?;
    blocking(move || set_channels(&client.settings(), &SshRunner::configured(), &id, &channels))
        .await
}
