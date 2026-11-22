//! Real work gets an issue (two-Macs T3a, 2026-11-15; decision c5 of
//! `workspaces/main/outputs/hollow-next-build-decisions-2026-11-14.md`).
//!
//! A person's own chat (top level, attended, not the COO's or a project
//! lead's conversation, never a subagent: those are [`super::work`]'s) that
//! did a build, research or a deliverable gets one tracker issue; a quick
//! question or a check gets none. [`classify`] reads only host facts of the
//! turn that just ended (its receipt and the reply's length), never the
//! words of the request.
//!
//! Settings' "Track agent work" (`tracker.trackAgentWork`) decides what
//! happens to a chat that qualifies: `off` nothing; `dryRun` (the default)
//! one audit row saying what Hollow would file (`tracker_write`, action
//! `create`, outcome `skipped`, detail starting "Dry run:"), nothing sent;
//! `on` a link with `linked_by` `chat` and the run's facts (create, Started,
//! Outcome), sent by the agent work outbox ([`super::work_outbox`]) under
//! its rules. Each chat is decided once: its create key (from the chat's key
//! alone) is the audit row's request key and the create journal's key, and
//! its link is keyed by the chat, so a retry never files a second issue.
//!
//! Where it goes is T1's rule ([`super::linear::team_for`]): a chat in a
//! workspace that names its tracker project is tracked work, filed in team
//! AI in that project; any other chat is Hollow-only work, filed in team
//! HOL with its `agent` label (refused, and audited once, when HOL isn't set
//! up). The R3 guard and the adapters still refuse every other team.

use super::linear::{self, Work};
use super::work::{self, Place, Prepared, Resolution, StartFacts};
use crate::config::Config;
use crate::session_store;
use crate::tracker::settings::AgentWork;
use rusqlite::{params, OptionalExtension};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A turn that ran at least this long, with a long answer, was research.
pub(crate) const RESEARCH_MINUTES: i64 = 10;
/// The answer length (characters) research needs besides its time.
pub(crate) const RESEARCH_REPLY_CHARS: usize = 1500;

/// What kind of real work a chat did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChatKind {
    Build,
    Research,
    Deliverable,
}

impl ChatKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ChatKind::Build => "build",
            ChatKind::Research => "research",
            ChatKind::Deliverable => "deliverable",
        }
    }
}

/// The host facts of one ended turn that [`classify`] reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TurnFacts {
    /// Pull requests this turn opened or merged and its pushes, from its own
    /// tool calls (the receipt's `pullRequests` and outside effects), never
    /// from words in the reply.
    pub prs: usize,
    /// Files changed or commits made in the turn's git repositories.
    pub repo_changes: usize,
    /// Files the turn itself wrote in its workspace's own `outputs/`.
    pub outputs: usize,
    pub working_ms: i64,
    pub reply_chars: usize,
}

/// Build (a pull request, or changes in a repository), deliverable (a file
/// in `outputs/`), research (a long turn with a long answer), else none: a
/// quick question or a check.
pub(crate) fn classify(facts: &TurnFacts) -> Option<ChatKind> {
    if facts.prs > 0 || facts.repo_changes > 0 {
        Some(ChatKind::Build)
    } else if facts.outputs > 0 {
        Some(ChatKind::Deliverable)
    } else if facts.working_ms >= RESEARCH_MINUTES * 60_000
        && facts.reply_chars >= RESEARCH_REPLY_CHARS
    {
        Some(ChatKind::Research)
    } else {
        None
    }
}

/// [`TurnFacts`] from a turn's receipt and reply. `workspace` is the chat
/// workspace's vault-relative folder (`workspaces/<id>`): only a file in its
/// own `outputs/` is a deliverable (none for a chat in no workspace).
pub(crate) fn facts_of(
    receipt: &crate::evidence::EvidenceReceipt,
    reply: &str,
    workspace: Option<&str>,
) -> TurnFacts {
    let outputs = workspace.map_or(0, |folder| {
        let prefix = format!("{}/outputs/", folder.trim_end_matches('/'));
        receipt
            .files
            .iter()
            .filter(|f| f.path.starts_with(&prefix) && !f.path[prefix.len()..].is_empty())
            .filter(|f| f.attribution != "shared" && f.change_type != "deleted")
            .count()
    });
    let pushes = receipt.outside_effects.as_ref().map_or(0, |effects| {
        effects
            .recorded
            .iter()
            .filter(|e| e.kind == "shell" && e.may_have_happened())
            .filter(|e| {
                e.label.starts_with("git push") || e.label.starts_with("GitHub pull request")
            })
            .count()
    });
    let repo_changes = receipt
        .repo_changes
        .iter()
        .map(|repo| {
            let committed = repo.head_after.is_some() && repo.head_after != repo.head_before;
            repo.total_files.max(repo.files.len()) + usize::from(committed)
        })
        .sum();
    let at = |text: &str| chrono::DateTime::parse_from_rfc3339(text).ok();
    let working_ms = match (at(&receipt.started_at), at(&receipt.finished_at)) {
        (Some(start), Some(end)) => (end - start).num_milliseconds().max(0),
        _ => 0,
    };
    TurnFacts {
        prs: receipt.pull_requests.len() + pushes,
        repo_changes,
        outputs,
        working_ms,
        reply_chars: reply.chars().count(),
    }
}

/// What this process runs with, per vault (registered at setup, like the
/// agent work switch; a change applies after a restart). Unregistered is
/// off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Running {
    pub mode: AgentWork,
    /// Team HOL is set up (`personalTeamId`).
    pub hol: bool,
    /// The tracker's provider is Linear (T1's routing rule applies).
    pub linear: bool,
}

static RUNNING: once_cell::sync::Lazy<Mutex<HashMap<PathBuf, Running>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(HashMap::new()));

/// Setup: record the running config for its vault. Tracking needs the
/// tracker set up for team AI; otherwise it is off.
pub(crate) fn register_running(config: &Config) {
    set_running(&config.vault_path, running_of(config));
}

/// What `config` runs with (T3b's Waiting on you copies read it too).
pub(crate) fn running_of(config: &Config) -> Running {
    let ready = work::ai_scope(config) && config.tracker.provider.is_some();
    Running {
        mode: if ready {
            config.tracker.agent_work
        } else {
            AgentWork::Off
        },
        hol: config.tracker.personal_team_id.is_some(),
        linear: config.tracker.provider.as_deref() == Some(linear::PROVIDER),
    }
}

pub(crate) fn set_running(vault: &Path, running: Running) {
    RUNNING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(vault.to_path_buf(), running);
}

fn running(vault: &Path) -> Option<Running> {
    RUNNING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(vault)
        .copied()
}

/// The running `tracker.trackAgentWork` mode for `vault` (T3g's dry run
/// audit reads it); unregistered is off.
pub(crate) fn running_mode(vault: &Path) -> AgentWork {
    running(vault).map_or(AgentWork::Off, |r| r.mode)
}

/// The chat's create key: its key alone, so every retry finds the same one.
pub(crate) fn create_key(session_key: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("chat\n{session_key}").as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("work-{}", &hex[..32])
}

/// Where a chat's issue goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Destination {
    /// Team AI, in the workspace's tracker project.
    Ai { project: String },
    /// Team HOL, no project, the `agent` label.
    Hol,
}

/// T1's rule for a chat in `place`: a tracked project's chat is tracked
/// work (AI), any other is Hollow-only (HOL). `Err` when that team isn't
/// one Hollow may file in here.
pub(crate) fn destination(place: &Place, running: &Running) -> Result<Destination, String> {
    let (work, dest) = match place {
        Place::Tracked(project) => (
            Work::Tracked,
            Destination::Ai {
                project: project.clone(),
            },
        ),
        Place::QuickChats | Place::Untracked => (Work::HollowOnly, Destination::Hol),
    };
    if running.linear {
        linear::team_for(work, None)?;
    }
    if dest == Destination::Hol && !running.hol {
        return Err("Hollow-only work goes to team Personal (HOL), which isn't set up here, so this chat gets no issue.".into());
    }
    Ok(dest)
}

/// The created issue's description: host facts only (what kind of work,
/// where, which chat, how its first real turn ended). The person's request
/// is never quoted; the title is the chat's own title.
pub(crate) fn description(
    kind: ChatKind,
    workspace_id: Option<&str>,
    session_key: &str,
    status: &str,
) -> String {
    format!(
        "A {} chat in {} (Hollow chat session {session_key}); its first real-work turn ended with status {status}.\n\n+++ Agent notes\n\nCreated by Hollow for a chat's real work (Settings, Track agent work). Hollow posts the chat's Started and Outcome comments here; quick questions and checks get no issue. The chat's own words stay in Hollow.\n\n+++",
        kind.as_str(),
        workspace_id.map_or_else(|| "no workspace".to_string(), |id| format!("workspace {id}")),
    )
}

/// The chat's folder and title, from the sessions store.
fn chat_row(vault: &Path, session_key: &str) -> Option<(String, Option<String>)> {
    session_store::with_read(vault, |conn| {
        conn.query_row(
            "SELECT cwd, title FROM sessions WHERE key = ?1",
            params![session_key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| format!("chat work: {e}"))
    })
    .ok()
    .flatten()
}

/// Whether the chat (with every conversation it was handed over from) was
/// already decided: a link (filed, or a subagent's), or an audit row under
/// its create key (a dry run, a refusal, a create that failed). A
/// successor with no link of its own takes its predecessor's, so its turns
/// report on the same issue. A chat link with no issue and no queued fact
/// (left by a crash before the transaction that records both) is dropped
/// and the chat decided again.
fn decided(vault: &Path, chain: &[String], key: &str) -> bool {
    use crate::audit::kind;
    let me = &chain[0];
    if let Some(link) = work::link_for(vault, me) {
        let orphan = link.linked_by == work::linked_by::CHAT
            && link.issue_id.is_none()
            && !work::has_events(vault, me);
        if !orphan {
            return true;
        }
        let _ = work::drop_link(vault, me);
    }
    if let Some(from) = chain[1..]
        .iter()
        .find(|k| work::link_for(vault, k).is_some())
    {
        work::continue_link(vault, from, me);
        return true;
    }
    chain.iter().any(|member| {
        [kind::TRACKER_WRITE, kind::TRACKER_WRITE_FAILED]
            .into_iter()
            .flat_map(|k| crate::audit::rows_of(vault, member, k))
            .any(|row| row.request_key.as_deref() == Some(key))
    })
}

/// What the person's own chat `session_key` is (from chat.rs, after the
/// turn's receipt and X1's own after-turn): `person_chat` false (an agent's
/// chat, an automation, a check-in, a bare run) does nothing. A turn that
/// failed is filed only when its own facts show real work. Never fails the
/// turn.
pub(crate) fn after_turn(
    vault: &Path,
    session_key: &str,
    person_chat: bool,
    status: &str,
    receipt: &crate::evidence::EvidenceReceipt,
    reply: &str,
) {
    let Some(running) = running(vault).filter(|r| r.mode != AgentWork::Off) else {
        return;
    };
    // A stopped turn is the person's call, not a finished piece of work.
    if !person_chat || status == "cancelled" {
        return;
    }
    if crate::chat_chain::is_main_conversation(vault, session_key)
        || crate::chat_chain::is_lead_conversation(vault, session_key)
    {
        return;
    }
    // A handover's successor is the same chat: one key for the whole chain,
    // from its first conversation.
    let mut chain = vec![session_key.to_string()];
    chain.extend(crate::chat_chain::predecessors(vault, session_key));
    let key = create_key(chain.last().map_or(session_key, String::as_str));
    if decided(vault, &chain, &key) {
        return;
    }
    let Some((cwd, title)) = chat_row(vault, session_key) else {
        return;
    };
    let workspace = crate::workspaces::workspace_for_cwd(vault, Path::new(&cwd))
        .ok()
        .flatten();
    let Some(kind) = classify(&facts_of(
        receipt,
        reply,
        workspace.as_ref().map(|w| w.path.as_str()),
    )) else {
        return;
    };
    let workspace_id = workspace.as_ref().map(|w| w.id.clone());
    let folder = workspace_id
        .as_deref()
        .and_then(|id| crate::workspaces::folder(vault, id).ok())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let provider = if running.linear { linear::PROVIDER } else { "" };
    let place = work::place_in(provider, workspace_id.as_deref(), &folder);
    let title = title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| format!("A {} chat", kind.as_str()));
    let title = work::clip(crate::claude_runner::redact(&title).trim(), 200);
    let decision = destination(&place, &running);
    let (team_key, project) = match &decision {
        Ok(Destination::Ai { project }) => (linear::TEAM_KEY, project.clone()),
        _ => (linear::PERSONAL_TEAM_KEY, String::new()),
    };
    let audit = |outcome: &str, detail: String| {
        crate::audit::tracker_write(
            vault,
            crate::audit::Write {
                action: "create",
                session_key: Some(session_key),
                asked_by: "person".into(),
                workspace_id: workspace_id.as_deref(),
                after: Some(serde_json::json!({
                    "title": title, "team": team_key, "projectId": project, "kind": kind.as_str()
                })),
                request_key: Some(&key),
                outcome,
                detail: Some(detail),
                ..crate::audit::Write::default()
            },
        );
    };
    let dest = match decision {
        Ok(dest) => dest,
        Err(why) => {
            audit("failed", why);
            return;
        }
    };
    if running.mode == AgentWork::DryRun {
        audit(
            "skipped",
            format!(
                "Dry run: Hollow would file this {} chat as an issue in team {team_key}; nothing was sent.",
                kind.as_str()
            ),
        );
        return;
    }
    let body = description(kind, workspace_id.as_deref(), session_key, status);
    let facts = StartFacts {
        title,
        workspace_id,
        cwd: folder,
        parent_key: None,
        depth: 1,
        asked_by: "person".into(),
        ..StartFacts::default()
    };
    let (project_id, team_id) = match dest {
        Destination::Ai { project } => (project, None),
        Destination::Hol => (String::new(), Some(linear::PERSONAL_TEAM_ID.to_string())),
    };
    let queued = work::begin_as(
        vault,
        session_key,
        Prepared {
            facts,
            resolution: Resolution::Create {
                project_id,
                create_key: key,
            },
            track: true,
        },
        work::Created {
            by: work::linked_by::CHAT,
            team_id,
            description: Some(body),
        },
    );
    // The turn that qualified is the run's first: its Outcome now.
    if queued {
        work::after_turn(vault, session_key, status, Some(receipt), reply);
    }
}

#[cfg(test)]
#[path = "chat_work_tests.rs"]
mod tests;
