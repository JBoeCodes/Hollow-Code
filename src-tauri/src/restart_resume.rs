//! Resume after restart (overnight 2026-10-10, A1; the turn-guards
//! follow-up, Part 3 item 4).
//!
//! When Hollow quits mid-turn, the next launch's queue recovery
//! (`run_queue::recover_at_startup`) ends that chat row `interrupted` with
//! [`run_queue::INTERRUPTED_RUNNING`] and closes its transcript turn the same
//! way. Those rows are the record of what was running: nothing new is
//! stored. After recovery the pump calls [`recovered`], and from then on
//! eligible rows from the last 24 hours form one "Resume N interrupted chats" item in Waiting on
//! you ([`waiting`], kind `interrupted`). Its card lists them; **Resume ticked**
//! ([`resume`]) sends each ticked conversation exactly the manual Continue's text
//! ([`CONTINUE_PROMPT`], `src/chatContinue.ts`) as an ordinary queued turn,
//! like the composer's Continue: its native session, agent, model (the
//! interrupted run's requested one), folder and last permission mode, plus the
//! chat's own Canvases choice, the composer's Ultracode switch and the chat's
//! own reasoning effort (the window passes it, and the chat's own, kept in the
//! session store, applies when it passes none: a COO-triggered resume, a phone
//! send); the effort is checked and applied as a composer send's is. With those the native
//! session's configuration fingerprint is the one it was started with, unless
//! the configuration itself changed meanwhile (an MCP server, the prompt), in
//! which case it gets a fresh session with the conversation bundle, as any
//! turn would. `continued_from` is empty, as the composer's is after a
//! restart. The turn runs at depth 0: a person clicked, at the conversation's
//! own last mode.
//!
//! Never automatic: a resume is the person's click, or the COO's
//! `sessions.list_interrupted_chats` and `sessions.resume_interrupted_chats`
//! actions (COO parity L13; the resume asks on its card and never takes a
//! chat an agent started), and a COO check-in skips the item. A chat is
//! offered ([`candidates`]) only while all of these hold:
//! - recovery ended it mid-run within the eligibility window (a turn that never started has
//!   its text put back in the composer instead);
//! - it was the person's turn: never an unattended one (the night queue, a
//!   Main agent wake or check-in, or a chat such a turn started:
//!   `main_agent::unattended::recorded_unattended`, which reads the flag the
//!   queue row keeps through its terminal write, and a Main agent turn's
//!   own "⟲ Hollow:" message);
//! - it is the conversation's last turn: no chat row was queued or run for
//!   it since (a manual Continue, a new message, a resume already sent);
//! - it was last seen working within [`STALE_AFTER_HOURS`] (its newest
//!   stream event, else its claim, else its ask);
//! - the conversation exists, is not archived, has a native session to
//!   resume (the chat's own Continue has the same rule) and its folder exists.
//!
//! The card (P1-40) lists each offered chat with its model and permission
//! mode, to tick one by one, and below them the chats an agent started that
//! the same quit stopped ([`listed`], `agent_started`): those open, never
//! resume here, and [`resume`] refuses one in words.
//!
//! A row that can't be read is left out (and logged), never the whole offer.
//! [`resume`] holds one lock and re-reads the candidates at the click, so a
//! key no longer offered (one a first click already resumed) is skipped and
//! reported, never sent twice.

use crate::run_queue::{self, QueueRow};
use chrono::{DateTime, Duration, Utc};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Exactly what a resume sends: the manual Continue's text, in lockstep with
/// `CONTINUE_PROMPT` in `src/chatContinue.ts` (test
/// `the_prompt_matches_the_chats_continue`).
pub(crate) const CONTINUE_PROMPT: &str =
    "Your previous run was interrupted. Continue from where you left off.";
/// A turn last seen working longer ago than this is stale: not offered.
pub(crate) const STALE_AFTER_HOURS: i64 = 24;
/// Titles named in the waiting item's second line.
const TITLES_NAMED: usize = 3;

/// One resume at a time, so a second click re-reads what the first sent.
static RESUMING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// When each vault's queue recovery finished in this process: the launch
/// whose interrupted chats are offered. Keyed by vault so tests never share.
static RECOVERED: Lazy<Mutex<HashMap<PathBuf, DateTime<Utc>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// The pump's startup recovery for `vault` finished; rows it interrupted
/// were finished at or after `boot_at`.
pub(crate) fn recovered(vault: &Path, boot_at: DateTime<Utc>) {
    RECOVERED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(vault.to_path_buf(), boot_at);
}

fn recovered_at(vault: &Path) -> Option<DateTime<Utc>> {
    RECOVERED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(vault)
        .copied()
}

/// One chat Hollow's last quit interrupted mid-turn, as the card lists it.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct InterruptedChat {
    pub session_key: String,
    pub title: String,
    pub agent: String,
    pub workspace_id: Option<String>,
    /// When the interrupted turn was asked (RFC 3339).
    pub asked_at: String,
    /// The model the interrupted run asked for; `None` is the agent's
    /// default. A resume asks for the same one (P1-40 shows it).
    pub model: Option<String>,
    /// The conversation's last permission mode, the one a resume runs at;
    /// `None` when no turn recorded one.
    pub permission_mode: Option<String>,
    /// An agent started this chat (depth 1 or deeper, or an unattended
    /// turn): listed to open, never offered to resume (P1-40).
    pub agent_started: bool,
}

/// One chat the person asked to resume, with the composer's settings for it.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResumeRequest {
    pub session_key: String,
    /// The chat's own Canvases choice (`off`, `helps`, `most`), or `None` to
    /// follow the setting, as its composer would send.
    #[serde(default)]
    pub canvases: Option<String>,
    /// The chat's own Ultracode switch (P1-10: each conversation's own, kept
    /// in the webview under its key); missing means off.
    #[serde(default)]
    pub ultracode: bool,
    /// The chat's own reasoning effort (Settings PR 7: one of
    /// `effort::LEVELS`, as its composer sends it); missing or empty follows
    /// the chat's own level in the session store (`chat::resolve_requested_effort`
    /// at run time), then the fixed level or Automatic. An
    /// unknown value refuses that chat's resume, never reaching the agent.
    #[serde(default)]
    pub effort: Option<String>,
}

/// What resuming one asked-for chat did.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ResumeOutcome {
    pub session_key: String,
    /// The continue turn is queued.
    pub queued: bool,
    /// Why it was not, when it was not.
    pub reason: Option<String>,
}

/// A candidate with what its turn needs.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Candidate {
    pub chat: InterruptedChat,
    pub cwd: String,
    pub native_session: String,
    pub model: Option<String>,
    pub permission_mode: Option<String>,
}

/// The chats this launch offers to resume (see the module doc), newest
/// first; empty before recovery has run in this process.
pub(crate) fn offered(vault: &Path, now: DateTime<Utc>) -> Result<Vec<Candidate>, String> {
    Ok(stopped(vault, now)?
        .into_iter()
        .filter(|c| !c.chat.agent_started)
        .collect())
}

/// What the card lists (P1-40): the offered chats, then the chats an agent
/// started that the same quit stopped (to open, never to resume here),
/// each group newest first.
pub(crate) fn listed(vault: &Path, now: DateTime<Utc>) -> Result<Vec<InterruptedChat>, String> {
    let (mut mine, theirs): (Vec<_>, Vec<_>) = stopped(vault, now)?
        .into_iter()
        .map(|c| c.chat)
        .partition(|c| !c.agent_started);
    mine.extend(theirs);
    Ok(mine)
}

/// Every chat this launch's quit stopped mid-run, offered or agent-started;
/// empty before recovery has run in this process.
fn stopped(vault: &Path, now: DateTime<Utc>) -> Result<Vec<Candidate>, String> {
    match recovered_at(vault) {
        Some(_) => all_candidates(vault, now - Duration::hours(STALE_AFTER_HOURS), now),
        None => Ok(Vec::new()),
    }
}

/// The chats recovery ended mid-run at or after `since` that may still be
/// resumed at `now`, one per conversation, newest first.
pub(crate) fn candidates(
    vault: &Path,
    since: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<Vec<Candidate>, String> {
    Ok(all_candidates(vault, since, now)?
        .into_iter()
        .filter(|c| !c.chat.agent_started)
        .collect())
}

/// [`candidates`] plus the chats an agent started (flagged `agent_started`).
fn all_candidates(
    vault: &Path,
    since: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<Vec<Candidate>, String> {
    let rows = run_queue::chats_interrupted_mid_run(vault, since)?;
    let homes = crate::workspaces::list(vault).unwrap_or_default();
    let mut out = Vec::new();
    for (row, later) in rows {
        let Some(key) = row.source_id.clone() else {
            continue;
        };
        // A conversation's older interrupted rows always have a later one,
        // so each conversation is offered at most once.
        if later {
            continue;
        }
        match candidate(vault, &row, &key, now, &homes) {
            Ok(Some(candidate)) => out.push(candidate),
            Ok(None) => {}
            Err(error) => eprintln!("[resume after restart] {key} left out: {error}"),
        }
    }
    Ok(out)
}

fn candidate(
    vault: &Path,
    row: &QueueRow,
    key: &str,
    now: DateTime<Utc>,
    homes: &[crate::workspaces::Workspace],
) -> Result<Option<Candidate>, String> {
    let unattended = row.payload.get("unattended") == Some(&serde_json::Value::Bool(true))
        || row
            .history_run_id
            .as_deref()
            .is_some_and(|run| crate::main_agent::unattended::recorded_unattended(vault, run));
    // A chat an agent started (a helper, a handover successor, a bake-off
    // variant: depth 1 and deeper) or one in Hollow's own scratch folders (a
    // pressure-test persona) belongs to its parent, which hears how it ended;
    // resumed from here it would run as the person's own at depth 0
    // (overnight S5). P1-40 lists it apart, to open, with no resume.
    let agent_started = unattended
        || row
            .payload
            .get("appDepth")
            .and_then(|d| d.as_u64())
            .is_some_and(|d| d > 0);
    let last_seen = crate::session_store::last_event_at(vault, key)?
        .into_iter()
        .chain(row.claimed_at.clone())
        .chain(Some(row.requested_at.clone()))
        .filter_map(|at| DateTime::parse_from_rfc3339(&at).ok())
        .map(|at| at.with_timezone(&Utc))
        .max();
    if last_seen.is_none_or(|at| now - at > Duration::hours(STALE_AFTER_HOURS)) {
        return Ok(None);
    }
    if crate::session_store::is_archived(vault, key)? != Some(false) {
        return Ok(None);
    }
    let Some(summary) = crate::session_store::session_by_key(vault, key)? else {
        return Ok(None);
    };
    let Some(native_session) = summary.harness_session_id.clone() else {
        return Ok(None);
    };
    if summary.cwd.is_empty()
        || !Path::new(&summary.cwd).is_dir()
        || crate::mcp_attach::bare_scratch(vault, Path::new(&summary.cwd))
    {
        return Ok(None);
    }
    let model = match row.history_run_id.as_deref() {
        Some(run) => crate::session_store::chat_run_model(vault, run)?,
        None => None,
    };
    let permission_mode = crate::session_store::last_permission(vault, key)?;
    let title = summary
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "A conversation".to_string());
    Ok(Some(Candidate {
        chat: InterruptedChat {
            session_key: key.to_string(),
            title: title.lines().next().unwrap_or_default().trim().to_string(),
            agent: summary.agent.clone(),
            workspace_id: homes
                .iter()
                .find(|w| w.cwd == summary.cwd)
                .map(|w| w.id.clone()),
            asked_at: row.requested_at.clone(),
            model: model.clone(),
            permission_mode: permission_mode.clone(),
            agent_started,
        },
        cwd: summary.cwd,
        native_session,
        model,
        permission_mode,
    }))
}

/// The one waiting item for this launch's offer, or `None` when nothing is
/// offered: `(source id, title, second line, when it started waiting)`.
pub(crate) fn waiting(
    vault: &Path,
    now: DateTime<Utc>,
) -> Result<Option<(String, String, String, String)>, String> {
    let Some(since) = recovered_at(vault) else {
        return Ok(None);
    };
    let offered = candidates(vault, since, now)?;
    if offered.is_empty() {
        return Ok(None);
    }
    let n = offered.len();
    let title = if n == 1 {
        "Resume 1 interrupted chat".to_string()
    } else {
        format!("Resume {n} interrupted chats")
    };
    let mut named: Vec<String> = offered
        .iter()
        .take(TITLES_NAMED)
        .map(|c| c.chat.title.clone())
        .collect();
    if n > TITLES_NAMED {
        named.push(format!("{} more", n - TITLES_NAMED));
    }
    let detail = format!("Hollow quit while they ran: {}", named.join(", "));
    let at = crate::session_store::db_time(since);
    Ok(Some((at.clone(), title, detail, at)))
}

/// A send for one candidate: the conversation's own agent, model, folder,
/// native session and last permission mode, the composer's `canvases` and
/// `ultracode`, at depth 0 (the person's click).
pub(crate) fn send_input(
    candidate: &Candidate,
    canvases: Option<String>,
    ultracode: bool,
) -> crate::commands::ChatSendInput {
    crate::commands::ChatSendInput {
        prompt: CONTINUE_PROMPT.to_string(),
        session_id: Some(candidate.native_session.clone()),
        session_key: Some(candidate.chat.session_key.clone()),
        cwd: Some(candidate.cwd.clone()),
        model: candidate.model.clone(),
        permission_mode: candidate.permission_mode.clone(),
        agent: Some(candidate.chat.agent.clone()),
        client_turn_id: None,
        app_depth: 0,
        ultracode,
        canvases,
        continued_from: Vec::new(),
        unattended: false,
        typed: false,
        host_asker: None,
        canned: None,
        worker: false,
    }
}

/// What resuming `candidate` sends for `request`: the Continue input and the
/// chat's own effort, checked as the composer's send checks it
/// (`commands::chat_effort`): an unknown level refuses this chat's resume.
fn resume_send(
    candidate: &Candidate,
    request: ResumeRequest,
) -> Result<(crate::commands::ChatSendInput, Option<String>), String> {
    let effort = crate::commands::chat_effort(request.effort)?;
    Ok((
        send_input(candidate, request.canvases, request.ultracode),
        effort,
    ))
}

/// Resume the asked-for chats (the person's **Resume ticked**): each key still
/// offered gets [`CONTINUE_PROMPT`] as a queued turn; any other is skipped
/// and said so. Keys are taken once each, in the order asked; each request
/// carries its own Canvases choice, Ultracode switch and effort.
pub(crate) async fn resume(
    app: Option<&tauri::AppHandle>,
    config: &crate::config::Config,
    chats: Vec<ResumeRequest>,
) -> Result<Vec<ResumeOutcome>, String> {
    let _one = RESUMING.lock().await;
    let vault = config.vault_path.clone();
    let stopped = tauri::async_runtime::spawn_blocking(move || stopped(&vault, Utc::now()))
        .await
        .map_err(|e| e.to_string())??;
    let (agents, offered): (Vec<_>, Vec<_>) =
        stopped.into_iter().partition(|c| c.chat.agent_started);
    let mut done = HashSet::new();
    let mut out = Vec::new();
    for chat in chats {
        let key = chat.session_key.trim().to_string();
        if !done.insert(key.clone()) {
            continue;
        }
        if agents.iter().any(|c| c.chat.session_key == key) {
            out.push(ResumeOutcome {
                session_key: key,
                queued: false,
                reason: Some("An agent started this chat: open it and press Continue.".to_string()),
            });
            continue;
        }
        let Some(candidate) = offered.iter().find(|c| c.chat.session_key == key) else {
            out.push(ResumeOutcome {
                session_key: key,
                queued: false,
                reason: Some(
                    "No longer interrupted (it has a newer turn, is archived or too old)."
                        .to_string(),
                ),
            });
            continue;
        };
        let (input, effort) = match resume_send(candidate, chat) {
            Ok(send) => send,
            Err(error) => {
                out.push(ResumeOutcome {
                    session_key: key,
                    queued: false,
                    reason: Some(error),
                });
                continue;
            }
        };
        // The chat's own effort goes with the turn as its composer's send
        // takes it (per chat, then the fixed level, then Automatic).
        let sent = crate::commands::send_chat_with_effort(app, config, input, effort, None).await;
        let outcome = match sent {
            Ok(_) => ResumeOutcome {
                session_key: key,
                queued: true,
                reason: None,
            },
            Err(error) => ResumeOutcome {
                session_key: key,
                queued: false,
                reason: Some(error),
            },
        };
        out.push(outcome);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_queue::{claim, enqueue, mark_running, recover_at_startup, NewRow, RunSource};
    use tempfile::TempDir;

    struct Bed {
        _dir: TempDir,
        vault: PathBuf,
        folder: PathBuf,
    }

    fn bed() -> Bed {
        let dir = TempDir::new().unwrap();
        let vault = dir.path().join("vault");
        let folder = dir.path().join("work");
        std::fs::create_dir_all(&vault).unwrap();
        std::fs::create_dir_all(&folder).unwrap();
        Bed {
            _dir: dir,
            vault,
            folder,
        }
    }

    fn chat_row(key: &str, asked: &str) -> NewRow {
        let mut row = NewRow::new(RunSource::Chat, Some(key), "Chat turn", "chat")
            .with_payload(serde_json::json!({"prompt": asked}));
        row.detail = Some(asked.to_string());
        row
    }

    /// A chat turn that was running when Hollow quit: queued, claimed,
    /// running, its message in the transcript and its native session known.
    fn running_turn(bed: &Bed, key: &str, asked: &str, at: DateTime<Utc>) -> i64 {
        let id = enqueue(&bed.vault, chat_row(key, asked), at).unwrap().id();
        claim(&bed.vault, id, at).unwrap().expect("claimed");
        let run = format!("run-{id}");
        mark_running(&bed.vault, id, &run, at).unwrap();
        crate::session_store::start_turn(
            &bed.vault,
            key,
            "claude",
            &bed.folder.to_string_lossy(),
            &run,
            asked,
        )
        .unwrap();
        crate::session_store::record_harness_id(&bed.vault, key, &format!("native-{key}")).unwrap();
        id
    }

    /// A turn of a chat an agent started (depth 1), running at the quit.
    fn helper_turn(bed: &Bed, key: &str, at: DateTime<Utc>) -> i64 {
        let v = &bed.vault;
        let mut row = chat_row(key, "helper work");
        row.payload = serde_json::json!({"prompt": "do the helper part", "appDepth": 1});
        let id = enqueue(v, row, at).unwrap().id();
        claim(v, id, at).unwrap().expect("claimed");
        let run = format!("run-{id}");
        mark_running(v, id, &run, at).unwrap();
        crate::session_store::start_turn(
            v,
            key,
            "claude",
            &bed.folder.to_string_lossy(),
            &run,
            "helper work",
        )
        .unwrap();
        crate::session_store::record_harness_id(v, key, &format!("native-{key}")).unwrap();
        id
    }

    fn keys(found: &[Candidate]) -> Vec<&str> {
        found.iter().map(|c| c.chat.session_key.as_str()).collect()
    }

    /// The whole rule: a chat running at the quit is offered once recovery
    /// has run; one that never started (its text goes to the composer) is
    /// not; a conversation with a later turn is not; an archived one is not.
    #[test]
    fn offers_only_the_chats_the_quit_cut_short_mid_run() {
        let bed = bed();
        let v = &bed.vault;
        let before = Utc::now() - Duration::minutes(10);
        let boot = Utc::now() - Duration::minutes(1);
        running_turn(&bed, "s-20261010-aaaa0001", "write the report", before);
        running_turn(&bed, "s-20261010-aaaa0002", "archived one", before);
        // Queued but never started: recovered as INTERRUPTED_QUEUED.
        enqueue(v, chat_row("s-20261010-aaaa0003", "never ran"), before).unwrap();

        assert!(
            offered(v, Utc::now()).unwrap().is_empty(),
            "nothing before recovery"
        );
        recover_at_startup(v, boot, Utc::now(), &HashSet::new()).unwrap();
        recovered(v, boot);
        // Archived once its turn is no longer running (archiving a running
        // conversation is refused, P1-39).
        crate::session_store::archive_session(v, "s-20261010-aaaa0002", true).unwrap();

        let found = offered(v, Utc::now()).unwrap();
        assert_eq!(keys(&found), vec!["s-20261010-aaaa0001"]);
        let c = &found[0];
        assert_eq!(c.chat.title, "write the report");
        assert_eq!(c.native_session, "native-s-20261010-aaaa0001");
        assert_eq!(c.cwd, bed.folder.to_string_lossy());

        // A later turn in that conversation (a manual Continue, a new
        // message) ends the offer.
        enqueue(v, chat_row("s-20261010-aaaa0001", "next"), Utc::now()).unwrap();
        assert!(offered(v, Utc::now()).unwrap().is_empty());
    }

    /// Unattended turns (the night queue, Main agent wakes, chats they
    /// started) are never offered as the person's: the queue row keeps its
    /// `unattended` flag through the terminal write, and nothing else of its
    /// payload.
    #[test]
    fn an_unattended_turn_is_never_offered() {
        let bed = bed();
        let v = &bed.vault;
        let before = Utc::now() - Duration::minutes(10);
        let boot = Utc::now() - Duration::minutes(1);
        let key = "s-20261010-iiii0001";
        let mut row = chat_row(key, "night item");
        row.payload = serde_json::json!({"prompt": "<page-context>secret</page-context>", "unattended": true});
        let id = enqueue(v, row, before).unwrap().id();
        claim(v, id, before).unwrap().expect("claimed");
        mark_running(v, id, "run-night", before).unwrap();
        crate::session_store::start_turn(
            v,
            key,
            "claude",
            &bed.folder.to_string_lossy(),
            "run-night",
            "night item",
        )
        .unwrap();
        crate::session_store::record_harness_id(v, key, "native-night").unwrap();
        let attended = running_turn(&bed, "s-20261010-iiii0002", "mine", before);
        recover_at_startup(v, boot, Utc::now(), &HashSet::new()).unwrap();
        recovered(v, boot);
        assert_eq!(
            keys(&offered(v, Utc::now()).unwrap()),
            vec!["s-20261010-iiii0002"]
        );
        let payload = |id| run_queue::get(v, id).unwrap().unwrap().payload;
        assert_eq!(payload(id), serde_json::json!({"unattended": true}));
        assert_eq!(payload(attended), serde_json::json!({}));
    }

    /// A chat an agent started (depth 1 or deeper) belongs to its parent:
    /// never offered, and its finished row keeps only its depth.
    #[test]
    fn an_agent_started_chat_is_never_offered() {
        let bed = bed();
        let v = &bed.vault;
        let before = Utc::now() - Duration::minutes(10);
        let boot = Utc::now() - Duration::minutes(1);
        let id = helper_turn(&bed, "s-20261010-dddd0001", before);
        running_turn(&bed, "s-20261010-dddd0002", "mine", before);
        recover_at_startup(v, boot, Utc::now(), &HashSet::new()).unwrap();
        recovered(v, boot);
        assert_eq!(
            keys(&offered(v, Utc::now()).unwrap()),
            vec!["s-20261010-dddd0002"]
        );
        assert_eq!(
            run_queue::get(v, id).unwrap().unwrap().payload,
            serde_json::json!({"appDepth": 1})
        );
        // P1-40: the card still names it, apart, to open (never to resume).
        let listed = listed(v, Utc::now()).unwrap();
        let named: Vec<(&str, bool)> = listed
            .iter()
            .map(|c| (c.session_key.as_str(), c.agent_started))
            .collect();
        assert_eq!(
            named,
            vec![
                ("s-20261010-dddd0002", false),
                ("s-20261010-dddd0001", true)
            ]
        );
        let (_, title, _, _) = waiting(v, Utc::now()).unwrap().expect("an item");
        assert_eq!(title, "Resume 1 interrupted chat", "only the person's own");
    }

    /// P1-40: the card shows each chat's permission mode and model, the
    /// ones its resume runs with.
    #[test]
    fn an_interrupted_chat_carries_its_mode_and_model() {
        let bed = bed();
        let v = &bed.vault;
        let before = Utc::now() - Duration::minutes(10);
        let boot = Utc::now() - Duration::minutes(1);
        let key = "s-20261010-mmmm0001";
        running_turn(&bed, key, "with a mode", before);
        crate::session_store::record_turn_permission(v, key, 1, "acceptEdits").unwrap();
        recover_at_startup(v, boot, Utc::now(), &HashSet::new()).unwrap();
        recovered(v, boot);
        let found = offered(v, Utc::now()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].chat.permission_mode.as_deref(),
            Some("acceptEdits")
        );
        assert_eq!(found[0].chat.permission_mode, found[0].permission_mode);
        assert_eq!(
            found[0].chat.model, None,
            "no priced turn: the agent's default"
        );
        assert!(!found[0].chat.agent_started);
        let json = serde_json::to_value(&found[0].chat).unwrap();
        assert_eq!(json["permissionMode"], "acceptEdits");
        assert!(json.get("model").is_some() && json.get("agentStarted").is_some());
    }

    /// P1-40: an agent-started chat is never resumed from the card, and the
    /// refusal says why.
    #[tokio::test]
    async fn resume_refuses_an_agent_started_chat_in_words() {
        let bed = bed();
        let v = bed.vault.clone();
        let before = Utc::now() - Duration::minutes(10);
        let boot = Utc::now() - Duration::minutes(1);
        helper_turn(&bed, "s-20261010-hhhh0001", before);
        recover_at_startup(&v, boot, Utc::now(), &HashSet::new()).unwrap();
        recovered(&v, boot);
        let config = crate::config::Config {
            vault_path: v.clone(),
            ..crate::config::Config::placeholder()
        };
        let out = resume(
            None,
            &config,
            vec![ResumeRequest {
                session_key: "s-20261010-hhhh0001".into(),
                canvases: None,
                ultracode: false,
                effort: None,
            }],
        )
        .await
        .unwrap();
        assert!(!out[0].queued);
        assert_eq!(
            out[0].reason.as_deref(),
            Some("An agent started this chat: open it and press Continue.")
        );
    }

    /// A pressure-test persona's chat (a scratch folder of Hollow's own,
    /// sent at depth 0) is never offered either (S5).
    #[test]
    fn a_chat_in_hollows_scratch_folder_is_never_offered() {
        let mut bed = bed();
        let v = bed.vault.clone();
        let before = Utc::now() - Duration::minutes(10);
        let boot = Utc::now() - Duration::minutes(1);
        let scratch = crate::stakeholder_sim::root(&v).unwrap().join("pt-1-cfo");
        std::fs::create_dir_all(&scratch).unwrap();
        let mine = bed.folder.clone();
        bed.folder = scratch.clone();
        running_turn(&bed, "s-20261010-pppp0001", "critique as the CFO", before);
        bed.folder = mine;
        running_turn(&bed, "s-20261010-pppp0002", "mine", before);
        recover_at_startup(&v, boot, Utc::now(), &HashSet::new()).unwrap();
        recovered(&v, boot);
        assert_eq!(
            keys(&offered(&v, Utc::now()).unwrap()),
            vec!["s-20261010-pppp0002"]
        );
        let _ = std::fs::remove_dir_all(crate::stakeholder_sim::root(&v).unwrap());
    }

    /// Staleness is measured from when the turn was last seen working, so a
    /// long turn asked yesterday but busy until the quit is still offered.
    #[test]
    fn a_long_turn_busy_until_the_quit_is_not_stale() {
        let bed = bed();
        let v = &bed.vault;
        let long_ago = Utc::now() - Duration::hours(STALE_AFTER_HOURS + 5);
        let boot = Utc::now() - Duration::minutes(1);
        let key = "s-20261010-jjjj0001";
        running_turn(&bed, key, "overnight research", long_ago);
        crate::session_store::append_event(v, key, 1, "assistant", "still working").unwrap();
        recover_at_startup(v, boot, Utc::now(), &HashSet::new()).unwrap();
        recovered(v, boot);
        assert_eq!(keys(&offered(v, Utc::now()).unwrap()), vec![key]);
    }

    #[test]
    fn a_stale_turn_or_a_vanished_folder_is_not_offered() {
        let bed = bed();
        let v = &bed.vault;
        let long_ago = Utc::now() - Duration::hours(STALE_AFTER_HOURS + 1);
        let recent = Utc::now() - Duration::minutes(5);
        let boot = Utc::now() - Duration::minutes(1);
        running_turn(&bed, "s-20261010-bbbb0001", "old work", long_ago);
        running_turn(&bed, "s-20261010-bbbb0002", "recent work", recent);
        recover_at_startup(v, boot, Utc::now(), &HashSet::new()).unwrap();
        recovered(v, boot);
        assert_eq!(
            keys(&offered(v, Utc::now()).unwrap()),
            vec!["s-20261010-bbbb0002"]
        );

        std::fs::remove_dir_all(&bed.folder).unwrap();
        assert!(offered(v, Utc::now()).unwrap().is_empty(), "folder gone");
    }

    /// A second quit must not discard the person's still-eligible offer.
    #[test]
    fn an_unanswered_offer_survives_a_second_quit() {
        let bed = bed();
        let v = &bed.vault;
        let first_boot = Utc::now() - Duration::hours(2);
        running_turn(
            &bed,
            "s-20261010-cccc0001",
            "earlier",
            first_boot - Duration::minutes(5),
        );
        recover_at_startup(v, first_boot, first_boot, &HashSet::new()).unwrap();
        let second_boot = Utc::now() - Duration::minutes(1);
        recover_at_startup(v, second_boot, Utc::now(), &HashSet::new()).unwrap();
        recovered(v, second_boot);
        assert_eq!(
            keys(&offered(v, Utc::now()).unwrap()),
            vec!["s-20261010-cccc0001"]
        );
    }

    /// Two rows of one conversation that were both live at the quit: only
    /// the newest is its last turn, so it is offered once.
    #[test]
    fn one_offer_per_conversation() {
        let bed = bed();
        let v = &bed.vault;
        let before = Utc::now() - Duration::minutes(10);
        let boot = Utc::now() - Duration::minutes(1);
        let key = "s-20261010-dddd0001";
        running_turn(&bed, key, "first", before);
        running_turn(&bed, key, "second", before);
        recover_at_startup(v, boot, Utc::now(), &HashSet::new()).unwrap();
        recovered(v, boot);
        assert_eq!(keys(&offered(v, Utc::now()).unwrap()), vec![key]);
    }

    #[test]
    fn the_send_is_exactly_the_continue_on_the_same_conversation() {
        let bed = bed();
        let v = &bed.vault;
        let before = Utc::now() - Duration::minutes(10);
        let boot = Utc::now() - Duration::minutes(1);
        running_turn(&bed, "s-20261010-eeee0001", "do it", before);
        recover_at_startup(v, boot, Utc::now(), &HashSet::new()).unwrap();
        recovered(v, boot);
        let found = offered(v, Utc::now()).unwrap();
        let input = send_input(&found[0], Some("most".into()), true);
        assert_eq!(
            input.canvases.as_deref(),
            Some("most"),
            "the chat's own choice"
        );
        assert!(input.ultracode, "the composer's switch");
        assert!(input.continued_from.is_empty());
        assert_eq!(input.prompt, CONTINUE_PROMPT);
        assert_eq!(input.session_key.as_deref(), Some("s-20261010-eeee0001"));
        assert_eq!(
            input.session_id.as_deref(),
            Some("native-s-20261010-eeee0001")
        );
        assert_eq!(input.agent.as_deref(), Some("claude"));
        assert_eq!(input.app_depth, 0, "the person's click, not an agent's");
        assert!(!input.unattended);
    }

    #[test]
    fn the_waiting_item_counts_and_names_them() {
        let bed = bed();
        let v = &bed.vault;
        let before = Utc::now() - Duration::minutes(10);
        let boot = Utc::now() - Duration::minutes(1);
        assert_eq!(waiting(v, Utc::now()).unwrap(), None, "no recovery yet");
        for (i, asked) in ["one", "two", "three", "four"].iter().enumerate() {
            running_turn(&bed, &format!("s-20261010-ffff000{i}"), asked, before);
        }
        recover_at_startup(v, boot, Utc::now(), &HashSet::new()).unwrap();
        recovered(v, boot);
        let (_, title, detail, _) = waiting(v, Utc::now()).unwrap().expect("an item");
        assert_eq!(title, "Resume 4 interrupted chats");
        assert_eq!(
            detail,
            "Hollow quit while they ran: four, three, two, 1 more"
        );
    }

    /// P1-10: Ultracode is each conversation's own, so each resumed chat
    /// carries its value (a request without one is off).
    #[test]
    fn each_resume_request_carries_its_own_ultracode() {
        let on: ResumeRequest =
            serde_json::from_str(r#"{"sessionKey":"s-1","canvases":null,"ultracode":true}"#)
                .unwrap();
        assert!(on.ultracode);
        let old: ResumeRequest = serde_json::from_str(r#"{"sessionKey":"s-2"}"#).unwrap();
        assert!(!old.ultracode, "missing means off");
    }

    /// C14: a chat set to Max resumes at Max (the composer's own level goes
    /// with the turn, ahead of the fixed level and Automatic); a request
    /// with none follows those; an unknown level is refused for that chat
    /// and never reaches the send.
    #[tokio::test]
    async fn resume_keeps_the_chats_own_effort_and_refuses_an_unknown_one() {
        let bed = bed();
        let v = bed.vault.clone();
        let before = Utc::now() - Duration::minutes(10);
        let boot = Utc::now() - Duration::minutes(1);
        running_turn(&bed, "s-20261010-kkkk0001", "at max", before);
        running_turn(&bed, "s-20261010-kkkk0002", "made up", before);
        recover_at_startup(&v, boot, Utc::now(), &HashSet::new()).unwrap();
        recovered(&v, boot);
        let found = offered(&v, Utc::now()).unwrap();
        let max = found
            .iter()
            .find(|c| c.chat.session_key == "s-20261010-kkkk0001")
            .unwrap();
        let ask: ResumeRequest = serde_json::from_str(
            r#"{"sessionKey":"s-20261010-kkkk0001","canvases":null,"ultracode":false,"effort":"max"}"#,
        )
        .unwrap();
        let (input, effort) = resume_send(max, ask).unwrap();
        assert_eq!(effort.as_deref(), Some("max"), "the chat's own level");
        assert_eq!(input.prompt, CONTINUE_PROMPT);
        let none: ResumeRequest =
            serde_json::from_str(r#"{"sessionKey":"s-20261010-kkkk0001"}"#).unwrap();
        assert_eq!(
            resume_send(max, none).unwrap().1,
            None,
            "missing names none: the chat's own level in the store (else the default) applies at run time"
        );
        let blank = ResumeRequest {
            session_key: "s-20261010-kkkk0001".into(),
            canvases: None,
            ultracode: false,
            effort: Some(" ".into()),
        };
        assert_eq!(resume_send(max, blank).unwrap().1, None);

        let config = crate::config::Config {
            vault_path: v.clone(),
            ..crate::config::Config::placeholder()
        };
        let out = resume(
            None,
            &config,
            vec![ResumeRequest {
                session_key: "s-20261010-kkkk0002".into(),
                canvases: None,
                ultracode: false,
                effort: Some("--dangerously-skip-permissions".into()),
            }],
        )
        .await
        .unwrap();
        assert!(!out[0].queued);
        assert_eq!(out[0].reason.as_deref(), Some("Unknown reasoning effort."));
        assert_eq!(
            keys(&offered(&v, Utc::now()).unwrap()).len(),
            2,
            "a refused resume sends nothing, so both are still offered"
        );
    }

    /// The click re-reads the offer: a key not offered is skipped and said
    /// so, never sent; a repeated key is taken once; an offered key goes to
    /// the ordinary send (refused here only because no pump owns the queue
    /// in a test).
    #[tokio::test]
    async fn resume_sends_only_what_is_still_offered() {
        let bed = bed();
        let v = bed.vault.clone();
        let before = Utc::now() - Duration::minutes(10);
        let boot = Utc::now() - Duration::minutes(1);
        running_turn(&bed, "s-20261010-gggg0001", "offered", before);
        recover_at_startup(&v, boot, Utc::now(), &HashSet::new()).unwrap();
        recovered(&v, boot);
        let config = crate::config::Config {
            vault_path: v.clone(),
            ..crate::config::Config::placeholder()
        };
        let ask = |key: &str| ResumeRequest {
            session_key: key.into(),
            canvases: None,
            ultracode: false,
            effort: None,
        };
        let out = resume(
            None,
            &config,
            vec![
                ask("s-20261010-zzzz0009"),
                ask("s-20261010-gggg0001"),
                ask("s-20261010-gggg0001"),
            ],
        )
        .await
        .unwrap();
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(out[0].session_key, "s-20261010-zzzz0009");
        assert!(!out[0].queued);
        assert!(out[0]
            .reason
            .as_deref()
            .unwrap()
            .starts_with("No longer interrupted"));
        assert_eq!(out[1].session_key, "s-20261010-gggg0001");
        // No pump owns the queue in a test, so `send_chat` itself refuses:
        // the key reached the send rather than being skipped. On a machine
        // with no agent CLI (a build runner, 2026-11-01) it refuses earlier,
        // on the agent check, which is still the send.
        let reason = out[1].reason.as_deref().unwrap_or_default();
        assert!(
            out[1].queued || reason.contains("run queue") || reason.contains("CLI is unavailable"),
            "an offered key reaches the send: {reason}"
        );
    }

    /// A Main agent check-in never sees the item (only a person resumes),
    /// while Waiting on you lists it.
    #[test]
    fn waiting_on_you_lists_it_and_a_check_in_ignores_it() {
        let bed = bed();
        let v = &bed.vault;
        let before = Utc::now() - Duration::minutes(10);
        let boot = Utc::now() - Duration::minutes(1);
        running_turn(&bed, "s-20261010-hhhh0001", "report", before);
        let helper = helper_turn(&bed, "s-20261010-hhhh0002", before);
        recover_at_startup(v, boot, Utc::now(), &HashSet::new()).unwrap();
        recovered(v, boot);
        let page = crate::needs_you::list(v, Vec::new(), None, None, None).unwrap();
        let item = page
            .items
            .iter()
            .find(|i| i.kind == crate::needs_you::INTERRUPTED_KIND)
            .expect("the item");
        assert_eq!(item.title, "Resume 1 interrupted chat");
        assert_eq!(item.state, "pending");
        let signals = crate::main_agent::signals(v, Utc::now(), Utc::now() - Duration::days(1));
        assert!(
            signals.iter().all(|s| !s.line.contains("interrupted chat")),
            "{signals:?}"
        );
        // Nor as a failed run (S5): the recovered row's "Hollow stopped…"
        // would read as a run to retry. A chat it doesn't offer (a helper's)
        // is still reported.
        let failed: Vec<&str> = signals
            .iter()
            .filter(|s| s.id.starts_with("failed:"))
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(failed, vec![format!("failed:{helper}")], "{signals:?}");
        // And a check-in can't continue it itself.
        let mut args = serde_json::Map::new();
        args.insert("session".into(), "s-20261010-hhhh0001".into());
        let refused = crate::main_agent::unattended::allows(v, "sessions.send", false, &args, true)
            .unwrap_err();
        assert!(refused.contains("person's click"), "{refused}");
        // Resumed (a later turn of it): it leaves the offer and still isn't
        // a failure.
        enqueue(v, chat_row("s-20261010-hhhh0001", "continue"), Utc::now()).unwrap();
        assert!(offered(v, Utc::now()).unwrap().is_empty());
        let signals = crate::main_agent::signals(v, Utc::now(), Utc::now() - Duration::days(1));
        let failed: Vec<&str> = signals
            .iter()
            .filter(|s| s.id.starts_with("failed:"))
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(failed, vec![format!("failed:{helper}")], "{signals:?}");
    }

    /// Lockstep with the chat's own Continue (`src/chatContinue.ts`).
    #[test]
    fn the_prompt_matches_the_chats_continue() {
        let ts = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/chatContinue.ts"),
        )
        .unwrap();
        assert!(
            ts.contains(&format!(
                "export const CONTINUE_PROMPT = '{CONTINUE_PROMPT}';"
            )),
            "CONTINUE_PROMPT differs from src/chatContinue.ts"
        );
    }
}
