//! One conversation from the person's side, whatever happens underneath
//! (2026-11-07, Alex: "Why are there two Main agent chats?").
//!
//! A chat that hands over (`app_sessions::handover`) continues in its
//! successor, and the Main agent's thread moves to a new conversation at a
//! handover or at its daily soft reset. Before this, the old conversation
//! still took messages: a tab left open ran "Install" a second time beside
//! its successor. Now:
//!
//! - **The current conversation** ([`current_of`]): for any chat, the last
//!   of its handovers; for a Main agent conversation, the Main agent's
//!   newest unarchived one ([`main_current`]), which every wake, check-in
//!   and the Main agent button use too. A conversation that isn't current
//!   reads "This conversation continued in …" in place of its composer.
//! - **A send that still reaches an old conversation** (a race at handover,
//!   the phone, `send_to_session`, a held send) goes to the current one
//!   instead ([`route`], called by `chat_queue::accept_turn_with_effort`,
//!   which every sender passes through, and by `commands::chat_send` first,
//!   so the person's send is held behind a running turn rather than
//!   refused). The agent sees a `<hollow-forwarded>` block naming where it was
//!   typed; both conversations show the note ([`forwards_for`]).
//! - **A project lead's chain** (L8, 2026-11-10): the same for every
//!   project's lead. Its fixed conversation (`project_agents::key`) and the
//!   successors its handovers and long-context rollovers queued are one chat
//!   ([`lead_root`]); its current one ([`lead_current`]) is what the
//!   project's sidebar row opens and where every send lands.
//! - **The chain** ([`chain_view`], `conversation_chain`): what the
//!   webview needs for the banner, the Main page's "Earlier in this
//!   conversation" and the cards that follow the chain.
//!
//! Forward notes: `<support>/Forwards/<vault>.json`, the newest [`KEPT`].

use crate::session_store;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// How many handover links a walk follows at most (the chain's cap, with
/// slack for records from before it).
const MAX_STEPS: usize = crate::app_sessions::MAX_HANDOVER_CHAIN as usize + 8;
/// Forward notes kept (newest).
const KEPT: usize = 300;
/// A forward note keeps this much of the message.
const PREVIEW_CHARS: usize = 160;
/// Earlier conversations a chain view lists at most (the Main agent's
/// accumulate a few a day).
pub(crate) const EARLIER_MAX: usize = 200;
/// The block a forwarded message carries for the agent
/// (`sessions::leading_blocks` strips it from the person's own text).
pub(crate) const FORWARD_TAG: &str = "hollow-forwarded";

static REGISTRY: Mutex<()> = Mutex::new(());

/// The Main agent's current conversation: its newest unarchived one, as
/// `(key, created_at)`. None before it has one.
pub(crate) fn main_current(vault: &Path) -> Option<(String, String)> {
    let cwd = crate::main_agent::main_cwd(vault)?;
    let listed = session_store::newest_session_in(vault, &cwd)
        .ok()
        .flatten()
        .map(|(key, created, _)| (key, created));
    // A new thread whose first turn still waits in the queue has no
    // conversation row yet (review of #1220): its live row counts, newest
    // first, so a second new thread never starts beside it.
    let queued = queued_new_in(vault, &cwd);
    // Compared as times: a queued row's and a conversation's stamps are
    // written in different RFC 3339 shapes.
    let at = |iso: &str| session_store::parse_iso(iso);
    let newest = match (listed, queued) {
        (Some(l), Some(q)) => {
            if at(&q.1) > at(&l.1) {
                q
            } else {
                l
            }
        }
        (l, q) => l.or(q)?,
    };
    let tail = handover_tail(vault, &newest.0);
    Some(if tail == newest.0 {
        newest
    } else {
        (tail, newest.1)
    })
}

/// The newest live chat row in folder `cwd` whose conversation has no row
/// yet, as `(key, requested_at)`.
fn queued_new_in(vault: &Path, cwd: &str) -> Option<(String, String)> {
    crate::run_queue::live_rows(vault)
        .unwrap_or_default()
        .into_iter()
        .filter(|row| row.source == crate::run_queue::RunSource::Chat)
        .filter(|row| row.payload.get("cwd").and_then(|v| v.as_str()) == Some(cwd))
        .filter_map(|row| {
            row.source_id
                .clone()
                .map(|key| (key, row.requested_at.clone()))
        })
        .filter(|(key, _)| session_store::session_cwd(vault, key).is_none())
        .max_by_key(|(_, at)| session_store::parse_iso(at))
}

/// Where conversation `key` runs: its row's folder, else (a first turn
/// still queued) its queued row's.
fn conversation_cwd(vault: &Path, key: &str) -> Option<String> {
    session_store::session_cwd(vault, key).or_else(|| {
        crate::run_queue::newest_chat_row(vault, key)
            .ok()
            .flatten()
            .and_then(|row| {
                row.payload
                    .get("cwd")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
    })
}

/// A conversation list's filter (the phone's `/v1/sessions`): the Main
/// agent's current conversation stands for it, its earlier ones are left
/// out (they open from it); every other conversation stays.
pub(crate) fn list_filter(vault: &Path) -> impl Fn(&crate::types::AgentSessionSummary) -> bool {
    let main = crate::main_agent::main_cwd(vault);
    let current = main_current(vault).map(|(key, _)| key);
    move |s| match &main {
        Some(cwd) if s.cwd == *cwd => current.as_deref() == Some(s.key.as_str()),
        _ => true,
    }
}

/// Whether conversation `key` runs in the Main agent's folder.
pub(crate) fn is_main_conversation(vault: &Path, key: &str) -> bool {
    match crate::main_agent::main_cwd(vault) {
        Some(cwd) => conversation_cwd(vault, key).as_deref() == Some(cwd.as_str()),
        None => false,
    }
}

/// The last conversation of `key`'s handovers (`key` when it never handed
/// over). Cycle-safe and bounded.
pub(crate) fn handover_tail(vault: &Path, key: &str) -> String {
    let mut at = key.to_string();
    let mut seen = vec![at.clone()];
    for _ in 0..MAX_STEPS {
        match crate::app_sessions::successor_of(vault, &at) {
            Some(next) if !seen.contains(&next) => {
                seen.push(next.clone());
                at = next;
            }
            _ => break,
        }
    }
    at
}

/// The conversation a message for `key` belongs in now, when that is
/// another one: the last of its handovers and, for a Main agent
/// conversation, the Main agent's current one. None: `key` is current, or
/// isn't a session key.
pub(crate) fn current_of(vault: &Path, key: &str) -> Option<String> {
    if !crate::agent_sessions::is_safe_session_key(key) {
        return None;
    }
    let tail = handover_tail(vault, key);
    let current = if is_main_conversation(vault, &tail) {
        main_current(vault).map_or(tail, |(newest, _)| newest)
    } else {
        tail
    };
    (current != key).then_some(current)
}

/// How a project lead's fixed conversation key starts
/// (`project_agents::key`).
pub(crate) const LEAD_PREFIX: &str = "s-project-";

/// The project lead conversation `key` belongs to (L8, 11-10): `key` itself
/// when it is a lead's fixed conversation, else the lead whose handovers
/// (long-context rollovers included) led to it. None for any other chat.
/// The lead keeps one row from the person's side: its successors are the
/// same chat, with its prompt, tool policy and depth.
pub(crate) fn lead_root(vault: &Path, key: &str) -> Option<String> {
    let key = key.trim();
    if key.starts_with(LEAD_PREFIX) {
        return Some(key.to_string());
    }
    if !crate::agent_sessions::is_safe_session_key(key) {
        return None;
    }
    predecessors(vault, key)
        .into_iter()
        .find(|k| k.starts_with(LEAD_PREFIX))
}

/// Whether conversation `key` is a project lead's (its fixed one or a
/// successor of it).
pub(crate) fn is_lead_conversation(vault: &Path, key: &str) -> bool {
    lead_root(vault, key).is_some()
}

/// A project lead's current conversation (L8): the last of its fixed
/// conversation's handovers, the one its sidebar row opens and every send,
/// wake and delegation reaches. None for a built-in workspace.
pub(crate) fn lead_current(vault: &Path, workspace_id: &str) -> Option<String> {
    crate::project_agents::lead_key(vault, workspace_id).map(|key| handover_tail(vault, &key))
}

/// [`handover_tail`] over a handover map read once (`from -> successor`),
/// for a list that resolves many leads at a time.
pub(crate) fn tail_in(next: &std::collections::HashMap<String, String>, key: &str) -> String {
    let mut at = key.to_string();
    let mut seen = vec![at.clone()];
    for _ in 0..MAX_STEPS {
        match next.get(&at) {
            Some(succ) if !seen.contains(succ) => {
                seen.push(succ.clone());
                at = succ.clone();
            }
            _ => break,
        }
    }
    at
}

/// Every recorded handover as `from -> successor` (the newest wins).
pub(crate) fn handover_map(vault: &Path) -> std::collections::HashMap<String, String> {
    crate::app_sessions::handover_pairs(vault)
        .into_iter()
        .map(|(succ, from)| (from, succ))
        .collect()
}

/// What a send `route` moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Routed {
    /// The conversation it was sent to; `None` for a new Main agent
    /// conversation started while the current one works ([`route_send`]).
    pub from: Option<String>,
    /// The conversation it goes to.
    pub to: String,
    pub to_title: Option<String>,
    /// The current conversation's native session and folder and agent.
    pub session_id: Option<String>,
    pub cwd: String,
    pub agent: String,
}

fn routed_to(vault: &Path, from: Option<String>, to: String) -> Option<Routed> {
    if let Some(summary) = session_store::session_by_key(vault, &to).ok().flatten() {
        return Some(Routed {
            from,
            to,
            to_title: summary.title.clone().filter(|t| !t.trim().is_empty()),
            session_id: summary.harness_session_id.clone(),
            cwd: summary.cwd.clone(),
            agent: summary.agent.clone(),
        });
    }
    // A successor whose first turn is still queued has no conversation row
    // yet (review of #1220): its queued row says where it runs. Without
    // either, the send keeps its own folder and agent (a handover's
    // successor runs in the same workspace).
    let row = crate::run_queue::newest_chat_row(vault, &to).ok().flatten();
    let field = |name: &str| {
        row.as_ref()
            .and_then(|r| r.payload.get(name))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    Some(Routed {
        from,
        to: to.clone(),
        to_title: None,
        session_id: None,
        cwd: field("cwd"),
        agent: field("agent"),
    })
}

/// Where a send for `key` goes: `None` when `key` is current (or new), else
/// the current conversation with what a send there resumes.
pub(crate) fn route(vault: &Path, key: Option<&str>) -> Option<Routed> {
    let from = key?.trim();
    let to = current_of(vault, from)?;
    routed_to(vault, Some(from.to_string()), to)
}

/// Whether conversation `key` has a turn queued or running.
pub(crate) fn is_live(vault: &Path, key: &str) -> bool {
    crate::run_queue::newest_chat_row(vault, key)
        .ok()
        .flatten()
        .is_some_and(|row| row.status.is_live())
}

/// [`route`], and for a send that starts a new conversation in the Main
/// agent's folder at depth 0 (the person's, the phone's, a wake's) while
/// the current Main agent conversation has a turn queued or running, that
/// conversation: a second Main thread would run beside it (seen 11-04 at
/// 12:04 AM). A handover's successor (depth 1 or more) starts its own.
pub(crate) fn route_send(
    vault: &Path,
    key: Option<&str>,
    cwd: &Path,
    app_depth: u32,
    continues: bool,
) -> Option<Routed> {
    if let Some(key) = key.filter(|k| crate::agent_sessions::is_safe_session_key(k)) {
        return route(vault, Some(key));
    }
    // Continue from summary starts a new conversation on purpose.
    if app_depth != 0 || continues {
        return None;
    }
    let main = crate::main_agent::main_cwd(vault)?;
    if Path::new(&main) != cwd {
        return None;
    }
    let (current, _) = main_current(vault)?;
    is_live(vault, &current)
        .then(|| routed_to(vault, None, current))
        .flatten()
}

/// The block a forwarded message leads with: where the person typed it,
/// for the agent (stripped from the person's own text at admission).
/// What a send [`route`] moved runs with in the current conversation: its
/// own agent and model (`wake_me::wake_model`: the model its newest turn
/// asked for, `""` for the agent's own default), and its own mode
/// (`app_mcp::own_mode`, bypass and auto kept), which the send's mode may
/// only lower (review of #1220: an old tab in bypass must not run a thread
/// in plan in bypass). A send that names no mode keeps the current
/// conversation's own (2026-11-12: before, it fell to `acceptEdits`); a
/// conversation with no recorded mode yet takes the send's, else Hollow's
/// default.
pub(crate) fn routed_settings(
    vault: &Path,
    routed: &Routed,
    send_mode: Option<&str>,
) -> (Option<String>, Option<String>, String) {
    let agent = (!routed.agent.is_empty()).then(|| routed.agent.clone());
    let model = agent
        .as_deref()
        .and_then(|a| crate::wake_me::wake_model(vault, &routed.to, a));
    let mode = match session_store::last_permission(vault, &routed.to)
        .ok()
        .flatten()
    {
        Some(last) => {
            crate::app_mcp::lowered(crate::app_mcp::own_mode(Some(last.as_str())), send_mode)
        }
        None => crate::app_mcp::own_mode(send_mode),
    };
    (agent, model, mode.to_string())
}

/// Who sent a message [`route`] moved: the wording of its note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sender {
    /// The person, from a composer or the phone.
    Person,
    /// Another agent (`send_to_session`).
    Agent,
    /// Hollow itself (a wake, a check-in, a continuation).
    Hollow,
}

pub(crate) fn forward_block(routed: &Routed, sender: Sender) -> String {
    let who = match sender {
        Sender::Person => "The person sent this",
        Sender::Agent => "Another agent sent this",
        Sender::Hollow => "Hollow queued this",
    };
    let what = match &routed.from {
        Some(from) => format!("{who} for conversation {from} after it had continued here"),
        None => format!("{who} as a new COO conversation while this one was working"),
    };
    let treat = match sender {
        Sender::Person => " Treat it as their next message in this conversation.",
        _ => "",
    };
    format!("<{FORWARD_TAG}>{what}; Hollow moved it to you.{treat}</{FORWARD_TAG}>\n\n")
}

/// `prompt` with [`forward_block`] in front (once).
pub(crate) fn with_forward_block(prompt: &str, routed: &Routed, sender: Sender) -> String {
    if prompt.trim_start().starts_with(&format!("<{FORWARD_TAG}>")) {
        return prompt.to_string();
    }
    format!("{}{prompt}", forward_block(routed, sender))
}

/// One message that went from one conversation to the chain's current one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ForwardNote {
    pub from: String,
    pub to: String,
    /// RFC 3339.
    pub at: String,
    /// The start of the person's own text.
    pub preview: String,
}

fn forwards_path(vault: &Path) -> Option<PathBuf> {
    let stem = session_store::vault_file_stem(vault);
    #[cfg(test)]
    {
        Some(crate::test_temp::process_dir("hollow-test-forwards-").join(format!("{stem}.json")))
    }
    #[cfg(not(test))]
    {
        crate::instance::support_dir().map(|r| r.join("Forwards").join(format!("{stem}.json")))
    }
}

fn load_forwards(vault: &Path) -> Vec<ForwardNote> {
    forwards_path(vault)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Remember that a message went from `routed.from` to `routed.to`. A lost
/// note is logged, never a failed send.
pub(crate) fn record_forward(vault: &Path, routed: &Routed, asked: &str, at: &str) {
    let _one = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    let (Some(path), Some(from)) = (forwards_path(vault), routed.from.clone()) else {
        return;
    };
    let mut all = load_forwards(vault);
    let text = crate::sessions::strip_injected_context(asked);
    let mut preview: String = text.trim().chars().take(PREVIEW_CHARS).collect();
    if text.trim().chars().count() > PREVIEW_CHARS {
        preview.push('…');
    }
    all.push(ForwardNote {
        from,
        to: routed.to.clone(),
        at: at.to_string(),
        preview,
    });
    let excess = all.len().saturating_sub(KEPT);
    all.drain(..excess);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_vec(&all) {
        Ok(bytes) => {
            if let Err(e) = crate::vault::atomic_write_private(&path, &bytes) {
                eprintln!("[chat chain] forward note not saved: {e}");
            }
        }
        Err(e) => eprintln!("[chat chain] forward note not saved: {e}"),
    }
}

/// Forward notes that touch `key` (sent from it, or arrived in it), oldest
/// first.
pub(crate) fn forwards_for(vault: &Path, key: &str) -> Vec<ForwardNote> {
    load_forwards(vault)
        .into_iter()
        .filter(|f| f.from == key || f.to == key)
        .collect()
}

/// One earlier conversation of a chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ChainLink {
    pub key: String,
    pub title: Option<String>,
    pub created_at: String,
}

/// A conversation's place in its chain (`conversation_chain`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ChainView {
    pub key: String,
    /// The chain's current conversation (`key` itself when it is current).
    pub current: String,
    pub current_title: Option<String>,
    /// A Main agent conversation (its chain is every Main agent
    /// conversation, newest current).
    pub main: bool,
    /// The conversations before the current one, newest first: for the
    /// Main agent its other conversations, else the handovers that led here.
    pub earlier: Vec<ChainLink>,
    /// Messages that went from this conversation to the current one, or
    /// arrived here from an earlier one.
    pub forwards: Vec<ForwardNote>,
}

fn link(vault: &Path, key: &str) -> Option<ChainLink> {
    let s = session_store::session_by_key(vault, key).ok().flatten()?;
    Some(ChainLink {
        key: s.key,
        title: s.title.filter(|t| !t.trim().is_empty()),
        created_at: s.created_at,
    })
}

/// The handovers that led to `key`, newest first (bounded, cycle-safe).
pub(crate) fn predecessors(vault: &Path, key: &str) -> Vec<String> {
    predecessors_in(&crate::app_sessions::handover_pairs(vault), key)
}

/// [`predecessors`] over handover pairs read once (`(to, from)`), for a
/// caller that walks many conversations (the spend view).
pub(crate) fn predecessors_in(pairs: &[(String, String)], key: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut at = key.to_string();
    for _ in 0..MAX_STEPS {
        let Some((_, from)) = pairs.iter().rev().find(|(to, _)| *to == at) else {
            break;
        };
        if from == key || out.contains(from) {
            break;
        }
        out.push(from.clone());
        at = from.clone();
    }
    out
}

/// The conversations before `key` in its chain when `key` is current (the
/// Main agent's other conversations, else the handovers that led to it);
/// empty when `key` isn't current. Their open cards show in `key`.
pub(crate) fn chain_keys(vault: &Path, key: &str) -> Vec<String> {
    if !crate::agent_sessions::is_safe_session_key(key) || current_of(vault, key).is_some() {
        return Vec::new();
    }
    if is_main_conversation(vault, key) {
        let cwd = crate::main_agent::main_cwd(vault).unwrap_or_default();
        return session_store::chain_sessions_in(vault, &cwd, EARLIER_MAX + 1)
            .unwrap_or_default()
            .into_iter()
            .map(|l| l.key)
            .filter(|k| k != key)
            .collect();
    }
    predecessors(vault, key)
}

/// `key`'s chain as the webview shows it.
pub(crate) fn chain_view(vault: &Path, key: &str) -> Result<ChainView, String> {
    if !crate::agent_sessions::is_safe_session_key(key) {
        return Err(format!("`{key}` is not a session key."));
    }
    let current = current_of(vault, key).unwrap_or_else(|| key.to_string());
    let main = is_main_conversation(vault, &current);
    let earlier: Vec<ChainLink> = if main {
        let cwd = crate::main_agent::main_cwd(vault).unwrap_or_default();
        session_store::chain_sessions_in(vault, &cwd, EARLIER_MAX + 1)?
            .into_iter()
            .filter(|l| l.key != current)
            .take(EARLIER_MAX)
            .collect()
    } else {
        predecessors(vault, &current)
            .iter()
            .filter_map(|k| link(vault, k))
            .collect()
    };
    let current_title = session_store::session_by_key(vault, &current)
        .ok()
        .flatten()
        .and_then(|s| s.title)
        .filter(|t| !t.trim().is_empty());
    Ok(ChainView {
        key: key.to_string(),
        forwards: forwards_for(vault, key),
        current,
        current_title,
        main,
        earlier,
    })
}

/// `conversation_chain`: a conversation's chain, for the banner, the Main
/// page's earlier conversations and the cards that follow the chain.
#[tauri::command]
pub async fn conversation_chain(
    state: tauri::State<'_, crate::commands::AppState>,
    key: String,
) -> Result<ChainView, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || chain_view(&vault, key.trim()))
        .await
        .map_err(|e| e.to_string())?
}

// ── Arrivals (quick chats, 2026-11-14) ──────────────────────────────────
// A chat moved into a project (Move to project…, a quick chat that grew)
// leaves its lead a one-line note, given once at the start of the lead's
// next turn. Stored per lead (its fixed key) in
// `<support>/Arrivals/<vault>.json`; a lost note is logged, never a failed
// move.

/// The block a lead's next turn leads with when chats moved into its project
/// (`sessions::leading_blocks` strips it from the person's own text).
pub(crate) const ARRIVED_TAG: &str = "hollow-arrived";
/// Arrival notes kept per lead (newest).
const ARRIVALS_KEPT: usize = 10;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Arrivals {
    #[serde(default)]
    leads: std::collections::BTreeMap<String, Vec<Arrival>>,
}

/// One arrival note: its line, and the turn that was given it
/// (`<key>#<ord>`), so a retried attempt of that turn carries it again and
/// a later turn drops it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Arrival {
    line: String,
    #[serde(default)]
    given: Option<String>,
}

fn arrivals_path(vault: &Path) -> Option<PathBuf> {
    let stem = session_store::vault_file_stem(vault);
    #[cfg(test)]
    {
        Some(crate::test_temp::process_dir("hollow-test-arrivals-").join(format!("{stem}.json")))
    }
    #[cfg(not(test))]
    {
        crate::instance::support_dir().map(|r| r.join("Arrivals").join(format!("{stem}.json")))
    }
}

fn load_arrivals(path: &Path) -> Arrivals {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_arrivals(path: &Path, all: &Arrivals) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_vec(all) {
        Ok(bytes) => {
            if let Err(e) = crate::vault::atomic_write_private(path, &bytes) {
                eprintln!("[chat chain] arrival note not saved: {e}");
            }
        }
        Err(e) => eprintln!("[chat chain] arrival note not saved: {e}"),
    }
}

/// Conversation `key` (titled `title`) moved into workspace `to` from
/// `from` (a workspace title, or None): note it for `to`'s project lead.
/// Nothing for a built-in workspace or the lead's own conversation.
pub(crate) fn record_arrival(
    vault: &Path,
    key: &str,
    title: Option<&str>,
    to: &str,
    from: Option<&str>,
) {
    // A project Move Hollow home re-keyed keeps its lead's old key.
    let Some(lead) = crate::project_agents::lead_key(vault, to) else {
        return;
    };
    if lead_root(vault, key).is_some() {
        return;
    }
    let Some(path) = arrivals_path(vault) else {
        return;
    };
    let name: String = title
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or("Conversation")
        .chars()
        .take(PREVIEW_CHARS)
        .collect();
    let line = format!(
        "The chat \"{name}\" ({key}) moved into this project from {}, with its history. It is listed under your Subagents; read_session reads it.",
        from.unwrap_or("another folder")
    );
    let _one = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = load_arrivals(&path);
    let lines = all.leads.entry(lead).or_default();
    lines.retain(|l| !l.line.contains(&format!("({key})")));
    lines.push(Arrival { line, given: None });
    let excess = lines.len().saturating_sub(ARRIVALS_KEPT);
    lines.drain(..excess);
    save_arrivals(&path, &all);
}

/// The arrival block for conversation `key`'s turn `turn_ord`: its lead's
/// notes not yet given, or given to this same turn (a retried attempt
/// carries them again); notes an earlier turn was given are dropped. Empty
/// for any other chat or with none waiting.
pub(crate) fn take_arrivals(vault: &Path, key: &str, turn_ord: i64) -> String {
    let Some(path) = arrivals_path(vault) else {
        return String::new();
    };
    if !path.is_file() {
        return String::new();
    }
    let Some(lead) = lead_root(vault, key) else {
        return String::new();
    };
    let turn = format!("{key}#{turn_ord}");
    let _one = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = load_arrivals(&path);
    let Some(lines) = all.leads.get_mut(&lead) else {
        return String::new();
    };
    lines.retain(|a| a.given.is_none() || a.given.as_deref() == Some(turn.as_str()));
    for a in lines.iter_mut() {
        a.given = Some(turn.clone());
    }
    let text: Vec<String> = lines.iter().map(|a| a.line.clone()).collect();
    if text.is_empty() {
        all.leads.remove(&lead);
    }
    save_arrivals(&path, &all);
    if text.is_empty() {
        return String::new();
    }
    format!("<{ARRIVED_TAG}>\n{}\n</{ARRIVED_TAG}>\n\n", text.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Quick chats (2026-11-14): a chat moved into a project leaves its
    /// lead one note, given once and only to the lead's turn.
    #[test]
    fn a_moved_chat_leaves_its_lead_one_note_given_once() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let lead = crate::project_agents::key("proj-qc").unwrap();
        record_arrival(
            v,
            "s-20261114-qc-a",
            Some("Pricing idea"),
            "proj-qc",
            Some("Quick chats"),
        );
        // A built-in target and the lead's own conversation leave nothing.
        record_arrival(
            v,
            "s-20261114-qc-b",
            None,
            crate::workspaces::QUICK_CHATS_ID,
            None,
        );
        record_arrival(v, &lead, None, "proj-qc", None);
        assert_eq!(take_arrivals(v, "s-20261114-qc-other", 1), "", "not a lead");
        let block = take_arrivals(v, &lead, 4);
        assert!(block.starts_with("<hollow-arrived>\n"), "{block}");
        assert!(
            block.contains(
                "\"Pricing idea\" (s-20261114-qc-a) moved into this project from Quick chats"
            ),
            "{block}"
        );
        assert_eq!(block.matches("moved into").count(), 1, "{block}");
        assert_eq!(
            take_arrivals(v, &lead, 4),
            block,
            "a retried attempt carries it again"
        );
        assert_eq!(take_arrivals(v, &lead, 5), "", "a later turn doesn't");
        // The lead's text is stripped from the person's own words.
        assert_eq!(
            crate::sessions::strip_injected_context(&format!("{block}Go on")),
            "Go on"
        );
    }

    #[test]
    fn a_handover_chain_ends_at_its_last_successor_and_a_cycle_stops() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        crate::app_sessions::record_handover(v, "s-20261107-cc-a", "s-20261107-cc-b", 1).unwrap();
        crate::app_sessions::record_handover(v, "s-20261107-cc-b", "s-20261107-cc-c", 2).unwrap();
        assert_eq!(handover_tail(v, "s-20261107-cc-a"), "s-20261107-cc-c");
        assert_eq!(handover_tail(v, "s-20261107-cc-c"), "s-20261107-cc-c");
        assert_eq!(
            current_of(v, "s-20261107-cc-a").as_deref(),
            Some("s-20261107-cc-c")
        );
        assert_eq!(current_of(v, "s-20261107-cc-c"), None, "current already");
        assert_eq!(current_of(v, "../etc"), None, "not a key");
        assert_eq!(
            predecessors(v, "s-20261107-cc-c"),
            vec!["s-20261107-cc-b".to_string(), "s-20261107-cc-a".to_string()]
        );
        // A cycle (a damaged registry) can't loop.
        crate::app_sessions::record_handover(v, "s-20261107-cy-x", "s-20261107-cy-y", 1).unwrap();
        crate::app_sessions::record_handover(v, "s-20261107-cy-y", "s-20261107-cy-x", 1).unwrap();
        let tail = handover_tail(v, "s-20261107-cy-x");
        assert!(tail == "s-20261107-cy-x" || tail == "s-20261107-cy-y");
    }

    /// 2026-11-12: a routed send keeps the current conversation's own mode
    /// (bypass and auto kept; a send naming none no longer falls to
    /// acceptEdits), and the send's own mode only lowers it. With no mode
    /// recorded there yet, the send's mode, else Hollow's default.
    #[test]
    fn a_routed_send_keeps_the_current_conversations_own_mode() {
        use crate::app_mcp::{perm_rank, PERMISSION_MODES};
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let to = "s-20261112-route-to";
        let routed = Routed {
            from: Some("s-20261112-route-from".into()),
            to: to.into(),
            to_title: None,
            session_id: None,
            cwd: "/tmp/x".into(),
            agent: String::new(),
        };
        assert_eq!(routed_settings(v, &routed, None).2, "acceptEdits");
        assert_eq!(
            routed_settings(v, &routed, Some("bypassPermissions")).2,
            "bypassPermissions"
        );
        let ord = session_store::start_turn(v, to, "claude", "/tmp/x", "R-route", "go").unwrap();
        for own in PERMISSION_MODES {
            session_store::record_turn_permission(v, to, ord, own).unwrap();
            assert_eq!(routed_settings(v, &routed, None).2, own, "none named");
            for sent in PERMISSION_MODES {
                let want = if perm_rank(sent) < perm_rank(own) {
                    sent
                } else {
                    own
                };
                assert_eq!(
                    routed_settings(v, &routed, Some(sent)).2,
                    want,
                    "{own} ← {sent}"
                );
            }
        }
    }

    #[test]
    fn a_forward_block_leads_once_and_is_not_the_persons_text() {
        let routed = Routed {
            from: Some("s-20261107-old".into()),
            to: "s-20261107-new".into(),
            to_title: None,
            session_id: None,
            cwd: "/tmp/x".into(),
            agent: "claude".into(),
        };
        let once = with_forward_block("Install", &routed, Sender::Person);
        assert!(once.starts_with("<hollow-forwarded>The person sent this"));
        assert_eq!(
            with_forward_block(&once, &routed, Sender::Person),
            once,
            "never twice"
        );
        let wake = with_forward_block("check-in", &routed, Sender::Hollow);
        assert!(wake.starts_with("<hollow-forwarded>Hollow queued this"));
        assert!(
            !wake.contains("their next message"),
            "never the person's words"
        );
        assert_eq!(crate::sessions::strip_injected_context(&once), "Install");
    }

    /// L8: a project lead is one row whatever happens underneath. Its
    /// handovers (rollovers) continue behind the same row: the project's
    /// key names the current one, a send to the fixed or an earlier one
    /// goes there, the successor is still the lead (prompt, depth), and its
    /// chain lists the earlier ones. The COO's chain is untouched.
    #[test]
    fn a_project_leads_handovers_keep_one_row_and_route_to_the_current_one() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let w = crate::workspaces::create(v, "Leads", "Work").unwrap();
        let lead = crate::project_agents::key(&w.id).unwrap();
        assert_eq!(lead_current(v, &w.id).as_deref(), Some(lead.as_str()));
        assert_eq!(current_of(v, &lead), None, "current before any handover");
        assert_eq!(lead_root(v, &lead).as_deref(), Some(lead.as_str()));
        session_store::start_turn(v, &lead, "claude", &w.cwd, "R-l8-a", "Plan").unwrap();
        let (b, c) = ("s-20261110-l8lead-b", "s-20261110-l8lead-c");
        crate::app_sessions::record_handover(v, &lead, b, 1).unwrap();
        session_store::start_turn(v, b, "claude", &w.cwd, "R-l8-b", "Continue").unwrap();
        crate::app_sessions::record_handover(v, b, c, 2).unwrap();
        session_store::start_turn(v, c, "codex", &w.cwd, "R-l8-c", "Continue").unwrap();
        // One row: the project names the last successor, everywhere.
        assert_eq!(lead_current(v, &w.id).as_deref(), Some(c));
        let detail = crate::workspaces::read(v, &w.id).unwrap();
        assert_eq!(detail.project_agent_key.as_deref(), Some(c));
        let listed = crate::workspaces::sessions_by_workspace(v).unwrap();
        let row = listed.iter().find(|g| g.workspace_id == w.id).unwrap();
        assert_eq!(row.project_agent_key.as_deref(), Some(c));
        // A send to the fixed or an earlier conversation goes to the current.
        assert_eq!(current_of(v, &lead).as_deref(), Some(c));
        assert_eq!(current_of(v, b).as_deref(), Some(c));
        let routed = route(v, Some(&lead)).unwrap();
        assert_eq!((routed.to.as_str(), routed.agent.as_str()), (c, "codex"));
        assert_eq!(routed.from.as_deref(), Some(lead.as_str()));
        assert!(route(v, Some(c)).is_none(), "current: no route");
        // The successor is the lead: its root, prompt and depth.
        assert_eq!(lead_root(v, c).as_deref(), Some(lead.as_str()));
        assert!(is_lead_conversation(v, b));
        assert!(crate::project_agents::context(v, c).is_some());
        assert!(crate::app_sessions::is_coordinator(v, c, Path::new(&w.cwd)));
        // Its chain: "Earlier in this conversation" lists the two before it.
        let view = chain_view(v, c).unwrap();
        assert!(!view.main);
        let earlier: Vec<&str> = view.earlier.iter().map(|l| l.key.as_str()).collect();
        assert_eq!(earlier, vec![b, lead.as_str()]);
        assert_eq!(chain_view(v, &lead).unwrap().current, c);
        // Another project's chat, and the COO's, are no lead's.
        assert!(lead_root(v, "s-20261110-plainchat").is_none());
        let main = crate::workspaces::ensure_main_agent(v).unwrap();
        session_store::start_turn(
            v,
            "s-20261110-coo00001",
            "claude",
            &main.to_string_lossy(),
            "R-l8-coo",
            "Hi",
        )
        .unwrap();
        assert!(lead_root(v, "s-20261110-coo00001").is_none());
        assert!(is_main_conversation(v, "s-20261110-coo00001"));
        assert_eq!(
            main_current(v).map(|(k, _)| k).as_deref(),
            Some("s-20261110-coo00001")
        );
        assert!(crate::project_agents::context(v, "s-20261110-coo00001").is_none());
    }

    /// Every file under `root`, by relative path, with its bytes' hash.
    fn tree_hashes(root: &Path) -> std::collections::BTreeMap<String, String> {
        use sha2::{Digest, Sha256};
        let mut out = std::collections::BTreeMap::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                let kind = entry.file_type().unwrap();
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                if kind.is_dir() {
                    out.insert(format!("{rel}/"), String::new());
                    stack.push(path);
                } else {
                    let bytes = std::fs::read(&path).unwrap();
                    out.insert(rel, format!("{:x}", Sha256::digest(&bytes)));
                }
            }
        }
        out
    }

    /// L8 moves no data: the lead chain, its earlier chats (a project's
    /// other chats; the COO has none since quick chats, 2026-11-14), its
    /// listing, routing, prompt and tool policy only read. A fixture vault's
    /// every file is byte-identical before and after.
    #[test]
    fn leads_and_earlier_chats_leave_a_fixture_vault_byte_identical() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let w = crate::workspaces::create(v, "Fixture", "Work").unwrap();
        std::fs::create_dir_all(Path::new(&w.cwd).join("outputs")).unwrap();
        std::fs::write(Path::new(&w.cwd).join("outputs/report.md"), "# Report\n").unwrap();
        let quick = crate::workspaces::ensure_quick_chats(v).unwrap();
        let main = crate::workspaces::ensure_main_agent(v).unwrap();
        let lead = crate::project_agents::key(&w.id).unwrap();
        session_store::start_turn(v, &lead, "claude", &w.cwd, "R-l8f-lead", "Plan").unwrap();
        session_store::start_turn(
            v,
            "s-20261101-l8fold01",
            "codex",
            &w.cwd,
            "R-l8f-old",
            "Old chat",
        )
        .unwrap();
        let ord = session_store::start_turn(
            v,
            "s-20261110-l8fsub01",
            "claude",
            &w.cwd,
            "R-l8f-sub",
            "Task",
        )
        .unwrap();
        session_store::finish_turn(v, "s-20261110-l8fsub01", ord, "success").unwrap();
        session_store::set_parent(
            v,
            "s-20261110-l8fsub01",
            &lead,
            session_store::FORK_SUBAGENT,
        )
        .unwrap();
        session_store::start_turn(
            v,
            "s-20261102-l8fquick",
            "claude",
            &quick.to_string_lossy(),
            "R-l8f-q",
            "Quick",
        )
        .unwrap();
        session_store::start_turn(
            v,
            "s-20261110-l8fcoo01",
            "claude",
            &main.to_string_lossy(),
            "R-l8f-coo",
            "Hi",
        )
        .unwrap();
        crate::app_sessions::record_handover(v, &lead, "s-20261110-l8flead2", 1).unwrap();
        let before = tree_hashes(v);
        assert!(
            before.keys().any(|k| k.ends_with("WORKSPACE.md")),
            "{before:?}"
        );
        // Everything L8 reads.
        let current = lead_current(v, &w.id).unwrap();
        assert_eq!(current, "s-20261110-l8flead2");
        crate::workspaces::read(v, &w.id).unwrap();
        crate::workspaces::sessions_by_workspace(v).unwrap();
        let now = chrono::Utc::now();
        let lead_view = crate::subagents::subagents_at(v, &lead, now).unwrap();
        assert_eq!(lead_view.done.len(), 1, "its subagent");
        assert!(
            lead_view
                .earlier
                .iter()
                .any(|r| r.key == "s-20261101-l8fold01"),
            "a project's other chat is under Earlier chats"
        );
        let coo_view = crate::subagents::subagents_at(v, "s-20261110-l8fcoo01", now).unwrap();
        // Quick chats (2026-11-14): they list in the sidebar's Quick chats
        // section, no longer under the COO's Earlier chats.
        assert!(
            !coo_view
                .earlier
                .iter()
                .any(|r| r.key == "s-20261102-l8fquick"),
            "a quick chat is not under the COO's Earlier chats"
        );
        chain_view(v, &current).unwrap();
        route(v, Some(&lead)).unwrap();
        crate::project_agents::context(v, &current).unwrap();
        crate::project_agents::lead_denied_tools(
            v,
            &current,
            Path::new(&w.cwd),
            crate::agent::AgentKind::Claude,
        );
        assert_eq!(tree_hashes(v), before, "the vault is byte-identical");
    }
}
