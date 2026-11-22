//! Run inside this chat (overhaul F1): an automation bound to one
//! conversation. Its `in_chat:` key names the conversation; each run is a
//! new turn there instead of a new chat.
//!
//! **Where it goes.** The bound key's handover chain's current conversation
//! (`chat_chain::current_of`), so a chat that handed over keeps its schedule.
//! A conversation that is gone (deleted) or archived, or that sits in a build
//! workspace (unattended work never runs on a person's branch), fails the
//! run visibly: an `error` row in the automation's run log with the reason,
//! and the usual failure notification ([`target`]).
//!
//! **How it runs.** The run (admitted, stamped and budgeted like any
//! automation run, `workflows::execute`) queues one chat turn and ends: the
//! automation's prompt under a one-line header ([`turn_text`]), as Hollow's
//! own unattended turn (`host_asker` [`ASKED_BY_AUTOMATION`]). The chat's
//! own agent and model answer (`wake_me::wake_model`), in the chat's own
//! last mode (`main_agent::wake_mode`, bypass and auto kept since
//! 2026-11-12), lowered only by an automation that set a mode of its own
//! weaker than the chat's ([`turn_mode`]). The automation's agent, model and
//! folder are not used. Being Hollow's turn, it never closes an open question card: the
//! pump holds it behind one (E1, `run_pump`'s card hold) and the person's own
//! message still goes first.
//!
//! **Behind a running turn.** A conversation takes one turn at a time, so a
//! run that finds a turn queued or running keeps its message pending here
//! and the minute pass ([`deliver_pending`]) sends it once that turn ends.
//! One pending message per automation: a newer run replaces it. One pending
//! longer than [`PENDING_HOURS`], or whose conversation is gone by then, is
//! dropped with an `error` row in the run log (the budget's notice when its
//! usage budget is what held it); one whose automation was deleted or
//! unbound since is dropped quietly. A database read that fails (busy,
//! unreadable) keeps it for the next pass, and a new run whose read fails
//! that way parks its message pending the same way; one that can't check
//! whether it was already sent drops after [`CHECK_CEILING_HOURS`].
//!
//! **Tools and budget (review of #1313).** A chat turn uses the chat's own
//! tools, so a bound automation can't keep an allowlist or denylist: the
//! save refuses one ([`TOOL_LISTS_REFUSED`]) and so does a run of a
//! hand-edited file. The turns' usage counts toward the automation's own
//! usage budget ([`turn_runs`], read by `budgets::scope_rows`) while each
//! row stays the chat's for every other figure, and a hard-stop budget
//! holds the send. Every send carries its run's [`turn_id`] as the chat
//! row's `clientTurnId`, which the row keeps after its turn ends
//! (`run_queue`'s terminal update), so a pending message whose entry outlived
//! its send is recognised in whichever conversation took it, before its age
//! is checked, and never sent twice. The state file's lock covers
//! only its read, change and write; a per-automation lock covers one
//! delivery, and the minute pass re-reads the entry under it, so a Run now
//! that replaced it wins. A pending message is dropped as unbound only when
//! its file is known gone: an error checking it keeps the message, until
//! [`PENDING_HOURS`] pass.
//!
//! Storage: `<support>/AutomationChats/<vault>.json` (pending messages and
//! the newest queued turns, for the budget).

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::types::{RunLogEntry, Workflow};

/// `inChat: "this"` in `save_automation`: the calling chat.
pub(crate) const THIS_CHAT: &str = "this";
/// The `turns.asked_by` of an automation's turn in its chat.
pub(crate) const ASKED_BY_AUTOMATION: &str = "hollow:automation";
/// A message waiting behind a running turn longer than this is dropped.
pub(crate) const PENDING_HOURS: i64 = 6;
/// A message whose send can't be checked ([`already_sent`] fails) is kept
/// until this many hours, then dropped; the queue keeps an automation's
/// chat rows at least this long (`run_queue`'s prune).
pub(crate) const CHECK_CEILING_HOURS: i64 = 2 * PENDING_HOURS;
/// Turns remembered for budget attribution ([`turn_runs`]), newest kept.
const TURNS_KEPT: usize = 2000;
/// Why a chat-bound automation can't keep tool lists: a chat turn doesn't
/// carry an automation's allowlist or denylist, so binding one would widen it.
pub(crate) const TOOL_LISTS_REFUSED: &str = "An automation that runs in a chat can't have allowed or disallowed tools: a chat turn uses the chat's own tools, so the lists would be ignored. Clear both lists, or set Runs in to New chat each run.";

/// The state file's lock: held only around its read, change and write,
/// never across a send or a database read.
static FILE: Mutex<()> = Mutex::new(());
/// One automation's delivery at a time (its send and its pending entry).
/// An entry lives only while a delivery holds or waits for it ([`one_delivery`]).
static PER_AUTOMATION: once_cell::sync::Lazy<Mutex<HashMap<String, Arc<Mutex<()>>>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(HashMap::new()));

/// Run `work` under automation `slug`'s lock, then forget the lock when no
/// other delivery holds or waits for it. Clones are made and dropped only
/// under the map's lock, so once ours is dropped there, a count of one (the
/// map's own) means nobody else has it, and the next delivery makes a fresh
/// one. Review of #1331: ours used to drop after the map's lock was released,
/// so two overlapping deliveries could each see the other's clone and both
/// leave the entry behind for good.
fn one_delivery<T>(slug: &str, work: impl FnOnce() -> T) -> T {
    let lock = {
        let mut all = PER_AUTOMATION.lock().unwrap_or_else(|e| e.into_inner());
        all.entry(slug.to_string()).or_default().clone()
    };
    let out = {
        let _one = lock.lock().unwrap_or_else(|e| e.into_inner());
        work()
    };
    let mut all = PER_AUTOMATION.lock().unwrap_or_else(|e| e.into_inner());
    let ours = all.get(slug).is_some_and(|l| Arc::ptr_eq(l, &lock));
    drop(lock);
    if ours && all.get(slug).is_some_and(|l| Arc::strong_count(l) == 1) {
        all.remove(slug);
    }
    out
}

/// The conversation a bound automation's run goes to.
#[derive(Debug, Clone)]
pub(crate) struct Target {
    /// The chain's current conversation.
    pub key: String,
    pub title: String,
    pub summary: crate::types::AgentSessionSummary,
}

fn title_of(summary: &crate::types::AgentSessionSummary) -> String {
    summary
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or("Untitled conversation")
        .to_string()
}

/// Why a bound automation has no chat to run in now.
#[derive(Debug, Clone, PartialEq)]
enum Miss {
    /// Established: the chat is gone or archived, sits in a build workspace,
    /// or the key isn't a conversation's. A waiting message drops.
    Unavailable(String),
    /// The database couldn't be read now (busy, unreadable). A waiting
    /// message stays for the next minute's pass.
    Unreadable(String),
}

impl Miss {
    fn message(self) -> String {
        match self {
            Miss::Unavailable(why) => why,
            Miss::Unreadable(error) => {
                format!("Hollow couldn't read the chat this automation runs in: {error}")
            }
        }
    }
}

/// Where bound key `bound`'s runs go now, or why they can't.
pub(crate) fn target(vault: &Path, bound: &str) -> Result<Target, String> {
    resolve(vault, bound).map_err(Miss::message)
}

/// [`target`], telling a chat that is known unavailable from a read that
/// failed (review of #1331: a busy database never drops a waiting message).
fn resolve(vault: &Path, bound: &str) -> Result<Target, Miss> {
    let bound = bound.trim();
    if !crate::agent_sessions::is_safe_session_key(bound) {
        return Err(Miss::Unavailable(format!(
            "{bound:?} isn't a conversation's session key, so this automation has no chat to run in."
        )));
    }
    let key = crate::chat_chain::current_of(vault, bound).unwrap_or_else(|| bound.to_string());
    let summary = crate::session_store::session_by_key(vault, &key)
        .map_err(Miss::Unreadable)?
        .ok_or_else(|| {
            Miss::Unavailable(format!("The chat this automation runs in ({key}) is gone: it was deleted. Set Runs in to New chat, or schedule it from another chat."))
        })?;
    let title = title_of(&summary);
    if crate::session_store::is_archived(vault, &key).map_err(Miss::Unreadable)? == Some(true) {
        return Err(Miss::Unavailable(format!(
            "The chat this automation runs in, “{title}”, is archived. Restore it, or set Runs in to New chat."
        )));
    }
    if crate::build_workspaces::workspace_for_worktree(vault, Path::new(&summary.cwd)).is_some() {
        return Err(Miss::Unavailable(format!(
            "“{title}” is in a build workspace, and unattended work never runs on a person's branch. Set Runs in to New chat."
        )));
    }
    Ok(Target {
        key,
        title,
        summary,
    })
}

/// A new binding's check (the automation editor's save and
/// `save_automation`): the conversation must exist, not be archived and not
/// be in a build workspace.
pub(crate) fn check_binding(vault: &Path, key: &str) -> Result<(), String> {
    target(vault, key)
        .map(|_| ())
        .map_err(|e| format!("{e} Nothing changed."))
}

/// "Runs in" on the save card: the chat's title and key, or a new chat.
pub(crate) fn runs_in_label(vault: &Path, bound: Option<&str>) -> String {
    match bound.map(str::trim).filter(|k| !k.is_empty()) {
        None => "a new chat each run".to_string(),
        Some(key) => match target(vault, key) {
            Ok(t) => format!("this chat: “{}” ({key})", t.title),
            Err(_) => format!("the chat {key} (not available now)"),
        },
    }
}

/// The message a run sends: a header naming the automation, then its prompt.
pub(crate) fn turn_text(title: &str, prompt: &str) -> String {
    format!(
        "Scheduled run of the automation “{}”.\n\n{}",
        title.trim(),
        prompt.trim()
    )
}

/// The automation's prompt with its `{{...}}` variables expanded by the same
/// two functions a normal run uses (`workflow_hygiene::template_vars` and
/// `compose_prompt`), so a bound run's turn reads like an unbound run's
/// prompt. A value Hollow took from outside (a trigger reason, a workspace
/// title) is tagged `<hollow-var>`, and the one-line note that it is data
/// follows the prompt, as the boundary block carries it in a normal run.
pub(crate) fn expand_prompt(
    vault: &Path,
    workflow: &Workflow,
    prompt: &str,
    ctx: &crate::workflow_hygiene::RunContext,
    now: DateTime<chrono::Local>,
) -> String {
    let cwd = workflow
        .cwd
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| vault.to_path_buf());
    let vars = crate::workflow_hygiene::template_vars(vault, workflow, &cwd, ctx, now);
    let (text, external) = crate::workflow_hygiene::compose_prompt(prompt, &vars);
    if external {
        format!(
            "{}\n\n{}",
            text.trim_end(),
            crate::workflow_hygiene::HOLLOW_VAR_NOTE
        )
    } else {
        text
    }
}

/// The send's idempotency key, from the automation run it belongs to: the
/// chat row's `clientTurnId`, so a message already queued is never sent
/// again ([`already_sent`]).
pub(crate) fn turn_id(run_id: &str) -> String {
    let clean: String = run_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(70)
        .collect();
    format!("auto-{clean}")
}

/// The chat row carrying `turn_id` since `since`, in any conversation: the
/// chain may have moved on (a handover, the Main agent's new thread) since the
/// send landed, so the current conversation alone would miss it (review of
/// #1331).
fn already_sent(vault: &Path, turn_id: &str, since: &str) -> Result<Option<i64>, String> {
    let since = DateTime::parse_from_rfc3339(since)
        .map(|at| crate::session_store::db_time(at.with_timezone(&Utc) - Duration::minutes(1)))
        .unwrap_or_default();
    crate::run_queue::chat_row_with_turn_id(vault, turn_id, &since)
}

/// The mode the automation's turn runs in (2026-11-12): the chat's own last
/// mode (`main_agent::wake_mode`), lowered when the automation set a mode of
/// its own that is weaker, acceptEdits included. `automation_mode` is the
/// mode its file names ([`named_mode`]); empty when it names none, which
/// never lowers (before, every chat-bound automation ran at most in
/// acceptEdits).
fn turn_mode(vault: &Path, key: &str, automation_mode: &str) -> String {
    let last = crate::session_store::last_permission(vault, key)
        .ok()
        .flatten();
    let chat = crate::main_agent::wake_mode(last.as_deref());
    let named = automation_mode.trim();
    crate::app_mcp::lowered(&chat, (!named.is_empty()).then_some(named)).to_string()
}

/// The permission mode `workflow`'s file names in its frontmatter, or `""`
/// when it names none. `Workflow::permission_mode` can't tell: the parser
/// fills an unset mode with acceptEdits (`vault_workflows::parse_content`).
/// A file Hollow can't read falls back to that parsed mode, so it lowers
/// rather than widens. Saving an automation always writes the key
/// (acceptEdits when the mode was left empty, `vault_workflows::upsert`), so
/// an automation saved from Hollow caps its turn at acceptEdits; only a
/// hand-written file without the key follows the chat's own mode. That
/// safe-side cap is kept on purpose.
pub(crate) fn named_mode(vault: &Path, workflow: &Workflow) -> String {
    let rel = format!(
        "{}/{}.md",
        crate::vault_workflows::WORKFLOWS_DIR,
        workflow.slug
    );
    let content = crate::vault::safe_join(vault, &rel)
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok());
    let Some(content) = content else {
        return workflow.permission_mode.clone();
    };
    let (map, _) = crate::vault_workflows::split_frontmatter(&content);
    map.get("permission_mode")
        .map(|m| m.trim().to_string())
        .unwrap_or_default()
}

/// The chat's agent and the model a turn there asks for.
fn turn_agent(
    vault: &Path,
    target: &Target,
) -> Result<(crate::agent::AgentKind, Option<String>), String> {
    match crate::agent::AgentKind::from_opt(&target.summary.agent) {
        Some(agent) => Ok((
            agent,
            crate::wake_me::wake_model(vault, &target.key, agent.as_str()),
        )),
        None => crate::agent_settings::resolve(None, None),
    }
}

/// The chat turn for `target` carrying `text` (already [`turn_text`]).
pub(crate) fn request(
    vault: &Path,
    target: &Target,
    automation_mode: &str,
    text: String,
    turn_id: String,
) -> Result<crate::chat::ChatSendRequest, String> {
    let s = &target.summary;
    let (agent, model) = turn_agent(vault, target)?;
    Ok(crate::chat::ChatSendRequest {
        prompt: text,
        session_id: s.harness_session_id.clone(),
        session_key: Some(target.key.clone()),
        cwd: PathBuf::from(&s.cwd),
        model,
        permission_mode: Some(turn_mode(vault, &target.key, automation_mode)),
        agent,
        client_turn_id: Some(turn_id),
        app_depth: 0,
        ultracode: false,
        canvases: None,
        continued_from: Vec::new(),
        unattended: true,
        typed: false,
        host_asker: Some(ASKED_BY_AUTOMATION),
        canned: None,
        worker: false,
    })
}

/// Why the automation's usage budget holds a turn now, if it does: the turn
/// counts toward that budget ([`turn_runs`]), so a hard stop holds it too.
fn budget_hold(vault: &Path, slug: &str, target: &Target, now: DateTime<Utc>) -> Option<String> {
    let agent = turn_agent(vault, target).ok().map(|(agent, _)| agent);
    match crate::budgets::preflight_readonly(
        &crate::budgets::Store::for_app(vault),
        Some(vault),
        &crate::budgets::RunIntent::workflow(slug, agent),
        now,
    ) {
        crate::budgets::Preflight::Blocked(notice) => Some(notice.message),
        _ => None,
    }
}

/// Why an automation can't run in its chat as written: tool lists a chat
/// turn would not carry ([`TOOL_LISTS_REFUSED`]).
fn tools_problem(workflow: &Workflow) -> Option<&'static str> {
    (!workflow.allowed_tools.is_empty() || !workflow.disallowed_tools.is_empty())
        .then_some(TOOL_LISTS_REFUSED)
}

/// What a run did with its message.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Delivered {
    /// Queued as chat row `queue_id` (an open card may still hold it).
    Queued { queue_id: i64, title: String },
    /// The chat has a turn queued or running: the message waits here.
    Waiting { title: String },
    /// The database couldn't be read now (busy, unreadable): the message
    /// waits here for the minute pass, which reads the chat again.
    Unread { error: String },
}

impl Delivered {
    /// The run's output line (its run-log note).
    pub(crate) fn note(&self) -> String {
        match self {
            Delivered::Queued { title, .. } => {
                format!("Queued as a new turn in the chat “{title}”.")
            }
            Delivered::Waiting { title } => format!(
                "Waiting behind the running turn in the chat “{title}”; it is sent when that turn ends."
            ),
            Delivered::Unread { error } => format!(
                "Waiting: Hollow couldn't read the chat this automation runs in just now ({error}); it is sent once it can."
            ),
        }
    }
}

/// A run's message waiting behind a running turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Pending {
    pub slug: String,
    pub run_id: String,
    /// The bound key (resolved down its chain again when sent).
    pub bound: String,
    pub permission_mode: String,
    /// The message ([`turn_text`]).
    pub text: String,
    pub since: String,
    /// Why the automation's usage budget held it at the last pass, if it did:
    /// the reason it drops with after [`PENDING_HOURS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<String>,
}

/// A queued automation turn, for budget attribution ([`turn_runs`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SentTurn {
    pub slug: String,
    pub queue_id: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct State {
    #[serde(default)]
    pub pending: Vec<Pending>,
    #[serde(default)]
    pub turns: Vec<SentTurn>,
}

fn path(vault: &Path) -> Option<PathBuf> {
    let stem = crate::session_store::vault_file_stem(vault);
    #[cfg(test)]
    {
        Some(
            crate::test_temp::process_dir("hollow-test-automation-chats-")
                .join(format!("{stem}.json")),
        )
    }
    #[cfg(not(test))]
    {
        crate::instance::support_dir()
            .map(|r| r.join("AutomationChats").join(format!("{stem}.json")))
    }
}

fn read_state(vault: &Path) -> State {
    path(vault)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// The messages waiting now.
pub(crate) fn load(vault: &Path) -> Vec<Pending> {
    let _file = FILE.lock().unwrap_or_else(|e| e.into_inner());
    read_state(vault).pending
}

/// Change the state file under [`FILE`] (read, `change`, write).
fn update<T>(vault: &Path, change: impl FnOnce(&mut State) -> T) -> Result<T, String> {
    let _file = FILE.lock().unwrap_or_else(|e| e.into_inner());
    let mut state = read_state(vault);
    let out = change(&mut state);
    let excess = state.turns.len().saturating_sub(TURNS_KEPT);
    state.turns.drain(..excess);
    let p = path(vault).ok_or("No app data folder.")?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_vec_pretty(&state).map_err(|e| e.to_string())?;
    crate::vault::atomic_write_private(&p, &text)?;
    Ok(out)
}

/// The history run ids of automation `slug`'s chat turns, which its usage
/// budget counts (`budgets::LedgerReader`): a turn's usage row is the chat's,
/// so it also counts in the chat's and the agent's figures as before.
pub(crate) fn turn_runs(vault: &Path, slug: &str) -> HashSet<String> {
    let ids: Vec<i64> = {
        let _file = FILE.lock().unwrap_or_else(|e| e.into_inner());
        read_state(vault)
            .turns
            .into_iter()
            .filter(|t| t.slug == slug)
            .map(|t| t.queue_id)
            .collect()
    };
    crate::run_queue::history_runs(vault, &ids)
        .unwrap_or_default()
        .into_iter()
        .collect()
}

/// Queue `text` in `target` as Hollow's unattended turn, keyed by `turn_id`.
fn send(
    vault: &Path,
    target: &Target,
    mode: &str,
    text: String,
    turn_id: String,
    now: DateTime<Utc>,
) -> Result<i64, String> {
    let req = request(vault, target, mode, text, turn_id)?;
    // `unattended` rides in the row's payload (`chat::queued_unattended`),
    // as for the event lead wake; the process registry is not needed.
    Ok(crate::chat::accept_turn(vault, req, now)?.queue_id)
}

fn remember_turn(vault: &Path, slug: &str, queue_id: i64) {
    let turn = SentTurn {
        slug: slug.to_string(),
        queue_id,
    };
    if let Err(error) = update(vault, |s| s.turns.push(turn)) {
        eprintln!("[automation in chat] {slug}: remember turn: {error}");
    }
}

/// A bound automation's run: queue its message in its chat, or keep it
/// pending behind a running turn or a database that can't be read now (lane
/// Q's re-review of #1343: a busy read failed a new run for good, as a
/// never-retried Prepare failure). `Err` is the run's failure (tool lists,
/// a chat that is gone, archived or in a build workspace, the automation's
/// budget holding it, or the send failed).
pub(crate) fn deliver(
    vault: &Path,
    workflow: &Workflow,
    prompt: &str,
    run_id: &str,
    now: DateTime<Utc>,
) -> Result<Delivered, String> {
    let bound = workflow
        .in_chat
        .as_deref()
        .ok_or("This automation isn't bound to a chat.")?;
    if let Some(problem) = tools_problem(workflow) {
        return Err(problem.into());
    }
    let text = turn_text(&workflow.title, prompt);
    let target = match resolve(vault, bound) {
        Ok(target) => target,
        // Nothing was sent: the minute pass checks `already_sent`, resolves
        // the chat and holds the budget before it sends, as for any pending
        // message.
        Err(Miss::Unreadable(error)) => {
            return one_delivery(&workflow.slug, || {
                park(vault, workflow, bound, text, run_id, now)?;
                Ok(Delivered::Unread { error })
            });
        }
        Err(miss) => return Err(miss.message()),
    };
    if let Some(hold) = budget_hold(vault, &workflow.slug, &target, now) {
        return Err(hold);
    }
    one_delivery(&workflow.slug, || {
        deliver_locked(vault, workflow, bound, target, text, run_id, now)
    })
}

/// [`deliver`]'s send or pending entry, under the automation's lock.
fn deliver_locked(
    vault: &Path,
    workflow: &Workflow,
    bound: &str,
    target: Target,
    text: String,
    run_id: &str,
    now: DateTime<Utc>,
) -> Result<Delivered, String> {
    // A run already waiting for this chat stays first in line.
    let waiting = load(vault).iter().any(|p| p.slug == workflow.slug);
    let sent = if waiting {
        Err(crate::chat::CHAT_ALREADY_QUEUED.to_string())
    } else {
        send(
            vault,
            &target,
            &named_mode(vault, workflow),
            text.clone(),
            turn_id(run_id),
            now,
        )
    };
    match sent {
        Ok(queue_id) => {
            remember_turn(vault, &workflow.slug, queue_id);
            Ok(Delivered::Queued {
                queue_id,
                title: target.title,
            })
        }
        Err(error) if error == crate::chat::CHAT_ALREADY_QUEUED => {
            park(vault, workflow, bound, text, run_id, now)?;
            Ok(Delivered::Waiting {
                title: target.title,
            })
        }
        Err(error) => Err(error),
    }
}

/// Keep run `run_id`'s message pending for the minute pass, replacing any
/// older one of the same automation (its run log says it won't be sent).
/// Under the automation's lock.
fn park(
    vault: &Path,
    workflow: &Workflow,
    bound: &str,
    text: String,
    run_id: &str,
    now: DateTime<Utc>,
) -> Result<(), String> {
    let pending = Pending {
        slug: workflow.slug.clone(),
        run_id: run_id.to_string(),
        bound: bound.to_string(),
        // The mode the file names, `""` for none ([`named_mode`]).
        permission_mode: named_mode(vault, workflow),
        text,
        since: now.to_rfc3339(),
        held: None,
    };
    let mut replaced = Vec::new();
    update(vault, |s| {
        s.pending.retain(|p| {
            let same = p.slug == pending.slug;
            if same {
                replaced.push(p.clone());
            }
            !same
        });
        s.pending.push(pending.clone());
    })?;
    for earlier in &replaced {
        log_replaced(vault, earlier, &pending.run_id, now);
    }
    Ok(())
}

/// The run-log row for a waiting message a newer run replaced: the earlier
/// run's first line said it would be sent, so say it won't be.
fn log_replaced(vault: &Path, earlier: &Pending, newer_run: &str, now: DateTime<Utc>) {
    let entry = RunLogEntry {
        at: now.to_rfc3339(),
        status: crate::vault_workflows::SKIPPED_STATUS.into(),
        duration_sec: 0,
        note: format!(
            "A newer run of this automation ({newer_run}) replaced this run's waiting message, so it will not be sent."
        ),
        run_id: Some(earlier.run_id.clone()),
        error_tail: None,
        exit_kind: None,
    };
    if let Err(error) = crate::vault_workflows::append_run_log(vault, &earlier.slug, entry) {
        eprintln!("[automation in chat] {}: run log: {error}", earlier.slug);
    }
}

/// A dropped message's run-log row.
fn log_dropped(vault: &Path, p: &Pending, why: &str, now: DateTime<Utc>) {
    let entry = RunLogEntry {
        at: now.to_rfc3339(),
        status: "error".into(),
        duration_sec: 0,
        note: format!("not run in its chat: {why}"),
        run_id: Some(p.run_id.clone()),
        error_tail: None,
        exit_kind: None,
    };
    if let Err(error) = crate::vault_workflows::append_run_log(vault, &p.slug, entry) {
        eprintln!("[automation in chat] {}: run log: {error}", p.slug);
    }
}

/// Whether an automation's file is known to be gone, from
/// `Path::try_exists`: an error (permissions, metadata) is unknown, `None`.
fn known_gone(found: std::io::Result<bool>) -> Option<bool> {
    found.ok().map(|exists| !exists)
}

/// Whether pending `p`'s automation still runs in its chat: `Some(false)`
/// only when that is established (the file is gone, or it reads with another
/// binding or none); `None` when it couldn't be read or checked now (kept
/// for later).
fn still_bound(vault: &Path, p: &Pending) -> Option<bool> {
    let rel = format!("{}/{}.md", crate::vault_workflows::WORKFLOWS_DIR, p.slug);
    match crate::vault::safe_join(vault, &rel).map(|file| known_gone(file.try_exists())) {
        Ok(Some(true)) => return Some(false),
        Ok(Some(false)) => {}
        Ok(None) | Err(_) => return None,
    }
    match crate::vault_workflows::read_full(vault, &p.slug) {
        Ok((w, _, _)) => Some(w.in_chat.as_deref() == Some(p.bound.as_str())),
        Err(_) => None,
    }
}

/// What one pending message's attempt came to.
enum Attempt {
    Sent(i64),
    /// Stays pending; `held` is the budget hold's notice to remember.
    Keep {
        held: Option<String>,
    },
    Forget,
    Drop(String),
}

fn attempt(vault: &Path, p: &Pending, now: DateTime<Utc>) -> Attempt {
    let keep = || Attempt::Keep {
        held: p.held.clone(),
    };
    let age = DateTime::parse_from_rfc3339(&p.since).map(|at| now - at.with_timezone(&Utc));
    let id = turn_id(&p.run_id);
    // Sent already (Hollow stopped before the entry was removed): never
    // twice. Checked first, so a send that landed before a long outage is
    // recorded as sent, not dropped as waiting too long.
    match already_sent(vault, &id, &p.since) {
        Ok(Some(queue_id)) => return Attempt::Sent(queue_id),
        Ok(None) => {}
        Err(error) => {
            if age.map_or(true, |age| age > Duration::hours(CHECK_CEILING_HOURS)) {
                return Attempt::Drop(format!(
                    "Hollow could not check whether it was already sent for {CHECK_CEILING_HOURS} hours: {error}"
                ));
            }
            eprintln!("[automation in chat] {} waits: {error}", p.slug);
            return keep();
        }
    }
    let bound = still_bound(vault, p);
    if bound == Some(false) {
        return Attempt::Forget;
    }
    // Past the cap it drops even while its file can't be read now, with the
    // budget's notice when the budget is what held it.
    let stale = age.map_or(true, |age| age > Duration::hours(PENDING_HOURS));
    if stale {
        return Attempt::Drop(
            p.held
                .clone()
                .unwrap_or_else(|| format!("waited over {PENDING_HOURS} hours and was never sent")),
        );
    }
    if bound.is_none() {
        return keep();
    }
    let target = match resolve(vault, &p.bound) {
        Ok(target) => target,
        Err(Miss::Unavailable(why)) => return Attempt::Drop(why),
        Err(Miss::Unreadable(error)) => {
            eprintln!("[automation in chat] {} waits: {error}", p.slug);
            return keep();
        }
    };
    if let Some(notice) = budget_hold(vault, &p.slug, &target, now) {
        return Attempt::Keep { held: Some(notice) };
    }
    match send(vault, &target, &p.permission_mode, p.text.clone(), id, now) {
        Ok(queue_id) => Attempt::Sent(queue_id),
        Err(error) => {
            if error != crate::chat::CHAT_ALREADY_QUEUED {
                eprintln!("[automation in chat] {} waits: {error}", p.slug);
            }
            Attempt::Keep { held: None }
        }
    }
}

/// Send each pending message whose chat is free now. One whose chat is gone
/// or archived, or that waited over [`PENDING_HOURS`], is dropped with an
/// `error` run-log row; a busy chat, a budget hold or a passing failure
/// keeps it. Each send carries its run's [`turn_id`], so a message whose
/// entry outlived its send is recognised and not sent again. Returns the
/// queued row ids.
pub(crate) fn deliver_pending(vault: &Path, now: DateTime<Utc>) -> Vec<i64> {
    let mut queued = Vec::new();
    for p in load(vault) {
        if let Some(id) = one_delivery(&p.slug, || deliver_one(vault, &p, now)) {
            queued.push(id);
        }
    }
    queued
}

/// One pending message's attempt, under its automation's lock. A Run now
/// that replaced it since the list was read wins: only the same entry (slug
/// and run id) still pending is sent.
fn deliver_one(vault: &Path, p: &Pending, now: DateTime<Utc>) -> Option<i64> {
    if !load(vault)
        .iter()
        .any(|q| q.slug == p.slug && q.run_id == p.run_id)
    {
        return None;
    }
    let outcome = attempt(vault, p, now);
    let (remove, turn) = match &outcome {
        Attempt::Sent(id) => (true, Some(*id)),
        Attempt::Keep { .. } => (false, None),
        Attempt::Forget | Attempt::Drop(_) => (true, None),
    };
    if let Attempt::Keep { held } = &outcome {
        if *held != p.held {
            let saved = update(vault, |s| {
                for q in s.pending.iter_mut() {
                    if q.slug == p.slug && q.run_id == p.run_id {
                        q.held = held.clone();
                    }
                }
            });
            if let Err(error) = saved {
                eprintln!("[automation in chat] pending: {error}");
            }
        }
    }
    if remove {
        let saved = update(vault, |s| {
            s.pending
                .retain(|q| !(q.slug == p.slug && q.run_id == p.run_id));
            if let Some(queue_id) = turn {
                if !s.turns.iter().any(|t| t.queue_id == queue_id) {
                    s.turns.push(SentTurn {
                        slug: p.slug.clone(),
                        queue_id,
                    });
                }
            }
        });
        if let Err(error) = saved {
            eprintln!("[automation in chat] pending: {error}");
        }
    }
    match outcome {
        Attempt::Sent(id) => Some(id),
        Attempt::Drop(why) => {
            log_dropped(vault, p, &why, now);
            None
        }
        Attempt::Keep { .. } | Attempt::Forget => None,
    }
}

/// Start the minute pass (the queue's owner only).
pub(crate) fn start(config: std::sync::Arc<crate::config::Config>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            let vault = config.vault_path.clone();
            let queued =
                tauri::async_runtime::spawn_blocking(move || deliver_pending(&vault, Utc::now()))
                    .await
                    .unwrap_or_default();
            if !queued.is_empty() {
                crate::run_queue::wake();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_queue::{self, RowStatus, RunSource};

    fn vault() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("vault-").tempdir().unwrap()
    }

    /// A conversation with one finished turn in the vault folder.
    fn chat(v: &Path, key: &str, title: &str) {
        let cwd = v.to_string_lossy().into_owned();
        let run = format!("run-{key}");
        let ord = crate::session_store::start_turn(v, key, "claude", &cwd, &run, title).unwrap();
        crate::session_store::finish_turn(v, key, ord, "success").unwrap();
    }

    fn bound(slug: &str, key: &str) -> Workflow {
        let mut w = crate::vault_workflows::parse_content(
            slug,
            "---\ntitle: Nightly check\nschedule: manual\npermission_mode: bypassPermissions\n---\nCheck the board.\n",
        )
        .unwrap()
        .0;
        w.slug = slug.into();
        w.in_chat = Some(key.into());
        w
    }

    fn input(title: &str, in_chat: Option<&str>) -> crate::types::WorkflowInput {
        crate::types::WorkflowInput {
            title: title.into(),
            description: String::new(),
            schedule: "manual".into(),
            agent: None,
            model: None,
            cwd: None,
            output_glob: None,
            permission_mode: "acceptEdits".into(),
            allowed_tools: vec![],
            disallowed_tools: vec![],
            prompt: "Check the board.".into(),
            edit_base: None,
            notify: None,
            skip_if_quiet: None,
            watch: None,
            precheck: None,
            triggers: None,
            in_chat: in_chat.map(str::to_string),
            ends: None,
            after_runs: None,
            runs_on: None,
        }
    }

    /// An automation saved in the vault, bound to `key`.
    fn saved(v: &Path, title: &str, key: &str) -> Workflow {
        crate::vault_workflows::upsert_input(v, None, &input(title, Some(key))).unwrap()
    }

    #[test]
    fn the_binding_parses_renders_and_survives_an_editor_save() {
        let dir = vault();
        let v = dir.path();
        chat(v, "s-20261112-f1bind01", "Lead");
        let input = |in_chat: Option<&str>| input("Board check", in_chat);
        let saved =
            crate::vault_workflows::upsert_input(v, None, &input(Some("s-20261112-f1bind01")))
                .unwrap();
        assert_eq!(saved.in_chat.as_deref(), Some("s-20261112-f1bind01"));
        let text = std::fs::read_to_string(v.join(&saved.path)).unwrap();
        assert!(text.contains("in_chat: s-20261112-f1bind01\n"), "{text}");
        // An editor save that doesn't send it keeps it.
        let kept =
            crate::vault_workflows::upsert_input(v, Some(&saved.slug), &input(None)).unwrap();
        assert_eq!(kept.in_chat.as_deref(), Some("s-20261112-f1bind01"));
        // The binding is in the permission stamp: a hand edit to another
        // chat is a change to review, never a silent retarget.
        let fields = crate::workflow_permissions::PermissionFields::from_workflow(&kept, "x");
        assert_eq!(fields.in_chat.as_deref(), Some("s-20261112-f1bind01"));
        let moved = crate::workflow_permissions::PermissionFields {
            in_chat: Some("s-20261112-other001".into()),
            ..fields.clone()
        };
        assert!(crate::workflow_permissions::PermissionFields::escalation(
            Some(&fields),
            &moved
        ));
        assert!(
            crate::workflow_permissions::PermissionFields::changed_fields(Some(&fields), &moved)
                .contains(&"runs in")
        );
        // An unbound stamp keeps its pre-F1 digest (no `inChat` key).
        let unbound = crate::workflow_permissions::PermissionFields {
            in_chat: None,
            ..fields
        };
        assert!(!serde_json::to_string(&unbound).unwrap().contains("inChat"));
        // Empty unbinds.
        let cleared =
            crate::vault_workflows::upsert_input(v, Some(&kept.slug), &input(Some(""))).unwrap();
        assert_eq!(cleared.in_chat, None);
        let text = std::fs::read_to_string(v.join(&cleared.path)).unwrap();
        assert!(!text.contains("in_chat"), "{text}");
        // A key with no conversation, or an archived one, is refused.
        let gone = crate::vault_workflows::upsert_input(
            v,
            Some(&cleared.slug),
            &input(Some("s-20261112-nochat01")),
        )
        .unwrap_err();
        assert!(
            gone.contains("deleted") && gone.ends_with("Nothing changed."),
            "{gone}"
        );
        crate::session_store::archive_session(v, "s-20261112-f1bind01", true).unwrap();
        let archived = crate::vault_workflows::upsert_input(
            v,
            Some(&cleared.slug),
            &input(Some("s-20261112-f1bind01")),
        )
        .unwrap_err();
        assert!(archived.contains("archived"), "{archived}");
    }

    /// 2026-11-12: an automation's turn keeps the chat's own mode (bypass
    /// and auto kept); a mode the automation's file names lowers it when
    /// weaker (acceptEdits included); none named never does.
    #[test]
    fn an_automation_turn_keeps_the_chats_own_mode() {
        use crate::app_mcp::{perm_rank, PERMISSION_MODES};
        let dir = vault();
        let v = dir.path();
        let key = "s-20261112-f1mode01";
        chat(v, key, "Modes");
        assert_eq!(turn_mode(v, key, ""), "acceptEdits", "none recorded");
        assert_eq!(turn_mode(v, key, "bypassPermissions"), "acceptEdits");
        assert_eq!(turn_mode(v, key, "plan"), "plan");
        let cwd = v.to_string_lossy().into_owned();
        let ord =
            crate::session_store::start_turn(v, key, "claude", &cwd, "run-mode", "x").unwrap();
        for own in PERMISSION_MODES {
            crate::session_store::record_turn_permission(v, key, ord, own).unwrap();
            assert_eq!(turn_mode(v, key, ""), own, "unset");
            for named in PERMISSION_MODES {
                let want = if perm_rank(named) < perm_rank(own) {
                    named
                } else {
                    own
                };
                assert_eq!(turn_mode(v, key, named), want, "{own} with {named}");
            }
        }
        crate::session_store::finish_turn(v, key, ord, "success").unwrap();
    }

    /// Review 2026-11-12, end to end through `deliver`: in a bypass chat, an
    /// automation whose file names acceptEdits runs in acceptEdits, one that
    /// names no mode (it parses as acceptEdits) runs in bypass, one naming
    /// plan runs in plan, and one naming bypass runs in bypass.
    #[test]
    fn an_automation_set_to_accept_edits_lowers_a_bypass_chat_and_an_unset_one_doesnt() {
        let dir = vault();
        let v = dir.path();
        let cwd = v.to_string_lossy().into_owned();
        for (n, line, want) in [
            (1, "permission_mode: acceptEdits\n", "acceptEdits"),
            (2, "", "bypassPermissions"),
            (3, "permission_mode: plan\n", "plan"),
            (
                4,
                "permission_mode: bypassPermissions\n",
                "bypassPermissions",
            ),
        ] {
            let key = format!("s-20261112-f1named{n}");
            let ord = crate::session_store::start_turn(
                v,
                &key,
                "claude",
                &cwd,
                &format!("run-n{n}"),
                "x",
            )
            .unwrap();
            crate::session_store::record_turn_permission(v, &key, ord, "bypassPermissions")
                .unwrap();
            crate::session_store::finish_turn(v, &key, ord, "success").unwrap();
            let slug = format!("f1-named-{n}");
            let text = format!("---\ntitle: Nightly check\nschedule: manual\n{line}in_chat: {key}\n---\nCheck the board.\n");
            std::fs::create_dir_all(v.join("workflows")).unwrap();
            std::fs::write(v.join(format!("workflows/{slug}.md")), &text).unwrap();
            let (mut w, _, _) = crate::vault_workflows::parse_content(&slug, &text).unwrap();
            w.slug = slug.clone();
            w.in_chat = Some(key.clone());
            if n <= 2 {
                assert_eq!(w.permission_mode, "acceptEdits", "parsed alike, set or not");
            }
            let done = deliver(
                v,
                &w,
                "Check the board.",
                &format!("run-f1-n{n}"),
                Utc::now(),
            )
            .unwrap();
            let Delivered::Queued { queue_id, .. } = done else {
                panic!("{done:?}");
            };
            let row = run_queue::get(v, queue_id).unwrap().unwrap();
            assert_eq!(row.payload["permissionMode"], want, "{slug}");
        }
    }

    #[test]
    fn a_run_follows_the_handover_chain_to_its_current_chat() {
        let dir = vault();
        let v = dir.path();
        chat(v, "s-20261112-f1chain1", "Lead");
        chat(v, "s-20261112-f1chain2", "Lead, continued");
        crate::app_sessions::record_handover(v, "s-20261112-f1chain1", "s-20261112-f1chain2", 1)
            .unwrap();
        let t = target(v, "s-20261112-f1chain1").unwrap();
        assert_eq!(t.key, "s-20261112-f1chain2");
        assert_eq!(t.title, "Lead, continued");

        let w = bound("f1-chain", "s-20261112-f1chain1");
        let now = Utc::now();
        let done = deliver(v, &w, "Check the board.", "run-f1-chain", now).unwrap();
        let Delivered::Queued { queue_id, .. } = done else {
            panic!("{done:?}");
        };
        let row = run_queue::get(v, queue_id).unwrap().unwrap();
        assert_eq!(row.source, RunSource::Chat);
        assert_eq!(row.source_id.as_deref(), Some("s-20261112-f1chain2"));
        assert_eq!(
            row.payload["hostAsker"],
            serde_json::json!(ASKED_BY_AUTOMATION)
        );
        assert_eq!(row.payload["unattended"], serde_json::json!(true));
        let prompt = row.payload["prompt"].as_str().unwrap_or_default();
        assert!(prompt.contains("Check the board."), "{prompt}");
        assert!(prompt.contains("“Nightly check”"), "{prompt}");
        // The chat's own mode (none recorded: Hollow's default), never
        // above it, whatever the automation's own mode (bypass here).
        let mode = row.payload["permissionMode"].as_str().unwrap_or_default();
        assert_eq!(mode, "acceptEdits");
    }

    #[test]
    fn a_gone_or_archived_chat_fails_the_run_with_the_reason() {
        let dir = vault();
        let v = dir.path();
        let now = Utc::now();
        let w = bound("f1-gone", "s-20261112-f1gone01");
        let gone = deliver(v, &w, "Check.", "run-f1-gone", now).unwrap_err();
        assert!(gone.contains("deleted"), "{gone}");
        chat(v, "s-20261112-f1arch01", "Old lead");
        crate::session_store::archive_session(v, "s-20261112-f1arch01", true).unwrap();
        let w = bound("f1-arch", "s-20261112-f1arch01");
        let archived = deliver(v, &w, "Check.", "run-f1-arch", now).unwrap_err();
        assert!(
            archived.contains("“Old lead”") && archived.contains("archived"),
            "{archived}"
        );
        assert!(
            run_queue::live_rows(v).unwrap().is_empty(),
            "nothing queued"
        );
    }

    #[test]
    fn a_run_queues_behind_an_open_card_and_never_closes_it() {
        let dir = vault();
        let v = dir.path();
        let key = "s-20261112-f1card01";
        let cwd = v.to_string_lossy().into_owned();
        let ord =
            crate::session_store::start_turn(v, key, "claude", &cwd, "run-card", "ask").unwrap();
        let arguments = Ok(serde_json::json!({
            "mode": "confirmation",
            "supersedeOnUserMessage": true,
            "confirmation": { "prompt": "Go?" }
        }));
        let card = crate::interactions::record(
            v,
            &crate::interactions::NewInteraction {
                session_key: key,
                turn_ord: ord,
                run_id: "run-card",
                agent: crate::agent::AgentKind::Claude,
                native_session_id: None,
                tool_use_id: "toolu-f1card",
                source: crate::interactions::InteractionSource::Mcp,
                arguments: &arguments,
                cwd: v,
                vault_root: v,
            },
        )
        .unwrap()
        .interaction;
        crate::session_store::finish_turn(v, key, ord, "success").unwrap();
        let now = Utc::now();
        let w = bound("f1-card", key);
        let Delivered::Queued { queue_id, .. } =
            deliver(v, &w, "Check the board.", "run-f1-card", now).unwrap()
        else {
            panic!("queued");
        };
        let row = run_queue::get(v, queue_id).unwrap().unwrap();
        assert!(!run_queue::payload_closes_cards(&row.payload));
        assert!(crate::interactions::list_pending(v, key)
            .unwrap()
            .iter()
            .any(|c| c.id == card.id));
        assert!(crate::interactions::open_hold_until(v, key, now).is_some());
        let live = run_queue::live_rows(v).unwrap();
        let facts = crate::run_pump::QueueFacts::read(v, &live);
        let queued = live.iter().find(|r| r.id == queue_id).unwrap();
        assert_eq!(queued.status, RowStatus::Queued);
        assert_eq!(
            crate::run_pump::item_with_reason(v, queued, Some(1), &facts, now)
                .waiting_reason
                .as_deref(),
            Some(crate::run_pump::CARD_HOLD_WAIT),
            "the open card holds it"
        );
    }

    #[test]
    fn a_run_waits_behind_a_running_turn_and_goes_when_it_ends() {
        let dir = vault();
        let v = dir.path();
        let key = "s-20261112-f1busy01";
        chat(v, key, "Busy lead");
        let now = Utc::now();
        let w = saved(v, "Busy check", key);
        let first = deliver(v, &w, "First.", "run-f1-busy-1", now).unwrap();
        let Delivered::Queued { queue_id, .. } = first else {
            panic!("{first:?}");
        };
        // The turn is still queued: the next run waits, never beside it.
        let second = deliver(v, &w, "Second.", "run-f1-busy-2", now).unwrap();
        assert!(matches!(second, Delivered::Waiting { .. }), "{second:?}");
        assert!(second.note().contains("Waiting behind"));
        assert_eq!(load(v).len(), 1);
        assert!(deliver_pending(v, now).is_empty(), "still busy");
        assert_eq!(load(v).len(), 1, "kept");
        // A third run replaces the waiting one: one per automation.
        deliver(v, &w, "Third.", "run-f1-busy-3", now).unwrap();
        let waiting = load(v);
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].run_id, "run-f1-busy-3");
        let (_, _, log) = crate::vault_workflows::read_full(v, &w.slug).unwrap();
        let replaced = log
            .iter()
            .find(|e| e.run_id.as_deref() == Some("run-f1-busy-2"))
            .expect("the replaced run's log says so");
        assert!(replaced.note.contains("replaced"), "{}", replaced.note);
        assert!(
            replaced.note.contains("will not be sent"),
            "{}",
            replaced.note
        );
        assert!(replaced.note.contains("run-f1-busy-3"), "{}", replaced.note);
        // The turn ends: the waiting message goes.
        run_queue::finish(v, queue_id, RowStatus::Done, None, now).unwrap();
        let sent = deliver_pending(v, now);
        assert_eq!(sent.len(), 1);
        assert!(load(v).is_empty());
        let row = run_queue::get(v, sent[0]).unwrap().unwrap();
        assert!(row.payload["prompt"]
            .as_str()
            .unwrap_or_default()
            .contains("Third."));
    }

    #[test]
    fn a_waiting_message_is_dropped_into_the_run_log_when_its_chat_goes() {
        let dir = vault();
        let v = dir.path();
        let key = "s-20261112-f1drop01";
        chat(v, key, "Lead");
        let now = Utc::now();
        let w = saved(v, "Gone chat check", key);
        let Delivered::Queued { queue_id, .. } =
            deliver(v, &w, "First.", "run-f1-drop-1", now).unwrap()
        else {
            panic!("queued");
        };
        deliver(v, &w, "Second.", "run-f1-drop-2", now).unwrap();
        // The running turn ends and the person archives the chat before
        // the minute pass sends the waiting message.
        run_queue::finish(v, queue_id, RowStatus::Done, None, now).unwrap();
        crate::session_store::archive_session(v, key, true).unwrap();
        assert!(deliver_pending(v, now).is_empty());
        assert!(load(v).is_empty(), "dropped");
        let (_, _, log) = crate::vault_workflows::read_full(v, &w.slug).unwrap();
        let last = log.last().unwrap();
        assert_eq!(last.status, "error");
        assert!(last.note.contains("archived"), "{}", last.note);
        assert_eq!(last.run_id.as_deref(), Some("run-f1-drop-2"));
    }

    #[test]
    fn a_message_waiting_too_long_is_dropped() {
        let dir = vault();
        let v = dir.path();
        let key = "s-20261112-f1late01";
        chat(v, key, "Lead");
        let now = Utc::now();
        let w = saved(v, "Late check", key);
        deliver(v, &w, "First.", "run-f1-late-1", now).unwrap();
        deliver(v, &w, "Second.", "run-f1-late-2", now).unwrap();
        assert_eq!(load(v).len(), 1);
        let later = now + Duration::hours(PENDING_HOURS) + Duration::minutes(1);
        assert!(deliver_pending(v, later).is_empty());
        assert!(load(v).is_empty());
    }

    #[test]
    fn a_waiting_message_of_an_unbound_automation_is_dropped_quietly() {
        let dir = vault();
        let v = dir.path();
        let key = "s-20261112-f1unbd01";
        chat(v, key, "Lead");
        let now = Utc::now();
        let w = saved(v, "Unbound check", key);
        deliver(v, &w, "First.", "run-f1-unbd-1", now).unwrap();
        deliver(v, &w, "Second.", "run-f1-unbd-2", now).unwrap();
        assert_eq!(load(v).len(), 1);
        crate::vault_workflows::upsert_input(v, Some(&w.slug), &input("Unbound check", Some("")))
            .unwrap();
        assert!(deliver_pending(v, now).is_empty());
        assert!(load(v).is_empty());
        let (_, _, log) = crate::vault_workflows::read_full(v, &w.slug).unwrap();
        assert!(log.iter().all(|e| e.status != "error"), "no failure row");
    }

    /// Review B1: a message whose send landed but whose entry outlived it
    /// (Hollow stopped in between) is recognised by its run's turn id and
    /// never sent twice.
    #[test]
    fn a_waiting_message_already_sent_is_never_sent_again() {
        let dir = vault();
        let v = dir.path();
        let key = "s-20261112-f1dedu01";
        chat(v, key, "Lead");
        let now = Utc::now();
        let w = saved(v, "Dedupe check", key);
        let Delivered::Queued { queue_id, .. } =
            deliver(v, &w, "First.", "run-f1-dedu-1", now).unwrap()
        else {
            panic!("queued");
        };
        assert_eq!(
            run_queue::get(v, queue_id).unwrap().unwrap().payload["clientTurnId"],
            serde_json::json!(turn_id("run-f1-dedu-1"))
        );
        deliver(v, &w, "Second.", "run-f1-dedu-2", now).unwrap();
        run_queue::finish(v, queue_id, RowStatus::Done, None, now).unwrap();
        // The pending message's send lands, then Hollow stops before the
        // entry is removed.
        let t = target(v, key).unwrap();
        let landed = send(
            v,
            &t,
            "acceptEdits",
            "Second.".into(),
            turn_id("run-f1-dedu-2"),
            now,
        )
        .unwrap();
        assert_eq!(load(v).len(), 1, "the entry outlived the send");
        let sent = deliver_pending(v, now);
        assert_eq!(sent, vec![landed], "recognised, not sent again");
        assert!(load(v).is_empty());
        let rows = run_queue::chat_rows_since(v, key, "").unwrap();
        assert_eq!(rows.len(), 2, "the first turn and the one send");
    }

    /// Review B2: a chat turn doesn't carry an automation's tool lists, so a
    /// bound automation can't keep them: the save refuses, and a hand edit
    /// fails its run.
    #[test]
    fn a_bound_automation_with_tool_lists_is_refused() {
        let dir = vault();
        let v = dir.path();
        let key = "s-20261112-f1tool01";
        chat(v, key, "Lead");
        let mut narrow = input("Read only check", Some(key));
        narrow.allowed_tools = vec!["Read".into()];
        let error = crate::vault_workflows::upsert_input(v, None, &narrow).unwrap_err();
        assert_eq!(error, TOOL_LISTS_REFUSED);
        let mut denied = input("Deny check", Some(key));
        denied.disallowed_tools = vec!["Bash".into()];
        assert_eq!(
            crate::vault_workflows::upsert_input(v, None, &denied).unwrap_err(),
            TOOL_LISTS_REFUSED
        );
        // Binding an existing automation that has tool lists is refused too.
        let mut plain = input("Listed check", None);
        plain.allowed_tools = vec!["Read".into()];
        let listed = crate::vault_workflows::upsert_input(v, None, &plain).unwrap();
        let mut bind = plain.clone();
        bind.in_chat = Some(key.into());
        assert_eq!(
            crate::vault_workflows::upsert_input(v, Some(&listed.slug), &bind).unwrap_err(),
            TOOL_LISTS_REFUSED
        );
        // A hand edit that binds one anyway fails its run, sending nothing.
        let mut w = bound("f1-tools", key);
        w.allowed_tools = vec!["Read".into()];
        let error = deliver(v, &w, "Check.", "run-f1-tools", Utc::now()).unwrap_err();
        assert_eq!(error, TOOL_LISTS_REFUSED);
        assert!(run_queue::live_rows(v).unwrap().is_empty());
    }

    /// Review M1: the automation's chat turns count toward its own usage
    /// budget, and only toward it.
    #[test]
    fn a_chat_turns_usage_counts_toward_its_automations_budget() {
        let dir = vault();
        let v = dir.path();
        let key = "s-20261112-f1budg01";
        chat(v, key, "Lead");
        let now = Utc::now();
        let w = saved(v, "Budget check", key);
        let Delivered::Queued { queue_id, .. } =
            deliver(v, &w, "Check.", "run-f1-budg", now).unwrap()
        else {
            panic!("queued");
        };
        run_queue::claim(v, queue_id, now).unwrap().unwrap();
        assert!(run_queue::mark_running(v, queue_id, "R-f1-budg-turn", now).unwrap());
        assert!(turn_runs(v, &w.slug).contains("R-f1-budg-turn"));
        let at = crate::session_store::db_time(now);
        let mut row = crate::session_store::tests::usage_row_fixture("k-f1-budg", &at, "success");
        row.run_id = Some("R-f1-budg-turn".into());
        row.scope = Some(key.into());
        row.input_tokens = Some(1000);
        crate::session_store::upsert_usage(v, &row).unwrap();
        let rows = crate::session_store::usage_rows_since(v, "").unwrap();
        let ours = crate::budgets::Scope::Workflow(w.slug.clone());
        let counted = crate::budgets::scope_rows(v, &rows, &ours);
        assert!(counted.iter().any(|r| ours.matches_row(r)), "counts for it");
        let other = crate::budgets::Scope::Workflow("another-automation".into());
        let elsewhere = crate::budgets::scope_rows(v, &rows, &other);
        assert!(!elsewhere.iter().any(|r| other.matches_row(r)));
        // The row stays the chat's for every other figure.
        assert_eq!(rows[0].surface, "chat");
    }

    /// Review M3: a pending message is dropped only when its automation is
    /// known to be gone or unbound; an unreadable file keeps it.
    #[test]
    fn a_waiting_message_survives_an_unreadable_automation_file() {
        let dir = vault();
        let v = dir.path();
        let key = "s-20261112-f1read01";
        chat(v, key, "Lead");
        let now = Utc::now();
        let w = saved(v, "Read check", key);
        deliver(v, &w, "First.", "run-f1-read-1", now).unwrap();
        deliver(v, &w, "Second.", "run-f1-read-2", now).unwrap();
        let file = v.join(&w.path);
        let good = std::fs::read_to_string(&file).unwrap();
        std::fs::write(&file, "---\nschedule: manual\n---\nno title\n").unwrap();
        deliver_pending(v, now);
        assert_eq!(load(v).len(), 1, "kept while it can't be read");
        std::fs::write(&file, good).unwrap();
        std::fs::remove_file(&file).unwrap();
        deliver_pending(v, now);
        assert!(load(v).is_empty(), "dropped once it is gone");
    }

    /// Lane T's re-review of #1313: the send landed and its turn already
    /// finished (which compacts the row's payload) before the pending entry
    /// was removed. The row keeps its `auto-` turn id, so the message is
    /// still recognised as sent and never sent again.
    #[test]
    fn a_waiting_message_whose_turn_already_finished_is_never_sent_again() {
        let dir = vault();
        let v = dir.path();
        let key = "s-20261112-f1fini01";
        chat(v, key, "Lead");
        let now = Utc::now();
        let w = saved(v, "Finished check", key);
        let Delivered::Queued { queue_id, .. } =
            deliver(v, &w, "First.", "run-f1-fini-1", now).unwrap()
        else {
            panic!("queued");
        };
        deliver(v, &w, "Second.", "run-f1-fini-2", now).unwrap();
        run_queue::finish(v, queue_id, RowStatus::Done, None, now).unwrap();
        let t = target(v, key).unwrap();
        let landed = send(
            v,
            &t,
            "acceptEdits",
            "Second.".into(),
            turn_id("run-f1-fini-2"),
            now,
        )
        .unwrap();
        run_queue::finish(v, landed, RowStatus::Done, None, now).unwrap();
        assert_eq!(
            run_queue::get(v, landed).unwrap().unwrap().payload,
            serde_json::json!({"unattended": true, "clientTurnId": turn_id("run-f1-fini-2")})
        );
        assert_eq!(load(v).len(), 1, "the entry outlived the send");
        assert_eq!(deliver_pending(v, now), vec![landed], "recognised as sent");
        assert!(load(v).is_empty());
        let rows = run_queue::chat_rows_since(v, key, "").unwrap();
        assert_eq!(rows.len(), 2, "the first turn and the one send");
    }

    /// Lane T's re-review: a Run now that replaced a pending message after
    /// the minute pass read the list wins; the old message is never sent.
    #[test]
    fn a_pending_message_replaced_after_the_pass_read_it_is_not_sent() {
        let dir = vault();
        let v = dir.path();
        let key = "s-20261112-f1supr01";
        chat(v, key, "Lead");
        let now = Utc::now();
        let w = saved(v, "Superseded check", key);
        let Delivered::Queued { queue_id, .. } =
            deliver(v, &w, "First.", "run-f1-supr-1", now).unwrap()
        else {
            panic!("queued");
        };
        deliver(v, &w, "Second.", "run-f1-supr-2", now).unwrap();
        // The pass reads the list, then a Run now replaces the entry.
        let read = load(v).remove(0);
        deliver(v, &w, "Third.", "run-f1-supr-3", now).unwrap();
        run_queue::finish(v, queue_id, RowStatus::Done, None, now).unwrap();
        assert_eq!(one_delivery(&w.slug, || deliver_one(v, &read, now)), None);
        assert_eq!(run_queue::chat_rows_since(v, key, "").unwrap().len(), 1);
        let waiting = load(v);
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].run_id, "run-f1-supr-3", "the newer one waits");
        // A delivery forgets its lock once nobody holds or waits for it.
        assert!(!PER_AUTOMATION.lock().unwrap().contains_key(w.slug.as_str()));
    }

    /// Holders of `slug`'s delivery lock: the map's copy plus each delivery's.
    fn holders(slug: &str) -> usize {
        PER_AUTOMATION
            .lock()
            .unwrap()
            .get(slug)
            .map_or(0, Arc::strong_count)
    }

    /// Review of #1331: overlapping deliveries for one automation leave no
    /// lock entry behind, whichever finishes first. The second delivery is
    /// started from inside the first and the first waits until the second
    /// holds its clone (waiting on the lock), so they always overlap; then
    /// the other way round, the second already holding the lock while the
    /// first waits.
    #[test]
    fn overlapping_deliveries_forget_their_lock() {
        for round in 0..50 {
            let slug = format!("overlap-a-{round}");
            let (s, b_slug) = (slug.clone(), slug.clone());
            one_delivery(&slug, move || {
                let b = std::thread::spawn(move || one_delivery(&b_slug, || ()));
                while holders(&s) < 3 {
                    std::thread::yield_now();
                }
                b
            })
            .join()
            .unwrap();
            assert!(!PER_AUTOMATION.lock().unwrap().contains_key(&slug));

            let slug = format!("overlap-b-{round}");
            let (holding_tx, holding_rx) = std::sync::mpsc::channel();
            let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
            let b_slug = slug.clone();
            let b = std::thread::spawn(move || {
                one_delivery(&b_slug, || {
                    holding_tx.send(()).unwrap();
                    go_rx.recv().unwrap();
                })
            });
            holding_rx.recv().unwrap();
            let a_slug = slug.clone();
            let a = std::thread::spawn(move || one_delivery(&a_slug, || ()));
            while holders(&slug) < 3 {
                std::thread::yield_now();
            }
            go_tx.send(()).unwrap();
            a.join().unwrap();
            b.join().unwrap();
            assert!(!PER_AUTOMATION.lock().unwrap().contains_key(&slug));
        }
    }

    /// Lane T's re-review: only a confirmed missing file drops a pending
    /// message; an error checking it (permissions, metadata) keeps it.
    #[test]
    fn an_error_checking_the_automation_file_keeps_the_message() {
        assert_eq!(known_gone(Ok(false)), Some(true));
        assert_eq!(known_gone(Ok(true)), Some(false));
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(known_gone(Err(denied)), None);
    }

    /// Lane T's re-review: an entry past [`PENDING_HOURS`] drops (with its
    /// run-log row when it can be written) even while its automation's file
    /// can't be read, instead of being retried every minute.
    #[test]
    fn an_expired_message_drops_while_its_file_is_unreadable() {
        let dir = vault();
        let v = dir.path();
        let key = "s-20261112-f1expr01";
        chat(v, key, "Lead");
        let now = Utc::now();
        let w = saved(v, "Expired check", key);
        deliver(v, &w, "First.", "run-f1-expr-1", now).unwrap();
        deliver(v, &w, "Second.", "run-f1-expr-2", now).unwrap();
        std::fs::write(v.join(&w.path), "---\nschedule: manual\n---\nno title\n").unwrap();
        deliver_pending(v, now);
        assert_eq!(load(v).len(), 1, "kept while young");
        let later = now + Duration::hours(PENDING_HOURS) + Duration::minutes(1);
        assert!(deliver_pending(v, later).is_empty());
        assert!(load(v).is_empty(), "dropped past the cap");
    }

    /// Follow-up to #1313: a bound run's turn carries the prompt as a normal
    /// run reads it, with the same variables expanded (`compose_prompt`).
    #[test]
    fn a_bound_runs_prompt_variables_are_expanded_like_a_normal_runs() {
        use crate::workflow_hygiene::{compose_prompt, template_vars, RunContext};
        let dir = vault();
        let v = dir.path();
        let key = "s-20261112-f1vars01";
        chat(v, key, "Lead");
        let mut w = bound("f1-vars", key);
        w.last_run_at = None;
        let mut ctx = RunContext::manual_for_test();
        ctx.source = RunSource::Trigger;
        ctx.reason = "meeting in 30m: <Standup>".into();
        let at = chrono::Local::now();
        let raw = "On {{date}} ({{ trigger_reason }}) since {{last_run_at}}; {{unknown}}.";
        let text = expand_prompt(v, &w, raw, &ctx, at);
        let (same, external) = compose_prompt(raw, &template_vars(v, &w, v, &ctx, at));
        assert!(external);
        assert!(text.starts_with(&same), "{text}");
        assert!(text.contains(&at.format("%Y-%m-%d").to_string()));
        assert!(text.contains(
            "<hollow-var name=\"trigger_reason\">meeting in 30m: &lt;Standup&gt;</hollow-var>"
        ));
        assert!(
            text.contains("since never; {{unknown}}."),
            "unknown stays literal"
        );
        assert!(text.ends_with(crate::workflow_hygiene::HOLLOW_VAR_NOTE));
        // No external value: no note, and a manual run has no reason.
        let plain = expand_prompt(
            v,
            &w,
            "{{trigger_reason}}Check {{date}}.",
            &RunContext::manual_for_test(),
            at,
        );
        assert_eq!(plain, format!("Check {}.", at.format("%Y-%m-%d")));
        // It rides into the queued turn.
        let sent = deliver(v, &w, &text, "run-f1-vars", Utc::now()).unwrap();
        let Delivered::Queued { queue_id, .. } = sent else {
            panic!("queued");
        };
        let row = run_queue::get(v, queue_id).unwrap().unwrap();
        let prompt = row.payload["prompt"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert!(
            prompt.contains("<hollow-var name=\"trigger_reason\">"),
            "{prompt}"
        );
        assert!(!prompt.contains("{{date}}"));
    }

    /// An exhausted hard-stop budget of the automation fails a bound run
    /// before it sends, and leaves a message already waiting where it is.
    #[test]
    fn an_exhausted_hard_stop_budget_fails_a_run_and_keeps_a_waiting_message() {
        let dir = vault();
        let v = dir.path();
        let key = "s-20261112-f1hold001";
        chat(v, key, "Lead");
        let now = Utc::now();
        let w = saved(v, "Hold check", key);
        let Delivered::Queued { queue_id, .. } =
            deliver(v, &w, "First.", "run-f1-hold-1", now).unwrap()
        else {
            panic!("queued");
        };
        // The first turn is still queued, so the second waits.
        let waiting = deliver(v, &w, "Second.", "run-f1-hold-2", now).unwrap();
        assert!(matches!(waiting, Delivered::Waiting { .. }), "{waiting:?}");
        // The first turn then runs and spends 1000 tokens.
        run_queue::claim(v, queue_id, now).unwrap().unwrap();
        assert!(run_queue::mark_running(v, queue_id, "R-f1-hold-turn", now).unwrap());
        let at = crate::session_store::db_time(now);
        let mut row = crate::session_store::tests::usage_row_fixture("k-f1-hold", &at, "success");
        row.run_id = Some("R-f1-hold-turn".into());
        row.scope = Some(key.into());
        row.input_tokens = Some(1000);
        crate::session_store::upsert_usage(v, &row).unwrap();
        // A hard-stop budget of 100 tokens a day for this automation.
        let store = crate::budgets::Store::for_app(v);
        crate::budgets::save(
            &store,
            v,
            crate::budgets::BudgetInput {
                id: None,
                scope: format!("workflow:{}", w.slug),
                metric: crate::budgets::Metric::Tokens,
                window: "day".into(),
                limit: 100.0,
                warn_percent: None,
                hard_stop: Some(true),
            },
            &|_| Some(crate::agent::AgentKind::Claude),
            now,
        )
        .unwrap();
        let t = target(v, key).unwrap();
        assert!(budget_hold(v, &w.slug, &t, now).is_some(), "held");
        let before = run_queue::live_rows(v).unwrap().len();
        let error = deliver(v, &w, "Third.", "run-f1-hold-3", now).unwrap_err();
        assert!(!error.is_empty());
        assert_eq!(
            run_queue::live_rows(v).unwrap().len(),
            before,
            "sent nothing"
        );
        // The waiting message is still the second, untouched, even after
        // the running turn ends and the minute pass looks at it.
        run_queue::finish(v, queue_id, RowStatus::Done, None, now).unwrap();
        assert!(deliver_pending(v, now).is_empty(), "held by the budget");
        let kept = load(v);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].run_id, "run-f1-hold-2");
        assert!(kept[0].text.contains("Second."));
        // Review of #1331 (4): it keeps the budget's notice, and drops with
        // it after the cap instead of "behind a running turn".
        let notice = budget_hold(v, &w.slug, &t, now).unwrap();
        assert_eq!(kept[0].held.as_deref(), Some(notice.as_str()));
        let later = now + Duration::hours(PENDING_HOURS) + Duration::minutes(1);
        assert!(deliver_pending(v, later).is_empty());
        assert!(load(v).is_empty(), "dropped past the cap");
        let (_, _, log) = crate::vault_workflows::read_full(v, &w.slug).unwrap();
        let last = log.last().unwrap();
        assert_eq!(last.status, "error");
        assert_eq!(last.note, format!("not run in its chat: {notice}"));
        assert_eq!(last.run_id.as_deref(), Some("run-f1-hold-2"));
    }

    /// Two runs of `w` in its chat `key`: the first queued and then finished,
    /// the second waiting. Returns the first's row id.
    fn one_waiting(v: &Path, w: &Workflow, key: &str, tag: &str, now: DateTime<Utc>) -> i64 {
        let Delivered::Queued { queue_id, .. } =
            deliver(v, w, "First.", &format!("run-{tag}-1"), now).unwrap()
        else {
            panic!("queued");
        };
        let second = deliver(v, w, "Second.", &format!("run-{tag}-2"), now).unwrap();
        assert!(matches!(second, Delivered::Waiting { .. }), "{second:?}");
        run_queue::finish(v, queue_id, RowStatus::Done, None, now).unwrap();
        assert_eq!(chat_rows(v, key), 1);
        queue_id
    }

    fn chat_rows(v: &Path, key: &str) -> usize {
        run_queue::chat_rows_since(v, key, "").unwrap().len()
    }

    fn error_rows(v: &Path, slug: &str) -> Vec<RunLogEntry> {
        let (_, _, log) = crate::vault_workflows::read_full(v, slug).unwrap();
        log.into_iter().filter(|e| e.status == "error").collect()
    }

    /// Review of #1331 (1): a database read that fails while the minute pass
    /// resolves the chat keeps the message for the next pass; it was dropped
    /// for good with a false "not run in its chat" row.
    #[test]
    fn a_waiting_message_survives_an_unreadable_session_store() {
        let dir = vault();
        let v = dir.path();
        let key = "s-20261112-f1dbrd01";
        chat(v, key, "Lead");
        let now = Utc::now();
        let w = saved(v, "Store check", key);
        one_waiting(v, &w, key, "f1-dbrd", now);
        // The conversations table can't be read (a real read error from
        // `session_by_key`, as a damaged or locked store gives).
        let db = rusqlite::Connection::open(crate::session_store::db_path_for(v).unwrap()).unwrap();
        db.execute_batch("ALTER TABLE sessions RENAME TO sessions_away")
            .unwrap();
        assert!(matches!(resolve(v, key), Err(Miss::Unreadable(_))));
        assert!(deliver_pending(v, now).is_empty());
        assert_eq!(load(v).len(), 1, "kept while the store can't be read");
        assert!(error_rows(v, &w.slug).is_empty(), "no failure row");
        db.execute_batch("ALTER TABLE sessions_away RENAME TO sessions")
            .unwrap();
        assert_eq!(deliver_pending(v, now).len(), 1, "sent once it reads");
        assert!(load(v).is_empty());
        assert_eq!(chat_rows(v, key), 2);
    }

    /// Lane Q's re-review of #1343: a new run that finds the store
    /// unreadable parks its message instead of failing for good (a Prepare
    /// failure is never retried), and the minute pass sends it once.
    #[test]
    fn a_new_run_on_an_unreadable_session_store_waits_and_is_sent_once() {
        let dir = vault();
        let v = dir.path();
        let key = "s-20261113-f1dbnew1";
        chat(v, key, "Lead");
        let now = Utc::now();
        let w = saved(v, "Store check new", key);
        let db = rusqlite::Connection::open(crate::session_store::db_path_for(v).unwrap()).unwrap();
        db.execute_batch("ALTER TABLE sessions RENAME TO sessions_away")
            .unwrap();
        let parked = deliver(v, &w, "Only.", "run-f1-dbnew-1", now).unwrap();
        assert!(matches!(parked, Delivered::Unread { .. }), "{parked:?}");
        assert!(parked.note().starts_with("Waiting: "), "{}", parked.note());
        let waiting = load(v);
        assert_eq!(waiting.len(), 1, "parked, not lost");
        assert_eq!(waiting[0].run_id, "run-f1-dbnew-1");
        assert!(deliver_pending(v, now).is_empty(), "still unreadable");
        assert_eq!(load(v).len(), 1);
        assert!(error_rows(v, &w.slug).is_empty(), "no failure row");
        db.execute_batch("ALTER TABLE sessions_away RENAME TO sessions")
            .unwrap();
        assert_eq!(deliver_pending(v, now).len(), 1, "sent once it reads");
        assert!(load(v).is_empty());
        assert_eq!(chat_rows(v, key), 1);
        assert!(deliver_pending(v, now).is_empty(), "never twice");
        assert_eq!(chat_rows(v, key), 1);
    }

    /// Review of #1343 (1): a queue that can't be read keeps a message only
    /// until [`CHECK_CEILING_HOURS`], then drops it with a row saying why.
    #[test]
    fn a_message_whose_send_cant_be_checked_drops_at_the_ceiling() {
        let dir = vault();
        let v = dir.path();
        let key = "s-20261112-f1ceil01";
        chat(v, key, "Lead");
        let now = Utc::now();
        let w = saved(v, "Ceiling check", key);
        one_waiting(v, &w, key, "f1-ceil", now);
        let db = rusqlite::Connection::open(crate::session_store::db_path_for(v).unwrap()).unwrap();
        db.execute_batch("ALTER TABLE run_queue RENAME TO run_queue_away")
            .unwrap();
        let past_cap = now + Duration::hours(PENDING_HOURS) + Duration::minutes(1);
        assert!(deliver_pending(v, past_cap).is_empty());
        assert_eq!(load(v).len(), 1, "kept past the cap while unchecked");
        assert!(error_rows(v, &w.slug).is_empty());
        let past_ceiling = now + Duration::hours(CHECK_CEILING_HOURS) + Duration::minutes(1);
        assert!(deliver_pending(v, past_ceiling).is_empty());
        assert!(load(v).is_empty(), "dropped at the ceiling");
        let errors = error_rows(v, &w.slug);
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0]
                .note
                .contains("could not check whether it was already sent"),
            "{}",
            errors[0].note
        );
        db.execute_batch("ALTER TABLE run_queue_away RENAME TO run_queue")
            .unwrap();
    }

    /// Review of #1331 (2): the send landed in chat A, Hollow stopped before
    /// the entry was removed, and A then handed over to B. The restart's pass
    /// finds the send in A and never sends it again to B.
    #[test]
    fn a_waiting_message_sent_before_a_handover_is_never_sent_again() {
        let dir = vault();
        let v = dir.path();
        let a = "s-20261112-f1hand01";
        let b = "s-20261112-f1hand02";
        chat(v, a, "Lead");
        let now = Utc::now();
        let w = saved(v, "Handover check", a);
        one_waiting(v, &w, a, "f1-hand", now);
        let t = target(v, a).unwrap();
        let landed = send(
            v,
            &t,
            "acceptEdits",
            "Second.".into(),
            turn_id("run-f1-hand-2"),
            now,
        )
        .unwrap();
        run_queue::finish(v, landed, RowStatus::Done, None, now).unwrap();
        chat(v, b, "Lead, continued");
        crate::app_sessions::record_handover(v, a, b, 1).unwrap();
        assert_eq!(target(v, a).unwrap().key, b);
        assert_eq!(load(v).len(), 1, "the entry outlived the send");
        assert_eq!(deliver_pending(v, now), vec![landed], "recognised in A");
        assert!(load(v).is_empty());
        assert_eq!(chat_rows(v, b), 0, "nothing sent to B");
    }

    /// Review of #1331 (3): a send that landed before a long outage is
    /// recognised as sent, not dropped with a false "waited too long" row.
    #[test]
    fn a_message_sent_before_a_long_outage_is_not_dropped_as_late() {
        let dir = vault();
        let v = dir.path();
        let key = "s-20261112-f1outg01";
        chat(v, key, "Lead");
        let now = Utc::now();
        let w = saved(v, "Outage check", key);
        one_waiting(v, &w, key, "f1-outg", now);
        let t = target(v, key).unwrap();
        let landed = send(
            v,
            &t,
            "acceptEdits",
            "Second.".into(),
            turn_id("run-f1-outg-2"),
            now,
        )
        .unwrap();
        let later = now + Duration::hours(PENDING_HOURS) + Duration::hours(2);
        assert_eq!(deliver_pending(v, later), vec![landed]);
        assert!(load(v).is_empty());
        assert!(error_rows(v, &w.slug).is_empty(), "no late row");
        assert_eq!(chat_rows(v, key), 2, "the first turn and the one send");
    }
}
