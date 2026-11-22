//! Which Hollow-owned MCP servers attach to a run, per surface (Paperclip
//! PR 13; cross-m34 §4). One place decides, so the chat brief, the Claude
//! preapprovals (`agent::owned_preapprovals`), Codex's approval mode and a
//! later fingerprint of the server names all agree.
//!
//! - Chat, with an agent that supports owned MCP (Claude, Codex): teamwork
//!   (when a teammate CLI is installed and the toggle is on, and the chat is
//!   not in `plan` mode: a delegate always writes), computer control
//!   and ask-your-vault (each behind its Settings toggle), and question cards
//!   (always: only chat has a user who can answer).
//! - Chat with Antigravity: the same servers as Claude and Codex, handed to
//!   `agy acp` in `session/new` for this chat only (2026-11-20). Its
//!   workflows and teammates run `agy -p`, which takes none; with
//!   `HOLLOW_AGY_HEADLESS` set it is a chat without owned MCP again, and
//!   question cards fall back to a fenced `hollow-questions` block
//!   (`questions_fenced`).
//! - Chat and workflows, with an owned-MCP agent: task tools
//!   (`hollow_tasks`, Paperclip PR 12) while Settings, Agent tools, Task
//!   tools is on and the GUI's task broker is listening, read-only or
//!   read-write by the caller's [`TasksAccess`] (a `plan` run reads only).
//! - Chat and workflows, with an owned-MCP agent: Hollow app tools
//!   (`hollow_app`, PR A1) while Settings, Connectors, Built into Hollow, Hollow app is on and
//!   the GUI's app broker is listening, at the caller's [`AppAccess`]; a
//!   tool-restricted workflow only when its allowed tools name the server
//!   (`AppAccess::for_workflow` returns `None` otherwise).
//! - Workflow: teamwork (never in `plan` mode), task tools and app tools. No
//!   question cards (nobody can answer an unattended run) and no computer
//!   control (nobody can answer its confirm dialogs).
//!   `workflows::prepare_spawn` builds its config here.
//! - Chat in a build workspace's worktree (Build PR B6), not in `plan`
//!   mode: services (`hollow_services`), while the GUI's service broker
//!   listens, with a grant tied to the turn's run id, chat and mode. Never
//!   for workflows. Preapproved in `auto` and bypass only
//!   (`agent::OWNED_MODE_PREAPPROVED`); outside bypass every service it
//!   starts is bounded to the worktree and the chat (merged plan decision 1,
//!   `services::bounded`).
//! - Heartbeat, one-shots, delegates, ingest: nothing.

use crate::agent::{self, AgentKind};
use crate::app_mcp::{AppAccess, AppGrant};
use crate::tasks_mcp::TasksAccess;
use crate::usage_ledger::{DelegateParent, Surface};
use serde_json::{json, Map, Value};
use std::path::Path;

/// Whether a chat with `agent` gets `hollow_vault` (and so its
/// `read_source` reader): an agent with owned MCP while Ask your vault is
/// on. The same rule [`server_map_for`] attaches by, decided before the
/// workspace envelope is built (agent effectiveness F3 part 2).
pub(crate) fn chat_has_vault_reader(agent: crate::agent::AgentKind) -> bool {
    crate::agent::capabilities(agent).owned_mcp && crate::vault_mcp::vault_mcp_enabled()
}

/// Which Hollow capabilities a spawn actually attached, for the environment
/// brief and the workspace capability list, so the agent is told exactly (and
/// only) what it has.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AttachedCaps {
    /// Vetted local computer controls are attached.
    pub(crate) computer: bool,
    /// `hollow_vault` (read-only `search_vault`) is attached.
    pub(crate) vault: bool,
    /// Per-teammate: the team server attaches when any other CLI is
    /// installed, but lists a delegation target only for an installed one, so
    /// the brief must not advertise a teammate the server won't offer.
    pub(crate) team_claude: bool,
    pub(crate) team_codex: bool,
    pub(crate) team_antigravity: bool,
    pub(crate) team_grok: bool,
    /// `hollow_code_context` (read-only callers, impact and reads of the
    /// cwd's checkout, CC2) is attached.
    pub(crate) code_context: bool,
    /// `hollow_questions` (`ask_user_questions`) is attached.
    pub(crate) questions: bool,
    /// `hollow_tasks` is attached (Paperclip PR 12).
    pub(crate) tasks: bool,
    /// The attached task server lists only its read tools (plan mode, or a
    /// tool-restricted automation).
    pub(crate) tasks_read_only: bool,
    /// `hollow_app` is attached (PR A1).
    pub(crate) app: bool,
    /// The attached app server can only read (plan mode, or
    /// an automation limited to reading).
    pub(crate) app_read_only: bool,
    /// `hollow_services` is attached (Build PR B6: a chat in a build
    /// worktree, not in plan mode).
    pub(crate) services: bool,
    /// No owned MCP: the agent is told to end its reply with a fenced
    /// `hollow-questions` block instead.
    pub(crate) questions_fenced: bool,
    /// The agent these were attached for ([`server_map_for`]): the
    /// capability index ends with its line of the per-provider contract
    /// (`capability_contract::guidance_line`, F9). `None` adds no line.
    pub(crate) agent: Option<AgentKind>,
}

impl AttachedCaps {
    pub(crate) fn teamwork(&self) -> bool {
        self.team_claude || self.team_codex || self.team_antigravity || self.team_grok
    }

    /// The owned servers these flags say were attached, by name, in the
    /// capability index's order: the same names the server map (and so the
    /// run manifest's `capabilities.ownedServers`) holds.
    #[cfg(test)]
    pub(crate) fn server_names(&self) -> Vec<&'static str> {
        [
            (self.questions, "hollow_questions"),
            (self.vault, "hollow_vault"),
            (self.code_context, crate::code_context_mcp::SERVER_NAME),
            (self.app, crate::app_mcp::SERVER_NAME),
            (self.tasks, crate::tasks_mcp::SERVER_NAME),
            (self.computer, "hollow_computer"),
            (self.teamwork(), "hollow_team"),
            (self.services, crate::services_mcp::SERVER_NAME),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect()
    }
}

/// Every owned server name [`AttachedCaps::server_names`] can give.
#[cfg(test)]
pub(crate) const OWNED_SERVER_NAMES: [&str; 8] = [
    "hollow_questions",
    "hollow_vault",
    "hollow_code_context",
    "hollow_app",
    "hollow_tasks",
    "hollow_computer",
    "hollow_team",
    "hollow_services",
];

/// Serialises the tests that flip a process-wide server switch
/// (`computer_mcp`'s and `vault_mcp`'s) with those that read one through
/// [`server_map_for`] and need it to hold still (OB03 review).
#[cfg(test)]
pub(crate) static SERVER_TOGGLES: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A chat in one of Hollow's own scratch folders for read-only model calls
/// (a pressure-test persona) runs bare (overnight S7):
/// everything it may use is in its prompt, and plan mode alone still
/// attached vault search, app and task reads and the question tool, so a
/// draft or task carrying instructions could reach the vault through them.
pub(crate) fn bare_scratch(vault: &std::path::Path, cwd: &std::path::Path) -> bool {
    crate::stakeholder_sim::owns_scratch(vault, cwd)
}

/// No Hollow server and nothing attached, for a [`bare_scratch`] chat.
pub(crate) fn for_bare(
    bare: bool,
    servers: Map<String, Value>,
    caps: AttachedCaps,
) -> (Map<String, Value>, AttachedCaps) {
    if bare {
        // The provider's contract line stays; everything attached goes.
        let agent = caps.agent;
        (
            Map::new(),
            AttachedCaps {
                agent,
                ..AttachedCaps::default()
            },
        )
    } else {
        (servers, caps)
    }
}

/// An unattended turn's owned servers (master plan #1: the night queue and
/// its runbooks): no teammates (a teammate could run a toolchain or reach
/// out with no one watching), no computer control and no services.
pub(crate) fn for_unattended(
    unattended: bool,
    mut servers: Map<String, Value>,
    mut caps: AttachedCaps,
) -> (Map<String, Value>, AttachedCaps) {
    if unattended {
        for name in [
            "hollow_team",
            "hollow_computer",
            crate::services_mcp::SERVER_NAME,
        ] {
            servers.remove(name);
        }
        caps.team_claude = false;
        caps.team_codex = false;
        caps.team_antigravity = false;
        caps.team_grok = false;
        caps.computer = false;
        caps.services = false;
    }
    (servers, caps)
}

/// The Hollow servers a run on `parent.surface` attaches, as an mcpServers
/// map, and what that attached. `perm` is the run's canonical permission
/// mode, `cwd` its working folder (the team and computer servers'
/// `HOLLOW_VAULT`), `vault_root` the configured vault. The vault server gets
/// `vault_root` and, separately, the connected project `cwd` is scoped to
/// (`vault_mcp::server_entry`; agent effectiveness F1).
///
/// A `plan` (read-only) run gets no teamwork: a delegate always runs with
/// write access, so it would be a way around the read-only mode. The Claude
/// and Codex preapprovals leave `hollow_team` out for plan too
/// (`agent::owned_preapprovals`), for any caller that attaches it anyway.
///
/// `tasks` is what the task server (whose `HOLLOW_VAULT_ROOT` is
/// `vault_root`) may do in this run (`TasksAccess::for_chat` /
/// `for_workflow`). `app` is what the Hollow app server may do, or `None`
/// when it must not attach (`AppAccess::for_chat` / `for_workflow`).
pub(crate) fn server_map_for(
    agent: AgentKind,
    perm: &str,
    cwd: &Path,
    vault_root: &Path,
    parent: &DelegateParent,
    tasks: TasksAccess,
    app: Option<AppGrant>,
) -> (Map<String, Value>, AttachedCaps) {
    let mut servers = Map::new();
    let mut caps = AttachedCaps {
        agent: Some(agent),
        ..AttachedCaps::default()
    };
    let owned = agent::capabilities(agent).owned_mcp;
    let team = perm != "plan";
    match parent.surface {
        Surface::Chat if owned => {
            if team {
                attach_team(
                    &mut servers,
                    &mut caps,
                    agent,
                    perm,
                    cwd,
                    vault_root,
                    parent,
                );
            }
            if let Some((name, entry)) =
                crate::computer_mcp::server_entry(cwd, perm, &parent.run_id, &parent.scope)
            {
                caps.computer = true;
                servers.insert(name.to_string(), entry);
            }
            // The configured vault, never the cwd (a workspace folder or a
            // build worktree is not the vault), with the run's connected
            // project as its own field: the scope automatic retrieval uses.
            // Its reader (`read_source`) also gets the run's workspace, whose
            // folder it may read (agent effectiveness F3 part 2), and the
            // run's scratch folder and id (repo features R3). The app
            // server attaches first, so the vault server can be told whether
            // this run has it (S03: its lexical-only reply names
            // `run_action vault.retrieve`); the map is ordered by name, so
            // the order here changes nothing else.
            attach_app(&mut servers, &mut caps, parent, app);
            let scope = crate::retrieval_broker::resolve_scope(vault_root, cwd);
            if let Some((name, entry)) = crate::vault_mcp::server_entry(
                vault_root,
                scope.project.as_deref(),
                scope.workspace.as_deref(),
                caps.app,
                parent.scratch.as_deref(),
                &parent.run_id,
            ) {
                caps.vault = true;
                servers.insert(name.to_string(), entry);
            }
            attach_code_context(&mut servers, &mut caps, cwd, vault_root);
            if let Some((name, entry)) = crate::question_mcp::server_entry() {
                caps.questions = true;
                servers.insert(name.to_string(), entry);
            }
            attach_tasks(&mut servers, &mut caps, vault_root, parent, tasks);
            // Services that outlive the turn (Build PR B6): only in a build
            // workspace's worktree, never read-only. The grant names this
            // run, its chat and its mode (bounded outside bypass).
            if team && crate::build_workspaces::workspace_for_worktree(vault_root, cwd).is_some() {
                if let Some((name, entry)) =
                    crate::services::server_entry(cwd, &parent.run_id, &parent.scope, perm)
                {
                    caps.services = true;
                    servers.insert(name.to_string(), entry);
                }
            }
        }
        Surface::Chat => caps.questions_fenced = true,
        // A workflow runs `agy -p`, which takes no servers (only an Antigravity
        // chat turn over ACP does).
        Surface::Workflow if owned && agent != AgentKind::Antigravity => {
            if team {
                attach_team(
                    &mut servers,
                    &mut caps,
                    agent,
                    perm,
                    cwd,
                    vault_root,
                    parent,
                );
            }
            attach_tasks(&mut servers, &mut caps, vault_root, parent, tasks);
            attach_app(&mut servers, &mut caps, parent, app);
            attach_code_context(&mut servers, &mut caps, cwd, vault_root);
        }
        _ => {}
    }
    (servers, caps)
}

/// `hollow_code_context` (CC2): read only, so plan and unattended runs keep
/// it; only when the toggle is on and `cwd` is a git checkout outside the
/// vault (`code_context_mcp::server_entry`).
fn attach_code_context(
    servers: &mut Map<String, Value>,
    caps: &mut AttachedCaps,
    cwd: &Path,
    vault_root: &Path,
) {
    if let Some((name, entry)) = crate::code_context_mcp::server_entry(cwd, Some(vault_root)) {
        caps.code_context = true;
        servers.insert(name.to_string(), entry);
    }
}

fn attach_team(
    servers: &mut Map<String, Value>,
    caps: &mut AttachedCaps,
    agent: AgentKind,
    perm: &str,
    cwd: &Path,
    vault_root: &Path,
    parent: &DelegateParent,
) {
    if let Some((name, entry)) =
        crate::team_mcp::server_entry_for(cwd, vault_root, agent, parent, perm)
    {
        // Mirror team_mcp::tool_specs' per-teammate gating.
        caps.team_claude = agent != AgentKind::Claude && agent::is_available(AgentKind::Claude);
        caps.team_codex = agent != AgentKind::Codex && agent::is_available(AgentKind::Codex);
        caps.team_antigravity =
            agent != AgentKind::Antigravity && agent::is_available(AgentKind::Antigravity);
        caps.team_grok = agent != AgentKind::Grok && agent::is_available(AgentKind::Grok);
        servers.insert(name.to_string(), entry);
    }
}

/// The task server, with the vault root (never the run's cwd) and the run's
/// access. Nothing when the switch is off or the broker isn't listening.
fn attach_tasks(
    servers: &mut Map<String, Value>,
    caps: &mut AttachedCaps,
    vault_root: &Path,
    parent: &DelegateParent,
    access: TasksAccess,
) {
    if let Some((name, entry)) = crate::tasks_mcp::server_entry(vault_root, parent, access) {
        caps.tasks = true;
        caps.tasks_read_only = access == TasksAccess::ReadOnly;
        servers.insert(name.to_string(), entry);
    }
}

/// The Hollow app server at the run's grant (access, depth, permission
/// mode). Nothing when `grant` is `None`, the switch is off or the broker
/// isn't listening.
fn attach_app(
    servers: &mut Map<String, Value>,
    caps: &mut AttachedCaps,
    parent: &DelegateParent,
    grant: Option<AppGrant>,
) {
    let Some(grant) = grant else { return };
    if let Some((name, entry)) = crate::app_mcp::server_entry(parent, &grant) {
        caps.app = true;
        caps.app_read_only = grant.access == AppAccess::Read;
        servers.insert(name.to_string(), entry);
    }
}

/// The inline `--mcp-config` JSON for a server map, or `None` when empty.
pub(crate) fn merge(servers: Map<String, Value>) -> Option<String> {
    if servers.is_empty() {
        None
    } else {
        Some(json!({ "mcpServers": servers }).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_chat_attaches_nothing() {
        let mut servers = Map::new();
        servers.insert("hollow_vault".into(), json!({}));
        servers.insert("hollow_app".into(), json!({}));
        let caps = AttachedCaps {
            computer: true,
            services: true,
            ..AttachedCaps::default()
        };
        let (kept, kept_caps) = for_bare(true, servers.clone(), caps);
        assert!(kept.is_empty());
        assert!(!kept_caps.computer && !kept_caps.services);
        let (same, same_caps) = for_bare(false, servers.clone(), caps);
        assert_eq!(same, servers);
        assert!(same_caps.computer);
    }

    fn parent(surface: Surface) -> DelegateParent {
        DelegateParent {
            surface,
            scope: "scope".into(),
            run_id: "run-1".into(),
            scratch: None,
            tool_limits: Default::default(),
        }
    }

    #[test]
    fn chat_attaches_question_cards_and_workflows_never_do() {
        let dir = std::path::Path::new("/tmp");
        for agent in [AgentKind::Claude, AgentKind::Codex] {
            let (servers, caps) = server_map_for(
                agent,
                "acceptEdits",
                dir,
                dir,
                &parent(Surface::Chat),
                TasksAccess::ReadWrite,
                None,
            );
            let questions = servers
                .get("hollow_questions")
                .expect("chat attaches question cards");
            assert_eq!(questions["args"], json!(["questions-mcp"]));
            assert!(caps.questions && !caps.questions_fenced);
            // The flags name exactly the attached servers (what the
            // capability index lists, F6).
            let mut named = caps.server_names();
            named.sort_unstable();
            let mut keys: Vec<&str> = servers.keys().map(String::as_str).collect();
            keys.sort_unstable();
            assert_eq!(named, keys);

            let (servers, caps) = server_map_for(
                agent,
                "acceptEdits",
                dir,
                dir,
                &parent(Surface::Workflow),
                TasksAccess::ReadWrite,
                None,
            );
            for never in ["hollow_questions", "hollow_computer", "hollow_vault"] {
                assert!(
                    !servers.contains_key(never),
                    "workflows never attach {never}"
                );
            }
            assert!(!caps.questions && !caps.questions_fenced && !caps.computer && !caps.vault);
        }
        let (servers, caps) = server_map_for(
            AgentKind::Antigravity,
            "acceptEdits",
            dir,
            dir,
            &parent(Surface::Chat),
            TasksAccess::ReadWrite,
            None,
        );
        // Antigravity chats take the owned servers over ACP (2026-11-20).
        assert!(servers.contains_key("hollow_questions"));
        assert!(caps.questions && !caps.questions_fenced);
        for surface in [
            Surface::Heartbeat,
            Surface::Oneshot,
            Surface::Delegate,
            Surface::Ingest,
        ] {
            let (servers, caps) = server_map_for(
                AgentKind::Claude,
                "acceptEdits",
                dir,
                dir,
                &parent(surface),
                TasksAccess::ReadWrite,
                None,
            );
            assert!(servers.is_empty());
            assert!(!caps.questions_fenced);
        }
        // A read-only run never attaches teamwork, on either surface, but a
        // plan chat keeps its question cards.
        for agent in [AgentKind::Claude, AgentKind::Codex] {
            for surface in [Surface::Chat, Surface::Workflow] {
                let (servers, caps) = server_map_for(
                    agent,
                    "plan",
                    dir,
                    dir,
                    &parent(surface),
                    TasksAccess::ReadOnly,
                    None,
                );
                assert!(
                    !servers.contains_key("hollow_team") && !caps.teamwork(),
                    "{surface:?}"
                );
            }
            let (servers, _) = server_map_for(
                agent,
                "plan",
                dir,
                dir,
                &parent(Surface::Chat),
                TasksAccess::ReadOnly,
                None,
            );
            assert!(servers.contains_key("hollow_questions"));
        }
        assert_eq!(merge(Map::new()), None);
    }

    /// Services (Build PR B6) attach only to a Claude or Codex chat whose cwd
    /// is a live build worktree, never in plan mode, never to workflows, and
    /// only while the service broker listens.
    #[tokio::test]
    async fn services_attach_only_to_build_worktree_chats() {
        let (_tmp, repo, vault, roots) = crate::build_workspaces::tests::fixture();
        let made = crate::build_workspaces::create(
            &vault,
            &roots,
            crate::build_workspaces::CreateInput {
                title: "Serve".into(),
                goal: "G".into(),
                repo: repo.to_string_lossy().into(),
                branch: None,
                base: None,
            },
        )
        .await
        .unwrap();
        let tree = std::path::PathBuf::from(made.workspace.cwd.clone());
        let sock = std::path::PathBuf::from("/tmp/hollow/services-7.sock");
        crate::services::with_attach_override(Some(sock.clone()), || {
            for agent in [AgentKind::Claude, AgentKind::Codex] {
                let (servers, caps) = server_map_for(
                    agent,
                    "bypassPermissions",
                    &tree,
                    &vault,
                    &parent(Surface::Chat),
                    TasksAccess::ReadWrite,
                    None,
                );
                let entry = servers
                    .get("hollow_services")
                    .expect("a build worktree chat gets services");
                assert_eq!(entry["args"], json!(["services-mcp"]));
                assert_eq!(
                    entry["env"]["HOLLOW_SERVICES_SOCK"],
                    json!(sock.to_string_lossy())
                );
                assert_eq!(
                    entry["env"]["HOLLOW_SERVICES_RUN"],
                    json!("run-1"),
                    "the grant names its run"
                );
                assert_eq!(entry["env"]["HOLLOW_SERVICES_CHAT"], json!("scope"));
                assert_eq!(
                    entry["env"]["HOLLOW_SERVICES_PERM"],
                    json!("bypassPermissions")
                );
                assert!(caps.services);
                let (servers, _) = server_map_for(
                    agent,
                    "auto",
                    &tree,
                    &vault,
                    &parent(Surface::Chat),
                    TasksAccess::ReadWrite,
                    None,
                );
                assert_eq!(
                    servers["hollow_services"]["env"]["HOLLOW_SERVICES_PERM"],
                    json!("auto"),
                    "auto's grant can't claim bypass"
                );
                let (servers, caps) = server_map_for(
                    agent,
                    "plan",
                    &tree,
                    &vault,
                    &parent(Surface::Chat),
                    TasksAccess::ReadOnly,
                    None,
                );
                assert!(
                    !servers.contains_key("hollow_services") && !caps.services,
                    "never read-only"
                );
                let (servers, _) = server_map_for(
                    agent,
                    "bypassPermissions",
                    &tree,
                    &vault,
                    &parent(Surface::Workflow),
                    TasksAccess::ReadWrite,
                    None,
                );
                assert!(
                    !servers.contains_key("hollow_services"),
                    "never automations"
                );
                let (servers, _) = server_map_for(
                    agent,
                    "bypassPermissions",
                    &vault,
                    &vault,
                    &parent(Surface::Chat),
                    TasksAccess::ReadWrite,
                    None,
                );
                assert!(
                    !servers.contains_key("hollow_services"),
                    "not outside a worktree"
                );
            }
            let (servers, _) = server_map_for(
                AgentKind::Claude,
                "bypassPermissions",
                &tree,
                &vault,
                &parent(Surface::Chat),
                TasksAccess::ReadWrite,
                None,
            );
            let config = merge(servers).unwrap();
            assert!(
                !crate::agent::owned_preapprovals(Some(&config), "acceptEdits")
                    .iter()
                    .any(|s| s.contains("hollow_services")),
                "not preapproved in acceptEdits"
            );
            assert!(
                crate::agent::owned_preapprovals(Some(&config), "bypassPermissions")
                    .contains(&"mcp__hollow_services"),
                "bypass approves everything (2026-10-02)"
            );
        });
        crate::services::with_attach_override(None, || {
            let (servers, _) = server_map_for(
                AgentKind::Claude,
                "bypassPermissions",
                &tree,
                &vault,
                &parent(Surface::Chat),
                TasksAccess::ReadWrite,
                None,
            );
            assert!(
                !servers.contains_key("hollow_services"),
                "no broker, no server"
            );
        });
    }

    /// Task tools attach to Claude and Codex chats and workflows (read-only
    /// for a plan run), never to Antigravity or the other surfaces, and only
    /// while the switch is on and the broker listens. The broker's socket is
    /// stood in for on this thread (`tasks_mcp::with_attach_override`).
    #[test]
    fn task_tools_attach_to_owned_chat_and_workflow_runs_only() {
        let (cwd, root) = (Path::new("/tmp/work"), Path::new("/vault"));
        let sock = std::path::PathBuf::from("/tmp/hollow/tasks-7.sock");
        crate::tasks_mcp::with_attach_override(true, Some(sock.clone()), || {
            for agent in [AgentKind::Claude, AgentKind::Codex] {
                for surface in [Surface::Chat, Surface::Workflow] {
                    for (perm, access, wire) in [
                        ("acceptEdits", TasksAccess::ReadWrite, "write"),
                        ("plan", TasksAccess::ReadOnly, "read"),
                    ] {
                        let (servers, caps) =
                            server_map_for(agent, perm, cwd, root, &parent(surface), access, None);
                        let entry = servers
                            .get("hollow_tasks")
                            .unwrap_or_else(|| panic!("{agent:?} {surface:?} {perm}"));
                        assert_eq!(entry["args"], json!(["tasks-mcp"]));
                        let env = &entry["env"];
                        assert_eq!(env["HOLLOW_TASKS_ACCESS"], json!(wire));
                        assert_eq!(
                            env["HOLLOW_VAULT_ROOT"],
                            json!("/vault"),
                            "the vault root, never the cwd"
                        );
                        assert_eq!(env["HOLLOW_TASKS_SOCK"], json!(sock.to_string_lossy()));
                        assert_eq!(env["HOLLOW_RUN_SURFACE"], json!(surface.as_str()));
                        assert!(caps.tasks);
                        assert_eq!(caps.tasks_read_only, access == TasksAccess::ReadOnly);
                    }
                }
            }
            // A chat over ACP gets task tools; a workflow runs `agy -p`: none.
            for (surface, attached) in [(Surface::Chat, true), (Surface::Workflow, false)] {
                let (servers, caps) = server_map_for(
                    AgentKind::Antigravity,
                    "acceptEdits",
                    cwd,
                    root,
                    &parent(surface),
                    TasksAccess::ReadWrite,
                    None,
                );
                assert_eq!(
                    servers.contains_key("hollow_tasks") && caps.tasks,
                    attached,
                    "Antigravity {surface:?}"
                );
            }
            for surface in [
                Surface::Heartbeat,
                Surface::Oneshot,
                Surface::Delegate,
                Surface::Ingest,
            ] {
                let (servers, caps) = server_map_for(
                    AgentKind::Claude,
                    "acceptEdits",
                    cwd,
                    root,
                    &parent(surface),
                    TasksAccess::ReadWrite,
                    None,
                );
                assert!(servers.is_empty() && !caps.tasks, "{surface:?}");
            }

            // One mcpServers map holds the team and task servers side by side.
            let (mut servers, _) = server_map_for(
                AgentKind::Claude,
                "acceptEdits",
                cwd,
                root,
                &parent(Surface::Workflow),
                TasksAccess::ReadWrite,
                None,
            );
            servers.insert("hollow_team".into(), json!({ "command": "team" }));
            let merged: Value = serde_json::from_str(&merge(servers).unwrap()).unwrap();
            let map = merged["mcpServers"].as_object().unwrap();
            assert_eq!(map["hollow_team"]["command"], json!("team"));
            assert_eq!(map["hollow_tasks"]["args"], json!(["tasks-mcp"]));
        });
        // Switched off, or no broker listening: nothing attaches.
        for (on, sock) in [(false, Some(sock)), (true, None)] {
            crate::tasks_mcp::with_attach_override(on, sock, || {
                let (servers, caps) = server_map_for(
                    AgentKind::Claude,
                    "acceptEdits",
                    cwd,
                    root,
                    &parent(Surface::Workflow),
                    TasksAccess::ReadWrite,
                    None,
                );
                assert!(!servers.contains_key("hollow_tasks") && !caps.tasks);
            });
        }
        assert_eq!(merge(Map::new()), None);
        let mut one = Map::new();
        one.insert("hollow_questions".into(), json!({ "command": "x" }));
        assert_eq!(
            merge(one).unwrap(),
            r#"{"mcpServers":{"hollow_questions":{"command":"x"}}}"#
        );
    }

    /// Hollow app tools (PR A1) attach to Claude and Codex chats and
    /// workflows at the caller's access, never when the caller says `None`,
    /// never to Antigravity or the other surfaces, and only while the switch
    /// is on and the broker listens.
    #[test]
    fn app_tools_attach_at_the_callers_access() {
        let (cwd, root) = (Path::new("/tmp/work"), Path::new("/vault"));
        let sock = std::path::PathBuf::from("/tmp/hollow/app-7.sock");
        crate::app_mcp::with_attach_override(true, Some(sock.clone()), || {
            for agent in [AgentKind::Claude, AgentKind::Codex] {
                for surface in [Surface::Chat, Surface::Workflow] {
                    for (access, wire) in [(AppAccess::Write, "write"), (AppAccess::Read, "read")] {
                        let (servers, caps) = server_map_for(
                            agent,
                            "acceptEdits",
                            cwd,
                            root,
                            &parent(surface),
                            TasksAccess::ReadWrite,
                            Some(AppGrant::new(access, 0, "acceptEdits")),
                        );
                        let entry = servers
                            .get("hollow_app")
                            .unwrap_or_else(|| panic!("{agent:?} {surface:?}"));
                        assert_eq!(entry["args"], json!(["app-mcp"]));
                        assert_eq!(entry["env"]["HOLLOW_APP_ACCESS"], json!(wire));
                        assert_eq!(entry["env"]["HOLLOW_APP_DEPTH"], json!("0"));
                        assert_eq!(entry["env"]["HOLLOW_APP_PERM"], json!("acceptEdits"));
                        assert_eq!(
                            entry["env"]["HOLLOW_APP_SOCK"],
                            json!(sock.to_string_lossy())
                        );
                        assert!(caps.app);
                        assert_eq!(caps.app_read_only, access == AppAccess::Read);
                    }
                    let (servers, caps) = server_map_for(
                        agent,
                        "acceptEdits",
                        cwd,
                        root,
                        &parent(surface),
                        TasksAccess::ReadWrite,
                        None,
                    );
                    assert!(
                        !servers.contains_key("hollow_app") && !caps.app,
                        "None never attaches"
                    );
                }
            }
            let (servers, _) = server_map_for(
                AgentKind::Antigravity,
                "acceptEdits",
                cwd,
                root,
                &parent(Surface::Chat),
                TasksAccess::ReadWrite,
                Some(AppGrant::new(AppAccess::Write, 0, "acceptEdits")),
            );
            assert!(
                servers.contains_key("hollow_app"),
                "an Antigravity chat takes app tools over ACP"
            );
            for surface in [
                Surface::Heartbeat,
                Surface::Oneshot,
                Surface::Delegate,
                Surface::Ingest,
            ] {
                let (servers, caps) = server_map_for(
                    AgentKind::Claude,
                    "acceptEdits",
                    cwd,
                    root,
                    &parent(surface),
                    TasksAccess::ReadWrite,
                    Some(AppGrant::new(AppAccess::Write, 0, "acceptEdits")),
                );
                assert!(servers.is_empty() && !caps.app, "{surface:?}");
            }
            // Preapproved for Claude and Codex alike (the broker decides).
            let (servers, _) = server_map_for(
                AgentKind::Claude,
                "plan",
                cwd,
                root,
                &parent(Surface::Chat),
                TasksAccess::ReadOnly,
                Some(AppGrant::new(AppAccess::Read, 0, "plan")),
            );
            let config = merge(servers).unwrap();
            assert!(crate::agent::owned_preapprovals(Some(&config), "plan")
                .contains(&"mcp__hollow_app"));
        });
        for (on, sock) in [(false, Some(sock)), (true, None)] {
            crate::app_mcp::with_attach_override(on, sock, || {
                let (servers, caps) = server_map_for(
                    AgentKind::Claude,
                    "acceptEdits",
                    cwd,
                    root,
                    &parent(Surface::Chat),
                    TasksAccess::ReadWrite,
                    Some(AppGrant::new(AppAccess::Write, 0, "acceptEdits")),
                );
                assert!(!servers.contains_key("hollow_app") && !caps.app);
            });
        }
    }

    /// S03 (red first): the vault server is told whether the same chat has
    /// hollow_app (`HOLLOW_VAULT_APP`), so its lexical-only reply can name
    /// `run_action vault.retrieve`; only told, never given a channel.
    #[test]
    fn the_vault_entry_says_whether_the_chat_has_hollow_app() {
        let _toggles = SERVER_TOGGLES.lock().unwrap_or_else(|e| e.into_inner());
        crate::vault_mcp::set_vault_mcp_enabled(true);
        let (cwd, root) = (Path::new("/tmp/work"), Path::new("/vault"));
        let sock = std::path::PathBuf::from("/tmp/hollow/app-8.sock");
        let flag = |grant: Option<AppGrant>| {
            let (servers, caps) = server_map_for(
                AgentKind::Claude,
                "acceptEdits",
                cwd,
                root,
                &parent(Surface::Chat),
                TasksAccess::ReadWrite,
                grant,
            );
            let vault = servers
                .get("hollow_vault")
                .expect("chat attaches the vault");
            assert_eq!(vault["env"].get("HOLLOW_APP_SOCK"), None, "no app channel");
            (caps.app, vault["env"][crate::vault_mcp::ENV_APP].clone())
        };
        crate::app_mcp::with_attach_override(true, Some(sock.clone()), || {
            for access in [AppAccess::Write, AppAccess::Read] {
                assert_eq!(
                    flag(Some(AppGrant::new(access, 0, "acceptEdits"))),
                    (true, json!("1"))
                );
            }
            assert_eq!(flag(None), (false, json!("")));
        });
        // The switch off or no broker: no hollow_app, so no flag.
        for (on, sock) in [(false, Some(sock)), (true, None)] {
            crate::app_mcp::with_attach_override(on, sock, || {
                assert_eq!(
                    flag(Some(AppGrant::new(AppAccess::Write, 0, "acceptEdits"))),
                    (false, json!(""))
                );
            });
        }
    }

    /// CC2: Claude and Codex chats and workflows in a git checkout get
    /// `hollow_code_context` rooted at its top level; Grok and Antigravity
    /// never (no owned MCP: the CLI twin instead); the toggle off, a cwd
    /// in the vault or outside git attach nothing. Plan keeps it, bare
    /// removes it, unattended keeps it.
    #[test]
    fn code_context_attaches_for_owned_agents_in_a_checkout() {
        let _toggles = SERVER_TOGGLES.lock().unwrap_or_else(|e| e.into_inner());
        let repo = crate::code_context::tests::repo(&[("src/lib.rs", "fn a() {}\n")]);
        let sub = repo.path().join("src");
        let vault = tempfile::tempdir().unwrap();
        let attach = |agent, perm: &str, surface, cwd: &Path, vault: &Path| {
            server_map_for(
                agent,
                perm,
                cwd,
                vault,
                &parent(surface),
                TasksAccess::ReadOnly,
                None,
            )
        };
        crate::code_context_mcp::set_code_context_mcp_enabled(true);
        for agent in [AgentKind::Claude, AgentKind::Codex] {
            for surface in [Surface::Chat, Surface::Workflow] {
                for perm in ["acceptEdits", "plan"] {
                    let (servers, caps) = attach(agent, perm, surface, &sub, vault.path());
                    let entry = &servers[crate::code_context_mcp::SERVER_NAME];
                    assert_eq!(entry["args"], json!(["code-context-mcp"]));
                    let root = std::fs::canonicalize(repo.path()).unwrap();
                    assert_eq!(
                        entry["env"][crate::code_context::ENV_ROOT],
                        json!(root.to_string_lossy())
                    );
                    assert!(caps.code_context, "{agent:?} {perm}");
                    assert!(caps.server_names().contains(&"hollow_code_context"));
                    let (kept, kept_caps) = for_unattended(true, servers.clone(), caps);
                    assert!(kept.contains_key("hollow_code_context") && kept_caps.code_context);
                    let (bare, bare_caps) = for_bare(true, servers, caps);
                    assert!(bare.is_empty() && !bare_caps.code_context);
                }
            }
        }
        let (servers, caps) = attach(
            AgentKind::Grok,
            "acceptEdits",
            Surface::Chat,
            &sub,
            vault.path(),
        );
        assert!(!servers.contains_key("hollow_code_context") && !caps.code_context);
        // The cwd inside the vault, or outside any checkout.
        let (servers, _) = attach(
            AgentKind::Claude,
            "acceptEdits",
            Surface::Chat,
            &sub,
            repo.path(),
        );
        assert!(!servers.contains_key("hollow_code_context"));
        let (servers, _) = attach(
            AgentKind::Claude,
            "acceptEdits",
            Surface::Chat,
            vault.path(),
            Path::new("/nonexistent"),
        );
        assert!(!servers.contains_key("hollow_code_context"));
        crate::code_context_mcp::set_code_context_mcp_enabled(false);
        let (servers, caps) = attach(
            AgentKind::Codex,
            "acceptEdits",
            Surface::Chat,
            &sub,
            vault.path(),
        );
        crate::code_context_mcp::set_code_context_mcp_enabled(true);
        assert!(!servers.contains_key("hollow_code_context") && !caps.code_context);
    }
}
