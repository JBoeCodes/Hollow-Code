//! Native resume for an imported Claude Code chat (H8, N26 phase 8).
//!
//! **Continue in Hollow** hands an imported transcript to a fresh session
//! ([`super::continue_imported`]). When the imported chat came from Claude
//! Code and its original jsonl is still where it was, Continue can instead
//! fork that very session: a new Hollow chat whose first turn runs
//! `claude --resume <id> --fork-session` in the source folder, so the model
//! keeps the session's own context, not a summary.
//!
//! **$0 flag probe (2026-11-19, Claude Code 2.1.326).** `claude --version`
//! and `claude --help` only print text, no model call. The help lists
//! `-r, --resume [value]` ("Resume a conversation by session ID"),
//! `--fork-session` ("When resuming, create a new session ID instead of
//! reusing the original") and `--session-id <uuid>`. Native resume needs the
//! first two; [`Capability`] is read from that help text ([`parse_help`]) and
//! cached per binary ([`CapabilityCache`]); the probe itself is injected, so
//! tests never start the CLI.
//!
//! **Offered only when every check holds** ([`check_offer`]): the import is
//! from Claude Code, the CLI supports both flags, the session id is a safe
//! resume id, the source jsonl still exists at `<root>/<cwd slug>/<id>.jsonl`
//! (or elsewhere one folder below the recorded root) as a regular file, its
//! own lines name the same `sessionId` and `cwd`, and that `cwd` is still a
//! folder. Anything else is a [`Refusal`] and the caller falls back to the
//! handoff, unchanged.
//!
//! **Never written.** Hollow reads at most the first 64 KB of the source to
//! compare the id and cwd. The fork is made by the CLI itself, which writes
//! the new session file; Hollow writes nothing under the person's Claude
//! folder. The imported record stays read-only.
//!
//! **Launch.** [`continue_native`] makes the chat in the source `cwd`
//! (`parent_key`, `fork_kind = 'import'`, so a refused or failed resume falls
//! back to the handoff bundle on the fresh session) and records the source id
//! in `import_native`. The composer sends that id as the resume id of the
//! first turn; [`fork_pending`] is true for exactly that id, and
//! `agent::build_claude` then adds `--fork-session`. The CLI announces the
//! fork's own new id, which replaces the stint's, so later turns resume the
//! fork plainly.

use super::{Continued, Source, FORK_KIND};
use crate::session_store::{db_time, with_read, with_write};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The most bytes of the source file read to check its id and cwd.
const HEAD_BYTES: u64 = 64 * 1024;
/// The most lines looked at in that head.
const HEAD_LINES: usize = 200;

// ── capability ────────────────────────────────────────────────────────

/// What the installed Claude Code CLI can do for a native resume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Capability {
    /// `claude --version`, first line, when it printed one.
    pub version: Option<String>,
    /// `--resume <id>` is listed.
    pub resume: bool,
    /// `--fork-session` is listed.
    pub fork_session: bool,
}

impl Capability {
    /// Both flags a native resume needs.
    pub fn supports_fork_resume(&self) -> bool {
        self.resume && self.fork_session
    }
}

/// The text `claude --version` and `claude --help` printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Probed {
    pub version: String,
    pub help: String,
}

/// Whether `help` lists `flag` as an option: at the start of a help line, or
/// after a short flag and a comma (`-r, --resume [value]`), followed by a
/// space, bracket or the end of the line. A flag only named in prose does
/// not count.
fn lists_flag(help: &str, flag: &str) -> bool {
    help.lines().any(|line| {
        let line = line.trim_start();
        let line = match line.strip_prefix('-') {
            // `-r, --resume`: skip the short form.
            Some(rest) if !rest.starts_with('-') => match rest.split_once(", ") {
                Some((_, after)) => after,
                None => return false,
            },
            _ => line,
        };
        line.strip_prefix(flag)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '[', '<', '=', ',']))
    })
}

/// The capability a probe's text shows. Pure.
pub(crate) fn parse_help(probed: &Probed) -> Capability {
    Capability {
        version: probed
            .version
            .lines()
            .next()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string),
        resume: lists_flag(&probed.help, "--resume"),
        fork_session: lists_flag(&probed.help, "--fork-session"),
    }
}

/// The capability, probed once per key (the binary's path and modified time)
/// and kept. A failed probe is not kept, so the next call tries again.
#[derive(Default)]
pub(crate) struct CapabilityCache {
    slot: Mutex<Option<(String, Capability)>>,
}

impl CapabilityCache {
    pub const fn new() -> Self {
        CapabilityCache {
            slot: Mutex::new(None),
        }
    }

    /// The cached capability for `key`, else `probe()`'s, kept under `key`.
    pub fn get_or_probe(
        &self,
        key: &str,
        probe: impl FnOnce() -> Result<Probed, String>,
    ) -> Result<Capability, String> {
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((cached, capability)) = slot.as_ref() {
            if cached == key {
                return Ok(capability.clone());
            }
        }
        let capability = parse_help(&probe()?);
        *slot = Some((key.to_string(), capability.clone()));
        Ok(capability)
    }
}

static CACHE: CapabilityCache = CapabilityCache::new();

/// The installed CLI's capability, probed by [`real_probe`] once per binary
/// version (path and modified time).
pub(crate) fn installed_capability() -> Result<Capability, String> {
    let bin = crate::agent::find_binary(crate::agent::AgentKind::Claude)
        .ok_or("Claude Code isn't installed.")?;
    let modified = std::fs::metadata(&bin)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    CACHE.get_or_probe(&format!("{}@{modified}", bin.display()), || {
        real_probe(&bin)
    })
}

/// Runs `<bin> --version` and `<bin> --help` (text only, no model call, no
/// prompt) with a 10 second limit each.
fn real_probe(bin: &Path) -> Result<Probed, String> {
    Ok(Probed {
        version: run_text(bin, "--version")?,
        help: run_text(bin, "--help")?,
    })
}

fn run_text(bin: &Path, flag: &str) -> Result<String, String> {
    let mut child = std::process::Command::new(bin)
        .arg(flag)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Claude Code {flag}: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("Claude Code printed nothing.")?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = (&mut stdout).take(1 << 20).read_to_string(&mut text);
        text
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("Claude Code {flag} didn't answer."));
            }
        }
    }
    reader
        .join()
        .map_err(|_| "Claude Code's output couldn't be read.".to_string())
}

// ── arguments ─────────────────────────────────────────────────────────

/// The flags a native resume adds to Claude's command, in order, or `None`
/// for an id that isn't safe to put after a flag. Pure.
pub(crate) fn fork_args(session_id: &str) -> Option<Vec<String>> {
    crate::agent::is_safe_resume_id(session_id).then(|| {
        vec![
            "--resume".to_string(),
            session_id.to_string(),
            "--fork-session".to_string(),
        ]
    })
}

// ── source file ───────────────────────────────────────────────────────

/// Why a native resume isn't offered. Each reads as a sentence the person
/// can be shown beside the handoff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    NotClaudeCode,
    CliMissing(String),
    CliTooOld,
    UnsafeId,
    SourceGone,
    SourceMismatch,
    FolderGone,
}

impl Refusal {
    pub fn reason(&self) -> String {
        match self {
            Refusal::NotClaudeCode => "Only Claude Code chats can be resumed natively.".into(),
            Refusal::CliMissing(why) => format!("Claude Code can't be checked: {why}"),
            Refusal::CliTooOld => {
                "The installed Claude Code can't fork a session (no --fork-session).".into()
            }
            Refusal::UnsafeId => "The original session's id isn't one Claude Code takes.".into(),
            Refusal::SourceGone => {
                "The original session file isn't where it was imported from.".into()
            }
            Refusal::SourceMismatch => {
                "The file there isn't the same session (its id or folder differ).".into()
            }
            Refusal::FolderGone => "The original session's folder no longer exists.".into(),
        }
    }
}

/// Claude Code's folder name for a project: every character that isn't an
/// ASCII letter or digit becomes `-`.
pub(crate) fn project_slug(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// A regular file (never a link) at `path`.
fn is_plain_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_file())
        .unwrap_or(false)
}

/// Where session `id` (from folder `cwd`) lives below `root`: the slug
/// folder first, else any one folder below `root` holding `<id>.jsonl`
/// (names only, nothing is read). `None` when no regular file is there.
pub(crate) fn locate(root: &Path, cwd: &str, id: &str) -> Option<PathBuf> {
    let name = format!("{id}.jsonl");
    let direct = root.join(project_slug(cwd)).join(&name);
    if is_plain_file(&direct) {
        return Some(direct);
    }
    let mut folders: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .collect();
    folders.sort();
    folders
        .into_iter()
        .map(|dir| dir.join(&name))
        .find(|candidate| is_plain_file(candidate))
}

/// Whether the head of `path` has a line naming `id` as its `sessionId` and
/// `cwd` as its `cwd`. Reads at most [`HEAD_BYTES`] and [`HEAD_LINES`].
pub(crate) fn head_matches(path: &Path, id: &str, cwd: &str) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let reader = BufReader::new(file.take(HEAD_BYTES));
    for line in reader.lines().take(HEAD_LINES) {
        let Ok(line) = line else { break };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let (Some(sid), Some(line_cwd)) = (
            value.get("sessionId").and_then(|v| v.as_str()),
            value.get("cwd").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        return sid == id && line_cwd == cwd;
    }
    false
}

/// What the import recorded about a conversation, as far as a native resume
/// needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Recorded {
    pub source: String,
    pub root: String,
    pub external_id: String,
    pub source_cwd: Option<String>,
}

/// The session id inside `external_id` (`claude-code:<id>`, with a
/// `#v<n>` suffix on a later version), for a Claude Code import.
fn session_id_of(external_id: &str) -> Option<&str> {
    let rest = external_id.strip_prefix(Source::ClaudeCode.as_str())?;
    let id = rest.strip_prefix(':')?;
    Some(id.split("#v").next().unwrap_or(id))
}

/// Every check, in order. Pure over what it is given, except for reading
/// the source file's name and head. On success: the source's session id and
/// folder.
pub(crate) fn check_offer(
    recorded: &Recorded,
    capability: &Result<Capability, String>,
) -> Result<(String, String), Refusal> {
    if Source::parse(&recorded.source) != Some(Source::ClaudeCode) {
        return Err(Refusal::NotClaudeCode);
    }
    match capability {
        Err(why) => return Err(Refusal::CliMissing(why.clone())),
        Ok(c) if !c.supports_fork_resume() => return Err(Refusal::CliTooOld),
        Ok(_) => {}
    }
    let id = session_id_of(&recorded.external_id).ok_or(Refusal::NotClaudeCode)?;
    if !crate::agent::is_safe_resume_id(id) {
        return Err(Refusal::UnsafeId);
    }
    let cwd = recorded
        .source_cwd
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .ok_or(Refusal::SourceMismatch)?;
    let path = locate(Path::new(&recorded.root), cwd, id).ok_or(Refusal::SourceGone)?;
    if !head_matches(&path, id, cwd) {
        return Err(Refusal::SourceMismatch);
    }
    if !std::fs::metadata(cwd).map(|m| m.is_dir()).unwrap_or(false) {
        return Err(Refusal::FolderGone);
    }
    Ok((id.to_string(), cwd.to_string()))
}

// ── the offer and the chat ────────────────────────────────────────────

/// What Continue in Hollow can do for one imported chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeOffer {
    /// A native fork-resume can be offered next to the handoff.
    pub available: bool,
    /// Why not, in a sentence, when it can't.
    pub reason: Option<String>,
    /// The installed CLI's version, when it was probed.
    pub cli_version: Option<String>,
    /// The folder the forked session runs in.
    pub cwd: Option<String>,
}

fn recorded(vault: &Path, key: &str) -> Result<Option<Recorded>, String> {
    if !crate::agent_sessions::is_safe_session_key(key) {
        return Err("That isn't a chat's key.".into());
    }
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT source, root, external_id, source_cwd FROM imports \
             WHERE session_key = ?1 ORDER BY id DESC LIMIT 1",
            params![key],
            |row| {
                Ok(Recorded {
                    source: row.get(0)?,
                    root: row.get(1)?,
                    external_id: row.get(2)?,
                    source_cwd: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())
    })
}

/// [`native_offer`] with the capability given (tests inject it).
pub(crate) fn native_offer_with(
    vault: &Path,
    key: &str,
    capability: &Result<Capability, String>,
) -> Result<NativeOffer, String> {
    let recorded = recorded(vault, key)?.ok_or("That conversation wasn't imported.")?;
    let cli_version = capability.as_ref().ok().and_then(|c| c.version.clone());
    Ok(match check_offer(&recorded, capability) {
        Ok((_, cwd)) => NativeOffer {
            available: true,
            reason: None,
            cli_version,
            cwd: Some(cwd),
        },
        Err(refusal) => NativeOffer {
            available: false,
            reason: Some(refusal.reason()),
            cli_version,
            cwd: None,
        },
    })
}

/// Whether imported chat `key` can be continued natively, with the reason
/// when not. Reads only the source file's name and head.
pub(crate) fn native_offer(vault: &Path, key: &str) -> Result<NativeOffer, String> {
    // Only a Claude Code import needs the CLI probed.
    let recorded = recorded(vault, key)?.ok_or("That conversation wasn't imported.")?;
    let capability = if Source::parse(&recorded.source) == Some(Source::ClaudeCode) {
        installed_capability()
    } else {
        Err("not needed".into())
    };
    native_offer_with(vault, key, &capability)
}

/// What the person picked for a native continue.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct NativeRequest {
    pub model: Option<String>,
    pub mode: Option<String>,
    pub effort: Option<String>,
}

/// A native continue's answer: the chat, and the id its first turn resumes.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContinuedNative {
    #[serde(flatten)]
    pub chat: Continued,
    /// The composer sends this as the first turn's resume id; the turn forks
    /// it (`--fork-session`) and the CLI's own new id replaces it.
    pub resume_session_id: String,
}

/// [`continue_native`] with the capability given.
pub(crate) fn continue_native_with(
    vault: &Path,
    key: &str,
    request: &NativeRequest,
    capability: &Result<Capability, String>,
    now: DateTime<Utc>,
) -> Result<ContinuedNative, String> {
    let recorded = recorded(vault, key)?.ok_or("That conversation wasn't imported.")?;
    let (session_id, cwd) = check_offer(&recorded, capability).map_err(|r| r.reason())?;
    let cli_version = capability.as_ref().ok().and_then(|c| c.version.clone());
    // The same validation and echo as the handoff path.
    let plain = super::ContinueRequest {
        agent: "claude".into(),
        model: request.model.clone(),
        workspace_id: None,
        mode: request.mode.clone(),
        effort: request.effort.clone(),
    };
    let mut chat = super::continue_imported(vault, key, &plain, now)?;
    // The chat runs where the session was made: Claude Code finds a session
    // only under its original folder.
    let workspace = crate::workspaces::workspace_for_cwd(vault, Path::new(&cwd))
        .ok()
        .flatten()
        .map(|w| w.id)
        .unwrap_or_else(|| crate::workspaces::QUICK_CHATS_ID.to_string());
    with_write(vault, "continue import natively", |tx| {
        tx.execute(
            "UPDATE sessions SET cwd = ?2 WHERE key = ?1 AND fork_kind = ?3",
            params![chat.key, cwd, FORK_KIND],
        )
        .map_err(|e| format!("native cwd: {e}"))?;
        tx.execute(
            "INSERT OR REPLACE INTO import_native(session_key, source_session_id, source_cwd, \
             cli_version, created_at) VALUES(?1, ?2, ?3, ?4, ?5)",
            params![chat.key, session_id, cwd, cli_version, db_time(now)],
        )
        .map_err(|e| format!("native record: {e}"))?;
        Ok(())
    })?;
    chat.workspace_id = workspace;
    chat.cwd = cwd;
    Ok(ContinuedNative {
        chat,
        resume_session_id: session_id,
    })
}

/// Continue imported chat `key` by forking its Claude Code session. Refuses
/// (with the reason) when [`native_offer`] wouldn't offer it; the caller
/// then uses the handoff.
pub(crate) fn continue_native(
    vault: &Path,
    key: &str,
    request: &NativeRequest,
    now: DateTime<Utc>,
) -> Result<ContinuedNative, String> {
    let capability = installed_capability();
    continue_native_with(vault, key, request, &capability, now)
}

/// Whether a turn of chat `key` that resumes `resume_id` must fork it: the
/// chat was made by [`continue_native`] and `resume_id` is still the source
/// session's id (after the first turn the stint holds the fork's own id).
pub(crate) fn fork_pending(vault: &Path, key: &str, resume_id: &str) -> bool {
    if !crate::agent_sessions::is_safe_session_key(key) || resume_id.is_empty() {
        return false;
    }
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT 1 FROM import_native WHERE session_key = ?1 AND source_session_id = ?2",
            params![key, resume_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(|e| e.to_string())
    })
    .ok()
    .flatten()
    .is_some()
}
