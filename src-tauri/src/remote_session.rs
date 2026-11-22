//! Remote sessions (2026-11-01, the Mac mini runner, Phase 2): a chat turn's
//! agent process runs on this Mac's remote runner (the SSH host
//! `hollow-slot --remote` uses, [`configured_runner`]) while everything else
//! stays here: the conversation, the run queue, History, usage, question
//! cards, Stop, and every tool Hollow serves. Design and security model:
//! docs/REMOTE_SESSIONS.md.
//!
//! The shape of one remote turn:
//!
//! 1. [`prepare`] (before the command is built): the worktree's exact state
//!    goes to the host by `git push` into a bare repository in the runner
//!    account's `~/hollow-remote`, exactly as `hollow-slot --remote` sends
//!    it, and the host checks it out in a folder of its own per local
//!    worktree (`sessions/<name>-<hash>`), on the same branch, with the
//!    uncommitted changes as uncommitted changes. That gives the
//!    [`RemoteTarget`] the command is built for.
//! 2. The agent command is built as usual (account routing, the run tag, the
//!    brokers' bound run ids), for the host's paths, and [`into_leader`]
//!    turns it into the run's local leader: `/usr/bin/perl` running
//!    [`LEADER_PL`], which listens on a private relay socket, starts `ssh`
//!    with that socket forwarded to the host (`-R`), and writes the agent's
//!    command line and environment (the account token included) to ssh's
//!    stdin as one JSON line, then this process's own stdin (a steerable
//!    turn's messages). Nothing secret is on either machine's command line
//!    or in a file: the header reaches the leader in its environment and the
//!    host over the SSH channel. The header's environment also carries the
//!    git identity this Mac uses for the checkout ([`checkout_identity`],
//!    as `GIT_AUTHOR_*` and `GIT_COMMITTER_*`), so the agent's commits there
//!    are this Mac's, never a guess from the runner account.
//! 3. On the host, [`REMOTE_RUN_PL`] (`--run`) reads that line and starts a
//!    detached supervisor (Phase 4), which runs the agent in a process group
//!    of its own, its output to files in `run/<run>/`; `--run` then streams
//!    them back over ssh into the usual parser and passes the leader's framed
//!    input on. A dropped connection ends only that stream: the leader
//!    reconnects (`--attach`) and the output goes on from the byte it had.
//!    The turn's finish ([`collect`]) stops a run still going there (Stop):
//!    TERM to the agent's group, then KILL after 10 s.
//! 4. Every tool the agent starts on the host is `hollow-remote-run
//!    --bridge <name>`, which connects to the relay with the run's relay
//!    token. The leader starts that server here, from the run's private
//!    config file, as its own child: the brokers' peer check walks to the
//!    leader, the registered run leader, and the server's proof never leaves
//!    this Mac. So the vault, the brokers and this Mac's tools stay here.
//! 5. When the agent exits, the host commits what it left (a snapshot commit
//!    of uncommitted changes on top of the branch) to a result ref, and
//!    [`collect`] fetches it and applies it to the local worktree, unless the
//!    worktree changed here meanwhile: then the result is kept on a branch and
//!    nothing here is touched.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

/// Where `hollow-slot --remote` and remote sessions find their runner:
/// `HOLLOW_SLOT_REMOTE` when set (even empty), else the first line of
/// `~/.config/hollow/slot-remote` that isn't blank or a `#` comment. `off`,
/// `local`, `none` or nothing means none. The same rules as `SLOT_TOOL`'s
/// `configured_remote`.
pub(crate) fn configured_runner() -> Option<String> {
    match configured_runner_setting() {
        RunnerSetting::Host(host) => Some(host),
        RunnerSetting::Off | RunnerSetting::Invalid => None,
    }
}

/// What the runner setting names ([`runner_setting`]). The machine note
/// (`this_mac.rs`) reads it too, so the runner it names is always the one
/// placement uses (J4 review).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RunnerSetting {
    /// Blank, `off`, `local` or `none`.
    Off,
    /// A host [`valid_host`] accepts, trimmed.
    Host(String),
    /// Anything else (`host:22`, `-J`, `user:secret@host`): no runner either.
    Invalid,
}

/// This Mac's runner setting, from the sources [`configured_runner`] names.
pub(crate) fn configured_runner_setting() -> RunnerSetting {
    match std::env::var("HOLLOW_SLOT_REMOTE") {
        Ok(value) => runner_setting(Some(&value), None),
        Err(_) => {
            let text = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
                .and_then(|base| {
                    std::fs::read_to_string(base.join("hollow").join("slot-remote")).ok()
                });
            runner_setting(None, text.as_deref())
        }
    }
}

/// The runner setting from `HOLLOW_SLOT_REMOTE` (`environment`, which wins
/// when set, even empty) or the `slot-remote` file's text (`config`).
pub(crate) fn runner_setting(environment: Option<&str>, config: Option<&str>) -> RunnerSetting {
    let raw = match environment {
        Some(value) => value,
        None => config
            .unwrap_or_default()
            .lines()
            .map(|line| line.split('#').next().unwrap_or("").trim())
            .find(|line| !line.is_empty())
            .unwrap_or_default(),
    };
    match raw.trim() {
        "" | "off" | "local" | "none" => RunnerSetting::Off,
        host if valid_host(host) => RunnerSetting::Host(host.to_string()),
        _ => RunnerSetting::Invalid,
    }
}

/// What placing a turn reads besides the vault's own stores: Settings,
/// Agents (Runs on, Overnight and worker chats; `None` when the file can't
/// be read) and this Mac's runner ([`configured_runner`]). Each is read only
/// when the rule needs it. Tests give their own (self-hosted models Q7): the
/// settings file is shared by the test process, and the runner comes from
/// this Mac's environment.
#[derive(Clone, Copy)]
pub(crate) struct PlaceReads {
    pub settings: fn() -> Option<crate::agent_settings::AgentSettings>,
    pub runner: fn() -> Option<String>,
}

impl PlaceReads {
    /// The saved settings and the configured runner.
    pub(crate) const LIVE: PlaceReads = PlaceReads {
        settings: live_settings,
        runner: configured_runner,
    };
}

fn live_settings() -> Option<crate::agent_settings::AgentSettings> {
    crate::agent_settings::get_settings().ok()
}

/// Where a chat turn's agent runs, decided before anything is sent
/// ([`turn_place`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TurnPlace {
    /// The person's own choice ([`runs_on`]).
    pub chosen: Option<Result<String, String>>,
    /// The runner unattended work prefers ([`unattended_runner`]).
    pub preferred: Option<String>,
    /// Self-hosted models Q7: the runner a self-hosted turn would have gone
    /// to (empty when the runner is chosen but none is set up). The turn
    /// runs on this Mac instead, and its first line says why.
    pub self_hosted_here: Option<String>,
}

/// Where a chat turn of `agent` in `cwd` runs. `prefers` is whether
/// unattended work may prefer the runner (overnight N6: an unattended turn
/// or a worker batch's chat that starts its conversation's native session):
/// then [`unattended_runner`], else the person's own choice, [`runs_on`].
/// Self-hosted models Q7: a turn on a self-hosted model (`self_hosted`,
/// `self_hosted::chat_on_self_hosted`) runs on this Mac whatever Runs on
/// says, because Hollow serves the model through this Mac's loopback
/// forwarder, which the runner can't reach, and signs in to its endpoint
/// here. It is decided here, before the turn's
/// fingerprint, a stale turn's choice or any preparation reads where it
/// runs, so nothing is sent there; it never refuses for that reason.
pub(crate) fn turn_place(
    vault: &Path,
    cwd: &Path,
    agent: crate::agent::AgentKind,
    prefers: bool,
    self_hosted: bool,
    reads: PlaceReads,
) -> TurnPlace {
    let preferred = prefers
        .then(|| unattended_runner(vault, cwd, agent, reads))
        .flatten();
    // Self-hosted models Q9 review: a self-hosted turn never goes to the
    // runner, so a Codex one's choice is read without the Codex sign-in
    // check (which reads `~/.codex` and would word a ChatGPT sign-in as "no
    // runner is set up"); its note names the runner it was kept from.
    let chosen = match &preferred {
        Some(_) => None,
        None if self_hosted => runs_on_checked(vault, cwd, agent, reads, false),
        None => runs_on(vault, cwd, agent, reads),
    };
    if !self_hosted {
        return TurnPlace {
            chosen,
            preferred,
            self_hosted_here: None,
        };
    }
    let self_hosted_here = match (chosen, preferred) {
        (Some(Ok(host)), _) | (None, Some(host)) => Some(host),
        (Some(Err(_)), _) => Some(String::new()),
        (None, None) => None,
    };
    TurnPlace {
        chosen: None,
        preferred: None,
        self_hosted_here,
    }
}

/// Where a turn of `agent` in `cwd` runs: `None` on this Mac; `Some(Ok(host))`
/// on the runner; `Some(Err(why))` when the Mac mini is chosen but no runner
/// is set up. A build workspace's Claude or Codex turn moves on its own Runs
/// on, else Settings, Devices, Runs on (Phase 2); a vault
/// workspace's own chat only on its own Runs on (Phase 3). The Main agent,
/// Quick chats and chats outside a workspace stay here.
pub(crate) fn runs_on(
    vault: &Path,
    cwd: &Path,
    agent: crate::agent::AgentKind,
    reads: PlaceReads,
) -> Option<Result<String, String>> {
    runs_on_checked(vault, cwd, agent, reads, true)
}

/// Whether the person's own Runs on sends `agent`'s turns in `cwd` to the
/// runner, set up or not: [`runs_on`] without the Codex sign-in check,
/// which decides whether such a turn can go, not where it was chosen to run.
/// The phone API's `runsOn` (v1.1, 2026-11-05) names it.
pub(crate) fn chosen_runner(
    vault: &Path,
    cwd: &Path,
    agent: crate::agent::AgentKind,
    reads: PlaceReads,
) -> bool {
    runs_on_checked(vault, cwd, agent, reads, false).is_some()
}

/// [`runs_on`], with the Codex sign-in check only when `credential` (Q9
/// review: a self-hosted Codex turn, which stays here, skips it).
fn runs_on_checked(
    vault: &Path,
    cwd: &Path,
    agent: crate::agent::AgentKind,
    reads: PlaceReads,
    credential: bool,
) -> Option<Result<String, String>> {
    use crate::agent::AgentKind;
    if !matches!(agent, AgentKind::Claude | AgentKind::Codex) {
        return None;
    }
    // A build workspace's worktree (Phase 2), or a vault workspace's own
    // folder (Phase 3); the Main agent and chats outside a workspace stay.
    let own = match crate::build_workspaces::runs_on_for_cwd(vault, cwd) {
        Some(own) => own,
        // A vault workspace moves only on its own choice: its folder may hold
        // sources that should stay here, so the Settings default (for build
        // workspaces) never sends it.
        None => {
            let id = crate::remote_folder::workspace_of(vault, cwd)?;
            Some(crate::remote_folder::runs_on_for(vault, &id)?)
        }
    };
    let choice = own.or_else(|| {
        (reads.settings)().and_then(|s| crate::agent_settings::runs_on_choice(s.runs_on.as_deref()))
    });
    if !choice
        .as_deref()
        .is_some_and(crate::agent_settings::is_runner)
    {
        return None;
    }
    // Before anything is sent: a Codex turn with no credential to hand over.
    if agent == AgentKind::Codex && credential {
        if let Err(why) = codex_credential() {
            return Some(Err(why));
        }
    }
    Some((reads.runner)().ok_or_else(|| {
        "This workspace runs on the runner, but no runner is set up on this Mac (one SSH host on the first line of ~/.config/hollow/slot-remote). Set Runs on to This Mac, or name the runner there.".to_string()
    }))
}

/// The runner unattended work prefers (overnight N6, 2026-11-02): a turn
/// with nobody watching (the night queue, a turn an unattended run started)
/// or a Main agent worker batch's chat, in a build workspace with no Runs on
/// of its own, goes to the runner when Settings, Devices,
/// Overnight and worker chats is the runner (saved `runner` or `mac-mini`;
/// since M7 absent follows Runs on, like `same`) and a runner is set up. Unlike the person's own choice ([`runs_on`], B-9), it runs here
/// when the runner can't take it ([`preferred_blocker`], a failed
/// preparation), and the chat says why. A workspace's own choice, vault
/// workspaces (C-3), the Main agent and quick chats are never moved by it.
pub(crate) fn unattended_runner(
    vault: &Path,
    cwd: &Path,
    agent: crate::agent::AgentKind,
    reads: PlaceReads,
) -> Option<String> {
    use crate::agent::AgentKind;
    // Cheap checks first: only Claude and Codex, only a build workspace.
    if !matches!(agent, AgentKind::Claude | AgentKind::Codex) {
        return None;
    }
    let own = crate::build_workspaces::runs_on_for_cwd(vault, cwd)?;
    let setting = (reads.settings)()?.unattended_runs_on;
    unattended_runner_for(agent, Some(own), setting.as_deref(), (reads.runner)())
}

/// [`unattended_runner`]'s rule: `own` is `None` outside a build workspace,
/// `Some(None)` for one following the default, `Some(Some(choice))` for one
/// with its own; `setting` is the saved Overnight and worker chats value.
pub(crate) fn unattended_runner_for(
    agent: crate::agent::AgentKind,
    own: Option<Option<String>>,
    setting: Option<&str>,
    runner: Option<String>,
) -> Option<String> {
    use crate::agent::AgentKind;
    if !matches!(agent, AgentKind::Claude | AgentKind::Codex) {
        return None;
    }
    // A build workspace following the default; one with its own choice
    // keeps it (`runs_on`), and anything else isn't moved by this.
    if !matches!(own, Some(None)) {
        return None;
    }
    if crate::agent_settings::unattended_runs_on_choice(setting).as_deref()
        != Some(crate::agent_settings::RUNNER)
    {
        return None;
    }
    runner
}

/// Whether a turn starts its conversation's native session (overnight N6):
/// no id asked to resume, and the open stint (`session_store::start_turn`
/// opens one before the turn runs) holds no native session yet. Only such a
/// turn may be preferred onto the runner, so one that stays here never
/// leaves a session there to resume, or the reverse.
pub(crate) fn starts_native_session(
    requested: Option<&str>,
    state: &crate::session_store::ResumeState,
) -> bool {
    let asked = requested
        .map(str::trim)
        .is_some_and(|id| !id.is_empty() && id != "antigravity");
    !asked
        && state
            .open
            .as_ref()
            .is_none_or(|open| open.harness_session_id.is_none())
}

/// Why a turn [`unattended_runner`] sends can't go to `host` now, before
/// anything is sent, as a sentence for the start notice. `long_work` is the
/// turn's account routing (an unattended turn's), and `claude_login`
/// whether it attaches a connector that needs the Claude Code login. A
/// remote turn of this Mac's still in the same worktree, the last status
/// check's answer (at most 5 minutes old, a failed preparation included),
/// and the agent's credential each keep it here.
pub(crate) fn preferred_blocker(
    host: &str,
    agent: crate::agent::AgentKind,
    cwd: &Path,
    long_work: bool,
    claude_login: bool,
) -> Option<Blocked> {
    use crate::agent::AgentKind;
    let blocked = |cause: &'static str, message: &str| {
        Some(Blocked {
            cause,
            message: message.to_string(),
        })
    };
    if worktree_busy(cwd) {
        return blocked("busy", BUSY_SENTENCE);
    }
    if let Some((state, why)) =
        crate::remote_status::recently_unavailable_state(host, std::time::Duration::from_secs(300))
    {
        let cause = if state == "refusing" {
            "refusing"
        } else {
            "unreachable"
        };
        return blocked(cause, &why);
    }
    match agent {
        AgentKind::Codex if codex_credential().is_err() => blocked("signIn", CODEX_KEY_SENTENCE),
        AgentKind::Claude if claude_login => blocked("signIn", CLAUDE_CONNECTOR_SENTENCE),
        AgentKind::Claude if !crate::claude_accounts::next_run_has_token(long_work) => {
            blocked("signIn", CLAUDE_ACCOUNT_SENTENCE)
        }
        _ => None,
    }
}

/// Why [`preferred_blocker`] keeps a turn here: its route's cause (RT1) and
/// the sentence the `stayed` line shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Blocked {
    pub cause: &'static str,
    pub message: String,
}

/// [`preferred_blocker`]'s own sentences, shared with RT1's test.
pub(crate) const BUSY_SENTENCE: &str =
    "Another turn of this worktree is still on it, and two can't share its copy there.";
pub(crate) const CODEX_KEY_SENTENCE: &str =
    "Codex has no API key to hand over there; its ChatGPT sign-in stays on this Mac.";
pub(crate) const CLAUDE_CONNECTOR_SENTENCE: &str =
    "A claude.ai connector this turn uses needs the Claude Code login, which stays on this Mac.";
pub(crate) const CLAUDE_ACCOUNT_SENTENCE: &str = "Claude runs there only on one of your added accounts (Settings → Accounts), and this turn would use the Claude Code login.";

/// What a failed preparation says, as the reason a preferred turn ran here:
/// without the advice to set Runs on to This Mac (it already runs here),
/// ending in a full stop.
pub(crate) fn stay_reason(error: &str) -> String {
    let why = error.trim().trim_end_matches(SET_HERE_HINT).trim_end();
    if why.ends_with('.') {
        why.to_string()
    } else {
        format!("{why}.")
    }
}

/// RT1 (overnight 11-17): a routing decision, as a chat turn's record keeps
/// it (`route` on the `remoteRun` `start` and `stayed` notices, which the
/// chat line reads): where the agent ran (`runner` or `here`), the runner
/// it was about, why (a `cause` and plain words), and whether it was a
/// fallback, meant for the runner and run here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Route {
    pub target: &'static str,
    pub host: String,
    pub cause: &'static str,
    pub reason: &'static str,
    pub fallback: bool,
}

impl Route {
    /// A turn sent to `host`: `preferred` when the unattended default sent
    /// it (overnight N6), `reattached` when it picks up a run an earlier
    /// Hollow left there (overnight R1), else Runs on chose it.
    pub(crate) fn there(host: &str, preferred: bool, reattached: bool) -> Self {
        let (cause, reason) = if reattached {
            (
                "reattached",
                "it picks up the run an earlier Hollow left there",
            )
        } else if preferred {
            (
                "preferred",
                "overnight and worker chats go to the runner when it answers",
            )
        } else {
            ("chosen", "Runs on is set to the runner")
        };
        Route {
            target: "runner",
            host: host.to_string(),
            cause,
            reason,
            fallback: false,
        }
    }

    /// A turn meant for `host` that runs here: `cause` when the code that
    /// kept it here knew it ([`Blocked`]), else read from a failed
    /// preparation's error ([`prepare_cause`]).
    pub(crate) fn fell_back(host: &str, cause: Option<&'static str>, message: &str) -> Self {
        let cause = cause.unwrap_or_else(|| prepare_cause(message));
        let reason = match cause {
            "unreachable" => "the runner did not answer",
            "refusing" => "the runner wasn't taking work",
            "busy" => "another turn of this worktree was still on the runner",
            "signIn" => "this turn's sign-in stays on this Mac",
            _ => "the runner couldn't be made ready",
        };
        Route {
            target: "here",
            host: host.to_string(),
            cause,
            reason,
            fallback: true,
        }
    }
}

/// A preparation the runner's preflight refused (exit 4), with its line.
pub(crate) fn refusing_message(host: &str, line: &str) -> String {
    format!("{host} isn't taking work now ({line})")
}

/// A preparation whose checkout couldn't be sent, with git's error.
pub(crate) fn checkout_send_error(host: &str, error: &str) -> String {
    format!("The checkout couldn't be sent to {host} ({error})")
}

/// The cause (RT1) of a failed preparation, from its error: these are
/// strings built by [`unreachable_message`], the preflight's refusal,
/// [`crate::remote_status::stalled_message`] and the checkout's send, and
/// `each_fallback_sentence_maps_to_its_cause` builds each one from its
/// producer, so a reworded producer fails that test.
pub(crate) fn prepare_cause(message: &str) -> &'static str {
    let has = |needle: &str| message.contains(needle);
    if has("didn't answer") || has("didn't finish the check") {
        "unreachable"
    } else if has("taking work") {
        "refusing"
    } else if has("checkout couldn't be sent") && connection_failed(message) {
        "unreachable"
    } else {
        "prepare"
    }
}

/// Whether an error's words are ssh failing to reach or keep a connection.
fn connection_failed(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    [
        "timed out",
        "connection refused",
        "connection closed",
        "connection reset",
        "could not resolve hostname",
        "nodename nor servname",
        "no route to host",
        "network is unreachable",
        "kex_exchange_identification",
        "broken pipe",
    ]
    .iter()
    .any(|n| m.contains(n))
}

/// The `stayed` notice (overnight N6) of a turn the unattended default
/// meant for `host` that runs here because of `message` (`cause` when
/// known), with its route (RT1).
pub(crate) fn stayed_notice(
    host: &str,
    cause: Option<&'static str>,
    message: &str,
    session_key: &str,
) -> Value {
    json!({
        "type": "remoteRun",
        "phase": "stayed",
        "host": host,
        "reason": stay_reason(message),
        "route": Route::fell_back(host, cause, message),
        "sessionKey": session_key,
    })
}

/// The last sentence of [`unreachable_message`].
const SET_HERE_HINT: &str = "To run this turn here, set Runs on to This Mac.";

/// Whether a remote turn of this Mac's has `cwd`'s worktree there now
/// (from the end of its preparation until its finish). Chat turns in one
/// worktree already run one at a time (its lease); this keeps a preferred
/// turn here if one is still finishing.
fn worktree_busy(cwd: &Path) -> bool {
    TURNS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .any(|t| t.git_worktree && cwd.starts_with(&t.worktree))
}

/// An SSH host alias or `user@host`: letters, digits, `.`, `_`, `@`, `-`,
/// never starting with `-` (an option).
pub(crate) fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && !host.starts_with('-')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | '-'))
}

/// Everything a remote turn needs once its folder on the host is ready.
/// Kept on disk while the turn runs (overnight R1, [`begin_turn`]), less
/// what only its start notice reads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteTarget {
    /// The SSH host (an alias from `~/.ssh/config`).
    pub host: String,
    /// The ssh program (tests use a stand-in).
    pub ssh: String,
    /// The host account's home.
    pub home: String,
    /// The session folder on the host (a git worktree of the bare repo).
    pub dir: String,
    /// The agent's working folder there (`dir` plus the local subfolder).
    pub cwd: String,
    /// The turn's scratch folder there (`TMPDIR`, `--add-dir`).
    pub scratch: String,
    /// The folder's own port there.
    pub port: u16,
    /// The folder's own `CARGO_TARGET_DIR` there.
    pub target: String,
    /// Earlier turns' results that never came back (Stop before the runner
    /// had written them, a failed fetch): kept there at preparation and
    /// brought here as these branches, which the turn's start notice names.
    #[serde(skip)]
    pub kept: Vec<String>,
    /// The local worktree, its HEAD and branch when the turn started, and the
    /// snapshot commit that was sent (HEAD itself for a clean tree).
    pub worktree: PathBuf,
    pub head: String,
    pub snapshot: String,
    /// The snapshot's tree (its commit may exist only on the host).
    pub snapshot_tree: String,
    pub branch: Option<String>,
    /// The bare repository's id on the host and this worktree's.
    pub repo_id: String,
    pub wt_id: String,
    /// Phase 2's git worktree session; false when a vault workspace's own
    /// folder is the session (Phase 3: no worktree to bring back).
    pub git_worktree: bool,
    /// The vault workspace folder sent with this turn (Phase 3): the agent's
    /// working folder, or a build workspace's vault folder beside its
    /// worktree. Its changes come back by `collect_folder`.
    pub folder: Option<crate::remote_folder::FolderSync>,
    /// Earlier turns' folder changes that never came back, applied here at
    /// preparation (the start notice says so).
    #[serde(skip)]
    pub folder_earlier: Option<crate::remote_folder::FolderBack>,
    /// This turn's id (8 hex digits): every attempt's run id starts with it,
    /// so finishing the turn stops only its own run there (Phase 4).
    pub turn: String,
    /// Idle remote sessions' folders this preparation removed because the
    /// runner's disk was under its floor (overnight N5; the start notice
    /// says so).
    #[serde(skip)]
    pub pruned: Option<DiskPruned>,
    /// Copies of a run that died there which couldn't be kept for an hour,
    /// moved aside on the runner instead of reset (overnight P3L-5; the
    /// start notice names them).
    #[serde(skip)]
    pub aside: Vec<String>,
    /// The git identity this Mac uses for the checkout (or the workspace
    /// folder), which the agent's commits there carry; None when this Mac
    /// has none configured for it, so git's own default there stays (the
    /// start notice says so).
    pub identity: Option<GitIdentity>,
    /// Remote runners (2026-11-03): what the turn may reach beyond its
    /// checkout (GitHub, Google Cloud), from its workspace's choice when it
    /// was prepared. No secret: those are read when the leader is built.
    #[serde(default)]
    pub access: crate::runner_access::TurnAccess,
    /// Overnight R1: this turn picks up a run an earlier Hollow left going
    /// there ([`stale_turn`]): its leader attaches instead of starting one.
    #[serde(skip)]
    pub reattach: Option<Reattach>,
}

/// Who commits: the author and committer git would use (11-02).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitIdentity {
    pub author_name: String,
    pub author_email: String,
    pub committer_name: String,
    pub committer_email: String,
}

impl GitIdentity {
    /// The variables that make git use it, whatever the host's config says.
    pub(crate) fn env(&self) -> [(&'static str, &str); 4] {
        [
            ("GIT_AUTHOR_NAME", &self.author_name),
            ("GIT_AUTHOR_EMAIL", &self.author_email),
            ("GIT_COMMITTER_NAME", &self.committer_name),
            ("GIT_COMMITTER_EMAIL", &self.committer_email),
        ]
    }
}

/// The git identity this Mac uses in `dir` (a checkout, or a folder in
/// none): `git var` with the checkout's whole config, so includes and
/// `includeIf` (by `gitdir`, by a remote's URL) pick a work identity where
/// one is set, and this process's `GIT_AUTHOR_*` win as they would for a
/// commit here. `user.useConfigOnly` keeps git from guessing one from the
/// account and host name: None when nothing is configured.
pub(crate) fn checkout_identity(dir: &Path) -> Option<GitIdentity> {
    identity_with(dir, |_| {})
}

/// [`checkout_identity`], with `configure` applied to each git command
/// (tests give it a config and an environment of their own).
pub(crate) fn identity_with(
    dir: &Path,
    configure: impl Fn(&mut std::process::Command),
) -> Option<GitIdentity> {
    let ident = |var: &str| {
        let mut cmd = std::process::Command::new("git");
        cmd.current_dir(dir)
            .args(["-c", "user.useConfigOnly=true", "var", var])
            .env("LC_ALL", "C")
            .stdin(Stdio::null());
        configure(&mut cmd);
        let out = cmd.output().ok()?;
        if !out.status.success() {
            if let Some(line) = identity_failure_note(&String::from_utf8_lossy(&out.stderr)) {
                eprintln!("[remote] no git identity from {}: {line}", dir.display());
            }
            return None;
        }
        parse_ident(&String::from_utf8_lossy(&out.stdout))
    };
    let (author_name, author_email) = ident("GIT_AUTHOR_IDENT")?;
    let (committer_name, committer_email) = ident("GIT_COMMITTER_IDENT")?;
    Some(GitIdentity {
        author_name,
        author_email,
        committer_name,
        committer_email,
    })
}

/// What the app log gets when `git var` fails (`stderr`, under `LC_ALL=C`):
/// nothing for a missing or empty name or email, the expected "none";
/// otherwise (a broken include) git's last line, with whatever it put in
/// angle brackets (an email) left out.
pub(crate) fn identity_failure_note(stderr: &str) -> Option<String> {
    if stderr.contains("auto-detection is disabled") || stderr.contains("empty ident") {
        return None;
    }
    let line = stderr.trim().lines().last().unwrap_or("").trim();
    // '<' and '>' are single bytes, never inside a multibyte character.
    Some(match (line.find('<'), line.rfind('>')) {
        (Some(a), Some(b)) if a < b => format!("{}<email>{}", &line[..a], &line[b + 1..]),
        _ => line.to_string(),
    })
}

/// `Name <email> <time> <zone>` (what `git var` prints) as name and email;
/// None for an empty name or email or anything else.
pub(crate) fn parse_ident(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    let close = line.rfind('>')?;
    let open = line[..close].rfind('<')?;
    let name = line[..open].trim();
    let email = line[open + 1..close].trim();
    let tail = line[close + 1..].trim();
    let mut parts = tail.split(' ');
    let time_ok = parts
        .next()
        .is_some_and(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_digit()));
    let zone_ok = parts.next().is_some_and(|z| {
        z.len() == 5
            && (z.starts_with('+') || z.starts_with('-'))
            && z[1..].chars().all(|c| c.is_ascii_digit())
    });
    if name.is_empty() || email.is_empty() || !time_ok || !zone_ok || parts.next().is_some() {
        return None;
    }
    Some((name.to_string(), email.to_string()))
}

/// Where a preparation moved a copy it couldn't keep (overnight P3L-5):
/// only a path under the runner's own `aside/` folder.
fn aside_of(paths: &Value, home: &str) -> Option<String> {
    paths
        .get("aside")
        .and_then(Value::as_str)
        .filter(|a| {
            a.strip_prefix(&format!("{home}/hollow-remote/aside/"))
                .is_some_and(|name| {
                    !name.is_empty()
                        && name
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
                        && name != "."
                        && name != ".."
                })
        })
        .map(str::to_string)
}

/// What a preparation's free-space prune removed there (overnight N5).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiskPruned {
    /// Remote sessions whose folders went.
    pub count: usize,
    /// Free space on the runner's volume when it started, and its floor.
    pub free_gb: u64,
    pub floor_gb: u64,
}

/// A vault workspace folder to send with a turn (Phase 3).
#[derive(Debug, Clone)]
pub struct FolderRequest {
    pub vault: PathBuf,
    pub folder: PathBuf,
    /// The conversation: each has its own copy there.
    pub session_key: String,
}

impl RemoteTarget {
    /// The result refs the host writes: the branch head the agent left, and
    /// that plus its uncommitted changes.
    fn result_refs(&self) -> (String, String) {
        let hash = self.wt_id.rsplit('-').next().unwrap_or(&self.wt_id);
        (
            format!("refs/hollow-session/{hash}-head"),
            format!("refs/hollow-session/{hash}-result"),
        )
    }
}

/// `n` random hex digits (relay tokens and socket names).
fn random_hex(n: usize) -> String {
    let bytes: [u8; 32] = rand::random();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    hex[..n.min(hex.len())].to_string()
}

/// A private temp folder removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(prefix: &str) -> Result<Self, String> {
        use std::os::unix::fs::DirBuilderExt;
        let dir = std::env::temp_dir().join(format!("{prefix}{}", random_hex(12)));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .map_err(|e| format!("no temp folder here ({e})"))?;
        Ok(Self(dir))
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The first `n` hex digits of `text`'s SHA-256 (`hollow-slot`'s `hash_of`).
fn hash_of(text: &str, n: usize) -> String {
    let hex = format!("{:x}", Sha256::digest(text.as_bytes()));
    hex[..n.min(hex.len())].to_string()
}

/// A folder name's safe part (`hollow-slot`'s `slug_of`), a leading dot
/// made `_` as there (2026-11-03: the host's `*.used` globs skip dot names).
/// Byte by byte, as the shell's `LC_ALL=C tr` works (2026-11-03): per char,
/// `café` was `caf-` here and `caf--` there, so the two named different
/// folders for a worktree whose name isn't ASCII.
fn slug_of(path: &str) -> String {
    let base = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    base.bytes()
        .enumerate()
        .map(|(i, b)| {
            if i == 0 && b == b'.' {
                '_'
            } else if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-') {
                char::from(b)
            } else {
                '-'
            }
        })
        .take(40)
        .collect()
}

fn git(cwd: &Path, args: &[&str]) -> Result<String, String> {
    git_env(cwd, args, &[])
}

/// Every git this module runs on this Mac carries the hardened helper's
/// pinned flags ([`crate::build_workspaces::SUBMODULE_SAFE`], Q-R7
/// 2026-11-13): a checkout can hold a planted `160000` entry for a decoy
/// repository whose config runs `core.fsmonitor`, and plain `status` or
/// `add -A` would start it here, unsandboxed. The caller's env overrides
/// (`GIT_DIR`, `GIT_INDEX_FILE`, ...) are kept as they were.
fn git_env(cwd: &Path, args: &[&str], env: &[(&str, &OsStr)]) -> Result<String, String> {
    let mut cmd = std::process::Command::new("git");
    cmd.current_dir(cwd)
        .args(crate::build_workspaces::SUBMODULE_SAFE)
        .args(args)
        .stdin(Stdio::null());
    for (key, value) in env {
        cmd.env(key, value);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("git {} couldn't start ({e})", args.join(" ")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(format!(
            "git {} failed: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Run `ssh <host> <command>` with `stdin`, bounded by ssh's own connect
/// timeout. Ok(stdout) on exit 0; Err names what failed, with the reachability
/// hint when ssh itself failed (exit 255).
/// Every ssh this module starts: no prompts, a bounded connect, a dead
/// connection noticed within a minute, and no agent or X11 forwarding (the
/// person's ssh config must not hand the runner their keys), and no shared
/// master connection (a reconnect after a drop must be a new one).
pub(crate) const SSH_OPTS: [&str; 14] = [
    "-a",
    "-x",
    "-o",
    "BatchMode=yes",
    "-o",
    "ConnectTimeout=10",
    "-o",
    "ServerAliveInterval=15",
    "-o",
    "ServerAliveCountMax=4",
    "-o",
    "ForwardAgent=no",
    "-o",
    "ControlPath=none",
];

/// `GIT_SSH_COMMAND` with [`SSH_OPTS`].
fn git_ssh_command(ssh: &str) -> String {
    format!("{} {}", shell_quote(ssh), SSH_OPTS.join(" "))
}

fn ssh_call(ssh: &str, host: &str, command: &str, stdin: &[u8]) -> Result<String, String> {
    use std::io::Write;
    let began = std::time::Instant::now();
    // Exit 255 must mean ssh itself failed: a command there that exits 255
    // (perl's `die`) is reported as 3, "couldn't get ready".
    let command = format!("{command}; s=$?; [ \"$s\" -ne 255 ] || s=3; exit $s");
    let mut child = std::process::Command::new(ssh)
        .args(SSH_OPTS)
        .args([host, &command])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ssh couldn't start ({e})"))?;
    if let Some(mut pipe) = child.stdin.take() {
        let _ = pipe.write_all(stdin);
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("ssh failed ({e})"))?;
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    // Any other exit means ssh got through: an unreachable status goes (a
    // refusing one only when `host_prep` passes the preflight). An ssh
    // killed by a signal has no exit code and says nothing either way.
    if matches!(out.status.code(), Some(c) if c != 255 && c != 4) {
        crate::remote_status::note_answered(host, began);
    }
    match out.status.code() {
        Some(0) => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        // Both are what a status check would have found, so the runner's
        // status says so too (Settings, and the next unattended turn,
        // overnight N6, which then runs here at once).
        Some(255) => {
            crate::remote_status::note_failure(host, "unreachable", &err);
            Err(unreachable_message(host, &err))
        }
        Some(4) => {
            let line = err.lines().last().unwrap_or("its preflight said no");
            crate::remote_status::note_failure(host, "refusing", line);
            Err(refusing_message(host, line))
        }
        _ => Err(format!(
            "{host} couldn't get the session ready ({})",
            err.lines().last().unwrap_or("no message")
        )),
    }
}

/// What the person reads when the runner can't be reached: ssh's error and
/// what to check for it (`remote_status::likely_causes`, overnight N2).
pub(crate) fn unreachable_message(host: &str, ssh_error: &str) -> String {
    let last = ssh_error
        .lines()
        .last()
        .unwrap_or("")
        .trim()
        .trim_end_matches('.');
    format!(
        "The runner ({host}) didn't answer{}. Nothing ran there. {} {SET_HERE_HINT}",
        if last.is_empty() {
            String::new()
        } else {
            format!(": {last}")
        },
        crate::remote_status::likely_causes(last)
    )
}

/// Get the host ready for a turn from the git checkout at `cwd`: send its
/// exact state, check it out there and return where the agent runs.
/// Blocking (git and ssh); call it off the async runtime.
pub(crate) fn prepare(host: &str, ssh: &str, cwd: &Path) -> Result<RemoteTarget, String> {
    prepare_with(host, ssh, cwd, None)
}

/// [`prepare`], with a build workspace's vault folder sent beside the
/// worktree (Phase 3).
pub(crate) fn prepare_with(
    host: &str,
    ssh: &str,
    cwd: &Path,
    folder: Option<&FolderRequest>,
) -> Result<RemoteTarget, String> {
    if !valid_host(host) {
        return Err(format!("{host} is not an SSH host name"));
    }
    let top = PathBuf::from(git(cwd, &["rev-parse", "--show-toplevel"]).map_err(|_| {
        "This folder isn't in a git checkout, so it can't run on the runner.".to_string()
    })?);
    let prefix = git(cwd, &["rev-parse", "--show-prefix"]).unwrap_or_default();
    let common = git(
        cwd,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let head = git(&top, &["rev-parse", "--verify", "-q", "HEAD^{commit}"])
        .map_err(|_| "This checkout has no commit yet.".to_string())?;
    let branch = git(&top, &["symbolic-ref", "--short", "-q", "HEAD"])
        .ok()
        .filter(|b| !b.is_empty());
    let top_str = top.to_string_lossy().into_owned();
    let repo_id = format!(
        "{}-{}",
        slug_of(common.trim_end_matches("/.git")),
        hash_of(&common, 12)
    );
    let wt_id = format!("{}-{}", slug_of(&top_str), hash_of(&top_str, 12));
    let scratch = TempDir::new("hollow-session-")?;
    let attachments = attachments_in_checkout(&prefix);
    let (snapshot, clean, snapshot_tree) =
        snapshot(&top, &common, &head, scratch.path(), Some(&attachments))?;

    // 1. Install the host's tools (sent each time, so both sides always
    // match), run its preflight, make the bare repositories and folders.
    let folder_ids = folder.map(|f| crate::remote_folder::ids(&f.vault, &f.folder, &f.session_key));
    let mut repo_ids = vec![repo_id.clone()];
    repo_ids.extend(folder_ids.iter().map(|(r, _)| r.clone()));
    // The folders this turn uses there are never pruned by this preparation.
    let mut keep = vec![wt_id.as_str()];
    keep.extend(folder_ids.iter().map(|(_, fid)| fid.as_str()));
    let (home, pruned) = host_prep(host, ssh, &repo_ids, &keep)?;

    // 2. Push HEAD and the snapshot, as `hollow-slot --remote` does: the
    // snapshot's objects live in a private folder, never this repository.
    let wt_hash = hash_of(&top_str, 12);
    let dest = format!("{host}:hollow-remote/repos/{repo_id}.git");
    let refs = [
        format!("{head}:refs/hollow-session/{wt_hash}-base"),
        format!("{snapshot}:refs/hollow-session/{wt_hash}-snap"),
    ];
    let ssh_cmd = git_ssh_command(ssh);
    let objects = scratch.path().join("objects");
    let alternates = Path::new(&common).join("objects");
    let mut env: Vec<(&str, &OsStr)> = vec![
        ("GIT_SSH_COMMAND", OsStr::new(&ssh_cmd)),
        ("GIT_SSH_VARIANT", OsStr::new("ssh")),
    ];
    if !clean {
        env.push(("GIT_OBJECT_DIRECTORY", objects.as_os_str()));
        env.push(("GIT_ALTERNATE_OBJECT_DIRECTORIES", alternates.as_os_str()));
    }
    git_env(
        &top,
        &[
            "push",
            "-q",
            "--no-verify",
            "--force",
            &dest,
            &refs[0],
            &refs[1],
        ],
        &env,
    )
    .map_err(|e| checkout_send_error(host, &e))?;

    // 3. Check it out in the session folder and get its paths.
    let ready = ssh_call(
        ssh,
        host,
        &format!(
            "/usr/bin/perl \"$HOME/hollow-remote/bin/hollow-remote-run\" --prepare {} {} {} {} {} {}",
            shell_quote(&repo_id),
            shell_quote(&wt_id),
            snapshot,
            head,
            shell_quote(branch.as_deref().unwrap_or("-")),
            if clean { 1 } else { 0 }
        ),
        b"",
    )?;
    let paths: Value = ready
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str(line).ok())
        .ok_or_else(|| format!("{host} didn't say where the session folder is"))?;
    let field = |key: &str| {
        paths
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| format!("{host} didn't give the session's {key}"))
    };
    let dir = field("dir")?;
    let scratch_dir = field("scratch")?;
    let target_dir = field("target")?;
    // Earlier turns' results no one brought back: here as branches (only once
    // the session is known to be ready, so the notice that names them shows).
    let kept = bring_kept(
        &top,
        &branch,
        host,
        ssh,
        &dest,
        &repo_id,
        &wt_hash,
        paths.get("kept"),
    );
    // The workspace's vault folder beside the worktree (Phase 3).
    let mut aside: Vec<String> = aside_of(&paths, &home).into_iter().collect();
    let (folder, folder_earlier) = match folder {
        Some(request) => {
            // A worktree copy just moved aside is named even when the folder
            // then fails (later preparations find no marker to say it).
            let (sync, _, earlier, folder_aside) = send_folder(host, ssh, &home, request, false)
                .map_err(|e| match aside.first() {
                    Some(at) => format!(
                        "{e} (what a run that stopped there left in the worktree, which couldn't be kept, is at {at} on {host})"
                    ),
                    None => e,
                })?;
            aside.extend(folder_aside);
            (Some(sync), earlier)
        }
        None => (None, None),
    };
    let identity = checkout_identity(&top);
    Ok(RemoteTarget {
        host: host.to_string(),
        ssh: ssh.to_string(),
        cwd: if prefix.is_empty() {
            dir.clone()
        } else {
            format!("{dir}/{}", prefix.trim_end_matches('/'))
        },
        dir,
        scratch: scratch_dir,
        port: paths.get("port").and_then(Value::as_u64).unwrap_or(0) as u16,
        target: target_dir,
        kept,
        home,
        worktree: top,
        head,
        snapshot,
        snapshot_tree,
        branch,
        repo_id,
        wt_id,
        git_worktree: true,
        folder,
        folder_earlier,
        turn: random_hex(8),
        pruned,
        aside,
        identity,
        access: Default::default(),
        reattach: None,
    })
}

/// The runner's `gh` or `gcloud` (remote runners, 2026-11-03): the real
/// program, given the run's credential for that one call when the workspace
/// turned it on (`hollow-remote-run --wrap`).
pub(crate) fn wrapper(tool: &str) -> String {
    format!(
        "#!/bin/sh\n# Hollow's {tool} on a remote runner (remote_session.rs): the real {tool}, with the\n# run's credential for this one call when its workspace turned it on. Sent by\n# Hollow before every turn; never edit by hand.\nexec /usr/bin/perl \"$(dirname \"$0\")/hollow-remote-run\" --wrap {tool} \"$@\"\n"
    )
}

/// Install the runner's tools, prune idle remote sessions' folders (never
/// `keep`'s), run its preflight, make the bare repositories `repo_ids` and
/// its folders; its home, and what the free-space prune removed.
fn host_prep(
    host: &str,
    ssh: &str,
    repo_ids: &[String],
    keep: &[&str],
) -> Result<(String, Option<DiskPruned>), String> {
    let files = json!({
        "hollow-remote-run": REMOTE_RUN_PL,
        "hollow-slot": crate::build_slots::SLOT_TOOL,
        "this-mac.md": crate::this_mac::runner_note(),
        // Remote runners (2026-11-03): first on the agent's PATH there, so a
        // turn's gh and gcloud get the run's credential one call at a time.
        "gh": wrapper("gh"),
        "gcloud": wrapper("gcloud"),
    });
    let ids: Vec<String> = repo_ids.iter().map(|id| shell_quote(id)).collect();
    let began = std::time::Instant::now();
    let prep = ssh_call(
        ssh,
        host,
        &format!(
            "/usr/bin/perl -e {} {} -- {}",
            shell_quote(HOST_PREP_PL),
            ids.join(" "),
            keep.iter()
                .map(|id| shell_quote(id))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        files.to_string().as_bytes(),
    )?;
    // Its preflight passed: a refusal found earlier no longer holds.
    crate::remote_status::note_prepared(host, began);
    let home = prep
        .lines()
        .find_map(|line| line.strip_prefix("home="))
        .map(str::to_string)
        .ok_or_else(|| format!("{host} didn't say where its home is"))?;
    // It goes into paths and the -R forward: absolute, one line, no `:`.
    if !home.starts_with('/') || home.contains([':', '\n', '\r', ' ']) {
        return Err(format!("{host} named an unusable home folder ({home})"));
    }
    // `pruned=<id> <free GB> <floor GB>`, one line per session removed.
    let mut pruned: Option<DiskPruned> = None;
    for line in prep.lines().filter_map(|l| l.strip_prefix("pruned=")) {
        let mut f = line.split(' ').skip(1).map(|v| v.parse::<u64>().ok());
        let (free_gb, floor_gb) = (f.next().flatten(), f.next().flatten());
        let entry = pruned.get_or_insert(DiskPruned {
            count: 0,
            free_gb: free_gb.unwrap_or(0),
            floor_gb: floor_gb.unwrap_or(0),
        });
        entry.count += 1;
    }
    Ok((home, pruned))
}

/// Get the runner ready for a vault workspace chat's turn (Phase 3): the
/// workspace folder is the session, checked out fresh there, with its own
/// scratch folder, port and build folder. Blocking (git and ssh).
pub(crate) fn prepare_folder(
    host: &str,
    ssh: &str,
    request: &FolderRequest,
) -> Result<RemoteTarget, String> {
    if !valid_host(host) {
        return Err(format!("{host} is not an SSH host name"));
    }
    let (repo_id, fid) =
        crate::remote_folder::ids(&request.vault, &request.folder, &request.session_key);
    let (home, pruned) = host_prep(host, ssh, std::slice::from_ref(&repo_id), &[fid.as_str()])?;
    let (sync, paths, earlier, aside) = send_folder(host, ssh, &home, request, true)?;
    let field = |key: &str| {
        paths
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| format!("{host} didn't give the session's {key}"))
    };
    Ok(RemoteTarget {
        host: host.to_string(),
        ssh: ssh.to_string(),
        home,
        dir: sync.dir.clone(),
        cwd: sync.dir.clone(),
        scratch: field("scratch")?,
        port: paths.get("port").and_then(Value::as_u64).unwrap_or(0) as u16,
        target: field("target")?,
        kept: Vec::new(),
        worktree: request.folder.clone(),
        head: String::new(),
        snapshot: String::new(),
        snapshot_tree: String::new(),
        branch: None,
        repo_id,
        wt_id: fid,
        git_worktree: false,
        folder: Some(sync),
        folder_earlier: earlier,
        turn: random_hex(8),
        pruned,
        aside: aside.into_iter().collect(),
        identity: checkout_identity(&request.folder),
        access: Default::default(),
        reattach: None,
    })
}

/// Send `request`'s folder: its snapshot pushed into the vault's bare
/// repository there and checked out fresh (`--prepare-folder`, `session`:
/// it is the agent's working folder); earlier turns' folder changes that
/// never came back are applied here first. The sync, the runner's answer,
/// what was applied, and where a copy it couldn't keep was moved there.
type FolderSent = (
    crate::remote_folder::FolderSync,
    Value,
    Option<crate::remote_folder::FolderBack>,
    Option<String>,
);

fn send_folder(
    host: &str,
    ssh: &str,
    home: &str,
    request: &FolderRequest,
    session: bool,
) -> Result<FolderSent, String> {
    // Only the first preparation can find a run that died there (it clears
    // the marker), so where it moved a copy is its answer's.
    let (sync, paths, _, aside) = send_folder_once(host, ssh, home, request, session)?;
    let earlier = bring_kept_folder(host, ssh, &sync, paths.get("kept"));
    // Earlier changes just applied here: send the folder again, so this
    // turn's copy has them (the runner has nothing more to keep by then).
    if earlier.as_ref().is_some_and(|e| {
        !e.changed.is_empty()
            || !e.merged.is_empty()
            || !e.copies.is_empty()
            || !e.unsure.is_empty()
    }) {
        let (sync, paths, _, _) =
            send_folder_once(host, ssh, home, request, session).map_err(|e| match &aside {
                Some(at) => format!(
                    "{e} (what a run that stopped there left in the folder, which couldn't be kept, is at {at} on {host})"
                ),
                None => e,
            })?;
        return Ok((sync, paths, earlier, aside));
    }
    Ok((sync, paths, earlier, aside))
}

/// One snapshot, push and `--prepare-folder`; kept results are the
/// caller's.
fn send_folder_once(
    host: &str,
    ssh: &str,
    home: &str,
    request: &FolderRequest,
    session: bool,
) -> Result<FolderSent, String> {
    use crate::remote_folder as rf;
    let (repo_id, fid) = rf::ids(&request.vault, &request.folder, &request.session_key);
    let git_dir = rf::private_repo(&request.vault)?;
    let hash = fid.rsplit('-').next().unwrap_or(&fid).to_string();
    let (snap, left) = rf::snapshot(&git_dir, &request.folder, &hash)?;
    let ssh_cmd = git_ssh_command(ssh);
    let env: [(&str, &OsStr); 3] = [
        ("GIT_DIR", git_dir.as_os_str()),
        ("GIT_SSH_COMMAND", OsStr::new(&ssh_cmd)),
        ("GIT_SSH_VARIANT", OsStr::new("ssh")),
    ];
    let dest = format!("{host}:hollow-remote/repos/{repo_id}.git");
    git_env(
        &git_dir,
        &[
            "push",
            "-q",
            "--no-verify",
            "--force",
            &dest,
            &format!("{snap}:refs/hollow-session/{hash}-snap"),
        ],
        &env,
    )
    .map_err(|e| format!("The workspace folder couldn't be sent to {host} ({e})"))?;
    let ready = ssh_call(
        ssh,
        host,
        &format!(
            "/usr/bin/perl \"$HOME/hollow-remote/bin/hollow-remote-run\" --prepare-folder {} {} {snap} {}",
            shell_quote(&repo_id),
            shell_quote(&fid),
            if session { 1 } else { 0 }
        ),
        b"",
    )?;
    let paths: Value = ready
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str(line).ok())
        .ok_or_else(|| format!("{host} didn't say where the workspace folder is"))?;
    let dir = paths
        .get("dir")
        .and_then(Value::as_str)
        .filter(|d| {
            d.starts_with(&format!("{home}/hollow-remote/folders/")) && !d.contains(['\n', '\r'])
        })
        .ok_or_else(|| format!("{host} didn't give the workspace folder's copy"))?
        .to_string();
    let sync = rf::FolderSync {
        local: request.folder.clone(),
        dir,
        repo_id,
        fid,
        snapshot: snap,
        git_dir,
        left,
    };
    let aside = aside_of(&paths, home);
    Ok((sync, paths, None, aside))
}

/// Earlier turns' folder results the runner kept (`[{ref, sha}]`): each
/// fetched, applied here three ways against its own snapshot, then dropped
/// there (compare and swap). One that can't be fetched or applied stays
/// there for the next turn.
fn bring_kept_folder(
    host: &str,
    ssh: &str,
    sync: &crate::remote_folder::FolderSync,
    listed: Option<&Value>,
) -> Option<crate::remote_folder::FolderBack> {
    let ssh_cmd = git_ssh_command(ssh);
    let env: [(&str, &OsStr); 3] = [
        ("GIT_DIR", sync.git_dir.as_os_str()),
        ("GIT_SSH_COMMAND", OsStr::new(&ssh_cmd)),
        ("GIT_SSH_VARIANT", OsStr::new("ssh")),
    ];
    let dir_env = [("GIT_DIR", sync.git_dir.as_os_str())];
    let src = format!("{host}:hollow-remote/repos/{}.git", sync.repo_id);
    let prefix = format!("refs/hollow-session/{}-kept-", sync.hash());
    let mut all = crate::remote_folder::FolderBack::default();
    let mut drops = Vec::new();
    // A `brought/<sha>` mark lasts only while the runner still lists that
    // result: once it no longer does (dropped there), the mark goes.
    let listed_shas: Vec<&str> = listed
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("sha").and_then(Value::as_str))
        .collect();
    let mark_prefix = format!("refs/hollow-folder/{}/brought/", sync.hash());
    if let Ok(found) = git_env(
        &sync.git_dir,
        &["for-each-ref", "--format=%(refname)", &mark_prefix],
        &dir_env,
    ) {
        for mark in found.lines() {
            let sha = mark.strip_prefix(&mark_prefix).unwrap_or_default();
            if !listed_shas.contains(&sha) {
                let _ = git_env(&sync.git_dir, &["update-ref", "-d", mark], &dir_env);
            }
        }
    }
    for item in listed.and_then(Value::as_array).into_iter().flatten() {
        let (Some(name), Some(sha)) = (
            item.get("ref").and_then(Value::as_str),
            item.get("sha").and_then(Value::as_str),
        ) else {
            continue;
        };
        let Some(suffix) = name.strip_prefix(&prefix) else {
            continue;
        };
        if suffix.is_empty()
            || !suffix
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
            || sha.len() < 40
            || !sha.chars().all(|c| c.is_ascii_hexdigit())
        {
            continue;
        }
        let local = format!("refs/hollow-folder/{}/kept", sync.hash());
        let got = git_env(
            &sync.git_dir,
            &[
                "fetch",
                "-q",
                "--no-tags",
                "--no-write-fetch-head",
                &src,
                &format!("+{name}:{local}"),
            ],
            &env,
        )
        .ok()
        .and_then(|_| git_env(&sync.git_dir, &["rev-parse", &local], &dir_env).ok());
        if got.as_deref() != Some(sha) {
            continue;
        }
        // An unchanged result is the snapshot itself (no parent). Otherwise
        // its parent must be a snapshot this Mac sent for an earlier turn (a
        // kept result is never on this turn's own, sent moments ago) to be
        // applied three ways: a base the runner made up could turn
        // "unchanged here" into a deletion or an overwrite of what is here.
        // One on any other base comes here only as conflicted copies beside
        // the files (overnight P3L-4: nothing here written over or removed,
        // the start notice names them), then goes there like any other.
        // Copies aren't idempotent like a three-way apply, so one brought is
        // marked here (`brought/<sha>`) until the runner has dropped it: a
        // drop that failed there is only sent again, never copied twice.
        let brought = format!("refs/hollow-folder/{}/brought/{sha}", sync.hash());
        let applied = match git_env(
            &sync.git_dir,
            &["rev-parse", "-q", "--verify", &format!("{sha}^1")],
            &dir_env,
        ) {
            Ok(base)
                if base != sync.snapshot
                    && crate::remote_folder::was_sent(&sync.git_dir, sync.hash(), &base) =>
            {
                crate::remote_folder::apply(&sync.git_dir, &sync.local, &base, sha)
            }
            Ok(_)
                if git_env(
                    &sync.git_dir,
                    &["rev-parse", "-q", "--verify", &brought],
                    &dir_env,
                )
                .is_ok() =>
            {
                Ok(crate::remote_folder::FolderBack::default())
            }
            Ok(base) => {
                let copied =
                    crate::remote_folder::apply_copies(&sync.git_dir, &sync.local, &base, sha);
                if copied.is_ok()
                    && git_env(&sync.git_dir, &["update-ref", &brought, sha], &dir_env).is_err()
                {
                    // Unmarked, it would be copied again: drop it there anyway.
                    eprintln!("[remote] couldn't mark {sha} as brought here");
                }
                copied
            }
            Err(_) => Ok(crate::remote_folder::FolderBack::default()),
        };
        let Ok(back) = applied else { continue };
        all.extend(back);
        drops.push(format!(
            "git -C \"$r\" update-ref -d {} {sha}",
            shell_quote(name)
        ));
    }
    if !drops.is_empty() {
        let _ = ssh_call(
            ssh,
            host,
            &format!(
                "r=\"$HOME/hollow-remote/repos/{}.git\"; {}",
                sync.repo_id,
                drops.join("; ")
            ),
            b"",
        );
    }
    (!all.is_empty()).then_some(all)
}

/// Bring every kept result the host listed (`[{ref, sha}]`) here, then have
/// the host drop exactly the ones that are here now (compare and swap on the
/// sha it listed). A commit a local branch already contains is here already
/// (not named again); any other goes on a `<branch>-mini-<n>`. One that
/// couldn't be fetched or kept stays there and is listed again at the next
/// preparation. Returns the branches, each once, for the start notice.
#[allow(clippy::too_many_arguments)]
fn bring_kept(
    top: &Path,
    branch: &Option<String>,
    host: &str,
    ssh: &str,
    dest: &str,
    repo_id: &str,
    wt_hash: &str,
    listed: Option<&Value>,
) -> Vec<String> {
    let ssh_cmd = git_ssh_command(ssh);
    let env: [(&str, &OsStr); 2] = [
        ("GIT_SSH_COMMAND", OsStr::new(&ssh_cmd)),
        ("GIT_SSH_VARIANT", OsStr::new("ssh")),
    ];
    let prefix = format!("refs/hollow-session/{wt_hash}-kept-");
    let mut branches: Vec<String> = Vec::new();
    let mut here = Vec::new();
    for item in listed.and_then(Value::as_array).into_iter().flatten() {
        let (Some(name), Some(sha)) = (
            item.get("ref").and_then(Value::as_str),
            item.get("sha").and_then(Value::as_str),
        ) else {
            continue;
        };
        let Some(suffix) = name.strip_prefix(&prefix) else {
            continue;
        };
        if suffix.is_empty()
            || !suffix
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
            || sha.len() < 40
            || !sha.chars().all(|c| c.is_ascii_hexdigit())
        {
            continue;
        }
        let local = format!("refs/hollow-remote/{wt_hash}/kept");
        let fetched = git_env(
            top,
            &[
                "fetch",
                "-q",
                "--no-tags",
                "--no-write-fetch-head",
                dest,
                &format!("+{name}:{local}"),
            ],
            &env,
        )
        .ok()
        .and_then(|_| git(top, &["rev-parse", &local]).ok());
        // What was fetched must be what the host listed, or it isn't dropped.
        let Some(got) = fetched.filter(|got| got == sha) else {
            continue;
        };
        // Already here (a branch contains it): nothing to name. Else a branch
        // of its own; if that fails it stays on the host for the next turn.
        let contained = git(
            top,
            &[
                "for-each-ref",
                "--contains",
                &got,
                "--format=%(refname)",
                "refs/heads/",
            ],
        )
        .is_ok_and(|out| !out.trim().is_empty());
        let kept_on = if contained {
            Some(None)
        } else {
            match keep_aside(top, branch, &got) {
                Ok(Collected::KeptAside { branch }) => Some(Some(branch)),
                _ => None,
            }
        };
        let Some(on) = kept_on else { continue };
        if let Some(on) = on {
            if !branches.contains(&on) {
                branches.push(on);
            }
        }
        here.push(format!(
            "git -C \"$r\" update-ref -d {} {got}",
            shell_quote(name)
        ));
    }
    if !here.is_empty() {
        let _ = ssh_call(
            ssh,
            host,
            &format!(
                "r=\"$HOME/hollow-remote/repos/{repo_id}.git\"; {}",
                here.join("; ")
            ),
            b"",
        );
    }
    branches
}

/// `/tmp/hollow-relay-<uid>`, made 0700 and checked to be this user's: a
/// socket path must be short (104 bytes), and `/tmp` is shared.
fn local_socket_dir() -> Result<PathBuf, String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let dir = PathBuf::from(format!("/tmp/hollow-relay-{}", unsafe { libc::geteuid() }));
    let _ = std::fs::create_dir(&dir);
    let meta = std::fs::symlink_metadata(&dir).map_err(|e| format!("no relay folder ({e})"))?;
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } {
        return Err(format!(
            "refusing the relay folder {}: not a folder of this user's",
            dir.display()
        ));
    }
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("the relay folder can't be made private ({e})"))?;
    Ok(dir)
}

/// The chat folder's attachments ([`crate::chat::ATTACHMENTS_DIR`]) as a
/// path in the checkout, from the chat folder's place in it (`rev-parse
/// --show-prefix`, or the remote cwd past the checkout's folder).
fn attachments_in_checkout(prefix: &str) -> String {
    match prefix.trim_matches('/') {
        "" => crate::chat::ATTACHMENTS_DIR.to_string(),
        p => format!("{p}/{}", crate::chat::ATTACHMENTS_DIR),
    }
}

/// Whether `rel` under `top` is a real folder that holds an attached file
/// (more than its `.gitignore`).
fn holds_attachments(top: &Path, rel: &str) -> bool {
    let dir = top.join(rel);
    if !std::fs::symlink_metadata(&dir).is_ok_and(|meta| meta.is_dir()) {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return false;
    };
    entries
        .flatten()
        .any(|entry| entry.file_name() != ".gitignore")
}

/// HEAD, or a commit of HEAD plus every uncommitted and untracked change,
/// made with a copy of the index in a private object folder under `scratch`,
/// so this repository gets nothing. Returns (commit, clean, compared tree).
/// The chat's attachments (`attachments`, a path in the checkout, given
/// when a turn is sent) ignore themselves with a `.gitignore` of `/*` (C5
/// review), so `status` and `add -A` pass them over: they are added to the
/// commit by name, so they travel with the turn, but never to the compared
/// tree (C5 third review). The runner's result leaves them out as well, so
/// whether the turn changed anything, whether this Mac changed meanwhile
/// (an attachment saved during the turn, for the next message, doesn't
/// count), the apply (in neither tree, they stay as they are) and a result
/// kept on a branch never involve them.
fn snapshot(
    top: &Path,
    common: &str,
    head: &str,
    scratch: &Path,
    attachments: Option<&str>,
) -> Result<(String, bool, String), String> {
    // `dirty`, not the pinned `all`: a submodule's new commit still counts,
    // and git still never enters a submodule to look at its files.
    let status = git(
        top,
        &[
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--ignore-submodules=dirty",
        ],
    )?;
    let attached = attachments.filter(|rel| holds_attachments(top, rel));
    if status.is_empty() && attached.is_none() {
        let tree = git(top, &["rev-parse", &format!("{head}^{{tree}}")])?;
        return Ok((head.to_string(), true, tree));
    }
    let objects = scratch.join("objects");
    std::fs::create_dir_all(&objects).map_err(|e| format!("no temp folder here ({e})"))?;
    let index = scratch.join("index");
    if let Ok(own) = git(
        top,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    ) {
        let _ = std::fs::copy(own, &index);
    }
    let alternates = Path::new(common).join("objects");
    let env: Vec<(&str, &OsStr)> = vec![
        ("GIT_INDEX_FILE", index.as_os_str()),
        ("GIT_OBJECT_DIRECTORY", objects.as_os_str()),
        ("GIT_ALTERNATE_OBJECT_DIRECTORIES", alternates.as_os_str()),
        ("GIT_AUTHOR_NAME", OsStr::new("hollow-session")),
        ("GIT_AUTHOR_EMAIL", OsStr::new("hollow-session@localhost")),
        ("GIT_COMMITTER_NAME", OsStr::new("hollow-session")),
        (
            "GIT_COMMITTER_EMAIL",
            OsStr::new("hollow-session@localhost"),
        ),
    ];
    git_env(top, &["add", "-A", "."], &env)
        .map_err(|e| format!("the uncommitted changes couldn't be gathered ({e})"))?;
    let compared = git_env(top, &["write-tree"], &env)?;
    let tree = match attached {
        Some(rel) => {
            let add = ["--literal-pathspecs", "add", "-f", "--", rel];
            git_env(top, &add, &env)
                .map_err(|e| format!("the attached files couldn't be gathered ({e})"))?;
            git_env(top, &["write-tree"], &env)?
        }
        None => compared.clone(),
    };
    // Only attachments HEAD already holds (sent before C5): nothing changed.
    let head_tree = format!("{head}^{{tree}}");
    if status.is_empty() && git(top, &["rev-parse", &head_tree]).is_ok_and(|t| t == tree) {
        return Ok((head.to_string(), true, tree));
    }
    let commit = git_env(
        top,
        &[
            "-c",
            "commit.gpgSign=false",
            "commit-tree",
            &tree,
            "-p",
            head,
            "-m",
            &format!("hollow-session: uncommitted changes on {head}"),
        ],
        &env,
    )?;
    Ok((commit, false, compared))
}

/// `'…'` for a POSIX shell (the host's login shell is zsh; the quoting is the
/// same).
pub(crate) fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Variables that name this Mac's paths or Hollow's own process tracking:
/// never sent to the host (it sets its own, or has no use for them).
const LOCAL_ONLY_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "TMPDIR",
    "TEMP",
    "TMP",
    "HOLLOW_SCRATCH_DIR",
    "CARGO_TARGET_DIR",
    "PORT",
    "PW_PORT",
    "HOLLOW_BUILD_SLOT_DIR",
    "HOLLOW_THIS_MAC",
    "HOLLOW_SLOT_REMOTE",
    // Smart routing 3: routing is off on the runner anyway.
    "HOLLOW_SLOT_ROUTE",
    "HOLLOW_RUN_TAG",
    "HOLLOW_BOUND_RUN_IDS",
    "HOLLOW_RELAY_TOKEN",
    "HOLLOW_RELAY_SOCK",
    "HOLLOW_REMOTE_HEADER",
    // Remote runners (2026-11-03): a credential reaches the runner only one
    // request at a time through the leader (`runner_access`), never in the
    // header, whatever this Mac's environment holds.
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "CLOUDSDK_AUTH_ACCESS_TOKEN_FILE",
    "HOLLOW_CRED_GITHUB",
    "HOLLOW_CRED_GITHUB_WHY",
    "HOLLOW_CRED_GCLOUD_MINT",
    "HOLLOW_CRED_GCLOUD_WHY",
    "HOLLOW_CRED_GCLOUD_LEVEL",
    // Only the turn's own access sets these there (`runner_access::header_env`).
    "HOLLOW_RUNNER_GITHUB",
    "HOLLOW_RUNNER_CLOUD",
    "HOLLOW_RUNNER_ORIGIN",
    "GIT_CONFIG_GLOBAL",
];

/// The tools the agent gets on the host: each one the bridge to the same
/// server here. Claude's `--mcp-config` takes this as a JSON string (no
/// secrets in it: the relay token comes from the environment).
pub(crate) fn bridge_config(config: &str, home: &str) -> Result<String, String> {
    let value: Value =
        serde_json::from_str(config).map_err(|e| format!("Invalid tool configuration: {e}"))?;
    let servers = value
        .get("mcpServers")
        .and_then(Value::as_object)
        .ok_or("Invalid tool configuration: missing mcpServers")?;
    let mut out = serde_json::Map::new();
    for (name, server) in servers {
        // An HTTP server is reached from the host directly: only one with no
        // headers (they are often credentials, and this goes on argv there).
        if server.get("url").is_some() {
            if server.get("headers").is_none() {
                out.insert(name.clone(), server.clone());
            }
            continue;
        }
        out.insert(
            name.clone(),
            json!({
                "command": "/usr/bin/perl",
                "args": [format!("{home}/hollow-remote/bin/hollow-remote-run"), "--bridge", name],
            }),
        );
    }
    Ok(json!({ "mcpServers": out }).to_string())
}

/// The `command=…,args=[…],env_vars=[…]` fields that start Codex's server
/// `name` on the host through the bridge (Codex passes a server only the
/// variables it names).
pub(crate) fn codex_bridge_fields(home: &str, name: &str) -> [String; 3] {
    let args = json!([
        format!("{home}/hollow-remote/bin/hollow-remote-run"),
        "--bridge",
        name
    ]);
    [
        "command=\"/usr/bin/perl\"".to_string(),
        format!("args={args}"),
        "env_vars=[\"HOLLOW_RELAY_SOCK\",\"HOLLOW_RELAY_TOKEN\"]".to_string(),
    ]
}

/// The header the host's `--run` reads: the agent's command line and
/// environment, and where to run it.
#[derive(Debug, Serialize, Deserialize)]
struct Header {
    v: u32,
    program: String,
    args: Vec<String>,
    env: std::collections::BTreeMap<String, String>,
    cwd: String,
    dir: String,
    scratch: String,
    port: u16,
    target: String,
    sock: String,
    /// This attempt's id (its sockets there and here, its process tag).
    run: String,
    token: String,
    head_ref: String,
    result_ref: String,
    /// This leader reads the run's stderr lines as `e` marks (overnight
    /// R2); a runner from before ignores the key and sends them bare.
    err_lines: bool,
    /// A vault workspace's folder (Phase 3): `--run` commits it on top of
    /// the snapshot that was sent, to its own result ref.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    folder: Option<HeaderFolder>,
    /// R5b (Q-R33): the run's prompt, which the runner makes the agent's
    /// stdin through an unlinked private file, so `ps` there never shows it
    /// (the same hand-over a local run gets).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stdin_text: Option<String>,
    /// R5b: files the runner opens, unlinked and private, on these
    /// descriptors (3 to 99) for the agent: today the appended system prompt
    /// (the account token stays in its environment, see [`into_leader_with`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    fds: Vec<HeaderFd>,
}

/// One private file for the agent's descriptor `fd` ([`Header::fds`]).
#[derive(Debug, Serialize, Deserialize)]
struct HeaderFd {
    fd: i32,
    text: String,
}

/// What a remote run's argv would carry but must not (R5b): built by
/// [`crate::agent::build_command`], placed in the header by [`into_leader_with`].
#[derive(Debug, Default, Clone)]
pub(crate) struct Handover {
    /// The prompt, read by the agent from stdin.
    pub stdin: Option<String>,
    /// Claude's appended system prompt.
    pub append: Option<String>,
}

/// The folder part of the header.
#[derive(Debug, Serialize, Deserialize)]
struct HeaderFolder {
    dir: String,
    repo: String,
    base: String,
    result_ref: String,
    exclude: String,
}

/// Turn the agent command `agent` (built for the host's paths) into the
/// run's local leader. `relay_config` is the run's private config file
/// (`run_auth::write_config`), which the leader starts the tools from;
/// `claude_config` replaces the value of Claude's `--mcp-config` with the
/// bridges. Fails when the run would use a login stored on this Mac (no
/// token to hand over).
#[cfg(test)]
pub(crate) fn into_leader(
    agent: &Command,
    kind: crate::agent::AgentKind,
    target: &RemoteTarget,
    relay_config: Option<&Path>,
    claude_config: Option<&str>,
) -> Result<Command, String> {
    into_leader_with(
        agent,
        kind,
        target,
        relay_config,
        claude_config,
        Handover::default(),
    )
}

/// [`into_leader`] with what the argv left out ([`Handover`]), which goes in
/// the header: the prompt as the agent's stdin, the appended system prompt
/// as a private file on a descriptor (R5b, Q-R33).
pub(crate) fn into_leader_with(
    agent: &Command,
    kind: crate::agent::AgentKind,
    target: &RemoteTarget,
    relay_config: Option<&Path>,
    claude_config: Option<&str>,
    handover: Handover,
) -> Result<Command, String> {
    use crate::agent::AgentKind;
    let std_cmd = agent.as_std();
    let program = match kind {
        AgentKind::Claude => "claude",
        AgentKind::Codex => "codex",
        _ => return Err("Only Claude and Codex can run on the runner.".into()),
    };
    let mut args: Vec<String> = std_cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    if let Some(bridges) = claude_config {
        if let Some(i) = args.iter().position(|a| a == "--mcp-config") {
            if let Some(value) = args.get_mut(i + 1) {
                *value = bridges.to_string();
            }
        }
    }
    let mut env = std::collections::BTreeMap::new();
    for (key, value) in std_cmd.get_envs() {
        let key = key.to_string_lossy().into_owned();
        if LOCAL_ONLY_ENV.contains(&key.as_str()) {
            continue;
        }
        if let Some(value) = value {
            env.insert(key, value.to_string_lossy().into_owned());
        }
    }
    let mut fds: Vec<HeaderFd> = Vec::new();
    // ENG-33: a Claude run on the runner is never the machine's login. No
    // token to hand over, or an account the command names `login`, is a
    // refusal, not a fallback (the Acme work login lives on the mini).
    let on_login = env
        .get(crate::claude_accounts::ACCOUNT_ENV)
        .is_some_and(|a| a.trim().is_empty() || a == crate::claude_accounts::LOGIN_ID);
    match kind {
        AgentKind::Claude if on_login || !env.contains_key("CLAUDE_CODE_OAUTH_TOKEN") => {
            return Err("Running Claude on the runner needs one of Hollow's added Claude accounts with room (Settings → Accounts): the Claude Code login stays on this Mac.".into());
        }
        AgentKind::Claude => {
            // The account token stays in the agent's environment there: with
            // a login present Claude 2.1.325 ignores the token descriptor
            // (R5b review), so only the appended prompt goes on a descriptor.
            if let Some(text) = handover.append.clone() {
                args.push("--append-system-prompt-file".into());
                args.push(format!("/dev/fd/{}", crate::secret_fds::APPEND_FD));
                fds.push(HeaderFd {
                    fd: crate::secret_fds::APPEND_FD,
                    text,
                });
            }
        }
        AgentKind::Codex => {
            let (key, value) = codex_credential()?;
            env.insert(key, value);
        }
        _ => {}
    }
    // Remote runners (2026-11-03): what this turn may reach beyond its
    // checkout. Flags and git's configuration ride in the header; secrets
    // only in the leader's environment, handed out one request at a time.
    crate::runner_access::header_env(&target.access, &mut env);
    let creds = crate::runner_access::credentials(&target.access);
    let mut cmd = leader_with(
        program,
        args,
        env,
        target,
        relay_config,
        args_take_stdin(kind, std_cmd),
        handover.stdin,
        fds,
    )?;
    for (key, value) in crate::runner_access::leader_env(&creds) {
        cmd.env(key, value);
    }
    // Hollow's own tracking stays on the leader (the run tag, the bound run
    // ids the guard registers, and the account's id: the usage ledger reads
    // the account from this command, so without it a run on an added account
    // was recorded as `login`, ENG-33), and so does the agent's PATH: the tools the
    // relay starts here need it (a library connector run by npx or uvx when
    // Hollow was opened from Finder).
    for (key, value) in std_cmd.get_envs() {
        let key = key.to_string_lossy();
        if matches!(
            key.as_ref(),
            "HOLLOW_RUN_TAG"
                | "HOLLOW_BOUND_RUN_IDS"
                | "PATH"
                | crate::claude_accounts::ACCOUNT_ENV
        ) {
            if let Some(value) = value {
                cmd.env(key.as_ref(), value);
            }
        }
    }
    Ok(cmd)
}

/// The local leader for `program` (`claude`, `codex`, or the runner's own
/// `hollow-remote-probe`) with `args` and `env` on the host, in `target`.
/// `live`: the program reads this run's stdin.
#[cfg(test)]
pub(crate) fn leader(
    program: &str,
    args: Vec<String>,
    env: std::collections::BTreeMap<String, String>,
    target: &RemoteTarget,
    relay_config: Option<&Path>,
    live: bool,
) -> Result<Command, String> {
    leader_with(
        program,
        args,
        env,
        target,
        relay_config,
        live,
        None,
        Vec::new(),
    )
}

/// [`leader`] with the header's private hand-overs ([`Header::stdin_text`],
/// [`Header::fds`]).
#[allow(clippy::too_many_arguments)]
fn leader_with(
    program: &str,
    args: Vec<String>,
    mut env: std::collections::BTreeMap<String, String>,
    target: &RemoteTarget,
    relay_config: Option<&Path>,
    live: bool,
    stdin_text: Option<String>,
    fds: Vec<HeaderFd>,
) -> Result<Command, String> {
    env.insert("TMPDIR".into(), format!("{}/", target.scratch));
    env.insert("HOLLOW_SCRATCH_DIR".into(), target.scratch.clone());
    // This Mac's identity for the checkout, over the same channel as the
    // rest of the header (never argv, never the runner's config): the
    // runner account has none, and a work checkout keeps its own. A
    // variable the agent command sets itself stays, the rest are this
    // Mac's: git's own rule here (an env name with the configured email).
    if let Some(identity) = &target.identity {
        for (key, value) in identity.env() {
            env.entry(key.to_string())
                .or_insert_with(|| value.to_string());
        }
    }
    let (head_ref, result_ref) = if target.git_worktree {
        target.result_refs()
    } else {
        (String::new(), String::new())
    };
    let mut token = random_hex(32);
    // Sockets of this attempt's own: a retried attempt (an account switch)
    // never meets the last one's forward or has its socket removed by it.
    // The run id starts with the turn's, so the turn's finish stops only its
    // own runs there.
    let turn = if target.turn.len() == 8 {
        target.turn.clone()
    } else {
        random_hex(8)
    };
    let mut run = format!("{turn}{}", random_hex(4));
    // Overnight R1: picking up a run an earlier Hollow started there: its
    // own run id and relay token (its supervisor adds that token to every
    // tool connection), attached from the bytes that Hollow handled.
    if let Some(r) = &target.reattach {
        run = r.run.clone();
        token = r.token.clone();
    }
    // A reattaching leader's own relay socket here, so a leader of the same
    // run still alive (orphaned) can't remove it when it quits.
    let socket_local = match &target.reattach {
        Some(_) => local_socket_dir()?.join(format!("{run}.{}.sock", random_hex(4))),
        None => local_socket_dir()?.join(format!("{run}.sock")),
    };
    // Each connection forwards `<base>.<n>.sock` (Phase 4: a reconnect takes
    // a new name; the first is the header's).
    let socket_base = format!("{}/hollow-remote/run/{run}", target.home);
    let socket_remote = format!("{socket_base}.0.sock");
    let header = Header {
        v: 1,
        program: program.into(),
        args,
        env,
        cwd: target.cwd.clone(),
        dir: target.dir.clone(),
        scratch: target.scratch.clone(),
        port: target.port,
        target: target.target.clone(),
        sock: socket_remote,
        run: run.clone(),
        token: token.clone(),
        head_ref,
        result_ref,
        err_lines: true,
        folder: target.folder.as_ref().map(|f| HeaderFolder {
            dir: f.dir.clone(),
            repo: format!("{}/hollow-remote/repos/{}.git", target.home, f.repo_id),
            base: f.snapshot.clone(),
            result_ref: f.result_ref(),
            exclude: crate::remote_folder::EXCLUDE.to_string(),
        }),
        stdin_text,
        fds,
    };
    let header = serde_json::to_string(&header).map_err(|e| e.to_string())?;
    let mut cmd = Command::new("/usr/bin/perl");
    cmd.arg("-e")
        .arg(LEADER_PL)
        .arg(
            relay_config
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "-".into()),
        )
        .arg(&socket_local)
        .arg(&socket_base)
        .arg(&run)
        .arg("--")
        .arg(&target.ssh)
        .args(SSH_OPTS)
        .arg(&target.host)
        .env("HOLLOW_RELAY_TOKEN", token)
        .current_dir(&target.worktree)
        .stdin(if live { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Overnight R1: the leader keeps what a restarted Hollow needs to attach
    // to this run in the turn's state file (0600, this Mac's app data).
    if let Ok(dir) = turns_dir() {
        cmd.env("HOLLOW_REMOTE_STATE", state_path(&dir, &turn));
    }
    match &target.reattach {
        // Attach from the bytes that Hollow handled: no header (the run has
        // it), the run's own attach secret, a forwarded socket name no
        // earlier leader used.
        Some(r) => {
            let first = 1000 + u32::from_str_radix(&random_hex(5), 16).unwrap_or(0) % 900_000;
            cmd.env("HOLLOW_ATTACH_SECRET", &r.secret).env(
                "HOLLOW_REMOTE_ATTACH",
                format!("{} {} {} {first}", r.out, r.err, r.sup),
            );
        }
        None => {
            cmd.env("HOLLOW_REMOTE_HEADER", header);
        }
    }
    Ok(cmd)
}

/// Whether the agent command reads stdin (a steerable turn).
fn args_take_stdin(kind: crate::agent::AgentKind, cmd: &std::process::Command) -> bool {
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    match kind {
        crate::agent::AgentKind::Claude => {
            args.iter().any(|a| a == "stream-json")
                && args
                    .windows(2)
                    .any(|w| w[0] == "--input-format" && w[1] == "stream-json")
        }
        crate::agent::AgentKind::Codex => args.iter().any(|a| a == "app-server"),
        _ => false,
    }
}

/// What a remote Codex run is told when it can't be given a credential.
pub(crate) const CODEX_CHATGPT_REFUSAL: &str = "Codex is signed in with ChatGPT on this Mac, and that sign-in can reach the runner only as a file stored there, which Hollow doesn't do. Set Runs on to This Mac for Codex turns; Claude turns run on the runner with one of your added accounts.";

/// The Codex credential a remote run hands over: an OpenAI API key, from
/// this Mac's Codex login (`$CODEX_HOME/auth.json`, else
/// `~/.codex/auth.json`), as `CODEX_API_KEY`, the only Codex credential its
/// environment carries. A ChatGPT sign-in has none to hand over: Codex reads
/// it only from a file (its `CODEX_ACCESS_TOKEN` takes a Codex agent
/// identity, not a ChatGPT token; checked with Codex 0.164 and 0.166), so
/// such a run is refused ([`CODEX_CHATGPT_REFUSAL`]).
fn codex_credential() -> Result<(String, String), String> {
    let home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))
        .ok_or("No Codex sign-in was found on this Mac.")?;
    let text = std::fs::read_to_string(home.join("auth.json")).map_err(|_| {
        "Codex isn't signed in on this Mac, so it can't run on the runner.".to_string()
    })?;
    let auth: Value = serde_json::from_str(&text)
        .map_err(|_| "Codex's sign-in file couldn't be read.".to_string())?;
    auth.get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .filter(|k| !k.is_empty())
        .map(|key| ("CODEX_API_KEY".to_string(), key.to_string()))
        .ok_or_else(|| CODEX_CHATGPT_REFUSAL.to_string())
}

/// The remote turn each conversation has under way (one at a time), from
/// its preparation until its finish brings the work back
/// (`chat::finish_observed_run`), whichever way the turn ends.
static TURNS: once_cell::sync::Lazy<
    std::sync::Mutex<std::collections::HashMap<String, RemoteTarget>>,
> = once_cell::sync::Lazy::new(Default::default);

pub(crate) fn begin_turn(session_key: &str, target: RemoteTarget, agent: crate::agent::AgentKind) {
    // Overnight R1: kept on disk too, so a Hollow that quits mid-turn finds
    // it at the conversation's next turn ([`stale_turn`]).
    if let Ok(dir) = turns_dir() {
        if let Err(error) = save_record(&dir, session_key, &target, agent.as_str()) {
            eprintln!("[remote] can't keep the turn's record ({error})");
        }
    }
    TURNS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(session_key.to_string(), target);
}

/// The turn's target at its finish, out of the table; its record on disk
/// stays until [`forget_turn`], after its work came back.
pub(crate) fn take_turn(session_key: &str) -> Option<RemoteTarget> {
    TURNS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(session_key)
}

// ── Reattaching after a restart (overnight R1) ──────────────────────────────
//
// The table above is in memory, so a Hollow that quit mid-turn used to lose
// the run: the next preparation of the folder stopped it there and kept its
// work as a branch. Now each remote turn leaves three small files here
// (app data `RemoteTurns/`, 0700): its target (`<session hash>.json`, by
// [`begin_turn`]), what its leader knows (`<turn>.state`: run, relay token,
// attach secret, supervisor pid, err offset; written by the leader), and
// the stdout bytes Hollow had handled (`<turn>.out`, [`Progress`]). The
// conversation's next turn finds them ([`stale_turn`]): Continue reattaches
// through `--attach` from exactly that byte; any other message first brings
// that turn's work back ([`collect_placed`], which stops it there).

/// What a leader needs to attach to a run an earlier Hollow started.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reattach {
    pub run: String,
    pub token: String,
    pub secret: String,
    pub sup: u32,
    /// Stdout bytes Hollow handled, and the err offset the leader had.
    pub out: u64,
    pub err: u64,
}

/// A remote turn an earlier Hollow left under way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleTurn {
    pub target: RemoteTarget,
    /// The agent it ran (`claude`, `codex`).
    pub agent: String,
    /// None when its leader never heard from the run there.
    pub reattach: Option<Reattach>,
}

#[derive(Serialize, Deserialize)]
struct TurnRecord {
    v: u32,
    session_key: String,
    #[serde(default)]
    agent: String,
    target: RemoteTarget,
}

pub(crate) fn turns_dir() -> Result<PathBuf, String> {
    let dir = crate::agent_settings::app_data_dir()?.join("RemoteTurns");
    make_private_dir(&dir)?;
    Ok(dir)
}

fn make_private_dir(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("{}: {e}", dir.display()))
}

fn record_path(dir: &Path, session_key: &str) -> PathBuf {
    dir.join(format!("{}.json", hash_of(session_key, 16)))
}

/// The leader's own file for the turn (`HOLLOW_REMOTE_STATE`).
pub(crate) fn state_path(dir: &Path, turn: &str) -> PathBuf {
    dir.join(format!("{turn}.state"))
}

fn out_path(dir: &Path, turn: &str) -> PathBuf {
    dir.join(format!("{turn}.out"))
}

/// Write `bytes` to `path` (0600) by rename.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    file.write_all(bytes)
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

pub(crate) fn save_record(
    dir: &Path,
    session_key: &str,
    target: &RemoteTarget,
    agent: &str,
) -> Result<(), String> {
    prune_records(dir);
    // An earlier turn's files this record replaces (its runner didn't
    // answer when it was last looked at) go with it: its token and secret.
    if let Some(old) = read_record(dir, session_key) {
        if old.target.turn != target.turn {
            forget_turn_in(dir, session_key, &old.target.turn);
        }
    }
    let record = TurnRecord {
        v: 1,
        session_key: session_key.to_string(),
        agent: agent.to_string(),
        target: target.clone(),
    };
    let text = serde_json::to_vec(&record).map_err(|e| e.to_string())?;
    write_private(&record_path(dir, session_key), &text)
}

/// Records no turn came back for in 14 days go (their runs there stopped
/// themselves after 12 hours unheard, and a preparation since kept them).
fn prune_records(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let old = std::time::Duration::from_secs(14 * 24 * 3600);
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > old);
        if stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// The leader's file: `run`, `token`, `secret`, `sup` and `err` lines.
fn read_state(path: &Path) -> Option<Reattach> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut r = Reattach::default();
    for line in text.lines() {
        let (key, value) = line.split_once(' ')?;
        match key {
            "run" => r.run = value.to_string(),
            "token" => r.token = value.to_string(),
            "secret" => r.secret = value.to_string(),
            "sup" => r.sup = value.parse().ok()?,
            "err" => r.err = value.parse().ok()?,
            _ => {}
        }
    }
    let hex = |s: &str, n: usize| s.len() >= n && s.bytes().all(|b| b.is_ascii_hexdigit());
    (hex(&r.run, 12) && r.run.len() <= 32 && hex(&r.token, 16) && hex(&r.secret, 16) && r.sup > 1)
        .then_some(r)
}

/// The remote turn this conversation had under way in an earlier Hollow:
/// a record on disk with no turn of this process's in the table. Its
/// `reattach` is what the leader had (None when it never heard from the
/// run), with the stdout bytes Hollow handled.
pub(crate) fn stale_turn(session_key: &str) -> Option<StaleTurn> {
    if TURNS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(session_key)
    {
        return None;
    }
    stale_turn_in(&turns_dir().ok()?, session_key)
}

/// The record, when it is one Hollow could have written: its names safe for
/// the paths and the runner commands they go into.
fn read_record(dir: &Path, session_key: &str) -> Option<TurnRecord> {
    let text = std::fs::read(record_path(dir, session_key)).ok()?;
    let record: TurnRecord = serde_json::from_slice(&text).ok()?;
    let t = &record.target;
    let hex8 = t.turn.len() == 8 && t.turn.bytes().all(|b| b.is_ascii_hexdigit());
    let name = |v: &str| {
        !v.is_empty()
            && v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    };
    let ok = record.v == 1
        && record.session_key == session_key
        && hex8
        && valid_host(&t.host)
        && name(&t.repo_id)
        && (!t.git_worktree || name(&t.wt_id))
        && t.folder
            .as_ref()
            .is_none_or(|f| name(&f.repo_id) && name(&f.fid))
        && t.home.starts_with('/')
        && t.dir.starts_with(&t.home);
    ok.then_some(record)
}

pub(crate) fn stale_turn_in(dir: &Path, session_key: &str) -> Option<StaleTurn> {
    let record = read_record(dir, session_key)?;
    let turn = record.target.turn.clone();
    let reattach = read_state(&state_path(dir, &turn))
        .filter(|r| r.run.starts_with(&turn))
        .map(|mut r| {
            r.out = std::fs::read_to_string(out_path(dir, &turn))
                .ok()
                .and_then(|t| t.trim().parse().ok())
                .unwrap_or(0);
            r
        });
    Some(StaleTurn {
        target: record.target,
        agent: record.agent,
        reattach,
    })
}

/// Drop the turn's files once its work came back (or nothing is left to
/// attach to).
pub(crate) fn forget_turn(session_key: &str, turn: &str) {
    if let Ok(dir) = turns_dir() {
        forget_turn_in(&dir, session_key, turn);
    }
}

pub(crate) fn forget_turn_in(dir: &Path, session_key: &str, turn: &str) {
    let _ = std::fs::remove_file(record_path(dir, session_key));
    if turn.len() == 8 && turn.bytes().all(|b| b.is_ascii_hexdigit()) {
        let _ = std::fs::remove_file(state_path(dir, turn));
        let _ = std::fs::remove_file(out_path(dir, turn));
    }
}

/// What the conversation's next turn does with a stale one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleChoice {
    /// Continue: this turn picks the run up through `--attach`.
    Reattach,
    /// Anything else: that turn's work comes back first (stopping it there).
    BringBack,
}

/// Overnight R1's rule (pure). Only a Continue (the composer's, or Resume
/// in Waiting on you: the same text) of a Claude turn going to the same
/// runner, in the folder the stale turn ran from, with what its leader knew,
/// reattaches (the earlier turn a Claude one too); Codex's app server needs
/// Hollow to drive each turn, so it never does.
pub(crate) fn stale_choice(
    stale: &StaleTurn,
    prompt: &str,
    agent: crate::agent::AgentKind,
    host: Option<&str>,
    cwd: &Path,
) -> StaleChoice {
    let continuing = prompt.trim() == crate::restart_resume::CONTINUE_PROMPT.trim();
    // The folder as git names it (its real path: `/private/tmp`, not `/tmp`).
    let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let inside = cwd.starts_with(&stale.target.worktree)
        || real(cwd).starts_with(real(&stale.target.worktree));
    let same_place = host == Some(stale.target.host.as_str()) && inside;
    if continuing
        && same_place
        && agent == crate::agent::AgentKind::Claude
        && stale.agent == "claude"
        && stale.reattach.is_some()
    {
        StaleChoice::Reattach
    } else {
        StaleChoice::BringBack
    }
}

/// What became of a stale turn before this turn's preparation.
#[derive(Debug)]
pub enum StaleOutcome {
    /// This turn attaches to it, with this target.
    Reattach(RemoteTarget),
    /// Its work came back first (stopping it there): the worktree's return
    /// (and whether its work is still on the runner) and the folder's.
    BroughtBack {
        target: RemoteTarget,
        worktree: (Result<Collected, String>, bool),
        folder: Option<Result<crate::remote_folder::FolderBack, String>>,
    },
    /// Nothing there to attach to or bring now: its run is gone (what it
    /// left is kept by the preparation, as before) or the runner didn't
    /// answer (the record stays for a later turn).
    Nothing,
}

/// Act on `choice` for `stale` (blocking: ssh and git). A run still there
/// is attached to or stopped and brought back; its files go once its work
/// is here.
pub(crate) fn settle_stale(
    session_key: &str,
    stale: StaleTurn,
    choice: StaleChoice,
    going_there: bool,
) -> StaleOutcome {
    // A turn that isn't going to that runner doesn't wait on it while it
    // was found unreachable in the last 5 minutes (the record stays for a
    // later turn); one going there, or a Continue, asks as usual.
    if choice == StaleChoice::BringBack
        && !going_there
        && crate::remote_status::recently_unreachable(
            &stale.target.host,
            std::time::Duration::from_secs(300),
        )
    {
        return StaleOutcome::Nothing;
    }
    let Some(run) = stale.reattach.as_ref().map(|r| r.run.clone()) else {
        // Its leader never heard from the run: nothing to attach to; the
        // preparation keeps whatever it left there.
        forget_turn(session_key, &stale.target.turn);
        return StaleOutcome::Nothing;
    };
    match run_there(&stale.target, &run) {
        Err(_) => StaleOutcome::Nothing,
        Ok(false) => {
            forget_turn(session_key, &stale.target.turn);
            StaleOutcome::Nothing
        }
        Ok(true) if choice == StaleChoice::Reattach => {
            let mut target = stale.target;
            target.reattach = stale.reattach;
            StaleOutcome::Reattach(target)
        }
        Ok(true) => {
            let worktree = collect_placed(&stale.target);
            let folder = collect_folder(&stale.target);
            forget_turn(session_key, &stale.target.turn);
            StaleOutcome::BroughtBack {
                target: stale.target,
                worktree,
                folder,
            }
        }
    }
}

/// Whether the run is still there to attach to: its folder in `run/` (live,
/// or ended with its output not yet read), and still the run of its session
/// folder: the folder's marker names it, or, ended, the folder's `.done`
/// does and its result ref is still there. A preparation from another
/// conversation (which stops a run nobody is attached to and keeps its
/// result elsewhere), or a run since, makes it gone. Blocking (one ssh call).
pub(crate) fn run_there(target: &RemoteTarget, run: &str) -> Result<bool, String> {
    if !(run.len() <= 32 && run.bytes().all(|b| b.is_ascii_hexdigit())) {
        return Ok(false);
    }
    // The result ref the run writes (a worktree's, else its folder's): a
    // preparation that kept an ended run's result moved it away.
    let (repo, result) = if target.git_worktree {
        (target.repo_id.clone(), target.result_refs().1)
    } else if let Some(f) = &target.folder {
        (f.repo_id.clone(), f.result_ref())
    } else {
        return Ok(false);
    };
    let out = ssh_call(
        &target.ssh,
        &target.host,
        &format!(
            "r={run}; d={dir}; g=\"$HOME/hollow-remote/repos/\"{repo}.git; \
             test -f \"$HOME/hollow-remote/run/$r/out\" || {{ echo gone; exit 0; }}; \
             if test -f \"$d.running\"; then m=$(sed -n 2p \"$d.running\"); \
             elif git -C \"$g\" rev-parse -q --verify {result} >/dev/null 2>&1; then m=$(sed -n 1p \"$d.done\" 2>/dev/null); \
             else m=; fi; \
             test \"$m\" = \"$r\" && echo there || echo gone",
            dir = shell_quote(&target.dir),
            repo = shell_quote(&repo),
            result = shell_quote(&result),
        ),
        b"",
    )?;
    Ok(out.trim() == "there")
}

/// Counts the stdout bytes Hollow has handled in a remote attempt, kept in
/// `<turn>.out` after every line, so a restarted Hollow attaches from
/// exactly the next one (overnight R1). Lines are counted as the leader
/// passed them: a line's bytes and its newline (a line too long to keep
/// counts its own length).
pub(crate) struct Progress {
    file: std::fs::File,
    bytes: u64,
    /// A line that isn't a whole JSON event (the agents' streams are one per
    /// line): counted only once another line follows it, which proves it
    /// had its newline. One cut off by a quit is so never counted.
    held: u64,
}

impl Progress {
    /// From `base` (a reattached run's bytes already handled, else 0).
    pub(crate) fn start(turn: &str, base: u64) -> Option<Progress> {
        Self::start_in(&turns_dir().ok()?, turn, base)
    }

    pub(crate) fn start_in(dir: &Path, turn: &str, base: u64) -> Option<Progress> {
        use std::os::unix::fs::OpenOptionsExt;
        if turn.len() != 8 {
            return None;
        }
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(out_path(dir, turn))
            .ok()?;
        let mut progress = Progress {
            file,
            bytes: base,
            held: 0,
        };
        progress.write();
        Some(progress)
    }

    pub(crate) fn add(&mut self, line: &str) {
        self.bytes += std::mem::take(&mut self.held);
        let whole = serde_json::from_str::<serde::de::IgnoredAny>(line).is_ok();
        let len = crate::process_group::dropped_line_bytes(line).unwrap_or(line.len() as u64) + 1;
        if whole {
            self.bytes += len;
        } else {
            self.held = len;
        }
        self.write();
    }

    fn write(&mut self) {
        use std::os::unix::fs::FileExt;
        // Fixed width, written in place: one small write a line.
        let _ = self
            .file
            .write_at(format!("{:020}\n", self.bytes).as_bytes(), 0);
    }
}

/// What a remote turn's agent is told, after Hollow's appended prompt.
pub(crate) fn agent_note(target: &RemoteTarget) -> String {
    if !target.git_worktree {
        return format!(
            "\n\nThis turn runs on the runner ({host}) in its own copy of this workspace's folder ({dir}); paths under {local} on this Mac are at {dir} there, so use paths relative to your working folder. Your changes to these files come back to this Mac when the turn ends: anything changed there meanwhile is kept, and a file changed on both sides is merged or kept twice (a conflicted copy beside it). Hollow's own tools (hollow_vault, hollow_tasks, hollow_app) act on this Mac's vault directly, so don't change one file both with your own tools and through them in this turn. Files over 50 MB stay on this Mac.",
            host = target.host,
            dir = target.dir,
            local = target.worktree.display()
        ) + &crate::runner_access::agent_note(&target.access);
    }
    let folder = target
        .folder
        .as_ref()
        .map(|f| {
            format!(
                " This workspace's vault folder ({local} on this Mac: brief, plan, outputs) is at {dir} there, and your changes to it come back the same way when the turn ends.",
                local = f.local.display(),
                dir = f.dir
            )
        })
        .unwrap_or_default();
    format!(
        "\n\nThis turn runs on the runner ({host}) in its own copy of this worktree ({dir}), on the same branch; paths under {local} on this Mac are at {dir} there (an attached file's too), so use paths relative to your working folder. An attached file there is a copy to read: nothing in .hollow-attachments comes back, so write what you make outside that folder. Commit there with git. Hollow's build tools (build.build_commit, build.publish, build.status) act on this Mac's copy, which gets your work only when this turn ends, so don't use them in this turn; a teammate you ask also works on this Mac's copy. `hollow-slot` runs in the runner's own build slots.{folder}{access}",
        host = target.host,
        dir = target.dir,
        local = target.worktree.display(),
        access = crate::runner_access::agent_note(&target.access)
    )
}

/// The chat notice for how a turn's work came back (`remoteRun`, phase
/// `back`).
pub(crate) fn back_notice(host: &str, collected: &Result<Collected, String>) -> Value {
    match collected {
        Ok(Collected::Unchanged) => {
            json!({ "type": "remoteRun", "phase": "back", "host": host, "outcome": "unchanged" })
        }
        Ok(Collected::Applied { head }) => {
            json!({ "type": "remoteRun", "phase": "back", "host": host, "outcome": "applied", "head": head })
        }
        Ok(Collected::KeptAside { branch }) => json!({
            "type": "remoteRun", "phase": "back", "host": host, "outcome": "keptAside", "branch": branch,
            "message": format!("This Mac's worktree changed while the turn ran on {host}, so its work wasn't applied here. It is on branch {branch}."),
        }),
        Err(message) => json!({
            "type": "remoteRun", "phase": "back", "host": host, "outcome": "failed", "message": message,
        }),
    }
}

/// How a turn's work came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Collected {
    /// The host changed nothing.
    Unchanged,
    /// Applied to the local worktree: its HEAD (moved when the agent
    /// committed) and whether files changed.
    Applied { head: String },
    /// The local worktree changed while the turn ran, so the host's work is
    /// on `branch` and the worktree is untouched.
    KeptAside { branch: String },
}

/// Bring the turn's work back from the host and apply it to the local
/// worktree. Blocking (git over ssh).
#[cfg(test)]
pub(crate) fn collect(target: &RemoteTarget) -> Result<Collected, String> {
    collect_placed(target).0
}

/// [`collect`], and whether the work is still on the runner: false once it
/// is here (applied, unchanged, or on a branch here even when applying
/// failed), so a failure notice can say where it is (overnight 11-02: the
/// morning report names only work still there).
pub(crate) fn collect_placed(target: &RemoteTarget) -> (Result<Collected, String>, bool) {
    if !target.git_worktree {
        return (Ok(Collected::Unchanged), false);
    }
    let (collected, fetched) = collect_inner(target);
    let still_there = fetched.is_none() && collected.is_err();
    // Safe here (applied, unchanged or on a branch here, even when applying
    // failed): the runner drops exactly the result refs that were fetched
    // (compare and swap), so the next preparation doesn't keep them as work
    // that never came back, and a result written later is never dropped
    // unseen. Anything else leaves them there for the next preparation.
    if let Some((head, result)) = fetched {
        let (head_ref, result_ref) = target.result_refs();
        let _ = ssh_call(
            &target.ssh,
            &target.host,
            &format!(
                "r=\"$HOME/hollow-remote/repos/{}.git\"; git -C \"$r\" update-ref -d {} {head}; git -C \"$r\" update-ref -d {} {result}",
                target.repo_id,
                shell_quote(&head_ref),
                shell_quote(&result_ref),
            ),
            b"",
        );
    }
    (collected, still_there)
}

/// The outcome, and the result refs' commits when they were fetched.
fn collect_inner(target: &RemoteTarget) -> (Result<Collected, String>, Option<(String, String)>) {
    let mut fetched = None;
    let outcome = collect_fetched(target, &mut fetched);
    (outcome, fetched)
}

fn collect_fetched(
    target: &RemoteTarget,
    fetched: &mut Option<(String, String)>,
) -> Result<Collected, String> {
    let top = &target.worktree;
    let (head_ref, result_ref) = target.result_refs();
    let hash = target.wt_id.rsplit('-').next().unwrap_or(&target.wt_id);
    let local_head_ref = format!("refs/hollow-remote/{hash}/head");
    let local_result_ref = format!("refs/hollow-remote/{hash}/result");
    let failed = |e: String| {
        format!(
            "The work couldn't be brought back from {} ({e})",
            target.host
        )
    };
    let still_there = |e: String| {
        format!(
            "The work couldn't be kept on a branch here ({e}); it is still on {}, and comes back as a branch at the next turn there.",
            target.host
        )
    };
    wait_for_run(target).map_err(failed)??;
    let ssh_cmd = git_ssh_command(&target.ssh);
    let env: Vec<(&str, &OsStr)> = vec![
        ("GIT_SSH_COMMAND", OsStr::new(&ssh_cmd)),
        ("GIT_SSH_VARIANT", OsStr::new("ssh")),
    ];
    let src = format!("{}:hollow-remote/repos/{}.git", target.host, target.repo_id);
    // No result: the agent never started there (the turn failed first).
    let listed = git_env(top, &["ls-remote", &src, &result_ref], &env).map_err(failed)?;
    if listed.trim().is_empty() {
        return Ok(Collected::Unchanged);
    }
    git_env(
        top,
        &[
            "fetch",
            "-q",
            "--no-tags",
            "--no-write-fetch-head",
            &src,
            &format!("+{head_ref}:{local_head_ref}"),
            &format!("+{result_ref}:{local_result_ref}"),
        ],
        &env,
    )
    .map_err(failed)?;
    let new_head = git(top, &["rev-parse", &local_head_ref])?;
    let result = git(top, &["rev-parse", &local_result_ref])?;
    // The runner drops its result refs only once the work is safe here
    // (applied, unchanged, or on a branch): set on exactly those paths. Any
    // other failure leaves them there, and the next preparation keeps them.
    let pair = (new_head.clone(), result.clone());
    let result_tree = git(top, &["rev-parse", &format!("{result}^{{tree}}")])?;
    if new_head == target.head && result_tree == target.snapshot_tree {
        *fetched = Some(pair);
        return Ok(Collected::Unchanged);
    }
    // Only a fast-forward of the branch the turn started on is applied: an
    // agent that checked out another branch there, or rewrote this one,
    // gets its work kept on a branch of its own.
    if new_head != target.head
        && git(
            top,
            &["merge-base", "--is-ancestor", &target.head, &new_head],
        )
        .is_err()
    {
        let kept = keep_aside(top, &target.branch, &result).map_err(still_there)?;
        *fetched = Some(pair);
        return Ok(kept);
    }
    // Untouched here since the turn started: HEAD where it was and the files
    // exactly the snapshot that was sent (compared by tree: the snapshot
    // commit itself may exist only on the host). The chat's attachments are
    // in neither tree (`snapshot`): one saved during the turn isn't a change
    // here, and the switch below leaves them all where they are.
    let scratch = TempDir::new("hollow-collect-")?;
    let common = git(
        top,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let now_head = git(top, &["rev-parse", "HEAD"])?;
    let (now_snap, _, now_tree) = snapshot(top, &common, &now_head, scratch.path(), None)?;
    if now_head != target.head || now_tree != target.snapshot_tree {
        let kept = keep_aside(top, &target.branch, &result).map_err(still_there)?;
        *fetched = Some(pair);
        return Ok(kept);
    }
    // Apply. First the files, with an index of the switch's own (this
    // checkout's index is never left pointing at the private objects): it
    // becomes the snapshot of what is here now, then a two-tree switch to the
    // result updates the working tree (a file only the snapshot had, an
    // untracked one included, goes). `read-tree` leaves no stat data; the
    // refresh records it, or the switch would take every file for an edit.
    let objects = scratch.path().join("objects");
    let alternates = Path::new(&common).join("objects");
    let index = scratch.path().join("switch-index");
    let mut env: Vec<(&str, &OsStr)> = vec![("GIT_INDEX_FILE", index.as_os_str())];
    if objects.exists() {
        env.push(("GIT_OBJECT_DIRECTORY", objects.as_os_str()));
        env.push(("GIT_ALTERNATE_OBJECT_DIRECTORIES", alternates.as_os_str()));
    }
    let switched = git_env(top, &["read-tree", &now_snap], &env)
        .and_then(|_| {
            git_env(top, &["update-index", "-q", "--refresh"], &env).or(Ok(String::new()))
        })
        .and_then(|_| git_env(top, &["read-tree", "-u", "-m", &now_snap, &result], &env));
    if let Err(error) = switched {
        let kept = keep_aside(top, &target.branch, &result).map_err(still_there)?;
        *fetched = Some(pair);
        return Err(format!(
            "The runner's work came back but couldn't be applied here ({error}); it is on {}.",
            match kept {
                Collected::KeptAside { branch } => branch,
                _ => local_result_ref,
            }
        ));
    }
    // Then the branch (or a detached HEAD), only from where it was, and the
    // index HEAD's: every change the agent didn't commit shows as not staged.
    if new_head != target.head {
        let moved = match &target.branch {
            Some(branch) => git(
                top,
                &[
                    "update-ref",
                    &format!("refs/heads/{branch}"),
                    &new_head,
                    &target.head,
                ],
            ),
            None => git(
                top,
                &["update-ref", "--no-deref", "HEAD", &new_head, &target.head],
            ),
        };
        if let Err(error) = moved {
            let _ = git(top, &["reset", "-q", "--mixed"]);
            // The commits get a branch of their own before the runner drops
            // its result refs; if even that fails, the runner keeps them and
            // the next turn brings them as a branch.
            let at = match keep_aside(top, &target.branch, &new_head) {
                Ok(Collected::KeptAside { branch }) => {
                    *fetched = Some(pair);
                    format!("on branch {branch}")
                }
                _ => format!(
                    "still on {}, and come back as a branch at the next turn there",
                    target.host
                ),
            };
            return Err(format!(
                "The runner's files are here, but its commits couldn't be moved onto this branch ({error}); they are {at}."
            ));
        }
    }
    // Applied: the work is here whatever the index refresh says.
    *fetched = Some(pair);
    git(top, &["reset", "-q", "--mixed"])?;
    Ok(Collected::Applied { head: new_head })
}

/// The turn is over here, however it ended: a run of its own still going
/// there (Phase 4: its supervisor outlives the connection, so Stop, a
/// deadline or a leader that gave up reach it only this way) is stopped
/// (TERM: it ends the agent, KILL after 10 s, then writes the result), and
/// so is one still waiting to start (the turn's stop file); whatever run
/// holds the folder is waited for, 30 s at most; then the turn's finished
/// runs' folders there (their output) go. Outer Err: ssh failed; inner Err:
/// still running there.
fn wait_for_run(target: &RemoteTarget) -> Result<Result<(), String>, String> {
    let marker = shell_quote(&format!("{}.running", target.dir));
    let own = target.turn.len() == 8 && target.turn.bytes().all(|b| b.is_ascii_hexdigit());
    let (stop, tidy) = if own {
        (
            format!(
                "r=\"$HOME/hollow-remote/run\"; : > \"$r/{turn}.stop\" 2>/dev/null; p=$(sed -n 1p {marker} 2>/dev/null); case \"$(sed -n 2p {marker} 2>/dev/null)\" in {turn}*) [ -n \"$p\" ] && {{ ps -o command= -p \"$p\" || tr '\\0' ' ' < \"/proc/$p/cmdline\"; }} 2>/dev/null | grep -q hollow-remote-run && kill -TERM \"$p\";; esac; ",
                turn = target.turn
            ),
            // `find`, not a glob: zsh (the runner's login shell) refuses a
            // pattern that matches nothing.
            format!(
                "find \"$r\" -maxdepth 1 -type d -name '{turn}????' -exec /bin/sh -c 'for d do [ -e \"$d/exit\" ] && rm -rf \"$d\"; done; true' x {{}} + 2>/dev/null; ",
                turn = target.turn
            ),
        )
    } else {
        (String::new(), String::new())
    };
    let waited = ssh_call(
        &target.ssh,
        &target.host,
        &format!(
            "{stop}i=0; while [ -e {marker} ] && [ $i -lt 60 ]; do p=$(sed -n 1p {marker} 2>/dev/null); kill -0 \"$p\" 2>/dev/null || break; sleep 0.5; i=$((i+1)); done; [ -e {marker} ] && kill -0 \"$(sed -n 1p {marker} 2>/dev/null)\" 2>/dev/null && echo live; {tidy}true"
        ),
        b"",
    )?;
    Ok(if waited.trim() == "live" {
        Err(format!(
            "The turn is still finishing on {}; its work will come back at the start of the next turn there.",
            target.host
        ))
    } else {
        Ok(())
    })
}

/// Bring a turn's workspace folder changes back (Phase 3), three ways
/// against what is here now (`remote_folder::apply`); None when no folder
/// went with the turn. The runner drops its result ref only once applied;
/// otherwise the next preparation keeps it and applies it then. Blocking.
pub(crate) fn collect_folder(
    target: &RemoteTarget,
) -> Option<Result<crate::remote_folder::FolderBack, String>> {
    let sync = target.folder.as_ref()?;
    Some(collect_folder_inner(target, sync).map_err(|e| {
        format!(
            "The workspace folder's changes couldn't be brought back from {} ({e}); they come back at the next turn there.",
            target.host
        )
    }))
}

fn collect_folder_inner(
    target: &RemoteTarget,
    sync: &crate::remote_folder::FolderSync,
) -> Result<crate::remote_folder::FolderBack, String> {
    wait_for_run(target)??;
    let ssh_cmd = git_ssh_command(&target.ssh);
    let env: [(&str, &OsStr); 3] = [
        ("GIT_DIR", sync.git_dir.as_os_str()),
        ("GIT_SSH_COMMAND", OsStr::new(&ssh_cmd)),
        ("GIT_SSH_VARIANT", OsStr::new("ssh")),
    ];
    let dir_env = [("GIT_DIR", sync.git_dir.as_os_str())];
    let src = format!("{}:hollow-remote/repos/{}.git", target.host, sync.repo_id);
    let remote_ref = sync.result_ref();
    let listed = git_env(&sync.git_dir, &["ls-remote", &src, &remote_ref], &env)?;
    if listed.trim().is_empty() {
        // The agent never started there.
        return Ok(crate::remote_folder::FolderBack::default());
    }
    let local = format!("refs/hollow-folder/{}/result", sync.hash());
    git_env(
        &sync.git_dir,
        &[
            "fetch",
            "-q",
            "--no-tags",
            "--no-write-fetch-head",
            &src,
            &format!("+{remote_ref}:{local}"),
        ],
        &env,
    )?;
    let result = git_env(&sync.git_dir, &["rev-parse", &local], &dir_env)?;
    let back = if result == sync.snapshot {
        crate::remote_folder::FolderBack::default()
    } else {
        // Applied against what was sent, never a base the runner names.
        let base = git_env(
            &sync.git_dir,
            &["rev-parse", &format!("{result}^1")],
            &dir_env,
        )?;
        if base != sync.snapshot {
            return Err("the runner's result isn't on top of what was sent".into());
        }
        crate::remote_folder::apply(&sync.git_dir, &sync.local, &sync.snapshot, &result)?
    };
    let _ = ssh_call(
        &target.ssh,
        &target.host,
        &format!(
            "git -C \"$HOME/hollow-remote/repos/{}.git\" update-ref -d {} {result}",
            sync.repo_id,
            shell_quote(&remote_ref)
        ),
        b"",
    );
    Ok(back)
}

/// The chat notice for how a workspace folder's changes came back
/// (`remoteRun`, phase `back`, scope `folder`).
pub(crate) fn folder_notice(
    host: &str,
    back: &Result<crate::remote_folder::FolderBack, String>,
) -> Value {
    match back {
        Ok(back) if back.is_empty() => json!({
            "type": "remoteRun", "phase": "back", "scope": "folder", "host": host, "outcome": "unchanged",
        }),
        Ok(back) => json!({
            "type": "remoteRun", "phase": "back", "scope": "folder", "host": host, "outcome": "applied",
            "changed": back.changed.len(), "merged": back.merged, "copies": back.copies,
            "keptHere": back.kept_here, "failed": back.failed,
            "notBrought": back.not_brought_json(),
        }),
        Err(message) => json!({
            "type": "remoteRun", "phase": "back", "scope": "folder", "host": host, "outcome": "failed", "message": message,
        }),
    }
}

/// Keep the host's result on a branch of its own and leave the worktree as
/// it is. A `<branch>-mini-<n>` already at exactly that commit is reused (a
/// collect retried after the runner's refs weren't dropped); a name that
/// can't be taken (a ref under it) is passed over. When none can, the
/// message carries git's own last error (overnight: B2 r3 minor).
fn keep_aside(top: &Path, branch: &Option<String>, result: &str) -> Result<Collected, String> {
    let base = branch.clone().unwrap_or_else(|| "detached".into());
    let result = git(
        top,
        &[
            "rev-parse",
            "--verify",
            "-q",
            &format!("{result}^{{commit}}"),
        ],
    )
    .unwrap_or_else(|_| result.to_string());
    let mut last = None;
    for n in 1..100 {
        let name = format!("{base}-mini-{n}");
        match git(
            top,
            &["rev-parse", "--verify", "-q", &format!("refs/heads/{name}")],
        ) {
            Ok(at) if at == result => return Ok(Collected::KeptAside { branch: name }),
            Ok(_) => continue,
            Err(_) => match git(top, &["branch", &name, &result]) {
                Ok(_) => return Ok(Collected::KeptAside { branch: name }),
                Err(e) => last = Some(e),
            },
        }
    }
    Err(no_branch_message(last.as_deref()))
}

/// [`keep_aside`]'s failure: git's last error (on one line, cut to 300
/// characters) when there was one, so a locked or broken repository says
/// so. No full stop: callers wrap it in parentheses.
fn no_branch_message(git_error: Option<&str>) -> String {
    let one_line = git_error
        .map(|e| e.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|e| !e.is_empty());
    match one_line {
        Some(e) => {
            let cut: String = e.chars().take(300).collect();
            let more = if cut.len() < e.len() { "…" } else { "" };
            format!("No branch could be made for the runner's work ({cut}{more})")
        }
        None => "No free branch name for the runner's work".into(),
    }
}

/// Run on the host by ssh before the push: `/usr/bin/perl -e` this
/// `<repo id>`, with a JSON object of files on stdin. Installs each file in
/// `~/hollow-remote/bin` (moved into place, so a run reading the old one
/// keeps it), runs the host's `hollow-run-preflight` (exit 4 with its last
/// line when it says no), makes the bare repository and the private relay
/// socket folder, drops what runs over a day old left (their sockets and
/// `run/<run>/` folders, never a live run's), prunes remote sessions'
/// folders by age and free space (overnight N5; never the session ids after
/// `--`, nor a live run's) and prints `home=`.
pub(crate) const HOST_PREP_PL: &str = r#"
use strict; use warnings; use JSON::PP;
my (@repo_ids, @keep); my $after = 0;
for my $a (@ARGV) { if ($a eq '--') { $after = 1 } elsif ($after) { push @keep, $a } else { push @repo_ids, $a } }
@repo_ids or do { print STDERR "bad repository id\n"; exit 2 };
for my $repo_id (@repo_ids) { $repo_id =~ /\A[A-Za-z0-9._-]+\z/ && $repo_id !~ /\A\.\.?\z/ or do { print STDERR "bad repository id\n"; exit 2 } }
my $home = $ENV{HOME}; my $root = "$home/hollow-remote";
# A process's command line: /proc on a Linux runner (2026-11-03, where procps
# may be missing), ps on macOS.
my $cmd_of = sub { my $p = shift; return `ps -o command= -p $p` unless $^O eq 'linux'; open(my $f, '<', "/proc/$p/cmdline") or return ''; local $/; my $c = <$f> // ''; close $f; $c =~ tr/\0/ /; return $c };
for my $d ("$root", "$root/bin", "$root/repos", "$root/sessions", "$root/folders", "$root/scratch", "$root/targets", "$root/run") {
  -d $d or mkdir($d, 0700) or do { print STDERR "can't make $d ($!)\n"; exit 3 };
}
chmod 0700, "$root/run";
# Raw bytes: written back exactly, and compared with the files as bytes.
my $files = do { local $/; JSON::PP->new->decode(<STDIN> // '{}') };
for my $name (sort keys %$files) {
  $name =~ /\A[A-Za-z0-9._-]+\z/ or next;
  my $dest = "$root/bin/$name"; my $tmp = "$dest.$$";
  my $old = ''; if (open(my $fh, '<', $dest)) { $old = do { local $/; <$fh> }; close $fh }
  next if $old eq $files->{$name};
  open(my $out, '>', $tmp) or do { print STDERR "can't write $tmp ($!)\n"; exit 3 };
  print $out $files->{$name}; close $out or exit 3;
  chmod 0755, $tmp; rename($tmp, $dest) or exit 3;
}
# Remote sessions' own folders (overnight N5), before the preflight, so a
# disk under its floor can recover: a session (sessions/<id> or
# folders/<id>, with scratch/<id>, targets/session-<id> and its small files
# in sessions/) not prepared for HOLLOW_REMOTE_DAYS (14) days goes; then,
# while the volume has less than HOLLOW_REMOTE_MIN_FREE_GB (40) free, the
# least recently used go one at a time, deleted before free space is read
# again, and only those unused for 12 hours: a session has no lock between
# its preparation and its run, and a turn can run that long. Never one
# being prepared now (the ids after --, stamped first so another
# preparation sees them as in use) or one whose run is live (a marker whose
# pid is a hollow-remote-run). Its results are refs in the bare repository,
# so the next turn there checks out again. HOLLOW_REMOTE_DF stands in for
# /bin/df in tests.
my $num = sub { my ($v, $d) = @_; defined $v && $v =~ /\A\d+\z/ ? $v : $d };
my $days = $num->($ENV{HOLLOW_REMOTE_DAYS}, 14); my $floor = $num->($ENV{HOLLOW_REMOTE_MIN_FREE_GB}, 40);
my $sid = qr/\A[A-Za-z0-9_-][A-Za-z0-9._-]*\z/;
my $free_gb = sub {
  local $/ = "\n";
  my $df = $ENV{HOLLOW_REMOTE_DF}; $df = '/bin/df' unless defined $df && $df =~ m{\A/};
  my $pid = open(my $fh, '-|') // return undef;
  if (!$pid) { open(STDERR, '>', '/dev/null'); open(STDIN, '<', '/dev/null'); exec { $df } $df, '-Pk', $root or exit 127 }
  my @l = <$fh>; close $fh;
  my @f = split ' ', ($l[1] // '');
  return defined $f[3] && $f[3] =~ /\A\d+\z/ ? int($f[3] / 1048576) : undef;
};
my %keep = map { ($_ => 1) } grep { $_ =~ $sid } @keep;
for my $id (keys %keep) { if (open(my $t, '>', "$root/sessions/$id.prepared")) { close $t } }
my %seen;
for my $kind ('sessions', 'folders', 'scratch') {
  opendir(my $d, "$root/$kind") or next;
  for my $e (readdir $d) {
    if (-d "$root/$kind/$e") { $seen{$e} = 1 if $e =~ $sid }
    elsif ($kind eq 'sessions' && $e =~ /\A(.+)\.(?:port|prepared)\z/) { my $id = $1; $seen{$id} = 1 if $id =~ $sid }
  }
}
if (opendir(my $d, "$root/targets")) { for my $t (readdir $d) { if ($t =~ /\Asession-(.+)\z/) { my $id = $1; $seen{$id} = 1 if $id =~ $sid } } }
my $busy = sub {
  my $id = shift; local $/ = "\n";
  for my $m ("$root/sessions/$id.running", "$root/folders/$id.running") {
    open(my $f, '<', $m) or next; my $p = <$f> // ''; close $f; chomp $p;
    return 1 if $p =~ /\A\d+\z/ && kill(0, $p) && $cmd_of->($p) =~ /hollow-remote-run/;
  }
  return 0;
};
my $last_use = sub {
  my $id = shift; my $t = 0;
  for my $p (map { "$root/$_" } "scratch/$id", "folders/$id", "sessions/$id", "sessions/$id.prepared", "sessions/$id.running", "folders/$id.running") { my $m = (stat $p)[9]; $t = $m if defined $m && $m > $t }
  return $t;
};
my %used = map { ($_ => $last_use->($_)) } keys %seen;
# Moved aside first (a rename), then deleted: and only once its use and
# its run are asked again, since a preparation or run beside this one may
# have started on it after the list was made (a deletion takes a while).
my $dropped = 0; my $trash = "$root/trash-sessions.$$";
if (opendir(my $d, $root)) { for my $e (readdir $d) { system('rm', '-rf', "$root/$e") if $e =~ /\Atrash-sessions\.\d+\z/ && -M "$root/$e" > 1 / 48 } }
my $drop = sub {
  my ($id, $idle) = @_;
  return 0 unless defined $id && $id =~ $sid && !$keep{$id};
  return 0 if time - $last_use->($id) < $idle || $busy->($id);
  system('rm', '-rf', $trash); mkdir($trash, 0700) or return 0;
  my ($n, $moved) = (0, 0);
  for my $p ("sessions/$id", "folders/$id", "scratch/$id", "targets/session-$id") { next unless -e "$root/$p"; $moved++ if rename("$root/$p", "$trash/" . $n++) }
  for my $m ("sessions/$id.running", "folders/$id.running") { unlink "$root/$m" unless $busy->($id) }
  unlink map { "$root/$_" } "sessions/$id.port", "sessions/$id.prepared", "sessions/$id.sent", "folders/$id.sent", "sessions/$id.done", "folders/$id.done", "sessions/$id.keep-failed", "folders/$id.keep-failed";
  system('rm', '-rf', $trash); delete $used{$id}; $dropped++;
  return $moved > 0;
};
for my $id (sort keys %used) { $drop->($id, $days * 86400) if time - $used{$id} > $days * 86400 }
for my $id (sort { $used{$a} <=> $used{$b} || $a cmp $b } keys %used) {
  my $free = $free_gb->(); last unless defined $free && $free < $floor;
  print "pruned=$id $free $floor\n" if $drop->($id, 12 * 3600);
}
if ($dropped) { opendir(my $d, "$root/repos"); for my $r (readdir $d) { system('git', '-C', "$root/repos/$r", 'worktree', 'prune') if $r =~ /\.git\z/ && -d "$root/repos/$r/worktrees" } }
# Copies a preparation moved aside after an hour of failed keeps
# (`aside/`, overnight P3L-5): gone after the same number of days, or
# oldest first, once 12 hours old, while the volume is under its floor.
if (opendir(my $ad, "$root/aside")) {
  my %at = map { ($_ => (stat "$root/aside/$_")[9] // 0) } grep { /\A[A-Za-z0-9._-]+\z/ && !/\A\.\.?\z/ } readdir $ad;
  for my $e (sort { $at{$a} <=> $at{$b} || $a cmp $b } keys %at) {
    my $age = time - $at{$e};
    my $low = $age > 12 * 3600 && do { my $free = $free_gb->(); defined $free && $free < $floor };
    system('rm', '-rf', "$root/aside/$e") if $age > $days * 86400 || $low;
  }
}
my $pre = '';
for my $p (split(/:/, $ENV{PATH} // ''), "$home/.local/bin") { if (-x "$p/hollow-run-preflight") { $pre = "$p/hollow-run-preflight"; last } }
my $status = 'ready; no hollow-run-preflight there';
if ($pre) {
  my $out = `"$pre" 2>&1 </dev/null`; my @lines = split /\n/, $out;
  if ($? != 0 && -f "$root/bin/hollow-slot") {
    # Under its floor, what hollow-slot keeps here may be the room: its
    # prune once (never anything in use), then ask again.
    system('/bin/sh', '-c', '/bin/sh "$1" --remote-prune "" </dev/null >&2', 'x', "$root/bin/hollow-slot");
    $out = `"$pre" 2>&1 </dev/null`; @lines = split /\n/, $out;
  }
  if ($? != 0) { print STDERR (($lines[-1] // 'preflight said no') . "\n"); exit 4 }
  $status = $lines[-1] // $status;
}
for my $repo_id (@repo_ids) {
  my $repo = "$root/repos/$repo_id.git";
  if (!-f "$repo/HEAD") { system('git', 'init', '-q', '--bare', $repo) == 0 or exit 3 }
}
# Runs over a day old whose supervisor is gone: their folders (output,
# exit code) and their sockets. A live run keeps everything.
my $live = sub { my $r = shift; local $/ = "\n"; open(my $f, '<', "$root/run/$r/pid") or return 0; my $p = <$f> // ''; close $f; chomp $p; return $p =~ /\A\d+\z/ && kill(0, $p) && $cmd_of->($p) =~ /hollow-remote-run/ };
opendir(my $rd, "$root/run");
for my $s (readdir $rd) {
  my $p = "$root/run/$s";
  if ($s =~ /\A([0-9a-f]+)?.*\.sock\z/) { unlink $p if -M $p > 1 && !(defined $1 && $live->($1)); next }
  if ($s =~ /\.stop\z/) { unlink $p if -M $p > 1; next }
  next unless $s =~ /\A[0-9a-f]{12}\z/ && -d $p && !$live->($s);
  my $age = -e "$p/exit" ? -M "$p/exit" : -M $p;
  system('rm', '-rf', $p) if defined $age && $age > 1;
}
print "home=$home\nstatus=$status\n";
"#;

/// The host side of a remote session, installed as
/// `~/hollow-remote/bin/hollow-remote-run`:
/// - `--prepare <repo id> <worktree id> <snapshot> <head> <branch|-> <clean>`
///   checks the pushed snapshot out in `sessions/<worktree id>` (a git
///   worktree of the bare repository, made again when broken), on the
///   branch, with a dirty tree's changes left uncommitted; a fresh scratch
///   folder, a port no other session folder holds (2700-2999), a build
///   folder (a clone of the same worktree's `hollow-slot` one when there is
///   one), `npm ci` when `package-lock.json` changed; prints the paths as one
///   JSON line.
/// - `--run` reads the header line from stdin, starts the run's detached
///   supervisor (the agent, its tool socket, the result refs when it ends),
///   then attaches: streams the run's output back and passes input on.
/// - `--attach <run> <stdout bytes> <err offset> <relay socket> <supervisor
///   pid> [1]` (Phase 4): the leader reconnecting after a dropped
///   connection (`1`: it reads stderr lines as marks, overnight R2).
/// - `--bridge <name>` connects one of the agent's tools to the relay, and
///   reconnects it after a drop.
///
/// It also runs on a Linux runner (2026-11-03): the peer check takes
/// `SO_PEERCRED` there instead of `LOCAL_PEERPID`, process facts come from
/// `/proc` (`cmd_of`, `proc_table`, `tagged`), the supervisor and its
/// attachers make themselves non-dumpable (`no_dump`), and a build folder's
/// clone is `cp --reflink=always` or none.
pub(crate) const REMOTE_RUN_PL: &str = r#"#!/usr/bin/perl
# hollow-remote-run: the Mac mini side of a Hollow remote session (Hollow,
# remote_session.rs). Sent by Hollow before every turn; never edit by hand.
# Linux (review of the Linux runner): any process of this account could read
# a run's relay token and attach secret out of this one's memory
# (/proc/<pid>/mem, ptrace) or reopen its pipes through /proc/<pid>/fd, which
# macOS doesn't allow. Non-dumpable (prctl PR_SET_DUMPABLE 0) it can't: those
# become root's alone, while cmdline and stat stay readable. Hollow's leader
# has written the header and secret into stdin before this starts, so
# --run and --attach do it in the BEGIN block below, before perl compiles
# the rest or loads a module; a descriptor opened before that keeps working.
# The supervisor forks from --run and inherits it; exec resets it, so the
# agent is an ordinary process of the account, as on macOS. prctl_nr: the
# SYS_prctl of this perl's own build (Config's archname, so a 32-bit perl on
# a 64-bit kernel never makes the wrong call), undef for one not known here,
# which skips it. A call that doesn't take says so on stderr.
sub prctl_nr { my $arch = shift // ''; return $arch =~ /\Ax86_64-/ ? 157 : $arch =~ /\A(?:aarch64|arm64)-/ ? 167 : undef }
sub no_dump { return unless $^O eq 'linux'; require Config; my $nr = prctl_nr($Config::Config{archname}); return unless defined $nr; syscall($nr, 4, 0, 0, 0, 0); print STDERR "hollow-remote: couldn't make this process non-dumpable (prctl), so other processes of this account can read its memory\n" if syscall($nr, 3, 0, 0, 0, 0) != 0; return }
BEGIN { no_dump() if ($ARGV[0] // '') =~ /\A--(?:run|attach)\z/ }
use strict; use warnings;
use IO::Socket::UNIX; use IO::Select; use POSIX qw(:sys_wait_h); use JSON::PP;
my $mode = shift // '';
my $home = $ENV{HOME}; my $root = "$home/hollow-remote";
# The last thing said, and a copy moved aside this run (`keep_or_give_up`):
# a preparation that fails after moving one still names it, on the last
# line, the one the Mac shows (overnight P3L-5).
my ($LAST_SAID, $ASIDE) = ('', '');
sub say_err { $LAST_SAID = "@_"; print STDERR "hollow-remote: @_\n" }
END { print STDERR "hollow-remote: $LAST_SAID (what a run that stopped here left, which couldn't be kept, is at $ASIDE)\n" if $? == 70 && $ASIDE ne '' }
sub name_ok { my $v = shift // ''; return $v =~ /\A[A-Za-z0-9._-]+\z/ && $v !~ /\A\.\.?\z/ }
sub quiet { my @cmd = @_; my $pid = fork(); if (!$pid) { open(STDOUT, '>', '/dev/null'); open(STDERR, '>', '/dev/null'); open(STDIN, '<', '/dev/null'); exec { $cmd[0] } @cmd or exit 127 } waitpid($pid, 0); return $? == 0 }
sub capture { my @cmd = @_; my $pid = open(my $fh, '-|'); return '' unless defined $pid; if (!$pid) { open(STDERR, '>', '/dev/null'); open(STDIN, '<', '/dev/null'); exec { $cmd[0] } @cmd or exit 127 } local $/; my $o = <$fh> // ''; close $fh; $o =~ s/\s+\z//; return $o }
# A Linux runner (2026-11-03): what macOS's ps and LOCAL_PEERPID say comes
# from /proc and SO_PEERCRED there (procps may be missing, and its ps has no
# -E). A file's whole text, or '' when it can't be read.
sub slurp { my $f = shift; open(my $h, '<', $f) or return ''; local $/; my $t = <$h> // ''; close $h; return $t }
# This machine's short name: hostname -s, else its node name with no domain.
sub host_name { my $h = capture("hostname", "-s"); if ($h eq '') { $h = (POSIX::uname())[1] // ''; $h =~ s/\..*//s } return $h }
# A process's command line.
sub cmd_of {
  my $p = shift;
  return capture('ps', '-o', 'command=', '-p', $p) unless $^O eq 'linux';
  (my $c = slurp("/proc/$p/cmdline")) =~ tr/\0/ /; $c =~ s/\s+\z//; return $c;
}
# Every process, a line each: "<pid> <parent pid> <process group>".
sub proc_table {
  return capture('ps', '-A', '-o', 'pid=,ppid=,pgid=') unless $^O eq 'linux';
  my $t = ''; opendir(my $d, '/proc') or return '';
  # /proc/<pid>/stat is "pid (name) state ppid pgrp ...", a name that may
  # hold spaces and parentheses: the last ) ends it.
  for my $p (grep { /\A\d+\z/ } readdir $d) { $t .= "$p $1 $2\n" if slurp("/proc/$p/stat") =~ /.*\)\s+\S+\s+(\d+)\s+(\d+)\s/s }
  closedir $d; return $t;
}
# Processes other than this one whose environment holds NAME=VALUE ($_[0]),
# as pid => process group: ps -E shows a non-Apple program's on macOS,
# /proc/<pid>/environ this account's on Linux.
sub tagged {
  my $kv = shift; my %found;
  if ($^O eq 'linux') {
    for (split /\n/, proc_table()) { my ($p, undef, $g) = split ' '; $found{$p} = $g if $p != $$ && index("\0" . slurp("/proc/$p/environ"), "\0$kv\0") >= 0 }
    return %found;
  }
  for (split /\n/, capture('ps', '-A', '-E', '-ww', '-o', 'pid=,pgid=,command=')) {
    my ($p, $g) = /\A\s*(\d+)\s+(\d+)\s/ or next;
    $found{$p} = $g if index($_, $kv) >= 0 && $p != $$;
  }
  return %found;
}
# The pid at the other end of Unix socket $s, as the kernel says: macOS's
# LOCAL_PEERPID (level SOL_LOCAL 0, option 0x002), Linux's SO_PEERCRED
# (struct ucred: pid, uid, gid). -1 when it can't say.
sub peer_pid {
  my $s = shift; return -1 unless $s;
  if ($^O eq 'linux') {
    my $raw = getsockopt($s, Socket::SOL_SOCKET(), eval { Socket::SO_PEERCRED() } // 17);
    return defined $raw && length($raw) >= 12 ? (unpack('i I I', $raw))[0] : -1;
  }
  my $raw = getsockopt($s, 0, 0x002);
  return defined $raw ? unpack('i', $raw) : -1;
}
# Whether the given folder's file system clones files (a reflink cp of a one-byte
# file; the name is one the prune of stray .seed-* files takes).
sub can_reflink { my $d = shift; my $p = "$d/.seed-reflink.$$"; open(my $o, '>', $p) or return 0; print $o "x"; close $o; my $ok = quiet('/bin/cp', '--reflink=always', $p, "$p.c"); unlink $p, "$p.c"; return $ok }

# A folder's marker (`<folder>.running`) names the run's supervisor (line 1)
# and the run (line 2); it is touched whenever that run's leader is heard.
# (pid, run) while that supervisor is alive.
sub marker_live {
  my $mark = shift;
  open(my $m, '<', $mark) or return;
  my $p = <$m> // ''; my $r = <$m> // ''; close $m; chomp($p, $r);
  return unless $p =~ /\A\d+\z/ && $p != $$ && kill(0, $p) && cmd_of($p) =~ /hollow-remote-run/;
  return ($p, $r);
}
# Its leader is gone (Hollow quit, or stopped it while this host couldn't be
# reached): no attacher is connected, or none was heard for 45 s (its
# heartbeat comes every 15 s while connected).
sub unattended {
  my ($mark, $run) = @_;
  return 0 unless name_ok($run);
  return 1 unless -e "$root/run/$run/attached";
  return time - ((stat $mark)[9] // 0) > 45;
}
# Wait, up to 30 s, for a run still in a folder to end; one whose leader is
# gone is stopped first (TERM: it ends its agent and writes its result, which
# the next preparation keeps). True when the folder is free.
sub wait_free {
  my $mark = shift; my $termed = 0;
  for (1 .. 60) {
    my ($p, $r) = marker_live($mark); return 1 unless $p;
    if (!$termed && unattended($mark, $r)) { kill('TERM', $p); $termed = 1 }
    select(undef, undef, undef, 0.5);
  }
  my ($p) = marker_live($mark);
  return !$p;
}
# A turn still in its folder here keeps it: wait before touching anything.
# True when its marker was left with no supervisor: that run died before its
# finish (which writes the result, then removes the marker), for example when
# this host restarted mid-turn.
sub wait_turn {
  my $mark = shift;
  wait_free($mark) or do { say_err("the previous turn is still finishing on this host"); exit 75 };
  # No supervisor holds it now: a marker still there was left by one that
  # died. The caller removes it once what that run left is kept.
  return -e $mark;
}
# The snapshot a folder was last prepared from (`<folder>.sent`), so a run
# that died there can be kept on top of it.
sub sent_of {
  my $dir = shift;
  open(my $s, '<', "$dir.sent") or return '';
  my $v = <$s> // ''; close $s; chomp $v;
  return $v =~ /\A[0-9a-f]{40,64}\z/ ? $v : '';
}
sub mark_sent {
  my ($dir, $snap) = @_;
  if (open(my $s, '>', "$dir.sent.$$")) { print $s "$snap\n"; close $s; rename("$dir.sent.$$", "$dir.sent") }
}
# The run whose finish wrote its result in a folder (`<folder>.done`): a
# marker naming that run, left by a supervisor killed just after, has
# nothing more to keep. Another attempt's run in the same copy still has.
sub mark_done {
  my ($dir, $run) = @_;
  if (open(my $d, '>', "$dir.done.$$")) { print $d "$run\n"; close $d; rename("$dir.done.$$", "$dir.done") }
}
sub line_of {
  my ($path, $n) = @_; local $/ = "\n";
  open(my $f, '<', $path) or return '';
  my $v = ''; for (1 .. $n) { $v = <$f> // '' } close $f; chomp $v;
  return $v;
}
# Whether the run a dead marker names already wrote its result.
sub run_done { my ($dir, $mark) = @_; my $run = line_of($mark, 2); return $run ne '' && name_ok($run) && line_of("$dir.done", 1) eq $run }
# A dead run's copy that couldn't be kept is tried again at the next
# preparation (exit 70) for an hour from the first failed keep
# (`<folder>.keep-failed`, overnight P3L-5: the marker's own time can be
# hours old when the next turn comes, which gave up at once); after that
# the copy is moved aside to `aside/<folder>-<time>-<pid>` (pruned after
# HOLLOW_REMOTE_DAYS, sooner under the disk floor) and the folder starts
# fresh, so one that can never be kept doesn't stop the folder for good and
# nothing is deleted. The aside path, or '' (kept, or nothing to move).
sub keep_or_give_up {
  my ($kept, $dir, $what) = @_;
  my $stamp = "$dir.keep-failed";
  if ($kept) { unlink $stamp; return '' }
  my $since = (stat $stamp)[9];
  if (!defined $since) {
    if (open(my $s, '>', $stamp)) { close $s; $since = time } else { $since = (stat "$dir.running")[9] // 0 }
  }
  if (time - $since < 3600) { say_err("couldn't keep what a run that stopped on this host left$what; trying again next time"); exit 70 }
  unlink $stamp;
  my $aside = '';
  if (-e $dir && (mkdir("$root/aside", 0700) || -d "$root/aside")) {
    (my $name = $dir) =~ s{.*/}{};
    my $to = "$root/aside/$name-" . time . "-$$";
    # A worktree copy's `.git` names the session's admin folder, which the
    # fresh checkout reuses under the same name: git in the copy would
    # move the live session's branch and index. So it is unlinked
    # (`.git-was-worktree`) and the run's HEAD written beside its files.
    # Only a worktree's `.git` file: a repository an agent made in a folder
    # copy (a `.git` folder) is its own and stays as it is.
    my $head = -f "$dir/.git" ? capture('git', '-C', $dir, 'rev-parse', '-q', '--verify', 'HEAD') : '';
    if (rename($dir, $to)) {
      if (-f "$to/.git") { rename("$to/.git", "$to/.git-was-worktree") or unlink("$to/.git") }
      if ($head ne '' && open(my $h, '>', "$to/.hollow-aside")) { print $h "head $head\n"; close $h }
      utime(undef, undef, $to); $aside = $to; $ASIDE = $to;
    } else { say_err("couldn't move it aside ($!); it goes") }
  }
  say_err("couldn't keep what a run that stopped on this host left$what for an hour; " . ($aside ne '' ? "moved it to $aside and starting fresh" : "starting it fresh"));
  return $aside;
}
# What a run that died left in a worktree session folder, kept as one of its
# results (`<hash>-kept-*`, which the Mac brings back on a branch) before the
# folder is reset: the branch head it left plus everything it didn't commit,
# like the commit its finish would have written. Nothing when it wrote its
# result after all (its finish drops `.sent` once the result is there), or
# when it left the folder exactly as it was sent. False only when the kept
# ref couldn't be written: the caller then leaves the folder and its marker
# alone, so a later preparation tries again.
sub salvage_worktree {
  my ($repo, $dir, $hash) = @_;
  return 1 unless -e "$dir/.git";
  my $sent = sent_of($dir); return 1 if $sent eq '';
  return 1 if run_done($dir, "$dir.running");
  # An earlier attempt's result in the same copy doesn't settle it (that
  # one is kept as well); only with no `.done` at all does a result count.
  return 1 if !-e "$dir.done" && capture('git', '-C', $repo, 'rev-parse', '-q', '--verify', "refs/hollow-session/$hash-result") ne '';
  # Only a copy of this repository: one whose git directory is elsewhere is
  # remade by the preparation, as before.
  my $c = capture('git', '-C', $dir, 'rev-parse', '--path-format=absolute', '--git-common-dir');
  return 1 unless $c ne '' && capture('/bin/sh', '-c', 'cd "$1" && pwd -P', 'x', $c) eq capture('/bin/sh', '-c', 'cd "$1" && pwd -P', 'x', $repo);
  my $headc = capture('git', '-C', $dir, 'rev-parse', 'HEAD');
  return 1 unless $headc =~ /\A[0-9a-f]{40,64}\z/;
  my $tmpidx = "$root/scratch/.salvage-index.$$";
  my $result = $headc;
  {
    local $ENV{GIT_INDEX_FILE} = $tmpidx;
    quiet('git', '-C', $dir, 'read-tree', 'HEAD');
    quiet('git', '-C', $dir, 'add', '-A', '.');
    my $tree = capture('git', '-C', $dir, 'write-tree');
    unlink $tmpidx;
    return 1 unless $tree =~ /\A[0-9a-f]+\z/;
    return 1 if $tree eq capture('git', '-C', $dir, 'rev-parse', "$sent^{tree}");
    if ($tree ne capture('git', '-C', $dir, 'rev-parse', 'HEAD^{tree}')) {
      local $ENV{GIT_AUTHOR_NAME} = 'hollow-session'; local $ENV{GIT_AUTHOR_EMAIL} = 'hollow-session@localhost';
      local $ENV{GIT_COMMITTER_NAME} = 'hollow-session'; local $ENV{GIT_COMMITTER_EMAIL} = 'hollow-session@localhost';
      my $r = capture('git', '-C', $dir, '-c', 'commit.gpgSign=false', 'commit-tree', $tree, '-p', $headc, '-m', "hollow-session: what a run that stopped there left, on $headc");
      return 0 unless $r =~ /\A[0-9a-f]+\z/;
      $result = $r;
    }
  }
  quiet('git', '-C', $repo, 'update-ref', "refs/hollow-session/$hash-kept-" . time . "-$$-died", $result) or return 0;
  say_err("kept what a run that stopped on this host left ($result)");
  return 1;
}
# The same for a vault workspace's folder copy: its files as one commit on
# top of the snapshot it was prepared from, with the junk a finish leaves
# out (`remote_folder::EXCLUDE`, the same text).
my $FOLDER_EXCLUDE = ".DS_Store\nnode_modules/\n.trash/\n*.hollow-tmp-*\n/sessions/\n";
# The bounds `remote_folder` keeps, the same numbers: `MAX_FILE_BYTES`,
# `GITIGNORED_MAX_FILES`, `GITIGNORED_MAX_BYTES`.
my ($FOLDER_MAX_BYTES, $IGNORED_MAX_FILES, $IGNORED_MAX_BYTES) = (52428800, 2000, 209715200);
# A folder that ignores itself (overnight R3 review): a tool's cache or
# environment, junk like `node_modules/` (`remote_folder::self_ignoring`, the
# same test): `CACHEDIR.TAG`, `pyvenv.cfg`, or a `.gitignore` of just `*`.
my %SELF_IGNORING;
sub self_ignoring {
  my $d = shift;
  return $SELF_IGNORING{$d} if exists $SELF_IGNORING{$d};
  my $yes = (-e "$d/CACHEDIR.TAG" || -l "$d/CACHEDIR.TAG" || -e "$d/pyvenv.cfg" || -l "$d/pyvenv.cfg") ? 1 : 0;
  my $g = "$d/.gitignore";
  if (!$yes && !-l $g && -f $g && -s _ <= 4096 && open(my $i, '<', $g)) {
    my @lines = grep { $_ ne '' && !/\A#/ } map { my $l = $_; $l =~ s/\A\s+|\s+\z//g; $l } <$i>;
    close $i;
    $yes = (@lines == 1 && $lines[0] eq '*') ? 1 : 0;
  }
  return $SELF_IGNORING{$d} = $yes;
}
# What of a folder copy stays there (overnight R3 review), relative: folders
# that ignore themselves and files over the cap; junk and repositories not
# entered.
sub folder_stays {
  my $fdir = shift; my (@caches, @big); my @todo = ('');
  while (@todo) {
    my $rel = pop @todo; my $d = $rel eq '' ? $fdir : "$fdir/$rel";
    opendir(my $dh, $d) or next; my @names = readdir $dh; closedir $dh;
    for my $n (@names) {
      next if $n eq '.' || $n eq '..' || $n eq '.DS_Store' || index($n, '.hollow-tmp-') >= 0;
      my $r = $rel eq '' ? $n : "$rel/$n"; my $p = "$fdir/$r";
      next if -l $p;
      if (-d _) {
        next if $n eq '.git' || $n eq 'node_modules' || $n eq '.trash' || ($rel eq '' && $n eq 'sessions') || -e "$p/.git";
        if (self_ignoring($p)) { push @caches, $r } else { push @todo, $r }
      } elsif (-f _ && -s _ > $FOLDER_MAX_BYTES) { push @big, $r }
    }
  }
  return (\@caches, \@big);
}
# One more path of a kind left out (`[count, first paths...]`, five named).
sub left_push { my ($l, $p) = @_; $l->[0]++; push @$l, $p if @$l < 6 && $p !~ /[\r\n]/ }
# What was left out, as the result message's lines (`remote_folder::left_there`
# reads them).
sub left_lines {
  my %kinds = @_; my $m = '';
  for my $k (sort keys %kinds) { my ($n, @first) = @{ $kinds{$k} }; next unless $n; $m .= "left: $k $n\n"; $m .= "left-path: $k $_\n" for @first }
  return $m;
}
# NUL-separated paths to a git command's stdin or a pathspec file.
sub nul_to {
  my ($fdir, $list, @cmd) = @_;
  my $pid = open(my $w, '|-'); return 0 unless defined $pid;
  if (!$pid) { open(STDOUT, '>', '/dev/null'); open(STDERR, '>', '/dev/null'); exec('git', '-C', $fdir, '-c', 'core.autocrlf=false', '-c', 'core.attributesFile=/dev/null', @cmd) or exit 127 }
  print $w join("\0", @$list), "\0"; close $w; return $? == 0;
}
# What only the copy's own `.gitignore` files hid from `add -A` (overnight
# R3), added by name as on the way there: the files still untracked after it
# that the junk list (`$fex`) doesn't name, listed with that list and no
# `.gitignore`; none in a cache or a repository made in the copy, none over
# the cap, and at most $IGNORED_MAX_FILES files and $IGNORED_MAX_BYTES bytes
# of them (the rest counted in `$past`). `update-index --stdin` takes them in
# linear time and passes over one that went meanwhile; it stops at one it
# can't read, so those are left out first. Never `add -f` on the folder: it
# would take the junk too. Secret-looking names aren't sorted out here: the
# Mac never takes the dangerous ones back (`remote_folder::never_taken_back`).
sub add_gitignored {
  my ($fdir, $fex, $caches, $past) = @_;
  my ($files, $bytes, @names) = (0, 0);
  NAME: for my $p (split /\0/, capture('git', '-C', $fdir, '-c', 'core.precomposeUnicode=true', 'ls-files', '-z', '--others', "--exclude-from=$fex")) {
    next if $p eq '' || $p =~ m{/\z};
    for my $c (@$caches) { next NAME if index("$p/", "$c/") == 0 }
    my @parts = split m{/}, $p; pop @parts; my $at = $fdir;
    for (@parts) { $at .= "/$_"; next NAME if -e "$at/.git" || -l "$at/.git" || self_ignoring($at) }
    my @st = lstat "$fdir/$p" or next;
    next if -f _ && (!-r _ || $st[7] > $FOLDER_MAX_BYTES);
    if ($files >= $IGNORED_MAX_FILES || $bytes + $st[7] > $IGNORED_MAX_BYTES) { left_push($past, $p); next }
    $files++; $bytes += $st[7]; push @names, $p;
  }
  nul_to($fdir, \@names, '-c', 'core.precomposeUnicode=true', 'update-index', '--add', '--remove', '-z', '--stdin') if @names;
}
# A folder copy's files as one commit on top of `$base` (the caller's
# environment names the repository, work tree and index; `$fex` holds the
# junk list): caches and files over the cap left out too, also when a
# `.gitignore` negation let `add -A` take them, what the copy's `.gitignore`
# names added within bounds, and what stayed there named in the message.
# What of those the snapshot held goes back in as it was sent (round 2
# review): a sent file grown past the cap there, or a sent folder that became
# a cache, would otherwise read as removed, and this Mac would remove its
# copy. '' when nothing changed, undef when git failed.
sub folder_commit {
  my ($fdir, $fex, $base, $msg) = @_;
  my ($caches, $big) = folder_stays($fdir);
  if (open(my $o, '>>', $fex)) {
    for my $p ((map { "$_/" } @$caches), @$big) { next if $p =~ /[\r\n]/; (my $q = $p) =~ s/([*?\[\\ !#])/\\$1/g; print $o "/$q\n" }
    close $o;
  }
  quiet('git', '-C', $fdir, '-c', 'core.autocrlf=false', '-c', 'core.attributesFile=/dev/null', 'read-tree', $base) or return undef;
  quiet('git', '-C', $fdir, '-c', 'core.autocrlf=false', '-c', 'core.attributesFile=/dev/null', '-c', "core.excludesFile=$fex", '-c', 'core.precomposeUnicode=true', 'add', '-A', '--ignore-errors', '.');
  my $past = [0];
  add_gitignored($fdir, $fex, $caches, $past);
  if (@$caches || @$big) {
    my $list = "$fex.rm";
    if (open(my $o, '>', $list)) { print $o join("\0", @$caches, @$big), "\0"; close $o;
      quiet('git', '-C', $fdir, '--literal-pathspecs', 'rm', '--cached', '-r', '-q', '-f', '--ignore-unmatch', '--pathspec-file-nul', "--pathspec-from-file=$list"); unlink $list }
    my %big = map { $_ => 1 } @$big; my @sent;
    for my $e (split /\0/, capture('git', '-C', $fdir, 'ls-tree', '-r', '-z', '--full-tree', $base)) {
      my (undef, $path) = split /\t/, $e, 2; next unless defined $path;
      push @sent, $e if $big{$path} || grep { index($path, "$_/") == 0 } @$caches;
    }
    nul_to($fdir, \@sent, 'update-index', '-z', '--index-info') or return undef if @sent;
  }
  my $bigl = [0]; left_push($bigl, $_) for @$big;
  my $left = left_lines(big => $bigl, budget => $past);
  my $tree = capture('git', '-C', $fdir, '-c', 'core.autocrlf=false', 'write-tree');
  return undef unless $tree =~ /\A[0-9a-f]+\z/;
  return '' if $left eq '' && $tree eq capture('git', '-C', $fdir, '-c', 'core.autocrlf=false', 'rev-parse', "$base^{tree}");
  my $r = capture('git', '-C', $fdir, '-c', 'core.autocrlf=false', '-c', 'commit.gpgSign=false', 'commit-tree', $tree, '-p', $base, '-m', $msg, ($left ne '' ? ('-m', $left) : ()));
  return $r =~ /\A[0-9a-f]+\z/ ? $r : undef;
}
sub salvage_folder {
  my ($repo, $fdir, $hash) = @_;
  return 1 unless -d $fdir;
  my $base = sent_of($fdir); return 1 if $base eq '';
  return 1 if run_done($fdir, "$fdir.running");
  return 1 if !-e "$fdir.done" && capture('git', '-C', $repo, 'rev-parse', '-q', '--verify', "refs/hollow-session/$hash-result") ne '';
  my $fidx = "$root/scratch/.salvage-folder-index.$$"; my $fex = "$root/scratch/.salvage-folder-exclude.$$";
  open(my $o, '>', $fex) or return 0; print $o $FOLDER_EXCLUDE; close $o;
  my $fres = '';
  {
    local $ENV{GIT_DIR} = $repo; local $ENV{GIT_WORK_TREE} = $fdir; local $ENV{GIT_INDEX_FILE} = $fidx;
    local $ENV{GIT_AUTHOR_NAME} = 'hollow-folder'; local $ENV{GIT_AUTHOR_EMAIL} = 'hollow-folder@localhost';
    local $ENV{GIT_COMMITTER_NAME} = 'hollow-folder'; local $ENV{GIT_COMMITTER_EMAIL} = 'hollow-folder@localhost';
    $fres = folder_commit($fdir, $fex, $base, 'hollow-folder: what a run that stopped there left');
  }
  unlink $fidx, $fex;
  # Git failed: not kept, so the copy stays for the next try.
  return 0 unless defined $fres;
  return 1 if $fres eq '';
  quiet('git', '-C', $repo, 'update-ref', "refs/hollow-session/$hash-kept-" . time . "-$$-died", $fres) or return 0;
  say_err("kept what a run that stopped on this host left in the folder ($fres)");
  return 1;
}
# A result no one brought back to the Mac is kept, never dropped: the next
# --run would write over it. Every kept result is listed at each
# preparation, and only the Mac drops one, after it has it (compare and
# swap): a fetch that fails leaves it here for the next turn.
sub keep_results {
  my ($repo, $hash) = @_;
  my @kept;
  my $res = capture('git', '-C', $repo, 'rev-parse', '-q', '--verify', "refs/hollow-session/$hash-result");
  # The result goes only once a kept ref holds it (and only if it is still
  # the one read: compare and swap). Not held: stop here.
  my $held = $res !~ /\A[0-9a-f]+\z/ || quiet('git', '-C', $repo, 'update-ref', "refs/hollow-session/$hash-kept-" . time . "-$$", $res);
  $held or do { say_err("couldn't keep an earlier turn's result on this host"); exit 70 };
  quiet('git', '-C', $repo, 'update-ref', '-d', "refs/hollow-session/$hash-head");
  quiet('git', '-C', $repo, 'update-ref', '-d', "refs/hollow-session/$hash-result", $res) if $res =~ /\A[0-9a-f]+\z/;
  for (split /\n/, capture('git', '-C', $repo, 'for-each-ref', '--format=%(refname) %(objectname)', "refs/hollow-session/")) {
    my ($ref, $sha) = split ' '; next unless defined $sha;
    push @kept, { ref => $ref, sha => $sha } if index($ref, "refs/hollow-session/$hash-kept-") == 0 && $sha =~ /\A[0-9a-f]+\z/;
  }
  return \@kept;
}
# A build folder's build-script outputs pointed at itself (build_slots.rs
# r_repath, the same code).
sub repath {
  local $/ = "\n";
  my ($t, $base) = @_; my ($name) = ($t // "") =~ m{/([^/]+)\z}; return unless defined $name && defined $base;
  opendir(my $d, $t) or return;
  my @dirs = grep { -d $_ } map { "$t/$_/build" } grep { !/\A\./ } readdir $d;
  return unless @dirs;
  my $pid = open(my $g, "-|") // return;
  if (!$pid) { open(STDERR, ">", "/dev/null"); exec { "/usr/bin/grep" } "grep", "-rlIF", "--", "$base/", @dirs or exit 127 }
  my @files = <$g>; close $g; chomp @files;
  for my $f (@files) {
    next if -l $f || !-f $f;
    my @st = stat $f; open(my $i, "<:raw", $f) or next; my $c = do { local $/; <$i> }; close $i; next if index($c, "\0") >= 0;
    (my $n = $c) =~ s{\Q$base/\E(?!\Q$name/\E)[A-Za-z0-9_-][A-Za-z0-9._-]*/}{$base/$name/}g;
    next if $n eq $c;
    my $tmp = "$f.repath.$$"; open(my $o, ">:raw", $tmp) or next; print $o $n; close $o or do { unlink $tmp; next };
    chmod($st[2] & 07777, $tmp); rename($tmp, $f) or do { unlink $tmp; next }; utime($st[8], $st[9], $f);
  }
}
# A session's fresh scratch folder, its own port (2700-2999, kept with it)
# and its build folder.
sub extras {
  my $id = shift;
  my $scratch = "$root/scratch/$id"; quiet('rm', '-rf', $scratch); mkdir($scratch, 0700) or do { say_err("no scratch folder"); exit 70 };
  my $pf = "$root/sessions/$id.port"; my $port = '';
  if (open(my $fh, '<', $pf)) { $port = <$fh> // ''; chomp $port; close $fh }
  if ($port !~ /\A(2[7-9][0-9][0-9])\z/) {
    my %taken; opendir(my $sd, "$root/sessions"); for my $f (readdir $sd) { next unless $f =~ /\.port\z/ && $f ne "$id.port"; open(my $p, '<', "$root/sessions/$f") or next; my $v = <$p> // ''; chomp $v; $taken{$v} = 1 }
    my $start = hex(substr(capture('/bin/sh', '-c', 'printf %s "$1" | /usr/bin/shasum -a 256', 'x', $id), 0, 4)) % 300;
    for my $i (0 .. 299) { my $try = 2700 + ($start + $i) % 300; next if $taken{$try}; $port = $try; last }
    if (open(my $o, '>', $pf)) { print $o "$port\n"; close $o }
  }
  return ($scratch, $port, "$root/targets/session-$id");
}

if ($mode eq '--prepare') {
  my ($repo_id, $wt_id, $snap, $head, $branch, $clean) = @ARGV;
  for ($repo_id, $wt_id) { name_ok($_) or do { say_err("bad name"); exit 2 } }
  for ($snap, $head) { /\A[0-9a-f]{40,64}\z/ or do { say_err("bad commit"); exit 2 } }
  my $repo = "$root/repos/$repo_id.git"; my $dir = "$root/sessions/$wt_id";
  my $died = wait_turn("$dir.running");
  my ($hash) = $wt_id =~ /-([0-9a-f]+)\z/;
  my $aside = '';
  if ($died) {
    $aside = keep_or_give_up(!defined $hash || salvage_worktree($repo, $dir, $hash), $dir, '');
    unlink "$dir.running";
  } else { unlink "$dir.keep-failed" }
  my $kept = defined $hash ? keep_results($repo, $hash) : [];
  my $ok = 0;
  if (-e "$dir/.git") {
    my $c = capture('git', '-C', $dir, 'rev-parse', '--path-format=absolute', '--git-common-dir');
    my $real_c = $c ne '' ? capture('/bin/sh', '-c', 'cd "$1" && pwd -P', 'x', $c) : '';
    my $real_r = capture('/bin/sh', '-c', 'cd "$1" && pwd -P', 'x', $repo);
    $ok = $real_c ne '' && $real_c eq $real_r && quiet('git', '-C', $dir, 'checkout', '-q', '-f', '--detach', $snap) && quiet('git', '-C', $dir, 'clean', '-q', '-f', '-d');
  }
  if (!$ok) {
    say_err("making the session folder on this host") if -e $dir;
    quiet('rm', '-rf', $dir); quiet('git', '-C', $repo, 'worktree', 'prune');
    quiet('git', '-C', $repo, 'worktree', 'add', '-q', '-f', '--detach', $dir, $snap) or do { say_err("couldn't check out $snap"); exit 70 };
  }
  if ($branch ne '-') {
    ($branch !~ /\A-/ && quiet('git', 'check-ref-format', '--branch', $branch)) or do { say_err("bad branch"); exit 2 };
    quiet('git', '-C', $dir, 'checkout', '-q', '-f', '--ignore-other-worktrees', '-B', $branch, $snap) or do { say_err("couldn't take branch $branch"); exit 70 };
  }
  quiet('git', '-C', $dir, 'reset', '-q', '--mixed', $head) if $clean eq '0';
  mark_sent($dir, $snap);
  my ($scratch, $port, $target) = extras($wt_id);
  # An APFS clone. On a Linux runner only a reflink one (btrfs, XFS): a
  # whole copy of a build folder takes minutes, so where the file system
  # can't clone (ext4) the session's starts empty (build_slots.rs does the
  # same).
  if (!-d $target && -d "$root/targets/$wt_id") {
    if ($^O ne 'linux') { quiet('/bin/cp', '-cR', "$root/targets/$wt_id", $target) and quiet('/bin/sh', '-c', 'cd "$1" && git ls-files -z | xargs -0 touch', 'x', $dir) }
    elsif (!can_reflink("$root/targets")) { say_err("the session's build folder starts empty: this file system can't clone the worktree's (cp --reflink)") }
    elsif (quiet('/bin/cp', '-a', '--reflink=always', "$root/targets/$wt_id", $target)) { quiet('/bin/sh', '-c', 'cd "$1" && git ls-files -z | xargs -0 touch', 'x', $dir) }
    else { quiet('rm', '-rf', $target) }
  }
  # Its build-script outputs name the folder it was cloned from: point them
  # here (build_slots.rs r_repath says why; same code).
  repath($target, "$root/targets") if -d $target;
  if (-f "$dir/package-lock.json") {
    my $want = substr(capture('/bin/sh', '-c', '/usr/bin/shasum -a 256 < "$1"', 'x', "$dir/package-lock.json"), 0, 64);
    my $have = ''; if (open(my $fh, '<', "$dir/node_modules/.hollow-remote-lock")) { $have = <$fh> // ''; chomp $have }
    if ($want ne $have) {
      say_err("npm ci for the session folder (package-lock.json is new here)");
      my $pid = fork(); if (!$pid) { chdir $dir; open(STDOUT, '>&', \*STDERR); open(STDIN, '<', '/dev/null'); exec('npm', 'ci', '--no-audit', '--no-fund', '--loglevel=error') or exit 127 }
      waitpid($pid, 0);
      if ($? == 0 && open(my $o, '>', "$dir/node_modules/.hollow-remote-lock")) { print $o "$want\n" }
    }
  }
  print encode_json({ dir => $dir, scratch => $scratch, port => $port + 0, target => $target, host => host_name(), kept => $kept, aside => $aside }) . "\n";
  exit 0;
}

# A vault workspace's folder (Phase 3): `--prepare-folder <repo id> <folder
# id> <snapshot> <session 0|1>` checks the pushed snapshot out fresh in
# `folders/<folder id>` (a plain folder: the bare repository is its git
# directory only while a result is made, so the agent sees no repository).
# With session 1 the folder is the agent's working folder, so it also gets
# the session's scratch folder, port and build folder, and its own marker.
if ($mode eq '--prepare-folder') {
  my ($repo_id, $fid, $snap, $session) = @ARGV;
  for ($repo_id, $fid) { name_ok($_) or do { say_err("bad name"); exit 2 } }
  ($snap // '') =~ /\A[0-9a-f]{40,64}\z/ or do { say_err("bad commit"); exit 2 };
  my $repo = "$root/repos/$repo_id.git"; my $fdir = "$root/folders/$fid";
  -d "$root/folders" or mkdir("$root/folders", 0700) or do { say_err("can't make $root/folders"); exit 70 };
  # A build workspace's folder (session 0) has the run's second marker; its
  # worktree's preparation has waited for that run already.
  my $died = wait_turn("$fdir.running");
  my ($hash) = $fid =~ /-([0-9a-f]+)\z/;
  defined $hash or do { say_err("bad folder id"); exit 2 };
  my $aside = '';
  if ($died) {
    $aside = keep_or_give_up(salvage_folder($repo, $fdir, $hash), $fdir, ' in the folder');
    unlink "$fdir.running";
  } else { unlink "$fdir.keep-failed" }
  my $kept = keep_results($repo, $hash);
  quiet('rm', '-rf', $fdir); mkdir($fdir, 0700) or do { say_err("can't make $fdir"); exit 70 };
  my $idx = "$root/scratch/.folder-index.$$";
  {
    local $ENV{GIT_DIR} = $repo; local $ENV{GIT_WORK_TREE} = $fdir; local $ENV{GIT_INDEX_FILE} = $idx;
    quiet('git', '-C', $fdir, '-c', 'core.autocrlf=false', '-c', 'core.attributesFile=/dev/null', 'read-tree', '--reset', '-u', $snap) or do { unlink $idx; say_err("couldn't check out the workspace folder"); exit 70 };
  }
  unlink $idx;
  mark_sent($fdir, $snap);
  my %out = (dir => $fdir, kept => $kept, aside => $aside, host => host_name());
  if (($session // '') eq '1') { my ($scratch, $port, $target) = extras($fid); %out = (%out, scratch => $scratch, port => $port + 0, target => $target) }
  print encode_json(\%out) . "\n";
  exit 0;
}

# One tool connection from the agent, spliced to the relay with the run's
# token (`--run`'s child; `$conn` passed its peer check).
sub splice_tool {
  my ($conn, $relay, $token) = @_;
  local $SIG{ALRM} = sub { exit 0 }; alarm 10;
  my $name = ''; my $c;
  while (sysread($conn, $c, 1)) { last if $c eq "\n"; $name .= $c; exit 0 if length($name) > 200 }
  alarm 0;
  name_ok($name) or exit 0;
  my $up = IO::Socket::UNIX->new(Type => SOCK_STREAM(), Peer => $relay) or exit 0;
  syswrite($up, "HOLLOW-RELAY 1 $token $name\n") or exit 0;
  my $sel = IO::Select->new($conn, $up);
  while ($sel->count) {
    for my $r ($sel->can_read) {
      my $to = $r == $conn ? $up : $conn;
      my $b; my $n = sysread($r, $b, 65536);
      if (!$n) { $sel->remove($r); shutdown($to, 1); next }
      my $o = 0; while ($o < $n) { my $w = syswrite($to, $b, $n - $o, $o); exit 0 unless $w; $o += $w }
    }
  }
  exit 0;
}

sub put { my ($fh, $b) = @_; my $o = 0; while ($o < length $b) { my $w = syswrite($fh, $b, length($b) - $o, $o); return 0 unless $w; $o += $w } return 1 }
# One line from `$fh`, waiting `$secs` at most (undef: none).
sub read_line {
  my ($fh, $secs) = @_; my $s = IO::Select->new($fh); my $line = ''; my $c; my $end = time + $secs;
  while (length($line) < 4096) { my $left = $end - time; return undef if $left <= 0 || !$s->can_read($left); sysread($fh, $c, 1) or return undef; return $line if $c eq "\n"; $line .= $c }
  return undef;
}
# Equal secrets, compared in constant time.
sub same { my ($x, $y) = @_; return 0 unless defined $x && defined $y && length($x) == length($y) && length($y) >= 16; my $d = 0; $d |= ord(substr($x, $_, 1)) ^ ord(substr($y, $_, 1)) for 0 .. length($y) - 1; return !$d }
# The relay socket an attach's ssh forwards here: a new name each attach
# (sshd won't take over a name an earlier connection may still hold).
sub relay_ok { my ($run, $p) = @_; return ($p // '') =~ m{\A\Q$root\E/run/\Q$run\E\.\d{1,6}\.sock\z} }

# Remote runners (2026-11-03): a run's GitHub or Google Cloud credential, one
# request at a time, through this run's tool socket (only the agent's own
# descendants get in) to the leader on the Mac that started the run, which
# answers `ok <value>` or `no <why>` from what it holds. Nothing is kept here.
sub cred_ask {
  my $what = shift;
  my $sock = $ENV{HOLLOW_RELAY_SOCK} // '';
  return (0, "this isn't a Hollow run on the runner") unless length $sock;
  my $s = IO::Socket::UNIX->new(Type => SOCK_STREAM(), Peer => $sock) or return (0, "the run's tool socket is gone ($!)");
  put($s, "hollow-credential-$what\n") or return (0, "the run's tool socket is gone");
  # Longer than the leader gives a mint (60 s), so a slow one says so.
  my $line = read_line($s, 90); close $s;
  return (0, "the Mac that started this run didn't answer (asleep, or the connection dropped; the run goes on and can ask again)") unless defined $line;
  return (1, $1, $2 // '') if $line =~ /\Aok (\S+)(?: (\S+))?\z/;
  return (0, $1) if $line =~ /\Ano (.+)\z/;
  return (0, "the Mac that started this run gave no credential");
}
# The program `$name` on PATH other than Hollow's own wrapper: its folder is
# this script's own (never found through HOME, which a command may change).
sub real_program {
  my $name = shift; require Cwd; require File::Basename;
  my $me = Cwd::abs_path($0) // $0;
  my %own = map { (Cwd::abs_path($_) // $_) => 1 } grep { -e $_ } (File::Basename::dirname($me) . "/$name", "$root/bin/$name");
  my %own_dir = map { (Cwd::abs_path($_) // $_) => 1 } grep { -d $_ } (File::Basename::dirname($me), "$root/bin");
  for my $d (split(/:/, $ENV{PATH} // ''), '/opt/homebrew/bin', '/usr/local/bin', '/usr/bin', '/bin', '/snap/bin') {
    next if $d eq ''; my $p = "$d/$name";
    next unless -f $p && -x _;
    my $real = Cwd::abs_path($p) // $p;
    next if $own_dir{Cwd::abs_path($d) // $d} || $own{$real} || $real eq $me;
    return $p;
  }
  return;
}
# Remote runners: with GitHub on, the run's own git configuration (no secret),
# standing in for the runner account's global one, which it includes: a
# credential helper for github.com only (after an empty one, which drops any
# other, such as Apple's Keychain helper), https for GitHub's ssh URLs, and
# `origin` (HOLLOW_RUNNER_ORIGIN) only inside this run's checkout: an
# includeIf on its repository, so no other repository the agent touches gets
# it. A file, not GIT_CONFIG_KEY_n: Codex drops variables named *KEY*.
sub cfg_quote { my $s = shift; $s =~ s/\\/\\\\/g; $s =~ s/"/\\"/g; return "\"$s\"" }
sub run_git_config {
  my ($scratch, $checkout) = @_;
  my $origin = delete $ENV{HOLLOW_RUNNER_ORIGIN} // '';
  return unless ($ENV{HOLLOW_RUNNER_GITHUB} // '') eq 'on';
  my $prev = $ENV{GIT_CONFIG_GLOBAL} // '';
  my $sq = sub { my $v = shift; $v =~ s/'/'\\''/g; return "'$v'" };
  my $text = "\# Hollow (remote runners): this run's git configuration; no secret.\n[include]\n";
  my $xdg = length($ENV{XDG_CONFIG_HOME} // '') ? cfg_quote("$ENV{XDG_CONFIG_HOME}/git/config") : '~/.config/git/config';
  $text .= length $prev ? "\tpath = " . cfg_quote($prev) . "\n" : "\tpath = $xdg\n\tpath = ~/.gitconfig\n";
  $text .= "[credential \"https://github.com\"]\n\thelper =\n\thelper = " . cfg_quote('!/usr/bin/perl ' . $sq->("$root/bin/hollow-remote-run") . ' --credential') . "\n";
  $text .= "[url \"https://github.com/\"]\n\tinsteadOf = git\@github.com:\n\tinsteadOf = ssh://git\@github.com/\n";
  if ($origin =~ m{\Ahttps://github\.com/[A-Za-z0-9._-]+/[A-Za-z0-9._-]+\.git\z} && length($checkout // '')) {
    my $common = capture('git', '-C', $checkout, 'rev-parse', '--path-format=absolute', '--git-common-dir');
    if ($common =~ m{\A/[^\n"\\]+\z}) {
      require Cwd; my %dirs = map { $_ => 1 } grep { defined } ($common, Cwd::abs_path($common));
      open(my $o, '>', "$scratch/gitconfig-origin") or return;
      print $o "[remote \"origin\"]\n\turl = $origin\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n"; close $o;
      $text .= "[includeIf \"gitdir:$_/\"]\n\tpath = " . cfg_quote("$scratch/gitconfig-origin") . "\n" for sort keys %dirs;
    }
  }
  open(my $o, '>', "$scratch/gitconfig") or return;
  print $o $text; close $o or return;
  $ENV{GIT_CONFIG_GLOBAL} = "$scratch/gitconfig";
}
# What a runner turn's gh may not do: print or store the credential.
sub gh_refused {
  my @w = grep { !/\A-/ } @_; my %f = map { $_ => 1 } grep { /\A-/ } @_;
  return '' unless @w && $w[0] eq 'auth';
  return "gh auth $w[1] isn't available on a runner: the run's GitHub credential comes from the Mac that started it, one call at a time, and is never stored here" if @w > 1 && $w[1] =~ /\A(token|login|logout|refresh|setup-git|switch)\z/;
  return "gh auth status doesn't show the token on a runner" if $f{'-t'} || $f{'--show-token'};
  return '';
}
# What a runner turn's gcloud may not do: print or store the credential
# (always), and production changes unless the workspace's Google Cloud access
# is My account (full). A guard rail: the hard limit is the account's IAM
# (a service account, Runs on → On the runner → Google Cloud).
sub gcloud_refused {
  my ($access, @a) = @_;
  my $words = ' ' . join(' ', grep { !/\A-/ } @a) . ' '; my %f = map { (split /=/, $_, 2)[0] => 1 } grep { /\A-/ } @a;
  return "gcloud $1 isn't available on a runner: the run's Google Cloud credential comes from the Mac that started it, one call at a time, and is never stored or shown here" if $words =~ / (auth (?:print-access-token|print-identity-token|login|activate-service-account|application-default)|config config-helper) /;
  return '' if $access eq 'full';
  my $prod = $words =~ / builds (approve|reject) / ? "builds $1"
    : $words =~ / run services (update-traffic|update|replace|delete) / ? "run services $1"
    : ($words =~ / run deploy / && !$f{'--no-traffic'}) ? 'run deploy (without --no-traffic)'
    : $words =~ / deploy (releases promote|rollouts approve|rollouts reject) / ? "deploy $1" : '';
  return $prod ne '' ? "gcloud $prod is a production change, which this workspace's runner turns may not make (Google Cloud access: no production changes). Run it on the Mac, or set the workspace's access to My account (full)" : '';
}

if ($mode eq '--credential') {
  # git's credential helper protocol: the action, then `key=value` lines.
  my $action = shift // ''; my %q;
  while (defined(my $l = <STDIN>)) { chomp $l; last if $l eq ''; my ($k, $v) = split /=/, $l, 2; $q{$k} = $v if defined $v }
  exit 0 unless $action eq 'get' && ($q{protocol} // '') eq 'https' && ($q{host} // '') eq 'github.com';
  if (($ENV{HOLLOW_RUNNER_GITHUB} // '') ne 'on') { print STDERR "hollow: GitHub isn't turned on for this workspace's runner turns (Runs on, On the runner, in Hollow on the Mac).\n"; exit 0 }
  my ($ok, $v) = cred_ask('github');
  if (!$ok) { print STDERR "hollow: no GitHub credential: $v\n"; exit 0 }
  print "protocol=https\nhost=github.com\nusername=x-access-token\npassword=$v\n";
  exit 0;
}
if ($mode eq '--wrap') {
  my $tool = shift // '';
  $tool =~ /\A(gh|gcloud)\z/ or do { say_err("usage: hollow-remote-run --wrap gh|gcloud …"); exit 2 };
  # A wrapper that somehow reaches itself again stops instead of looping.
  my $depth = ($ENV{HOLLOW_WRAP_DEPTH} // 0) =~ /\A(\d+)\z/ ? $1 : 0;
  $depth < 4 or do { print STDERR "hollow: $tool keeps reaching Hollow's own wrapper; check PATH\n"; exit 127 };
  $ENV{HOLLOW_WRAP_DEPTH} = $depth + 1;
  my $real = real_program($tool) or do { print STDERR "$tool isn't installed on this runner\n"; exit 127 };
  if ($tool eq 'gh') {
    if (($ENV{HOLLOW_RUNNER_GITHUB} // '') eq 'on') {
      if (my $why = gh_refused(@ARGV)) { print STDERR "hollow: $why\n"; exit 1 }
      my ($ok, $v) = cred_ask('github');
      $ok or do { print STDERR "hollow: no GitHub credential: $v\n"; exit 1 };
      $ENV{GH_TOKEN} = $v;
    }
    exec { $real } $real, @ARGV or exit 127;
  }
  my $access = $ENV{HOLLOW_RUNNER_CLOUD} // '';
  exec { $real } $real, @ARGV or exit 127 if $access eq '';
  if (my $why = gcloud_refused($access, @ARGV)) { print STDERR "hollow: $why\n"; exit 1 }
  my ($ok, $v, $level) = cred_ask('gcloud');
  $ok or do { print STDERR "hollow: no Google Cloud credential: $v\n"; exit 1 };
  # The level the Mac holds decides, not this run's variable.
  if (my $why = gcloud_refused($level ne '' ? $level : 'limited', @ARGV)) { print STDERR "hollow: $why\n"; exit 1 }
  # One file for this one call, made new (never through a link), deleted
  # when gcloud ends.
  my $dir = $ENV{HOLLOW_SCRATCH_DIR} // ''; $dir = "$root/scratch" unless length $dir && -d $dir;
  my $file = sprintf('%s/.gcloud-token-%d-%08x', $dir, $$, int(rand(0xffffffff)));
  require Fcntl;
  sysopen(my $o, $file, Fcntl::O_WRONLY() | Fcntl::O_CREAT() | Fcntl::O_EXCL() | (eval { Fcntl::O_NOFOLLOW() } // 0), 0600)
    or do { print STDERR "hollow: can't write the token file ($!)\n"; exit 1 };
  print $o $v; close $o;
  $ENV{CLOUDSDK_AUTH_ACCESS_TOKEN_FILE} = $file;
  my $kid = fork() // do { unlink $file; print STDERR "hollow: can't start gcloud\n"; exit 1 };
  if (!$kid) { exec { $real } $real, @ARGV or exit 127 }
  $SIG{$_} = do { my $s = $_; sub { kill($s, $kid) } } for qw(TERM INT HUP);
  1 while waitpid($kid, 0) == -1 && $!{EINTR};
  my $st = $?; unlink $file;
  exit($st & 127 ? 128 + ($st & 127) : $st >> 8);
}

if ($mode eq '--bridge') {
  # One of the agent's tools: this run's tool socket (it checks this
  # process descends from the agent, then adds the relay token, which never
  # reaches the agent's environment), copied both ways, one JSON-RPC message
  # a line. When the way to the Mac drops (the run goes on, Phase 4), the
  # requests still waiting are answered with an error, and the bridge
  # reconnects (every 2 s, then every 15 s after a minute, for as long as
  # the leader does: 12 hours) and replays the agent's
  # `initialize` to the new server, so the agent keeps its tools.
  my $name = shift // '';
  name_ok($name) or do { say_err("bad tool name"); exit 2 };
  $SIG{PIPE} = 'IGNORE';
  my $sock = $ENV{HOLLOW_RELAY_SOCK} // '';
  my $connect = sub { my $s = IO::Socket::UNIX->new(Type => SOCK_STREAM(), Peer => $sock) or return; put($s, "$name\n") or return; return $s };
  my $up = $connect->() or do { say_err("can't reach Hollow's tools on the Mac that started this run ($!)"); exit 1 };
  my $js = JSON::PP->new->allow_nonref;
  my $msg = sub { my $l = shift; return undef if length($l) > 8_000_000; my $m = eval { $js->decode($l) }; return ref $m eq 'HASH' ? $m : undef };
  my ($init, $inited);
  my %pending;
  my ($ready, $replay, $replay_by, $lost_at, $retry_at, $replays) = (1, undef, 0, undef, 0, 0);
  my ($abuf, $ubuf, $agent_open) = ('', '', 1);
  my $out = sub { put(\*STDOUT, "$_[0]\n") or exit 0 };
  my $refuse = sub { $out->($js->encode({ jsonrpc => '2.0', id => $_[0], error => { code => -32000, message => "Hollow's tools on the Mac that started this run are unreachable right now; try again" } })) };
  my $lose = sub {
    if ($up) { close $up; $up = undef }
    ($ready, $replay, $ubuf) = (0, undef, '');
    $refuse->($_) for values %pending;
    %pending = ();
    exit 0 unless $agent_open;
    $lost_at //= time; $retry_at = time + 2;
  };
  my $from_agent = sub {
    my $l = shift; my $m = $msg->($l);
    my $request = $m && defined $m->{method} && exists $m->{id};
    if ($m && defined $m->{method}) {
      $init = $l if $m->{method} eq 'initialize';
      $inited = $l if $m->{method} eq 'notifications/initialized';
    }
    if ($ready && $up) {
      $pending{ $js->encode($m->{id}) } = $m->{id} if $request;
      put($up, "$l\n") or $lose->();
    } elsif ($request) { $refuse->($m->{id}) }
  };
  my $from_up = sub {
    my $l = shift;
    if (defined $replay) {
      my $m = $msg->($l);
      if ($m && !defined $m->{method} && defined $m->{id} && !ref $m->{id} && $m->{id} eq $replay) {
        ($replay, $ready, $lost_at) = (undef, 1, undef);
        if (defined $inited) { put($up, "$inited\n") or $lose->() }
        return;
      }
    } elsif (%pending) {
      my $m = $msg->($l);
      delete $pending{ $js->encode($m->{id}) } if $m && !defined $m->{method} && exists $m->{id};
    }
    $out->($l);
  };
  while (1) {
    if (!$up && time >= $retry_at) {
      if (time - $lost_at > 12 * 3600) { say_err("Hollow's tools on the Mac that started this run stayed unreachable"); exit 1 }
      if ($up = $connect->()) {
        if (defined $init) {
          my $m = $msg->($init); $replays++; $replay = "hollow-replay-$replays"; $m->{id} = $replay; $replay_by = time + 10;
          put($up, $js->encode($m) . "\n") or $lose->();
        } else { ($ready, $lost_at) = (1, undef) }
      } else { $retry_at = time + (time - $lost_at < 60 ? 2 : 15) }
    }
    $lose->() if $up && defined $replay && time > $replay_by;
    my $sel = IO::Select->new; $sel->add(\*STDIN) if $agent_open; $sel->add($up) if $up;
    for my $r ($sel->can_read(0.5)) {
      # A connection lost earlier in this batch is closed: skip it.
      my $fd = fileno($r); next unless defined $fd;
      my $b; my $n = sysread($r, $b, 65536);
      if ($fd == 0) {
        if (!$n) { $agent_open = 0; if ($up) { shutdown($up, 1) } else { exit 0 } next }
        $abuf .= $b;
        while ((my $i = index($abuf, "\n")) >= 0) { my $l = substr($abuf, 0, $i + 1, ''); chomp $l; $from_agent->($l) if $l =~ /\S/ }
      } elsif ($up && $r == $up) {
        if (!$n) { $lose->(); next }
        $ubuf .= $b;
        while ($up && (my $i = index($ubuf, "\n")) >= 0) { my $l = substr($ubuf, 0, $i + 1, ''); chomp $l; $from_up->($l) }
      }
    }
  }
}

# Phase 4: a run lives in `run/<run>/` (0700) under a supervisor detached
# from the connection: the agent's output goes to `out` and `err` there, its
# input comes from Hollow through whichever attacher is connected, and an
# `exit` file (the code, written last, by rename) says it ended. A dropped
# connection ends only the attacher: Hollow's leader reconnects with
# `--attach`, and the output goes on from the byte it had.
#
# The attacher: proves the attach secret to the supervisor (which then
# serves the agent's tools through this attach's relay socket and takes
# input from it), says `<nonce> in <bytes of input the run has> <input
# ended> <supervisor pid>`, then streams `out` from `$out_at` to stdout and
# `err` from `$err_at` to stderr (each whole line as `<nonce> e <offset>
# <line> <nonce>` when the leader asked for it), a mark `<nonce> err
# <offset>` after each run of lines, until the run has ended and both are
# drained:
# `<nonce> done <code>`. Its stdin is the leader's frames, passed
# to the supervisor as they come; a bare end of stdin means the connection
# went, not that input ended.
sub attach {
  my ($run, $secret, $relay, $out_at, $err_at, $nonce, $sup, $lines) = @_;
  my $rd = "$root/run/$run";
  # The runner's marks carry this connection's nonce, which only the leader
  # and this process know: text in the run's files can't forge them. For a
  # leader that asked (`$lines`, overnight R2) every mark also ends with it,
  # so one cut short can't pass for a whole one, and `in` says so.
  my $end = $lines ? " $nonce" : '';
  my $mark = sub { return put(\*STDERR, "hollow-remote: $nonce @_$end\n") };
  $SIG{PIPE} = 'IGNORE';
  my ($of, $ef);
  (open($of, '<', "$rd/out") && open($ef, '<', "$rd/err")) or do { $mark->('gone'); exit 2 };
  (defined sysseek($of, $out_at, 0) && defined sysseek($ef, $err_at, 0)) or do { $mark->('gone'); exit 2 };
  my $sup_live = sub { return $sup =~ /\A[1-9]\d*\z/ && kill(0, $sup) };
  my $ctl;
  if (!-e "$rd/exit") {
    $ctl = IO::Socket::UNIX->new(Type => SOCK_STREAM(), Peer => "$rd/ctl.sock");
    # The secret goes only to the run's own supervisor (its pid came from
    # the supervisor itself, not a file), never to a socket someone else
    # put at that path.
    my $peer = peer_pid($ctl);
    my $ok = $ctl && $sup_live->() && "$peer" eq $sup && put($ctl, "HOLLOW-ATTACH 1 $secret $relay\n") ? (read_line($ctl, 10) // '') : '';
    if ($ok =~ /\Aok (\d+) ([01])\z/) { $mark->("in $1 $2 $sup") } else { close $ctl if $ctl; $ctl = undef }
  }
  # Ended (or ending) without a supervisor to take input: nothing more goes in.
  $mark->("in 0 1 $sup") unless $ctl;
  my $sel = IO::Select->new(\*STDIN); $sel->add($ctl) if $ctl;
  my ($ebuf, $epos, $ended, $orphan_at) = ('', $err_at, 0, undef);
  while (1) {
    my ($b, $moved) = ('', 0);
    my $n = sysread($of, $b, 65536);
    if ($n) { put(\*STDOUT, $b) or exit 0; $moved = 1 }
    $n = sysread($ef, $b, 65536);
    if ($n) { $ebuf .= $b; $moved = 1 }
    # With a leader that asks for it (`$lines`, overnight R2), each whole
    # line of `err` goes as a mark of its own, `<nonce> e <its offset in
    # err> <the line> <nonce>`, so the leader counts exactly the bytes it
    # passed on, sees a line cut short or gone missing (the nonce at the end,
    # the offset that must follow on), and a reconnect asks from there: no
    # line shows twice across a drop, and none is lost. An older leader gets
    # the lines bare, as before.
    my $cut = rindex($ebuf, "\n") + 1;
    if ($cut) {
      my $chunk = substr($ebuf, 0, $cut, ''); my $o = '';
      if ($lines) { for my $l (split /(?<=\n)/, $chunk) { $o .= "hollow-remote: $nonce e $epos " . substr($l, 0, -1) . " $nonce\n"; $epos += length $l } }
      else { ($o, $epos) = ($chunk, $epos + $cut) }
      put(\*STDERR, $o . "hollow-remote: $nonce err $epos$end\n") or exit 0;
    }
    if (!$moved) {
      if ($ended) {
        my $code = 70; if (open(my $e, '<', "$rd/exit")) { my $v = <$e> // ''; chomp $v; $code = $v if $v =~ /\A\d{1,3}\z/; close $e }
        # The last line, if it never got its newline (`f`: none counted).
        my $tail = !length $ebuf ? '' : $lines ? "hollow-remote: $nonce f $epos $ebuf $nonce\n" : "$ebuf\n";
        put(\*STDERR, $tail . "hollow-remote: $nonce done $code$end\n");
        exit($code > 255 ? 255 : $code);
      }
      # Once the exit file is there, one more pass drains what came before it.
      if (-e "$rd/exit") { $ended = 1; next }
      if (!$ctl && !$sup_live->()) {
        $orphan_at //= time;
        if (time - $orphan_at > 5) { put(\*STDERR, "hollow-remote: the run's supervisor on this host ended without finishing it\nhollow-remote: $nonce done 70$end\n"); exit 70 }
      }
    }
    for my $r ($sel->can_read($moved ? 0 : 0.05)) {
      my $k = sysread($r, $b, 65536);
      if ($ctl && $r == $ctl) {
        next if $k;
        $sel->remove($ctl); close $ctl; $ctl = undef;
        # Replaced by a newer attacher (the run goes on), unless it ended.
        select(undef, undef, undef, 0.2);
        exit 0 if !-e "$rd/exit" && $sup_live->();
      } else {
        exit 0 unless $k;
        put($ctl, $b) if $ctl;
      }
    }
  }
}

# The run's supervisor (`--run`'s grandchild, in a session of its own, with
# no terminal or connection): starts the agent and stays with it whatever
# happens to the connection. Stop is a TERM (Hollow's collect sends it); 12
# hours without a word from Hollow stops it too.
sub supervise {
  my ($h, $rd, $ready, $secret) = @_;
  my ($token, $relay, $dir, $run) = map { $h->{$_} // '' } qw(token sock dir run);
  open(STDIN, '<', '/dev/null'); open(STDOUT, '>', '/dev/null'); open(STDERR, '>>', "$rd/err");
  my $fail = sub { put($ready, "$_[0]\n"); exit 70 };
  $SIG{PIPE} = 'IGNORE'; $SIG{HUP} = 'IGNORE';
  my $stop = 0; $SIG{TERM} = $SIG{INT} = sub { $stop = 1 };
  # An earlier attempt of this turn (an account switch resumes at once) may
  # still be in the folder: wait for it, up to 30 s, stopping it first when
  # its leader is gone, so the two never overlap.
  my $mark = "$dir.running";
  wait_free($mark) or $fail->("the previous turn is still finishing on this host");
  # Hollow finished the turn (Stop) before this run got here: don't start.
  my $stop_file = "$root/run/" . substr($run, 0, 8) . ".stop";
  -e $stop_file and $fail->("the turn was stopped before it started here");
  if (open(my $m, '>', "$mark.$$")) { print $m "$$\n$run\n"; close $m; rename("$mark.$$", $mark) }
  # A build workspace's vault folder beside the worktree (Phase 3) gets the
  # same marker while the run is live, so a preparation's prune (overnight
  # N5) never takes it from under the run.
  my $fmdir = ref $h->{folder} eq 'HASH' ? $h->{folder}{dir} // '' : '';
  my $fmark = $fmdir =~ m{\A\Q$root\E/folders/[A-Za-z0-9_-][A-Za-z0-9._-]*\z} && "$fmdir.running" ne $mark ? "$fmdir.running" : '';
  if ($fmark && open(my $m, '>', "$fmark.$$")) { print $m "$$\n$run\n"; close $m; rename("$fmark.$$", $fmark) }
  if (open(my $p, '>', "$rd/pid")) { print $p "$$\n"; close $p }
  # The agent's tools reach the relay only through this run's own socket,
  # which admits a process only when it descends from the agent. The token
  # stays in this process: perl is an Apple binary, whose environment ps -E
  # doesn't show other processes (on Linux, no_dump keeps its memory and
  # environment from the account). Attachers prove the
  # attach secret (never the relay token) on the control socket.
  my $um = umask(077);
  my $tsock = "$root/run/$run.tools.sock"; unlink $tsock;
  my $tools = IO::Socket::UNIX->new(Type => SOCK_STREAM(), Local => $tsock, Listen => 64) or $fail->("no tool socket ($!)");
  my $csock = "$rd/ctl.sock"; unlink $csock;
  my $ctl = IO::Socket::UNIX->new(Type => SOCK_STREAM(), Local => $csock, Listen => 8) or $fail->("no control socket ($!)");
  umask($um);
  # This attempt's own scratch folder, inside the turn's.
  my $scratch = "$h->{scratch}/$run"; mkdir($scratch, 0700);
  my %env = %{ $h->{env} || {} };
  $ENV{$_} = $env{$_} for keys %env;
  $ENV{HOLLOW_RELAY_SOCK} = $tsock; delete $ENV{HOLLOW_RELAY_TOKEN};
  $ENV{TMPDIR} = "$scratch/"; $ENV{HOLLOW_SCRATCH_DIR} = $scratch;
  my $tag = "hollow-remote-$run"; $ENV{HOLLOW_REMOTE_RUN} = $tag;
  $ENV{PORT} = $h->{port}; $ENV{PW_PORT} = $h->{port}; $ENV{CARGO_TARGET_DIR} = $h->{target};
  $ENV{HOLLOW_SLOT_REMOTE} = 'off'; $ENV{PATH} = "$root/bin:" . ($ENV{PATH} // '/usr/bin:/bin');
  $ENV{HOLLOW_BUILD_SLOT_DIR} = "/tmp/hollow-build-slots-$<";
  $ENV{HOLLOW_THIS_MAC} = "$root/bin/this-mac.md";
  # Remote runners: the run's own git configuration when GitHub is on.
  run_git_config($scratch, $h->{dir});
  $ENV{CARGO_PROFILE_DEV_DEBUG} //= '0'; $ENV{CARGO_PROFILE_TEST_DEBUG} //= '0';
  my $prog = $h->{program};
  my @argv = ($prog, @{ $h->{args} || [] });
  @argv = ('/usr/bin/perl', "$root/bin/hollow-remote-run", '--probe', @{ $h->{args} || [] }) if $prog eq 'hollow-remote-probe';
  pipe(my $in_r, my $in_w) or $fail->("no pipe for the agent's input");
  my $pid = fork() // $fail->("can't start $prog");
  if (!$pid) {
    setpgrp(0, 0); $SIG{$_} = 'DEFAULT' for qw(PIPE HUP TERM INT);
    # R5b (Q-R33): the prompt and the appended system prompt come in the
    # header and reach the agent as unlinked private files, never argv
    # (`ps` shows it to this account's other processes): the prompt as its
    # stdin, the rest on the
    # descriptors (3 to 99) the header names.
    my $private = sub {
      my $text = shift; $text = '' unless defined $text;
      open(my $fh, '+>', undef) or exit 70; binmode($fh);
      my $o = 0;
      while ($o < length $text) { my $w = syswrite($fh, $text, length($text) - $o, $o); defined $w or exit 70; $o += $w }
      sysseek($fh, 0, 0); return $fh;
    };
    if (defined $h->{stdin_text}) { my $sfh = $private->($h->{stdin_text}); open(STDIN, '<&', $sfh) or exit 70; close $sfh }
    else { open(STDIN, '<&', $in_r) or exit 70 }
    my @held;
    for my $f (@{ $h->{fds} || [] }) {
      my $want = ref $f eq 'HASH' ? ($f->{fd} // '') : '';
      $want =~ /\A\d+\z/ && $want >= 3 && $want < 100 or exit 70;
      my $pfh = $private->($f->{text}); my $src = fileno($pfh);
      # A temporary file is close-on-exec, and dup2 onto another number clears
      # that on the copy; one that landed on its own target number (the lowest
      # free one) is cleared here (F_SETFD is 2, FD_CLOEXEC off).
      # (kept in @held: a handle that goes out of scope closes its descriptor)
      if ($src == $want) { fcntl($pfh, 2, 0) or exit 70; push @held, $pfh }
      else { defined POSIX::dup2($src, $want) or exit 70; close $pfh }
    }
    open(STDOUT, '>>', "$rd/out") or exit 70;
    chdir($h->{cwd}) or do { print STDERR "hollow-remote: no folder $h->{cwd}\n"; exit 70 };
    exec { $argv[0] } @argv or do { print STDERR "hollow-remote: $prog couldn't start ($!)\n"; exit 127 };
  }
  # Both sides set the group, so a stop in the gap still reaches it.
  POSIX::setpgid($pid, $pid);
  close $in_r; $in_w->blocking(0);
  put($ready, "ok $$\n"); close $ready;
  my $own = getpgrp();
  # Everything the agent starts, sampled once a second (pid => group): a
  # tool's command in a group of its own, or one whose parent exits, still
  # ends with the run.
  my %seen;
  my $sample = sub {
    my (%pp, %pg);
    for (split /\n/, proc_table()) { my ($p, $q, $g) = split ' '; next unless defined $g; $pp{$p} = $q; $pg{$p} = $g }
    for my $p (keys %pp) { my $q = $p; for (1 .. 64) { last if !defined $q || $q <= 1; if ($q == $pid) { $seen{$p} = $pg{$p}; last } $q = $pp{$q} } }
    # A process gone since the last sample is forgotten at once, so a pid
    # reused later (another run's) is never signalled.
    delete $seen{$_} for grep { !exists $pp{$_} } keys %seen;
    return \%pp;
  };
  my $descends = sub { my ($peer, $pp) = @_; my $q = $peer; for (1 .. 64) { return 0 if !defined $q || $q <= 1; return 1 if $q == $pid; $q = $pp->{$q} } return 0 };
  my $signal = sub {
    my $sig = shift;
    kill($sig, -$pid);
    my %groups = map { $_ => 1 } grep { $_ && $_ != $own } values %seen;
    kill($sig, -$_) for keys %groups;
    kill($sig, grep { $_ != $$ } keys %seen);
  };
  my $sel = IO::Select->new($ctl, $tools);
  my ($att, $att_pid, $fbuf, $pend, $in_got, $in_end) = (undef, 0, '', '', 0, 0);
  my ($status, $done); my $stopped = 0; my $heard = time; my $touched = 0; my $sampled = 0; my %kids;
  my $hear = sub { $heard = time; if (time - $touched >= 5) { utime(undef, undef, $mark); $touched = time } };
  my $detach = sub { return unless $att; $sel->remove($att); close $att; $att = undef; unlink "$rd/attached" };
  while (!$done) {
    while ((my $k = waitpid(-1, WNOHANG)) > 0) { if ($k == $pid) { $status = $?; $done = 1 } delete $kids{$k} }
    last if $done;
    if (time - $sampled >= 1) { $sample->(); $sampled = time; $stop = 1 if -e $stop_file }
    # The agent's input, written as it takes it; closed once it has ended.
    if (length $pend && $in_w) {
      my $w = syswrite($in_w, $pend);
      if (defined $w) { substr($pend, 0, $w, '') } elsif (!$!{EAGAIN}) { close $in_w; $in_w = undef; $pend = '' }
    }
    if ($in_end && !length $pend && $in_w) { close $in_w; $in_w = undef }
    for my $r ($sel->can_read(length $pend && $in_w ? 0.05 : 0.5)) {
      if ($r == $ctl) {
        my $conn = $ctl->accept or next;
        my ($magic, $v, $t, $path) = split / /, (read_line($conn, 2) // ''), 4;
        if (!(defined $path && $magic eq 'HOLLOW-ATTACH' && $v eq '1' && same($t, $secret) && relay_ok($run, $path))) { close $conn; next }
        my $peer = peer_pid($conn);
        # A new attach replaces the last: that attacher goes, and so do the
        # tool connections made through its relay (their agents reconnect).
        if ($att) { $detach->(); kill('TERM', $att_pid) if $att_pid > 1 && cmd_of($att_pid) =~ /hollow-remote-run/ }
        kill('TERM', keys %kids);
        ($att, $att_pid, $fbuf, $relay) = ($conn, $peer, '', $path);
        $sel->add($att);
        if (open(my $a, '>', "$rd/attached")) { close $a }
        put($att, "ok $in_got $in_end\n");
        $touched = 0; $hear->();
      } elsif ($r == $tools) {
        my $conn = $tools->accept or next;
        my $peer = peer_pid($conn);
        if ($peer > 0 && $descends->($peer, $sample->())) {
          my $k = fork();
          if (defined $k && !$k) { $SIG{$_} = 'DEFAULT' for qw(TERM INT); close $tools; close $ctl; close $att if $att; close $in_w if $in_w; splice_tool($conn, $relay, $token) }
          $kids{$k} = 1 if $k;
        }
        close $conn;
      } elsif ($att && $r == $att) {
        my $b; my $n = sysread($att, $b, 65536);
        if (!$n) { $detach->(); next }
        $fbuf .= $b; $hear->();
        # Frames: `H` (heartbeat), `E` (input ended), `D <length>` and the bytes.
        while (length $fbuf) {
          if ($fbuf =~ /\A([HE])\n/) { $in_end = 1 if $1 eq 'E'; substr($fbuf, 0, 2, ''); next }
          if ($fbuf =~ /\AD (\d{1,9})\n/) {
            my ($len, $hl) = ($1, length($1) + 3);
            last if length($fbuf) < $hl + $len;
            $pend .= substr($fbuf, $hl, $len) if $in_w && !$in_end; $in_got += $len;
            substr($fbuf, 0, $hl + $len, ''); next;
          }
          last if $fbuf =~ /\A(?:[HE]|D(?: \d{0,9})?)\z/;
          $detach->(); last;
        }
      }
    }
    if ($stop && !$stopped) { $stopped = time; $signal->('TERM') }
    if ($stopped && time - $stopped >= 10) { $signal->('KILL') }
    if (!$stop && time - $heard > 12 * 3600) { say_err("no word from Hollow for 12 hours: the run stops"); $stop = 1 }
  }
  # What the agent left running ends with it: everything sampled, and any
  # process still carrying this run's tag (ps -E shows it for non-Apple
  # programs: node, cargo, claude; /proc on Linux, `tagged`).
  $sample->();
  my %tagged = tagged("HOLLOW_REMOTE_RUN=$tag");
  $seen{$_} = $tagged{$_} for keys %tagged;
  $signal->('TERM');
  for (1 .. 20) { last unless grep { kill(0, $_) } keys %seen; select(undef, undef, undef, 0.25) }
  $signal->('KILL');
  kill('TERM', keys %kids);
  my $code = $status & 127 ? 128 + ($status & 127) : $status >> 8;
  $code = 143 if $stopped;
  # The result: the branch head the agent left, and that plus everything it
  # didn't commit. The marker goes only once both are written, so the Mac
  # collects after they are there.
  # A vault workspace's folder (Phase 3): its files as one commit on top of
  # the snapshot that was sent (the snapshot itself when nothing changed).
  my $folder_mark_kept = '';
  if (ref $h->{folder} eq 'HASH') {
    my $f = $h->{folder};
    my ($fdir, $frepo, $base, $fref) = map { $f->{$_} // '' } qw(dir repo base result_ref);
    if (index($fdir, "$root/folders/") == 0 && index($frepo, "$root/repos/") == 0 && $base =~ /\A[0-9a-f]{40,64}\z/ && $fref =~ m{\Arefs/hollow-session/[0-9a-f]+-result\z} && -d $fdir) {
      my $fidx = "$root/scratch/.folder-index.$$"; my $fex = "$root/scratch/.folder-exclude.$$";
      if (open(my $o, '>', $fex)) { print $o ($f->{exclude} // ''); close $o }
      local $ENV{GIT_DIR} = $frepo; local $ENV{GIT_WORK_TREE} = $fdir; local $ENV{GIT_INDEX_FILE} = $fidx;
      local $ENV{GIT_AUTHOR_NAME} = 'hollow-folder'; local $ENV{GIT_AUTHOR_EMAIL} = 'hollow-folder@localhost';
      local $ENV{GIT_COMMITTER_NAME} = 'hollow-folder'; local $ENV{GIT_COMMITTER_EMAIL} = 'hollow-folder@localhost';
      # The snapshot itself when nothing changed. When git failed, no result
      # and the marker stays (round 2 review): the next preparation keeps
      # the copy (`salvage_folder`) instead of taking it as unchanged.
      my $fres = folder_commit($fdir, $fex, $base, 'hollow-folder: the turn on the runner');
      if (defined $fres) {
        $fres = $base if $fres eq '';
        quiet('git', '-C', $fdir, '-c', 'core.autocrlf=false', '-c', 'core.attributesFile=/dev/null', 'update-ref', $fref, $fres)
          and mark_done($fdir, $run);
        say_err("folder result $fres");
      } else {
        $folder_mark_kept = "$fdir.running";
        say_err("couldn't make the folder's result; the next preparation keeps the copy");
      }
      unlink $fidx, $fex;
    }
  }
  # A worktree's result (none for a vault workspace's own folder).
  my $headc = $h->{head_ref} ? capture('git', '-C', $dir, 'rev-parse', 'HEAD') : '';
  my $tmpidx = "$root/scratch/.index.$$";
  my $result = $headc;
  if ($headc =~ /\A[0-9a-f]+\z/) {
    local $ENV{GIT_INDEX_FILE} = $tmpidx;
    quiet('git', '-C', $dir, 'read-tree', 'HEAD');
    quiet('git', '-C', $dir, 'add', '-A', '.');
    # A chat's attachments ignore themselves (a .gitignore of /*), so they
    # stay out of the result: the Mac compares and applies without them (C5
    # third review), and a result kept on a branch never holds them.
    my $tree = capture('git', '-C', $dir, 'write-tree');
    if ($tree ne '' && $tree ne capture('git', '-C', $dir, 'rev-parse', 'HEAD^{tree}')) {
      local $ENV{GIT_AUTHOR_NAME} = 'hollow-session'; local $ENV{GIT_AUTHOR_EMAIL} = 'hollow-session@localhost';
      local $ENV{GIT_COMMITTER_NAME} = 'hollow-session'; local $ENV{GIT_COMMITTER_EMAIL} = 'hollow-session@localhost';
      my $r = capture('git', '-C', $dir, '-c', 'commit.gpgSign=false', 'commit-tree', $tree, '-p', $headc, '-m', "hollow-session: uncommitted changes on $headc");
      $result = $r if $r =~ /\A[0-9a-f]+\z/;
    }
    unlink $tmpidx;
    quiet('git', '-C', $dir, 'update-ref', $h->{head_ref}, $headc);
    # Once its result is there (or brought back and dropped), a marker this
    # run might still leave behind must not have it kept a second time; a
    # later attempt's run in the same copy still is.
    quiet('git', '-C', $dir, 'update-ref', $h->{result_ref}, $result) and mark_done($dir, $run);
    say_err("result $result head $headc");
  }
  quiet('rm', '-rf', $scratch);
  unlink $csock, "$rd/attached";
  if (opendir(my $rdh, "$root/run")) { for my $f (readdir $rdh) { unlink "$root/run/$f" if index($f, "$run.") == 0 && $f =~ /\.sock\z/ } closedir $rdh }
  for my $mk ($mark, $fmark || ()) { next if $mk eq $folder_mark_kept; if (open(my $m, '<', $mk)) { local $/ = "\n"; my $p = <$m> // ''; close $m; chomp $p; unlink $mk if $p eq $$ } }
  # Last, by rename: an attacher that sees it has everything before it.
  if (open(my $e, '>', "$rd/exit.tmp")) { print $e "$code\n"; close $e; rename("$rd/exit.tmp", "$rd/exit") }
  exit $code;
}

if ($mode eq '--attach') {
  # `--attach <run> <stdout offset> <err offset> <relay socket> <supervisor
  # pid> [1]`, with `<attach secret> <nonce>` as the first line of stdin:
  # Hollow's leader reconnecting (`1`: it reads `err` lines as marks).
  my $first = ''; my $c;
  while (sysread(STDIN, $c, 1)) { last if $c eq "\n"; $first .= $c; if (length($first) > 200) { say_err("bad attach line"); exit 2 } }
  my ($secret, $nonce) = split / /, $first, 2;
  (defined $nonce && $nonce =~ /\A[0-9a-f]{8,64}\z/) or do { say_err("bad attach line"); exit 2 };
  my ($run, $out_at, $err_at, $relay, $sup, $lines) = @ARGV;
  (name_ok($run) && ($out_at // '') =~ /\A\d+\z/ && ($err_at // '') =~ /\A\d+\z/ && ($sup // '') =~ /\A\d+\z/) or do { say_err("usage: --attach <run> <out> <err> <relay> <pid>"); exit 2 };
  -f "$root/run/$run/out" or do { put(\*STDERR, "hollow-remote: $nonce gone" . (($lines // '') eq '1' ? " $nonce" : '') . "\n"); exit 2 };
  relay_ok($run, $relay) or do { say_err("bad relay socket"); exit 2 };
  attach($run, $secret, $relay, $out_at, $err_at, $nonce, $sup, ($lines // '') eq '1' ? 1 : 0);
}

if ($mode eq '--run') {
  $SIG{PIPE} = 'IGNORE';
  # The header: one JSON line, read a byte at a time so the frames after it
  # stay untouched; decoded without UTF-8 decoding, so its strings stay the
  # bytes they were for the environment and the command line.
  my $line = ''; my $c;
  while (sysread(STDIN, $c, 1)) { last if $c eq "\n"; $line .= $c; if (length($line) > 16_000_000) { say_err("header too long"); exit 2 } }
  my $h = eval { JSON::PP->new->decode($line) } or do { say_err("no run header"); exit 2 };
  $line = '';
  # Then `<attach secret> <nonce>`, from the leader: the secret proves an
  # attacher to the supervisor (the relay token never goes there), the nonce
  # marks this connection's lines.
  while (sysread(STDIN, $c, 1)) { last if $c eq "\n"; $line .= $c; if (length($line) > 200) { say_err("bad attach line"); exit 2 } }
  my ($secret, $nonce) = split / /, $line, 2; $line = '';
  (defined $nonce && length($secret) >= 16 && $nonce =~ /\A[0-9a-f]{8,64}\z/) or do { say_err("bad attach line"); exit 2 };
  my ($token, $relay, $run) = map { $h->{$_} // '' } qw(token sock run);
  name_ok($run) or do { say_err("bad run id"); exit 2 };
  length($token) >= 16 or do { say_err("no relay token"); exit 2 };
  relay_ok($run, $relay) or do { say_err("bad relay socket"); exit 2 };
  ($h->{program} // '') =~ /\A(claude|codex|hollow-remote-probe)\z/ or do { say_err("not an agent: " . ($h->{program} // '')); exit 2 };
  my $rd = "$root/run/$run";
  mkdir($rd, 0700) or do { say_err("can't make $rd ($!)"); exit 70 };
  for my $f ('out', 'err') { open(my $o, '>', "$rd/$f") or do { say_err("can't write $rd/$f ($!)"); exit 70 }; close $o }
  # The supervisor: a grandchild in a session of its own, so nothing that
  # happens to this connection reaches it. It says `ok` once the agent runs.
  pipe(my $ready_r, my $ready_w) or exit 70;
  my $kid = fork() // do { say_err("can't start the run's supervisor"); exit 70 };
  if (!$kid) {
    close $ready_r; POSIX::setsid();
    my $sup = fork();
    exit 0 if !defined $sup || $sup;
    supervise($h, $rd, $ready_w, $secret);
  }
  close $ready_w; waitpid($kid, 0);
  my $said = do { local $/; <$ready_r> } // ''; close $ready_r;
  my ($sup) = $said =~ /\Aok (\d+)\n\z/;
  if (!$sup) { $said =~ s/\s+\z//; say_err($said ne '' ? $said : "the run's supervisor didn't start"); exit($said =~ /still finishing/ ? 75 : 70) }
  my $lines = $h->{err_lines} ? 1 : 0;
  $h = undef; $token = '';
  attach($run, $secret, $relay, 0, 0, $nonce, $sup, $lines);
}
# A stand-in agent for checking the runner without an agent sign-in (Hollow's
# tests and the real-runner checks): each argument is an action, run in
# order, reporting on stdout. It can do nothing an agent can't.
if ($mode eq '--probe') {
  $| = 1;
  for my $a (@ARGV) {
    my ($what, @rest) = split /:/, $a, 3;
    if ($what eq 'where') {
      print encode_json({ host => host_name(), user => scalar(getpwuid($<)), cwd => capture('pwd', '-P'),
        token_len => length($ENV{CLAUDE_CODE_OAUTH_TOKEN} // ''), relay => ($ENV{HOLLOW_RELAY_SOCK} ? 1 : 0),
        port => $ENV{PORT} // '', tmpdir => $ENV{TMPDIR} // '', target => $ENV{CARGO_TARGET_DIR} // '',
        local_only => join(',', grep { defined $ENV{$_} } qw(HOLLOW_RUN_TAG HOLLOW_BOUND_RUN_IDS HOLLOW_REMOTE_HEADER)) }) . "\n";
    } elsif ($what eq 'bridge') {
      my ($name, $text) = @rest;
      pipe(my $r, my $w); pipe(my $r2, my $w2);
      my $kid = fork();
      if (!$kid) { open(STDIN, '<&', $r); open(STDOUT, '>&', $w2); close $w; close $r2; exec('/usr/bin/perl', "$root/bin/hollow-remote-run", '--bridge', $name) or exit 127 }
      close $r; close $w2; print $w "$text\n"; close $w;
      local $/; my $got = <$r2> // ''; waitpid($kid, 0); chomp $got;
      print "bridge $name: $got\n";
    } elsif ($what eq 'write') { my ($file, $text) = @rest; open(my $o, '>', $file) or die; print $o "$text\n"; close $o; print "wrote $file\n" }
    elsif ($what eq 'rm') { unlink $rest[0]; print "removed $rest[0]\n" }
    elsif ($what eq 'commit') { my ($file, $msg) = @rest; quiet('git', 'add', '--', $file); quiet('git', '-c', 'user.name=probe', '-c', 'user.email=probe@localhost', '-c', 'commit.gpgSign=false', 'commit', '-q', '-m', $msg // 'probe') and print "committed $file\n" }
    elsif ($what eq 'ident') { print 'ident: ', join(' | ', map { my $i = capture('git', 'var', $_); $i =~ s/ \d+ [-+]\d{4}\z//; $i } qw(GIT_AUTHOR_IDENT GIT_COMMITTER_IDENT)), "\n" }
    elsif ($what eq 'cat') { open(my $i, '<', $rest[0]) or do { print "no $rest[0]\n"; next }; my $t = <$i> // ''; chomp $t; print "$rest[0]: $t\n" }
    elsif ($what eq 'pidfile') { open(my $o, '>', $rest[0]) or die; print $o "$$\n"; close $o }
    elsif ($what eq 'group') { my $k = fork(); if (defined $k && !$k) { setpgrp(0, 0); open(my $o, '>', $rest[0]) or exit 1; print $o "$$\n"; close $o; open(STDOUT, '>', '/dev/null'); exec('/bin/sleep', '120') or exit 127 } print "grouped\n" }
    elsif ($what eq 'say') { print join(':', @rest), "\n" }
    # Remote runners: what git's credential helper hands out (its length and
    # hash, never the value), and a shell command as the agent runs one.
    elsif ($what eq 'gitcred') {
      require Digest::SHA; pipe(my $r, my $w); pipe(my $r2, my $w2);
      my $kid = fork();
      if (!$kid) { open(STDIN, '<&', $r); open(STDOUT, '>&', $w2); open(STDERR, '>&', $w2); close $w; close $r2; exec('git', 'credential', 'fill') or exit 127 }
      close $r; close $w2; print $w "protocol=https\nhost=github.com\n\n"; close $w;
      local $/; my $got = <$r2> // ''; waitpid($kid, 0);
      my ($pw) = $got =~ /^password=(.*)$/m;
      if (defined $pw) { print 'gitcred: len=', length($pw), ' sha=', substr(Digest::SHA::sha256_hex($pw), 0, 16), "\n" }
      else { $got =~ s/\s+/ /g; print "gitcred: none ($got)\n" }
    }
    elsif ($what eq 'sh') { my $out = capture('/bin/sh', '-c', join(':', @rest) . ' 2>&1; echo "rc=$?"'); $out =~ s/\n/ | /g; print "sh: $out\n" }
    elsif ($what eq 'err') { print STDERR join(':', @rest), "\n" }
    elsif ($what eq 'hold') {
      my ($name, $file) = @rest; my $k = fork();
      if (defined $k && !$k) { open(STDOUT, '>', '/dev/null'); my $s = IO::Socket::UNIX->new(Type => SOCK_STREAM(), Peer => $ENV{HOLLOW_RELAY_SOCK}) or exit 1; syswrite($s, "$name\n"); my $b; 1 while sysread($s, $b, 4096); open(my $o, '>', $file) or exit 1; print $o "closed\n"; close $o; exit 0 }
      print "holding $name\n";
    }
    elsif ($what eq 'wait') { for (1 .. 100) { last if -e $rest[0]; select(undef, undef, undef, 0.1) } print -e $rest[0] ? "saw $rest[0]\n" : "no $rest[0]\n" }
    elsif ($what eq 'sleep') { sleep($rest[0] // 1) }
    elsif ($what eq 'stdin') { my $l = <STDIN> // ''; chomp $l; print "stdin: $l\n" }
    elsif ($what eq 'exit') { exit($rest[0] // 0) }
  }
  exit 0;
}
say_err("usage: hollow-remote-run --prepare … | --run | --attach … | --bridge <name>");
exit 2;
"#;

/// The run's local leader, `/usr/bin/perl -e` this `<config file|-> <relay
/// socket> <runner socket base> <run> -- <ssh> <options…> <host>`, with
/// `HOLLOW_RELAY_TOKEN` and `HOLLOW_REMOTE_HEADER` in its environment (deleted
/// at once, so nothing it starts inherits them). It listens on the relay
/// socket (0600, in a 0700 folder) and serves each relay connection in a
/// child: the first line must be `HOLLOW-RELAY 1 <token> <name>`, and that
/// server starts from the config file with the connection as its stdin and
/// stdout. It starts ssh (`-R <base>.<n>.sock:<relay>`, a new name each
/// connection) running `hollow-remote-run --run`, writes the header line and
/// then `<attach secret> <nonce>` (the secret proves an attacher to the
/// run's supervisor; the nonce, new each connection, marks the runner's
/// lines), then frames its own stdin (`D <length>` and the bytes; `E` when
/// it ends; `H`, a heartbeat, every 15 s). It passes ssh's stdout on,
/// counting the bytes, and ssh's stderr minus the runner's marks
/// (`<nonce> in|err|done|gone`), with each line of the run's own stderr
/// unwrapped from its `<nonce> e <offset> <line> <nonce>` mark and counted,
/// so a reconnect asks from exactly the byte after it (stdout and stderr
/// each arrive once across a drop, overnight R2); a damaged mark ends the
/// connection and asks again. When ssh ends without `done` after the run
/// started there (Phase 4: a dropped connection, this Mac asleep), it says
/// so once and reconnects with backoff (1 s doubling to 30 s, for 12 hours
/// at most) with `--attach <run> <stdout bytes> <err offset> <socket>
/// <supervisor pid> 1`, then sends the input the run hasn't had. Exits with the run's code; everything it starts is in its
/// process group, so Stop's group kill ends all of it here (the run there
/// ends when `collect` stops it).
pub(crate) const LEADER_PL: &str = r#"
use strict; use warnings;
use IO::Socket::UNIX; use IO::Select; use POSIX qw(:sys_wait_h); use JSON::PP;
my ($config, $sock, $rbase, $run, $sep, @ssh) = @ARGV;
(defined $sep && $sep eq '--' && @ssh >= 2 && ($run // '') =~ /\A[0-9a-f]{1,32}\z/) or do { print STDERR "hollow-remote: usage\n"; exit 2 };
my $host = pop @ssh;
my $token = delete $ENV{HOLLOW_RELAY_TOKEN} // ''; my $header = delete $ENV{HOLLOW_REMOTE_HEADER} // '';
length($token) >= 16 or do { print STDERR "hollow-remote: no relay token\n"; exit 2 };
$SIG{PIPE} = 'IGNORE';
my %servers;
if ($config ne '-') {
  open(my $fh, '<', $config) or do { print STDERR "hollow-remote: can't read this run's tool configuration ($!)\n"; exit 70 };
  # Raw bytes in, raw bytes out: values stay UTF-8 for the environment.
  local $/; my $cfg = eval { JSON::PP->new->decode(<$fh>) } || {}; close $fh;
  %servers = %{ $cfg->{mcpServers} || {} };
}
unlink $sock; my $mask = umask(077);
my $srv = IO::Socket::UNIX->new(Type => SOCK_STREAM(), Local => $sock, Listen => 64) or do { print STDERR "hollow-remote: can't open the tool relay ($!)\n"; exit 70 };
umask($mask); chmod 0600, $sock;
open(my $null, '<', '/dev/null');
sub put { my ($fh, $b) = @_; my $o = 0; while ($o < length $b) { my $w = syswrite($fh, $b, length($b) - $o, $o); return 0 unless $w; $o += $w } return 1 }
sub say_err { put(\*STDERR, "hollow-remote: @_\n") }
sub sq { my $s = shift; $s =~ s/'/'\\''/g; return "'$s'" }
sub random_hex { my $n = shift; open(my $u, '<', '/dev/urandom') or die "no randomness\n"; my $raw = ''; read($u, $raw, $n) == $n or die "no randomness\n"; close $u; return unpack('H*', $raw) }
# Proves an attacher to the run's supervisor there. Unlike the relay token,
# it never goes to a relay socket. A leader picking up a run an earlier
# Hollow started (overnight R1, HOLLOW_REMOTE_ATTACH) is given that run's.
my $given = delete $ENV{HOLLOW_ATTACH_SECRET} // '';
my $secret = $given =~ /\A[0-9a-f]{16,64}\z/ ? $given : random_hex(16);
# Where this run's facts are kept for a restarted Hollow (overnight R1).
my $state = delete $ENV{HOLLOW_REMOTE_STATE} // '';
my $attach = delete $ENV{HOLLOW_REMOTE_ATTACH} // '';
# Remote runners (2026-11-03): the run's credentials, handed out one request
# at a time (`hollow-credential-github|gcloud` on the relay), never sent
# ahead and never in what this leader starts: the GitHub token, or the argv
# that mints a Google Cloud access token on this Mac; else why not.
my %cred = map { my $v = delete $ENV{"HOLLOW_CRED_$_"} // ''; $v =~ s/[\r\n]+/ /g; ($_ => $v) } qw(GITHUB GITHUB_WHY GCLOUD_MINT GCLOUD_WHY GCLOUD_LEVEL);
# The access level goes back with each Google Cloud token (the runner's guard
# uses it, not a variable of the run's).
$cred{GCLOUD_LEVEL} = 'limited' unless $cred{GCLOUD_LEVEL} =~ /\A(limited|full|service-account)\z/;
sub cred_line {
  my $what = shift;
  if ($what eq 'github') {
    return "ok $cred{GITHUB}" if $cred{GITHUB} =~ /\A[A-Za-z0-9_]{10,255}\z/;
    return 'no ' . ($cred{GITHUB_WHY} ne '' ? $cred{GITHUB_WHY} : "GitHub isn't turned on for this workspace's runner turns (Runs on, On the runner, in Hollow on the Mac)");
  }
  my $argv = $cred{GCLOUD_MINT} ne '' ? eval { JSON::PP->new->decode($cred{GCLOUD_MINT}) } : undef;
  return 'no ' . ($cred{GCLOUD_WHY} ne '' ? $cred{GCLOUD_WHY} : "Google Cloud isn't turned on for this workspace's runner turns (Runs on, On the runner, in Hollow on the Mac)") unless ref $argv eq 'ARRAY' && @$argv;
  pipe(my $r, my $w) or return 'no the Mac that started this run couldn\'t open a pipe';
  my $kid = fork() // return 'no the Mac that started this run couldn\'t start gcloud';
  if (!$kid) { close $r; open(STDIN, '<', '/dev/null'); open(STDOUT, '>&', $w); open(STDERR, '>', '/dev/null'); exec { $argv->[0] } @$argv or exit 127 }
  close $w;
  my $out = '';
  my $ok = eval { local $SIG{ALRM} = sub { die "late\n" }; alarm 60; local $/; $out = <$r> // ''; alarm 0; 1 };
  alarm 0; close $r;
  if (!$ok) { kill('KILL', $kid) }
  waitpid($kid, 0);
  return "ok $1 $cred{GCLOUD_LEVEL}" if $ok && $? == 0 && $out =~ /\A([\x21-\x7e]{20,4000})\s*\z/;
  return 'no ' . ($ok ? "gcloud on the Mac that started this run couldn't make an access token (sign in there with gcloud auth login" . (grep({ /\A--impersonate-service-account=/ } @$argv) ? ', and check it may impersonate the service account' : '') . ')' : "gcloud on the Mac that started this run didn't answer within a minute");
}
sub serve {
  my $conn = shift; my $line = ''; my $c;
  local $SIG{ALRM} = sub { exit 0 }; alarm 10;
  while (sysread($conn, $c, 1)) { last if $c eq "\n"; $line .= $c; exit 0 if length($line) > 4096 }
  alarm 0;
  my ($magic, $v, $t, $name) = split / /, $line, 4;
  exit 0 unless defined $name && $magic eq 'HOLLOW-RELAY' && $v eq '1' && length($t) == length($token);
  my $diff = 0; $diff |= ord(substr($t, $_, 1)) ^ ord(substr($token, $_, 1)) for 0 .. length($token) - 1;
  exit 0 if $diff;
  if ($name =~ /\Ahollow-credential-(github|gcloud)\z/) { put($conn, cred_line($1) . "\n"); exit 0 }
  my $s = $servers{$name} or exit 0;
  my $cmd = $s->{command} // exit 0;
  my %env = %{ $s->{env} || {} }; $ENV{$_} = $env{$_} for keys %env;
  open(STDIN, '<&', $conn) or exit 0; open(STDOUT, '>&', $conn) or exit 0; open(STDERR, '>', '/dev/null'); close $conn;
  exec { $cmd } $cmd, @{ $s->{args} || [] } or exit 127;
}
# The turn's input (a steerable turn's messages), kept whole: after a
# reconnect the runner says how much it has and gets exactly the rest.
my ($input, $input_end, $stdin_open) = ('', 0, 1);
# Passed on so far: stdout bytes, and the runner's err file to the end of the
# last line passed on (to its last mark, from a runner that sends lines bare).
my ($got, $err_at, $sup) = (0, 0, 0);
my ($n, $ever, $done, $gone, $dropped, $retry_at, $backoff) = (0, 0, undef, 0, undef, undef, 1);
# Damaged lines this run (overnight R2), and whether stderr now comes bare.
my ($damaged, $bare) = (0, 0);
my (%c, %kids);
# Overnight R1: `<stdout bytes> <err offset> <supervisor pid> <first socket
# number>` of a run an earlier Hollow left going: the first connection
# attaches from there. This turn's own first message (the Continue that
# asked for it) isn't the run's, so it is dropped; what comes after (a
# steer) and the input's end go to the run, after the input it already had.
my ($reattach, $skip, $ibase) = (0, 0, undef);
# The run is known to have started there, so a failed first connection
# reconnects like any later one.
if ($attach =~ /\A(\d+) (\d+) (\d+) (\d{1,6})\z/ && $4 > 0) { ($got, $err_at, $sup, $n, $reattach, $skip, $ever) = ($1, $2, $3, $4, 1, 1, 1); $header = '' }
elsif ($attach ne '') { print STDERR "hollow-remote: usage\n"; exit 2 }
length($header) || $reattach or do { print STDERR "hollow-remote: no run header\n"; exit 2 };
# A new attempt's leader names its own run at once (its supervisor pid comes
# with `in`), so the turn's file never pairs the last run with this one's
# byte count.
sub save_state {
  return unless $state =~ m{\A/};
  my $mask = umask(077);
  if (open(my $o, '>', "$state.$$")) { print $o "run $run\ntoken $token\nsecret $secret\nsup $sup\nerr $err_at\n"; close $o; rename("$state.$$", $state) }
  umask($mask);
}
save_state();
sub quit {
  my $code = shift;
  kill('TERM', $c{pid}) if $c{pid};
  close $srv; unlink $sock;
  my @k = keys %kids; kill('TERM', @k);
  for (1 .. 20) { last unless grep { kill(0, $_) } @k; select(undef, undef, undef, 0.1); 1 while waitpid(-1, WNOHANG) > 0 }
  kill('KILL', grep { kill(0, $_) } @k);
  exit($code > 255 ? 255 : $code);
}
sub connect_runner {
  my $relay = "$rbase.$n.sock";
  # exec: no shell stays between ssh and the run holding its stdin (a
  # Linux runner: a dumpable shell's /proc/<pid>/fd/0 would be open to the
  # account). zsh execs a lone command anyway.
  my $run_cmd = 'exec /usr/bin/perl "$HOME/hollow-remote/bin/hollow-remote-run"';
  my $cmd = $n == 0 ? "$run_cmd --run" : "$run_cmd --attach " . join(' ', $run, $got, $err_at, sq($relay), $sup, $bare ? () : 1);
  my @cmd = (@ssh, '-o', 'ExitOnForwardFailure=yes', '-R', "$relay:$sock", $host, $cmd);
  my ($ir, $iw, $or, $ow, $er, $ew);
  (pipe($ir, $iw) && pipe($or, $ow) && pipe($er, $ew)) or return 0;
  my $pid = fork(); return 0 unless defined $pid;
  if (!$pid) {
    open(STDIN, '<&', $ir) or exit 127; open(STDOUT, '>&', $ow) or exit 127; open(STDERR, '>&', $ew) or exit 127;
    exec { $cmd[0] } @cmd or do { print STDERR "hollow-remote: ssh couldn't start ($!)\n"; exit 127 };
  }
  close $ir; close $ow; close $ew; $iw->blocking(0);
  # Each connection's marks carry a nonce of its own: text from the run's
  # files can't pass for them.
  my $nonce = random_hex(8);
  %c = (pid => $pid, in => $iw, out => $or, err => $er, wbuf => ($n == 0 ? "$header\n" : '') . "$secret $nonce\n", nonce => $nonce, sent => 0, ready => 0, eoff => 0, lbuf => '', beat => time, seen => 0);
  $header = '' if $n == 0;
  $n++;
  return 1;
}
sub err_line {
  my $l = shift;
  # After a broken count nothing more from this connection is passed on.
  return if $c{bad};
  my $nonce = $c{nonce};
  my $p = "hollow-remote: $nonce ";
  # Not one of the runner's lines (ssh's own words, the runner's messages):
  # passed on as they are.
  if (index($l, $nonce) < 0) { put(\*STDERR, "$l\n"); return }
  # With a runner that ends its marks with the nonce too (`in` says so,
  # overnight R2), only a whole one counts: one cut short, ssh's words
  # joined on or landed inside it, never passes for one.
  my $m = index($l, $p) == 0 ? substr($l, length $p) : undef;
  my $whole = defined $m && $m =~ s/ \Q$nonce\E\z//;
  # Any whole mark says the format, so a damaged `in` doesn't hide it.
  $c{lines} = 1 if $whole;
  if (defined $m && !$c{ready} && $m =~ /\Ain (\d+) ([01]) (\d+)\z/) {
    # A reattached run's input before this leader's counts as had.
    $ibase //= $reattach ? $1 : 0;
    my $had = $1 - $ibase; $had = 0 if $had < 0;
    ($c{sent}, $c{eoff}, $c{ready}) = ($had > length($input) ? length($input) : $had, $2, 1);
    $sup = $3 if $3 > 1;
    save_state();
    say_err("reconnected to $host; the run went on there") if $dropped;
    ($ever, $dropped, $backoff) = (1, undef, 1);
    return;
  }
  if (defined $m && ($whole || !$c{lines})) {
    # One line of the run's `err`, starting where the last one ended:
    # passed on and counted, so a reconnect asks from the byte after it
    # and no line shows twice or goes missing (`f`: the last line, with no
    # newline). From a runner that sends lines bare, `err` marks move it.
    if ($whole && $m =~ /\A([ef]) (\d{1,15}) (.*)\z/s && $2 == $err_at) { put(\*STDERR, "$3\n"); $err_at += length($3) + ($1 eq 'e' ? 1 : 0); return }
    if ($m =~ /\Aerr (\d+)\z/) { if (!$whole) { $err_at = $1; save_state(); return } if ($1 == $err_at) { save_state(); return } }
    elsif ($m =~ /\Adone (\d+)\z/) { $done = $1; return }
    elsif ($m eq 'gone') { $gone = 1; return }
  }
  # One of the runner's lines that isn't whole, or a line of `err` that
  # never arrived before this one: the count can't go on from here, so this
  # connection ends and a reconnect asks again from the end of the last line
  # passed on. A damaged `in` on a reconnect asks again the same way (the
  # input waits for it); on the first connection, or from a runner that
  # sends lines bare, it is only left out.
  recount() if $c{ready} ? $c{lines} : $ever;
}
# Five damaged lines on live connections in one run: ssh keeps writing into
# the stream, so the run's stderr comes bare from then on (lines may show
# twice across a drop, as before R2) rather than reconnecting for ever. ssh
# gets a second to end by itself first: a line cut by a drop (ssh's last
# words joined on as it exits) is a drop, not damage.
sub recount {
  # Past ten, a runner that keeps sending one is only left out.
  return if $damaged >= 10;
  $c{bad} = 1;
  $c{term_at} //= time + 1 if $c{pid};
}
sub end_damaged {
  return unless $c{pid} && $c{term_at} && time > $c{term_at};
  $c{damaged} = 1; $c{term_at} = 0; $damaged++; $bare = 1 if $damaged >= 5;
  kill('TERM', $c{pid});
}
connect_runner() or do { say_err("ssh couldn't start"); exit 70 };
while (1) {
  my $rs = IO::Select->new($srv); $rs->add(\*STDIN) if $stdin_open;
  $rs->add($c{out}) if $c{out}; $rs->add($c{err}) if $c{err};
  my $ws = IO::Select->new; $ws->add($c{in}) if $c{in} && length $c{wbuf};
  my ($r, $w) = IO::Select->select($rs, $ws->count ? $ws : undef, undef, 0.2);
  for my $fh (@{ $r || [] }) {
    my $b;
    if ($fh == $srv) {
      my $conn = $srv->accept or next;
      my $pid = fork();
      if (!defined $pid) { close $conn; next }
      if (!$pid) { close $srv; serve($conn); exit 0 }
      $kids{$pid} = 1; close $conn;
    } elsif (fileno($fh) == 0) {
      my $k = sysread(STDIN, $b, 65536);
      if ($k && $skip) { my $i = index($b, "\n"); if ($i < 0) { $b = '' } else { $b = substr($b, $i + 1); $skip = 0 } }
      if ($k) { $input .= $b } else { ($stdin_open, $input_end) = (0, 1); open(STDIN, '<&', $null) }
    } elsif ($c{out} && $fh == $c{out}) {
      my $k = sysread($fh, $b, 65536);
      if ($k) { put(\*STDOUT, $b) or quit(255); $got += $k; $c{seen} = time } else { close $fh; delete $c{out} }
    } elsif ($c{err} && $fh == $c{err}) {
      my $k = sysread($fh, $b, 65536);
      if ($k) { $c{seen} = time; $c{lbuf} .= $b; while ((my $i = index($c{lbuf}, "\n")) >= 0) { my $l = substr($c{lbuf}, 0, $i + 1, ''); chomp $l; err_line($l) } }
      else { close $fh; delete $c{err} }
    }
  }
  if ($c{in} && $c{ready}) {
    if (length($input) > $c{sent}) { my $part = substr($input, $c{sent}); $c{wbuf} .= 'D ' . length($part) . "\n" . $part; $c{sent} = length $input }
    if ($input_end && !$c{eoff}) { $c{wbuf} .= "E\n"; $c{eoff} = 1 }
  }
  if ($c{in} && time - $c{beat} >= 15) { $c{wbuf} .= "H\n"; $c{beat} = time }
  if ($c{in} && length $c{wbuf}) {
    my $k = syswrite($c{in}, $c{wbuf});
    if (defined $k) { substr($c{wbuf}, 0, $k, '') } elsif (!$!{EAGAIN}) { close $c{in}; delete $c{in} }
  }
  while ((my $k = waitpid(-1, WNOHANG)) > 0) { if ($c{pid} && $k == $c{pid}) { ($c{status}, $c{pid}, $c{ended}) = ($?, 0, time) } else { delete $kids{$k} } }
  end_damaged();
  if (%c && !$c{pid}) {
    # Drain what ssh left: everything once the run said it was done (30 s
    # of quiet at most); otherwise until 2 s pass with nothing new (on a
    # dropped connection, whatever still holds its pipes no longer counts).
    next if ($c{out} || $c{err}) && time - ($c{seen} > $c{ended} ? $c{seen} : $c{ended}) < (defined $done ? 30 : 2);
    close $c{out} if $c{out}; close $c{err} if $c{err}; close $c{in} if $c{in};
    # A cut-off last line: passed on, unless it is one of the runner's
    # (a line of `err` among them comes again after a reconnect).
    my $own = "hollow-remote: $c{nonce} ";
    put(\*STDERR, $c{lbuf}) if length $c{lbuf} && !$c{bad} && index($c{lbuf}, $c{nonce}) < 0 && index($c{lbuf}, $own) != 0 && index($own, $c{lbuf}) != 0;
    my ($status, $bad) = ($c{status}, $c{damaged}); %c = ();
    quit($done) if defined $done;
    if ($gone) { say_err("the run is no longer on $host; its work comes back at the next turn there"); quit(255) }
    # The run never started there: its own message is above.
    quit($status & 127 ? 128 + ($status & 127) : $status >> 8) unless $ever;
    if ($bad) { say_err("a line of the run's output from $host came through damaged; asking for it again") }
    elsif (!$dropped) { $dropped = time; say_err("the connection to $host dropped; the run goes on there, reconnecting") }
    if (defined $dropped && time - $dropped > 12 * 3600) { say_err("$host stayed unreachable for 12 hours; the run's work comes back at the next turn there"); quit(255) }
    ($retry_at, $backoff) = (time + $backoff, $backoff >= 16 ? 30 : $backoff * 2);
  }
  if (!%c && defined $retry_at && time >= $retry_at) { $retry_at = undef; connect_runner() or $retry_at = time + $backoff }
}
"#;

#[cfg(test)]
#[path = "remote_session_tests.rs"]
mod tests;
