use crate::agent::{self, AgentEvent, AgentKind};
use crate::automated_history::{BeginRunError, LeaseHolder, RunLease};
use crate::chat_resume::{
    Exhaustion, Failover, FailoverFacts, Held, Probation, ResetReason, ResumePlan,
};
use crate::core_host::Core;
use crate::interactions::{self, InteractionSource, NewInteraction};
use crate::mcp_attach::AttachedCaps;
use crate::prep_timing::{Mark, PrepMarks, Step};
use crate::process_group::{
    self, Ended, GroupGuard, StderrPolicy, StopSignal, Supervision, Timings,
};
use crate::run_queue::{self, CancelOutcome, ChatEnqueue, NewRow, QueueRow, RowStatus, RunSource};
use crate::usage::LedgerOutcome;
use crate::usage_ledger::{self, DelegateParent, Meter, Surface, UsageRun};
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use tokio::process::Command;

pub const EVENT: &str = "chat-event";

/// Bound the admin (`/mcp`, `/plugin`) one-shot so a hung `claude` can't
/// block the IPC call indefinitely.
/// AG4b: the file in a turn's scratch folder where the answers to its
/// deferred tool calls wait for the hook (`crate::claude_defer`).
const DEFER_ANSWERS_FILE: &str = "claude-defer-answers.json";
/// AG4b: what the resumed session is told once the card is answered.
const DEFER_PROMPT: &str = "The person has answered the approval for your last tool call. Go on.";
/// AG4b: the most cards one turn may ask before it ends on the last deferral.
const MAX_DEFERRALS: u32 = 24;
const ADMIN_TIMEOUT: Duration = Duration::from_secs(60);
/// Bound the slash-command discovery probe — it only needs the first
/// `system/init` event, so a stuck process shouldn't wedge the chat UI.
const DISCOVER_TIMEOUT: Duration = Duration::from_secs(30);

/// Managed-runtime tools stripped from the embedded chat spawn (see the
/// `disallowed_tools` note in the spawn). The managed `claude` build exposes
/// these for its own host client; in Hollow's headless chat they have no
/// channel, so the model calling one errors and the agent loop re-answers.
/// `AskUserQuestion` stays off for good: since Paperclip PR 13 the agent asks
/// through Hollow's own `hollow_questions` MCP (`ask_user_questions`), whose
/// completed tool event becomes a question card, and Antigravity through a
/// fenced `hollow-questions` block.
const HOST_ONLY_TOOLS: &[&str] = &["SendUserFile", "AskUserQuestion"];

/// Appended to quick and non-workspace chat prompts, after their brief:
/// files the agent writes are listed under its reply, and the person opens
/// one in Hollow's preview pane with a click (run 15 P1: it never opens by
/// itself). The visual-answer guidance itself is [`crate::visual_prompt`],
/// which every chat gets (it replaced the HTML preference prompt that used to
/// sit here, Canvas C3 wave 2 C4). An `.html` the agent wrote opens in Canvas
/// (scripts, no network) in the pane (10-11 B1, `canvas/canvasGrants.ts`
/// `fileMode`).
const PREVIEW_PANE_PROMPT: &str = "\
# The preview pane

Files you create or edit with Write/Edit are listed under your reply; the \
person opens one in Hollow's preview pane with a click (it never opens by \
itself): .html renders in the artifact sandbox (your scripts run; nothing \
remote loads unless the person allows it for that file), .md renders \
formatted, SVG/images display, CSV shows as a table and other text as \
source. The pane re-renders after every edit you make, so iterate on the file \
in place — don't write v2/v3 copies. If your rendered HTML throws runtime \
script errors, they are captured and the person can hand them back to you \
with one click; when that happens, fix the same file. After writing a \
substantial artifact, Read it back to verify it is complete and well-formed \
before declaring it done.";

/// Where meetings come from (P1-20): the agent half of the shared meetings
/// wording, `PLAUD_FOR_AGENTS` and `CALENDAR_SOURCE` in src/meetingsCopy.ts
/// (meetingsCopy.test.ts checks this line verbatim). Hollow's own Recordings
/// sign-in isn't the agents' access, so the brief names only the connector.
const MEETINGS_BRIEF: &str = "Plaud tools for agents come from the Plaud connector, with its own sign-in. Calendar comes from the source you connect in Settings → Connectors.";

fn downloads_folder() -> String {
    dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "~/Downloads".to_string())
}

/// Tell the embedded agent where the canonical vault and saved local files
/// live and exactly which Hollow capabilities are attached to this session.
/// The same text in every mode (HB01): the computer line points at the
/// turn's `<hollow-mode>` line. The workspace policy is not repeated here:
/// every adapter already appends it (`agent::chat_policy`).
fn environment_brief(vault: &std::path::Path, cwd: &std::path::Path, caps: AttachedCaps) -> String {
    // Setup-mode placeholder config has an empty vault path; fall back to the
    // cwd so the brief never claims a vault at "".
    let vault = if vault.as_os_str().is_empty() {
        cwd
    } else {
        vault
    };
    let vault_s = vault.display();
    let downloads = downloads_folder();
    let cwd_note = if cwd == vault {
        " (the vault — you are already inside it)"
    } else {
        " — NOT the vault; use the vault root above for vault files"
    };

    let mut s = String::with_capacity(3000);
    s.push_str("# Your environment (Hollow)\n\n");
    s.push_str(
        "You are the embedded agent inside Hollow, the user's work surface. \
Hollow (Projects, Tasks, Library, Automations and workspaces) keeps local vault files; \
tracked projects and tasks follow the configured tracker. Use local file tools directly. \
You already know these locations; never search the \
filesystem to find them. Needs you opens from the bell, then See all.\n\n",
    );
    s.push_str(&format!("- Vault root: {vault_s}\n"));
    s.push_str(&format!(
        "- Working directory: {}{cwd_note}\n",
        cwd.display()
    ));
    s.push_str(&format!("- The user's Downloads folder: {downloads}\n\n"));
    s.push_str(
        "Vault layout (README.md describes it; folders may be absent until first use): \
workspaces/<id>/ (one folder per outcome; tracked projects keep their \
PROJECT.md beside the workspace brief), tasks/ (one \
.md per task), goals/, me/ (operator profile), library/ (reference material), learnings/, \
workflows/, drafts/. ",
    );
    s.push_str(MEETINGS_BRIEF);
    s.push_str("\n\n");
    if caps.tasks && caps.tasks_read_only {
        // A plan chat reads tasks through the tools but changes nothing.
        s.push_str(
            "To see whether a task already exists, CHECK with list_tasks (hollow_tasks); this \
run can't create or change tasks, so describe the change you would make instead. For a \
project, CHECK its workspace's PROJECT.md. Hollow keys tasks by their title→slug, so a \
second file with the same title is a confusing duplicate (it lands as `name 2.md`).\n\n",
        );
    } else if caps.tasks {
        // Paperclip PR 12: with hollow_tasks attached, tasks go through its
        // tools (validated, conflict-checked, recorded in the run).
        s.push_str(
            "Before creating a task, CHECK with list_tasks (hollow_tasks) whether it already \
exists. For a project, edit its workspace's PROJECT.md. Hollow keys tasks by their title→slug, \
so a second file with the same title is a confusing duplicate (it lands as `name 2.md`).\n\n",
        );
    } else {
        s.push_str(
            "Before creating a task or project, CHECK whether one already exists (search/glob \
tasks/ or workspaces/*/PROJECT.md) and edit that file instead of writing a new one. Hollow \
keys tasks by their title→slug, so a second file with the same title is a confusing duplicate \
(it lands as `name 2.md`). When restructuring or moving status, update/relocate the \
existing note — do not leave a near-identical twin behind.\n\n",
        );
    }
    s.push_str(
        "Linear is the source of truth for tracked projects and tasks, configured Engineering team only. \
The tracker adapter owns sync. Never edit mirror identity or read-only fields or sync by hand. \
Hollow no longer syncs with Notion; notion-official is an ordinary connector. The vault holds notes, recordings and Calendar. \
Hollow is where the person reads: briefs, reports, decisions and requests appear here, even with a tracker copy.\n\n",
    );
    s.push_str(&format!(
        "For a saved file, check Downloads ({downloads}) and local vault copies in \
captures/, sources/, or imported/. Hollow has no built-in messaging or attachment fetching.\n\n",
    ));
    s.push_str(
        "Never run Hollow's binary without a known command (--version is safe): a bare run starts a second app.\n\n",
    );
    s.push_str("Capabilities attached to this session:\n");
    if caps.computer {
        s.push_str(&format!(
            "- hollow_computer — vetted desktop verbs: open_url / open_app / search_bookmarks \
instantly; {}.\n",
            crate::mode_contract::COMPUTER_POINTER
        ));
    }
    if caps.vault {
        s.push_str(
            "- hollow_vault — search_vault: keyword-search the user's whole vault (notes, \
meetings, people, projects, recordings) and get ranked matches with snippets. Reach for it to \
GROUND answers in the user's own notes before answering from general knowledge — it's faster \
than globbing. read_source: read numbered lines of a file \
Hollow attached or cited (an attached source, a retrieved note, a connected project file) when \
its excerpt isn't enough, instead of guessing. read_skill: read a skill the latest \
<hollow-skills> block lists, whole, before following it.\n",
        );
    }
    if caps.teamwork() {
        let mut teammates = Vec::new();
        if caps.team_claude {
            teammates.push("claude");
        }
        if caps.team_codex {
            teammates.push("codex");
        }
        if caps.team_antigravity {
            teammates.push("antigravity");
        }
        if caps.team_grok {
            teammates.push("grok");
        }
        s.push_str(&format!("- hollow_team — ask_agent: a bounded consultation (a quick question answered in the call; see \"Delegating\") with the agent argument set to one of: {}.\n", teammates.join(", ")));
    }
    if caps.tasks {
        s.push_str(if caps.tasks_read_only {
            "- hollow_tasks — list_tasks / get_task / next_claimable / list_projects / get_project: \
read Hollow's tasks and projects (read-only in this run; see \"Task tools\" below).\n"
        } else {
            "- hollow_tasks — list_tasks / get_task / next_claimable / create_task / update_task / \
set_status / claim_task / release_task / add_comment / propose_tasks / list_projects / get_project / \
create_project / update_project / set_project_status: read and change tasks and projects \
(see \"Task tools\" below).\n"
        });
    }
    if caps.app {
        s.push_str(
            "- hollow_app — list_workspaces and the app's other reads, plus list_actions / \
run_action for the rest of Hollow (see \"Hollow app tools\" below).\n",
        );
    }
    if caps.questions {
        s.push_str(
            "- hollow_questions — ask_user_questions: post a structured question card when you are \
blocked on a decision only the user can make (see \"Asking the user\" below).\n",
        );
    } else if caps.questions_fenced {
        s.push_str(
            "- Question cards: end your reply with one fenced hollow-questions block when you are \
blocked on a decision only the user can make (see \"Asking the user\" below).\n",
        );
    }
    s.push_str(
        "\nUse the vault proactively for tasks, projects and notes. Ground changes in \
current evidence and preserve local links and custom metadata.",
    );
    s
}

/// The heading of the capability index ([`capability_index`]).
pub(crate) const INDEX_HEADING: &str = "# Hollow capabilities in this run";

/// The capability index's build-slot line (OF07), for a turn that can take
/// a slot (`build_slots::can_take_slot`) and has `hollow-slot` on its PATH.
/// Kept to 32 bytes: the simple-text and total budgets (`g1_budget_tests`,
/// `g2_budget_tests`) had no room for the first 120-byte wording, and
/// Codex's simple-text turn now sits 1 byte under its bar. The tool itself
/// names the `--dev-server` form on every build call (and in its usage), so
/// a chat outside a build workspace, whose note says it too, still learns it.
const SLOT_INDEX_LINE: &str = "- Big jobs: `hollow-slot <cmd>`\n";

/// The compact capability index (agent effectiveness F6, wave 3 · G1): one
/// line per capability this run actually has (the owned servers by the
/// names the server map holds, `AttachedCaps::server_names`; the answer
/// forms for the Canvases choice; question cards; library connectors when
/// the agent can have any; skills; scratch) and where the details are, then
/// the agent's line of the per-provider contract (F9, wave 4 · V1:
/// `capability_contract::guidance_line`, what it doesn't have here and the
/// fallback). About 0.7–1.5 KB. Every chat's appended prompt carries it after "Visual
/// answers in Hollow"; it replaced the workspace and quick-chat briefs'
/// `computer=…, teamwork=…` list, which left out vault search and app
/// tools. Fixed text per input, like the brief: only the attached
/// capabilities, plan (against every other mode) and the Canvases choice
/// change it, and fingerprint v3 hashes all three (OB04).
fn capability_index(
    caps: AttachedCaps,
    canvases: crate::visual_prompt::CanvasFrequency,
    perm: &str,
) -> String {
    use crate::visual_prompt::CanvasFrequency;
    let mut s = String::with_capacity(1_400);
    s.push_str(INDEX_HEADING);
    s.push_str("\n\nWhat this run has, and where each one's details are. Nothing else of Hollow's is attached.\n\n");
    let guide = match canvases {
        CanvasFrequency::Most => "\"Visual answers in Hollow\" above".to_string(),
        _ => format!(
            "the visual guide ({}; added to a turn that looks visual)",
            crate::visual_prompt::GUIDE_PATH
        ),
    };
    s.push_str(&match canvases {
        CanvasFrequency::Off => {
            "- Answer forms: Markdown only (Canvases are off in this chat).\n".to_string()
        }
        _ => format!(
            "- Answer forms: Markdown, canvases (`hollow-canvas`, kit@1; `hc-flow` for diagrams) and ask-canvases. Formats: {guide}.\n"
        ),
    });
    if caps.questions {
        s.push_str(
            "- Question cards: hollow_questions (ask_user_questions); see \"Asking the user\".\n",
        );
    } else if caps.questions_fenced {
        s.push_str("- Question cards: a fenced hollow-questions block at the end of a reply; see \"Asking the user\".\n");
    }
    if caps.vault {
        s.push_str("- Vault search: hollow_vault (search_vault ranks the vault's notes; read_source reads numbered lines of an attached or cited file).\n");
    }
    if caps.code_context {
        s.push_str("- Code context: hollow_code_context (map, symbols, callers, callees, impact, read of this checkout; every name match is listed, resolved:true marks the ones a rule ties to the definition; slice gives a symbol's cards and, with its handle, only what changed; changes and affected_tests list a diff's symbols and the tests worth running first, never instead of the full gate).\n");
    }
    if caps.app {
        s.push_str(if caps.app_read_only {
            "- Hollow app (read-only): hollow_app; list_actions and describe_action find the rest of Hollow; see \"Hollow app tools\".\n"
        } else {
            "- Hollow app: hollow_app; list_actions and describe_action find the rest of Hollow, run_action runs one; start_session hands off a durable child task; see \"Hollow app tools\" and \"Delegating\".\n"
        });
    }
    if caps.tasks {
        s.push_str(if caps.tasks_read_only {
            "- Tasks (read-only): hollow_tasks; see \"Task tools\".\n"
        } else {
            "- Tasks: hollow_tasks; see \"Task tools\".\n"
        });
    }
    if caps.computer {
        s.push_str("- Computer: hollow_computer (open apps, URLs and bookmarks; each tool says what it asks).\n");
    }
    if caps.teamwork() {
        s.push_str("- Teammates: hollow_team (ask_agent: a bounded consultation with another installed agent, answered in the call); see \"Delegating\".\n");
    }
    if caps.services {
        s.push_str("- Services: hollow_services (start and stop this worktree's dev servers); see its guidance below.\n");
    }
    // Every turn that can take a slot has `hollow-slot` first on PATH and
    // the toolchain caps (OF07, `build_slots::chat_env`); plan mode and
    // Antigravity's sandboxed modes run no commands. One call takes the
    // slot, runs the command and frees it, so a slot is never held across
    // commands or past the Bash tool's timeout. `execute_turn` drops the
    // line again when the tool couldn't be written.
    let slots = match caps.agent {
        Some(agent) => crate::build_slots::can_take_slot(agent, perm),
        None => perm != "plan",
    };
    if slots {
        s.push_str(SLOT_INDEX_LINE);
    }
    if !caps.questions_fenced && perm != "plan" {
        s.push_str("- Library connectors: a turn's are listed in its <hollow-connectors> block; a turn without one has none.\n");
    }
    // Skills (F9, wave 4 · V2): the relevant ones are listed per turn, in
    // its message, never here (this text is fixed per input, and Claude
    // caches it).
    s.push_str(if caps.vault {
        "- Skills: the latest <hollow-skills> block lists the person's skills that fit the request (a selected one first, with its text when small); hollow_vault read_skill reads one whole by name.\n"
    } else if caps.app {
        "- Skills: the latest <hollow-skills> block lists the person's skills that fit the request (a selected one first, with its text when small); hollow_app run_action skills.read_skill reads one whole by its path.\n"
    } else {
        "- Skills: the latest <hollow-skills> block lists the person's skills that fit the request (a selected one first, with its text when small); open one at its path with your own file tools (if your mode refuses a file outside the working folder, ask the person to paste it).\n"
    });
    s.push_str("- Scratch folder: $HOLLOW_SCRATCH_DIR; see \"Scratch space\".");
    // What this agent doesn't have here and the fallback, from the
    // per-provider contract (F9, wave 4 · V1).
    if let Some(line) = caps
        .agent
        .map(crate::capability_contract::guidance_line)
        .filter(|line| !line.is_empty())
    {
        s.push('\n');
        s.push_str(&line);
    }
    s
}

/// The chat turn's appended system prompt (Claude) or prompt preamble (other
/// adapters): "Visual answers in Hollow" for the turn's Canvases choice
/// (every chat and provider; the formats themselves are the visual guide,
/// which a visual turn's message carries, F6 G1), the capability index,
/// then the workspace brief or the environment brief (with the
/// preview-pane note outside workspaces), then the
/// byte-stable guidance consts in a FIXED order (question cards first, then
/// PR 12's task tools, then the screen-jobs paragraph, then PR 15a's scratch
/// space, which is always present).
/// Everything here must stay deterministic for the same inputs (Claude
/// caches it): the scratch guidance names only `$HOLLOW_SCRATCH_DIR`, never
/// the turn's folder. Fingerprint v3 (OB04) hashes what selects this text
/// (caps, Canvases choice, [`PromptKind`]) and `chat_resume::PROMPT_CONTRACT`,
/// not the text: a different Canvases choice starts the native session
/// fresh once, a wording change doesn't.
#[cfg(test)]
fn chat_append_prompt(
    is_workspace: bool,
    vault: &Path,
    cwd: &Path,
    caps: AttachedCaps,
    canvases: crate::visual_prompt::CanvasFrequency,
    perm: &str,
) -> String {
    chat_append_layers(is_workspace, vault, cwd, caps, canvases, perm).joined()
}

/// COO rebuild L10: a subagent's "Visual answers" layer. Its reply goes to
/// the conversation that started it, never to the person.
pub(crate) const LEAN_VISUAL: &str = "Your reply goes to the conversation that started you, not to the person: answer in plain Markdown, with no canvases and no suggested next message.";

/// L10: a subagent's capability index, in place of the full one (its tools'
/// own schemas say what each does).
pub(crate) const LEAN_INDEX: &str = "# Hollow capabilities in this run\n\nThe tools attached to this run are all of Hollow's you have; each tool's schema says what it does. You work for the conversation that started you: you can't ask the person questions, so decide, note what you decided, and say it in your reply.";

/// Whether conversation `key` is a subagent (L10's lean profile): started
/// by another conversation (`parent` on its start, or `fork_kind`).
pub(crate) fn is_subagent_turn(vault: &Path, key: &str, parent: Option<&str>) -> bool {
    parent.is_some_and(|p| !p.trim().is_empty())
        || crate::session_store::session_by_key(vault, key)
            .ok()
            .flatten()
            .and_then(|s| s.fork_kind)
            .as_deref()
            == Some(crate::session_store::FORK_SUBAGENT)
}

/// L10: the lean subagent profile. A subagent doesn't talk to the person, so
/// its appended prompt drops the visual guide, the full capability index,
/// the question-card guidance and the suggested-next-message block; its
/// brief (the workspace or environment one) and the rest of the policy stay.
/// The COO and the leads keep the full set.
fn lean_subagent(mut layers: AppendLayers, caps: AttachedCaps) -> AppendLayers {
    layers.visual = LEAN_VISUAL.to_string();
    if !layers.index.is_empty() {
        layers.index = LEAN_INDEX.to_string();
    }
    if let Some(guidance) =
        crate::interaction_schema::guidance(caps.questions, caps.questions_fenced)
    {
        layers.policy = layers.policy.replacen(&format!("\n\n{guidance}"), "", 1);
    }
    layers.policy = layers
        .policy
        .replacen(&format!("\n\n{}", crate::next_step::GUIDANCE), "", 1);
    layers
}

/// The chat's appended system prompt in its four layers, for the run's
/// context manifest (agent effectiveness F11): "Visual answers in Hollow",
/// the capability index (F6, G1), the environment brief (or the workspace
/// or quick-chat one), and the policy guidance after it. [`AppendLayers::joined`] is exactly
/// [`chat_append_prompt`].
struct AppendLayers {
    visual: String,
    /// The capability index (F6, G1).
    index: String,
    brief: String,
    /// Starts with its own blank line.
    policy: String,
}

impl AppendLayers {
    fn joined(&self) -> String {
        format!(
            "{}\n\n{}\n\n{}{}",
            self.visual, self.index, self.brief, self.policy
        )
    }
}

/// The brief of a bare scratch turn (a pressure-test persona, a bake-off
/// variant; overnight S7): no vault root, layout or tools to point at.
const BARE_BRIEF: &str = "You are running one self-contained, read-only task that Hollow set up. Everything you need is in the message. You have no Hollow tools and no access to the person's vault or other files: don't search for or read anything outside your own folder, and don't ask the person anything. Answer in your reply.";

/// Which brief a chat's appended prompt carries ([`chat_append_layers`]).
/// Fingerprint v3 hashes this kind, never the prompt's text (OB04).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptKind {
    /// A bare scratch turn (overnight S7).
    Bare,
    /// The Main agent's workspace.
    Main,
    /// The quick-chats workspace.
    Quick,
    /// An outcome workspace.
    Workspace,
    /// A vault chat: the environment brief.
    Vault,
}

impl PromptKind {
    fn of(is_workspace: bool, vault: &Path, cwd: &Path) -> PromptKind {
        if crate::mcp_attach::bare_scratch(vault, cwd) {
            return PromptKind::Bare;
        }
        if !is_workspace {
            return PromptKind::Vault;
        }
        let builtin = crate::workspaces::workspace_for_cwd(vault, cwd)
            .ok()
            .flatten()
            .map(|w| w.id);
        if builtin
            .as_deref()
            .is_some_and(crate::workspaces::is_main_agent)
        {
            PromptKind::Main
        } else if builtin
            .as_deref()
            .is_some_and(crate::workspaces::is_quick_chats)
        {
            PromptKind::Quick
        } else {
            PromptKind::Workspace
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            PromptKind::Bare => "bare",
            PromptKind::Main => "main",
            PromptKind::Quick => "quick",
            PromptKind::Workspace => "workspace",
            PromptKind::Vault => "vault",
        }
    }
}

/// The workspace brief's `log/<chat-key>.md` with this chat's own key (V5b),
/// so the instruction resolves whether or not the shared note is sent. A key
/// that isn't a well-formed session key leaves the placeholder.
fn with_chat_key(brief: &str, session_key: &str) -> String {
    match crate::shared_docs::chat_log_path(session_key) {
        Some(path) => brief.replace("log/<chat-key>.md", &path),
        None => brief.to_string(),
    }
}

/// SK1: `brief` with the repo's `<hollow-repo-trust>` label after it, when
/// the chat's checkout isn't the person's (`repo_trust`).
fn with_repo_trust(brief: &str, trust: Option<&crate::repo_trust::RepoTrust>) -> String {
    match trust {
        Some(trust) => format!("{brief}\n\n{}", trust.label()),
        None => brief.to_string(),
    }
}

/// SK1: the `Skill(<name>)` rules for the repo skills still waiting for the
/// person's OK and the file-tool denies on `repo-trust.json`, on an agent
/// that holds deny rules; none on the others (the label asks them in words).
fn repo_trust_denied(
    trust: Option<&crate::repo_trust::RepoTrust>,
    agent: AgentKind,
) -> Vec<String> {
    let Some(trust) = trust else {
        return Vec::new();
    };
    // The record of allowed skills and owners is denied to Claude's file
    // tools too (review of SK1).
    trust
        .denied_tools()
        .into_iter()
        .chain(crate::repo_trust::store_denies())
        .filter(|rule| crate::agent::holds_deny(agent, rule))
        .collect()
}

fn chat_append_layers(
    is_workspace: bool,
    vault: &Path,
    cwd: &Path,
    caps: AttachedCaps,
    canvases: crate::visual_prompt::CanvasFrequency,
    perm: &str,
) -> AppendLayers {
    let kind = PromptKind::of(is_workspace, vault, cwd);
    // A bare scratch turn (overnight S7) gets only its brief: no capability
    // index, provider contract, skills line, question or canvas guidance,
    // each of which points at tools, files or the person it doesn't have.
    if kind == PromptKind::Bare {
        return AppendLayers {
            visual: "Answer in plain Markdown.".into(),
            index: String::new(),
            brief: BARE_BRIEF.to_string(),
            policy: String::new(),
        };
    }
    let quick = kind == PromptKind::Quick;
    let main = kind == PromptKind::Main;
    let brief = if main {
        format!(
            "{}\n\n{}",
            main_agent_prompt(vault, cwd),
            PREVIEW_PANE_PROMPT
        )
    } else if quick {
        format!(
            "{}\n\n{}",
            quick_chat_prompt(vault, cwd),
            PREVIEW_PANE_PROMPT
        )
    } else if is_workspace {
        format!("You are working in a Hollow outcome workspace at {}. Workspace files define the work. Log in log/<chat-key>.md; WORKSPACE.md/PLAN.md only for workspace-wide changes. Keep sources as is, deliverables in outputs/. Do not search unrelated vault folders or import global project decisions unless requested. The host capabilities attached to this run are listed under \"Hollow capabilities in this run\"; use only those.", cwd.display())
    } else {
        format!(
            "{}\n\n{}",
            environment_brief(vault, cwd, caps),
            PREVIEW_PANE_PROMPT
        )
    };
    // Visual answers (canvases) work with every chat provider and
    // every workspace type. Fixed text per Canvases choice, which the resume
    // fingerprint hashes (never this text, OB04); the formats (the visual guide) go
    // in a visual turn's message instead, below Most (F6, G1).
    // The ask-canvas guide depends on how this chat can ask (question
    // cards, a fenced block, or neither; Canvas C4 A3).
    let asking = crate::visual_prompt::Asking::of(caps.questions, caps.questions_fenced);
    let visual = crate::visual_prompt::visual_prompt(canvases, asking);
    let index = capability_index(caps, canvases, perm);
    let mut prompt = String::new();
    if let Some(guidance) =
        crate::interaction_schema::guidance(caps.questions, caps.questions_fenced)
    {
        prompt.push_str("\n\n");
        prompt.push_str(guidance);
    }
    // Task tools (Paperclip PR 12), after the question guidance.
    if caps.tasks {
        prompt.push_str("\n\n");
        prompt.push_str(if caps.tasks_read_only {
            crate::tasks_mcp::PROMPT_GUIDANCE_READ_ONLY
        } else {
            crate::tasks_mcp::PROMPT_GUIDANCE
        });
    }
    // Hollow app tools (PR A1), after the task tools; fixed text per
    // access, the same in every mode (HB01: its card sentences are the
    // turn's `<hollow-mode>` line).
    if caps.app {
        prompt.push_str("\n\n");
        prompt.push_str(&crate::app_mcp::chat_prompt_guidance(caps.app_read_only));
    }
    // The two delegation contracts (F10, D1), after the app tools: fixed
    // text per (teammates attached, app tools that can start sessions).
    if let Some(guidance) =
        crate::delegation::guidance(caps.teamwork(), caps.app && !caps.app_read_only)
    {
        prompt.push_str("\n\n");
        prompt.push_str(&guidance);
    }
    // Services (Build PR B6), only when attached (build worktrees): the
    // text is constant, so it changes the fingerprint only of those chats.
    if caps.services {
        prompt.push_str("\n\n");
        prompt.push_str(crate::services_mcp::PROMPT_GUIDANCE);
    }
    // The suggested next message (2026-10-01), every chat, just before the
    // scratch guidance; constant text.
    prompt.push_str("\n\n");
    prompt.push_str(crate::next_step::GUIDANCE);
    // Background work ends with the turn (overnight A3): agents ended turns
    // waiting on a background gate with a ScheduleWakeup that never came.
    prompt.push_str("\n\n");
    prompt.push_str(BACKGROUND_GUIDANCE);
    // Quote globs (S16, evening 10-16), every chat; constant text.
    prompt.push(' ');
    prompt.push_str(SHELL_GUIDANCE);
    // Edit with the edit tool, wait inside one command (OB16), every chat;
    // constant text.
    prompt.push(' ');
    prompt.push_str(EDIT_GUIDANCE);
    // The jobs the cut screens did (overnight 11-08 S2), every chat except
    // a bare scratch turn (that returned above). Constant text: fingerprint
    // v3 hashes PromptKind, not this wording, so it does not start a fresh
    // native session.
    prompt.push_str("\n\n");
    prompt.push_str(SCREEN_JOBS);
    // Scratch space (Paperclip PR 15a), last and unconditional: every chat
    // spawn sets `HOLLOW_SCRATCH_DIR` (a folder, or the never-existing
    // sentinel when none could be made).
    prompt.push_str("\n\n");
    prompt.push_str(crate::scratch::SCRATCH_GUIDANCE);
    AppendLayers {
        visual,
        index,
        brief,
        policy: prompt,
    }
}

/// What a chat turn's context is made of, for [`record_turn_layers`].
struct TurnLayers<'a> {
    append: &'a AppendLayers,
    /// Codex, Antigravity or Grok got the one-line reference instead of
    /// the appended prompt (G2).
    configured_held: bool,
    workspace: Option<&'a str>,
    recall: &'a str,
    /// The turn's `<hollow-clock>` line (overnight A2).
    clock: &'a str,
    /// The turn's `<hollow-context>` line (repo features R2b); empty for
    /// none.
    context_note: &'a str,
    /// The turn's `<hollow-mode>` line (HB01).
    mode: &'a str,
    connectors_note: &'a str,
    /// The `<hollow-skills>` index (F9, wave 4 · V2); empty for none.
    skills: &'a str,
    evidence: &'a str,
    /// The turn's wrapper around an empty body: workspace, recall, clock,
    /// context note, connectors note, skill index and evidence, as
    /// `execute_turn`'s `wrap` joins them.
    wrapped: &'a str,
}

/// Record a chat turn's context layers on its manifest (agent effectiveness
/// F11), in the order the agent reads them: the appended system prompt's
/// three layers, the wrapper's, then the composed (or rebuilt) body's,
/// taken from `first_prompt` after the wrapper. Sizes and hashes only.
fn record_turn_layers(
    manifest: &mut crate::run_manifest::RunManifest,
    parts: &TurnLayers<'_>,
    first_prompt: &str,
) {
    if parts.configured_held {
        manifest.layer("configuredHeld", crate::agent::CONFIGURED_HELD);
    } else {
        manifest.layer("visualPrompt", &parts.append.visual);
        manifest.layer("capabilityIndex", &parts.append.index);
        manifest.layer("environmentBrief", &parts.append.brief);
        manifest.layer("policy", &parts.append.policy);
    }
    if let Some(context) = parts.workspace {
        manifest.layer("workspace", context);
    }
    manifest.layer("recall", parts.recall);
    manifest.layer("clock", parts.clock);
    manifest.layer("contextNote", parts.context_note);
    manifest.layer("permissionMode", parts.mode);
    manifest.layer("connectorsNote", parts.connectors_note);
    manifest.layer("skillIndex", parts.skills);
    manifest.layer("retrievalEvidence", parts.evidence);
    if let Some(body) = first_prompt.strip_prefix(parts.wrapped) {
        manifest
            .layers
            .extend(crate::run_manifest::composer_layers(body));
    }
}

/// One chat turn's visual guide (agent effectiveness F6, wave 3 · G1):
/// the guide for the chat's Canvases choice and question channel, whether
/// the request looked visual, and whether the guide file was written.
struct TurnGuide {
    canvases: crate::visual_prompt::CanvasFrequency,
    cue: Option<&'static str>,
    file: bool,
    guide: String,
    sha: String,
}

impl TurnGuide {
    /// Writes the guide into `scratch` (the turn's folder) when there is one.
    fn new(
        canvases: crate::visual_prompt::CanvasFrequency,
        asking: crate::visual_prompt::Asking,
        cue: Option<&'static str>,
        scratch: Option<&Path>,
    ) -> Self {
        let guide = crate::visual_prompt::visual_guide(canvases, asking);
        // Canvases off: no guide, so no file to name or read.
        let file = !guide.is_empty() && crate::visual_prompt::write_guide_file(scratch, &guide);
        let sha = crate::run_manifest::sha256_hex(guide.as_bytes());
        TurnGuide {
            canvases,
            cue,
            file,
            guide,
            sha,
        }
    }

    /// How the guide would reach the native `session` of conversation
    /// `key`, without recording anything.
    fn delivery(
        &self,
        vault: &Path,
        key: &str,
        session: crate::context_compiler::Session,
    ) -> crate::visual_prompt::Delivery {
        let fresh = session.fresh();
        let given = if fresh {
            None
        } else {
            crate::visual_prompt::given(vault, key)
        };
        crate::visual_prompt::delivery(
            self.canvases,
            self.cue,
            self.file,
            fresh,
            given.as_deref(),
            &self.sha,
        )
    }

    /// The guide block's size when this turn would carry it, else 0.
    fn block_bytes(
        &self,
        vault: &Path,
        key: &str,
        session: crate::context_compiler::Session,
    ) -> usize {
        if self.delivery(vault, key, session) == crate::visual_prompt::Delivery::Turn {
            crate::visual_prompt::guide_block(&self.guide).len()
        } else {
            0
        }
    }

    /// Decide for the native `session` of conversation `key`, and put the
    /// guide block (when it goes) in `compiled`'s body just ahead of the
    /// caller's text (`composed` past a leading task packet, which the body
    /// always ends with; the packet stays first, F10 D1), with the decision
    /// on its manifest record. What the session then holds is returned, not
    /// recorded: the caller records it ([`GuideGiven::record`]) only once
    /// the agent spawned, so a turn that stops before spawn or fails to
    /// spawn never counts the guide as given (wave 3 review of G1). When
    /// the turn's budget had no room for it (`allowed` false, G2) a block
    /// that would have gone stays in the file, as `onDemand`.
    fn apply(
        &self,
        vault: &Path,
        key: &str,
        session: crate::context_compiler::Session,
        mut compiled: crate::context_compiler::Compiled,
        composed: &str,
        allowed: bool,
    ) -> (crate::context_compiler::Compiled, Option<GuideGiven>) {
        use crate::visual_prompt::Delivery;
        let fresh = session.fresh();
        let delivery = match self.delivery(vault, key, session) {
            Delivery::Turn if !allowed && self.file => Delivery::OnDemand,
            delivery => delivery,
        };
        let given = match delivery {
            Delivery::Turn => Some(GuideGiven(self.sha.clone())),
            _ if fresh => Some(GuideGiven(String::new())),
            _ => None,
        };
        if delivery == Delivery::Turn {
            let block = crate::visual_prompt::guide_block(&self.guide);
            // The compiler moves a durable child task's packet to the front
            // (F10, D1), so the body ends with what follows the packet.
            let composed = crate::delegation::split_leading(composed).1;
            let at = compiled
                .body
                .len()
                .checked_sub(composed.len())
                .filter(|&at| {
                    compiled.body.ends_with(composed) && compiled.body.is_char_boundary(at)
                })
                .unwrap_or(0);
            compiled.body.insert_str(at, &block);
        }
        compiled.record.visual_guide = Some(crate::run_manifest::VisualGuideRecord {
            delivery: delivery.as_str().to_string(),
            cue: self.cue.map(str::to_string),
            file: self.file,
            bytes: self.guide.len() as u64,
            sha256: self.sha.clone(),
        });
        (compiled, given)
    }
}

/// What a composed turn's native session holds of the visual guide once
/// the agent took the prompt ([`TurnGuide::apply`]): the guide's SHA-256
/// when the turn carried it, empty for a fresh session that wasn't given
/// it. Recorded only after a spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GuideGiven(String);

impl GuideGiven {
    /// Record it for conversation `key`'s turn `ord`.
    fn record(&self, vault: &Path, key: &str, ord: i64) {
        crate::visual_prompt::record_given(vault, key, ord, &self.0);
    }
}

/// A chat turn's relevant skill index (agent effectiveness F9, wave 4 ·
/// V2), from the person's own words (`asked`), telling the agent to read a
/// skill whole the way this run can: `hollow_vault` `read_skill`, else
/// `hollow_app` `skills.read_skill`, else its own file tools.
fn turn_skills(
    roots: &crate::skill_index::Roots,
    asked: &str,
    caps: &AttachedCaps,
) -> crate::skill_index::TurnSkills {
    crate::skill_index::for_turn(
        roots,
        asked,
        crate::skill_index::Reader::of(caps.vault, caps.app),
    )
}

/// Everything one chat turn's prompt is built from (agent effectiveness F6,
/// wave 3 · G2): the configured half, the conversation and turn the
/// context compiler reads, the visual guide, and the turn's wrapper
/// (workspace envelope, approved recall, the connectors note and the
/// retrieved evidence). `compose` is called once for the plan's session and
/// again, fresh, for a failed resume's retry.
struct TurnComposer<'a> {
    vault: &'a Path,
    session_key: &'a str,
    continued_from: &'a [String],
    /// Hollow queued the turn itself (`ChatSendRequest::host_asker`, OB19):
    /// what it continues from gives task state, never a bundle.
    host: bool,
    turn_ord: i64,
    entry: crate::context_compiler::Entry,
    operator_enabled: bool,
    reader: crate::task_state::Reader,
    /// The caller's composed text.
    composed: &'a str,
    guide: &'a TurnGuide,
    agent: AgentKind,
    /// The appended prompt (`AppendLayers::joined`) and the workspace
    /// policy: the configured half.
    append: &'a str,
    policy: &'a str,
    /// The `<hollow-workspace>` envelope, in full.
    workspace: Option<&'a crate::workspaces::WorkspaceContext>,
    /// The envelope rebuilt within a byte limit (the turn's budget).
    rebuild: &'a (dyn Fn(usize) -> Option<crate::workspaces::WorkspaceContext> + Sync),
    /// The native session a resumed turn resumes.
    native: Option<&'a str>,
    recall: &'a str,
    connectors_note: &'a str,
    /// The turn's `<hollow-skills>` index (F9, wave 4 · V2); empty for none.
    skills: &'a str,
    evidence: &'a str,
    /// The turn's `<hollow-clock>` line ([`clock_line`]); empty for none.
    clock: &'a str,
    /// The turn's `<hollow-context>` line ([`turn_context_note`], repo
    /// features R2b); empty for none. Only a resumed session's prompt
    /// carries it.
    context_note: &'a str,
    /// The turn's `<hollow-mode>` line (`mode_contract::turn_line`, HB01);
    /// empty for none.
    mode: &'a str,
}

/// The `<hollow-clock>` line a turn's message carries (after the workspace
/// envelope and recall, before the connectors note): the local time the turn
/// started, with its UTC offset (overnight A2).
pub(crate) fn clock_line<Tz: chrono::TimeZone>(now: &chrono::DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    format!(
        "<hollow-clock>Local time at the start of this turn: {}. Use it for any time you write; never guess one.</hollow-clock>\n\n",
        now.format("%a %Y-%m-%d %H:%M (UTC%:z)")
    )
}

/// Repo features R2b: the `<hollow-context>` line a turn's message carries
/// after its clock, on `plan`. A resumed session that isn't `quiet` (plan
/// mode, bare, unattended: [`gauge_quiet`]) is told once when the figure
/// its last turn stored passed a growth step the conversation wasn't told
/// (`context_gauge::start_note`, which stores the step before the gauge
/// registers, so the in-turn note doesn't repeat it). A fresh native
/// session (a new or reset plan) starts from nothing: its conversation's
/// stored figure is cleared. Off the always-on path: empty below 300K and
/// between steps.
fn turn_context_note(vault: &Path, key: &str, plan: &ResumePlan, quiet: bool) -> String {
    match plan {
        ResumePlan::Resume(_) if quiet => String::new(),
        ResumePlan::Resume(_) => crate::context_gauge::start_note(vault, key).unwrap_or_default(),
        ResumePlan::New | ResumePlan::Reset(_) => {
            crate::context_gauge::reset_steps(vault, key);
            String::new()
        }
    }
}

#[cfg(test)]
mod shell_guidance_tests {
    /// S16: every chat's appended prompt tells the agent to quote globs,
    /// right after the background line; a bare scratch turn has no shell.
    #[test]
    fn every_chat_is_told_to_quote_globs() {
        let vault = std::path::Path::new("/Users/x/vault");
        let prompt = super::chat_append_prompt(
            false,
            vault,
            vault,
            super::AttachedCaps::default(),
            crate::visual_prompt::CanvasFrequency::Helps,
            "bypassPermissions",
        );
        assert!(prompt.contains(&format!(
            "{} {}",
            super::BACKGROUND_GUIDANCE,
            super::SHELL_GUIDANCE
        )));
        assert!(super::SHELL_GUIDANCE.contains("--include='*.ts'"));
    }

    /// OB16: every chat, in every mode and for every provider, is told to
    /// change files with its edit tool even where its CLI allows scripts
    /// (Claude Code's bypass reminder does), and to wait inside one
    /// command, right after the glob line.
    #[test]
    fn every_chat_is_told_to_edit_with_its_edit_tool_and_wait_in_one_command() {
        let vault = std::path::Path::new("/Users/x/vault");
        let tail = format!(
            "{} {} {}",
            super::BACKGROUND_GUIDANCE,
            super::SHELL_GUIDANCE,
            super::EDIT_GUIDANCE
        );
        for agent in crate::capability_contract::PROVIDERS {
            let owned = crate::agent::capabilities(agent).owned_mcp;
            let caps = super::AttachedCaps {
                questions: owned,
                questions_fenced: !owned,
                tasks: owned,
                app: owned,
                vault: owned,
                agent: Some(agent),
                ..Default::default()
            };
            for perm in crate::app_mcp::PERMISSION_MODES {
                for (is_workspace, cwd) in [
                    (false, vault.to_path_buf()),
                    (true, vault.join("workspaces/w1")),
                ] {
                    let prompt = super::chat_append_prompt(
                        is_workspace,
                        vault,
                        &cwd,
                        caps,
                        crate::visual_prompt::CanvasFrequency::Helps,
                        perm,
                    );
                    assert_eq!(prompt.matches(&tail).count(), 1, "{agent:?} {perm} {cwd:?}");
                }
            }
        }
        let text = super::EDIT_GUIDANCE;
        for phrase in [
            "edit tool",
            "apply_patch",
            "heredocs",
            "every replacement",
            "until <check>",
            "one tool call per check",
        ] {
            assert!(text.contains(phrase), "{phrase}");
        }
        assert!(text.len() <= 320, "{} bytes", text.len());
    }

    /// Overnight 11-08 S2: the jobs the cut screens did are one paragraph
    /// after the edit line and before scratch, in every chat. A bare
    /// scratch turn (a pressure-test persona) does not get it.
    #[test]
    fn every_chat_is_told_the_jobs_the_cut_screens_did() {
        let vault = std::path::Path::new("/Users/x/vault");
        let prompt = super::chat_append_prompt(
            false,
            vault,
            vault,
            super::AttachedCaps::default(),
            crate::visual_prompt::CanvasFrequency::Helps,
            "bypassPermissions",
        );
        let tail = format!(
            "{} {} {}\n\n{}",
            super::BACKGROUND_GUIDANCE,
            super::SHELL_GUIDANCE,
            super::EDIT_GUIDANCE,
            super::SCREEN_JOBS
        );
        assert_eq!(prompt.matches(&tail).count(), 1);
        assert_eq!(prompt.matches(super::SCREEN_JOBS).count(), 1);
        for phrase in [
            "Plan row",
            "approval card",
            "ledger",
            "team AI",
            "tracker (shown as Linear)",
            "External action",
            "does not create that issue",
            "library/work/people",
            "library/records/people",
            "personas/<role>.md",
            "archiving a workspace",
            "overnight work",
            "Automations stay",
            "due today or overdue",
            "build.install_brings",
        ] {
            assert!(super::SCREEN_JOBS.contains(phrase), "{phrase}");
        }
        assert!(!super::SCREEN_JOBS.contains("hollow_tasks"));
        assert!(!super::SCREEN_JOBS.contains('\n'));
        assert!(!super::SCREEN_JOBS.contains('\u{2014}'));
        assert!(!super::SCREEN_JOBS.contains('\u{2013}'));
        assert!(
            super::SCREEN_JOBS.len() + 2 <= super::SCREEN_JOBS_ALLOWANCE,
            "{} bytes",
            super::SCREEN_JOBS.len()
        );

        let stem = crate::session_store::vault_file_stem(vault);
        let root = crate::test_temp::process_dir("hollow-test-pressure-").join(stem);
        let persona = root.join("pt-role");
        let bare = super::chat_append_prompt(
            false,
            vault,
            &persona,
            super::AttachedCaps::default(),
            crate::visual_prompt::CanvasFrequency::Helps,
            "plan",
        );
        assert!(bare.contains(super::BARE_BRIEF), "not a bare scratch turn");
        assert!(
            !bare.contains(super::SCREEN_JOBS),
            "a pressure-test persona must not get the screen jobs"
        );
    }

    /// Overnight S16 ride-along: the hollow_tasks roster line names every
    /// tool `tasks_mcp` serves at that access, so a new tool (S09's claims)
    /// can't ship without the agent being told about it.
    #[test]
    fn the_tasks_roster_lists_every_served_tool() {
        use crate::tasks_mcp::{tool_specs, TasksAccess};
        let vault = std::path::Path::new("/Users/x/vault");
        for (read_only, access) in [
            (false, TasksAccess::ReadWrite),
            (true, TasksAccess::ReadOnly),
        ] {
            let caps = super::AttachedCaps {
                tasks: true,
                tasks_read_only: read_only,
                ..Default::default()
            };
            let brief = super::environment_brief(vault, vault, caps);
            let line = brief
                .lines()
                .find(|l| l.starts_with("- hollow_tasks — "))
                .expect("a hollow_tasks roster line");
            let (names, _) = line["- hollow_tasks — ".len()..].split_once(':').unwrap();
            let mut listed: Vec<&str> = names.split(" / ").map(str::trim).collect();
            let specs = tool_specs(access);
            let mut served: Vec<&str> = specs.iter().map(|s| s["name"].as_str().unwrap()).collect();
            listed.sort_unstable();
            served.sort_unstable();
            assert_eq!(listed, served, "{access:?} roster: {line}");
        }
    }
}

#[cfg(test)]
mod clock_tests {
    use chrono::TimeZone;

    /// Overnight A2: every turn's message says when it started, with the
    /// offset, so an agent never has to guess a time.
    #[test]
    fn the_clock_line_names_the_local_time_and_offset() {
        let edt = chrono::FixedOffset::west_opt(4 * 3600).unwrap();
        let at = edt.with_ymd_and_hms(2026, 10, 9, 19, 16, 29).unwrap();
        assert_eq!(
            super::clock_line(&at),
            "<hollow-clock>Local time at the start of this turn: Fri 2026-10-09 19:16 (UTC-04:00). Use it for any time you write; never guess one.</hollow-clock>\n\n"
        );
    }
}

/// The appended prompt's line on shell globs (S16, evening 10-16), constant
/// text after [`BACKGROUND_GUIDANCE`]. zsh (the shell here) aborts a whole
/// command line on a glob that matches nothing ("no matches found:
/// --include=*.ts"): about 31 Bash calls failed that way. Quoting is
/// harmless in bash, so the line is conditional, not provider-specific.
pub(crate) const SHELL_GUIDANCE: &str = "In zsh, an unquoted glob that matches nothing aborts the command (\"no matches found\"): quote globs, as in --include='*.ts' or -name '*.md'.";

/// The appended prompt's line on editing and waiting (OB16, overnight
/// 10-17), constant text after [`SHELL_GUIDANCE`]. Agents edited code with
/// inline Python scripts that read, `.replace()` and rewrite a file (3,743
/// of them against 332 Edit calls in a week, 696 checking nothing), which
/// Claude Code's own bypass-mode reminder allows ("sed, heredocs, or short
/// scripts"), so the line overrides it by name. They also polled with one
/// tool call per check; a bare `sleep` may be refused by the CLI's Bash
/// rules, which point to an until-loop.
pub(crate) const EDIT_GUIDANCE: &str = "In Hollow, change files with your edit tool (Edit; apply_patch on Codex), even where your CLI allows sed, heredocs or scripts; script only a many-file rename, checking every replacement. To wait, loop in one command (`until <check>; do sleep 30; done`, within the tool's timeout), not one tool call per check.";

/// The jobs the cut screens did (overnight 11-08 S2, overhaul C1), constant
/// text after [`EDIT_GUIDANCE`] and before scratch guidance. Every chat gets
/// it, including a read-only run, so the job is not lost when the screen
/// goes. A bare scratch turn does not: it returns before this is appended.
/// There is no Herald reader and no "Send to Linear" control; a check-in
/// reads the due list itself, and an automation records a local task.
pub(crate) const SCREEN_JOBS: &str = "When this run can write, add a Plan row or a task if the work calls for it, and ask with an approval card when it is a real choice. A commitment stays in the ledger. A clear one becomes a team AI issue on the tracker (shown as Linear) only as an External action: a card unless this chat is in bypass. An automation records a local task and does not create that issue. Update people notes in library/work/people and library/records/people. When asked to pressure-test a draft, read personas/<role>.md. The COO suggests archiving a workspace in chat, and takes one-off and overnight work (Automations stay). On a check-in, read tasks due today or overdue. What an install brings still comes from build.install_brings.";

/// How many bytes [`SCREEN_JOBS`] may add to the appended prompt, including
/// the blank line before it. The F6 check that a simple turn stays 10 KB
/// under the old baseline spends this and nothing more.
pub(crate) const SCREEN_JOBS_ALLOWANCE: usize = 800;

/// How many bytes [`crate::delegation::DEPENDENCY_RULE`] adds to a chat that
/// can start sessions (its bullet and newline in "Delegating", Alex
/// 11-10). The F6 and G2 budget checks spend this beside
/// [`SCREEN_JOBS_ALLOWANCE`] and nothing more.
#[cfg(test)]
pub(crate) const DEPENDENCY_RULE_ALLOWANCE: usize = crate::delegation::DEPENDENCY_RULE.len() + 3;

// In the library itself, not only the tests. Clippy checks the lib without
// `cfg(test)`, and this is the cap those tests spend.
const _: () = assert!(SCREEN_JOBS.len() + 2 <= SCREEN_JOBS_ALLOWANCE);

/// The appended prompt's line on background work (overnight A3), constant
/// text. A Hollow chat turn is one CLI process: what it leaves running stops
/// with it, and a CLI wakeup tool has no process left to wake; Hollow's
/// `sessions.wake_me` (overnight C3) does resume one, where it is offered.
/// How to wait in the foreground is [`EDIT_GUIDANCE`]'s (OB16).
pub(crate) const BACKGROUND_GUIDANCE: &str = "Background work ends with your turn: a background shell or agent still running is stopped, and ScheduleWakeup or CronCreate never resume a Hollow chat (sessions.wake_me can, where offered). Run long checks in the foreground.";

/// One composed prompt and what was decided for it.
struct ComposedTurn {
    prompt: String,
    record: crate::run_manifest::CompilerRecord,
    /// PS1: told to the person when a fresh session got no earlier turns.
    notice: Option<String>,
    /// The workspace envelope as this prompt carries it.
    workspace: Option<String>,
    /// Codex, Antigravity and Grok get the one-line reference instead of
    /// the appended prompt (`RunExtras::configured_held`).
    configured_held: bool,
    /// What the native session holds once this prompt reaches it; written
    /// with the announced native id after the agent ran.
    pending: crate::stable_layers::Baseline,
    /// The visual guide the native session holds once this prompt reaches
    /// it; recorded only after the agent spawned.
    guide_given: Option<GuideGiven>,
    /// The attached sources' record when the budget rebuilt the envelope.
    attached_sources: Option<crate::run_manifest::AttachedSourcesRecord>,
    /// The per-turn layers this prompt left out (S02).
    left_out: LeftOut,
}

/// The per-turn layers a held resumed prompt left out because its native
/// session was given each one as is (S02), in
/// [`crate::stable_layers::TURN_LAYERS`]'s order: recall, the chat intent,
/// the skill index and the mode line. The second field says an empty skill
/// index goes as [`crate::skill_index::NONE_FIT`] (overnight A15).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct LeftOut([bool; 4], bool);

impl LeftOut {
    const RECALL: usize = 0;
    const INTENT: usize = 1;
    const SKILLS: usize = 2;
    const MODE: usize = 3;

    /// Layer `at`'s `text` as the prompt carries it: nothing when left out,
    /// the empty skill block when no skill fits but the session holds
    /// earlier picks.
    fn sent(self, at: usize, text: &str) -> &str {
        if self.0[at] {
            ""
        } else if at == Self::SKILLS && self.1 && text.is_empty() {
            crate::skill_index::NONE_FIT
        } else {
            text
        }
    }

    fn any(self) -> bool {
        self.0.contains(&true)
    }

    /// The names of the layers left out, for the manifest.
    fn names(self) -> Vec<String> {
        crate::stable_layers::TURN_LAYERS
            .iter()
            .zip(self.0)
            .filter(|(_, out)| *out)
            .map(|(name, _)| name.to_string())
            .collect()
    }
}

impl TurnComposer<'_> {
    /// The turn's wrapper around `body`: the workspace envelope, then
    /// approved recall, then the clock, the context note (R2b), the mode
    /// line, the connectors note, the skill index and the evidence, less
    /// what `left_out` names (S02).
    fn wrap(&self, left_out: LeftOut, workspace: Option<&str>, body: &str) -> String {
        let recall = left_out.sent(LeftOut::RECALL, self.recall);
        let prompt = format!(
            "{}{}{}{}{}{}{body}",
            self.clock,
            self.context_note,
            left_out.sent(LeftOut::MODE, self.mode),
            self.connectors_note,
            left_out.sent(LeftOut::SKILLS, self.skills),
            self.evidence
        );
        let prompt = if recall.is_empty() {
            prompt
        } else {
            format!("{recall}\n\n{prompt}")
        };
        match workspace.filter(|w| !w.is_empty()) {
            Some(context) => format!("{context}{prompt}"),
            None => prompt,
        }
    }

    fn compose(&self, session: crate::context_compiler::Session) -> ComposedTurn {
        use crate::context_budget::{Ask, Channel, Tier};
        use crate::run_manifest::StableLayersRecord;
        let budget = crate::context_budget::for_agent(self.agent);
        let in_prompt = budget.configured == Channel::Prompt;
        let append_sha = crate::run_manifest::sha256_hex(self.append.as_bytes());
        // What an exactly resumed native session already holds (G2).
        let resumed = session == crate::context_compiler::Session::Resume;
        let given = if resumed {
            crate::stable_layers::read(self.vault, self.session_key).map(|(_, b)| b)
        } else {
            None
        };
        let held = crate::stable_layers::holds(given.as_ref(), self.native);
        // Whether this resumed turn carries the task-state record again
        // (R1): after a compaction the stream reported, or on the refresh
        // of an agent whose compaction Hollow can't see.
        let reground =
            crate::stable_layers::reground(given.as_ref(), self.native, &held, self.agent);
        let mut reason = match &held {
            Err(why) if resumed => Some(why.to_string()),
            _ => None,
        };
        let configured_held = in_prompt
            && matches!(&held, Ok(b) if resumed && b.configured == append_sha && !self.append.is_empty());
        let configured_bytes = if !in_prompt {
            self.append.len() + 2 + self.policy.len()
        } else if configured_held {
            self.policy.len() + 2 + crate::agent::CONFIGURED_HELD.len() + 2
        } else if self.append.is_empty() {
            self.policy.len() + 2
        } else {
            self.policy.len() + 2 + self.append.len() + 2
        };
        let full = self.workspace.map(|w| w.text.as_str());
        let parts = self.workspace.and_then(|w| w.parts.as_ref());
        let mut delta = match &held {
            Ok(given) if resumed => {
                let delta =
                    full.and_then(|f| crate::stable_layers::envelope_delta(f, parts, given));
                if delta.is_none() && full.is_some() {
                    reason = Some("changed".into());
                }
                delta
            }
            _ => None,
        };
        let sent_len = delta
            .as_ref()
            .map_or(full.map_or(0, str::len), |d| d.text.len());
        // The per-turn layers (S02): a held resumed turn leaves out each one
        // its session was given as is (recall, the composer's chat intent,
        // the skill index, the mode line). A changed one is sent; a new,
        // reset or retried session, another native id and the refresh get
        // them all.
        let intent = crate::sessions::leading_block(self.composed, "hollow-chat-intent");
        let intent_text = match &intent {
            Some(at) => self.composed[at.clone()].trim_end(),
            None => "",
        };
        // No skill fits this turn, but the resumed session holds an earlier
        // turn's picks (overnight A15): the latest block applies, so an
        // empty one says none do. Sent once, then held like any layer; a
        // new session never gets it.
        let none_fit = resumed
            && self.skills.is_empty()
            && crate::stable_layers::holds_skills(given.as_ref(), self.native);
        let skills_text = if none_fit {
            crate::skill_index::NONE_FIT
        } else {
            self.skills
        };
        let turn_layers = [self.recall, intent_text, skills_text, self.mode];
        let mut left_out = match &held {
            Ok(given) if resumed => LeftOut(
                std::array::from_fn(|at| {
                    let name = crate::stable_layers::TURN_LAYERS[at];
                    crate::stable_layers::layer_given(given, name, turn_layers[at])
                }),
                false,
            ),
            _ => LeftOut::default(),
        };
        left_out.1 = none_fit;
        // The caller's text, less a chat intent left out.
        let composed = match intent.filter(|_| left_out.0[LeftOut::INTENT]) {
            Some(at) => {
                let (before, after) = (&self.composed[..at.start], &self.composed[at.end..]);
                format!("{before}{after}")
            }
            None => self.composed.to_string(),
        };
        let recall = left_out.sent(LeftOut::RECALL, self.recall);
        let mode = left_out.sent(LeftOut::MODE, self.mode);
        let skills = left_out.sent(LeftOut::SKILLS, self.skills);
        let mut outside = vec![
            Ask::whole("configured", Tier::Fixed, configured_bytes),
            Ask::cuttable("workspace", Tier::Evidence, sent_len),
        ];
        if !recall.is_empty() {
            outside.push(Ask::whole("recall", Tier::Reserved, recall.len() + 2));
        }
        // The turn's clock line (overnight A2): one short line, never cut.
        if !self.clock.is_empty() {
            outside.push(Ask::whole("clock", Tier::Fixed, self.clock.len()));
        }
        // The turn's context note (R2b): a resumed session's only (a retry
        // or a fresh session has a new, small context), never cut.
        let context_note = if resumed { self.context_note } else { "" };
        if !context_note.is_empty() {
            outside.push(Ask::whole("contextNote", Tier::Fixed, context_note.len()));
        }
        // The turn's mode line (HB01): what the mode allows, never cut.
        if !mode.is_empty() {
            outside.push(Ask::whole("permissionMode", Tier::Fixed, mode.len()));
        }
        outside.push(Ask::whole(
            "connectorsNote",
            Tier::Fixed,
            self.connectors_note.len(),
        ));
        // Bounded by the index's own caps (five lines, 8 KB of selected
        // text), and what the person asked for: never cut.
        if !skills.is_empty() {
            outside.push(Ask::whole("skillIndex", Tier::Fixed, skills.len()));
        }
        outside.push(Ask::whole(
            "retrievalEvidence",
            Tier::Evidence,
            self.evidence.len(),
        ));
        let turn_budget = crate::context_compiler::TurnBudget {
            agent: self.agent,
            budget,
            outside,
            guide: self
                .guide
                .block_bytes(self.vault, self.session_key, session),
            guide_required: !self.guide.file,
        };
        let (compiled, allocation) = crate::context_compiler::compile_turn_within(
            self.vault,
            self.session_key,
            self.continued_from,
            self.host,
            self.turn_ord,
            self.entry,
            session,
            self.operator_enabled,
            self.reader,
            &composed,
            Some(&turn_budget),
            reground,
        );
        let allocation = allocation.unwrap_or_default();
        // An envelope over its grant is rebuilt in full within it.
        let mut rebuilt = None;
        if allocation.cut("workspace") {
            rebuilt = (self.rebuild)(allocation.granted("workspace").unwrap_or(0));
            delta = None;
            if resumed {
                reason = Some("budget".into());
            }
        }
        let (sent_full, sent_parts) = match &rebuilt {
            Some(w) => (Some(w.text.as_str()), w.parts.as_ref()),
            None => (full, parts),
        };
        let (mut compiled, guide_given) = self.guide.apply(
            self.vault,
            self.session_key,
            session,
            compiled,
            &composed,
            !allocation.cut("visualGuide"),
        );
        let workspace = match &delta {
            Some(d) => (!d.text.is_empty()).then(|| d.text.clone()),
            None => sent_full.map(str::to_string),
        };
        let mut pending = match (&held, &delta) {
            (Ok(given), Some(_)) => crate::stable_layers::advanced(given, full, parts),
            _ => crate::stable_layers::full_baseline("", "", sent_full, sent_parts),
        };
        // A per-turn layer left out counts as a resumed turn of deltas, so
        // the refresh still comes every REFRESH_TURNS turns (S02).
        if let (Ok(given), None) = (&held, &delta) {
            if configured_held || left_out.any() {
                pending.resumed_since_full = given.resumed_since_full + 1;
            }
        }
        pending.configured = if in_prompt { append_sha } else { String::new() };
        // What the session holds of the per-turn layers once this prompt
        // reaches it: each one as it is now, sent or left out as given
        // (S02). Recorded with the rest, only once the agent took it.
        let turn_layers: Vec<(&str, &str)> = crate::stable_layers::TURN_LAYERS
            .into_iter()
            .zip(turn_layers)
            .collect();
        crate::stable_layers::record_layers(&mut pending, &turn_layers);
        // A prompt that leaned on what the resumed session holds (a delta,
        // the held configured half, a per-turn layer left out) is held only
        // by that same session: it names the id it was composed against (G2
        // review). A full one leaves it empty, so whichever id the agent
        // announces holds it.
        if let Ok(given) = &held {
            if resumed && (delta.is_some() || configured_held || left_out.any()) {
                pending.native = given.native.clone();
            }
        }
        let (unchanged, changed, referenced) = delta
            .as_ref()
            .map_or((0, 0, 0), |d| (d.unchanged, d.changed, d.referenced));
        compiled.record.stable_layers = Some(StableLayersRecord {
            configured: if !in_prompt {
                "systemPrompt"
            } else if configured_held {
                "held"
            } else {
                "full"
            }
            .to_string(),
            workspace: if full.is_none() {
                "none"
            } else if delta.is_some() {
                "delta"
            } else if self.workspace.is_some_and(|w| w.slim) {
                "child-slim"
            } else {
                "full"
            }
            .to_string(),
            reason,
            full_bytes: full.map_or(0, str::len) as u64,
            sent_bytes: workspace.as_deref().map_or(0, str::len) as u64,
            unchanged,
            changed,
            excerpts_referenced: referenced,
            resumed_since_full: pending.resumed_since_full,
            held_layers: left_out.names(),
        });
        let wrapper = TurnComposer {
            context_note,
            ..*self
        };
        ComposedTurn {
            prompt: wrapper.wrap(left_out, workspace.as_deref(), &compiled.body),
            record: compiled.record,
            notice: compiled.notice,
            workspace,
            configured_held,
            pending,
            guide_given,
            attached_sources: rebuilt.and_then(|w| w.sources),
            left_out,
        }
    }
}

/// The system prompt of a quick chat (the Quick chats workspace,
/// 2026-09-30): no project, so answer directly and keep no workspace files.
fn quick_chat_prompt(vault: &Path, cwd: &Path) -> String {
    format!(
        "You are in a Hollow chat in Chats (id quick-chats): a standalone conversation with no project, brief or plan behind it. Answer the user's request directly. Do not create or maintain WORKSPACE.md, PLAN.md, SOURCES.md or an outputs/ folder, and do not write files, unless the user asks you to. Your working folder is {}; the user's vault is at {}, so read it only when the request needs it. If the conversation turns into ongoing work, suggest moving it into a workspace (right-click the conversation, then Move to… or New workspace from this chat). The host capabilities attached to this run are listed under \"Hollow capabilities in this run\"; use only those.",
        cwd.display(),
        vault.display(),
    )
}

/// The system prompt of the COO's conversations (main agent M1, 2026-10-07;
/// renamed and given its delegate-by-default role in L6, 11-10): the
/// always-on agent behind the top bar's orb. It reads across the vault and
/// delegates real work to a project's lead or a subagent, instead of doing
/// that work in its own turn. Its tools are not narrowed for it. Its standing orders and
/// notepad arrive in the workspace envelope (`workspaces::MAIN_CONTEXT_DOCS`).
/// With a runner COO paired (M3a), it is the dispatcher: the worker-batch
/// bullet gives way to `dispatcher_coo::note`, nothing else changes.
fn main_agent_prompt(vault: &Path, cwd: &Path) -> String {
    main_agent_prompt_as(vault, cwd, crate::dispatcher_coo::active().as_ref())
}

/// [`main_agent_prompt`] as the single COO (`None`) or the dispatcher to
/// `runner`.
fn main_agent_prompt_as(
    vault: &Path,
    cwd: &Path,
    runner: Option<&crate::dispatcher_coo::Runner>,
) -> String {
    let dispatch = runner.map(|r| crate::dispatcher_coo::note(&r.name));
    format!(
        "You are Hollow's COO, from the orb: the person's first stop for prompts, conversations and questions. Track projects, recommend steps and coordinate requested work in its workspace. In {cwd}: WORKSPACE.md keeps a short cross-day notepad for work/decisions; PLAN.md tracks work (remove finished rows); STANDING_ORDERS.md contains the person's orders (follow them; edit only when asked); outputs/ holds summaries. For vault {vault}, use permitted reads/tools: hollow_app list_workspaces/read_workspace for briefs/plans, list_sessions/read_session for chats, hollow_tasks and hollow_vault search.\n\nHow you work:\n- You delegate by default (a habit, not a tool limit: you may do anything the person can in Hollow). Answer directly what a few reads answer (status, where things are, what a chat found, Hollow settings) and run any Hollow action yourself. A project's building, research, planning or deliverables always go to its lead (send_to_session to read_workspace's projectAgentKey) to run and report; else, a subagent (start_session in \"quick-chats\"), split by real dependencies with independent parts started at once; then end your turn and report when the work returns. {follow_up} Overnight work: a project's goes to its lead; a multi-project night gets an orchestrator that is your subagent, its lanes that orchestrator's subagents.\n- Work in its workspace: hollow_app start_session (send_to_session for an existing chat), with a task packet, then end your turn instead of waiting in it: Hollow wakes you once when a chat you started ends (not one the person stopped; one that hands over, when its last successor ends); a child's child's ending doesn't wake you but arrives in your next turn's note. Use wait_for_session only for worthwhile short waits; read_session to check. For a later look at an unfinished chat, call run_action sessions.wake_me {{minutes, reason}} (1-120) and end your turn; a turn that starts with \"⟲ Hollow:\" can't, and the next check-in or ending brings you back. Never loop on waits or sleeps: background shells, ScheduleWakeup and CronCreate never bring you back. Don't edit another workspace's files from here; ongoing work without a home gets create_workspace.\n- For work status, check list_sessions, read_session and waiting-on-you items; report finished, running, stuck and needs-the-person.\n- Reply briefly: what matters/what you did, then next step. Don't list started chats in PLAN.md: list_sessions mine has them.\n- Never accept a Result or approve a learning or proposal (only the person does), and confirm before anything that sends, publishes, deletes or spends outside this Mac.\n- {workers}\n- Record each suggestion answer (Do it / Not now / Never) with run_action sessions.suggestion_outcome; Never also needs learning.submit_learning_proposal for the lasting preference. Follow their choice.\n- A message that starts with \"⟲ Hollow:\" is Hollow's check-in or child-completion report, not the person. Nobody is watching: act only within standing orders and Hollow's unattended limits.\n- {schedule}\n\nUse only the host capabilities under \"Hollow capabilities in this run\".",
        cwd = cwd.display(),
        vault = vault.display(),
        schedule = crate::runner_coo::schedule_note(),
        workers = dispatch
            .as_deref()
            .unwrap_or(crate::project_agents::WORKER_GUIDANCE),
        follow_up = crate::fresh_brief::FOLLOW_UP_RULE,
    )
}

/// The Main agent prompt's byte budget (overnight E7): it rides every Main
/// agent turn, so growth is a deliberate choice, not drift.
#[cfg(test)]
const MAIN_AGENT_PROMPT_BUDGET: usize = 4_600;

#[cfg(test)]
mod main_agent_prompt_tests {
    use super::*;

    #[test]
    fn the_main_agent_ends_its_turn_and_asks_to_be_woken_instead_of_waiting() {
        let p = main_agent_prompt(Path::new("/vault"), Path::new("/vault/workspaces/main"));
        assert!(
            p.len() <= MAIN_AGENT_PROMPT_BUDGET,
            "the COO prompt is {} bytes, over its {} budget",
            p.len(),
            MAIN_AGENT_PROMPT_BUDGET
        );
        // M3b: the app's prompt carries the app's schedule line, and a
        // `hollow serve` COO's prompt (its own line instead) fits too.
        assert!(p.contains(crate::main_agent::SCHEDULE_NOTE));
        for role in [
            crate::runner_coo::Role::Runner,
            crate::runner_coo::Role::Off,
        ] {
            let served = p.replace(
                crate::main_agent::SCHEDULE_NOTE,
                crate::runner_coo::schedule_note_for(role),
            );
            assert!(
                served.len() <= MAIN_AGENT_PROMPT_BUDGET,
                "{role:?}: {}",
                served.len()
            );
        }
        assert!(p.contains("coordinate requested work in its workspace"));
        // L6: the COO delegates by default, and the prompt says it is a
        // default, not a narrowing of its tools.
        assert!(p.starts_with("You are Hollow's COO"));
        assert!(p.contains("first stop for prompts, conversations and questions"));
        assert!(p.contains("You delegate by default (a habit, not a tool limit"));
        assert!(p.contains("run any Hollow action yourself"));
        // 11-14 card 2: project work from the COO is always its lead's; the
        // lead runs and reports it.
        assert!(p.contains("A project's building, research, planning or deliverables always go to its lead (send_to_session"));
        assert!(
            p.contains("to run and report; else, a subagent (start_session in \"quick-chats\")")
        );
        assert!(p.contains("then end your turn and report when the work returns"));
        // K3: small follow-ups go fresh with a brief, and edits are batched.
        assert!(p.contains(crate::fresh_brief::FOLLOW_UP_RULE));
        // The dependency rule itself comes once, in the Delegating block
        // every COO turn with writable app tools gets; the role points at it.
        assert!(p.contains("split by real dependencies with independent parts started at once"));
        assert!(!p.contains(crate::delegation::DEPENDENCY_RULE));
        assert!(p.contains(
            "an orchestrator that is your subagent, its lanes that orchestrator's subagents"
        ));
        assert!(!p.to_lowercase().contains("main agent"));
        // Follow started chats by ending the turn and being woken, not by a wait loop.
        assert!(p.contains("then end your turn instead of waiting in it"));
        assert!(p.contains("Hollow wakes you once when a chat you started ends"));
        assert!(p.contains("one that hands over, when its last successor ends"));
        assert!(p.contains("doesn't wake you but arrives in your next turn's note"));
        assert!(p.contains("run_action sessions.wake_me {minutes, reason} (1-120)"));
        assert!(p.contains("a turn that starts with \"⟲ Hollow:\" can't"));
        assert!(p.contains("Never loop on waits or sleeps"));
        assert!(!p.contains("then wait_for_session or read_session to follow it"));
        // T3a: Main's chats table in PLAN.md is no longer written.
        assert!(p.contains("Don't list started chats in PLAN.md: list_sessions mine has them."));
        assert!(!p.contains("Log every started chat"));
        assert!(p.contains(crate::project_agents::WORKER_GUIDANCE));
        assert!(
            !p.contains("company."),
            "the removed company is out of the COO prompt"
        );
        assert!(p.contains("'Where are my projects?', status questions and ordinary conversation need a direct answer, no workers"));
        // What it names exists: the action, and its 1-120 range.
        let wake =
            crate::app_actions::find("sessions.wake_me").expect("sessions.wake_me is an action");
        assert_eq!(wake.id, "sessions.wake_me");
        // And unattended turns really can't call it (so the prompt's "can't" is true).
        let refused = crate::main_agent::unattended::allows(
            Path::new("/vault"),
            "sessions.wake_me",
            false,
            &serde_json::Map::new(),
            true,
        );
        assert!(refused.is_err(), "an unattended turn can't schedule a wake");
    }

    /// M3a: with no runner COO paired, the COO's prompt is the single COO's
    /// byte for byte; as the dispatcher only the worker bullet differs, and
    /// it stays within the budget.
    #[test]
    fn the_dispatcher_swaps_only_the_worker_bullet_and_no_runner_changes_nothing() {
        let (vault, cwd) = (Path::new("/vault"), Path::new("/vault/workspaces/main"));
        let single = main_agent_prompt(vault, cwd);
        assert_eq!(single, main_agent_prompt_as(vault, cwd, None));
        assert!(single.contains(crate::project_agents::WORKER_GUIDANCE));
        assert!(!single.contains("dispatcher"));
        let runner = crate::dispatcher_coo::Runner {
            machine_id: "m-8d21c4be90f17a3e".into(),
            name: "M".repeat(40),
        };
        let dispatcher = main_agent_prompt_as(vault, cwd, Some(&runner));
        let note = crate::dispatcher_coo::note(&runner.name);
        assert_eq!(
            dispatcher,
            single.replace(crate::project_agents::WORKER_GUIDANCE, &note)
        );
        assert!(note.len() <= crate::project_agents::WORKER_GUIDANCE.len());
        assert!(dispatcher.len() <= MAIN_AGENT_PROMPT_BUDGET);
        assert!(dispatcher.contains("run_action sessions.route_to_runner {request, workspace?}"));
        assert!(crate::app_actions::find(crate::dispatcher_coo::ACTION).is_some());
    }

    /// M3a PR2: `agents.cooDispatcher` `off` with a runner COO paired keeps
    /// the COO's prompt the single COO's byte for byte; `auto` swaps the
    /// bullet.
    #[test]
    fn the_dispatcher_setting_off_keeps_the_single_coo_prompt_with_a_runner_paired() {
        use crate::dispatcher_coo::tests::{record, PAIRED, SETTING_OFF};
        let (vault, cwd) = (Path::new("/vault"), Path::new("/vault/workspaces/main"));
        let single = main_agent_prompt_as(vault, cwd, None);
        let runner = crate::peer_machines::MachineRecord {
            coo_seen_at: Some(chrono::Utc::now().to_rfc3339()),
            ..record("m-8d21c4be90f17a3e", "Mac mini", Some("s-1"))
        };
        PAIRED.with(|p| *p.borrow_mut() = vec![runner]);
        assert_ne!(
            main_agent_prompt(vault, cwd),
            single,
            "auto: the dispatcher"
        );
        SETTING_OFF.with(|s| s.set(true));
        let off = main_agent_prompt(vault, cwd);
        SETTING_OFF.with(|s| s.set(false));
        PAIRED.with(|p| p.borrow_mut().clear());
        assert_eq!(off.as_bytes(), single.as_bytes());
    }
}

/// Stop signal of the chat turn in progress, keyed by its Hollow session key
/// (Paperclip PR 7b), so the stop button reaches exactly that conversation's
/// turn and a stale click can never stop another session's. Registered in
/// [`admit_turn`] right after the turn takes the vault lease, so a stop during
/// the rest of preparation (retrieval, which now runs inside the lease, and
/// Codex's connector probe) is honored before spawn. A turn still waiting in
/// the run queue has no registration: [`cancel`] takes it out of the queue
/// instead. Released once the turn's processes have been ended; the turn owns
/// the child itself through `process_group::drive`.
///
/// One slot per conversation since build programme PR B4 (build workspaces'
/// turns run in parallel).
static CURRENT_STOP: Lazy<Mutex<HashMap<String, StopSlot>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// One conversation's registered turn: its stop signal and, for a turn the
/// pump admitted, its queue row, so a stop aimed at that row never reaches
/// a later turn that took the slot ([`cancel_running_row`]; repo ideas
/// 10-27 #8, review of #1065).
struct StopSlot {
    signal: Arc<StopSignal>,
    row: Option<i64>,
    /// Review fix (P1b): a person's own attended top-level chat, the only
    /// kind Pause all asks (set by `execute_turn` once it knows; false until
    /// then, so a turn not yet classified is never asked).
    own_chat: Arc<std::sync::atomic::AtomicBool>,
}

/// Tests that register a stop, admit a chat turn or touch the chat-row gate
/// hold this (then `agent_sessions::LIVE_TEST_LOCK`, never the reverse):
/// [`register_stop`] stops any previously registered signal, and cargo runs
/// tests on parallel threads of one process.
#[cfg(test)]
pub(crate) static TURN_TEST_LOCK: Mutex<()> = Mutex::new(());

/// Holds a turn's slot in [`CURRENT_STOP`]; dropping it clears the slot only
/// if it still holds this turn's signal.
pub(crate) struct StopRegistration {
    key: String,
    signal: Arc<StopSignal>,
    own_chat: Arc<std::sync::atomic::AtomicBool>,
}

impl StopRegistration {
    fn signal(&self) -> Arc<StopSignal> {
        Arc::clone(&self.signal)
    }

    /// Review fix (P1b): whether Pause all may ask this turn: a person's own
    /// chat (depth 0, attended, not bare, not a coordinator's), the same
    /// exclusions as a limit pause, since a resume runs as one.
    pub(crate) fn set_own_chat(&self, own: bool) {
        self.own_chat
            .store(own, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Drop for StopRegistration {
    fn drop(&mut self) {
        let ended = {
            let mut slots = CURRENT_STOP.lock().unwrap_or_else(PoisonError::into_inner);
            let mine = slots
                .get(&self.key)
                .is_some_and(|current| Arc::ptr_eq(&current.signal, &self.signal));
            if mine {
                slots.remove(&self.key);
                // P1a: a pause asked of this turn ends with it (under the slot
                // lock, so a request never lands on an ended turn).
                crate::chat_pause::clear_request(&self.key);
            }
            mine
        };
        // P1b: a turn a Pause all asked that ended without pausing settles
        // as finished first (one that paused already settled in
        // `settle_pause`, so this is a no-op then). Outside the slot lock:
        // the last one posts the summary notification.
        if ended {
            crate::chat_pause::settle_and_report(&self.key, crate::chat_pause::Settled::Ended);
        }
    }
}

/// Tests: [`register_stop_row`] for a turn with no queue row.
#[cfg(test)]
pub(crate) fn register_stop(key: &str) -> StopRegistration {
    register_stop_row(key, None)
}

/// Claim `key`'s stop slot for its new turn, the turn of queue row `row`
/// (admission passes it). A signal still registered for the same
/// conversation is stopped, mirroring the old "kill any prior child"; a
/// conversation runs one turn at a time, so none normally is. Other
/// conversations' slots are untouched (PR B4).
fn register_stop_row(key: &str, row: Option<i64>) -> StopRegistration {
    let signal = Arc::new(StopSignal::new());
    let own_chat = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let slot = StopSlot {
        signal: Arc::clone(&signal),
        row,
        own_chat: Arc::clone(&own_chat),
    };
    let previous = {
        let mut slots = CURRENT_STOP.lock().unwrap_or_else(PoisonError::into_inner);
        // P1a: a pause asked of an earlier turn never reaches this one.
        crate::chat_pause::clear_request(key);
        slots.insert(key.to_string(), slot)
    };
    if let Some(previous) = previous {
        previous.signal.request();
        // P1b: the turn a Pause all asked was replaced, never paused.
        crate::chat_pause::settle_and_report(key, crate::chat_pause::Settled::Ended);
    }
    StopRegistration {
        key: key.to_string(),
        signal,
        own_chat,
    }
}

/// Request a stop of `key`'s registered turn. False when that conversation
/// has no registered turn.
pub(crate) fn request_stop_for(key: &str) -> bool {
    let signal = CURRENT_STOP
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key)
        .map(|slot| Arc::clone(&slot.signal));
    match signal {
        Some(signal) => {
            signal.request();
            send_prep::cancel_session(key);
            true
        }
        None => false,
    }
}

/// P1a: ask `key`'s running turn to pause at its next safe point
/// (`chat_pause`), shown as pausing. False when the conversation has no
/// registered turn or that turn is already stopping (a Stop wins). Checked
/// and recorded under the slot lock, so the request can't outlive the turn.
pub(crate) fn request_pause_for(key: &str) -> bool {
    let slots = CURRENT_STOP.lock().unwrap_or_else(PoisonError::into_inner);
    let running = slots.get(key).is_some_and(|slot| !slot.signal.requested());
    let requested = crate::chat_pause::request_pause(key, running);
    // Review fix: set under the slot lock, and never over Cancelling, so a
    // Stop that lands right after this still shows as cancelling.
    if requested {
        crate::agent_sessions::mark_pausing(key);
    }
    requested
}

/// P1b: Pause all. Ask every conversation with a registered turn that is not
/// stopping and is a person's own chat (review fix: never a child task,
/// unattended, bare or coordinator turn) to pause at its next safe point, as `all`, except `except` (an
/// agent's own chat). Queued rows not yet admitted have no slot and are left
/// alone. Returns the keys asked, sorted, and notes them as one batch whose
/// last chat to settle posts the summary notification (`chat_pause`).
pub(crate) fn request_pause_all(vault: &Path, except: Option<&str>) -> Vec<String> {
    let mut asked: Vec<String> = {
        let slots = CURRENT_STOP.lock().unwrap_or_else(PoisonError::into_inner);
        slots
            .iter()
            .filter(|(key, slot)| {
                !slot.signal.requested()
                    && slot.own_chat.load(std::sync::atomic::Ordering::Relaxed)
                    && except != Some(key.as_str())
            })
            .map(|(key, _)| key.clone())
            .filter(|key| {
                // A chat already asked keeps its first reason and stays out
                // of the batch (its own Pause has no summary).
                !crate::chat_pause::pause_requested(key)
                    && crate::chat_pause::request_pause_as(
                        key,
                        true,
                        crate::chat_pause::PauseReason::All,
                    )
            })
            .collect()
    };
    asked.sort();
    for key in &asked {
        crate::agent_sessions::mark_pausing(key);
    }
    crate::chat_pause::begin_pause_all(vault, &asked);
    asked
}

/// M5 PR2: the person's own chats with a turn registered here and not
/// stopping (the chats Pause all would ask), sorted: the ones a sleep moves
/// to another Mac (`chat_move_trigger`).
pub(crate) fn running_own_chats() -> Vec<String> {
    let mut keys: Vec<String> = CURRENT_STOP
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter(|(_, slot)| {
            !slot.signal.requested() && slot.own_chat.load(std::sync::atomic::Ordering::Relaxed)
        })
        .map(|(key, _)| key.clone())
        .collect();
    keys.sort();
    keys
}

/// P1a review fix: keep the pause of a turn whose attempt ended at a safe
/// point (`Yield::Pause`), unless a Stop came first. Checked and recorded
/// under the slot lock while the turn's registration still holds it, so a
/// Stop either lands before (the pause is dropped and the turn closes as
/// stopped) or after (it still reaches this turn's slot, and `cancel` then
/// clears the record). The reason recorded is the request's (P1b: `all` for
/// Pause all), returned when the chat is now recorded as paused.
#[allow(clippy::too_many_arguments)]
// Writes SQLite under the stop-slot lock: a busy store can hold it for up
// to the session store's 2 s busy timeout (known, accepted: rare and short).
fn settle_pause(
    vault: &Path,
    key: &str,
    stop: &StopSignal,
    agent: &str,
    native: Option<&str>,
    note: &str,
    now: i64,
) -> Option<crate::chat_pause::PauseReason> {
    use crate::chat_pause::{PauseReason, Settled};
    let kept = {
        let _slots = CURRENT_STOP.lock().unwrap_or_else(PoisonError::into_inner);
        let reason = crate::chat_pause::requested_reason(key).unwrap_or(PauseReason::User);
        crate::chat_pause::clear_request(key);
        (!stop.requested()
            && crate::chat_pause::record_turn_pause_as(
                vault, key, agent, native, note, now, reason, None, 0,
            ))
        .then_some(reason)
    };
    // P1b: outside the slot lock, as the registration's drop.
    let how = if kept.is_some() {
        Settled::Paused(note.to_string())
    } else {
        Settled::Ended
    };
    crate::chat_pause::settle_and_report(key, how);
    kept
}

/// P1d review fix: record a turn's usage-limit pause only if no Stop came,
/// checked under the slot lock while its registration holds it, as
/// [`settle_pause`]. `Some(true)`: paused; `Some(false)`: a Stop won (close
/// as stopped, nothing recorded); `None`: the record could not be written
/// (the turn fails as before).
#[allow(clippy::too_many_arguments)]
fn settle_limit_pause(
    vault: &Path,
    key: &str,
    stop: &StopSignal,
    agent: &str,
    native: Option<&str>,
    limit: &crate::chat_pause::LimitPause,
    attempts: i64,
    now: i64,
) -> Option<bool> {
    let kept = settle_limit_record(vault, key, stop, agent, native, limit, attempts, now);
    // P1b delta review: a chat Pause all asked that paused at its limit
    // settles as paused in the summary, outside the slot lock.
    if kept == Some(true) {
        crate::chat_pause::settle_and_report(
            key,
            crate::chat_pause::Settled::Paused(limit.note.clone()),
        );
    }
    kept
}

/// [`settle_limit_pause`]'s record, under the slot lock.
#[allow(clippy::too_many_arguments)]
fn settle_limit_record(
    vault: &Path,
    key: &str,
    stop: &StopSignal,
    agent: &str,
    native: Option<&str>,
    limit: &crate::chat_pause::LimitPause,
    attempts: i64,
    now: i64,
) -> Option<bool> {
    let _slots = CURRENT_STOP.lock().unwrap_or_else(PoisonError::into_inner);
    if stop.requested() {
        return Some(false);
    }
    crate::chat_pause::record_turn_pause_as(
        vault,
        key,
        agent,
        native,
        &limit.note,
        now,
        crate::chat_pause::PauseReason::UsageLimit,
        limit.resume_at,
        attempts,
    )
    .then_some(true)
}

/// P1a: whether `key` has a turn registered, queued or running, so a resume
/// would start a second one.
pub(crate) fn turn_live(vault: &Path, key: &str) -> bool {
    CURRENT_STOP
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains_key(key)
        || queued_live_session(vault, Some(key)).is_some()
}

/// Stop `key`'s running turn only while it is queue row `row`'s, checked
/// and signalled under the slot lock; a slot that ended or changed hands is
/// left alone. Shown as cancelling, as [`cancel_running`].
pub(crate) fn cancel_running_row(key: &str, row: i64) -> bool {
    let signal = CURRENT_STOP
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key)
        .filter(|slot| slot.row == Some(row))
        .map(|slot| {
            send_prep::cancel_session_row(key, row);
            slot.signal.request();
            Arc::clone(&slot.signal)
        });
    if signal.is_some() {
        crate::agent_sessions::set_phase_for_key(
            key,
            crate::agent_sessions::SessionPhase::Cancelling,
        );
    }
    signal.is_some()
}

/// Stop `key`'s running turn and show it as cancelling.
pub(crate) fn cancel_running(key: &str) -> bool {
    let requested = request_stop_for(key);
    if requested {
        crate::agent_sessions::set_phase_for_key(
            key,
            crate::agent_sessions::SessionPhase::Cancelling,
        );
    }
    requested
}

/// Whether `key`'s turn was asked to stop, or holds no stop slot any more
/// (it is finishing). A stopped turn's lease outlives its child sweep, so a
/// worker batch checks this before each enqueue (#1068 review).
pub(crate) fn stop_requested_for(key: &str) -> bool {
    CURRENT_STOP
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key)
        .is_none_or(|slot| slot.signal.requested())
}

fn preparation_stop_requested(key: &str, row: i64) -> bool {
    CURRENT_STOP
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key)
        .is_some_and(|slot| slot.row == Some(row) && slot.signal.requested())
}

/// The keys of the registered turns (tests: the keyed-slot check that
/// replaced `is_busy`), sorted.
#[cfg(test)]
fn registered_stop_keys() -> Vec<String> {
    let mut keys: Vec<String> = CURRENT_STOP
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

/// The one registered key, when exactly one turn is registered (tests).
#[cfg(test)]
fn registered_stop_key() -> Option<String> {
    let keys = registered_stop_keys();
    (keys.len() == 1).then(|| keys[0].clone())
}

/// The `chat-event` a stopped turn ends with, after its receipt and before
/// `done`. `stop` is what ending the turn's processes established, or `None`
/// when the turn was stopped before its agent started. The frontend renders
/// it as a neutral notice (`cancelledNotice`), never as an error.
fn cancelled_event(stop: Option<process_group::StopOutcome>) -> Value {
    crate::chat_event::build::cancelled(stop.map(process_group::StopOutcome::as_str), false)
}

/// P1a: the `chat-event` a paused turn ends with, after its receipt and
/// before `done`, in place of `cancelled`: what the chat was doing and when
/// (unix seconds).
/// P1b to P1d: also why (`reason`) and when it resumes on its own
/// (`resumeAt`, unix seconds or null).
fn paused_event(session_key: &str, note: &str, at: i64) -> Value {
    crate::chat_event::build::paused(
        session_key,
        note,
        at,
        crate::chat_pause::PauseReason::User.as_str(),
    )
}

/// The `cancelled` event of a turn stopped before its agent was spawned
/// (Paperclip PR 14b): its native session never saw the request, so the
/// frontend does not offer Continue for it.
fn cancelled_before_spawn_event() -> Value {
    crate::chat_event::build::cancelled(None, true)
}

/// The `activity` chat event a running turn's ticker emits every 15 s
/// (Paperclip PR 15b): when the agent last printed anything (its spawn
/// before the first line) and the progress phase of its latest tool call.
/// Tagged with the turn's `TurnTag` by the caller. Informational only.
fn activity_event(
    run_id: &str,
    session_key: &str,
    snapshot: &crate::run_activity::ActivitySnapshot,
) -> Value {
    crate::chat_event::build::activity(
        run_id,
        session_key,
        snapshot.last_event_at.clone(),
        snapshot.progress,
    )
}

/// How a chat turn's agent is supervised. It requires the turn's registered
/// stop signal, so the stop button always reaches the running agent. The
/// optional deadline and the watchdog are the agent's settings
/// (`agent_settings::run_limits`, merged plan slice 3), set per attempt by
/// `drive_turn_attempts`.
fn turn_supervision<'a>(stop: &'a StopSignal, agent_name: &str) -> Supervision<'a> {
    Supervision {
        stop: Some(stop),
        // Chat is never stopped by a budget (D2: it only warns).
        limit: None,
        // Set per attempt by `drive_turn_attempts` (merged plan slice 3).
        deadline: None,
        stderr: StderrPolicy::Echo(format!("[{agent_name} stderr]")),
        timings: Timings::default(),
        // Set per turn by `drive_turn_attempts` (Paperclip PR 15b).
        watch: None,
    }
}

// ── The chat-row gate and where a turn runs ─────────────────────────────────

// The gate lives in chat_rows.rs and the working-directory check in
// chat_context.rs (ST11); these keep every `chat::` path.
use crate::chat_context::prepare_chat_send_context;
pub(crate) use crate::chat_context::{preflight_chat_context, CHAT_CONTEXT_RECOVERED_ERROR};
#[cfg(test)]
use crate::chat_rows::active_chat_row;
pub(crate) use crate::chat_rows::{active_chat_rows, ChatRowGuard};

fn is_main_agent_cwd(vault: &Path, cwd: &Path) -> bool {
    crate::workspaces::workspace_for_cwd(vault, cwd)
        .ok()
        .flatten()
        .is_some_and(|workspace| crate::workspaces::is_main_agent(&workspace.id))
}

/// The tool rules a turn in the COO's folder adds (L6): the standing orders
/// rule and nothing else, on agents that hold deny rules. Other agents run
/// the COO too; for them the rule stays in the prompt.
pub(crate) fn coo_denied_tools(vault: &Path, cwd: &Path, agent: AgentKind) -> Vec<String> {
    if !is_main_agent_cwd(vault, cwd) {
        return Vec::new();
    }
    crate::main_agent::STANDING_ORDERS_DENY
        .iter()
        .filter(|rule| crate::agent::holds_deny(agent, rule))
        .map(|rule| rule.to_string())
        .collect()
}

/// Locate the `claude` CLI binary. Tries common install locations because a
/// macOS .app launched from Finder doesn't inherit the user's shell PATH.
pub fn find_claude_binary() -> Option<PathBuf> {
    // Test bed only: a candidate launched with a fake-agent folder.
    if let Some(fake) = crate::instance::candidate_agent_bin("claude") {
        return Some(fake);
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".local/bin/claude"));
        candidates.push(home.join(".claude/local/claude"));
    }
    candidates.push(PathBuf::from("/opt/homebrew/bin/claude"));
    candidates.push(PathBuf::from("/usr/local/bin/claude"));
    candidates.push(PathBuf::from("/usr/bin/claude"));
    if let Some(p) = candidates.into_iter().find(|p| p.exists()) {
        return Some(p);
    }
    // Last resort: rely on PATH (works under `tauri dev`).
    Some(PathBuf::from("claude"))
}

fn emit(app: &impl crate::core_host::CoreHost, payload: Value) {
    emit_event(app, payload);
}

/// Every `chat-event` leaves here, wherever it was made (this module, the
/// pump, held sends, Stop). The phone API's live buffer sees each one first,
/// in the order its thread emits it (`phone_access::live`), so a phone
/// follows a turn from the same events the window draws. A Rust listener on
/// the event bus would not do: Tauri defers an event to a pending list when
/// another thread is emitting, which can reorder a turn's deltas.
pub(crate) fn emit_event(app: &impl crate::core_host::CoreHost, payload: Value) {
    #[cfg(test)]
    crate::chat_event::assert_conforms(&payload);
    crate::phone_access::tap(&payload);
    let _ = app.emit(EVENT, payload);
}

/// How a started chat turn ends: what [`finish_observed_run`] finalizes.
/// Every exit of [`execute_turn`] after the transcript commit builds one and
/// hands it to the funnel; none writes the transcript, the usage row or the
/// receipt itself.
struct TurnClose {
    /// The history, transcript and receipt status: `success`, `error` or
    /// `cancelled` (a paused turn's too, P1a: see [`TurnClose::paused`]).
    status: &'static str,
    /// The usage row's outcome.
    outcome: LedgerOutcome,
    /// The agent's reply, kept whole in the transcript (its `hollow-next`
    /// block included, so a reopened conversation still offers it); empty
    /// when it wrote none.
    assistant: String,
    /// The transcript's closing event (`error` or `cancelled`) and its text.
    event: Option<(&'static str, String)>,
    /// Emitted once the receipt is out: the error or the stop notice. An exit
    /// that emitted its own error first carries none.
    notice: Option<Value>,
    /// `TurnEnd.error`, which the queue row shows.
    error: Option<String>,
    /// The native session `done` hands back.
    session_id: Option<String>,
}

impl TurnClose {
    /// The turn failed with `message`: the transcript's `error` event, and
    /// the `error` chat event after the receipt.
    fn failed(outcome: LedgerOutcome, session_id: Option<String>, message: String) -> TurnClose {
        TurnClose {
            status: "error",
            outcome,
            assistant: String::new(),
            event: Some(("error", message.clone())),
            notice: Some(crate::chat_event::build::error(message.clone())),
            error: Some(message),
            session_id,
        }
    }

    /// The user stopped the turn: the transcript's `cancelled` event records
    /// `state`, and `notice` (the stop event) follows the receipt.
    fn cancelled(
        outcome: LedgerOutcome,
        state: &str,
        notice: Value,
        session_id: Option<String>,
    ) -> TurnClose {
        TurnClose {
            status: "cancelled",
            outcome,
            assistant: String::new(),
            event: Some(("cancelled", state.to_string())),
            notice: Some(notice),
            error: None,
            session_id,
        }
    }

    /// P1a: the turn paused at a safe point with `note`, keeping its native
    /// session. Its history, receipt and queue status is `cancelled` (neutral:
    /// never an error, a failover, a retry or a follow-up), while the
    /// transcript's closing event is `paused` with the note and the `paused`
    /// chat event follows the receipt instead of `cancelled`.
    fn paused(
        outcome: LedgerOutcome,
        note: String,
        session_key: &str,
        session_id: Option<String>,
    ) -> TurnClose {
        let at = chrono::Utc::now().timestamp();
        TurnClose {
            status: "cancelled",
            outcome,
            assistant: String::new(),
            notice: Some(paused_event(session_key, &note, at)),
            event: Some(("paused", note)),
            error: None,
            session_id,
        }
    }

    /// P1b to P1d: a pause's reason and resume time, on its `paused` event.
    fn paused_for(
        mut self,
        reason: crate::chat_pause::PauseReason,
        resume_at: Option<i64>,
    ) -> TurnClose {
        if let Some(notice) = self.notice.as_mut().and_then(Value::as_object_mut) {
            notice.insert("reason".into(), json!(reason.as_str()));
            notice.insert("resumeAt".into(), json!(resume_at));
        }
        self
    }

    /// The turn paused (P1a), rather than being stopped.
    fn is_pause(&self) -> bool {
        matches!(self.event, Some(("paused", _)))
    }

    /// Keep the agent's reply in the transcript too.
    fn with_reply(self, assistant: String) -> TurnClose {
        TurnClose { assistant, ..self }
    }
}

/// Turn guards: the night guard ended the turn (`night_stop` is its night
/// total and cap). Its item becomes `stopped` before the turn's own end, so
/// a settle never reads it as failed, and the turn ends with the same words;
/// otherwise the failover's stop text, if any, stands.
fn close_night_stop(
    vault: &Path,
    session_key: &str,
    night_stop: Option<(f64, f64)>,
    failover_stop: Option<String>,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<String> {
    let Some((total, cap)) = night_stop else {
        return failover_stop;
    };
    let note = crate::night_queue::mid_turn_note(total, cap);
    if let Err(e) = crate::night_queue::stopped_mid_turn(vault, session_key, &note, now) {
        eprintln!("[night queue] mid-turn stop: {e}");
    }
    Some(note)
}

/// Q8: a self-hosted turn's endpoint for its usage row, as `(name, served
/// model id)` (`UsageRun::set_self_hosted`); `None` for any other turn.
fn self_hosted_endpoint(spec: &agent::RunSpec) -> Option<(&str, &str)> {
    spec.extras
        .self_hosted
        .as_ref()
        .map(|t| (t.name.as_str(), t.served_model.as_str()))
}

/// Q8: the cost a turn's transcript `result` keeps: Claude Code's own
/// figure, or none for a self-hosted run (Claude Code made it up for an id
/// it doesn't know; the run is billed by GPU time).
fn transcript_cost(ledger: &UsageRun, reported: Option<f64>) -> Option<f64> {
    reported.filter(|_| ledger.billing() != crate::usage::Billing::SelfHosted)
}

/// Turn guards: a night turn's running estimate gives way to its real cost
/// (`cost`, the usage row's) once the turn has ended.
fn reconcile_night_spend(
    spend: &crate::night_queue::NightSpend,
    run_id: &str,
    session_key: &str,
    cost: Option<f64>,
) {
    if let Some(was) = spend.reconcile(run_id, cost) {
        eprintln!(
            "[night queue] {session_key}: estimated ${:.2}, real {}",
            was.total(),
            cost.map_or_else(|| "unreported".to_string(), |c| format!("${c:.2}"))
        );
    }
}

/// Save the reply's closed canvas fences to its conversation's store: the
/// workspace whose folder holds its cwd, else its own app-data store, as
/// `conversationCanvasScope` picks in the window. Never changes the turn's
/// outcome.
fn save_reply_canvases(vault: &std::path::Path, session_key: &str, run_id: &str, reply: &str) {
    let bodies = crate::canvas_fence::reply_canvas_bodies(reply);
    if bodies.is_empty() {
        return;
    }
    let Ok(app_data) = crate::agent_settings::app_data_dir() else {
        return;
    };
    let workspace = crate::session_store::session_cwd(vault, session_key)
        .and_then(|cwd| {
            crate::workspaces::workspace_for_cwd(vault, std::path::Path::new(&cwd))
                .ok()
                .flatten()
        })
        .map(|w| w.id);
    let now = chrono::Utc::now().to_rfc3339();
    crate::workspace_canvases::record_reply(
        vault,
        &app_data,
        crate::workspace_canvases::Scope {
            workspace_id: workspace.as_deref(),
            session_key: Some(session_key),
        },
        &bodies,
        run_id,
        &now,
    );
}

/// Repo features R2b: store chat run `run_id`'s last context figure and
/// its window on conversation `key` (`context_gauge::store_figure`), once
/// its ledger finished: the gauge's reading dies with the turn. Codex's
/// figure and window come from its rollout (overnight 9 A10, the ledger's
/// `context_window`); Antigravity reports none, so nothing is stored.
fn store_context_figure(
    vault: &Path,
    key: &str,
    run_id: &str,
    agent: Option<AgentKind>,
    usage: &usage_ledger::RunUsage,
) {
    // A self-hosted model's window is exact (Q3), like Codex's reported one.
    let exact = usage.context_window.or_else(|| {
        agent.and_then(|a| crate::context_gauge::exact_window(a, usage.model.as_deref()))
    });
    let figure = agent.zip(usage.context_tokens).map(|(agent, tokens)| {
        let window = exact.or_else(|| {
            crate::context_gauge::context_window(agent, usage.model.as_deref(), Some(tokens))
        });
        (tokens, window)
    });
    let reported = exact.is_some();
    crate::context_gauge::store_figure(vault, key, run_id, figure, reported);
}

/// Overnight 9 A10: compactions a Codex turn's rollout reported count as
/// the stream's own (`ParseState::compactions`, OB22's receipt count) and
/// mark the turn compacted, so R1 re-grounds the next resumed turn as it
/// does after a Claude `compact_boundary`.
pub(crate) fn apply_codex_rollout(state: &mut agent::ParseState, seen: crate::codex_rollout::Seen) {
    if seen.compactions > 0 {
        state.compacted = true;
        state.compactions = state.compactions.saturating_add(seen.compactions);
    }
}

/// Overnight 9 A10: what a Codex turn's rollout said, into the stream state
/// ([`apply_codex_rollout`]), the ledger's figure (the ring and the stored
/// figure) and the live gauge (`sessions.context`, the step and compaction
/// notes).
fn feed_codex_rollout(
    seen: crate::codex_rollout::Seen,
    state: &mut agent::ParseState,
    ledger: &mut UsageRun,
    gauge: &crate::context_gauge::GaugeGuard,
) {
    apply_codex_rollout(state, seen);
    if let Some((tokens, window)) = seen.figure {
        ledger.set_context_figure(tokens, window);
        gauge.publish_figure(tokens, Some(window));
    }
    gauge.set_compactions(state.compactions);
}

/// THE terminal funnel: every exit of a started chat turn (success, error,
/// stop, stall, deadline) routes through here exactly once, with its
/// [`TurnClose`]. Merged plan slice 3 (B·F10): the transcript, the usage row
/// and the receipt are finalized together, so no exit finalizes only some
/// of them. A turn no live process will finish (a crash, a lost turn) has
/// its own funnel, `run_queue`'s `interrupt_chat_run`: it closes the
/// transcript turn and the usage rows in the transaction that ends the queue
/// row, and writes no receipt (no evidence survived the process); history's
/// startup reconciliation closes the manifest. In order: it writes the transcript's tail (the reply, then the
/// closing event), finishes the turn's usage row and emits the `usage` chat
/// event (so the chip reflects stopped and failed turns too), closes any
/// teammate delegations still recorded as running under this run, then
/// releases the vault lease (`RunLease::finish`, the public `finish_run`, so
/// PR 5's protected-change flags are computed and notified), releases the
/// live-phase record, closes the session-store turn, writes the receipt,
/// emits the exit's notice and builds the [`TurnEnd`]. It emits no `done`:
/// since Paperclip PR 7b the pump emits that once the queue row is terminal.
///
/// The usage steps come first because they need no lease and must never run
/// under the history lock or PR 5's permission guard (`finish_run` takes
/// both); PR 8's claim-time budget check can then see this turn's usage as
/// soon as the lease is free.
///
/// Ordering is load-bearing. The phase record is released only AFTER
/// `finish_run` has had its chance to drop the lease, so the live chat detail
/// `agent_sessions::describe_holder` adds to a refusal ("a Claude turn is
/// still cancelling …") never outlives the lease it describes, while every
/// refusal that lands during the tail of the turn still names it. Every caller
/// reaches this only after `process_group::drive` has ended the turn's
/// processes, so the lease never outlives them. `workspaces::record_session`
/// stays in here (after the lease, before the pump's next admission), so the
/// next pump-admitted run's baseline never races it. Other conversations may
/// still be live in the same workspace folder, so it is a host write
/// (`automated_history::host_write`, chat concurrency F3, 2026-10-20) that
/// their History leaves out.
///
/// Paperclip PR 16: after the delegate rows are closed and before the lease
/// is released, the turn's repository changes are captured (outside
/// `HISTORY_LOCK`; nothing runs for an unarmed or empty baseline) and set on
/// the collector before its receipt is written. That capture is why this is
/// async.
///
/// `learn_from` is the user's side of the turn's exchange ([`LearnFrom`]),
/// given only by the exit that can succeed: a successful turn's [`TurnEnd`]
/// carries its learning job.
#[allow(clippy::too_many_arguments)]
async fn finish_observed_run(
    out: &TurnEvents<'_>,
    vault: &std::path::Path,
    lease: RunLease,
    mut collector: crate::evidence::EvidenceCollector,
    session_key: &str,
    turn_ord: i64,
    ledger: UsageRun,
    live: LiveRecord,
    close: TurnClose,
    learn_from: Option<LearnFrom>,
) -> TurnEnd {
    let status = close.status;
    // P1a: a resume's turn gives its claim back on every exit, so one that
    // ended before its spawn leaves the chat paused; after a spawn the
    // record is already gone, and a new pause starts unclaimed, so this
    // releases nothing then.
    // P1c delta review: a Stop's close (`cancelled`) is the person's own
    // doing, so it clears the time without the "did not start" warning.
    crate::chat_pause::release_claim_as(
        vault,
        session_key,
        out.tag.client_turn_id.as_deref(),
        status != "cancelled",
    );
    // Mid-turn steering: a steer the agent never took says so, and every
    // steer goes on the receipt.
    let (steers, moved) = crate::steering::settle(session_key, lease.id());
    for steer in &moved {
        let _ = crate::session_store::append_event(
            vault,
            session_key,
            turn_ord,
            "steerState",
            &crate::steering::state_payload(steer),
        );
        out.emit(crate::steering::steer_event(session_key, steer));
    }
    collector.set_steers(&steers);
    // The reply's canvas fences, for the receipt (Canvas C3 wave 2 C4).
    collector.set_reply_canvases(&close.assistant);
    // Whether its final reply claims a pass (S27).
    collector.set_reply_claim(&close.assistant);
    // The kept passages its final reply names (S26).
    collector.set_final_reply_citations(&close.assistant);
    // Repo features R5: what earlier turns read, for its `path:line` cites.
    if collector.has_reply_cites() {
        collector.set_earlier_reads(crate::step_spine::earlier_reads(
            vault,
            session_key,
            turn_ord,
        ));
    }
    // Its finished canvases, saved whether or not a window watched the reply
    // (question cards, 2026-10-14): before the transcript, so whoever reads
    // the reply finds them saved.
    save_reply_canvases(vault, session_key, lease.id(), &close.assistant);
    // The transcript's tail first. A trace that cannot be written must never
    // change the turn's outcome.
    if !close.assistant.trim().is_empty() {
        let _ = crate::session_store::append_event(
            vault,
            session_key,
            turn_ord,
            "assistant",
            &close.assistant,
        );
    }
    if let Some((kind, text)) = &close.event {
        let _ = crate::session_store::append_event(vault, session_key, turn_ord, kind, text);
    }
    let run_id = lease.id().to_string();
    let agent = ledger.agent();
    let (usage, _) = ledger.finish_receipt(close.outcome);
    // Repo features R2b: the turn's last context figure, for the next
    // resumed turn's `<hollow-context>` line.
    store_context_figure(vault, session_key, &run_id, agent, &usage);
    // OB05: the turn's calls and peak context, for its receipt.
    collector.set_api_usage(usage.calls.clone());
    reconcile_night_spend(
        &crate::night_queue::SPEND,
        &run_id,
        session_key,
        usage.cost_usd,
    );
    out.emit(crate::chat_event::build::usage(
        usage.turn_json(),
        session_key,
    ));
    if let Err(error) = usage_ledger::close_orphaned_delegates(vault, &run_id) {
        eprintln!("[usage] closing this turn's teammate rows failed: {error}");
    }
    // A stopped turn stops the child task turns it started (F10, D2), before
    // its lease goes, so a queued one never starts in the gap.
    // A pause (P1a) leaves them running: the chat resumes later.
    let child_stops = if status == "cancelled" && !close.is_pause() {
        stop_child_turns_now(out.app, vault, &run_id)
    } else {
        Vec::new()
    };
    // A turn that ran on the Mac mini (Phase 2): its work comes back to the
    // worktree before the receipt captures its changes, and the chat says
    // how (applied, unchanged, kept on a branch, or why not).
    // A vault workspace's folder (Phase 3) comes back the same way, file by
    // file, and the files it wrote are this run's own in History.
    if let Some(target) = crate::remote_session::take_turn(session_key) {
        let host = target.host.clone();
        let worktree = target.git_worktree;
        let turn = target.turn.clone();
        let local_folder = target.folder.as_ref().map(|f| f.local.clone());
        let (back, folder) = tokio::task::spawn_blocking(move || {
            (
                crate::remote_session::collect_placed(&target),
                crate::remote_session::collect_folder(&target),
            )
        })
        .await
        .unwrap_or_else(|e| {
            let why = format!("bringing the work back failed ({e})");
            (
                (Err(why.clone()), true),
                local_folder.as_ref().map(|_| Err(why)),
            )
        });
        // Overnight R1: the turn's record goes once its work is here (or
        // said where it is); a Hollow that quits before this finds it.
        crate::remote_session::forget_turn(session_key, &turn);
        record_remote_back(
            out,
            vault,
            session_key,
            turn_ord,
            &run_id,
            RemoteBack {
                host: &host,
                worktree: worktree.then_some(back),
                folder,
                local_folder: local_folder.as_deref(),
                earlier: false,
            },
        );
    }
    // Timed as repoCaptureMs (Paperclip PR 15c) only when an armed baseline
    // was there to capture against.
    let repo_baseline = collector.take_repo_baseline();
    let repo_timed = repo_baseline.is_some();
    let repo_started = std::time::Instant::now();
    let repo = crate::repo_diff::finish_for_run(vault, &run_id, repo_baseline).await;
    let repo_capture_ms = repo_timed.then(|| crate::evidence::ms_since(repo_started));
    collector.timings_mut().repo_capture_ms = repo_capture_ms;
    collector.set_repo_changes(repo);
    // CP1: the turn's after checkpoint, when its before one was recorded.
    crate::turn_checkpoints::after_turn(vault.to_path_buf(), session_key.to_string(), turn_ord)
        .await;
    // Usage budgets (Paperclip PR 8a): chat never stops, but a turn that
    // pushed a hard-stop budget over its limit pauses the AUTOMATIONS under
    // it now, before the lease is released (and while the run is tracked, so
    // the pause's queue write is attributed). Infallible.
    crate::budgets::after_run(
        &crate::budgets::Store::for_app(vault),
        vault,
        &crate::budgets::RunIntent::chat(agent),
        chrono::Utc::now(),
    );
    // Hold queued automations back for the follow-up grace, so a routine
    // never grabs the vault the moment a turn ends and refuses the user's next
    // message (Paperclip PR 7). Noted BEFORE the lease is released: releasing
    // it wakes the pump, which reads the gap at once, so a note written after
    // the release lost that race and let a queued routine start straight away.
    // Noted again afterwards so the grace also covers a slow finalization.
    // A scoped turn (PR B3) never held the vault, so it holds nothing back.
    let scoped = lease.scope().is_some();
    if !scoped {
        crate::run_pump::note_chat_finished(chrono::Utc::now());
    }
    // Tool calls this turn left unanswered can never be approved now
    // (Paperclip PR 21).
    crate::approvals::forget_session(session_key);
    // The files its own structured writes named, so a turn that shared its
    // workspace folder is credited only with them (chat concurrency F1,
    // 2026-10-20). Handed over just before the diff, once its agent is gone.
    if scoped {
        crate::automated_history::note_own_writes(
            vault,
            &run_id,
            collector.own_written_paths(vault),
        );
    }
    // The history diff and lease release, timed (Paperclip PR 15c).
    let diff_started = std::time::Instant::now();
    let finished = lease.finish(status);
    collector.timings_mut().finalize_diff_ms = Some(crate::evidence::ms_since(diff_started));
    // The services this turn started, command and folder, for its receipt
    // (merged plan decision 1). Taken after the lease is gone, so the run is
    // no longer live and its services grant can start nothing more.
    collector.set_services(crate::services::take_started_by_run(&run_id));
    if !scoped {
        crate::run_pump::note_chat_finished(chrono::Utc::now());
    }
    // Released on every path, including the finalization-failure path below —
    // a leaked record would report "busy" forever.
    crate::agent_sessions::clear_run(&run_id);
    drop(live);
    // A trace that cannot be written must never change the turn's outcome.
    let _ = crate::session_store::finish_turn(vault, session_key, turn_ord, status);
    let _ = crate::step_spine::finish_turn(vault, session_key, turn_ord, status);
    // Settings' optional model retitle after the first answer (morning N5),
    // for the person's own chats only.
    if out.retitle.load(std::sync::atomic::Ordering::Relaxed) {
        let only_on = out.retitle_on.get().map(String::as_str);
        crate::chat_title::after_turn(
            out.app,
            vault,
            session_key,
            turn_ord,
            status,
            &run_id,
            only_on,
        );
    }
    let history = match finished {
        Ok(run) => run,
        Err(error) => {
            out.emit(crate::chat_event::build::error(format!(
                "history finalization failed: {error}"
            )));
            return end_of_turn(out, session_key, close, learn_from, run_id, false);
        }
    };
    // Only its own changes count (F1): another chat's in its folder don't.
    let changed = history.changes.iter().any(|change| !change.unattributed);
    let changes = history
        .changes
        .into_iter()
        .map(crate::evidence::ObservedChange::from_history)
        .collect();
    // Its cost with its descendants' (F10, D2), and what the stop did to
    // each child task turn.
    let mut rollup = crate::descendants::rollup(vault, &run_id);
    for child in rollup.descendants.iter_mut() {
        child.stop = child_stops
            .iter()
            .find(|(id, _)| Some(*id) == child.child_queue_id)
            .map(|(_, outcome)| outcome.to_string());
    }
    collector.set_descendants(rollup);
    // The receipt copies the queue row's wait and mirrors its timings back
    // into that row, which is still `running` (the pump ends it after
    // `execute_turn` returns).
    match collector.finish_mirrored(vault, status, changes) {
        Ok(receipt) => {
            if let Err(error) = crate::workspaces::record_session(vault, session_key) {
                out.emit(crate::chat_event::build::error(format!(
                    "Workspace session record was not saved: {error}"
                )));
            }
            // X1: a linked run's audit seam and, when tracked, its Outcome.
            crate::tracker::work::after_turn(
                vault,
                session_key,
                status,
                Some(&receipt),
                &close.assistant,
            );
            // T3a: the person's own chat that did real work gets an issue
            // (Settings, Track agent work: a dry run records it only).
            crate::tracker::chat_work::after_turn(
                vault,
                session_key,
                out.person_chat.load(std::sync::atomic::Ordering::Relaxed),
                status,
                &receipt,
                &close.assistant,
            );
            out.emit(crate::chat_event::build::evidence_receipt(&receipt));
        }
        Err(error) => {
            out.emit(crate::chat_event::build::error(format!(
                "evidence receipt failed: {error}"
            )));
        }
    }
    end_of_turn(out, session_key, close, learn_from, run_id, changed)
}

/// The funnel's last step: the exit's notice, after the receipt, then its
/// [`TurnEnd`], carrying the learning job of a successful turn (`learn_from`
/// is the user's side of its exchange; `changed`, whether it changed the
/// vault).
fn end_of_turn(
    out: &TurnEvents<'_>,
    session_key: &str,
    close: TurnClose,
    learn_from: Option<LearnFrom>,
    run_id: String,
    changed: bool,
) -> TurnEnd {
    if let Some(notice) = close.notice {
        out.emit(notice);
    }
    // Durable-learning extraction stays post-turn and detached; the pump
    // starts it after `done` (see `spawn_learning`).
    let learning = learning_job(learn_from, close.status, &close.assistant, run_id, changed);
    TurnEnd {
        status: close.status.into(),
        error: close.error,
        done: out.done(close.session_id, session_key),
        learning,
    }
}

/// Whether a tool name is a teamwork delegation, whose result may name
/// images the teammate created. Claude's `mcp__hollow_team__ask_agent` and,
/// since Paperclip PR 13 normalized Codex MCP calls, Codex's identical name
/// both match, so a Codex delegation's images render inline too.
fn is_team_tool(name: &str) -> bool {
    name.contains("hollow_team")
}

/// Probe `claude -p` to capture the `system/init` event, which advertises
/// the available `slash_commands` for this user's environment (built-ins +
/// installed skills + plugin commands). We send `/clear` as a no-op prompt
/// and `--no-session-persistence` so the probe doesn't write a session log.
pub async fn discover_slash_commands(cwd: PathBuf) -> Result<Vec<String>, String> {
    let bin = find_claude_binary().ok_or_else(|| "claude binary not found".to_string())?;
    let mut cmd = Command::new(&bin);
    crate::agent::isolate_claude(
        &mut cmd,
        None,
        None,
        &crate::agent::workspace_policy(AgentKind::Claude, "acceptEdits"),
    );
    cmd.arg("-p")
        .arg("/clear")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--verbose")
        .arg("--no-session-persistence")
        .arg("--permission-mode")
        .arg("acceptEdits");
    cmd.current_dir(&cwd);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    process_group::isolate(&mut cmd);

    let child = cmd.spawn().map_err(|e| format!("spawn claude: {e}"))?;
    let guard = GroupGuard::adopt(&cmd, &child);

    // Bound the probe: a hung claude must not wedge the chat UI. It only
    // needs the first `system/init` event, so the probe is stopped (with
    // everything it started) as soon as that arrives.
    let probe_stop = StopSignal::new();
    let mut commands: Vec<String> = Vec::new();
    let mut seen_init = false;
    let supervision = Supervision {
        stop: Some(&probe_stop),
        limit: None,
        deadline: Some(DISCOVER_TIMEOUT),
        stderr: StderrPolicy::Silent,
        timings: Timings::default(),
        watch: None,
    };
    let driven = process_group::drive(child, guard, supervision, |line| {
        if seen_init || line.is_empty() {
            return;
        }
        let evt: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return,
        };
        let typ = evt.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let subtype = evt.get("subtype").and_then(|v| v.as_str()).unwrap_or("");
        if typ == "system" && subtype == "init" {
            if let Some(arr) = evt.get("slash_commands").and_then(|v| v.as_array()) {
                commands = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
            }
            seen_init = true;
            probe_stop.request();
        }
    })
    .await;

    if driven.missing_stdout {
        return Err("no stdout from claude".to_string());
    }
    if driven.ended == Ended::TimedOut {
        return Err("claude slash-command discovery timed out".to_string());
    }
    Ok(commands)
}

// ── Queued chat turns (Paperclip PR 7b, D3) ─────────────────────────────────

// Accepting, admitting and ending a queued turn live in chat_queue.rs (B14),
// a child module so it still reads this file's private items; these keep
// every `chat::` path another module uses.
#[path = "chat_queue.rs"]
mod chat_queue;
#[path = "send_prep.rs"]
mod send_prep;
#[cfg(test)]
use chat_queue::turn_inputs;
pub(crate) use chat_queue::{
    accept_keyed_turn, accept_turn, admit_turn, queued_end_events, queued_unattended, AdmittedTurn,
    ChatAdmission, ChatSendRequest, ChatTurnPayload, QueuedEnd,
};
#[cfg(test)]
pub(crate) use chat_queue::{accept_turn_with_effort, priority_turn_ready};
use chat_queue::{
    end_unstarted_turn, exit_error_records, exit_outcome, failover_outcome, fenced_card_allowed,
    learning_job, live_result_kind, reopen_unstarted_answers, settle_unstarted_answers,
    stable_layers_holder, turn_inputs_prefetched, turn_learns, LearnFrom, LearningJob, LiveRecord,
    TurnEnd, TurnEvents, TurnInputs,
};

#[cfg(test)]
pub(crate) use chat_queue::AcceptedTurn;
pub(crate) use chat_queue::CHAT_ALREADY_QUEUED;
#[cfg(test)]
use chat_queue::{ExitOutcome, TurnTag};

// ── Attempts: the fresh-session retry (Paperclip PR 14b) ────────────────────

/// AG7: the subagent thread a stream line belongs to: Claude's top-level
/// `parent_tool_use_id` (the spawning `Task` call's id), or `None` for the
/// main thread and for any agent whose lines have no such field (Codex,
/// Antigravity). A cheap scan first, so most lines are never parsed twice;
/// the parsed top-level field then decides.
fn subagent_parent(line: &str) -> Option<String> {
    const FIELD: &str = "\"parent_tool_use_id\"";
    let named = line.match_indices(FIELD).any(|(at, _)| {
        line[at + FIELD.len()..]
            .trim_start()
            .strip_prefix(':')
            .is_some_and(|rest| rest.trim_start().starts_with('"'))
    });
    if !named {
        return None;
    }
    serde_json::from_str::<Value>(line)
        .ok()?
        .get("parent_tool_use_id")?
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

/// AG7: `payload` with `parentToolUseId` added for a subagent's event only,
/// so a main-thread event is exactly what it was (additive for old readers).
fn attributed(mut payload: Value, parent: Option<&str>) -> Value {
    if let (Some(parent), Some(object)) = (parent, payload.as_object_mut()) {
        object.insert("parentToolUseId".into(), json!(parent));
    }
    payload
}

/// Where one turn's `chat-event`s go: [`TurnEvents`] in production, a
/// recorder in tests (the attempt loop needs no AppHandle).
trait ChatSink {
    fn emit(&self, payload: Value);
    /// AG4b: the app that shows an approval card for a deferred tool call;
    /// `None` for a sink that cannot show one (a test sink), whose deferred
    /// calls are declined.
    fn approval_core(&self) -> Option<Core> {
        None
    }
}

fn proposed_plan_text(name: &str, input: &Value) -> Option<String> {
    if !matches!(name, "ExitPlanMode" | "exit_plan_mode") {
        return None;
    }
    let text = input.get("plan").and_then(Value::as_str)?.trim();
    if text.is_empty() {
        return None;
    }
    Some(crate::claude_runner::truncate(
        &crate::claude_runner::redact(text),
        16_000,
    ))
}

/// Compiler progress only: complete means prepared, not accepted by an agent.
fn emit_context_handoff(out: &impl ChatSink, key: &str, run: &str, state: &str, entries: u32) {
    out.emit(crate::chat_event::build::panel(
        key,
        "hollow:handoff",
        json!({ "runId": run, "state": state, "entries": entries }),
    ));
}

impl ChatSink for TurnEvents<'_> {
    fn emit(&self, payload: Value) {
        emit(self.app, self.tag.apply(payload));
    }
    fn approval_core(&self) -> Option<Core> {
        Some(self.app.clone())
    }
}

/// Repo features R2a: whether the context gauge keeps quiet for a turn in
/// permission `mode`: plan mode, a bare scratch turn, the night queue's
/// unattended turn, or a Main agent wake or check-in (`main_unattended`,
/// asked last since it reads the run queue).
fn gauge_quiet(
    mode: &str,
    bare: bool,
    unattended: bool,
    main_unattended: impl FnOnce() -> bool,
) -> bool {
    mode == "plan" || bare || unattended || main_unattended()
}

/// What every attempt of one turn shares.
struct TurnCx<'a> {
    vault: &'a Path,
    session_key: &'a str,
    turn_ord: i64,
    run_id: &'a str,
    /// The run's working folder (question-card targets resolve against it).
    run_cwd: &'a Path,
    agent: AgentKind,
    /// The configuration fingerprint recorded with a newly announced native id.
    fingerprint: &'a str,
    /// Its per-part hashes, recorded with it (OB03).
    config_parts: &'a str,
    /// What a config-changed reset names (`chat_resume::reset_parts`):
    /// the `sessionReset` event's `changed`.
    reset_parts: &'a [&'static str],
    /// Set by [`announce_reset`]: this turn moved the conversation to a
    /// fresh native session, so an answer it could not deliver is bound to
    /// the abandoned one ([`settle_unstarted_answers`]).
    reset_announced: std::sync::atomic::AtomicBool,
    /// Repo features R2a: the context gauge never notes this turn's growth
    /// steps (plan mode, a bare scratch turn, an unattended turn).
    quiet: bool,
    /// The person sent this turn, so a successful spawn closes open cards
    /// that yield to free text. A wake, check-in, pulse or scheduled turn
    /// leaves them open.
    closes_cards: bool,
}

/// The turn's warn-only budget guard (Paperclip PR 8b) and the warnings it
/// delivered, shared by both attempts.
struct TurnWatch {
    guard: Option<crate::budgets::BudgetGuard>,
    delivered: Vec<crate::budgets::BudgetNotice>,
    /// COO rebuild L9: an unattended turn's subagent and project budgets,
    /// checked at each safe point (`cfo::budgets::Watch`).
    cfo: Option<crate::cfo::budgets::Watch>,
}

/// One attempt's stream state. A retried attempt starts a new one, so
/// nothing the failed resume parsed (its native id, envelope, question-card
/// tracking or text) carries over.
struct TurnStream {
    /// Captured only at an explicit provider proposal event, never from prose.
    proposed_plan: Option<String>,
    state: agent::ParseState,
    results: crate::result_kind::ResultTracker,
    reported_failure: bool,
    /// The assistant's text so far, for the transcript and learning.
    assistant: String,
    reasoning: String,
    /// tool_use id → name, so a teamwork delegation's images surface.
    tool_names: HashMap<String, String>,
    /// The last tool the attempt called, for a pause's note (P1a).
    last_tool: Option<String>,
    /// P1d delta review: the model itself replied (a real text or delta,
    /// never an API-error envelope such as a provider's limit text).
    model_replied: bool,
    /// Question-card calls (Paperclip PR 13), per attempt, seeded with the
    /// native id this attempt resumes (none for a fresh one).
    asks: crate::interaction_schema::AskRecorder,
    probation: Probation,
    /// Only ever an exact id this adapter announced (or the one resumed).
    last_session_id: Option<String>,
    /// The id this attempt's own stream announced; `None` until it does,
    /// unlike `last_session_id`, which starts as the resumed id (G2 review:
    /// only an announced id records what a native session holds).
    announced: Option<String>,
    /// The attempt's last `rate_limit_event` (R1: its own account's state).
    last_quota: Option<crate::claude_quota::StreamQuota>,
    quota_notice_sent: bool,
    /// R1: while this attempt could still move to another account, its
    /// error envelope (an API-error message, an error `result`) waits here
    /// until content follows it or the attempt ends, so a turn that fails
    /// over never shows the dead account's error. Always empty otherwise.
    defer: bool,
    /// AG4b: this attempt may end on a deferred tool call (an ask-first
    /// Claude turn with the hook): its clean `result` envelope waits in
    /// `deferred` like an error's, and the call is kept in `deferred_call`.
    defer_tools: bool,
    deferred_call: Option<crate::claude_defer::DeferredCall>,
    deferred: Vec<Deferred>,
    /// Turn guards (2026-10-09): the attempt's open tool calls, so it is
    /// only ever ended at a safe point (`chat_resume::SafePoints`).
    safe: crate::chat_resume::SafePoints,
    /// The attempt posted a question card: never moved at the ceiling.
    posted_card: bool,
    /// The attempt's running cost estimate (night turns only).
    estimate: f64,
    /// Why a guard ended the attempt at a safe point, once.
    yielded: Option<Yield>,
}

/// Why a turn guard ended a running attempt at a safe point.
#[derive(Debug, Clone, PartialEq)]
enum Yield {
    /// The account passed the Spread ceiling and `to` has room: resume the
    /// same native session there (no further proactive ceiling move).
    Ceiling { to: String },
    /// Tonight's committed spend plus every running night turn's estimate
    /// reached the overnight stop.
    Night { total: f64, cap: f64 },
    /// COO rebuild L9: the subagent (or its project) reached its CFO budget;
    /// the turn ends with this pause note.
    Budget { note: String },
    /// P1a: the chat was asked to pause (`chat_pause`); the turn ends
    /// keeping its native session, to resume later.
    Pause,
}

/// An unattended night turn's cost guard (turn guards): the process-wide
/// ledger, this turn's conversation and its run id.
pub(crate) struct NightWatch<'a> {
    pub spend: &'a crate::night_queue::NightSpend,
    pub key: &'a str,
    pub run: &'a str,
}

/// What an attempt that may fail over holds back (R1, [`TurnStream::defer`]).
enum Deferred {
    Held(Held),
    /// Text for the assistant buffer (`note_assistant_text`).
    Text(String),
    /// A result's text, used only when the buffer is still empty.
    ResultText(String),
}

impl TurnStream {
    fn new(resume: Option<String>, holding: bool) -> Self {
        TurnStream {
            proposed_plan: None,
            state: agent::ParseState::default(),
            results: crate::result_kind::ResultTracker::default(),
            reported_failure: false,
            assistant: String::new(),
            reasoning: String::new(),
            tool_names: HashMap::new(),
            last_tool: None,
            model_replied: false,
            asks: crate::interaction_schema::AskRecorder::new(resume.clone()),
            probation: Probation::new(holding),
            last_session_id: resume,
            announced: None,
            last_quota: None,
            quota_notice_sent: false,
            defer: false,
            defer_tools: false,
            deferred_call: None,
            deferred: Vec::new(),
            safe: crate::chat_resume::SafePoints::default(),
            posted_card: false,
            estimate: 0.0,
            yielded: None,
        }
    }

    /// Whether this attempt's `result` envelope waits in `deferred`: an
    /// error's does, and so does a clean one that ended on a gated tool call
    /// (AG4b), which is kept in `deferred_call` for the card and dropped if
    /// the turn resumes.
    fn hold_result(&mut self, is_error: bool, line: &str) -> bool {
        if self.defer_tools && !is_error {
            self.deferred_call = crate::claude_defer::deferred_call(line);
        }
        is_error || self.deferred_call.is_some()
    }

    /// [`Self::hold`], or keep `item` back while the attempt may fail over
    /// and `deferrable` (it belongs to an error envelope).
    fn hold_or_defer<S: ChatSink + ?Sized>(
        &mut self,
        cx: &TurnCx<'_>,
        sink: &S,
        item: Held,
        deferrable: bool,
    ) {
        if deferrable && self.defer {
            self.deferred.push(Deferred::Held(item));
        } else {
            self.hold(cx, sink, item);
        }
    }

    /// Everything deferred, now, in order: the envelope was not the end
    /// (content followed it) or the turn is not moving.
    fn flush_deferred<S: ChatSink + ?Sized>(&mut self, cx: &TurnCx<'_>, sink: &S) {
        for item in std::mem::take(&mut self.deferred) {
            match item {
                Deferred::Held(item) => self.hold(cx, sink, item),
                Deferred::Text(text) => note_assistant_text(&mut self.assistant, &text),
                Deferred::ResultText(text) => {
                    if self.assistant.trim().is_empty() {
                        self.assistant.push_str(&text);
                    }
                }
            }
        }
    }

    /// Perform `item` now, or hold it while the attempt is on probation.
    fn hold<S: ChatSink + ?Sized>(&mut self, cx: &TurnCx<'_>, sink: &S, item: Held) {
        if let Some(item) = self.probation.push(item) {
            perform(cx, sink, item);
        }
    }

    /// End probation: perform everything held, in order.
    fn commit<S: ChatSink + ?Sized>(&mut self, cx: &TurnCx<'_>, sink: &S) {
        for item in self.probation.commit() {
            perform(cx, sink, item);
        }
    }
}

fn perform<S: ChatSink + ?Sized>(cx: &TurnCx<'_>, sink: &S, item: Held) {
    match item {
        Held::Emit(payload) => sink.emit(payload),
        Held::Store { kind, payload } => {
            let _ = crate::session_store::append_event(
                cx.vault,
                cx.session_key,
                cx.turn_ord,
                kind,
                &payload,
            );
        }
        Held::HarnessId(id) => {
            let _ = crate::session_store::record_harness_stamp(
                cx.vault,
                cx.session_key,
                &id,
                cx.fingerprint,
                cx.config_parts,
            );
        }
    }
}

/// Content ends an attempt's probation: after it, the resume demonstrably
/// worked (and a tool event means the contract's "no tool events ran" no
/// longer holds). An API-error message and the result never commit.
fn commits_attempt(evt: &AgentEvent) -> bool {
    matches!(
        evt,
        AgentEvent::TurnBoundary
            | AgentEvent::AssistantDelta(_)
            | AgentEvent::AssistantText(_)
            | AgentEvent::ToolUse { .. }
            | AgentEvent::ToolResult { .. }
    )
}

/// Append a complete assistant text block to the turn's text, once.
fn note_assistant_text(buffer: &mut String, text: &str) {
    if !text.trim().is_empty() && !buffer.ends_with(text) {
        if !buffer.is_empty() {
            buffer.push('\n');
        }
        buffer.push_str(text);
    }
}

/// Move the conversation to a fresh native session: close the stint with
/// `reason` and open a new one (expiring the old session's open question
/// cards in the same transaction and emitting them, cross-m34 H4), record a
/// `sessionReset` trace event, and emit `sessionReset`, on which the
/// frontend clears its native id and those cards. A config-changed reset
/// names the fingerprint parts that changed (`changed`, OB03:
/// `chat_resume::PARTS`, or `unknown`). Failures only log.
fn announce_reset<S: ChatSink + ?Sized>(sink: &S, cx: &TurnCx<'_>, reason: ResetReason) {
    cx.reset_announced
        .store(true, std::sync::atomic::Ordering::Relaxed);
    match crate::session_store::restart_stint(
        cx.vault,
        cx.session_key,
        cx.agent.as_str(),
        reason.as_str(),
    ) {
        Ok(expired) => {
            for row in &expired {
                sink.emit(interactions::event_payload(row));
            }
        }
        Err(error) => eprintln!("[chat] could not start a fresh stint: {error}"),
    }
    let changed = (reason == ResetReason::ConfigChanged).then(|| {
        cx.reset_parts
            .iter()
            .map(|part| part.to_string())
            .collect::<Vec<String>>()
    });
    // Overnight 9 A9: "away 3 h at 714K", for the notice.
    let away = match reason {
        ResetReason::Rested {
            away_minutes,
            tokens,
        } => crate::chat_resume::rested_text(away_minutes, tokens),
        _ => None,
    };
    let payload = crate::chat_event::build::session_reset(
        reason.wire(),
        cx.agent.as_str(),
        cx.session_key,
        changed,
        away,
    );
    let _ = crate::session_store::append_event(
        cx.vault,
        cx.session_key,
        cx.turn_ord,
        "sessionReset",
        &payload.to_string(),
    );
    sink.emit(payload);
}

/// Who asked for a chat turn, for the CFO (COO rebuild L7): the person, a
/// check-in or wake Hollow queued itself (`host_asker`), another agent
/// (`hollow_app` start or send, a subagent), else unattended work.
fn turn_origin(
    person_sent: bool,
    host_asker: Option<&str>,
    app_depth: u32,
    subagent: bool,
) -> crate::cfo::Origin {
    use crate::cfo::Origin;
    if person_sent {
        Origin::Person
    } else if host_asker.is_some_and(|asker| !asker.is_empty()) {
        Origin::CheckIn
    } else if subagent || app_depth > 0 {
        Origin::Subagent
    } else {
        Origin::Automation
    }
}

/// The effort a turn asks the CFO for: the one its send named (the
/// composer's, an agent's or the phone's), else the chat's own (kept in the
/// session store, so a run Hollow starts itself, an automation in the chat,
/// a wake-up, a continuation, a handover successor, a resume after a restart
/// or a phone send, uses it too), else `None`: the CFO's Fixed or Automatic
/// level. Resolved when the turn runs, so a level changed while the turn
/// waited applies.
pub(crate) fn resolve_requested_effort(
    vault: &Path,
    session_key: &str,
    named: Option<String>,
) -> Option<String> {
    named.filter(|level| !level.trim().is_empty()).or_else(|| {
        crate::session_store::chat_effort(vault, session_key)
            .ok()
            .flatten()
    })
}

/// The CFO's decision for a chat turn and the added Claude account it would
/// put unattended work on (COO rebuild L7). `asked` is the person's own text
/// (or the agent's task), never the composed prompt.
#[allow(clippy::too_many_arguments)]
fn turn_cfo(
    vault: &Path,
    agent: AgentKind,
    model: Option<&str>,
    asked: &str,
    origin: crate::cfo::Origin,
    build: bool,
    requested_effort: Option<String>,
    saved_effort: Option<String>,
    placement: Option<String>,
) -> (crate::cfo::Decision, Option<String>) {
    let settings = crate::cfo::settings_for(Some(vault));
    // The accounts this turn may start on: the CFO picks among the same ones
    // the run pump held it on (review H1).
    let readings = if origin.attended() {
        Vec::new()
    } else {
        crate::cfo::accounts::live_pool(&crate::cfo::accounts::readings(), agent.as_str())
    };
    crate::cfo::decide_turn(
        &settings,
        &readings,
        crate::cfo::TurnAsk {
            agent,
            model,
            prompt: asked,
            origin,
            build,
            requested_effort,
            placement,
        },
        saved_effort,
    )
}

/// The receipt's effort line for a turn about to run `spec` on `agent`:
/// the requested level and the one [`crate::effort::resolve`] sends, the
/// same call `agent::build_command` makes. `None` when nothing was asked for.
fn turn_effort(agent: AgentKind, spec: &agent::RunSpec) -> Option<crate::evidence::ReceiptEffort> {
    // A self-hosted run always sends one (Q3: `medium` when none is saved).
    let requested = match spec.extras.effort.as_deref() {
        Some(level) => level.trim().to_string(),
        None if spec.extras.self_hosted.is_some() => "default".to_string(),
        None => return None,
    };
    let choice = agent::run_effort(agent, spec);
    Some(crate::evidence::ReceiptEffort {
        requested,
        sent: choice.sent(),
        note: choice.note,
    })
}

/// A Claude turn's agent started beside its preparation (run 15 S2): the
/// CLI spends most of a second on its own start-up (its settings, the
/// person's SessionStart hooks, plugins, its MCP servers) before it reads
/// the first stdin line, and a steerable turn's prompt is exactly that line,
/// so its command can be built and spawned as soon as connector selection
/// and the repository snapshot have settled, while retrieval still runs.
/// The first attempt adopts the child instead of building and spawning
/// (the spec it was built from is the turn's own, less the prompt, which
/// the command never holds: [`early_start_fits`]). Unadopted (preparation
/// failed, a stop came first), dropping it drops its [`GroupGuard`], which
/// SIGKILLs the process tree and releases the run's broker binding.
pub(crate) struct EarlyStart {
    cmd: Command,
    child: tokio::process::Child,
    guard: GroupGuard,
}

/// Whether `spec` can start early: a steerable Claude turn, whose command
/// carries no prompt (`agent::live_claude_input`: it reads stdin).
fn early_start_fits(agent: AgentKind, spec: &agent::RunSpec) -> bool {
    agent == AgentKind::Claude && agent::live_claude_input(spec)
}

impl EarlyStart {
    /// Build `spec`'s command with `build` (the first attempt's builder, so
    /// its timing mark is the first command's) and spawn it, its stdin piped
    /// for the prompt. `None` when the spec doesn't fit, the build gave the
    /// unavailable stand-in, or the spawn failed: the first attempt then
    /// builds and spawns as before (and reports a failure itself).
    fn start(
        agent: AgentKind,
        spec: &agent::RunSpec,
        run_id: &str,
        build: impl FnOnce(&agent::RunSpec) -> Command,
    ) -> Option<Self> {
        if !early_start_fits(agent, spec) {
            return None;
        }
        let cmd = build(spec);
        if agent::is_unavailable_command(&cmd) {
            return None;
        }
        Self::spawn(cmd, run_id)
    }

    /// Spawn an already built command as the turn's early start.
    fn spawn(mut cmd: Command, run_id: &str) -> Option<Self> {
        cmd.stdin(Stdio::piped());
        let child = cmd.spawn().ok()?;
        let guard = GroupGuard::adopt_run(&cmd, &child, Some(run_id));
        Some(EarlyStart { cmd, child, guard })
    }

    /// End an agent that never got its prompt, and wait until its process
    /// tree is gone (review S2 F2): before the turn's changes are captured
    /// and its lease released, never after. How the cleanup went goes on
    /// the receipt with the run's other stops (`add_process_stop`).
    async fn end(self, timings: &Timings) -> Option<process_group::StopOutcome> {
        let EarlyStart {
            mut child,
            mut guard,
            ..
        } = self;
        guard.settle(&mut child, timings).await
    }
}

/// How [`drive_turn_attempts`] ended.
enum AttemptsEnd {
    /// A stop was requested before the first spawn.
    StoppedBeforeSpawn,
    /// An attempt failed to spawn. `session_id` is what `done` may carry.
    SpawnFailed {
        message: String,
        session_id: Option<String>,
    },
    /// The resume failed and would have been retried, but a stop was
    /// requested first: no second spawn. `stream` still holds the failed
    /// attempt's announcements, which are dropped with it.
    StoppedBeforeRetry {
        driven: process_group::Driven,
        stream: TurnStream,
    },
    /// The last attempt ran (`retried`: the fresh retry ran). A failover
    /// (R1) is not a retry: it resumes the same native session.
    Driven {
        driven: process_group::Driven,
        stream: TurnStream,
        agent_runs: bool,
        retried: bool,
        /// R1: the account ran out and the turn could not move; the error the
        /// turn ends with instead of the exit's own.
        failover_stop: Option<String>,
        /// Turn guards: the night guard ended the turn at a safe point, at
        /// this total of that cap.
        night_stop: Option<(f64, f64)>,
        /// L9: the CFO budget ended the turn at a safe point, with this note.
        budget_stop: Option<String>,
        /// P1a: the chat paused at a safe point; what it was doing.
        paused: Option<String>,
    },
}

impl AttemptsEnd {
    /// P1a: a paused attempt's note and the native session its stream named.
    fn paused(&self) -> Option<(&str, Option<&str>)> {
        match self {
            AttemptsEnd::Driven {
                paused: Some(note),
                stream,
                ..
            } => Some((note.as_str(), stream.last_session_id.as_deref())),
            _ => None,
        }
    }
}

/// What the attempt loop asks about Claude accounts (R1, quota failover):
/// [`LiveAccounts`] in production, a stub in tests.
trait FailoverAccounts {
    async fn refresh(&self) {}
    fn reject(&self, _account: &str, _reset: Option<i64>) -> Option<String> {
        None
    }
    /// The added account a built command runs on; `None` for Claude Code's
    /// login or while no account is added.
    fn account_of(&self, spec: &agent::RunSpec, cmd: &Command) -> Option<String>;
    /// The Settings toggle (and added accounts exist).
    fn enabled(&self) -> bool;
    /// Turn guards: Spread usage with the toggle on (and added accounts), so
    /// a running turn may move at the ceiling. Never in Single mode.
    fn ceiling_switch_enabled(&self) -> bool;
    /// Whether a run's own `rate_limit_event` puts its account out of room.
    fn stream_exhausted(&self, quota: &crate::claude_quota::StreamQuota) -> bool;
    /// One fresh probe; `None` when it could not tell.
    fn probe_exhausted(&self, account: &str) -> impl std::future::Future<Output = Option<bool>>;
    /// The eligible account to move to, never one in `exclude`.
    fn next(&self, exclude: &[String]) -> Option<String>;
    fn label(&self, account: &str) -> String;
    /// The soonest time an account has room again (unix seconds).
    fn soonest_room(&self) -> Option<i64>;
    /// Login override: the built command runs on Claude Code's own login
    /// while added accounts exist (a claude.ai connector put it there).
    fn on_login(&self, _cmd: &Command) -> bool {
        false
    }
    /// Login override: the login's own `rate_limit_event` puts it past its
    /// line while an added account has room, the first time for that window.
    fn login_warning(
        &self,
        _quota: &crate::claude_quota::StreamQuota,
    ) -> Option<crate::claude_accounts::LoginLow> {
        None
    }
}

/// The GUI's accounts (`claude_accounts`).
struct LiveAccounts;

impl FailoverAccounts for LiveAccounts {
    async fn refresh(&self) {
        crate::claude_accounts::refresh_before_pick().await;
    }
    fn reject(&self, account: &str, reset: Option<i64>) -> Option<String> {
        crate::claude_accounts::reject_account(account, reset)
    }
    fn account_of(&self, _spec: &agent::RunSpec, cmd: &Command) -> Option<String> {
        crate::claude_accounts::account_of(cmd).filter(|id| id != crate::claude_accounts::LOGIN_ID)
    }
    fn enabled(&self) -> bool {
        crate::claude_accounts::failover_enabled()
    }
    fn ceiling_switch_enabled(&self) -> bool {
        crate::claude_accounts::ceiling_switch_enabled()
    }
    fn stream_exhausted(&self, quota: &crate::claude_quota::StreamQuota) -> bool {
        crate::claude_accounts::stream_exhausted(quota)
    }
    async fn probe_exhausted(&self, account: &str) -> Option<bool> {
        crate::claude_accounts::probe_exhausted(account).await
    }
    fn next(&self, exclude: &[String]) -> Option<String> {
        crate::claude_accounts::failover_target(exclude)
    }
    fn label(&self, account: &str) -> String {
        crate::claude_accounts::label(account)
    }
    fn soonest_room(&self) -> Option<i64> {
        crate::claude_accounts::soonest_room()
    }
    fn on_login(&self, cmd: &Command) -> bool {
        crate::claude_accounts::account_of(cmd).as_deref() == Some(crate::claude_accounts::LOGIN_ID)
            && crate::claude_accounts::has_added()
    }
    fn login_warning(
        &self,
        quota: &crate::claude_quota::StreamQuota,
    ) -> Option<crate::claude_accounts::LoginLow> {
        crate::claude_accounts::login_low_now(&quota.readings())
            .filter(crate::claude_accounts::first_login_warning)
    }
}

/// Why the attempt loop spawns again after a failed attempt.
enum Next {
    /// The resume failed: a fresh session (Paperclip PR 14b).
    Fresh,
    /// The account ran out: the same session on `to` (R1).
    Failover {
        session: String,
        to: String,
        from_label: String,
        to_label: String,
        reason: crate::chat_resume::SwitchReason,
    },
    /// A provider outage (overnight 9 A3): the same native session again,
    /// once per turn, after waiting `waited` seconds ([`backoff_wait`]).
    Backoff { session: String, waited: u64 },
    /// AG4b: a gated tool call was deferred and its card answered: the same
    /// native session again, which the hook now answers for that call.
    Deferred { session: String },
}

/// How long a turn waits before its one resume after a provider outage
/// (overnight 9 A3): 60–120 s, jittered so lanes that failed together
/// don't come back together. Tests wait 50 ms.
fn backoff_wait() -> Duration {
    if cfg!(test) {
        return Duration::from_millis(50);
    }
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    backoff_wait_from(seed)
}

fn backoff_wait_from(seed: u32) -> Duration {
    Duration::from_secs(60 + u64::from(seed % 61))
}

/// When a turn that ran out has room again (repo ideas 10-27 #8, P1d): the
/// soonest added account's reset after R1's stop (`soonest`), else the
/// attempt's own last rate-limit event, else the reset the provider's error
/// text carries (`said`: the turn's error, then its result's own text, P1d,
/// since a failed exit's error is its stderr). Unix seconds; `None` when
/// nothing says.
pub(crate) fn limit_reset_at(
    r1_stopped: bool,
    soonest: impl FnOnce() -> Option<i64>,
    last_quota: Option<&crate::claude_quota::StreamQuota>,
    said: &[Option<&str>],
    now: i64,
) -> Option<i64> {
    r1_stopped
        .then(soonest)
        .flatten()
        .or_else(|| last_quota.and_then(|quota| crate::agent_out::stream_reset(quota, now)))
        .or_else(|| {
            said.iter()
                .flatten()
                .find_map(|text| crate::agent_out::said_reset(text, now))
        })
}

/// R1: whether a failed Claude attempt moves to another account. Reads the
/// attempt's result kind and outage signal, its own last `rate_limit_event`
/// and, when that does not settle it, one fresh probe, then
/// [`crate::chat_resume::decide_failover`]. `tried` holds the accounts this
/// turn already ran out on, the current one last. Returns the next step
/// and, for a stop, its message.
async fn judge_failover<A: FailoverAccounts>(
    accounts: &A,
    driven: &process_group::Driven,
    stream: &TurnStream,
    tried: &[String],
    evidence: &mut crate::evidence::EvidenceCollector,
) -> (Failover, Option<String>) {
    let Some(Ok(status)) = driven.status.as_ref() else {
        return (Failover::No, None);
    };
    let Some(account) = tried.last() else {
        return (Failover::No, None);
    };
    let record = stream.results.record();
    let kind = crate::result_kind::classify(&crate::result_kind::ResultFacts {
        agent: AgentKind::Claude,
        ended: driven.ended,
        exit_success: status.success(),
        result: record.as_ref(),
        stderr_tail: &driven.stderr_tail,
        budget_stop: false,
        stalled_minutes: None,
    });
    let outage = crate::result_kind::is_outage(record.as_ref(), &driven.stderr_tail);
    if kind == crate::result_kind::ResultKind::ProviderQuota && !outage {
        let reset = [
            record.as_ref().and_then(|r| r.text.as_deref()),
            Some(driven.stderr_tail.as_str()),
        ]
        .into_iter()
        .flatten()
        .find_map(|text| crate::agent_out::said_reset(text, Utc::now().timestamp()))
        .or_else(|| {
            limit_reset_at(
                false,
                || None,
                stream.last_quota.as_ref(),
                &[
                    record.as_ref().and_then(|r| r.text.as_deref()),
                    Some(&driven.stderr_tail),
                ],
                Utc::now().timestamp(),
            )
        });
        if let Some(reason) = accounts.reject(account, reset) {
            evidence.record_account_choice(reason);
        }
        accounts.refresh().await;
    }
    let mut facts = FailoverFacts {
        enabled: accounts.enabled(),
        account: Some(account.as_str()),
        kind,
        outage,
        // A resumed id is not proof this attempt received its prompt. Never
        // continue an earlier turn after a pre-init rejection on stderr.
        session: stream.announced.as_deref().or_else(|| {
            // Pre-init rejection retries the exact current request; never
            // continue an inherited session that did not accept this turn.
            (kind == crate::result_kind::ResultKind::ProviderQuota
                && !stream.model_replied
                && stream.tool_names.is_empty()
                && stream.last_tool.is_none()
                && !stream.posted_card)
                .then_some("unaccepted-request")
        }),
        stream: stream
            .last_quota
            .as_ref()
            .map(|quota| Exhaustion::from(Some(accounts.stream_exhausted(quota))))
            .unwrap_or(Exhaustion::Unknown),
        probe: None,
        tried,
    };
    if kind == crate::result_kind::ResultKind::Transient
        && facts.stream == Exhaustion::Exhausted
        && !facts.outage
    {
        if let Some(reason) = accounts.reject(
            account,
            limit_reset_at(
                false,
                || None,
                stream.last_quota.as_ref(),
                &[],
                Utc::now().timestamp(),
            ),
        ) {
            evidence.record_account_choice(reason);
        }
        accounts.refresh().await;
    }
    let mut decision = crate::chat_resume::decide_failover(&facts, || accounts.next(tried));
    if decision == Failover::Probe {
        facts.probe = Some(Exhaustion::from(accounts.probe_exhausted(account).await));
        if facts.probe == Some(Exhaustion::Exhausted) || facts.stream == Exhaustion::Exhausted {
            if let Some(reason) = accounts.reject(account, None) {
                evidence.record_account_choice(reason);
            }
            accounts.refresh().await;
        }
        decision = crate::chat_resume::decide_failover(&facts, || accounts.next(tried));
    }
    let message = match &decision {
        Failover::Stop(stop) => {
            let resets = accounts
                .soonest_room()
                .and_then(|at| crate::chat_resume::reset_label(at, &chrono::Local::now()));
            Some(crate::chat_resume::failover_stop_message(
                *stop,
                resets.as_deref(),
            ))
        }
        _ => None,
    };
    (decision, message)
}

/// The quiet transcript row of a failover (R1) or a ceiling move (turn
/// guards): recorded in the turn's trace and emitted as `accountSwitched
/// {from, to, reason, sessionKey}` (labels; `reason` is `limit` or
/// `ceiling`).
/// Steer now (mid-turn steering, 2026-10-21): hand the person's `text` to
/// the running turn of `session_key` and record it as their message under
/// that turn. Returns the `steer` chat-event (tagged like the turn's own)
/// for the caller to emit, or a `steering::STEER_*` code. The UI's
/// `chat_steer` command and the COO's `sessions.steer_session` action (on a
/// chat it started, COO parity L13) reach it; no agent tool steers. Only
/// tests call this person-only form; the live paths call [`accept_steer_by`].
#[cfg(test)]
pub(crate) fn accept_steer(
    session_key: &str,
    text: &str,
    id: &str,
    now: DateTime<Utc>,
) -> Result<Value, &'static str> {
    accept_steer_by(session_key, text, id, now, crate::steering::SteerBy::Person)
}

/// [`accept_steer`] from `by`: the person's composer stores `by: person`;
/// the COO's `sessions.steer_session` (L13 review) stores `by: agent` with
/// its run, so the chat's task state never reads it as the person's.
pub(crate) fn accept_steer_by(
    session_key: &str,
    text: &str,
    id: &str,
    now: DateTime<Utc>,
    by: crate::steering::SteerBy<'_>,
) -> Result<Value, &'static str> {
    if !crate::agent_sessions::is_safe_session_key(session_key) {
        return Err(crate::steering::STEER_NOT_RUNNING);
    }
    if !crate::steering::is_safe_id(id) {
        return Err(crate::steering::STEER_INVALID);
    }
    let accepted = crate::steering::steer_by(session_key, text, id, now, by)?;
    let mut stored = serde_json::to_value(&accepted.record).unwrap_or_else(|_| json!({}));
    if let Some(object) = stored.as_object_mut() {
        object.insert("by".into(), json!(by.as_str()));
    }
    // Stored first: the agent never gets a message the transcript lacks.
    if crate::session_store::append_event(
        &accepted.vault,
        session_key,
        accepted.turn_ord,
        "steer",
        &stored.to_string(),
    )
    .is_err()
    {
        crate::steering::forget(session_key, id);
        return Err(crate::steering::STEER_NOT_STORED);
    }
    if let Err(code) = crate::steering::publish(session_key, id) {
        // The turn closed its input meanwhile: the stored record says the
        // agent never got it, and the composer sends it as the next message.
        let _ = crate::session_store::append_event(
            &accepted.vault,
            session_key,
            accepted.turn_ord,
            "steerState",
            &json!({ "id": id, "state": "notDelivered" }).to_string(),
        );
        return Err(code);
    }
    Ok(crate::steering::tagged(
        crate::steering::steer_event(session_key, &accepted.record),
        &accepted.tag,
    ))
}

/// Write a steerable attempt's input lines until it's told to close (or
/// the process stops reading); dropping `stdin` is the CLI's EOF. A Codex
/// attempt's steer ([`crate::steering::PipeMsg::Steer`]) becomes the
/// `turn/steer` its `codex` session writes (held there until the turn id is
/// known, then written here on `HeldSteers` or before the next steer, so
/// steers keep their order: this task is the only one that writes them);
/// one it refuses (the turn ended) writes nothing and stays `sent` for the
/// next attempt or `steering::settle`.
async fn write_live_input(
    mut stdin: tokio::process::ChildStdin,
    mut lines: tokio::sync::mpsc::UnboundedReceiver<crate::steering::PipeMsg>,
    codex: Option<CodexSession>,
) {
    use tokio::io::AsyncWriteExt;
    while let Some(msg) = lines.recv().await {
        let out = match msg {
            crate::steering::PipeMsg::Line(line) => vec![line],
            crate::steering::PipeMsg::Steer { id, text } => match &codex {
                Some(session) => {
                    let mut session = session.lock().unwrap_or_else(PoisonError::into_inner);
                    session.steer(&id, &text).unwrap_or_default()
                }
                None => Vec::new(),
            },
            crate::steering::PipeMsg::HeldSteers => match &codex {
                Some(session) => session
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take_held_steers(),
                None => Vec::new(),
            },
            crate::steering::PipeMsg::Close => break,
        };
        for line in out {
            let mut bytes = line.into_bytes();
            bytes.push(b'\n');
            if stdin.write_all(&bytes).await.is_err() || stdin.flush().await.is_err() {
                return;
            }
        }
    }
}

/// A Codex attempt's `codex app-server` session (steering PR 3), shared by
/// the attempt's stdout reader and its input writer. A std mutex, never
/// held across an `.await`.
type CodexSession = Arc<std::sync::Mutex<crate::codex_app_server::AppSession>>;

/// What a Codex attempt's session says its steers did: delivered (its
/// message joined the turn) or refused by the server (`no active turn to
/// steer`), stored and shown like Claude's `command_lifecycle`. One the
/// turn ended before stays `sent`: a backoff's next attempt writes it
/// again, else `steering::settle` marks it not delivered as the turn ends.
fn record_codex_steers<S: ChatSink + ?Sized>(
    sink: &S,
    cx: &TurnCx<'_>,
    updates: Vec<crate::codex_app_server::SteerUpdate>,
) {
    use crate::codex_app_server::SteerUpdate;
    for update in updates {
        let moved = match update {
            SteerUpdate::Delivered { client_id } => crate::steering::mark(
                cx.session_key,
                cx.run_id,
                &client_id,
                crate::steering::SteerState::Delivered,
            ),
            SteerUpdate::Refused { client_id, reason } => {
                // The reason rides the record to the chat (P1-30).
                eprintln!("[steering] codex refused steer {client_id}: {reason}");
                crate::steering::refuse(cx.session_key, cx.run_id, &client_id, &reason)
            }
            SteerUpdate::Undelivered { .. } => Vec::new(),
        };
        record_steer_moves(sink, cx, &moved);
    }
}

/// A steer that moved (delivered, not delivered): stored and shown.
fn record_steer_moves<S: ChatSink + ?Sized>(
    sink: &S,
    cx: &TurnCx<'_>,
    moved: &[crate::steering::SteerRecord],
) {
    for steer in moved {
        let _ = crate::session_store::append_event(
            cx.vault,
            cx.session_key,
            cx.turn_ord,
            "steerState",
            &crate::steering::state_payload(steer),
        );
        sink.emit(crate::steering::steer_event(cx.session_key, steer));
    }
}

/// `queued_turn_count` on a Claude `result` line (0 when absent).
fn result_queued_turns(line: &str) -> u64 {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|v| v.get("queued_turn_count").and_then(Value::as_u64))
        .unwrap_or(0)
}

/// A Mac mini runner notice (`remoteRun`): kept in the conversation's
/// record, like an account switch, then shown.
fn record_remote_notice(
    out: &TurnEvents<'_>,
    vault: &std::path::Path,
    session_key: &str,
    turn_ord: i64,
    payload: Value,
) {
    let _ = crate::session_store::append_event(
        vault,
        session_key,
        turn_ord,
        "remoteRun",
        &payload.to_string(),
    );
    out.emit(payload);
}

/// How a remote turn's work came back, for [`record_remote_back`].
struct RemoteBack<'a> {
    host: &'a str,
    /// The worktree's return and whether its work is still on the runner.
    worktree: Option<(Result<crate::remote_session::Collected, String>, bool)>,
    folder: Option<Result<crate::remote_folder::FolderBack, String>>,
    local_folder: Option<&'a std::path::Path>,
    /// Overnight R1: an earlier Hollow's turn, brought back before this one.
    earlier: bool,
}

/// The chat notices for a remote turn's return (worktree, then folder), and
/// the folder files it wrote credited to `run_id` in History (not an earlier
/// Hollow's turn's).
fn record_remote_back(
    out: &TurnEvents<'_>,
    vault: &std::path::Path,
    session_key: &str,
    turn_ord: i64,
    run_id: &str,
    back: RemoteBack<'_>,
) {
    // An earlier Hollow's turn's files aren't this run's to undo.
    if let (Some(Ok(done)), Some(local), false) = (&back.folder, back.local_folder, back.earlier) {
        let rel = local
            .strip_prefix(vault)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !rel.is_empty() {
            let written = done
                .changed
                .iter()
                .chain(&done.merged)
                .chain(&done.copies)
                .map(|file| format!("{rel}/{file}"))
                .collect();
            crate::automated_history::note_own_writes(vault, run_id, written);
        }
    }
    for notice in remote_back_notices(&back) {
        record_remote_notice(out, vault, session_key, turn_ord, notice);
    }
}

/// [`record_remote_back`]'s notices, in order: the worktree's return, then
/// the folder's. Pure, so a self-hosted turn's note can count an earlier
/// turn's return before it is recorded (self-hosted models Q7 review).
fn remote_back_notices(back: &RemoteBack<'_>) -> Vec<Value> {
    let mut notices = Vec::new();
    if let Some((collected, still_there)) = &back.worktree {
        let mut notice = crate::remote_session::back_notice(back.host, collected);
        // Where a failed return's work is (the morning report reads it).
        if collected.is_err() {
            notice["on"] = json!(if *still_there { "runner" } else { "here" });
        }
        if back.earlier {
            notice["earlier"] = json!(true);
        }
        notices.push(notice);
    }
    if let Some(folder) = &back.folder {
        let mut notice = crate::remote_session::folder_notice(back.host, folder);
        // The earlier turn is named once, on its first notice.
        if back.earlier && back.worktree.is_none() {
            notice["earlier"] = json!(true);
        }
        notices.push(notice);
    }
    notices
}

/// The host a stale remote turn's choice (`remote_session::stale_choice`)
/// is given: where this turn goes, from its place (`turn_place`). None for
/// a turn that stays here, a self-hosted one whose Runs on is the runner
/// included (its place holds neither), so it never reattaches there.
fn stale_host(
    runs_on: &Option<Result<String, String>>,
    preferred: &Option<String>,
) -> Option<String> {
    match (runs_on, preferred) {
        (Some(Ok(host)), _) | (None, Some(host)) => Some(host.clone()),
        _ => None,
    }
}

/// What a turn records once a stale remote turn was settled, in order
/// (self-hosted models Q7 review). A self-hosted turn whose Runs on is the
/// runner (`stay`: the host it was kept from, and its run) gets its note
/// first ([`crate::self_hosted::runner_note`]), then comes the earlier
/// turn's return (`earlier`, [`remote_back_notices`]; an earlier Hollow's
/// turn writes no History entry, so its notices are all there is).
///
/// The note's `leftOnRunner`: `pending` when the stale turn's record is
/// still there after the settle (`still_stale`: the runner didn't answer,
/// or wasn't asked because it didn't answer minutes ago, so a later
/// message tries again, and its run may still be going);
/// else `runner` when, by the morning report's rule
/// (`night_queue::left_on_runner`) over the chat's start and back notices
/// (`trail`, oldest first, read only for a note) and that return, work is
/// still there that only a turn there brings back; else none.
fn settled_notices(
    stay: Option<(&str, &crate::self_hosted::TurnRun)>,
    earlier: Option<&RemoteBack<'_>>,
    still_stale: bool,
    trail: impl FnOnce() -> Vec<String>,
    session_key: &str,
) -> Vec<Value> {
    let back = earlier.map(remote_back_notices).unwrap_or_default();
    let Some((host, run)) = stay else {
        return back;
    };
    let left = if still_stale {
        Some("pending")
    } else {
        let mut events = trail();
        events.extend(back.iter().map(Value::to_string));
        crate::night_queue::left_on_runner(&events, true).map(|_| "runner")
    };
    let mut notices = vec![crate::self_hosted::runner_note(
        host,
        run,
        left,
        session_key,
    )];
    notices.extend(back);
    notices
}

fn announce_failover<S: ChatSink + ?Sized>(
    sink: &S,
    cx: &TurnCx<'_>,
    from: &str,
    to: &str,
    reason: crate::chat_resume::SwitchReason,
) {
    let payload =
        crate::chat_event::build::account_switched(from, to, reason.as_str(), cx.session_key);
    let _ = crate::session_store::append_event(
        cx.vault,
        cx.session_key,
        cx.turn_ord,
        "accountSwitched",
        &payload.to_string(),
    );
    sink.emit(payload);
}

/// Login override (2026-10-09): the receipt row saying a run went on
/// Claude Code's own login because it carries claude.ai connectors, also
/// recorded in the turn's trace as `accountLogin {connectors}`.
fn record_login_override(
    cx: &TurnCx<'_>,
    evidence: &mut crate::evidence::EvidenceCollector,
    connectors: &[String],
    remedy: Option<&str>,
) {
    evidence.record_switch(crate::evidence::login_override(connectors, remedy));
    let payload = json!({ "type": "accountLogin", "connectors": connectors });
    let _ = crate::session_store::append_event(
        cx.vault,
        cx.session_key,
        cx.turn_ord,
        "accountLogin",
        &payload.to_string(),
    );
}

/// Login override (2026-10-09): the login is past its line while an added
/// account has room. Once per window: a quiet chat row (`loginLow
/// {message}`), the turn's trace, and the bell.
fn announce_login_low<S: ChatSink + ?Sized>(
    sink: &S,
    cx: &TurnCx<'_>,
    low: &crate::claude_accounts::LoginLow,
) {
    let message = low.message();
    let payload = crate::chat_event::build::login_low(cx.session_key, &message);
    let _ = crate::session_store::append_event(
        cx.vault,
        cx.session_key,
        cx.turn_ord,
        "loginLow",
        &payload.to_string(),
    );
    sink.emit(payload);
    crate::claude_accounts::notify_login_low(cx.vault, low);
}

/// Account routing information (connector set-aside or background quota wait).
/// Spread usage across accounts (2026-10-21): connectors set to Always stepped aside
/// for this turn because the login is out of room. A quiet chat row of the
/// `loginLow` kind (the transcript replays it), once per conversation and
/// window; no bell, since it's this chat's own news.
fn announce_account_notice<S: ChatSink + ?Sized>(
    sink: &S,
    vault: &Path,
    session_key: &str,
    turn_ord: i64,
    message: &str,
) {
    let payload = crate::chat_event::build::login_low(session_key, message);
    let _ = crate::session_store::append_event(
        vault,
        session_key,
        turn_ord,
        "loginLow",
        &payload.to_string(),
    );
    sink.emit(payload);
}

/// A failed resume's fresh retry on the turn's manifest: the retry's
/// compiler record, the fallback that says so, and no `contextNote` layer
/// (R2b review), since the retry's prompt carries none.
fn fresh_retry_manifest(
    manifest: &mut crate::run_manifest::RunManifest,
    record: crate::run_manifest::CompilerRecord,
) {
    manifest.compiler = Some(record);
    manifest.layers.retain(|layer| layer.name != "contextNote");
    manifest.fallback(
        "session",
        "fresh native session after a failed resume, with the conversation rebuilt",
    );
}

/// Spawn and drive the turn's agent: once, or twice when a resume fails.
///
/// A [`ResumePlan::Resume`] attempt starts on probation. It is retried ONCE,
/// fresh, inside the same lease, history run, turn row and receipt, only when
/// it never committed (no content or tool event), its agent really ran and
/// exited on its own with stdout, no stop was requested, and
/// `result_kind::is_resume_failure` matches its envelope or stderr. The
/// retry, in order: a stop check (a stop between attempts is
/// [`AttemptsEnd::StoppedBeforeRetry`], with no side effect), the failed
/// attempt's held items discarded, its usage row finished as an error (that
/// invocation did fail; the retry begins a new row, so a retried turn is two
/// rows under one run id), [`announce_reset`] (`resumeFailed`), then the
/// spec's resume cleared and its prompt rebuilt by `retry_prompt`. Nothing
/// deletes the provider's session files.
///
/// Every hook of the single drive rides along: question cards per attempt
/// (PR 13), open cards superseded once, on the first successful spawn of the
/// person's own turn (a wake, check-in, pulse or scheduled turn leaves them
/// open), the warn-only budget guard (PR 8b), the usage row, the receipt's tool
/// evidence. `build` turns the spec into a command (`agent::build_command`
/// in production; tests pass a stub), so the spec is edited in place and
/// never rebuilt.
///
/// `activity` (Paperclip PR 15b) is the turn's activity clock and its
/// no-output watchdog (every agent since merged plan slice 3): every
/// attempt's `drive` stamps the same clock (the spawn time stays the first
/// attempt's), and each tool call sets its progress phase. A question card
/// never holds the watchdog: `ask_user_questions` returns at once and the
/// answer arrives as the user's next turn, so nothing in the attempt waits
/// on it, and its completed tool event already restamps the clock. Only a
/// pending approval card ([`crate::run_activity::CardHold`], taken in
/// `approvals`) pauses the watchdog. A stall ends the attempt as
/// `Ended::Stalled`, which is never retried (it is not an `Exited` resume
/// failure).
///
/// `deadline` (merged plan slice 3) is the turn's optional deadline, counted
/// from the start of the attempts: a retried or failed-over attempt gets
/// what is left of it, never a fresh one. Expiry ends the attempt as
/// `Ended::TimedOut`, the whole process tree ended like a stop.
///
/// Turn guards (2026-10-09) end a running attempt only at a safe point (no
/// tool call open, `chat_resume::SafePoints`), through the attempt's own
/// `Supervision::limit` (so `Ended::LimitReached`; a user's Stop wins):
/// - the **ceiling switch**: a Claude attempt on an added account whose last
///   `rate_limit_event` is at or over the Spread ceiling (overage included),
///   at its next main request after a tool result, when `chat_resume::decide_ceiling` names an
///   account with room. It resumes the same native session there like R1,
///   with `CEILING_PROMPT` and a `ceiling` switch on the receipt and the
///   transcript row. It keeps ONE usage row across both attempts (the CLI's
///   resumed terminal is session-cumulative, and only the whole session's
///   messages verify it; the row names the account the turn finished on).
///   It runs only before another switch ([`crate::chat_resume::MAX_FAILOVERS`]);
///   later terminal quota failures may still use untried accounts.
/// - the **night guard** (`night`, unattended night turns): once the night's
///   committed spend plus every running night turn's estimate reaches the
///   overnight stop, the turn ends as [`AttemptsEnd::Driven`] with
///   `night_stop`, its usage row carrying the estimate as a partial cost.
///
/// `early` (run 15 S2) is the agent [`execute_turn`] started beside its
/// preparation: the first attempt takes its command, child and group guard
/// instead of calling `build` and spawning, then writes the prompt to it as
/// it would to a fresh child. A retry or a failover builds as before. Every
/// return before the first attempt adopts it (a stop) drops it, ending it.
#[allow(clippy::too_many_arguments)]
async fn drive_turn_attempts<S, P, B, N, A>(
    sink: &S,
    cx: &TurnCx<'_>,
    stop: &StopSignal,
    timings: Timings,
    spec: &mut agent::RunSpec,
    plan: &ResumePlan,
    mut retry_prompt: P,
    mut build: B,
    new_ledger: N,
    ledger: &mut UsageRun,
    evidence: &mut crate::evidence::EvidenceCollector,
    watch: &mut TurnWatch,
    activity: Option<&process_group::Watch>,
    deadline: Option<Duration>,
    accounts: &A,
    night: Option<&NightWatch<'_>>,
    mut early: Option<EarlyStart>,
) -> AttemptsEnd
where
    S: ChatSink + ?Sized,
    P: FnMut() -> (String, crate::run_manifest::CompilerRecord),
    B: FnMut(&agent::RunSpec) -> Command,
    N: Fn() -> UsageRun,
    A: FailoverAccounts,
{
    let agent = cx.agent;
    let agent_name = agent.as_str();
    // S27 review: Antigravity's stream carries no tool calls (plan completion
    // L2, as in workflows), so its receipt's effects and its reply's claim
    // flags are unknown, never "none".
    if matches!(agent, AgentKind::Antigravity) {
        evidence.mark_tool_calls_unreported();
    }
    let resumed = match plan {
        ResumePlan::Resume(id) => Some(id.clone()),
        ResumePlan::New | ResumePlan::Reset(_) => None,
    };
    let mut previous: Option<(process_group::Driven, TurnStream, Next)> = None;
    // The turn's deadline is one budget across its attempts.
    let deadline_at = deadline.map(|limit| tokio::time::Instant::now() + limit);
    let mut superseded = false;
    let mut attempt: u32 = 0;
    let mut retried = false;
    // R1: the accounts this turn ran out on, and how often it moved.
    let mut tried: Vec<String> = Vec::new();
    let mut failovers: u32 = 0;
    // Overnight 9 A3: resumes after a provider outage, at most one.
    let mut backoffs: u32 = 0;
    // AG4b: the cards this turn has asked for deferred Claude tool calls.
    let mut deferrals: u32 = 0;
    // OB11: the turn's live context for `sessions.context`, by run id; the
    // guard takes the entry away on every return. R2a: it starts from the
    // growth step the conversation was already told.
    // Q3: a self-hosted model's window and levels are known before its
    // turn registers, under the model id its CLI is given.
    if let Some(target) = spec.extras.self_hosted.as_ref() {
        crate::self_hosted::remember_for(
            agent,
            &target.served_model,
            target.context_window,
            &target.effort_levels,
        );
    }
    let gauge_model = spec
        .extras
        .self_hosted
        .as_ref()
        .map(|t| t.served_model.clone())
        .or_else(|| spec.model.clone());
    let gauge = crate::context_gauge::register(
        cx.run_id,
        agent,
        gauge_model.as_deref(),
        cx.quiet,
        crate::context_gauge::announced_step(cx.vault, cx.session_key),
    );
    // S02 review: a compaction in an attempt a failover left behind still
    // happened to the session the next attempt resumes.
    let mut compacted = false;
    // OB22: and its count, which the next attempt's stream continues from.
    let mut compactions: u32 = 0;
    let capture_limit = agent::compact_window(agent, spec).map(u64::from);
    let mut capture_requested = false;
    loop {
        match previous.as_ref().map(|(_, _, next)| next) {
            Some(Next::Fresh) => {
                spec.resume = None;
                // R2b: a fresh session starts with no stored figure, and
                // was told no step (the live reading's too).
                crate::context_gauge::reset_steps(cx.vault, cx.session_key);
                gauge.reset_announced();
                let (prompt, record) = retry_prompt();
                spec.prompt = prompt;
                fresh_retry_manifest(evidence.context_mut(), record);
                // A16: the failed attempt's text is gone, so its reply mark
                // would read past a shorter retry reply (no claim, no
                // cites). Not on a failover: it carries the earlier text.
                evidence.mark_reply_start(0);
            }
            // The same native session on the next account, told only to go
            // on: the session already holds the user's message.
            Some(Next::Failover {
                session,
                to,
                reason,
                ..
            }) => {
                if !session.is_empty() {
                    spec.resume = Some(session.clone());
                    spec.prompt = reason.prompt().to_string();
                }
                spec.extras.account = Some(to.clone());
            }
            Some(Next::Backoff { session, .. }) => {
                spec.resume = Some(session.clone());
                spec.prompt = crate::chat_resume::BACKOFF_PROMPT.to_string();
            }
            Some(Next::Deferred { session }) => {
                spec.resume = Some(session.clone());
                spec.prompt = DEFER_PROMPT.to_string();
            }
            None => {}
        }
        // Run 15 S2: the first attempt takes the agent started beside the
        // turn's preparation (its spec is this one less the prompt, which a
        // steerable Claude command never holds); every other attempt builds.
        // Should the spec no longer fit, the early agent is ended here and
        // the attempt builds and spawns as before.
        if agent == AgentKind::Claude && attempt == 0 && early.is_none() && !stop.requested() {
            accounts.refresh().await;
        }
        let (mut cmd, mut started) = match early.take() {
            Some(EarlyStart { cmd, child, guard })
                if attempt == 0 && early_start_fits(agent, spec) =>
            {
                (cmd, Some((child, guard)))
            }
            Some(unfit) => {
                evidence.add_process_stop(unfit.end(&timings).await);
                (build(&*spec), None)
            }
            None => (build(&*spec), None),
        };
        // A build failure (validation, the Codex connector probe) returns the
        // `/bin/sh` stand-in, which spawns fine but runs no agent. Such a turn
        // neither supersedes cards nor keeps an answer it could not deliver.
        let agent_runs = !agent::is_unavailable_command(&cmd);
        if agent_runs {
            if let Some(reason) = crate::claude_accounts::pick_receipt(&cmd) {
                evidence.record_account_choice(reason);
            }
        }
        // Mid-turn steering: the agent reads this attempt's messages from
        // stdin, the prompt first (`agent::RunExtras::live_input`). For Codex
        // exactly when its command is `codex app-server` (an interactive
        // turn), since exec would block on an open stdin.
        let live_input = agent_runs
            && match agent {
                AgentKind::Codex => agent::live_codex_input(spec),
                // OC1: a Grok ACP turn speaks JSON-RPC on its stdin.
                AgentKind::Grok => agent::grok_acp(spec),
                // 2026-11-20: so does an Antigravity chat turn.
                AgentKind::Antigravity => agent::antigravity_acp(spec),
                _ => spec.extras.live_input,
            };
        if live_input {
            cmd.stdin(Stdio::piped());
        }
        if attempt == 0 {
            ledger.set_billing(crate::usage::classify(agent, &cmd, true));
            // Q8: a self-hosted run's endpoint, by name, for "billed by GPU
            // time", and its served id as the requested model.
            ledger.set_self_hosted(self_hosted_endpoint(spec));
        }
        if stop.requested() {
            // An early agent the stop overtook never gets its prompt: its
            // process tree is gone before the turn is finished (run 15 S2).
            if let Some((mut child, mut guard)) = started.take() {
                evidence.add_process_stop(guard.settle(&mut child, &timings).await);
            }
            return match previous.take() {
                None => AttemptsEnd::StoppedBeforeSpawn,
                Some((driven, stream, Next::Fresh)) => {
                    AttemptsEnd::StoppedBeforeRetry { driven, stream }
                }
                // A stop before the failover's (or the backoff's) spawn: the
                // failed attempt is the turn's outcome, shown as it would
                // have been.
                Some((
                    driven,
                    mut stream,
                    Next::Failover { .. } | Next::Backoff { .. } | Next::Deferred { .. },
                )) => {
                    stream.flush_deferred(cx, sink);
                    stream.commit(cx, sink);
                    AttemptsEnd::Driven {
                        driven,
                        stream,
                        agent_runs: true,
                        retried,
                        failover_stop: None,
                        night_stop: None,
                        budget_stop: None,
                        paused: None,
                    }
                }
            };
        }
        // What the failed attempt's text was, carried into a failover's.
        let mut carried = String::new();
        let mut carried_plan = None;
        let mut carried_reasoning = String::new();
        if let Some((_, mut failed, next)) = previous.take() {
            // A ceiling move keeps the turn's one usage row: the attempt was
            // ended before any terminal, and the resumed attempt's
            // session-cumulative terminal verifies only against both
            // attempts' messages. Otherwise the failed attempt's row closes.
            let ceiling = matches!(
                next,
                Next::Failover {
                    reason: crate::chat_resume::SwitchReason::Ceiling,
                    ..
                }
            );
            if !ceiling {
                let first = std::mem::replace(ledger, new_ledger());
                // A deferred segment ended as planned, an error otherwise.
                let usage = if matches!(next, Next::Deferred { .. }) {
                    first.finish(LedgerOutcome::Success)
                } else {
                    first.finish(LedgerOutcome::Error)
                };
                // The night guard carries the closed attempt's cost forward.
                if let Some(night) = night {
                    night.spend.close_attempt(night.run, usage.cost_usd);
                }
            }
            match next {
                Next::Fresh => {
                    // The failed resume leaves no trace in the UI or the store.
                    failed.probation.discard();
                    drop(failed);
                    retried = true;
                    announce_reset(sink, cx, ResetReason::ResumeFailed);
                }
                Next::Failover {
                    session,
                    from_label,
                    to_label,
                    reason,
                    ..
                } => {
                    // The dead account's error envelope is never shown; what
                    // the attempt did show (and still held) stays.
                    failed.deferred.clear();
                    failed.commit(cx, sink);
                    carried = std::mem::take(&mut failed.assistant);
                    carried_plan = failed.proposed_plan.take();
                    carried_reasoning = std::mem::take(&mut failed.reasoning);
                    compacted |= failed.state.compacted;
                    // It started from the count carried in, so it holds it.
                    compactions = failed.state.compactions;
                    drop(failed);
                    announce_failover(sink, cx, &from_label, &to_label, reason);
                    // Merged plan decision 6: the move goes on the receipt too.
                    evidence.record_switch(crate::evidence::ReceiptSwitch {
                        kind: crate::evidence::SWITCH_ACCOUNT.into(),
                        reason: reason.receipt(&from_label),
                        from: from_label,
                        to: to_label,
                    });
                    // A resumed Claude session's totals are session-cumulative.
                    // A ceiling move's row already knows (a fresh session's
                    // first terminal covers both attempts; a resumed one's
                    // baseline is the turn before).
                    if reason == crate::chat_resume::SwitchReason::Limit && !session.is_empty() {
                        ledger.set_resumed(Some(session));
                    }
                }
                Next::Deferred { session } => {
                    // The deferred call's empty result is never shown; what the
                    // attempt did show stays, and the resumed attempt carries
                    // its text and compactions on, as after a failover.
                    failed.deferred.clear();
                    failed.commit(cx, sink);
                    carried = std::mem::take(&mut failed.assistant);
                    carried_plan = failed.proposed_plan.take();
                    carried_reasoning = std::mem::take(&mut failed.reasoning);
                    compacted |= failed.state.compacted;
                    compactions = failed.state.compactions;
                    drop(failed);
                    ledger.set_resumed(Some(session));
                }
                Next::Backoff { session, waited } => {
                    // As a failover: the outage's error envelope is never
                    // shown, what the attempt did show stays, and the
                    // resumed attempt carries its text and compactions on.
                    failed.deferred.clear();
                    failed.commit(cx, sink);
                    carried = std::mem::take(&mut failed.assistant);
                    carried_plan = failed.proposed_plan.take();
                    carried_reasoning = std::mem::take(&mut failed.reasoning);
                    compacted |= failed.state.compacted;
                    compactions = failed.state.compactions;
                    drop(failed);
                    let label = agent.label();
                    evidence.record_switch(crate::evidence::ReceiptSwitch {
                        kind: crate::evidence::SWITCH_RETRY.into(),
                        reason: format!(
                            "Retried once after the provider was overloaded ({waited} s later, same session)"
                        ),
                        from: label.to_string(),
                        to: label.to_string(),
                    });
                    ledger.set_resumed(Some(session));
                }
            }
            ledger.set_billing(crate::usage::classify(agent, &cmd, true));
            // Q8: the fresh row keeps the self-hosted endpoint too.
            ledger.set_self_hosted(self_hosted_endpoint(spec));
        }
        let session_id = spec.resume.clone();
        // R1: the added account this attempt runs on, and whether it may
        // still move (then its error envelope waits, see `TurnStream::defer`).
        // A self-hosted run (Q1, Q5) has none, so it never fails over or
        // moves at the ceiling: it stays on its endpoint.
        let account =
            (agent == AgentKind::Claude && agent_runs && spec.extras.self_hosted.is_none())
                .then(|| accounts.account_of(spec, &cmd))
                .flatten();
        let may_fail_over = account.is_some() && accounts.enabled();
        // Overnight 9 A3: an attempt that may still back off once after a
        // provider outage holds its error envelope the same way. Not Claude:
        // its CLI retries its own overloads, and R1 owns its accounts.
        let may_back_off = agent != AgentKind::Claude && agent_runs && backoffs == 0;
        // AG4b: an ask-first Claude attempt with the defer hook may end on a
        // gated tool call that waits for the person's card.
        let may_defer = agent == AgentKind::Claude && agent_runs && agent::claude_defers(spec);
        // Login override: a run a claude.ai connector put on the login says
        // so on its receipt (first attempt; a retry is the same run).
        let on_login = agent == AgentKind::Claude && agent_runs && accounts.on_login(&cmd);
        if on_login && attempt == 0 {
            let names = agent::login_connectors(agent, spec);
            if !names.is_empty() {
                let remedy = agent::login_remedy(agent, spec);
                record_login_override(cx, evidence, &names, remedy.as_deref());
            }
        }
        let codex_profile = agent::codex_command_uses_profile(&cmd);
        #[cfg(unix)]
        let mut relay_fd = None;
        #[cfg(unix)]
        if agent::capabilities(agent).shared_server
            && live_input
            && spec.extras.remote.is_none()
            && started.is_none()
            && spec.process_scope == agent::ProcessScope::GuiOwned
        {
            if let Some((relay, fd)) = crate::shared_codex::relay(&cmd) {
                cmd = relay;
                relay_fd = Some(fd);
            }
        }
        #[cfg(unix)]
        let shared_codex_attempt = relay_fd.is_some();
        #[cfg(not(unix))]
        let shared_codex_attempt = false;
        // The early start's child and guard (run 15 S2), already running.
        let (mut child, early_guard) = match started.take() {
            Some((child, guard)) => (child, Some(guard)),
            None => match cmd.spawn() {
                Ok(child) => (child, None),
                Err(error) => {
                    return AttemptsEnd::SpawnFailed {
                        message: format!("spawn {agent_name}: {error}"),
                        session_id,
                    }
                }
            },
        };
        #[cfg(unix)]
        drop(relay_fd);
        // The attempt's input: its prompt, then any steer the turn took that
        // no attempt delivered yet (an account switch ended the one that
        // had it), then the person's steers as they come. Closed at the
        // answer's `result` (`steering::on_result`), or with the process.
        // Steering PR 3: a Codex attempt is one `codex app-server` turn,
        // whose session opens with `initialize` (the prompt, the same
        // contextual one exec gets, goes in `turn/start`; a resume is
        // `thread/resume`).
        let mut codex: Option<CodexSession> = None;
        // OC1: a Grok ACP turn's session (`acp_client`), its JSON-RPC
        // answers written back through the same input writer.
        let mut acp: Option<crate::acp_client::AcpSession> = None;
        let live_tx = match child.stdin.take().filter(|_| live_input) {
            Some(stdin) => {
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                let first = if agent == AgentKind::Grok || agent == AgentKind::Antigravity {
                    let start = if agent == AgentKind::Grok {
                        agent::grok_acp_start(spec)
                    } else {
                        agent::antigravity_acp_start(spec)
                    };
                    let (session, first) = crate::acp_client::AcpSession::start(start);
                    acp = Some(session);
                    tokio::spawn(write_live_input(stdin, rx, None));
                    first
                } else if agent == AgentKind::Codex {
                    // The thread follows the profile decision the spawned
                    // process carries, never a second worktree check.
                    let start = agent::codex_app_server_start(spec, codex_profile);
                    let (session, first) = crate::codex_app_server::AppSession::start(start);
                    let session = Arc::new(std::sync::Mutex::new(session));
                    codex = Some(Arc::clone(&session));
                    tokio::spawn(write_live_input(stdin, rx, Some(session)));
                    first
                } else {
                    tokio::spawn(write_live_input(stdin, rx, None));
                    vec![crate::steering::input_line(
                        agent,
                        &format!("hollow-turn-{}-{attempt}", cx.run_id),
                        &spec.prompt,
                    )]
                };
                for line in first {
                    let _ = tx.send(crate::steering::PipeMsg::Line(line));
                }
                // Grok has no steer (OC1 keeps it so): an ACP pipe never
                // takes a steer's raw line.
                let attempt_no = if acp.is_some() {
                    None
                } else {
                    crate::steering::attach(cx.session_key, cx.run_id, tx.clone())
                };
                Some((tx, attempt_no))
            }
            None => None,
        };
        // A protocol session (Codex app-server, Grok ACP) closes its own
        // input at its turn's end and is never a steering line reader.
        let codex_live = codex.is_some() || acp.is_some();
        ledger.launched(&cmd);
        // Paperclip PR 16: the repository baseline is usable once an agent
        // actually spawned (idempotent across a resume retry).
        evidence.arm_repo_baseline();
        let guard =
            early_guard.unwrap_or_else(|| GroupGuard::adopt_run(&cmd, &child, Some(cx.run_id)));
        // A stop that raced the spawn keeps showing as Cancelling, a pause
        // asked before it as Pausing (P1a).
        if !stop.requested() {
            let phase = if crate::chat_pause::pause_requested(cx.session_key) {
                crate::agent_sessions::SessionPhase::Pausing
            } else {
                crate::agent_sessions::SessionPhase::Running
            };
            crate::agent_sessions::set_phase(cx.run_id, phase);
        }
        // P1a review fix: the chat is no longer paused once a turn of it
        // really started (a resume, or the person's own message); a turn
        // that never spawned leaves it paused.
        if attempt == 0 {
            crate::chat_pause::on_turn_start(cx.vault, cx.session_key);
        }
        // The person's own turn is running, so this message now answers any
        // open question card that yields to free text (Paperclip PR 13). Only
        // here, after the spawn: a send that fails to start never destroys a
        // card. A wake, check-in, pulse or scheduled turn does not
        // (`closes_cards` is false), so the card stays open and answerable.
        // An answer turn's own card is already answered, so it is never
        // superseded. Nor for the build-failure stand-in, which runs no
        // agent. Once per turn: a retry supersedes nothing more.
        if !superseded && cx.turn_ord > 0 && agent_runs && cx.closes_cards {
            superseded = true;
            match interactions::supersede_for_message(cx.vault, cx.session_key, cx.turn_ord) {
                Ok(rows) => {
                    for row in &rows {
                        sink.emit(interactions::event_payload(row));
                    }
                }
                Err(error) => eprintln!("[interactions] supersede failed: {error}"),
            }
        }
        let mut stream = TurnStream::new(session_id, attempt == 0 && resumed.is_some());
        stream.defer = may_fail_over || may_back_off || may_defer;
        stream.defer_tools = may_defer;
        stream.assistant = carried;
        stream.proposed_plan = carried_plan;
        stream.reasoning = carried_reasoning;
        stream.state.compacted = compacted;
        stream.state.compactions = compactions;
        // drive() is the only waiter: it keeps delivering stdout events while it
        // ends the turn's whole process tree on stop (or sweeps leftovers on a
        // normal exit), and returns only after that, so every path releases
        // the lease after the processes are gone.
        let now = tokio::time::Instant::now();
        let remaining = deadline_at.map(|at| at.saturating_duration_since(now));
        let supervision = Supervision {
            timings,
            watch: activity.cloned(),
            deadline: remaining,
            ..turn_supervision(stop, agent_name)
        };
        let clock = activity.map(|w| Arc::clone(&w.activity));
        // Turn guards: what ends this attempt at a safe point (the ceiling
        // switch, the night guard). Ends the run like a stop, as
        // `Ended::LimitReached`; a user's Stop at the same moment wins.
        let yield_signal = StopSignal::new();
        let supervision = Supervision {
            limit: Some(&yield_signal),
            ..supervision
        };
        // The accounts a ceiling switch must not pick: those the turn ran
        // out on, and this one (last: its organisation decides, #16).
        let exclude: Vec<String> = tried.iter().cloned().chain(account.clone()).collect();
        // Background tasks the CLI cuts off as it exits (overnight A3),
        // recorded once the attempt is known to have exited on its own.
        let mut cut_tasks = crate::evidence::CutTasks::default();
        let mut rollout = crate::codex_rollout::Watch::for_spec(agent, spec);
        // One exec-style stdout line: every agent's, or each line a Codex
        // app-server session translated (below).
        let mut on_line = |line: &str| {
            cut_tasks.observe(line);
            if live_tx.is_some() && !codex_live {
                let moved = crate::steering::observe_line(cx.session_key, cx.run_id, line);
                record_steer_moves(sink, cx, &moved);
            }
            let events = agent::parse_line(agent, line, &mut stream.state);
            // AG7: the subagent this line's events belong to, if any.
            let parent = subagent_parent(line);
            let parent = parent.as_deref();
            // Whether this line is a safe point, once all its events are in.
            let safe = stream.safe.observe_line(line, &events);
            for evt in events {
                if commits_attempt(&evt) {
                    stream.flush_deferred(cx, sink);
                    stream.commit(cx, sink);
                }
                // A question card, once its completed tool event says it was
                // posted (Paperclip PR 13). Recorded before the event is shown.
                if let Some(call) = stream.asks.feed(agent, line, &evt) {
                    stream.posted_card = true;
                    interactions::persist_and_notify(
                        cx.vault,
                        &NewInteraction {
                            session_key: cx.session_key,
                            turn_ord: cx.turn_ord,
                            run_id: cx.run_id,
                            agent,
                            native_session_id: call.native_session_id.as_deref(),
                            tool_use_id: &call.tool_use_id,
                            source: InteractionSource::Mcp,
                            arguments: &call.arguments,
                            cwd: cx.run_cwd,
                            vault_root: cx.vault,
                        },
                        &mut |payload| sink.emit(payload),
                    );
                }
                let grew = ledger.observe(&evt);
                stream.results.observe(&evt);
                // Message usage only: the reading goes away with the turn,
                // so the result's totals would never be read.
                if matches!(evt, AgentEvent::MessageUsage { .. }) {
                    gauge.publish_run(ledger);
                    gauge.set_compactions(stream.state.compactions);
                }
                // Antigravity over ACP says its context and window itself.
                if let AgentEvent::ContextFigure { tokens, window } = &evt {
                    ledger.set_context_figure(*tokens, *window);
                    gauge.publish_figure(*tokens, Some(*window));
                }
                // Overnight 9 A10: Codex streams neither, so its rollout
                // file says them, read at each completed item.
                if let Some(seen) = rollout.as_mut().and_then(|watch| watch.observe(&evt)) {
                    feed_codex_rollout(seen, &mut stream.state, ledger, &gauge);
                }
                if !capture_requested && !cx.quiet {
                    let reading = crate::context_gauge::reading_json(cx.run_id);
                    let limit = capture_limit
                        .or_else(|| reading.get("advisoryTokens").and_then(Value::as_u64))
                        .or_else(|| reading.get("contextWindow").and_then(Value::as_u64));
                    if reading
                        .get("contextTokens")
                        .and_then(Value::as_u64)
                        .is_some_and(|tokens| crate::learning_capture::approaching(tokens, limit))
                    {
                        capture_requested = true;
                        crate::learning_capture::boundary(None, cx.vault, cx.session_key);
                    }
                }
                // The night guard's running estimate, published as it grows
                // so every other night turn weighs it too.
                if let (Some(night), true) = (night, grew) {
                    // Q8: 0 for a self-hosted turn (billed by GPU time).
                    stream.estimate = ledger.night_estimate();
                    let _ = night.spend.check(night.key, night.run, stream.estimate);
                }
                // The first model event, the first useful output (F11) and
                // the tool calls before it (OB08a), apart from launch and any
                // stderr, read the same way for every agent.
                if let Some(clock) = &clock {
                    clock.note_event(&evt);
                }
                if let Some(budget_guard) = watch.guard.as_mut() {
                    let running = if grew { ledger.running_total() } else { None };
                    budget_guard.bind_run(ledger.key());
                    // Warn-only: a chat intent's guard never yields a stop.
                    for notice in budget_guard.on_event(&evt, grew, running).warnings {
                        sink.emit(crate::chat_event::build::budget_warning(
                            cx.session_key,
                            &notice,
                        ));
                        watch.delivered.push(notice);
                    }
                }
                match evt {
                    AgentEvent::Reasoning(text) => {
                        if parent.is_none() && agent::capabilities(cx.agent).reasoning {
                            let text = crate::claude_runner::redact(&text);
                            if !stream.reasoning.ends_with(&text) {
                                if !stream.reasoning.is_empty() {
                                    stream.reasoning.push_str("\n\n");
                                }
                                stream.reasoning.push_str(&text);
                                stream.reasoning =
                                    crate::claude_runner::truncate(&stream.reasoning, 16_000);
                            }
                            stream.hold(
                                cx,
                                sink,
                                Held::Store {
                                    kind: "reasoning",
                                    payload: stream.reasoning.clone(),
                                },
                            );
                            stream.hold(
                                cx,
                                sink,
                                Held::Emit(crate::chat_event::build::panel(
                                    cx.session_key,
                                    "hollow:reasoning",
                                    json!({ "text": stream.reasoning, "runId": cx.run_id }),
                                )),
                            );
                        }
                    }
                    // Thinking only times the first model event (OB08a).
                    AgentEvent::MessageUsage { .. }
                    | AgentEvent::ContextFigure { .. }
                    | AgentEvent::TurnUsage(_)
                    | AgentEvent::ProviderRateLimit(_)
                    | AgentEvent::Thinking => {}
                    // Plan usage: the ledger credits it to the run's account
                    // (`claude_accounts`); never stored or emitted.
                    // R1 reads the last one to tell whether this run's own
                    // account ran out.
                    AgentEvent::RateLimit(quota) => {
                        if on_login {
                            if let Some(low) = accounts.login_warning(&quota) {
                                announce_login_low(sink, cx, &low);
                            }
                        }
                        if account.is_some()
                            && stream.announced.is_some()
                            && failovers < crate::chat_resume::MAX_FAILOVERS
                            && !stream.quota_notice_sent
                            && !stream.posted_card
                        {
                            if let Some(message) = crate::chat_resume::quota_background_notice(
                                accounts.ceiling_switch_enabled(),
                                account.is_some(),
                                accounts.stream_exhausted(&quota),
                                stream.safe.background(),
                            ) {
                                stream.quota_notice_sent = true;
                                announce_account_notice(
                                    sink,
                                    cx.vault,
                                    cx.session_key,
                                    cx.turn_ord,
                                    message,
                                );
                            }
                        }
                        stream.last_quota = Some(quota);
                    }
                    AgentEvent::Plan { completed, total } => {
                        let _ = crate::session_store::append_event(
                            cx.vault,
                            cx.session_key,
                            cx.turn_ord,
                            "plan",
                            &json!({"completed":completed,"total":total}).to_string(),
                        );
                    }
                    AgentEvent::Session(sid) => {
                        ledger.set_native_session(&sid);
                        // Record a CHANGE, not an announcement. Claude re-emits its
                        // session id on essentially every stream chunk: a live
                        // two-turn conversation produced 7 then 35 identical rows,
                        // each with its own UPDATE, before this guard.
                        let changed = stream.last_session_id.as_deref() != Some(sid.as_str());
                        if changed {
                            stream.hold(cx, sink, Held::HarnessId(sid.clone()));
                            stream.hold(
                                cx,
                                sink,
                                Held::Store {
                                    kind: "session",
                                    payload: sid.clone(),
                                },
                            );
                        }
                        // On probation an unchanged id is held once, not per chunk.
                        let announce = changed || !stream.probation.holding();
                        stream.last_session_id = Some(sid.clone());
                        stream.announced = Some(sid.clone());
                        if announce {
                            stream.hold(
                                cx,
                                sink,
                                Held::Emit(crate::chat_event::build::session(&sid)),
                            );
                        }
                    }
                    AgentEvent::TurnBoundary => {
                        sink.emit(crate::chat_event::build::turn_boundary())
                    }
                    AgentEvent::AssistantDelta(text) => {
                        stream.model_replied |= !text.trim().is_empty();
                        stream.assistant.push_str(&text);
                        sink.emit(attributed(
                            crate::chat_event::build::assistant_delta(&text),
                            parent,
                        ))
                    }
                    AgentEvent::AssistantText(text) => {
                        stream.model_replied |= !text.trim().is_empty();
                        note_assistant_text(&mut stream.assistant, &text);
                        sink.emit(attributed(
                            crate::chat_event::build::assistant_text(&text),
                            parent,
                        ))
                    }
                    // An API-error wrapper shows exactly as the assistant text it
                    // replaced (Paperclip PR 14a), and `results` counts it as part
                    // of the error envelope. It never commits the attempt: a
                    // poisoned session's error must stay retryable (PR 14b).
                    // While the attempt may fail over (R1) it waits in `deferred`.
                    AgentEvent::ApiError { text, .. } => {
                        let emit = Held::Emit(crate::chat_event::build::assistant_text(&text));
                        if stream.defer {
                            stream.deferred.push(Deferred::Text(text));
                        } else {
                            note_assistant_text(&mut stream.assistant, &text);
                        }
                        stream.hold_or_defer(cx, sink, emit, true);
                    }
                    AgentEvent::ToolUse { id, name, input } => {
                        if parent.is_none() && agent::capabilities(cx.agent).plan_proposals {
                            if let Some(plan) = proposed_plan_text(&name, &input) {
                                stream.proposed_plan = Some(plan);
                            }
                        }
                        // The progress phase reads the tool NAME only.
                        if let Some(clock) = &clock {
                            clock.set_progress(crate::run_activity::progress_for_tool(&name));
                        }
                        evidence.record_tool_use(&id, &name, &input);
                        // S27: the final reply is the text after the last call.
                        evidence.mark_reply_start(stream.assistant.len());
                        let _ = crate::session_store::append_event(
                            cx.vault,
                            cx.session_key,
                            cx.turn_ord,
                            "toolUse",
                            &attributed(json!({ "id": id, "name": name, "input": input }), parent)
                                .to_string(),
                        );
                        // Newer than the bounded tool event, so even a large
                        // plan has readable counts after its input is truncated.
                        if matches!(
                            name.as_str(),
                            "TodoWrite" | "update_plan" | "functions.update_plan"
                        ) {
                            if let Some(plan) = crate::subagents::reported_progress(&input) {
                                let _ = crate::session_store::append_event(
                                    cx.vault,
                                    cx.session_key,
                                    cx.turn_ord,
                                    "plan",
                                    &json!({"completed":plan.completed,"total":plan.total})
                                        .to_string(),
                                );
                            }
                        }
                        // The step spine (master plan F6): a trace that can't
                        // be written never fails the turn.
                        let _ = crate::step_spine::tool_started(
                            cx.vault,
                            cx.session_key,
                            cx.turn_ord,
                            cx.run_id,
                            &id,
                            &name,
                            &input,
                        );
                        if !id.is_empty() {
                            stream.tool_names.insert(id.clone(), name.clone());
                        }
                        stream.last_tool = Some(name.clone());
                        // Where a later steer is drawn among the steps.
                        crate::steering::note_tool(cx.session_key, cx.run_id);
                        sink.emit(attributed(
                            crate::chat_event::build::tool_use(&id, &name, input.clone()),
                            parent,
                        ));
                        // After the emit, so the webview has the row before an
                        // approval bound to it can arrive (Paperclip PR 21).
                        crate::approvals::note_tool_use_at(
                            cx.session_key,
                            cx.run_id,
                            &id,
                            &name,
                            &input,
                        );
                    }
                    AgentEvent::ToolResult {
                        id,
                        content,
                        is_error,
                    } => {
                        evidence.record_tool_result(&id, &content, is_error);
                        // S27: a result moves the reply mark too, so a
                        // subagent's closing text before it isn't the reply.
                        evidence.mark_reply_start(stream.assistant.len());
                        crate::approvals::note_tool_result(&id);
                        let _ = crate::session_store::append_event(
                            cx.vault,
                            cx.session_key,
                            cx.turn_ord,
                            "toolResult",
                            &attributed(
                                json!({ "id": id, "isError": is_error, "content": content }),
                                parent,
                            )
                            .to_string(),
                        );
                        let _ = crate::step_spine::tool_finished(
                            cx.vault,
                            cx.run_cwd,
                            cx.session_key,
                            cx.turn_ord,
                            cx.run_id,
                            &id,
                            &content,
                            is_error,
                            evidence.check_outcome(&id),
                        );
                        // For a teamwork delegation, surface any image the teammate
                        // created so it renders inline in the chat, not as a bare path.
                        let is_team = stream
                            .tool_names
                            .get(&id)
                            .map(|n| is_team_tool(n))
                            .unwrap_or(false);
                        let images = if is_team {
                            extract_result_images(&content)
                        } else {
                            Vec::new()
                        };
                        let id_for_images = id.clone();
                        sink.emit(attributed(
                            crate::chat_event::build::tool_result(&id, &content, is_error),
                            parent,
                        ));
                        if !images.is_empty() {
                            sink.emit(attributed(
                                crate::chat_event::build::tool_images(&id_for_images, images),
                                parent,
                            ));
                        }
                    }
                    AgentEvent::Result {
                        text,
                        usage,
                        is_error,
                        subtype,
                        ..
                    } => {
                        // The raw harness figure, kept for the transcript only
                        // (session-cumulative on a Claude resume). The chip and
                        // Settings read the usage ledger. None for a
                        // self-hosted run (Q8).
                        let cost_usd = transcript_cost(ledger, usage.cost_usd);
                        stream.reported_failure |= is_error;
                        // The envelope's own kind (Paperclip PR 14a): the
                        // frontend labels an error result with it, and shows a
                        // refusal as a neutral notice.
                        let result_kind = live_result_kind(agent, &stream.results);
                        let held = stream.hold_result(is_error, line);
                        stream.hold_or_defer(
                            cx,
                            sink,
                            Held::Store {
                                kind: "result",
                                payload: json!({"costUsd": cost_usd, "isError": is_error, "subtype": subtype, "resultKind": result_kind.as_str()}).to_string(),
                            },
                            held,
                        );
                        if let Some(final_text) =
                            text.as_deref().filter(|value| !value.trim().is_empty())
                        {
                            if is_error && stream.defer {
                                stream
                                    .deferred
                                    .push(Deferred::ResultText(final_text.to_string()));
                            } else if stream.assistant.trim().is_empty() {
                                stream.assistant.push_str(final_text);
                            }
                        }
                        let payload = crate::chat_event::build::result(
                            text.clone(),
                            cost_usd,
                            is_error,
                            &subtype,
                            result_kind.as_str(),
                            stream.last_session_id.clone(),
                        );
                        stream.hold_or_defer(cx, sink, Held::Emit(payload), held);
                        // The answer is in: close the input so the CLI exits,
                        // unless a steer is still on its way in (then it runs
                        // as one more step of this turn). A turn without a
                        // steering entry (tests, a stand-in) closes at once.
                        // (A Codex session closes its own input at
                        // `turn/completed`, below.)
                        if let Some((tx, attempt_no)) = live_tx.as_ref().filter(|_| !codex_live) {
                            let queued = result_queued_turns(line);
                            match (
                                crate::steering::on_result(cx.session_key, cx.run_id, queued),
                                attempt_no,
                            ) {
                                (crate::steering::AfterResult::Closed, _) => {}
                                (crate::steering::AfterResult::Waiting(wait), Some(attempt_no)) => {
                                    let key = cx.session_key.to_string();
                                    let run = cx.run_id.to_string();
                                    let attempt_no = *attempt_no;
                                    tokio::spawn(async move {
                                        tokio::time::sleep(crate::steering::LATE_STEER_GRACE).await;
                                        crate::steering::close_if_stalled(
                                            &key, &run, attempt_no, wait,
                                        );
                                    });
                                }
                                // No steering entry for this attempt: nothing
                                // can be pending, so close at once.
                                _ => {
                                    let _ = tx.send(crate::steering::PipeMsg::Close);
                                }
                            }
                        }
                    }
                }
            }
            // Turn guards act only at a safe point, once per attempt.
            if safe && stream.yielded.is_none() {
                let night_over = night.and_then(|n| n.spend.check(n.key, n.run, stream.estimate));
                // L9: the CFO budget, on this turn's own tokens so far.
                let budget_over = night_over.is_none().then(|| {
                    let own = ledger
                        .running_total()
                        .map_or(0, |t| t.input.saturating_add(t.output));
                    watch.cfo.as_mut().and_then(|cfo| cfo.check(own))
                });
                // P1a: a pause the person asked for, once the stream named its
                // native session (the resume continues it). Not after a
                // question card: the agent is already ending to wait.
                let pause = crate::chat_pause::pause_requested(cx.session_key)
                    && !stream.posted_card
                    && stream.last_session_id.is_some();
                let verdict = match (night_over, budget_over.flatten()) {
                    (Some((total, cap)), _) => Some(Yield::Night { total, cap }),
                    (None, Some(note)) => Some(Yield::Budget { note }),
                    (None, None) if pause => Some(Yield::Pause),
                    (None, None) => {
                        let facts = crate::chat_resume::CeilingFacts {
                            enabled: account.is_some() && accounts.ceiling_switch_enabled(),
                            account: account.as_deref(),
                            session: stream.last_session_id.as_deref(),
                            over: account.is_some()
                                && stream
                                    .last_quota
                                    .as_ref()
                                    .is_some_and(|quota| accounts.stream_exhausted(quota)),
                            failovers,
                            posted_card: stream.posted_card,
                            background: stream.safe.background(),
                        };
                        crate::chat_resume::decide_ceiling(&facts, || accounts.next(&exclude))
                            .map(|to| Yield::Ceiling { to })
                    }
                };
                if let Some(verdict) = verdict {
                    stream.yielded = Some(verdict);
                    yield_signal.request();
                }
            }
        };
        let mut codex_closed = false;
        // Overnight R1: a remote attempt's stdout bytes handled so far, kept
        // after each line, so a Hollow that quits mid-turn reattaches from
        // exactly the next (a reattached run counts on from where it was).
        let mut progress = spec.extras.remote.as_ref().and_then(|target| {
            crate::remote_session::Progress::start(
                &target.turn,
                target.reattach.as_ref().map_or(0, |r| r.out),
            )
        });
        // Antigravity's pending permission cards: `acp_gone` ends their waits
        // once the attempt's process is gone, `acp_denied` carries the tools
        // the person declined back to the session.
        let acp_gone = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let acp_denied: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
        let mut driven = process_group::drive(child, guard, supervision, |raw| {
            // OC1: the agent's ACP line through the attempt's session: its
            // answers go back on stdin, its headless lines read as `grok -p`'s.
            if let Some(session) = acp.as_mut() {
                if stop.requested() || yield_signal.requested() {
                    session.note_stopping();
                    // A card still open answers nothing now (review: a click
                    // during teardown must not write an Allow).
                    acp_gone.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                // A tool the person declined on its card (the card's task
                // wrote the answer) reads as the denial it was.
                for id in
                    std::mem::take(&mut *acp_denied.lock().unwrap_or_else(PoisonError::into_inner))
                {
                    session.note_declined(&id);
                }
                let step = session.on_line(raw);
                if let Some((tx, _)) = &live_tx {
                    for write in step.writes {
                        let _ = tx.send(crate::steering::PipeMsg::Line(write));
                    }
                    // 2026-11-20: Antigravity's permission requests become
                    // the person's approval cards; the answer goes back on
                    // stdin from the card's task, and a Stop ends the wait.
                    for ask in step.asks {
                        spawn_acp_card(
                            sink.approval_core(),
                            cx,
                            ask,
                            tx.clone(),
                            acp_gone.clone(),
                            acp_denied.clone(),
                        );
                    }
                }
                for line in &step.lines {
                    on_line(line);
                }
                if step.finished && !codex_closed {
                    codex_closed = true;
                    // The turn is over: no card may still answer into stdin.
                    acp_gone.store(true, std::sync::atomic::Ordering::Relaxed);
                    if let Some((tx, _)) = &live_tx {
                        let _ = tx.send(crate::steering::PipeMsg::Close);
                    }
                }
                return;
            }
            let Some(session) = codex.as_ref() else {
                on_line(raw);
                if let Some(progress) = progress.as_mut() {
                    progress.add(raw);
                }
                return;
            };
            // Steering PR 3: the server's line through the attempt's
            // session: its answers go back on stdin, its steer outcomes are
            // recorded, and its exec lines read as an exec line would.
            let (step, updates) = {
                let mut session = session.lock().unwrap_or_else(PoisonError::into_inner);
                // Hollow's Stop (or a turn guard) ends the process group:
                // what the server prints as it shuts down is that stop.
                if stop.requested() || yield_signal.requested() {
                    session.note_stopping();
                }
                let step = session.on_line(raw);
                (step, session.steer_events())
            };
            if let Some((tx, _)) = &live_tx {
                for write in step.writes {
                    let _ = tx.send(crate::steering::PipeMsg::Line(write));
                }
                // Held steers go out through the writer, in order.
                if step.held_steers {
                    let _ = tx.send(crate::steering::PipeMsg::HeldSteers);
                }
            }
            record_codex_steers(sink, cx, updates);
            for line in &step.exec_lines {
                on_line(line);
            }
            // The turn is over: close stdin, so the server exits (EOF), and
            // refuse a later steer as ending.
            if step.finished && !codex_closed {
                codex_closed = true;
                crate::steering::close_input(cx.session_key, cx.run_id);
                if let Some((tx, _)) = &live_tx {
                    let _ = tx.send(crate::steering::PipeMsg::Close);
                }
            }
        })
        .await;
        acp_gone.store(true, std::sync::atomic::Ordering::Relaxed);
        // A later attempt (an account switch) starts a run of its own there.
        if let Some(target) = spec.extras.remote.as_mut() {
            target.reattach = None;
        }
        // Classify before ended_early closes the session. A shared server
        // retirement can interrupt unrelated turns after uncertain effects;
        // earlier provider/network wording must never authorize a replay.
        let uncertain_shared = shared_codex_attempt
            && codex.as_ref().is_some_and(|session| {
                !session
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .finished()
            });
        // Round-2 review: a Codex server that exited on its own before its
        // turn ended (after an error it said it would retry, say) leaves no
        // terminal line, and its exit 0 would read as a success. Not after a
        // stop, a guard or the deadline: those keep their own outcome.
        if let Some(session) = codex.as_ref() {
            let early =
                (driven.ended == Ended::Exited && !driven.missing_stdout && !stop.requested())
                    .then(|| {
                        session
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .ended_early()
                    })
                    .flatten();
            if let Some(line) = early {
                on_line(&line);
            }
        }
        // OC1: an ACP agent that exited before answering its prompt (not
        // after a stop, a guard or the deadline) is a failed turn, never a
        // silent success.
        if let Some(session) = acp.as_mut() {
            if driven.ended == Ended::Exited && !driven.missing_stdout && !stop.requested() {
                if let Some(line) = session.ended_early() {
                    on_line(&line);
                }
            }
        }
        // The attempt's process is gone, and its input with it.
        if let Some((tx, _)) = live_tx {
            crate::steering::detach(cx.session_key, cx.run_id);
            let _ = tx.send(crate::steering::PipeMsg::Close);
        }
        // A10 review: Codex writes its rollout through a buffered writer, so
        // the turn's last size or compaction line can land after
        // `turn.completed`; read once more now that the process has exited.
        if let Some(seen) = rollout.as_mut().and_then(crate::codex_rollout::Watch::poll) {
            feed_codex_rollout(seen, &mut stream.state, ledger, &gauge);
        }
        // Overnight 9 A13: the model the rollout says ran prices a
        // default-model turn at the person's row for it.
        if let Some(model) = rollout
            .as_ref()
            .and_then(crate::codex_rollout::Watch::model)
        {
            ledger.set_turn_model(model);
        }
        cut_tasks.commit(
            driven.ended == Ended::Exited && driven.leader_exited_first,
            evidence,
        );
        let resume_failed = attempt == 0
            && !uncertain_shared
            && resumed.is_some()
            && stream.probation.holding()
            && agent_runs
            && !driven.missing_stdout
            && driven.ended == Ended::Exited
            && crate::result_kind::is_resume_failure(
                agent,
                stream.results.record().as_ref(),
                &driven.stderr_tail,
            );
        // Turn guards: an attempt a guard ended at a safe point. Only when
        // the yield really ended it (a user's Stop wins; an attempt that
        // exited first just finished).
        let yielded = if driven.ended == Ended::LimitReached && !stop.requested() {
            stream.yielded.take()
        } else {
            None
        };
        // P1a: a Stop that landed while a pause was ending the attempt wins:
        // the turn ends as stopped, never paused.
        if driven.ended == Ended::LimitReached
            && stop.requested()
            && stream.yielded == Some(Yield::Pause)
        {
            driven.ended = Ended::Stopped;
        }
        match yielded {
            Some(Yield::Ceiling { to: _ }) => {
                let from = account.clone().unwrap_or_default();
                tried.push(from.clone());
                accounts.refresh().await;
                let to = accounts.next(&tried);
                if stop.requested() || to.is_none() {
                    let failover_stop = if stop.requested() {
                        driven.ended = Ended::Stopped;
                        None
                    } else {
                        Some(crate::chat_resume::failover_stop_message(
                            crate::chat_resume::FailoverStop::NoneEligible,
                            accounts
                                .soonest_room()
                                .and_then(|at| {
                                    crate::chat_resume::reset_label(at, &chrono::Local::now())
                                })
                                .as_deref(),
                        ))
                    };
                    stream.flush_deferred(cx, sink);
                    stream.commit(cx, sink);
                    return AttemptsEnd::Driven {
                        driven,
                        stream,
                        agent_runs: true,
                        retried,
                        failover_stop,
                        night_stop: None,
                        budget_stop: None,
                        paused: None,
                    };
                }
                let to = to.expect("eligible account checked");
                let session = stream.last_session_id.clone().unwrap_or_default();
                let next = Next::Failover {
                    session,
                    to: to.clone(),
                    from_label: accounts.label(&from),
                    to_label: accounts.label(&to),
                    reason: crate::chat_resume::SwitchReason::Ceiling,
                };
                // What this attempt left running counts too (overnight A3).
                evidence.set_leftovers(driven.left_running());
                previous = Some((driven, stream, next));
                failovers += 1;
                attempt += 1;
                continue;
            }
            Some(Yield::Night { total, cap }) => {
                // Ended before the agent reported a cost: the row keeps the
                // estimate, so the night never reads it as unknown.
                ledger.set_cost_estimate(stream.estimate);
                stream.flush_deferred(cx, sink);
                stream.commit(cx, sink);
                return AttemptsEnd::Driven {
                    driven,
                    stream,
                    agent_runs,
                    retried,
                    failover_stop: None,
                    night_stop: Some((total, cap)),
                    budget_stop: None,
                    paused: None,
                };
            }
            Some(Yield::Pause) => {
                let note = crate::chat_pause::note(&stream.assistant, stream.last_tool.as_deref());
                stream.flush_deferred(cx, sink);
                stream.commit(cx, sink);
                return AttemptsEnd::Driven {
                    driven,
                    stream,
                    agent_runs,
                    retried,
                    failover_stop: None,
                    night_stop: None,
                    budget_stop: None,
                    paused: Some(note),
                };
            }
            Some(Yield::Budget { note }) => {
                // Recorded only now that the turn really yielded to it.
                if let Some(cfo) = watch.cfo.as_ref() {
                    cfo.commit();
                }
                stream.flush_deferred(cx, sink);
                stream.commit(cx, sink);
                return AttemptsEnd::Driven {
                    driven,
                    stream,
                    agent_runs,
                    retried,
                    failover_stop: None,
                    night_stop: None,
                    budget_stop: Some(note),
                    paused: None,
                };
            }
            None => {}
        }
        if !resume_failed {
            // AG4b: an attempt that ended on a deferred tool call asks the
            // person, records the answer for the hook and resumes the same
            // native session. When it cannot (a stop, no session, the card
            // cap, an answers file that cannot be written) the call never
            // ran and the turn says so instead of ending blank.
            if let Some(call) = stream.deferred_call.take() {
                let session = stream
                    .announced
                    .clone()
                    .or_else(|| stream.last_session_id.clone())
                    .filter(|s| !s.trim().is_empty());
                let answers = spec.extras.defer_answers.clone();
                let mut why = "Hollow could not resume the session";
                if let (Some(session), Some(answers)) = (session, answers) {
                    why = "the turn ended before the approval could be answered";
                    if deferrals >= MAX_DEFERRALS {
                        why = "this turn already asked for the most approvals it may";
                    }
                    if !stop.requested()
                        && !driven.missing_stdout
                        && driven.ended == Ended::Exited
                        && deferrals < MAX_DEFERRALS
                    {
                        let answer = ask_deferred_call(sink, cx, stop, &call).await;
                        match crate::claude_defer::record_answer(
                            &answers,
                            &call.id,
                            // A Stop that landed with the click wins: never a
                            // stale Allow left in the file.
                            answer.approved && !stop.requested(),
                            answer.reason.as_deref(),
                        ) {
                            Ok(()) if !stop.requested() => {
                                evidence.set_leftovers(driven.left_running());
                                previous = Some((driven, stream, Next::Deferred { session }));
                                deferrals += 1;
                                attempt += 1;
                                continue;
                            }
                            Ok(()) => {}
                            Err(error) => {
                                why = "Hollow could not record the answer";
                                eprintln!(
                                    "[chat] could not record a deferred call's answer: {error}"
                                )
                            }
                        }
                    }
                }
                let text = format!(
                    "Claude asked to use {} and it was not run: {why}.",
                    call.name
                );
                note_assistant_text(&mut stream.assistant, &text);
                stream.hold_or_defer(
                    cx,
                    sink,
                    Held::Emit(crate::chat_event::build::assistant_text(&text)),
                    false,
                );
            }
            // R1: an attempt whose own account ran out resumes the same
            // native session on the next eligible account not yet tried.
            let judged = match account.as_ref() {
                Some(account)
                    if agent_runs
                        && !driven.missing_stdout
                        && driven.ended == Ended::Exited
                        && !stop.requested() =>
                {
                    tried.push(account.clone());
                    judge_failover(accounts, &driven, &stream, &tried, evidence).await
                }
                _ => (Failover::No, None),
            };
            if let (Failover::Switch { to }, _) = &judged {
                let session = stream.announced.clone().unwrap_or_default();
                let from_label =
                    accounts.label(tried.last().map(String::as_str).unwrap_or_default());
                let to_label = accounts.label(to);
                // What this attempt left running counts too (overnight A3).
                evidence.set_leftovers(driven.left_running());
                previous = Some((
                    driven,
                    stream,
                    Next::Failover {
                        session,
                        to: to.clone(),
                        from_label,
                        to_label,
                        reason: crate::chat_resume::SwitchReason::Limit,
                    },
                ));
                failovers += 1;
                attempt += 1;
                continue;
            }
            // Overnight 9 A3: a provider outage (Codex's "model at capacity",
            // an overload, a network failure) resumes the same native session
            // once, after a minute or two, unless the turn's deadline passes
            // first. A Stop during the wait ends the turn on this attempt.
            // Failed as a whole: its final result is an error or the CLI
            // exited non-zero (review: a Codex turn can report a non-fatal
            // `error` event and still complete).
            let failed = stream.results.record().is_some_and(|r| r.is_error)
                || !matches!(driven.status.as_ref(), Some(Ok(status)) if status.success());
            // Only a session this attempt announced holds the user's message
            // (G2): a resumed turn that failed before announcing one has
            // nothing to go on from.
            let session = stream.announced.clone().filter(|s| !s.trim().is_empty());
            if let Some(session) = session.filter(|_| {
                may_back_off
                    && !uncertain_shared
                    && failed
                    && !driven.missing_stdout
                    && driven.ended == Ended::Exited
                    && !stop.requested()
                    && crate::result_kind::is_backoff_outage(
                        agent,
                        stream.results.record().as_ref(),
                        &driven.stderr_tail,
                    )
            }) {
                let wait = backoff_wait();
                let fits = deadline_at.is_none_or(|at| {
                    tokio::time::Instant::now() + wait + Duration::from_secs(60) < at
                });
                if fits {
                    evidence.set_leftovers(driven.left_running());
                    tokio::select! {
                        _ = tokio::time::sleep(wait) => {}
                        _ = stop.wait() => {}
                    }
                    previous = Some((
                        driven,
                        stream,
                        Next::Backoff {
                            session,
                            waited: wait.as_secs(),
                        },
                    ));
                    backoffs += 1;
                    attempt += 1;
                    continue;
                }
            }
            // Not a retryable resume failure nor a failover: whatever the
            // attempt still holds is its real outcome, shown now.
            stream.flush_deferred(cx, sink);
            stream.commit(cx, sink);
            return AttemptsEnd::Driven {
                driven,
                stream,
                agent_runs,
                retried,
                failover_stop: judged.1,
                night_stop: None,
                budget_stop: None,
                paused: None,
            };
        }
        if stop.requested() {
            return AttemptsEnd::StoppedBeforeRetry { driven, stream };
        }
        // What this attempt left running counts too (overnight A3).
        evidence.set_leftovers(driven.left_running());
        previous = Some((driven, stream, Next::Fresh));
        attempt += 1;
    }
}

/// An Antigravity permission request over ACP, shown as the turn's approval
/// card (the same inline Allow/Deny card Hollow's own tools raise). The card
/// waits on a blocking thread (it polls for up to the broker's 120 s); the
/// answer, an allow only on the person's Allow, is written to the agent's
/// stdin, and a Deny is noted for the session. A sink with no app, a card
/// that failed, a timeout and a finished attempt all decline: never an allow
/// by default.
fn spawn_acp_card(
    app: Option<Core>,
    cx: &TurnCx<'_>,
    ask: crate::acp_client::PermissionAsk,
    tx: tokio::sync::mpsc::UnboundedSender<crate::steering::PipeMsg>,
    gone: Arc<std::sync::atomic::AtomicBool>,
    denied: Arc<std::sync::Mutex<Vec<String>>>,
) {
    let session = cx.session_key.to_string();
    let run = cx.run_id.to_string();
    tokio::spawn(async move {
        let allow = match app {
            None => false,
            Some(app) => {
                let action = ask.action_text();
                let title = format!("Antigravity: {}", ask.tool);
                let input = ask.input.clone();
                let watch = gone.clone();
                tokio::task::spawn_blocking(move || {
                    crate::approvals::ask_deferred_app(
                        &app,
                        crate::approvals::InProcessCard {
                            title: &title,
                            action: &action,
                            kind: "app",
                            tool: None,
                            args: &input,
                        },
                        &|| watch.load(std::sync::atomic::Ordering::Relaxed),
                        &session,
                        &run,
                    )
                    .approved
                })
                .await
                .unwrap_or(false)
            }
        };
        if !allow {
            denied
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(ask.tool_call_id.clone());
        }
        if !gone.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = tx.send(crate::steering::PipeMsg::Line(ask.reply(allow)));
        }
    });
}

/// AG4b: the person's answer to the card of a deferred Claude tool call. The
/// card waits on a blocking thread (it polls for up to the broker's 120 s),
/// and a Stop ends the wait. A sink with no app declines the call.
async fn ask_deferred_call<S: ChatSink + ?Sized>(
    sink: &S,
    cx: &TurnCx<'_>,
    stop: &StopSignal,
    call: &crate::claude_defer::DeferredCall,
) -> crate::approvals::Answer {
    let declined = |why: &str| crate::approvals::Answer {
        approved: false,
        reason: Some(why.to_string()),
    };
    let action = match crate::claude_defer::action_text(call) {
        Ok(action) => action,
        Err(why) => return declined(&why),
    };
    let Some(app) = sink.approval_core() else {
        return declined("Hollow could not show an approval card");
    };
    let gone = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut handle = tokio::task::spawn_blocking({
        let gone = gone.clone();
        let call = call.clone();
        let session = cx.session_key.to_string();
        let run = cx.run_id.to_string();
        move || {
            let title = format!("Claude: {}", call.name);
            crate::approvals::ask_deferred_app(
                &app,
                crate::approvals::InProcessCard {
                    title: &title,
                    action: &action,
                    kind: "app",
                    tool: None,
                    args: &call.input,
                },
                &|| gone.load(std::sync::atomic::Ordering::Relaxed),
                &session,
                &run,
            )
        }
    });
    let joined = tokio::select! {
        done = &mut handle => done,
        _ = stop.wait() => {
            gone.store(true, std::sync::atomic::Ordering::Relaxed);
            handle.await
        }
    };
    joined.unwrap_or_else(|_| declined("the approval card failed"))
}

/// The Hollow servers a chat turn attaches and what they give it, and so
/// its fingerprint's `mcp` and `caps` parts (OB03: one builder, which a
/// test drives for a Main-agent wake and the person's turn). Task tools
/// read only in plan mode (Paperclip PR 12); the server gets the vault
/// root, never the turn's cwd. A workspace's turn (a scoped run, PR B3)
/// writes tasks too since 2026-09-29: the broker takes the vault for each
/// such write in this run's name and records it in this run's History
/// (`automated_history::with_live_run`), so the agent's own file access
/// stays its folder. Hollow app tools (PR A1) follow the same rule; a turn
/// a `hollow_app` session start queued runs one level deeper (A2). Master
/// plan #1: an unattended turn gets no teammates, computer control or
/// services. A bare turn's `for_bare` is the caller's.
fn turn_servers(
    agent: AgentKind,
    perm: &str,
    cwd: &Path,
    vault: &Path,
    parent: &DelegateParent,
    (app_depth, start_depth): (u32, u32),
    unattended: bool,
) -> (serde_json::Map<String, Value>, AttachedCaps) {
    let tasks_access = crate::tasks_mcp::TasksAccess::for_chat(perm);
    let app_access = Some(crate::app_mcp::AppAccess::for_chat(perm));
    let app_grant = app_access.map(|access| {
        crate::app_mcp::AppGrant::new(access, app_depth, perm).listing_from(start_depth)
    });
    let (servers, attached) =
        crate::mcp_attach::server_map_for(agent, perm, cwd, vault, parent, tasks_access, app_grant);
    crate::mcp_attach::for_unattended(unattended, servers, attached)
}

/// Whether the person waits on the turn that runs as history run `run_id`
/// (phone API v1.1 review, 2026-11-05): they sent it (`person_sent`, from
/// [`ChatTurnPayload::person_sent`]: their own message from the window or
/// the phone, a card answer, a held send) and it isn't an unattended Main
/// agent run ([`crate::main_agent::unattended::is_unattended_run`], which
/// also fails closed on a Main agent turn whose message is Hollow's).
/// `runStarted` carries it as `attended`; the phone API pushes
/// `turnFinished` only for such a turn. A Main agent wake, check-in or
/// swarm wake says `unattended: false` and is never attended.
pub(crate) fn turn_attended(vault: &Path, person_sent: bool, run_id: &str) -> bool {
    person_sent && !crate::main_agent::unattended::is_unattended_run(vault, run_id)
}

/// Run an admitted chat turn inside its lease: workspace context and recall
/// (computed read-only by a prefetch admission started, possibly before
/// the lease, and recorded once, in `turn_inputs_prefetched`: J3a), the session-store turn (the transcript commit of the user's message),
/// `runStarted`, retrieval (inside the lease since PR 7b, so a stop during
/// it is honored), then the agent, streamed as `chat-event`s tagged with the
/// turn's `TurnTag`. Every exit after the commit returns through
/// `finish_observed_run`, exactly once (merged plan slice 3, B·F10); the one
/// before it ends a turn that never started (`end_unstarted_turn`). Nothing
/// here uses `?`: the `RunLease` drop is only the backstop.
///
/// With `--include-partial-messages`, claude emits `stream_event` deltas as
/// the model generates. We forward those as `assistantDelta` events so the
/// frontend can stream tokens. The final aggregated `assistant` event still
/// arrives at the end, but we skip emitting its text again to avoid dupes
/// — the deltas already built up the full message.
pub(crate) async fn execute_turn(app: Core, vault: PathBuf, turn: AdmittedTurn) -> TurnEnd {
    let AdmittedTurn {
        lease,
        live,
        registration,
        tag,
        session_key,
        payload,
        agent,
        cwd,
        session_id,
        recall_prefetch,
    } = turn;
    let out = TurnEvents {
        app: &app,
        tag,
        retitle: std::sync::atomic::AtomicBool::new(false),
        retitle_on: std::sync::OnceLock::new(),
        person_chat: std::sync::atomic::AtomicBool::new(false),
    };
    // Each preparation step's mark (agent effectiveness F8): these two are
    // taken before the turn's evidence collector exists, then folded in.
    let mut early_marks = PrepMarks::default();
    // Recall, connector selection and learning read the admitted request and
    // its resolved entities, never the composed prompt (agent effectiveness
    // F2): profile prose, page context or an old conversation bundle can't
    // pick memories or connectors, or make the turn "learn".
    let recall_started = Instant::now();
    // The turn asks to resume its native session (OB25). Only a hint for
    // the inputs: a follow-up skips retrieval only once the resume plan
    // below really resumes (`RetrievalBasis::on_plan`).
    let resumed = chat_queue::asks_resume(session_id.as_deref());
    // J3a: started at admission; taken if ready within its cutoff, else
    // recall ranks by stems here and never waits for its vector.
    let (
        TurnInputs {
            recall,
            connector_query,
            mut learn_from,
            source_query,
            visual_cue,
            retrieval_basis,
            recall_scope: _,
        },
        recall_pickup,
    ) = turn_inputs_prefetched(&vault, &payload, &session_key, resumed, recall_prefetch);
    early_marks.record(Step::Recall, Mark::since(recall_started));
    // The person sent it: a continuation on another agent ([`agent_out`])
    // and `runStarted`'s `attended` ([`turn_attended`]) read it; so does
    // the recall pickup's log line (J3a: own and agent turns apart).
    let person_sent = payload.person_sent();
    crate::recall_prefetch::log_pickup(person_sent, &recall_pickup);
    let ChatTurnPayload {
        prompt,
        model,
        permission_mode,
        asked,
        app_depth,
        ultracode,
        effort: requested_effort,
        canvases,
        continued_from,
        unattended,
        typed,
        host_asker,
        canned,
        worker,
        parent: subagent_of,
        ..
    } = payload;
    // Repo ideas 2026-10-27 #8: what a continuation on another agent needs
    // if this turn runs out (`agent_out`), taken before the run uses them:
    // whether the person sent it, and whether it continues one itself
    // (asked once, as it runs).
    let out_turn = crate::agent_out::Continuation {
        session_key: session_key.clone(),
        cwd: cwd.to_string_lossy().into_owned(),
        permission_mode: permission_mode.clone(),
        canvases: canvases.clone(),
        from: agent,
        to: agent,
        resets: None,
        stops: crate::agent_out::stops(&session_key),
    };
    let continued_out = out
        .tag
        .client_turn_id
        .as_deref()
        .is_some_and(crate::agent_out::was_continuation);
    // A pressure-test persona or bake-off variant in its scratch folder runs
    // bare (overnight S7): no Hollow tools or shared connectors, none of the
    // vault's context (retrieval, recall, the operator profile, the skill
    // index, the vault brief) and no learning from it. Everything it may use
    // is in its message.
    let bare = crate::mcp_attach::bare_scratch(&vault, &cwd);
    // Self-hosted models Q5 review: a chat on a self-hosted model keeps its
    // words off Anthropic. The model retitle and the learning extraction
    // run on the extraction model (Anthropic's for Claude), so neither
    // runs for its turns, unless (Q10) Settings, Advanced,
    // Background jobs sends Chat titles or Learning extraction to
    // the chat's own entry: then they run there, and only there
    // (`only_on`), and stay skipped otherwise.
    let self_hosted_chat = crate::self_hosted::chat_on_self_hosted(agent, model.as_deref());
    let own_entry = model
        .as_deref()
        .and_then(crate::self_hosted::entry_id)
        .filter(|_| self_hosted_chat)
        .map(str::to_string);
    let on_own = |kind| {
        own_entry
            .as_deref()
            .is_none_or(|own| crate::self_hosted_jobs::routes_to(kind, own))
    };
    let titles_ok = on_own(crate::self_hosted_jobs::JobKind::Titles);
    out.retitle.store(
        app_depth == 0 && !unattended && !bare && titles_ok,
        std::sync::atomic::Ordering::Relaxed,
    );
    out.person_chat.store(
        app_depth == 0 && !unattended && !bare,
        std::sync::atomic::Ordering::Relaxed,
    );
    if let Some(own) = own_entry.clone().filter(|_| titles_ok) {
        let _ = out.retitle_on.set(own);
    }
    // Only the person's own typed words teach (fewer learnings, 10-11): an
    // agent's packet, Hollow's messages and a bare run never do.
    let learns = turn_learns(typed, app_depth, unattended, bare)
        && on_own(crate::self_hosted_jobs::JobKind::Learning);
    learn_from.self_hosted = own_entry.clone();
    let run_id = lease.id().to_string();
    if learns {
        crate::learning_capture::started(
            &vault,
            &session_key,
            crate::learning_capture::Input {
                user: crate::next_step::strip(&learn_from.user),
                references: learn_from.references.clone(),
                run_id: run_id.clone(),
                self_hosted: own_entry.clone(),
                ..Default::default()
            },
        );
        crate::vault_learning::mark_person_run(
            &run_id,
            &session_key,
            crate::vault_learning::asks_to_learn(&learn_from.user),
        );
        crate::vault_learning::note_person_words(
            &run_id,
            &crate::next_step::strip(&learn_from.user),
        );
    }
    let stop = registration.signal();
    // One immutable decision governs admission, the UI and every attempt's
    // transport. Stop drops the bounded version read before any LIVE lock.
    let steer_support = tokio::select! {
        biased;
        _ = stop.wait() => crate::steering::support_info(agent, None),
        resolved = crate::steering::resolve_support(agent) => resolved,
    };
    // Local reads, before the turn is announced. A failure ends the turn
    // after `runStarted`, as an ordinary failed turn.
    // Attached sources are the passages that best match the request (agent
    // effectiveness F3 part 2); what was chosen goes on the manifest below.
    // A durable child task with a packet gets the slim envelope (S01), and
    // so does a handover's successor (overnight 9 A6, reversing S01's
    // exception: lane successors' first turns carried 74–75 KB of brief and
    // plan they can read themselves). `expected` asking for the "full brief"
    // keeps the full one.
    let child_packet = crate::delegation::child_envelope(app_depth, &prompt);
    let wants_canvas = crate::delegation::child_wants_canvas(app_depth, &prompt);
    let source_request = crate::workspaces::SourceRequest {
        query: &source_query,
        reader: if crate::mcp_attach::chat_has_vault_reader(agent) {
            crate::workspaces::SourceReader::ReadSource
        } else {
            crate::workspaces::SourceReader::OwnTools
        },
        child_packet: child_packet.as_deref(),
    };
    let sources_started = Instant::now();
    let (workspace_context, attached_sources) =
        match crate::workspaces::context_for(&vault, &cwd, &source_request) {
            Ok(Some(mut c)) => {
                early_marks.record(Step::AttachedSources, Mark::since(sources_started));
                let sources = c.sources.take();
                (Ok(Some(c)), sources)
            }
            Ok(None) => (Ok(None), None),
            Err(e) => (Err(e), None),
        };
    // Approved memory applies in every conversation, workspace or not
    // (2026-09-26: it used to be skipped inside workspaces).
    let mut recall = match &workspace_context {
        Ok(_) => recall,
        Err(_) => crate::vault_learning::RecallResult::default(),
    };
    // RC1: the silence rules and the served ledger cut what the agent already
    // has or should not get, and leave a trace the side panel card lists.
    let session_holds = resumed
        && crate::recall_trace::session_holds_block(&vault, &session_key, &recall.prompt_block);
    let recall_trace = crate::recall_trace::gate(
        &session_key,
        &asked,
        &crate::recall_trace::TurnKind {
            unattended,
            person_sent,
            app_depth,
            resumed,
        },
        &mut recall,
        session_holds,
    );
    // Master plan #17: "what did I know on 9/15 about …" gets the
    // workspace's brief, plan and decisions as of that day, with where each
    // version came from.
    if workspace_context.is_ok() {
        if let Some(block) =
            crate::asof::question_block(&vault, &cwd, &asked, chrono::Local::now().date_naive())
        {
            if !recall.prompt_block.is_empty() {
                recall.prompt_block.push_str("\n\n");
            }
            recall.prompt_block.push_str(&block);
        }
    }
    if bare {
        recall = crate::vault_learning::RecallResult::default();
    }
    // The transcript commit (merged plan slice 3, B·F10): the user's message
    // is in the conversation's transcript before the turn is announced or
    // anything is recorded for it. Until then the queue row holds it; a turn
    // whose transcript cannot be opened never starts, and its text goes back
    // to the composer. It used to run on as ordinal 0, recorded nowhere.
    // CP1: while a restore of this chat's worktree runs no turn starts; it
    // leaves through the one unstarted exit (its text goes back).
    let restoring = crate::turn_checkpoints::restoring(&session_key);
    let started = if restoring {
        Err(crate::turn_checkpoints::RESTORE_BUSY.to_string())
    } else {
        crate::session_store::start_turn(
            &vault,
            &session_key,
            agent.as_str(),
            &cwd.to_string_lossy(),
            &run_id,
            &asked,
        )
    };
    let turn_ord = match started {
        Ok(turn_ord) => turn_ord,
        Err(error) => {
            let message = if restoring {
                error
            } else {
                format!("the conversation's transcript could not be written: {error}")
            };
            // P1a: a resume that never started gives its claim back.
            crate::chat_pause::release_claim(
                &vault,
                &session_key,
                out.tag.client_turn_id.as_deref(),
            );
            return end_unstarted_turn(&out, lease, &session_key, message);
        }
    };
    // L1: a subagent's conversation records who started it, once (its first
    // turn; a later message never re-parents it). Best effort, like every
    // trace write: the Subagents list falls back to the delegation records.
    if let Some(parent) = subagent_of.as_deref() {
        if let Err(error) = crate::session_store::set_parent(
            &vault,
            &session_key,
            parent,
            crate::session_store::FORK_SUBAGENT,
        ) {
            eprintln!("[subagent] {session_key}'s parent {parent} wasn't recorded: {error}");
        }
    }
    // Persist capture eligibility and privacy separately from the CLI agent.
    // A private model may use the same Claude/Codex harness as a cloud model.
    if learns {
        let scope = own_entry
            .as_deref()
            .map(|id| format!("private:{id}"))
            .unwrap_or_else(|| format!("cloud:{}", agent.as_str()));
        if let Err(error) =
            crate::session_store::record_turn_capture_scope(&vault, &session_key, turn_ord, &scope)
        {
            eprintln!("[learning capture] {error}; an unmarked turn cannot enter a digest");
        }
    }
    // Mid-turn steering: the person may steer this turn from its chat's
    // own composer (`chat_steer`) until it finishes; the guard ends that.
    let _steerable = crate::steering::open(
        &session_key,
        &run_id,
        (agent, steer_support.support),
        turn_ord,
        vault.clone(),
        out.tag
            .apply(json!({}))
            .as_object()
            .cloned()
            .unwrap_or_default(),
    );
    // The turn's usage row, begun once its message is committed (never for
    // a queued turn that did not start) and finished in `finish_observed_run`
    // on every exit. Infallible. A resumed native session is recorded so a
    // Claude resume's cumulative totals can be reduced.
    let mut ledger = UsageRun::begin(
        Meter::for_vault(
            &vault,
            Surface::Chat,
            Some(session_key.clone()),
            Some(run_id.clone()),
        ),
        Some(agent),
        model.clone(),
    );
    // The resumed native session is recorded on the row once the resume
    // plan says the turn really resumes it (Paperclip PR 14b), below.
    // The turn's approved recall, kept in its durable transcript so a saved
    // feedback vote (Paperclip PR 17) can snapshot which learnings were in
    // context. Best effort, like every trace write.
    let paths: Vec<String> = recall.items.iter().map(|item| item.path.clone()).collect();
    let _ = crate::session_store::record_recall(&vault, &session_key, turn_ord, &paths);
    // What `send_to_session` caps a later agent-sent turn at.
    let mode = crate::app_mcp::normalize_perm(permission_mode.as_deref().unwrap_or(""));
    let _ = crate::session_store::record_turn_permission(&vault, &session_key, turn_ord, mode);
    // Hollow queued this turn itself (a wake, a check-in, the weekly update,
    // a wake-up: OB20): its tag, so it never reads as the person's. Else
    // another agent asked it (`hollow_app` start_session / send_to_session):
    // the task-state record says so (K2 follow-up). A request Hollow wrote
    // for the person (overnight 9 A1) carries its own tag.
    if let Some(by) = host_asker.as_deref().or(canned.as_deref()) {
        let _ = crate::session_store::record_turn_asker(&vault, &session_key, turn_ord, by);
    } else if app_depth > 0 {
        let _ = crate::session_store::record_turn_asker(&vault, &session_key, turn_ord, "agent");
    }
    // The constraints a child turn's packet carried join this
    // conversation's task-state record, so its own children inherit them
    // (RF-797-F3). Nothing for a turn without a packet.
    crate::delegation::record_child_packet(&vault, &session_key, turn_ord, app_depth, &prompt);
    // The files the prompt's trailing attachment blocks name, kept so the
    // phone's transcript can show them (phone API v1.2, best effort).
    crate::phone_access::attachments::record_turn(&vault, &session_key, turn_ord, &cwd, &prompt);
    let mut evidence =
        crate::evidence::EvidenceCollector::new("chat", session_id.as_deref(), &run_id);
    evidence.context_mut().attached_sources = attached_sources;
    // The preparation timeline's clock (F8): admission's, and the marks
    // taken before the collector existed.
    evidence.timings_mut().set_admitted(lease.admitted_at());
    evidence.context_mut().timer.0.extend(early_marks);
    // Recall's lifecycle decisions (F7); retrieval adds its own below.
    evidence
        .context_mut()
        .add_lifecycle(recall.history, recall.lifecycle.clone());
    evidence.context_mut().recall_meaning = recall.meaning.recorded();
    // Stamped before the spawn, unlike `source_id`, which cannot be known until
    // the agent's stream announces its id — and never is, if a first turn dies
    // first. That is what leaves such a receipt permanently unqueryable.
    evidence.set_session_key(&session_key);
    // Phase timings (Paperclip PR 15c): admission's baseline first; the
    // agent's phases once `drive` returns, the diff in `finish_observed_run`.
    evidence.timings_mut().baseline_snapshot_ms = lease.baseline_ms();
    // `asked` lets a window that never saw the send (a reload while the turn
    // waited) show what is being answered.
    // `permissionMode`: the mode it runs in, so a window that adopted a turn
    // it didn't send (a check-in, a wake) knows whether its native session
    // may carry on into the person's next turn (2026-11-04).
    // `unattended`: the payload's own flag, nobody watches it (a night item,
    // an isolated wake, an unattended runbook step; v1.1, 2026-11-05).
    // `attended`: the person waits on it ([`turn_attended`]), the only turn
    // whose end the phone API pushes as `turnFinished` (v1.1 review,
    // 2026-11-05). A Main agent wake or check-in says `unattended: false`
    // (its limits come from `is_unattended_run`) and `attended: false`.
    // `rt`: the recall trace, a short key because the app bundle is at its budget.
    out.emit(crate::chat_event::build::run_started(
        &run_id,
        &session_key,
        &recall.items,
        Some(asked.clone()),
        Some(&steer_support),
        Some(mode.to_string()),
        Some(&recall_trace),
        Some(unattended),
        Some(turn_attended(&vault, person_sent, &run_id)),
    ));
    // Usage budgets (Paperclip PR 8a): chat only warns (D2). A warning is
    // delivered once per budget, window and level into the conversation it
    // was claimed for, AFTER that turn's `runStarted`, tagged with its
    // session key so the composer shows it only in that conversation.
    // The turn's claim-time budget also arms a warn-only guard for a Claude
    // turn (Paperclip PR 8b): a level crossed INSIDE the turn (the guard is
    // seeded with every level the baseline met) warns mid-turn, the same
    // way. It reads only per-message stream usage, never the result's
    // session-cumulative `modelUsage`, and never stops the turn (D2).
    let chat_budget = match crate::budgets::preflight(
        &crate::budgets::Store::for_app(&vault),
        &vault,
        &crate::budgets::RunIntent::chat(Some(agent)),
        chrono::Utc::now(),
    ) {
        crate::budgets::Preflight::Warn(budget, notices) => {
            for notice in notices {
                out.emit(crate::chat_event::build::budget_warning(
                    &session_key,
                    &notice,
                ));
            }
            Some(budget)
        }
        crate::budgets::Preflight::Clear(budget) => Some(budget),
        // Chat is never blocked (it is not an enforced intent).
        crate::budgets::Preflight::Blocked(_) => None,
    };
    let chat_guard = chat_budget
        .as_ref()
        .and_then(|budget| crate::budgets::BudgetGuard::for_agent(budget, agent));
    let workspace_context = match workspace_context {
        Ok(context) => context,
        Err(error) => {
            reopen_unstarted_answers(&out, &vault, &session_key, turn_ord);
            return finish_observed_run(
                &out,
                &vault,
                lease,
                evidence,
                &session_key,
                turn_ord,
                ledger,
                live,
                TurnClose::failed(
                    LedgerOutcome::SpawnFailed,
                    session_id,
                    format!("workspace context: {error}"),
                ),
                None,
            )
            .await;
        }
    };
    // Retrieval sees only the user's words (the composer's context blocks
    // would fill its 16-term budget); `directory_connections::context` strips
    // again, idempotently. It runs inside the lease since PR 7b, so the
    // snapshot it reads is the one the run is judged against.
    let retrieval_vault = vault.clone();
    let retrieval_cwd = cwd.clone();
    // Library connectors for this turn (the context prioritizer picks the
    // "when relevant" ones from the user's words and the names of what a
    // reference resolved to). Named in the per-turn prompt, never the
    // appended system prompt.
    let connector_perm = permission_mode
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(crate::app_mcp::unrecorded_mode())
        .to_string();
    let is_work_workspace = workspace_context.is_some();
    // Default to `acceptEdits` so edits stream through without prompts but
    // Bash and unknown tools still ask. The frontend's permission picker
    // overrides this per-chat — `bypassPermissions` is opt-in (danger mode).
    // Each agent maps these canonical modes to its own sandbox/approval flags.
    // With no mode at all: `acceptEdits` in the app, serve's own default
    // there (`app_mcp::unrecorded_mode`).
    let perm = permission_mode
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| crate::app_mcp::unrecorded_mode().to_string());

    // Only Hollow-owned servers are attached (teamwork, computer control,
    // vault search, question cards, task tools; `mcp_attach` decides per
    // surface).
    // agent::isolate_claude supplies strict MCP config for every Claude run,
    // and the preapproved servers' tools run without a prompt
    // (`agent::owned_preapprovals`).
    //
    // The turn's scratch folder (Paperclip PR 15a), made only now that the
    // lease is held. Keyed by the SESSION key, not the run id: Claude lists
    // `--add-dir` paths in its environment block, so a per-run path would
    // change a resumed session's prompt every turn. Removed when this
    // function returns (the processes are gone by then), through a
    // tombstone rename, so the next turn's same-name folder is never hit.
    //
    // The servers, and with them the resume plan below, are settled before
    // retrieval since overnight R1: a follow-up skips retrieval only when
    // its native session really resumes.
    let (_scratch_dir, scratch_spec) =
        crate::scratch::create_for_turn(Some(vault.as_path()), &session_key, &run_id);
    // Tools the managed `claude` build ships for its host client — file
    // delivery and interactive prompts — that have no channel in Hollow's
    // headless, event-streamed chat. Left enabled, the model calls them,
    // they error, and the agent loop re-answers (the "answered twice"
    // artifact + a red tool row). Generated images already render inline
    // via the toolImages path, and there's no UI for AskUserQuestion-style
    // prompts (the model should just ask in plain text), so disallow them.
    // These three host-only tool names are inert compatibility exclusions on other adapters.
    let mut disallowed_tools: Vec<String> = HOST_ONLY_TOOLS.iter().map(|s| s.to_string()).collect();
    // While the computer-control server is attached, close the Bash escape
    // hatch (`osascript`, `open`) so the vetted, confirm-gated verbs are the
    // one path to desktop control — defense-in-depth against prompt-injected
    // detours from synced vault content, not a hard sandbox.
    if agent == AgentKind::Claude && crate::computer_mcp::computer_control_enabled() {
        disallowed_tools.extend(crate::computer_mcp::BASH_DENY.iter().map(|s| s.to_string()));
    }
    // The COO never edits the person's standing orders itself (main agent
    // M4): its own file tools are denied that one file in every turn. That
    // is the only rule its turns add (L6, 11-10: it may do anything the
    // person can), and only on an agent that holds such rules; any agent
    // may run the COO, and its prompt carries the same rule.
    disallowed_tools.extend(coo_denied_tools(&vault, &cwd, agent));
    // A project lead delegates (L8, 11-10): its turns get no shell and edit
    // only its project's notes, on agents that hold such rules; the others
    // carry the same policy in the lead's prompt. The COO keeps every tool.
    disallowed_tools.extend(crate::project_agents::lead_denied_tools(
        &vault,
        &session_key,
        &cwd,
        agent,
    ));
    // SK1: a checkout that isn't the person's (`repo_trust`). Its skills
    // the person hasn't allowed are denied by name on agents that hold deny
    // rules; the brief's label (below) tells every agent.
    // The vault and its workspaces are the person's own folder.
    let in_vault = cwd.starts_with(&vault)
        || std::fs::canonicalize(&cwd)
            .ok()
            .zip(std::fs::canonicalize(&vault).ok())
            .is_some_and(|(c, v)| c.starts_with(v));
    let repo_trust = if bare || in_vault {
        None
    } else {
        crate::repo_trust::for_cwd(&cwd)
    };
    disallowed_tools.extend(repo_trust_denied(repo_trust.as_ref(), agent));
    // Teammates keep these limits (`team_mcp::delegate_grant`).
    let parent = DelegateParent {
        surface: Surface::Chat,
        scope: session_key.clone(),
        run_id: run_id.clone(),
        scratch: scratch_spec.dir().map(Path::to_path_buf),
        tool_limits: crate::team_mcp::ToolLimits::new(&[], &disallowed_tools),
    };
    // Task and app tools at the turn's mode and depth, less what an
    // unattended turn never gets (`turn_servers`).
    // L1: the COO's and a project lead's turns list the session starters
    // from depth 0 whoever messaged them (`app_sessions::start_depth`); the
    // run's own depth, which person-only checks read, is unchanged.
    let start_depth = crate::app_sessions::start_depth(&vault, &session_key, &cwd, app_depth);
    // P1b, P1d review fix: a person's own chat (depth 0, attended, not bare,
    // not the COO's or a lead's): only it is asked by Pause all or paused at
    // a usage limit, since a resume runs as an attended depth 0 chat.
    let own_chat = app_depth == 0
        && !unattended
        && !bare
        && !crate::app_sessions::is_coordinator(&vault, &session_key, &cwd);
    registration.set_own_chat(own_chat);
    let depths = (app_depth, start_depth);
    let (servers, attached) = turn_servers(agent, &perm, &cwd, &vault, &parent, depths, unattended);
    let (servers, caps) = crate::mcp_attach::for_bare(bare, servers, attached);
    // Only the attached servers' NAMES enter the configuration fingerprint
    // (Paperclip PR 14b); their bodies carry per-run values.
    let mcp_servers = crate::chat_resume::mcp_server_names(&servers);
    // How often to draw canvases (Canvas C3 wave 2 C4): this chat's own
    // choice, else Settings, Chat, Canvases. An unreadable settings file
    // reads as the default here; the effort read below fails the turn.
    let chosen = crate::visual_prompt::resolve(
        canvases.as_deref(),
        crate::agent_settings::get_settings()
            .ok()
            .and_then(|settings| settings.canvases)
            .as_deref(),
    );
    // What this native session is started with (Paperclip PR 14b). Stored
    // with each newly announced native id; a resume whose stored fingerprint
    // differs starts a fresh session instead. v3 (OB04) hashes what selects
    // the appended prompt (its caps, Canvases choice and kind) with
    // `PROMPT_CONTRACT`, never its bytes. Each part's hash is stored with
    // it (OB03), so a reset names what changed.
    let prompt_kind = PromptKind::of(is_work_workspace, &vault, &cwd);
    let resume_state =
        crate::session_store::resume_state(&vault, &session_key).unwrap_or_else(|error| {
            eprintln!("[chat] the conversation's stints could not be read: {error}");
            crate::session_store::ResumeState::default()
        });
    // The Mac mini runner (Phase 2): where this turn's agent process runs.
    // A native session lives on the machine that ran it, so the machine is
    // part of the folder the fingerprint hashes: moving between machines
    // starts a fresh native session, re-grounded with the conversation, as
    // switching agents does.
    // Overnight N6: unattended work and a worker batch's chats prefer the
    // runner (Settings, Devices, Overnight and worker chats), only
    // when the turn starts its conversation's native session, so a turn that
    // stays here below never has a session there to resume.
    let prefers = (unattended || worker)
        && crate::remote_session::starts_native_session(session_id.as_deref(), &resume_state);
    // Self-hosted models Q7: a self-hosted turn runs here whatever Runs on
    // says (Hollow serves the model through this Mac's loopback forwarder,
    // which the runner can't reach, and signs in here), decided now, before
    // the stale turn's choice, the fingerprint's folder or any preparation
    // read where it runs; its first line says why (below, once its route
    // resolved and a stale turn was settled).
    let crate::remote_session::TurnPlace {
        chosen: runs_on,
        preferred,
        self_hosted_here,
    } = crate::remote_session::turn_place(
        &vault,
        &cwd,
        agent,
        prefers,
        self_hosted_chat,
        crate::remote_session::PlaceReads::LIVE,
    );
    // Self-hosted models Q5: a turn on `self-hosted:<id>` runs on that
    // registry entry through Hollow's loopback forwarder, with a run key
    // minted for this turn. `self_hosted_run` holds the key until this
    // function returns, however the turn ends (an answer, an error, Stop),
    // and its drop revokes it. A missing, switched-off or unreadable entry,
    // or an agent its API style doesn't run on (Claude for `anthropic`,
    // Codex for `openai`, Q9), ends the turn here with why: it never
    // runs on another model instead. Only the registry file is read and a
    // local listener started; the endpoint isn't contacted until the agent
    // makes its first call. Self-hosted models Q7 review: resolved before
    // a stale remote turn is settled, so the note below (which names the
    // model) can be the turn's first line, and a refused route says only
    // why it was refused.
    let self_hosted_run = match crate::self_hosted::turn_route(agent, model.as_deref()).await {
        Ok(run) => run,
        Err(message) => {
            reopen_unstarted_answers(&out, &vault, &session_key, turn_ord);
            return finish_observed_run(
                &out,
                &vault,
                lease,
                evidence,
                &session_key,
                turn_ord,
                ledger,
                live,
                TurnClose::failed(LedgerOutcome::SpawnFailed, session_id, message),
                None,
            )
            .await;
        }
    };
    // Overnight R1: a remote turn this conversation had under way when an
    // earlier Hollow quit. Continue picks its run up there (through
    // `--attach`, from the bytes that Hollow had handled); any other message
    // first brings that turn's work back (stopping it there), so this turn
    // starts from it. A run that is gone keeps today's salvage. Before the
    // repository snapshot, so that work isn't this turn's in its receipt.
    let mut reattached: Option<crate::remote_session::RemoteTarget> = None;
    // Its return is recorded below, after a self-hosted turn's note.
    let mut earlier_target: Option<crate::remote_session::RemoteTarget> = None;
    let mut earlier_parts = None;
    if let Some(stale) = crate::remote_session::stale_turn(&session_key) {
        let host = stale_host(&runs_on, &preferred);
        let choice =
            crate::remote_session::stale_choice(&stale, &prompt, agent, host.as_deref(), &cwd);
        let key = session_key.to_string();
        let going_there = host.as_deref() == Some(stale.target.host.as_str());
        let settled = tokio::task::spawn_blocking(move || {
            crate::remote_session::settle_stale(&key, stale, choice, going_there)
        })
        .await
        .unwrap_or(crate::remote_session::StaleOutcome::Nothing);
        match settled {
            crate::remote_session::StaleOutcome::Reattach(target) => reattached = Some(target),
            crate::remote_session::StaleOutcome::BroughtBack {
                target,
                worktree,
                folder,
            } => {
                earlier_parts = Some((target.git_worktree.then_some(worktree), folder));
                earlier_target = Some(target);
            }
            crate::remote_session::StaleOutcome::Nothing => {}
        }
    }
    let earlier_back =
        earlier_target
            .as_ref()
            .zip(earlier_parts)
            .map(|(target, (worktree, folder))| RemoteBack {
                host: &target.host,
                worktree,
                folder,
                local_folder: target.folder.as_ref().map(|f| f.local.as_path()),
                earlier: true,
            });
    // Self-hosted models Q7: a self-hosted turn whose Runs on is the runner
    // runs here (`turn_place`), and its first line says so and why, before
    // an earlier turn's return (Q7 review: a stale remote turn is still
    // stopped and brought back first, as for any message, so this turn
    // starts from its work). Whether that turn's record is still there (the
    // runner didn't answer, so a later message tries again) and the chat's
    // start and back notices decide what it says about work left there.
    let stay = self_hosted_here.as_deref().zip(self_hosted_run.as_ref());
    let still_stale = stay.is_some() && crate::remote_session::stale_turn(&session_key).is_some();
    for notice in settled_notices(
        stay,
        earlier_back.as_ref(),
        still_stale,
        || crate::session_store::remote_run_trail(&vault, &session_key, 40).unwrap_or_default(),
        &session_key,
    ) {
        record_remote_notice(&out, &vault, &session_key, turn_ord, notice);
    }
    let fingerprint_cwd: PathBuf = match (&runs_on, &preferred) {
        (Some(Ok(host)), _) | (_, Some(host)) => PathBuf::from(format!("{host}:{}", cwd.display())),
        _ => cwd.to_path_buf(),
    };
    let input_at =
        |canvases: crate::visual_prompt::CanvasFrequency| crate::chat_resume::FingerprintInput {
            agent,
            model: model.as_deref(),
            perm: &perm,
            cwd: &fingerprint_cwd,
            mcp_servers: &mcp_servers,
            caps,
            canvases,
            workspace: prompt_kind.as_str(),
            skills: &[],
        };
    // A child turn gets at most When it helps, steady per native session
    // (overnight 9 A6, `visual_prompt::steady`).
    let canvases = crate::visual_prompt::steady(
        chosen,
        app_depth,
        wants_canvas,
        resume_state
            .open
            .as_ref()
            .and_then(|open| open.config_fingerprint.as_deref()),
        |level| crate::chat_resume::config_fingerprint(&input_at(level)),
    );
    let fingerprint_input = input_at(canvases);
    let fingerprint = crate::chat_resume::config_fingerprint(&fingerprint_input);
    let config_parts = crate::chat_resume::config_parts(&fingerprint_input);
    // Only resume an exact id returned by this adapter; never guess "last".
    let requested = session_id.as_deref().filter(|id| *id != "antigravity");
    // The version restamp (OB04): a stint v1 or v2 stamped resumes when the
    // last turn that ran on its native session had this agent, mode class
    // and model and the conversation's folder is unchanged, read from the
    // store only for such a stint; it is restamped v3 after success.
    let restamp = crate::chat_resume::older_stamp(&resume_state).is_some_and(|open| {
        let previous = open.harness_session_id.as_deref().and_then(|native| {
            crate::session_store::previous_turn(&vault, &session_key, turn_ord, native)
                .ok()
                .flatten()
        });
        crate::chat_resume::may_restamp(
            open,
            agent,
            &perm,
            model.as_deref(),
            &cwd,
            previous.as_ref(),
        )
    });
    let plan =
        crate::chat_resume::plan_resume_with(requested, &resume_state, &fingerprint, restamp);
    // Overnight 9 A9: a large Main-agent chat back after more than an hour
    // away starts fresh (the bundle and task state) instead of re-reading
    // its whole context uncached. Read only for a resume that may rest.
    // Never on an unattended turn (the night queue, a Main-agent wake or
    // check-in): a fresh stint expires the conversation's open question
    // cards, and only the person's own return should do that (A9 review).
    // Nor on a turn another agent sent (RF-810-F1).
    let plan = match plan {
        ResumePlan::Resume(_)
            if prompt_kind == PromptKind::Main
                && agent == AgentKind::Claude
                && perm != "plan"
                && !bare
                && crate::chat_resume::may_rest(
                    agent,
                    &perm,
                    bare,
                    unattended || crate::main_agent::unattended::is_unattended_run(&vault, &run_id),
                    app_depth,
                    true,
                    crate::main_agent::settings().fresh_after_away,
                ) =>
        {
            let away = crate::session_store::last_turn_ended(&vault, &session_key, turn_ord)
                .ok()
                .flatten()
                .and_then(|at| chrono::DateTime::parse_from_rfc3339(at.trim()).ok())
                .map(|at| (chrono::Utc::now() - at.with_timezone(&chrono::Utc)).num_minutes());
            let tokens = crate::context_gauge::stored_tokens(&vault, &session_key);
            crate::chat_resume::with_rest(plan, crate::chat_resume::rests(true, away, tokens))
        }
        plan => plan,
    };
    // What a config-changed reset names (OB03): the parts that differ from
    // the stint it compared, or `unknown`.
    let reset_parts = match &plan {
        ResumePlan::Reset(ResetReason::ConfigChanged) => {
            crate::chat_resume::reset_parts(&resume_state, &config_parts)
        }
        _ => Vec::new(),
    };
    let resume_id = match &plan {
        ResumePlan::Resume(id) => Some(id.clone()),
        ResumePlan::New | ResumePlan::Reset(_) => None,
    };
    // A wake or check-in queries its subject and a short follow-up its
    // conversation's topic (overnight OB18, OB25); one that skips (an empty
    // subject, a follow-up that adds nothing to the native session it
    // really resumes) retrieves nothing, like a bare turn, and keeps its
    // conversation's last connector decision with no Jev call. A follow-up
    // whose plan starts a fresh session retrieves with its prior's terms.
    let retrieval_basis = retrieval_basis.on_plan(resume_id.is_some());
    let no_retrieval = bare || retrieval_basis.basis.skips();
    let retrieval_query = if no_retrieval {
        String::new()
    } else if let Some(query) = &retrieval_basis.query {
        query.clone()
    } else {
        asked.clone()
    };
    let kept_connectors = (!bare && retrieval_basis.basis.skips())
        .then(|| crate::evidence::last_connector_selection(&vault, &session_key))
        .flatten();
    // A follow-up whose conversation has no decision to keep selects as
    // any turn does (Jev is sent only the request as typed).
    let keeps_connectors = retrieval_basis.basis == crate::turn_request::QueryBasis::SkippedHost
        || kept_connectors.is_some();
    // Retrieval and connector selection don't read each other, so they are
    // prepared at once, under one deadline for their optional parts (Jev,
    // the semantic half, the Microsoft 365 refresh), each falling back
    // locally when it passes (agent effectiveness F8, O2). Jev's target is
    // read once for both.
    let deadline = crate::prep_deadline::PrepDeadline::start();
    let jev = crate::jev::Target::app().await;
    // Each carries its own mark from the moment they start together.
    let prep_started = Instant::now();
    let retrieval_future = async {
        let prepared = crate::directory_connections::prioritized_context(
            retrieval_vault,
            retrieval_cwd,
            retrieval_query,
            &deadline,
            &jev,
        )
        .await;
        (prepared, Mark::since(prep_started))
    };
    // A bare turn (overnight S7) selects no connectors: its request
    // (a persona's draft) goes to no ranker or sign-in refresh.
    let connectors_future = async {
        let attached = if bare {
            crate::connectors::Attached::default()
        } else if keeps_connectors {
            crate::connectors::attach_kept(
                agent,
                &connector_perm,
                false,
                &connector_query,
                kept_connectors.as_ref(),
                &deadline,
            )
            .await
        } else {
            // The conversation's last relevance decision may be reused
            // (S28); approvals are read afresh either way.
            crate::connectors::attach(
                agent,
                &connector_perm,
                false,
                &connector_query,
                &deadline,
                &jev,
                Some((vault.as_path(), session_key.as_str())),
            )
            .await
        };
        (attached, Mark::since(prep_started))
    };
    // Paperclip PR 16: snapshot the turn's target git repositories inside the
    // lease and before the spawn, never under HISTORY_LOCK (empty, with no
    // git run, for a vault-root cwd). Its targets come from the vault and
    // the cwd alone, so since S29 it runs beside retrieval and connector
    // selection instead of after prompt assembly, where its git work added
    // to every turn. It keeps its own pre-run budget (`repo_diff`'s limits),
    // never the preparation deadline. Armed only once the agent spawned.
    let repo_snapshot = async {
        let started = Instant::now();
        // CP1: the turn's before checkpoint, beside the snapshot (only in a
        // chat that owns an isolated worktree; quiet otherwise).
        let (baseline, ()) = tokio::join!(
            crate::repo_diff::begin(&vault, &cwd, Some(&*stop)),
            crate::turn_checkpoints::before_turn(
                vault.to_path_buf(),
                session_key.to_string(),
                turn_ord
            )
        );
        (baseline, Mark::since(started))
    };
    // Run 15 S2: connector selection and the repository snapshot settle the
    // agent's command (its servers; the baseline it must never run before),
    // so they are awaited first, with retrieval polled beside them; the
    // agent then starts while retrieval finishes (below).
    tokio::pin!(retrieval_future);
    let mut retrieval_done = None;
    let ((attached, connectors), (repo_baseline, snapshot)) = {
        let settle = async { tokio::join!(connectors_future, repo_snapshot) };
        tokio::pin!(settle);
        loop {
            // Retrieval first, so it starts at once (review S2 F4).
            tokio::select! {
                biased;
                done = &mut retrieval_future, if retrieval_done.is_none() => {
                    retrieval_done = Some(done);
                }
                settled = &mut settle => break settled,
            }
        }
    };
    evidence.set_repo_baseline(repo_baseline);
    evidence
        .context_mut()
        .timer
        .0
        .record(Step::RepoSnapshot, snapshot);
    // What the agent's command is built from, known before retrieval ends:
    // the selected connectors (their servers and approvals), the appended
    // prompt, the toolchain environment and the effort. The selection's
    // notice and records stay below, after retrieval.
    let crate::connectors::Attached {
        connectors: shared,
        selection,
        auth_warm,
        auth_cut,
        set_aside,
        login_out,
    } = attached;
    // Master plan #1: an unattended turn gets no shared connectors; nor does
    // a bare one (overnight S7).
    let shared = crate::connectors::for_turn(unattended || bare, shared);
    // Self-hosted models Q5 review: a claude.ai connector runs on Claude
    // Code's own login, which a self-hosted turn never uses, so it stays
    // out with a note saying why (the command builder refuses one too).
    let (shared, off_login) = if self_hosted_chat {
        crate::connectors::without_claude_login(shared)
    } else {
        (shared, Vec::new())
    };
    let shared_connectors: Vec<agent::SharedConnector> = shared
        .iter()
        .map(|a| agent::SharedConnector {
            id: a.id.clone(),
            auto_approve: a.auto_approve,
            claude_login: a.claude_login,
            name: a.name.clone(),
            remedy: a.remedy.clone(),
        })
        .collect();
    // Library connectors join after the fingerprint's names (above): a
    // different selection next turn never forces a fresh native session.
    let mut servers = servers;
    for attachment in &shared {
        servers.insert(attachment.id.clone(), attachment.entry.clone());
    }
    let mcp_config = crate::mcp_attach::merge(servers);
    // Steer the chat agent to answer in rich inline HTML/canvas by default,
    // and brief it on its environment (vault root + layout, mirror caveats,
    // Downloads, which Hollow MCP capabilities this spawn attached) so it
    // acts like it lives here instead of rediscovering the world each turn.
    // Every adapter carries this brief; Claude has a system-prompt flag and
    // other CLIs receive the same explicit context in their prompt.
    let mut append_layers =
        chat_append_layers(is_work_workspace, &vault, &cwd, caps, canvases, &perm);
    append_layers.brief = with_chat_key(&append_layers.brief, &session_key);
    // COO rebuild L10: a subagent gets the lean profile.
    if is_subagent_turn(&vault, &session_key, subagent_of.as_deref()) {
        append_layers = lean_subagent(append_layers, caps);
    }
    if let Some(project) = crate::project_agents::context(&vault, &session_key) {
        append_layers.brief.push_str(&project);
    }
    // SK1: the repo's untrusted label, last in the brief, so no existing
    // section moves.
    append_layers.brief = with_repo_trust(&append_layers.brief, repo_trust.as_ref());
    // The turn's toolchain environment (OF07). Its PATH entry is
    // `hollow-slot`'s folder: when the tool couldn't be written there is
    // none, and the index doesn't name the tool. The turn's own port (F6
    // part A) stays leased until this function returns, after the agent's
    // process has exited.
    let ToolchainEnv {
        env: toolchain_env,
        port: _turn_port,
    } = build_workspace_env(&vault, &cwd, agent, &perm, &session_key);
    if !toolchain_env.iter().any(|(key, _)| key == "PATH") {
        append_layers.index = append_layers.index.replace(SLOT_INDEX_LINE, "");
    }
    let append_system_prompt = Some(append_layers.joined());
    // The saved reasoning effort (Paperclip PR 14a). Read like the agent
    // itself (an unreadable settings file fails the turn the same way), and
    // passed unvalidated: `effort::resolve` maps it per model. The chat's
    // Ultracode switch (Track U) replaces it for this turn; Claude then needs
    // to know whether its CLI takes ultracode (a free probe, once per binary).
    // COO rebuild L7: otherwise the CFO decides (a requested effort wins;
    // Automatic picks by job kind, Fixed keeps the saved level), and its
    // decision goes in the manifest and on the receipt.
    let cfo_build = build_workspace_folder(&vault, &cwd).is_some();
    let cfo_origin = turn_origin(
        person_sent,
        host_asker.as_deref(),
        app_depth,
        subagent_of.is_some(),
    );
    // Why `start_session` placed this subagent where it is (its first turn).
    let cfo_placement = crate::cfo::take_placement(out.tag.client_turn_id.as_deref());
    let effort = match crate::agent_settings::effort(agent) {
        Ok(_) if ultracode => {
            if agent == AgentKind::Claude {
                // Retrieval keeps going beside the probe, which may wait on
                // a CLI the first time (review S2 F4).
                let probe = crate::agent::claude_ultracode_supported();
                tokio::pin!(probe);
                loop {
                    tokio::select! {
                        biased;
                        done = &mut retrieval_future, if retrieval_done.is_none() => {
                            retrieval_done = Some(done);
                        }
                        _ = &mut probe => break,
                    }
                }
            }
            let (mut decision, _) = turn_cfo(
                &vault,
                agent,
                model.as_deref(),
                &asked,
                cfo_origin,
                cfo_build,
                Some(crate::effort::ULTRACODE.to_string()),
                None,
                cfo_placement.clone(),
            );
            decision.effort_source = crate::cfo::EffortSource::You;
            evidence.set_cfo(decision);
            Some(crate::effort::ULTRACODE.to_string())
        }
        Ok(saved) => {
            let (decision, _account) = turn_cfo(
                &vault,
                agent,
                model.as_deref(),
                &asked,
                cfo_origin,
                cfo_build,
                resolve_requested_effort(&vault, &session_key, requested_effort),
                saved,
                cfo_placement.clone(),
            );
            let effort = decision.effort.clone();
            evidence.set_cfo(decision);
            effort
        }
        Err(error) => {
            reopen_unstarted_answers(&out, &vault, &session_key, turn_ord);
            return finish_observed_run(
                &out,
                &vault,
                lease,
                evidence,
                &session_key,
                turn_ord,
                ledger,
                live,
                TurnClose::failed(
                    LedgerOutcome::SpawnFailed,
                    session_id,
                    format!("agent settings: {error}"),
                ),
                None,
            )
            .await;
        }
    };
    // L1: the effort this turn runs with, for a follow-up and the
    // Subagents list (best effort, like every trace write).
    if let Some(effort) = effort.as_deref() {
        let _ = crate::session_store::record_turn_effort(&vault, &session_key, turn_ord, effort);
    }
    // A build workspace runs in its worktree; its brief, plan and outputs
    // stay writable in its vault folder.
    let extra_dirs: Vec<PathBuf> = build_workspace_folder(&vault, &cwd).into_iter().collect();
    // The Mac mini runner (Phase 2): the worktree's exact state goes to the
    // runner and is checked out there before the command is built. A runner
    // that can't take it fails the turn with why (nothing ran anywhere): the
    // person chose it, so the turn never moves back here unasked. Overnight
    // N6: a turn the unattended default sends (`preferred`) runs here
    // instead, with a notice saying why, when the runner can't take it.
    // Self-hosted models Q7: a self-hosted turn whose Runs on is the runner
    // was kept here above (`turn_place`), so nothing below goes there; its
    // first line, recorded above, says so and why.
    let wanted = match (runs_on, preferred) {
        (Some(chosen), _) => Some((chosen, false)),
        (None, Some(host)) => Some((Ok(host), true)),
        (None, None) => None,
    };
    let mut stayed = false;
    let remote_target = match wanted {
        None => None,
        Some((chosen, preferred)) => {
            let host_name = chosen.as_ref().ok().cloned().unwrap_or_default();
            let blocked = match &chosen {
                Ok(host) if preferred => crate::remote_session::preferred_blocker(
                    host,
                    agent,
                    &cwd,
                    unattended,
                    shared_connectors.iter().any(|c| c.claude_login),
                ),
                _ => None,
            };
            // RT1: the blocker knows its cause; a failed preparation's is
            // read from its error (`remote_session::prepare_cause`).
            let blocked_cause = blocked.as_ref().map(|b| b.cause);
            let prepared = match (chosen, blocked) {
                _ if reattached.is_some() => Ok(reattached.take().unwrap_or_default()),
                (_, Some(why)) => Err(why.message),
                (Err(why), None) => Err(why),
                (Ok(host), None) => {
                    let at = cwd.to_path_buf();
                    // Phase 3: a vault workspace's folder goes too, as the
                    // agent's working folder (a workspace chat) or beside a
                    // build workspace's worktree.
                    let folder =
                        crate::remote_folder::folder_for(&vault, &cwd).map(|(folder, is_cwd)| {
                            (
                                crate::remote_session::FolderRequest {
                                    vault: vault.to_path_buf(),
                                    folder,
                                    session_key: session_key.to_string(),
                                },
                                is_cwd,
                            )
                        });
                    tokio::task::spawn_blocking(move || match folder {
                        Some((request, true)) => {
                            crate::remote_session::prepare_folder(&host, "ssh", &request)
                        }
                        Some((request, false)) => {
                            crate::remote_session::prepare_with(&host, "ssh", &at, Some(&request))
                        }
                        None => crate::remote_session::prepare(&host, "ssh", &at),
                    })
                    .await
                    .unwrap_or_else(|e| Err(format!("the runner's preparation failed ({e})")))
                }
            };
            match prepared {
                Ok(mut target) => {
                    // Remote runners (2026-11-03): what this turn may reach
                    // beyond its checkout, from its workspace's choice (the
                    // secrets themselves are read when the leader is built).
                    // Off the async worker: git, the Keychain, maybe gh.
                    let (v, c) = (vault.to_path_buf(), cwd.to_path_buf());
                    let top = target.git_worktree.then(|| target.worktree.clone());
                    let (access, reach) = tokio::task::spawn_blocking(move || {
                        let access = crate::runner_access::for_turn(&v, &c, top.as_deref());
                        let reach = crate::runner_access::notice(&access);
                        (access, reach)
                    })
                    .await
                    .unwrap_or_default();
                    target.access = access;
                    record_remote_notice(
                        &out,
                        &vault,
                        &session_key,
                        turn_ord,
                        json!({
                            "type": "remoteRun",
                            "phase": "start",
                            "host": target.host,
                            "dir": target.dir,
                            "kept": target.kept,
                            "pruned": target.pruned.as_ref().map(|p| json!({
                                "count": p.count,
                                "freeGb": p.free_gb,
                                "floorGb": p.floor_gb,
                            })),
                            "scope": if target.git_worktree { "worktree" } else { "folder" },
                            "aside": target.aside,
                            // Overnight R1: picking up a run an earlier
                            // Hollow left going there.
                            "reattached": target.reattach.is_some(),
                            // 11-02: commits there carry this Mac's identity
                            // for the checkout; with none here, git's default
                            // there stays, and the notice says so once.
                            "noIdentity": target.git_worktree && target.identity.is_none(),
                            // Remote runners: GitHub and Google Cloud, when
                            // the workspace turned them on (no secret).
                            "access": reach,
                            // Overnight R3: what of the workspace folder
                            // stayed here (secret-looking, over the cap, a
                            // repository of its own), named.
                            "leftHere": target.folder.as_ref().and_then(|f| f.left.notice()),
                            "earlier": target.folder_earlier.as_ref().map(|e| json!({
                                "changed": e.changed.len(),
                                "merged": e.merged,
                                "copies": e.copies,
                                "unsure": e.unsure,
                                "keptHere": e.kept_here,
                                "failed": e.failed,
                                "notBrought": e.not_brought_json(),
                            })),
                            // RT1: why it went there.
                            "route": crate::remote_session::Route::there(
                                &target.host,
                                preferred,
                                target.reattach.is_some(),
                            ),
                            "sessionKey": session_key,
                        }),
                    );
                    crate::remote_session::begin_turn(&session_key, target.clone(), agent);
                    Some(target)
                }
                // Overnight N6: unattended work never fails because the
                // runner can't take it; it runs here and says why.
                Err(message) if preferred => {
                    // RT1: with its route (here, why, a fallback).
                    record_remote_notice(
                        &out,
                        &vault,
                        &session_key,
                        turn_ord,
                        crate::remote_session::stayed_notice(
                            &host_name,
                            blocked_cause,
                            &message,
                            &session_key,
                        ),
                    );
                    stayed = true;
                    None
                }
                Err(message) => {
                    reopen_unstarted_answers(&out, &vault, &session_key, turn_ord);
                    return finish_observed_run(
                        &out,
                        &vault,
                        lease,
                        evidence,
                        &session_key,
                        turn_ord,
                        ledger,
                        live,
                        TurnClose::failed(LedgerOutcome::SpawnFailed, session_id, message),
                        None,
                    )
                    .await;
                }
            }
        }
    };
    // Overnight N6: a preferred turn that stayed here is stamped with this
    // Mac's folder, so its native session resumes here (it is always the
    // conversation's first, so nothing above resumed or reset one).
    let (fingerprint, config_parts) = if stayed {
        let here = crate::chat_resume::FingerprintInput {
            cwd: &cwd,
            ..fingerprint_input.clone()
        };
        (
            crate::chat_resume::config_fingerprint(&here),
            crate::chat_resume::config_parts(&here),
        )
    } else {
        (fingerprint, config_parts)
    };
    // A remote turn's agent learns where it runs, and that Hollow's build
    // tools act on this Mac's copy of the worktree until the turn ends.
    let append_system_prompt = match &remote_target {
        Some(target) => {
            append_system_prompt.map(|p| p + &crate::remote_session::agent_note(target))
        }
        None => append_system_prompt,
    };
    // The turn's spec less its prompt (run 15 S2): the early start below and
    // the first attempt are built from exactly this, the prompt set once the
    // message is composed.
    // A self-hosted turn names its served model (Q5), as its command does,
    // and carries its target with it: both come from one call, so the
    // served id never goes without its target (Q5 review). The
    // fingerprint keeps the chat's own id.
    let (spec_model, self_hosted_target) =
        crate::self_hosted::turn_spec_model(model.as_deref(), self_hosted_run.as_ref());
    let template = agent::RunSpec {
        prompt: String::new(),
        resume: resume_id.clone(),
        model: spec_model,
        perm: perm.clone(),
        cwd: cwd.clone(),
        allowed_tools: Vec::new(),
        disallowed_tools: disallowed_tools.clone(),
        interactive: true,
        // An Antigravity turn on the runner or a self-hosted model runs
        // `agy -p` (`agent::antigravity_acp`), which takes no servers: none
        // are handed over rather than the run refused.
        mcp_config: if agent == AgentKind::Antigravity
            && (remote_target.is_some() || self_hosted_target.is_some())
        {
            None
        } else {
            mcp_config
        },
        append_system_prompt: append_system_prompt.clone(),
        process_scope: agent::ProcessScope::GuiOwned,
        usage_stream: false,
        max_budget_usd: None,
        extras: agent::RunExtras {
            mcp_run_tag: None,
            effort,
            // Never part of the fingerprint above (argv and env are not hashed).
            scratch: Some(scratch_spec.clone()),
            shared_connectors: shared_connectors.clone(),
            // Affinity records picks; it never outranks headroom.
            account_affinity: Some(format!("chat:{session_key}")),
            // Only a retry pins its already-ranked target. Initial selection
            // happens after refresh, independently of the CFO's earlier view.
            account: None,
            // A chat is not long work (#16), except unattended work: a night
            // turn, a subagent whatever its `unattended` flag, a check-in or
            // wake (COO rebuild L7). Its account is picked away from one past
            // the CFO's line, never kept there by the conversation's affinity.
            long_work: unattended || !cfo_origin.attended(),
            extra_dirs: extra_dirs.clone(),
            // A build workspace's own build folders, port and toolchain caps,
            // and the `hollow-slot` tool (build programme PR B7); any other
            // turn that can take a slot, the caps and `hollow-slot` (OF07;
            // built before the prompt, above); then the compaction window
            // from Settings, when set (OB22).
            env: toolchain_env
                .into_iter()
                .chain(crate::agent_settings::compaction_env(agent))
                .collect(),
            // Codex, Antigravity and Grok: the resumed native session holds
            // the appended prompt (G2); only while the spec resumes. Set
            // with the prompt below.
            configured_held: false,
            // The message names the mode; the policy doesn't (HB01).
            mode_per_turn: true,
            // Mid-turn steering: an agent with a true steer reads its
            // messages from stdin, so the person can steer it mid-turn.
            live_input: steer_support.support == crate::steering::SteerSupport::Native,
            // The Mac mini runner's folder, when the turn runs there.
            remote: remote_target,
            // A self-hosted model's endpoint (Q1), from the model (Q5).
            self_hosted: self_hosted_target,
            // OC1: Settings' ACP switch; `agent::grok_acp` decides per mode.
            grok_acp: agent == AgentKind::Grok && crate::agent_settings::acp_enabled(agent),
            antigravity_acp: agent == AgentKind::Antigravity,
            // AG4b: an ask-first Claude turn's gated tool calls defer to an
            // approval card; the answers wait in the turn's scratch folder.
            defer_answers: (agent == AgentKind::Claude)
                .then(|| scratch_spec.dir().map(|dir| dir.join(DEFER_ANSWERS_FILE)))
                .flatten(),
            // H8: the first turn of a native-resume import forks the source session.
            fork_resume: agent == AgentKind::Claude
                && resume_id.as_deref().is_some_and(|id| {
                    crate::session_import::native_resume::fork_pending(&vault, &session_key, id)
                }),
        },
    };
    // The first attempt's command build (and Codex's inventory inside it):
    // a steerable Claude turn's runs here, its agent started at once and
    // left to its own start-up while retrieval finishes (run 15 S2); any
    // other turn's is the last preparation step, its spawn at once (F8).
    let first_command: std::sync::Mutex<Option<(Mark, Option<crate::codex_inventory::Inventory>)>> =
        std::sync::Mutex::new(None);
    let build_first = |spec: &agent::RunSpec| {
        let (cmd, built, inventory) = crate::prep_timing::timed_command(agent, spec);
        if let Ok(mut first) = first_command.lock() {
            first.get_or_insert((built, inventory));
        }
        cmd
    };
    // Never once a stop was requested: the turn will end before its first
    // attempt, as it always has (review S2 F1).
    agent::refresh_accounts(agent, &template).await;
    let mut early = if stop.requested() {
        None
    } else {
        EarlyStart::start(agent, &template, &run_id, build_first)
    };
    // An agent that ran may have changed the repository, so its baseline is
    // armed now, whether or not the agent ever gets its prompt (review S2
    // F3; idempotent with the attempt loop's).
    if early.is_some() {
        evidence.arm_repo_baseline();
    }
    let started_early = early.is_some();
    let (retrieved, retrieval) = match retrieval_done {
        Some(done) => done,
        None => loop {
            // A stop while retrieval finishes ends the early agent at once;
            // the turn still ends before its first attempt (review S2 F1).
            tokio::select! {
                biased;
                done = &mut retrieval_future => break done,
                _ = stop.wait(), if early.is_some() => {
                    if let Some(early) = early.take() {
                        evidence.add_process_stop(early.end(&Timings::default()).await);
                    }
                }
            }
        },
    };
    // What the query was read from (OB18, OB25), on every chat turn but a
    // bare one, whether retrieval succeeded or not.
    let query_basis = (!bare).then(|| retrieval_basis.basis.as_str().to_string());
    let retrieved = match retrieved {
        // What retrieval considered and kept goes on this run's own context
        // manifest (agent effectiveness F11), never a process-global report.
        Ok(mut prepared) => {
            prepared.retrieval.query_basis = query_basis;
            let manifest = evidence.context_mut();
            manifest.preparation.retrieval_ms = Some(retrieval.ms);
            manifest.timer.0.record(Step::Retrieval, retrieval);
            manifest.set_retrieval(prepared.retrieval, prepared.jev);
            manifest.add_lifecycle(prepared.history, prepared.lifecycle);
            prepared.text
        }
        Err(error) => {
            if query_basis.is_some() {
                evidence.context_mut().retrieval = Some(crate::run_manifest::RetrievalRecord {
                    query_basis,
                    ..Default::default()
                });
            }
            // The early agent goes before the turn's changes are captured
            // and its lease released (review S2 F2).
            if let Some(early) = early.take() {
                evidence.add_process_stop(early.end(&Timings::default()).await);
            }
            reopen_unstarted_answers(&out, &vault, &session_key, turn_ord);
            return finish_observed_run(
                &out,
                &vault,
                lease,
                evidence,
                &session_key,
                turn_ord,
                ledger,
                live,
                TurnClose::failed(
                    LedgerOutcome::SpawnFailed,
                    session_id,
                    format!("retrieval failed: {error}"),
                ),
                None,
            )
            .await;
        }
    };
    // Spread usage across accounts (2026-10-21): connectors set to Always that the
    // turn left out because Claude Code's login is out of room (none for a
    // turn that gets no connectors at all). A row once per conversation and
    // window, and the agent's note every turn.
    // A self-hosted turn (Q5 review) names every claude.ai connector it
    // left out in its own note instead: none of them can come back by name.
    let (set_aside, off_login) = if unattended || bare {
        (Vec::new(), Vec::new())
    } else if self_hosted_chat {
        let mut names = off_login;
        for name in set_aside {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        (Vec::new(), names)
    } else {
        (set_aside, off_login)
    };
    let set_aside_reason = login_out
        .as_ref()
        .filter(|_| !set_aside.is_empty())
        .map(|low| {
            if crate::claude_accounts::first_set_aside_notice(&session_key, low) {
                announce_account_notice(
                    &out,
                    &vault,
                    &session_key,
                    turn_ord,
                    &low.set_aside_message(&set_aside),
                );
            }
            low.set_aside_reason()
        });
    // Everything from here to the layers below is prompt assembly (F8).
    let assembly_started = Instant::now();
    {
        let manifest = evidence.context_mut();
        manifest.preparation.connectors_ms = Some(connectors.ms);
        manifest.timer.0.record(Step::Connectors, connectors);
        manifest.timer.0.record_some(Step::MicrosoftAuth, auth_warm);
        // `.selection` (one decision per "when relevant" candidate, with its
        // reason) goes on the run's context manifest (agent effectiveness F11).
        manifest.set_connectors(selection);
        manifest.set_deadline(&deadline, auth_cut);
        manifest.capabilities.connectors = shared.iter().map(|a| a.id.clone()).collect();
    }
    let connectors_note = format!(
        "{}{}{}",
        crate::connectors::prompt_note(&shared, &connector_perm, false),
        crate::connectors::set_aside_note(&set_aside, set_aside_reason.as_deref()),
        crate::connectors::self_hosted_set_aside_note(&off_login)
    );
    let evidence_text = if no_retrieval {
        String::new()
    } else {
        retrieved
    };
    // The turn's relevant skill index (F9, wave 4 · V2): local and
    // deterministic, from the person's own words, in the turn's message
    // (never the appended prompt), with how this run reads a skill.
    let mut skill_index = turn_skills(&crate::skill_index::Roots::for_vault(&vault), &asked, &caps);
    if bare {
        skill_index = crate::skill_index::TurnSkills {
            block: String::new(),
            record: None,
        };
    }
    // Target paths of question cards resolve against the run's own folder.
    let run_cwd = cwd.clone();
    let requested_model = model.clone();
    // The host context compiler (agent effectiveness F4) decides what the
    // body carries besides the caller's text, for every entry point: the
    // conversation bundle and the operator profile when the native session
    // starts fresh, from the session store and `me/`. The turn's own wrapper
    // stays around it. A failed resume retried inside the turn compiles
    // again, fresh.
    let operator_enabled = crate::operator::is_enabled() && !bare;
    let entry = crate::context_compiler::Entry::for_depth(app_depth);
    // How a fresh session can read the earlier turns its task-state record
    // summarizes (F4, K2): hollow_app's read_session when this run has it.
    let reader = if mcp_servers
        .iter()
        .any(|name| name == crate::app_mcp::SERVER_NAME)
    {
        crate::task_state::Reader::AppTools
    } else {
        crate::task_state::Reader::None
    };
    // The visual guide (F6, G1): written to the turn's scratch folder,
    // where the capability index names it, and added to a body whose
    // request looks visual (or when the file couldn't be written), once
    // per native session; never part of the appended prompt.
    let guide = TurnGuide::new(
        canvases,
        crate::visual_prompt::Asking::of(caps.questions, caps.questions_fenced),
        visual_cue,
        scratch_spec.dir(),
    );
    // What every prompt of this turn is built from, the fresh-session
    // retry's too (Paperclip PR 14b): the turn's wrapper (workspace context,
    // approved recall, the connectors note and the retrieved evidence)
    // around the body the context compiler builds.
    // The configured half (the appended prompt and the workspace policy),
    // as the adapters place it.
    let appended = append_system_prompt.clone().unwrap_or_default();
    // The mode-free chat policy (HB01); the mode is the turn's line below.
    let policy_text = crate::agent::chat_policy(agent, &perm);
    // The envelope cut to a byte limit, when the turn's budget needs it.
    let envelope_cwd = cwd.clone();
    let rebuild = |max: usize| {
        crate::workspaces::context_for_within(&vault, &envelope_cwd, &source_request, max)
            .ok()
            .flatten()
    };
    // The local time this turn starts (overnight A2): agents get the date
    // from their CLI but no clock, and wrote made-up times into packets and
    // plans. In the message, never the appended prompt, which Claude caches
    // as its prefix.
    let turn_clock = clock_line(&chrono::Local::now());
    // Repo features R2a: the context gauge keeps quiet for a plan, bare or
    // unattended turn; R2b: so does the start-of-turn context note, which
    // a resumed session gets once its last turn ended past a step it wasn't
    // told. Before the gauge registers, which reads the step it stores.
    let quiet = gauge_quiet(mode, bare, unattended, || {
        crate::main_agent::unattended::is_unattended_run(&vault, &run_id)
    });
    let turn_context = turn_context_note(&vault, &session_key, &plan, quiet);
    // The turn's permission mode (HB01, evening 10-16), in the message for
    // the same reason: in the configured half it reset the native session
    // whenever a Main-agent wake (acceptEdits) and the person's turn
    // (bypass) alternated. The latest line in the conversation applies: a
    // resumed turn whose session holds this exact line leaves it out (S02),
    // and a changed mode's line is sent.
    // An agent whose reads stop at its folders is told which they are
    // (OB02): the working directory, the scratch folder (both named; the
    // brief has their paths) and any other `--add-dir` folder (`extra_dirs`,
    // above).
    let reads = crate::mode_contract::Reads {
        scratch: scratch_spec.dir().is_some(),
        extra: &extra_dirs,
        app: caps.app,
        vault: caps.vault,
    };
    let turn_mode = crate::mode_contract::turn_line(
        &perm,
        crate::agent::capabilities(agent).owned_mcp,
        caps.app && !caps.app_read_only,
        caps.computer,
        crate::capability_contract::reads_only_its_roots(agent).then_some(&reads),
    );
    let composer = TurnComposer {
        vault: &vault,
        session_key: &session_key,
        continued_from: &continued_from,
        host: host_asker.is_some(),
        turn_ord,
        entry,
        operator_enabled,
        reader,
        composed: &prompt,
        guide: &guide,
        agent,
        append: &appended,
        policy: &policy_text,
        workspace: workspace_context.as_ref(),
        rebuild: &rebuild,
        native: resume_id.as_deref(),
        recall: &recall.prompt_block,
        connectors_note: &connectors_note,
        skills: &skill_index.block,
        evidence: &evidence_text,
        clock: &turn_clock,
        context_note: &turn_context,
        mode: &turn_mode,
    };
    // What the native session will hold of the stable layers (G2), from
    // the last prompt composed: the retry's replaces the first's. Written
    // once the agent ran, with the id it announced.
    let pending = Mutex::new(None);
    // The visual guide the native session will hold, from the last prompt
    // composed; recorded only once the agent spawned (wave 3 review of G1).
    let guide_given = Mutex::new(None);
    // A follow-up that skipped retrieval on a resumed session (OB25)
    // retrieves when a failed resume retries in a fresh one: the broker's
    // order, no Jev; its record replaces the skipped one after the attempts.
    let refetched: Mutex<Option<crate::run_manifest::RetrievalRecord>> = Mutex::new(None);
    // Q3: what a self-hosted turn's prompts are cut to fit (the retry's too).
    let fit_target = template.extras.self_hosted.clone();
    let fit_append = template.append_system_prompt.as_deref().map_or(0, str::len);
    let retry_prompt = || {
        use crate::turn_request::QueryBasis;
        emit_context_handoff(&out, &session_key, &run_id, "pending", 0);
        // Never on a bare turn: it gets no vault context at all.
        let skipped = !bare && retrieval_basis.basis == QueryBasis::SkippedFollowup;
        let fresh = skipped.then(|| {
            let fetch = || {
                crate::directory_connections::local_context(
                    &vault,
                    &run_cwd,
                    retrieval_basis.query.as_deref().unwrap_or_default(),
                    crate::prep_deadline::PrepDeadline::start().at,
                )
            };
            // This closure runs on an async worker: tell the runtime it
            // blocks (a current-thread runtime, in tests, can't be told).
            match tokio::runtime::Handle::try_current().map(|h| h.runtime_flavor()) {
                Ok(tokio::runtime::RuntimeFlavor::MultiThread) => {
                    tokio::task::block_in_place(fetch)
                }
                _ => fetch(),
            }
        });
        let given = match &fresh {
            Some(fresh) => fresh.text.as_str(),
            None => composer.evidence,
        };
        let retrying = TurnComposer {
            evidence: given,
            ..composer
        };
        let turn = retrying.compose(crate::context_compiler::Session::Retry);
        let entries = if turn.record.continuity.sent {
            turn.record.continuity.entries
        } else {
            0
        };
        if let Some(mut fresh) = fresh {
            fresh.retrieval.query_basis = Some(QueryBasis::FollowupPrior.as_str().into());
            *refetched.lock().unwrap_or_else(PoisonError::into_inner) = Some(fresh.retrieval);
        }
        // Q3: a cut prompt may have lost a layer, so none counts as given.
        let (prompt, cut) = match fit_target.as_ref() {
            Some(target) => crate::self_hosted::fit_pack(&turn.prompt, fit_append, target),
            None => (turn.prompt, None),
        };
        let handoff_cut = cut.is_some();
        if let Some(note) = cut {
            eprintln!("[self-hosted] the retry's prompt was cut: {note}");
            *pending.lock().unwrap_or_else(PoisonError::into_inner) = None;
            *guide_given.lock().unwrap_or_else(PoisonError::into_inner) = None;
        } else {
            *pending.lock().unwrap_or_else(PoisonError::into_inner) = Some(turn.pending);
            *guide_given.lock().unwrap_or_else(PoisonError::into_inner) = turn.guide_given;
        }
        emit_context_handoff(
            &out,
            &session_key,
            &run_id,
            if entries > 0 && !handoff_cut {
                "complete"
            } else {
                "unavailable"
            },
            if handoff_cut { 0 } else { entries },
        );
        (prompt, turn.record)
    };
    let compile_started = Instant::now();
    let handoff = !bare
        && crate::context_compiler::Session::of(&plan).fresh()
        && (turn_ord > 1 || !continued_from.is_empty());
    if handoff {
        emit_context_handoff(&out, &session_key, &run_id, "pending", 0);
    }
    let composed = composer.compose(crate::context_compiler::Session::of(&plan));
    let handoff_entries = if composed.record.continuity.sent {
        composed.record.continuity.entries
    } else {
        0
    };
    let compile_mark = Mark::since(compile_started);
    if let Some(message) = &composed.notice {
        out.emit(crate::chat_event::build::panel(
            &session_key,
            "hollow:context",
            json!({ "message": message }),
        ));
    }
    *pending.lock().unwrap_or_else(PoisonError::into_inner) = Some(composed.pending.clone());
    *guide_given.lock().unwrap_or_else(PoisonError::into_inner) = composed.guide_given.clone();
    // Before the first spawn (S02 review): from here the message may reach
    // the resumed session whatever ends the turn (a failover whose spawn
    // fails after the first attempt wrote it, Hollow quitting mid-turn), so
    // a per-turn layer this prompt carries in another text (a flipped mode
    // line) stops counting as given now. A turn that never runs only costs
    // a resend; a finished one writes its own record over this.
    let reached = [resume_id.as_deref()];
    crate::stable_layers::unsettle(
        &vault,
        &session_key,
        turn_ord,
        &composed.pending,
        &reached,
        false,
    );
    let first_prompt = composed.prompt;
    let configured_held = composed.configured_held;
    // The run's context layers (agent effectiveness F11), sizes and hashes
    // only, in the order the agent reads them: the configured half as sent,
    // then the message the composer built around the composed (or rebuilt)
    // body.
    {
        let manifest = evidence.context_mut();
        if let Some(sources) = composed.attached_sources {
            manifest.attached_sources = Some(sources);
        }
        // A per-turn layer the resumed session holds as is was left out
        // (S02), so it is no layer of this prompt.
        let left_out = composed.left_out;
        record_turn_layers(
            manifest,
            &TurnLayers {
                append: &append_layers,
                configured_held,
                workspace: composed.workspace.as_deref(),
                recall: left_out.sent(LeftOut::RECALL, &recall.prompt_block),
                clock: &turn_clock,
                context_note: &turn_context,
                mode: left_out.sent(LeftOut::MODE, &turn_mode),
                connectors_note: &connectors_note,
                skills: left_out.sent(LeftOut::SKILLS, &skill_index.block),
                evidence: &evidence_text,
                wrapped: &composer.wrap(left_out, composed.workspace.as_deref(), ""),
            },
            &first_prompt,
        );
        manifest.capabilities.owned_servers = mcp_servers.clone();
        manifest.skills = skill_index.record.clone();
        manifest.compiler = Some(crate::run_manifest::CompilerRecord {
            reset_parts: reset_parts.iter().map(|part| part.to_string()).collect(),
            ..composed.record
        });
        if let ResumePlan::Reset(reason) = &plan {
            manifest.fallback(
                "session",
                format!(
                    "fresh native session with the conversation rebuilt ({})",
                    reason.as_str()
                ),
            );
        }
        manifest.timer.0.record(Step::ContextCompile, compile_mark);
        manifest
            .timer
            .0
            .record(Step::PromptAssembly, Mark::since(assembly_started));
    }
    // The turn's spec: the template the early start was built from (run 15
    // S2), with the composed prompt and whether the resumed session holds
    // the appended prompt (G2; Codex, Antigravity and Grok, only while the
    // spec resumes). Neither is in a steerable Claude turn's command.
    let mut spec = template;
    spec.prompt = first_prompt;
    spec.extras.configured_held = configured_held;
    // Q3: a self-hosted model's turn fits its window: a context pack too big
    // for it is cut in the middle with a note, never sent whole.
    let mut handoff_cut = false;
    if let Some(target) = fit_target.as_ref() {
        let (fitted, note) = crate::self_hosted::fit_pack(&spec.prompt, fit_append, target);
        if let Some(note) = note {
            handoff_cut = true;
            evidence.context_mut().fallback("self-hosted window", note);
            spec.prompt = fitted;
            // A cut prompt may have lost a layer, so none counts as given.
            *pending.lock().unwrap_or_else(PoisonError::into_inner) = None;
            *guide_given.lock().unwrap_or_else(PoisonError::into_inner) = None;
        }
    }
    if handoff {
        emit_context_handoff(
            &out,
            &session_key,
            &run_id,
            if handoff_entries > 0 && !handoff_cut {
                "complete"
            } else {
                "unavailable"
            },
            if handoff_cut { 0 } else { handoff_entries },
        );
    }
    // What the turn asked for and what its agent gets (Track U), on the
    // receipt: `ultracode → xhigh` with the reason when the CLI lacks it.
    if let Some(effort) = turn_effort(agent, &spec) {
        evidence.set_effort(effort);
    }
    // OB23: the Bash output cap its command carries (env, never hashed).
    let bash_cap = agent::bash_output_cap(agent, &spec);
    evidence.context_mut().capabilities.bash_output_max_chars = bash_cap;
    // OB22: the compaction window it carries (env or Codex's `-c`).
    evidence.context_mut().capabilities.compact_window = agent::compact_window(agent, &spec);
    // S13: what Claude Code loads by itself (CLAUDE.md, auto memory, skills).
    evidence
        .context_mut()
        .set_native_memory(agent, &spec.cwd, spec.extras.remote.is_some());
    // S26: notes opened outside it are the receipt's `notesOpened`.
    evidence.context_mut().set_run_cwd(&spec.cwd);
    let agent_name = agent.as_str();
    let cx = TurnCx {
        vault: &vault,
        session_key: &session_key,
        turn_ord,
        run_id: &run_id,
        run_cwd: &run_cwd,
        agent,
        fingerprint: &fingerprint,
        config_parts: &config_parts,
        reset_parts: &reset_parts,
        reset_announced: std::sync::atomic::AtomicBool::new(false),
        quiet,
        closes_cards: person_sent,
    };
    // The step spine's first model step (master plan F6).
    let _ = crate::step_spine::turn_started(&vault, &session_key, turn_ord, &run_id);
    match &plan {
        // A resumed Claude session's totals are session-cumulative; the row
        // reduces them.
        ResumePlan::Resume(id) => ledger.set_resumed(Some(id.clone())),
        ResumePlan::New => {}
        // The requested session was started with another configuration (or
        // a fresh attempt for it never announced its id): a fresh one now.
        ResumePlan::Reset(reason) => announce_reset(&out, &cx, *reason),
    }
    // Overnight 9 A9: the run record says why it started fresh.
    if let ResumePlan::Reset(ResetReason::Rested {
        away_minutes,
        tokens,
    }) = plan
    {
        let label = agent.label().to_string();
        evidence.record_switch(crate::evidence::ReceiptSwitch {
            kind: crate::evidence::SWITCH_RESTED.into(),
            reason: crate::chat_resume::rested_text(away_minutes, tokens)
                .unwrap_or_else(|| "after more than an hour away".into()),
            from: label.clone(),
            to: label,
        });
    }
    // L9: only unattended work is bound by the CFO's budgets.
    let cfo_watch = if cfo_origin.attended() {
        None
    } else {
        crate::cfo::budgets::Watch::new(&vault, &session_key, Some(Path::new(&cwd)))
    };
    let mut watch = TurnWatch {
        guard: chat_guard,
        delivered: Vec::new(),
        cfo: cfo_watch,
    };
    let new_ledger = || {
        UsageRun::begin(
            Meter::for_vault(
                &vault,
                Surface::Chat,
                Some(session_key.clone()),
                Some(run_id.clone()),
            ),
            Some(agent),
            requested_model.clone(),
        )
    };
    // The silent-run indicator and the no-output watchdog (Paperclip PR
    // 15b): one activity clock for the turn, attached to the live record so
    // `agent_session_live` reports it (and the approval broker can pause it
    // while a card waits), stamped by `drive` on every stdout and stderr
    // line, and reported every 15 s as an `activity` chat event by a ticker
    // that is dropped as soon as the agent has finished. The watchdog and
    // the optional deadline are the agent's own settings, the same path for
    // every agent (merged plan slice 3, A·R2; before it a Claude turn ran
    // with neither).
    let activity = Arc::new(crate::run_activity::RunActivity::new());
    crate::agent_sessions::attach_activity(&run_id, Arc::clone(&activity));
    // Self-hosted models Q11: a self-hosted model may be cold, 4 to 6
    // quiet minutes before its first byte, so its turn (a bake-off
    // variant's too) waits at least one request's timeout before the
    // watchdog or a deadline ends it.
    let limits = crate::self_hosted::cold_start_limits(
        crate::agent_settings::run_limits(agent),
        self_hosted_run.as_ref().map(|run| &run.target),
    );
    let stall_after = limits.stall_after;
    let turn_watch = process_group::Watch {
        activity: Arc::clone(&activity),
        stall_after,
    };
    let ticker = {
        let app = app.clone();
        let tag = out.tag.clone();
        let run_id = run_id.clone();
        let session_key = session_key.clone();
        crate::run_activity::spawn_ticker(
            Arc::clone(&activity),
            crate::run_activity::TICK,
            move |snapshot| {
                emit(
                    &app,
                    tag.apply(activity_event(&run_id, &session_key, &snapshot)),
                );
            },
        )
    };
    // Turn guards: an unattended (night) turn weighs its running cost
    // against the overnight stop at each safe point. Inactive unless the
    // night queue's drain lists this conversation as a running item.
    // Claude only: its stream reports per-message usage the price table
    // covers (Codex reports none mid-turn; other agents aren't priced).
    let night_watch = (unattended && agent == AgentKind::Claude).then(|| NightWatch {
        spend: &crate::night_queue::SPEND,
        key: &session_key,
        run: &run_id,
    });
    let attempts = drive_turn_attempts(
        &out,
        &cx,
        &stop,
        Timings::default(),
        &mut spec,
        &plan,
        retry_prompt,
        |spec: &agent::RunSpec| {
            let (cmd, built, inventory) = crate::prep_timing::timed_command(agent, spec);
            if let Ok(mut first) = first_command.lock() {
                first.get_or_insert((built, inventory));
            }
            cmd
        },
        new_ledger,
        &mut ledger,
        &mut evidence,
        &mut watch,
        Some(&turn_watch),
        limits.deadline,
        &LiveAccounts,
        night_watch.as_ref(),
        early,
    )
    .await;
    // A skipped follow-up's fresh retry retrieved after all (OB25).
    if let Ok(Some(record)) = refetched.into_inner() {
        evidence.context_mut().retrieval = Some(record);
    }
    // Spawn, first output and turn (Paperclip PR 15c), read from the
    // lease's admission clock and the turn's activity clock together, now
    // that the processes are gone. Nothing is recorded for a turn that never
    // spawned. An agent that started early (run 15 S2) is clocked from its
    // prompt's hand-over, where `drive` takes it: Preparation runs to the
    // hand-over and Model started from it; its first output came before the
    // prompt and was read only at the hand-over, so it is not reported (the
    // manifest's `spawn` step spans the early start instead).
    let mut first_outputs = activity.first_outputs().for_agent(agent);
    if started_early {
        first_outputs.any = None;
        first_outputs.stdout = None;
    }
    evidence.timings_mut().record_drive(
        lease.admitted_elapsed_ms(),
        activity.since_spawn_ms(),
        first_outputs,
    );
    if let Some((built, inventory)) = first_command.lock().ok().and_then(|first| *first) {
        let manifest = evidence.context_mut();
        manifest.timer.0.record(Step::Command, built);
        manifest.set_codex_inventory(inventory);
        manifest
            .timer
            .0
            .spawn_called(crate::prep_timing::end_of(&built));
    }
    // The agent has finished: no more activity to report.
    drop(ticker);
    // P1a: a paused attempt is kept as paused only if no Stop came first,
    // decided while the stop button can still reach this turn.
    let pause_kept = attempts.paused().and_then(|(note, native)| {
        settle_pause(
            &vault,
            &session_key,
            &stop,
            agent.as_str(),
            native,
            note,
            chrono::Utc::now().timestamp(),
        )
    });
    crate::budgets::record_conversation_notices(
        &crate::budgets::Store::for_app(&vault),
        &vault,
        &watch.delivered,
        chrono::Utc::now(),
    );

    let (driven, stream, agent_runs, retried, failover_stop, r1_stopped, paused, guard_stop) =
        match attempts {
            AttemptsEnd::StoppedBeforeSpawn => {
                evidence.set_result_kind(crate::result_kind::ResultKind::Cancelled);
                settle_unstarted_answers(&out, &cx);
                return finish_observed_run(
                    &out,
                    &vault,
                    lease,
                    evidence,
                    &session_key,
                    turn_ord,
                    ledger,
                    live,
                    TurnClose::cancelled(
                        LedgerOutcome::Cancelled,
                        "before-spawn",
                        cancelled_before_spawn_event(),
                        resume_id,
                    ),
                    None,
                )
                .await;
            }
            AttemptsEnd::SpawnFailed {
                message,
                session_id: done_id,
            } => {
                // The agent never ran: an answer this turn carried comes back.
                settle_unstarted_answers(&out, &cx);
                return finish_observed_run(
                    &out,
                    &vault,
                    lease,
                    evidence,
                    &session_key,
                    turn_ord,
                    ledger,
                    live,
                    TurnClose::failed(LedgerOutcome::SpawnFailed, done_id, message),
                    None,
                )
                .await;
            }
            AttemptsEnd::StoppedBeforeRetry { driven, stream } => {
                // The failed resume's held announcements are dropped with its
                // stream; that invocation failed, so its row is an error, while
                // the turn ends as the user's stop. No fresh session was started.
                drop(stream);
                evidence.add_process_stop(driven.outcome);
                evidence.set_result_kind(crate::result_kind::ResultKind::Cancelled);
                settle_unstarted_answers(&out, &cx);
                return finish_observed_run(
                    &out,
                    &vault,
                    lease,
                    evidence,
                    &session_key,
                    turn_ord,
                    ledger,
                    live,
                    TurnClose::cancelled(
                        LedgerOutcome::Error,
                        "before-retry",
                        cancelled_event(None),
                        None,
                    ),
                    None,
                )
                .await;
            }
            AttemptsEnd::Driven {
                driven,
                stream,
                agent_runs,
                retried,
                failover_stop,
                night_stop,
                budget_stop,
                paused,
            } => {
                // R1 stopped: every Claude account it could move to is out.
                let r1_stopped =
                    failover_stop.is_some() && night_stop.is_none() && budget_stop.is_none();
                // P1d: a night or budget guard ended it, never the limit.
                let guard_stop = night_stop.is_some() || budget_stop.is_some();
                // L9: a budget stop's note is what the turn ends with.
                let failover_stop = budget_stop.or_else(|| {
                    close_night_stop(
                        &vault,
                        &session_key,
                        night_stop,
                        failover_stop,
                        chrono::Utc::now(),
                    )
                });
                (
                    driven,
                    stream,
                    agent_runs,
                    retried,
                    failover_stop,
                    r1_stopped,
                    paused,
                    guard_stop,
                )
            }
        };
    let TurnStream {
        proposed_plan,
        results,
        reported_failure,
        assistant: assistant_text,
        asks,
        last_session_id,
        announced,
        state: parse_state,
        last_quota,
        model_replied,
        ..
    } = stream;
    // Claude compacted the session's history in this turn (S02 review): a
    // per-turn layer it held may be summarized away, so none counts as
    // given and the next turn sends them all. This turn itself may run
    // without the mode line until then.
    let compacted = parse_state.compacted;
    // OB22: how many times, on the receipt and in the manifest.
    evidence.set_compactions(parse_state.compactions);
    // The agent ran, before its outcome is known (S02 review): again, with
    // what only the run knows (the id it announced, a compaction, a fresh
    // retry's prompt), so a stopped, stalled or failed turn leaves no
    // changed or compacted layer counted as given. A finished turn records
    // its own layers below.
    if agent_runs {
        if let Some(pending) = pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            let reached = [announced.as_deref(), resume_id.as_deref()];
            crate::stable_layers::unsettle(
                &vault,
                &session_key,
                turn_ord,
                pending,
                &reached,
                compacted,
            );
        }
    }

    if driven.missing_stdout {
        evidence.add_process_stop(driven.outcome);
        settle_unstarted_answers(&out, &cx);
        return finish_observed_run(
            &out,
            &vault,
            lease,
            evidence,
            &session_key,
            turn_ord,
            ledger,
            live,
            TurnClose::failed(
                LedgerOutcome::Error,
                last_session_id,
                "no stdout from agent".to_string(),
            ),
            None,
        )
        .await;
    }
    if !agent_runs {
        // Only the stand-in ran; it prints the build error and exits 1, which
        // the normal failure path below reports. The answer comes back.
        settle_unstarted_answers(&out, &cx);
    } else if let Some(given) = guide_given
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
    {
        // The agent spawned and read the last composed prompt: its native
        // session now holds the visual guide that prompt carried (G1). A
        // turn stopped before spawn, failing to spawn or printing nothing
        // returned above and records nothing, so the next turn resends it.
        given.record(&vault, &session_key, turn_ord);
    }

    // From here every exit keeps the reply in the transcript, whole (the
    // funnel writes it); learning reads it without its `hollow-next` block.

    evidence.add_process_stop(driven.outcome);
    evidence.set_leftovers(driven.left_running());
    evidence.set_source_id(last_session_id.as_deref());
    // P1a: the chat paused at a safe point. Its native session is kept and
    // recorded as paused (`chat_pause`), so a resume continues it with full
    // context; the turn closes neutral, never as an error, and nothing
    // fails over, retries or follows up.
    // A Stop that landed after the pause ended the attempt wins
    // (`settle_pause`): the turn closes as stopped, and nothing is paused.
    if let Some(note) = paused {
        evidence.set_result_kind(crate::result_kind::ResultKind::Cancelled);
        return finish_observed_run(
            &out,
            &vault,
            lease,
            evidence,
            &session_key,
            turn_ord,
            ledger,
            live,
            if let Some(reason) = pause_kept {
                TurnClose::paused(
                    LedgerOutcome::Cancelled,
                    note,
                    &session_key,
                    last_session_id,
                )
                .paused_for(reason, None)
            } else {
                TurnClose::cancelled(
                    LedgerOutcome::Cancelled,
                    driven.outcome.map_or("unknown", |o| o.as_str()),
                    cancelled_event(driven.outcome),
                    last_session_id,
                )
            }
            .with_reply(assistant_text),
            None,
        )
        .await;
    }
    let result_record = results.record();
    // The no-output watchdog (Paperclip PR 15b) or the turn's deadline
    // (merged plan slice 3) ended the turn: an error, recorded once as the
    // `stalled` or `timedOut` result kind (cross-m34 M6: no separate receipt
    // line), so the receipt says which, with `result_kind::stalled_message`
    // or `deadline_message`, a `timeout` usage row, and never continued. The
    // process tree was ended like a stop. A user's Stop pressed at the same
    // moment wins in `drive` (it is checked first).
    let quiet_minutes = crate::run_activity::stall_minutes(stall_after);
    let deadline_minutes = crate::run_activity::stall_minutes(limits.deadline);
    let own_limit = match driven.ended {
        Ended::Stalled => Some((
            crate::result_kind::ResultKind::Stalled,
            crate::result_kind::stalled_message(quiet_minutes),
        )),
        Ended::TimedOut => Some((
            crate::result_kind::ResultKind::TimedOut,
            crate::result_kind::deadline_message(deadline_minutes),
        )),
        Ended::Exited | Ended::Stopped | Ended::LimitReached => None,
    };
    if let Some((kind, message)) = own_limit {
        evidence.set_result_kind(kind);
        out.emit(crate::chat_event::build::error_kind(
            message.clone(),
            kind.as_str(),
        ));
        return finish_observed_run(
            &out,
            &vault,
            lease,
            evidence,
            &session_key,
            turn_ord,
            ledger,
            live,
            TurnClose {
                status: "error",
                outcome: usage_ledger::outcome_for(driven.ended, false),
                assistant: assistant_text,
                event: Some(("error", message.clone())),
                notice: None,
                error: Some(message),
                session_id: last_session_id,
            },
            None,
        )
        .await;
    }
    if driven.ended == Ended::Stopped {
        evidence.set_result_kind(crate::result_kind::ResultKind::Cancelled);
        let stop_state = driven.outcome.map(|outcome| outcome.as_str());
        // Receipt first (finish_observed_run emits it), then the neutral stop
        // notice; the pump emits `done`, which ends the turn in the UI.
        return finish_observed_run(
            &out,
            &vault,
            lease,
            evidence,
            &session_key,
            turn_ord,
            ledger,
            live,
            TurnClose::cancelled(
                LedgerOutcome::Cancelled,
                stop_state.unwrap_or("unknown"),
                cancelled_event(driven.outcome),
                last_session_id,
            )
            .with_reply(assistant_text),
            None,
        )
        .await;
    }

    let status = match driven.status {
        Some(Ok(status)) => status,
        wait_failure => {
            let error = match wait_failure {
                Some(Err(error)) => error.to_string(),
                _ => "exit status unavailable".to_string(),
            };
            let message = format!("wait {agent_name}: {error}");
            evidence.set_result_kind(crate::result_kind::ResultKind::Error);
            return finish_observed_run(
                &out,
                &vault,
                lease,
                evidence,
                &session_key,
                turn_ord,
                ledger,
                live,
                TurnClose::failed(LedgerOutcome::Error, last_session_id, message)
                    .with_reply(assistant_text),
                None,
            )
            .await;
        }
    };
    let stderr_text = driven.stderr_tail;

    // Paperclip PR 14a: classify how the turn ended, record it on the
    // receipt, and label the error once (see `exit_outcome`).
    let result_kind = crate::result_kind::classify(&crate::result_kind::ResultFacts {
        agent,
        ended: driven.ended,
        exit_success: status.success(),
        result: result_record.as_ref(),
        stderr_tail: &stderr_text,
        budget_stop: false,
        stalled_minutes: None,
    });
    evidence.set_result_kind(result_kind);
    let outcome = exit_outcome(
        result_kind,
        result_record.as_ref(),
        status.success(),
        reported_failure,
        || {
            if stderr_text.trim().is_empty() {
                format!("{agent_name} exited with {status}")
            } else {
                format!("{agent_name} exited with {status}: {}", stderr_text.trim())
            }
        },
    );
    // R1: every account is out of room (or the turn already moved once):
    // that, not the exit's own text, is what the turn ends with. A usage
    // limit no failover moved says when the agent has room again (repo
    // ideas 10-27 #8): the soonest added account's reset after R1's stop,
    // else the attempt's own last rate-limit event, else the reset the
    // provider's text carries. A self-hosted endpoint that refuses for load
    // (Q5) is not Claude's usage running out: its own error shows, and the
    // turn never continues on another agent.
    let quota_kind =
        result_kind == crate::result_kind::ResultKind::ProviderQuota && self_hosted_run.is_none();
    let ran_out = r1_stopped || quota_kind;
    // P1d keeps the time itself too: a limit pause resumes at it.
    let reset_at = if ran_out {
        limit_reset_at(
            r1_stopped,
            crate::claude_accounts::soonest_room,
            last_quota.as_ref(),
            &[
                outcome.error.as_deref(),
                result_record
                    .as_ref()
                    .and_then(crate::result_kind::ResultRecord::error_text)
                    .as_deref(),
            ],
            chrono::Utc::now().timestamp(),
        )
    } else {
        None
    };
    let resets = reset_at.and_then(|at| crate::chat_resume::reset_label(at, &chrono::Local::now()));
    let failover_stop = failover_stop.or_else(|| {
        quota_kind.then(|| {
            crate::agent_out::limit_message(
                agent.label(),
                resets.as_deref(),
                outcome.error.as_deref(),
            )
        })
    });
    // The person chose another agent for when one runs out, and this turn
    // changed nothing: the pump continues it there after `done`.
    let mut continued_elsewhere = false;
    if ran_out && outcome.run_status == "error" && !stop.requested() {
        let facts = crate::agent_out::OutTurn {
            agent,
            ran_out,
            tool_calls: evidence.tool_calls_seen(),
            person_sent,
            continued: continued_out,
        };
        let to = crate::agent_out::continuation(
            &facts,
            crate::agent_settings::when_out(),
            &crate::agent::available(),
        );
        if let Some(to) = to {
            continued_elsewhere = true;
            crate::agent_out::plan(out_turn.onto(to, resets.clone()));
        }
    }
    // P1d: a turn that ran out pauses instead of failing (nothing lost, its
    // native session kept), resuming at the reset when the provider gave
    // one. It closes as a P1a pause; Stop and Resume work on it as on any.
    // Review fix: credit or billing exhaustion is not a usage window: it
    // keeps the old failure with the provider's words.
    let credit = [
        outcome.error.as_deref(),
        result_record
            .as_ref()
            .and_then(crate::result_kind::ResultRecord::error_text)
            .as_deref(),
        Some(stderr_text.as_str()),
    ]
    .into_iter()
    .flatten()
    .any(crate::result_kind::is_credit_exhaustion);
    let limit = crate::chat_pause::limit_pause(&crate::chat_pause::LimitFacts {
        ran_out,
        every_account: r1_stopped,
        failed: outcome.run_status == "error",
        stopped: stop.requested(),
        continued_elsewhere,
        guard_stop,
        own_chat,
        native: last_session_id.as_deref(),
        agent_label: agent.label(),
        credit,
        reset_at,
        reset_label: resets.as_deref(),
    });
    if let Some(limit) = limit {
        // Review fix (P1d, item 7): a limit hit right after an automatic
        // resume, before any reply or tool, counts that attempt; past the
        // cap the chat stays paused with no time and a notification.
        // Delta review: progress is a real model reply or a tool call, never
        // the provider's limit text in an API-error envelope.
        let progressed = evidence.tool_calls_seen().is_some_and(|n| n > 0) || model_replied;
        let (limit, attempts, gave_up) = crate::chat_pause::count_auto_attempt(
            limit,
            agent.label(),
            out.tag.client_turn_id.as_deref(),
            progressed,
        );
        // Review fix (P1d, item 5): recorded under the slot lock while this
        // turn's registration still holds it, as `settle_pause`: a Stop that
        // came first closes the turn as stopped with no record.
        if let Some(kept) = settle_limit_pause(
            &vault,
            &session_key,
            &stop,
            agent.as_str(),
            last_session_id.as_deref(),
            &limit,
            attempts,
            chrono::Utc::now().timestamp(),
        ) {
            if kept && gave_up {
                crate::chat_pause::notify_gave_up(&vault, &session_key);
            }
            drop(registration);
            return finish_observed_run(
                &out,
                &vault,
                lease,
                evidence,
                &session_key,
                turn_ord,
                ledger,
                live,
                if kept {
                    TurnClose::paused(
                        LedgerOutcome::Cancelled,
                        limit.note,
                        &session_key,
                        last_session_id,
                    )
                    .paused_for(crate::chat_pause::PauseReason::UsageLimit, limit.resume_at)
                } else {
                    TurnClose::cancelled(
                        LedgerOutcome::Cancelled,
                        driven.outcome.map_or("unknown", |o| o.as_str()),
                        cancelled_event(driven.outcome),
                        last_session_id,
                    )
                }
                .with_reply(assistant_text),
                None,
            )
            .await;
        }
    }
    // The processes are gone; the stop button no longer has a turn to reach.
    // Held until here so a stop also reaches a retried attempt and (review
    // fix, P1d) the limit pause's settle above. Every earlier exit drops it
    // with the function, after its lease, the order `AdmittedTurn` declares.
    drop(registration);
    let outcome = failover_outcome(outcome, failover_stop);
    let run_status = outcome.run_status;
    if perm == "plan"
        && !bare
        && (run_status == "success"
            || result_kind == crate::result_kind::ResultKind::PermissionDenied)
    {
        if let Some(text) = proposed_plan {
            out.emit(crate::chat_event::build::panel(
                &session_key,
                "hollow:proposedPlan",
                json!({ "runId": run_id, "text": text }),
            ));
        }
    }
    let (stored, emitted) = exit_error_records(&outcome, result_kind);
    // Stored by the funnel, as the transcript's closing event.
    let stored = stored.map(|msg| ("error", msg.to_string()));
    if let Some(event) = emitted {
        out.emit(event);
    }
    let error = outcome.error;
    // The agent took the last composed prompt: its native session now
    // holds those stable layers (G2), recorded under the id this turn's
    // stream announced. A turn that failed, was stopped or announced
    // nothing records nothing more than the per-turn layers it unsettled
    // above, so its layers are never counted as given.
    let pending = pending
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    if let Some(mut held) = pending {
        // A reply the agent finished, as for a fenced question card.
        let finished = run_status == "success"
            || result_kind == crate::result_kind::ResultKind::PermissionDenied;
        if let Some(native) =
            stable_layers_holder(agent_runs, finished, announced.as_deref(), &held.native)
        {
            held.native = native.to_string();
            // After a compaction (R1), the next resumed turn re-grounds the
            // agent with the task-state record; the next turn that finishes
            // writes its own record, clearing the mark.
            if compacted {
                crate::stable_layers::forget_layers(&mut held);
                held.compacted_turn = turn_ord;
            }
            crate::stable_layers::write(&vault, &session_key, turn_ord, &held);
        }
    }

    // Antigravity has no owned MCP: its question card is the last fenced
    // `hollow-questions` block of a finished reply (a success, or one whose
    // actions the permission mode denied; never a failed or stopped one),
    // one per run.
    // A bare turn (overnight S7) never asks the person anything.
    if !bare && fenced_card_allowed(agent, run_status, result_kind) {
        if let Some(arguments) = asks.fenced(&assistant_text) {
            interactions::persist_and_notify(
                &vault,
                &NewInteraction {
                    session_key: &session_key,
                    turn_ord,
                    run_id: &run_id,
                    agent,
                    native_session_id: asks.native(),
                    tool_use_id: crate::interaction_schema::FENCED_TOOL_USE_ID,
                    source: InteractionSource::Fenced,
                    arguments: &arguments,
                    cwd: &run_cwd,
                    vault_root: &vault,
                },
                &mut |payload| out.emit(payload),
            );
        }
    }
    // A legacy stint (NULL fingerprint) that just resumed successfully learns
    // its configuration now: the resumed session re-announces the same id,
    // so the id was never recorded again (Paperclip PR 14b).
    if run_status == "success" && !retried && resume_id.is_some() {
        if let Some(native) = last_session_id.as_deref() {
            let _ = crate::session_store::stamp_config_fingerprint(
                &vault,
                &session_key,
                native,
                &fingerprint,
            );
            // A stint v1 or v2 stamped that the version restamp let resume
            // is overwritten with v3 (OB04), so the check runs once.
            if restamp {
                let _ = crate::session_store::restamp_config_fingerprint(
                    &vault,
                    &session_key,
                    native,
                    &fingerprint,
                );
            }
            // Its parts (OB03), when this turn's fingerprint is the stored
            // one and none are stored yet: a stint stamped before them, or
            // just stamped above, names what changed at its next reset.
            let _ = crate::session_store::stamp_config_parts(
                &vault,
                &session_key,
                native,
                &fingerprint,
                &config_parts,
            );
        }
    }
    // The one exit that can succeed, so the only one that hands over the
    // request a successful turn learns from.
    finish_observed_run(
        &out,
        &vault,
        lease,
        evidence,
        &session_key,
        turn_ord,
        ledger,
        live,
        TurnClose {
            status: run_status,
            outcome: usage_ledger::outcome_for(driven.ended, run_status == "success"),
            assistant: assistant_text,
            event: stored,
            notice: None,
            error,
            session_id: last_session_id,
        },
        learns.then_some(learn_from),
    )
    .await
}

/// Durable-learning extraction is deliberately post-turn and detached: it
/// cannot delay or fail the user's original chat. The extractor is tool-less
/// and only enqueues review items (through `review_queue`'s attributed host
/// writer, so it flags no later run); accepted context is a separate explicit
/// action in Settings. The pump calls this after the turn's `done`, for
/// every chat turn: one without a job (cancelled, failed or never started)
/// ends its capture turn, so the chat is not left in flight.
pub(crate) fn spawn_learning(
    app: &Core,
    vault: &Path,
    session_key: &str,
    run_id: &str,
    job: Option<LearningJob>,
) {
    let Some(job) = job else {
        // M1 PR2: migrate (learning_capture takes the GUI's handle).
        crate::learning_capture::ended(app.gui(), vault, session_key, run_id);
        return;
    };
    crate::learning_capture::finished(
        app.gui(),
        vault,
        session_key,
        crate::learning_capture::Input {
            user: job.user,
            references: job.references,
            assistant: job.assistant,
            run_id: job.run_id,
            self_hosted: job.self_hosted,
            material_change: job.changed,
        },
    );
}

/// A live chat row as `agent_session_live` reports it when no turn holds the
/// live record: a turn still waiting in the queue (`queued`, with an empty
/// run id), or one the pump is between steps on. `continueArtifactSession`
/// refuses while it is set, so Resume session cannot replace a queued turn's
/// key. `None` when no chat row is live or the queue cannot be read.
pub(crate) fn queued_live_session(
    vault: &Path,
    key: Option<&str>,
) -> Option<crate::types::LiveAgentSession> {
    let rows = run_queue::live_rows(vault).ok()?;
    let row = rows.iter().find(|row| {
        row.source == RunSource::Chat && key.is_none_or(|key| row.source_id.as_deref() == Some(key))
    })?;
    // Working, not Queued (2026-11-16): a claimed row, or a queued one
    // nothing truly holds back, is `preparing`; only a row with a real wait
    // (a chat-count limit, memory, another run's folder or lane) is `queued`.
    let working = row.status == RowStatus::Claimed
        || (row.status == RowStatus::Queued
            && crate::run_pump::item_with_reason(
                vault,
                row,
                None,
                &crate::run_pump::QueueFacts::read(vault, &rows),
                chrono::Utc::now(),
            )
            .reads_working());
    let key = row.source_id.clone()?;
    let agent = row
        .payload
        .get("agent")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let (phase, run_id) = match row.status {
        RowStatus::Running => (
            crate::agent_sessions::SessionPhase::Running,
            row.history_run_id.clone().unwrap_or_default(),
        ),
        _ if working => (
            crate::agent_sessions::SessionPhase::Preparing,
            String::new(),
        ),
        _ => (crate::agent_sessions::SessionPhase::Queued, String::new()),
    };
    Some(crate::types::LiveAgentSession {
        key,
        agent,
        run_id,
        phase: phase.as_str().to_string(),
        phase_entered_at: row
            .claimed_at
            .clone()
            .unwrap_or_else(|| row.requested_at.clone()),
        failure_reason: None,
        // The queue row knows nothing of the agent's output; a running turn's
        // activity comes from the live record (Paperclip PR 15b).
        last_event_at: None,
        progress: None,
    })
}

/// Max images surfaced from a single tool result, and the per-file byte cap.
/// Bounds the IPC payload so a chatty result can't flood the channel.
const MAX_RESULT_IMAGES: usize = 6;
pub(crate) const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;

/// Pull absolute image-file paths out of a teamwork tool result, read each one
/// that exists on disk, and return them as `data:` URLs the chat UI can render
/// inline. This is how a delegated "generate an image" shows the picture in the
/// chat instead of just a path — the bytes go straight to the renderer (which
/// already displays `data:` images for attachments) and never through the
/// model's context. Bounded in count + per-file size.
fn extract_result_images(content: &str) -> Vec<String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use regex::Regex;
    static IMG_PATH_RE: Lazy<Regex> = Lazy::new(|| {
        // Absolute path ending in a common image extension. The char class
        // stops at whitespace and the usual trailing delimiters so a path
        // inside prose / a markdown link is captured cleanly.
        Regex::new(r#"(?i)/[^\s)\]}'"`]+\.(?:png|jpe?g|gif|webp|bmp|svg)"#).unwrap()
    });
    // Normalize `file://` URIs to plain absolute paths first.
    let cleaned = content.replace("file://", " ");
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for m in IMG_PATH_RE.find_iter(&cleaned) {
        if out.len() >= MAX_RESULT_IMAGES {
            break;
        }
        let path = m.as_str();
        if !seen.insert(path.to_string()) {
            continue;
        }
        let p = std::path::Path::new(path);
        let Ok(meta) = std::fs::metadata(p) else {
            continue;
        };
        if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_IMAGE_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(p) else {
            continue;
        };
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let mime = sniff_image_mime(&bytes, &ext);
        out.push(format!("data:{};base64,{}", mime, STANDARD.encode(&bytes)));
    }
    out
}

/// Detect an image's real MIME from its magic bytes — teammates sometimes save
/// JPEG bytes under a `.png` name (Gemini image gen does this) — falling back
/// to the file extension when the signature isn't recognized.
fn sniff_image_mime(bytes: &[u8], ext: &str) -> &'static str {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        return "image/png";
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return "image/jpeg";
    }
    if bytes.starts_with(b"GIF8") {
        return "image/gif";
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return "image/webp";
    }
    if bytes.starts_with(&[0x42, 0x4D]) {
        return "image/bmp";
    }
    match ext {
        "svg" => "image/svg+xml",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => "image/png",
    }
}

/// Run an arbitrary `claude <args>` subcommand non-interactively and return
/// the combined stdout+stderr. Used by the chat's `/mcp` and `/plugin`
/// local slash commands so the user can manage MCPs / plugins from inside
/// the app without dropping to a terminal.
///
/// `cwd` is critical for scope-sensitive subcommands like `mcp add` —
/// `claude` registers the server under the project that owns the current
/// directory, so we set it to the chat's working dir so user expectations
/// line up (the chat is "in" project X, so additions go to project X).
pub async fn run_admin(args: Vec<String>, cwd: Option<PathBuf>) -> Result<String, String> {
    if !matches!(
        args.iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice(),
        ["mcp", "list"] | ["plugin", "list"] | ["plugin", "marketplace", "list"]
    ) {
        return Err("Hollow only lists agent integrations. Manage external connectors in the agent's own app.".into());
    }
    let bin = find_claude_binary().ok_or_else(|| "claude binary not found".to_string())?;
    let mut cmd = Command::new(&bin);
    for a in &args {
        cmd.arg(a);
    }
    if let Some(p) = cwd {
        cmd.current_dir(p);
    }
    // Bounded, in its own process group, and settled like an agent run so a
    // hung claude (or anything it started) can't outlive the IPC call.
    let output = process_group::run_to_completion(&mut cmd, None, ADMIN_TIMEOUT, None)
        .await
        .map_err(|e| format!("spawn claude: {e}"))?;
    if output.timed_out {
        return Err(format!("claude {} timed out", args.join(" ")));
    }
    let status = match output.status {
        Some(status) => status.to_string(),
        None => "unknown".to_string(),
    };
    let success = output.status.is_some_and(|status| status.success());
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = output.stderr;
    let mut combined = stdout;
    if !stderr.trim().is_empty() {
        if !combined.is_empty() && !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push_str(&stderr);
    }
    if combined.trim().is_empty() {
        combined = format!("(claude {} exited {status})", args.join(" "));
    }
    if !success && !combined.contains("exited") {
        combined.push_str(&format!("\n(exit {status})"));
    }
    Ok(combined)
}

/// Stop a chat turn (Paperclip PR 7b: keyed). With a session key: a running
/// turn of that session is signalled (the turn keeps reading the agent's
/// events while it ends every process it started, finishes history and the
/// receipt, emits `cancelled`, and the pump emits `done` once the row is
/// terminal); a turn still waiting in the queue is taken out of it, and this
/// emits its `cancelled{queued:true}` and `done` (the only emits here); a
/// turn the pump has claimed but not started is flagged, and the pump ends
/// it before it starts. Only the process that owns the run queue touches
/// queue rows (`run_pump::accepting`), as `cancel_queued_run` does. A
/// missing, empty or unsafe key stops nothing and is an error
/// ([`stop_running_turn`]). The stop slot's lock is released before the
/// queue is touched. `Ok(true)` when something was stopped (a running turn
/// signalled, a queued one withdrawn, a claimed one flagged), `Ok(false)`
/// when the conversation had nothing queued or running (the phone API's
/// `{"stopped": false}`).
pub async fn cancel(
    app: impl Into<Core>,
    vault: PathBuf,
    session_key: Option<String>,
) -> Result<bool, String> {
    let app = app.into();
    let asked = session_key.clone();
    let stopped = stop_running_turn(session_key)?;
    // P1a review fix: a Stop always wins over a pause, after the running
    // turn was signalled (`settle_pause` sees that signal, or wrote its
    // record before it, which this clears). A paused chat is unpaused.
    if let Some(key) = asked.as_deref().map(str::trim) {
        stop_clears_pause(&vault, key);
    }
    let Some(key) = stopped else {
        return Ok(true);
    };
    // Only the queue owner ends queue rows (as `cancel_queued_run`): a
    // second process ending the owner's queued turn would leave the owner's
    // composer waiting for events it never gets.
    crate::run_pump::accepting()?;
    let lookup = key.clone();
    let withdrawn =
        tauri::async_runtime::spawn_blocking(move || withdraw_queued(&vault, &lookup, Utc::now()))
            .await
            .map_err(|e| e.to_string())??;
    let stopped = match withdrawn {
        Withdrawn::Cancelled { events, item } => {
            for event in events {
                emit(&app, event);
            }
            crate::run_pump::emit(&app, "cancelled", item);
            run_queue::wake();
            true
        }
        Withdrawn::StopRequested { item } => {
            crate::run_pump::emit(&app, "cancelled", item);
            run_queue::wake();
            true
        }
        // It started between the two checks: its stop slot is registered now.
        Withdrawn::Running => cancel_running(&key),
        Withdrawn::Nothing => false,
    };
    Ok(stopped)
}

/// P1a: a Stop of `key` ends its pause: the paused record is cleared, so it
/// is never resumed. True when there was one. Logged, never fatal.
pub(crate) fn stop_clears_pause(vault: &Path, key: &str) -> bool {
    if !crate::agent_sessions::is_safe_session_key(key) {
        return false;
    }
    crate::chat_pause::clear(vault, key).unwrap_or_else(|error| {
        eprintln!("[chat pause] stop {key}: {error}");
        false
    })
}

/// The AppHandle-free first half of [`cancel`]: refuse a key that names no
/// conversation, else signal that conversation's running turn. `Ok(None)`
/// when a running turn was signalled, `Ok(Some(key))` when none is (the
/// queue is next). Fails closed (review A·R9): a missing, empty or unsafe key
/// used to fall back to signalling EVERY registered turn, so a malformed
/// stop from any caller ended other conversations' turns. No caller wants
/// that: the composer's Stop and `sessions.stop` always send a key.
fn stop_running_turn(session_key: Option<String>) -> Result<Option<String>, String> {
    let Some(key) = session_key.filter(|key| crate::agent_sessions::is_safe_session_key(key))
    else {
        return Err("Stop needs the conversation's session key, and this request had none (or not a valid one). Nothing was stopped.".into());
    };
    // Repo ideas 10-27 #8: a Stop also cancels a continuation on another
    // agent, planned or about to be sent, whatever the turn's state.
    crate::agent_out::note_stop(&key);
    if cancel_running(&key) {
        return Ok(None);
    }
    Ok(Some(key))
}

/// What a keyed cancel did to that session's queued chat row.
#[derive(Debug)]
pub(crate) enum Withdrawn {
    /// No live chat row for the key.
    Nothing,
    /// A queued row ended `cancelled`; `events` end the turn in the webview.
    Cancelled {
        events: Vec<Value>,
        item: crate::types::RunQueueItem,
    },
    /// A claimed row was flagged; the pump ends it before it starts.
    StopRequested { item: crate::types::RunQueueItem },
    /// The row is running; stop its registered turn.
    Running,
}

/// The sync, AppHandle-free half of a keyed [`cancel`] (tests drive it).
/// The row's end events are built from the row as read BEFORE the cancel,
/// because ending it clears its payload (and with it the `clientTurnId`).
pub(crate) fn withdraw_queued(
    vault: &Path,
    key: &str,
    now: DateTime<Utc>,
) -> Result<Withdrawn, String> {
    let Some(row) = run_queue::live_rows(vault)?
        .into_iter()
        .find(|row| row.source == RunSource::Chat && row.source_id.as_deref() == Some(key))
    else {
        return Ok(Withdrawn::Nothing);
    };
    let item = |vault: &Path| -> Result<crate::types::RunQueueItem, String> {
        let current = run_queue::get(vault, row.id)?.unwrap_or_else(|| row.clone());
        Ok(run_queue::to_item(&current, None))
    };
    let outcome = run_queue::cancel(vault, row.id, now)?;
    send_prep::cancel(vault, row.id);
    match outcome {
        CancelOutcome::Cancelled => Ok(Withdrawn::Cancelled {
            events: {
                // P1a: a withdrawn resume gives its claim back.
                crate::chat_pause::release_for_row(vault, &row, false);
                queued_end_events(&row, &QueuedEnd::Cancelled)
            },
            item: item(vault)?,
        }),
        CancelOutcome::StopRequested => Ok(Withdrawn::StopRequested { item: item(vault)? }),
        CancelOutcome::Running { .. } => Ok(Withdrawn::Running),
        CancelOutcome::SkipRequested | CancelOutcome::AlreadyFinished => Ok(Withdrawn::Nothing),
    }
}

/// Stop run `run_id`'s child task turns ([`stop_child_turns`]) and tell
/// the webview, as a keyed [`cancel`] of each would: a queued one's end
/// events and the queue's `cancelled`. Returns each turn's queue row and
/// what happened to it.
fn stop_child_turns_now(app: &Core, vault: &Path, run_id: &str) -> Vec<(i64, &'static str)> {
    let mut out = Vec::new();
    for (id, outcome, withdrawn) in stop_child_turns(vault, run_id, Utc::now()) {
        match withdrawn {
            Withdrawn::Cancelled { events, item } => {
                for event in events {
                    emit(app, event);
                }
                crate::run_pump::emit(app, "cancelled", item);
                run_queue::wake();
            }
            Withdrawn::StopRequested { item } => {
                crate::run_pump::emit(app, "cancelled", item);
                run_queue::wake();
            }
            Withdrawn::Running | Withdrawn::Nothing => {}
        }
        out.push((id, outcome));
    }
    out
}

/// The AppHandle-free half of a stopped turn's stop reaching the child task
/// turns run `run_id` queued (agent effectiveness F10, D2;
/// `descendants::child_tasks`): a queued one leaves the queue
/// (`cancelled`), a claimed one is flagged so the pump ends it before it
/// starts (`stopRequested`), a running one is stopped as the composer's
/// Stop would (`signalled`; its row ends `cancelled` when it has), and one
/// that already ended is left alone (`finished`). Only the turn the run
/// queued is touched, never a later turn of that conversation. A normal
/// finish stops nothing: a child task outlives the turn that started it.
pub(crate) fn stop_child_turns(
    vault: &Path,
    run_id: &str,
    now: DateTime<Utc>,
) -> Vec<(i64, &'static str, Withdrawn)> {
    let mut out = Vec::new();
    for record in crate::descendants::child_tasks(vault, run_id) {
        let (Some(id), Some(key)) = (record.child_queue_id, record.child_session.as_deref()) else {
            continue;
        };
        let live =
            crate::descendants::child_row(vault, &record).is_some_and(|row| row.status.is_live());
        if !live {
            out.push((id, "finished", Withdrawn::Nothing));
            continue;
        }
        // A conversation has at most one live chat turn, so the key's live
        // row is this one.
        let stopped = match withdraw_queued(vault, key, now) {
            Ok(Withdrawn::Cancelled { events, item }) => {
                ("cancelled", Withdrawn::Cancelled { events, item })
            }
            Ok(Withdrawn::StopRequested { item }) => {
                ("stopRequested", Withdrawn::StopRequested { item })
            }
            Ok(Withdrawn::Running) if cancel_running(key) => ("signalled", Withdrawn::Running),
            Ok(_) => ("finished", Withdrawn::Nothing),
            Err(error) => {
                eprintln!("[chat] stopping child turn {id} of run {run_id}: {error}");
                ("finished", Withdrawn::Nothing)
            }
        };
        out.push((id, stopped.0, stopped.1));
    }
    out
}

/// Persist a pasted/dropped chat image to disk so the next `claude -p` turn
/// can Read it (the Read tool renders images natively). Saved into a hidden
/// `.hollow-attachments/` folder INSIDE the chat working dir, so the path is
/// always within Claude's working directory (Read needs no extra grant) and
/// the folder is pruned from the vault tree/search. We generate the filename
/// ourselves (no caller-controlled path component) → no path-traversal surface.
/// Returns the absolute path to embed in the prompt.
///
/// Paperclip PR 19: a folder the send would refuse as unsafe (`/`, `/tmp`,
/// bare `~`, …; never the configured `vault`) is refused here too, before
/// anything is written, so a paste can't leave `~/.hollow-attachments/`
/// behind a send that is then refused.
///
/// C5 (2026-11-04): any file attaches. With `file_name` (a file that isn't
/// an image) the saved name keeps a cleaned stem and extension of it
/// ([`attachment_name_parts`]) before the same generated unique part, so
/// the agent sees a readable name. The folder, the 20 MB cap, the empty
/// refusal and the unsafe-folder refusal are the same as for an image.
pub fn save_attachment(
    data_b64: &str,
    mime: &str,
    file_name: Option<&str>,
    base_dir: &std::path::Path,
    vault: &std::path::Path,
) -> Result<String, String> {
    if let Some(reason) = crate::agent::unsafe_working_dir(base_dir, Some(vault)) {
        return Err(crate::agent::unsafe_cwd_error(&reason));
    }
    let what = match file_name {
        Some(_) => "file",
        None => "image",
    };
    use base64::{engine::general_purpose::STANDARD, Engine};
    // Refused on its base64 length first, so an oversized payload is never
    // decoded (C5 review).
    if data_b64.trim().len() > ATTACHMENT_MAX_BASE64 {
        return Err(format!("{what} too large (max 20 MB)"));
    }
    let bytes = STANDARD
        .decode(data_b64.trim())
        .map_err(|e| format!("decode {what}: {e}"))?;
    // Clipboard screenshots are big but bounded; reject pathological payloads
    // so a huge base64 string can't be forced over the IPC boundary.
    if bytes.len() > ATTACHMENT_MAX_BYTES {
        return Err(format!("{what} too large (max 20 MB)"));
    }
    if bytes.is_empty() {
        return Err(format!("empty {what}"));
    }
    let converted = crate::webkit_capture::heic_png(&bytes)?;
    let bytes = converted.as_deref().unwrap_or(&bytes);
    let ext = if converted.is_some() {
        "png"
    } else {
        match mime {
            "image/png" => "png",
            "image/jpeg" | "image/jpg" => "jpg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            "image/svg+xml" => "svg",
            "image/bmp" => "bmp",
            "image/tiff" => "tiff",
            _ => "png",
        }
    };
    if !base_dir.is_dir() {
        return Err(format!(
            "chat working directory is unavailable: {}",
            base_dir.display()
        ));
    }
    let dir = attachments_dir(base_dir)?;
    let unique = unique_attachment_suffix();
    let name = match file_name {
        Some(original) if converted.is_some() => {
            let (stem, _) = attachment_name_parts(original);
            format!("{stem}-{unique}-heic-converted.png")
        }
        Some(original) => match attachment_name_parts(original) {
            (stem, Some(ext)) => format!("{stem}-{unique}.{ext}"),
            (stem, None) => format!("{stem}-{unique}"),
        },
        None if converted.is_some() => format!("paste-{unique}-heic-converted.png"),
        None => format!("paste-{unique}.{ext}"),
    };
    let path = dir.join(name);
    std::fs::write(&path, bytes).map_err(|e| format!("write {what}: {e}"))?;
    Ok(path.to_string_lossy().to_string())
}

/// A chat folder's hidden attachments folder, made when it is missing (the
/// folder itself must exist: only the leaf is created). The Mac composer's
/// [`save_attachment`] and the phone API's uploads
/// (`phone_access::attachments`) both save here, so a phone's files sit with
/// the composer's and travel with a turn the same way.
///
/// Intentionally create only the leaf. `create_dir_all` would recreate a
/// deleted worktree path and make the send-side `is_dir` guard believe the
/// stale conversation was healthy again. One that is there must be a real
/// folder: a link (a checkout can carry one) would send the file
/// elsewhere (C5 review).
pub(crate) fn attachments_dir(base_dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let dir = base_dir.join(ATTACHMENTS_DIR);
    match std::fs::create_dir(&dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if !std::fs::symlink_metadata(&dir).is_ok_and(|meta| meta.is_dir()) {
                return Err(format!(
                    "{ATTACHMENTS_DIR} in the chat folder isn't a plain folder"
                ));
            }
        }
        Err(error) => return Err(format!("create attachment dir: {error}")),
    }
    keep_attachments_out_of_git(&dir)?;
    Ok(dir)
}

/// The generated part of a saved attachment's name: process id, monotonic
/// nanos and a counter (no RNG, which is unavailable in some build contexts
/// and not needed here). A saved file is `<stem>-<this>.<ext>`;
/// `phone_access::attachments::readable_name` strips it again.
pub(crate) fn unique_attachment_suffix() -> String {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{}-{}", std::process::id(), nanos, n)
}

/// The largest attachment [`save_attachment`] keeps: 20 MB.
pub(crate) const ATTACHMENT_MAX_BYTES: usize = 20 * 1024 * 1024;
/// The base64 length of [`ATTACHMENT_MAX_BYTES`]: anything longer is
/// refused before it is decoded.
pub(crate) const ATTACHMENT_MAX_BASE64: usize = ATTACHMENT_MAX_BYTES.div_ceil(3) * 4;

/// The hidden folder in a chat's working folder that holds its attachments.
pub(crate) const ATTACHMENTS_DIR: &str = ".hollow-attachments";

/// What the attachments folder's `.gitignore` holds: `/*`, every entry of
/// the folder, the `.gitignore` included. Not `*`: a `.gitignore` of just
/// `*` is how a tool marks its cache (`remote_folder::self_ignoring`), and a
/// cache stays on this Mac when a chat runs on the runner.
pub(crate) const ATTACHMENTS_GITIGNORE: &str = "/*\n";

/// C5 review: a build workspace's chat runs in its git worktree, so the
/// attachments folder sits inside a repository whose Commit runs
/// `git add -A`. A `.gitignore` of `/*` inside it keeps every attached file
/// (and the `.gitignore` itself) out of git; the runner's snapshot adds the
/// folder by name, so attachments still travel with a turn
/// (`remote_session::snapshot`). Written when missing or different (an
/// older `*` included); a `.gitignore` that is a link or not a plain file is
/// refused, never followed.
pub(crate) fn keep_attachments_out_of_git(dir: &std::path::Path) -> Result<(), String> {
    let ignore = dir.join(".gitignore");
    if let Ok(meta) = std::fs::symlink_metadata(&ignore) {
        if !meta.file_type().is_file() {
            return Err("the attachment folder's .gitignore isn't a plain file".into());
        }
        if std::fs::read_to_string(&ignore).is_ok_and(|text| text == ATTACHMENTS_GITIGNORE) {
            return Ok(());
        }
    }
    std::fs::write(&ignore, ATTACHMENTS_GITIGNORE)
        .map_err(|e| format!("write attachment .gitignore: {e}"))
}

/// The stem and extension a saved attached file keeps from its original
/// name (C5, 2026-11-04). Only the last path component is read, so no part
/// of the name can point into a folder. The stem keeps ASCII letters,
/// digits, dot, dash and underscore (any other run of characters becomes
/// one dash), has no leading or trailing dot or dash, is at most 48
/// characters, and is `file` when nothing is left. The extension is
/// lowercased ASCII letters and digits, at most 10 of them; a longer or
/// odder one stays in the stem instead.
pub(crate) fn attachment_name_parts(original: &str) -> (String, Option<String>) {
    let base = original.rsplit(['/', '\\']).next().unwrap_or("");
    let (stem, ext) = match base.rsplit_once('.') {
        Some((stem, ext))
            if !stem.trim_matches('.').is_empty()
                && !ext.is_empty()
                && ext.len() <= 10
                && ext.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            (stem, Some(ext.to_ascii_lowercase()))
        }
        _ => (base, None),
    };
    let mut clean = String::new();
    for c in stem.chars() {
        let c = if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
            c
        } else {
            '-'
        };
        if c == '-' && clean.ends_with('-') {
            continue;
        }
        clean.push(c);
    }
    let edge = |c: char| c == '.' || c == '-';
    let clean: String = clean.trim_matches(edge).chars().take(48).collect();
    let clean = clean.trim_end_matches(edge);
    let stem = if clean.is_empty() { "file" } else { clean };
    (stem.to_string(), ext)
}

/// Read caps for the chat preview pane: text files are cut at 1.5 MB (the
/// pane shows a "truncated" note), images ride the same 10 MB cap as the
/// inline tool-result images.
const PREVIEW_TEXT_CAP: usize = 1_500_000;

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];

fn read_preview_image(reader: impl std::io::Read) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    reader
        .take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err("image too large to preview (limit 10 MB)".into());
    }
    Ok(bytes)
}

/// The most places a file link may name for `locate_file`, and the longest
/// path it looks at.
const LOCATE_MAX_PATHS: usize = 64;
const LOCATE_MAX_PATH_BYTES: usize = 4096;

/// The first of `paths` that is a file, or None (2026-11-04). A file a reply
/// names is tried against each place it may be (the chat's folder, the
/// project's repository, the vault, a Mac mini runner path mapped back to this
/// Mac): one base used to send every other kind of path to "it may have been
/// moved". Only absolute paths count, at most 64 of them; nothing is read,
/// only looked up, as `preview_file` already reads any absolute path.
pub fn locate_file(paths: &[String]) -> Option<String> {
    paths
        .iter()
        .take(LOCATE_MAX_PATHS)
        .filter(|p| p.len() <= LOCATE_MAX_PATH_BYTES && Path::new(p.as_str()).is_absolute())
        .find(|p| std::fs::metadata(p.as_str()).is_ok_and(|m| m.is_file()))
        .cloned()
}

/// The most files `find_files_by_tail` returns, the most roots it joins the
/// tail to, and the most vault entries its walk looks at.
const FIND_MAX_MATCHES: usize = 20;
const FIND_MAX_ROOTS: usize = 1024;
const FIND_MAX_WALK: usize = 120_000;

/// Files whose path ends with `tail`, when no place `locate_file` tried holds
/// it (2026-11-10: a path relative to another project's folder, copied from a
/// child chat's report, said "couldn't find"). First `tail` under each of
/// `roots` (every project's folder and linked repository), then, when none
/// holds it, a bounded walk of the vault that skips hidden folders,
/// `node_modules` and `target`. Each file once, at most 20; nothing is read.
/// `tail` must be relative and stay inside its root (no `..`).
pub fn find_files_by_tail(tail: &str, roots: &[String], vault: Option<&Path>) -> Vec<String> {
    let tail = tail.trim_start_matches("./").trim_end_matches('/');
    if tail.is_empty()
        || tail.len() > LOCATE_MAX_PATH_BYTES
        || tail.starts_with('/')
        || tail.split('/').any(|s| s == ".." || s.is_empty())
    {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |p: &Path, out: &mut Vec<String>| {
        let key = p.to_string_lossy().to_lowercase();
        if out.len() < FIND_MAX_MATCHES && seen.insert(key) {
            out.push(p.to_string_lossy().to_string());
        }
    };
    for root in roots.iter().take(FIND_MAX_ROOTS) {
        let root = Path::new(root.as_str());
        if !root.is_absolute() {
            continue;
        }
        let candidate = root.join(tail);
        if std::fs::metadata(&candidate).is_ok_and(|m| m.is_file()) {
            push(&candidate, &mut out);
        }
    }
    if !out.is_empty() {
        return out;
    }
    let Some(vault) = vault.filter(|v| v.is_absolute()) else {
        return out;
    };
    let name = tail.rsplit('/').next().unwrap_or(tail);
    let suffix = format!("/{tail}");
    let walk = walkdir::WalkDir::new(vault)
        .into_iter()
        .filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            e.depth() == 0 || !(n.starts_with('.') || n == "node_modules" || n == "target")
        })
        .take(FIND_MAX_WALK);
    for entry in walk.flatten() {
        if entry.file_type().is_file() && entry.file_name().to_string_lossy() == name {
            let rel = entry.path().to_string_lossy();
            if rel.ends_with(&suffix) {
                push(entry.path(), &mut out);
                if out.len() >= FIND_MAX_MATCHES {
                    break;
                }
            }
        }
    }
    out
}

/// Read a file the chat agent created/edited so the preview pane can render
/// it, restricted to canonical paths inside `roots` (from `media::roots`).
/// Absolute paths only; images come back as a `data:` URL, valid UTF-8 comes
/// back as text (capped), PDF/video return metadata for their scoped readers;
/// anything else is "binary" — the pane offers open-externally for those.
pub fn preview_file_scoped(
    path: &str,
    roots: &[PathBuf],
) -> Result<crate::types::ChatPreviewFile, String> {
    let p = std::path::Path::new(path);
    if !p.is_absolute() || path.len() > 8192 || path.contains('\0') {
        return Err("preview path must be absolute".into());
    }
    let canonical = p.canonicalize().map_err(|e| format!("read {path}: {e}"))?;
    let inside = roots.iter().any(|root| {
        let root = root.canonicalize().unwrap_or_else(|_| root.clone());
        canonical.starts_with(&root)
    });
    if !inside {
        return Err("This file is outside the conversation and project folders.".into());
    }
    let file = crate::media::open_regular(&canonical)
        .ok_or_else(|| format!("read {path}: could not open file"))?;
    // Re-check canonical path after open to defeat a directory-to-symlink swap.
    if p.canonicalize().ok().as_ref() != Some(&canonical) {
        return Err(format!("read {path}: path changed while opening"));
    }
    preview_opened_file(&canonical, path, file)
}

/// Read an already-admitted, opened regular file for the preview pane.
fn preview_opened_file(
    canonical: &Path,
    display_path: &str,
    file: std::fs::File,
) -> Result<crate::types::ChatPreviewFile, String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let meta = file
        .metadata()
        .map_err(|e| format!("read {display_path}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("not a file: {display_path}"));
    }
    let modified_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);
    let size = meta.len();
    let ext = canonical
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    // A video plays through the `hollow-media` scheme (media.rs): report its
    // size and time only, never read its bytes here.
    if crate::media::is_video_ext(&ext) {
        return Ok(crate::types::ChatPreviewFile {
            kind: "video".into(),
            text: None,
            data_url: None,
            size,
            modified_ms,
            truncated: false,
        });
    }

    if ext == "pdf" {
        return Ok(crate::types::ChatPreviewFile {
            kind: "pdf".into(),
            text: None,
            data_url: None,
            size,
            modified_ms,
            truncated: false,
        });
    }

    if IMAGE_EXTS.contains(&ext.as_str()) {
        if size > MAX_IMAGE_BYTES {
            return Err(format!(
                "image too large to preview ({} MB)",
                size / (1024 * 1024)
            ));
        }
        let bytes = read_preview_image(file)?;
        let mime = sniff_image_mime(&bytes, &ext);
        return Ok(crate::types::ChatPreviewFile {
            kind: "image".into(),
            text: None,
            data_url: Some(format!("data:{};base64,{}", mime, STANDARD.encode(&bytes))),
            size,
            modified_ms,
            truncated: false,
        });
    }

    // Read up to the cap + 1 byte so we know whether we cut anything (a
    // 3 GB mkv costs 1.5 MB, not its size).
    let mut bytes = Vec::new();
    {
        use std::io::Read;
        file.take(PREVIEW_TEXT_CAP as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("read {display_path}: {e}"))?;
    }
    let truncated = bytes.len() > PREVIEW_TEXT_CAP;
    if truncated {
        bytes.truncate(PREVIEW_TEXT_CAP);
        // Don't split a UTF-8 sequence at the cut point: drop at most the
        // three bytes of a cut sequence. Popping until valid emptied a big
        // binary file into blank "text".
        for _ in 0..3 {
            if std::str::from_utf8(&bytes).is_ok() {
                break;
            }
            bytes.pop();
        }
    }
    match String::from_utf8(bytes) {
        Ok(text) => Ok(crate::types::ChatPreviewFile {
            kind: "text".into(),
            text: Some(text),
            data_url: None,
            size,
            modified_ms,
            truncated,
        }),
        Err(_) => Ok(crate::types::ChatPreviewFile {
            kind: "binary".into(),
            text: None,
            data_url: None,
            size,
            modified_ms,
            truncated: false,
        }),
    }
}

/// Unscoped helper retained for existing unit tests; production callers must
/// go through [`preview_file_scoped`].
#[cfg(test)]
pub fn preview_file(path: &str) -> Result<crate::types::ChatPreviewFile, String> {
    let p = std::path::Path::new(path);
    if !p.is_absolute() {
        return Err("preview path must be absolute".into());
    }
    let canonical = p.canonicalize().map_err(|e| format!("read {path}: {e}"))?;
    let file = std::fs::File::open(&canonical).map_err(|e| format!("read {path}: {e}"))?;
    preview_opened_file(&canonical, path, file)
}

/// The scope a chat turn in `cwd` runs under (build programme PR B3):
/// `workspaces/<id>` when `cwd` is exactly a live build workspace's worktree
/// or (since 2026-09-28) exactly a workspace's own vault folder
/// `vault/workspaces/<id>`, so conversations in different workspaces run at
/// once; else `None` (a full-vault turn: the vault root or any other folder).
pub(crate) fn run_scope(vault: &Path, cwd: &Path) -> Option<String> {
    run_scope_mode(vault, cwd).map(|(scope, _)| scope)
}

/// [`run_scope`], and how the turn is admitted there. A build workspace's
/// worktree (code, a git index and toolchains two turns must not share) is
/// held alone (`build_workspaces::turn_mode`): with no extra worktree, the
/// whole folder, as before F6 part B, since its folder chats are told to
/// work in that worktree; once the workspace has lanes, only its lock key
/// (`build_workspaces::lock_key`), its turn a shared run of the folder, so
/// the worktrees and the folder's own conversations (which then get no
/// worktree as theirs) run at once. A workspace's own vault folder,
/// Quick chats included, is shared: several of its conversations run at
/// once (2026-10-07), and History marks what overlapping turns both changed.
pub(crate) fn run_scope_mode(
    vault: &Path,
    cwd: &Path,
) -> Option<(String, crate::automated_history::ScopeMode)> {
    use crate::automated_history::ScopeMode;
    if let Some((id, _, build)) = crate::build_workspaces::worktree_build(vault, cwd) {
        let mode = crate::build_workspaces::turn_mode(vault, &id, &build.worktree);
        return Some((format!("workspaces/{id}"), mode));
    }
    workspace_folder_scope(vault, cwd).map(|scope| {
        let mode = scope
            .strip_prefix("workspaces/")
            .and_then(|id| crate::build_workspaces::folder_mode(vault, id))
            .unwrap_or(ScopeMode::Shared);
        (scope, mode)
    })
}

/// `workspaces/<id>` when `cwd` resolves to a direct child folder of the
/// vault's `workspaces/` (symlinks resolved on both sides, so a cwd spelled
/// through `~/Hollow/Vault` matches). The folder itself must be a real
/// directory; `try_begin_scoped` checks that again at admission.
fn workspace_folder_scope(vault: &Path, cwd: &Path) -> Option<String> {
    let workspaces = vault.canonicalize().ok()?.join("workspaces");
    let cwd = cwd.canonicalize().ok()?;
    let id = cwd.strip_prefix(&workspaces).ok()?;
    let mut parts = id.components();
    let first = match parts.next()? {
        std::path::Component::Normal(name) => name.to_str()?,
        _ => return None,
    };
    if parts.next().is_some() || first.starts_with('.') || !cwd.is_dir() {
        return None;
    }
    Some(format!("workspaces/{first}"))
}

/// A chat turn's toolchain environment (`build_slots::chat_env`): the build
/// settings of the build workspace whose worktree is `cwd` (PR B7), in
/// every mode; for any other folder the caps and `hollow-slot` first on
/// PATH (OF07) when the turn can take a slot (`build_slots::can_take_slot`:
/// not plan mode, which runs no Bash, nor Antigravity's sandboxed modes).
/// No PATH entry when `hollow-slot` couldn't be written.
///
/// Every turn but a build worktree's own also leases a port of its own
/// (F6 part A, 2026-10-20; `build_slots::lease_turn_port`, keyed by
/// `owner`, the session key), so conversations running at once in one
/// workspace folder, or in a build workspace's vault folder beside its
/// worktree's turn, never share a `PORT` / `PW_PORT`. The caller holds
/// [`ToolchainEnv::port`] until the turn ends.
fn build_workspace_env(
    vault: &Path,
    cwd: &Path,
    agent: AgentKind,
    perm: &str,
    owner: &str,
) -> ToolchainEnv {
    build_workspace_env_with(vault, cwd, agent, perm, || {
        crate::build_slots::lease_turn_port(owner)
    })
}

/// [`build_workspace_env`] with the port lease passed in (tests inject an
/// exhausted range).
fn build_workspace_env_with(
    vault: &Path,
    cwd: &Path,
    agent: AgentKind,
    perm: &str,
    lease: impl FnOnce() -> Option<crate::build_slots::TurnPort>,
) -> ToolchainEnv {
    // A worktree (the primary or a lane, F6 part B) runs with its own
    // build folder and port; a build workspace's vault folder with its
    // primary's build folder.
    // Once a workspace has lanes its folder's conversations run beside its
    // worktrees' turns, so none of them is given a worktree's build folder.
    let worktree = crate::build_workspaces::worktree_build(vault, cwd).map(|(_, _, b)| b);
    let in_worktree = worktree.is_some();
    let build = worktree
        .or_else(|| {
            crate::workspaces::workspace_for_cwd(vault, cwd)
                .ok()
                .flatten()
                .filter(|w| crate::build_workspaces::live_lane_count(vault, &w.id) == 0)
                .and_then(|workspace| workspace.build)
        })
        .filter(|b| b.exists);
    if build.is_none() && !crate::build_slots::can_take_slot(agent, perm) {
        return ToolchainEnv::default();
    }
    let tool = crate::build_slots::ensure_tool()
        .map_err(|error| eprintln!("[build-slots] hollow-slot unavailable: {error}"))
        .ok();
    let path = build_workspace_base_path();
    // The worktree itself keeps its stable port; a build workspace's vault
    // folder (the binding matches it too) is a shared folder like any other.
    let in_worktree = in_worktree && build.is_some();
    // When every turn port is taken the turn gets none, never the
    // worktree's (review 2026-10-20; `build_slots::TurnPortEnv`).
    use crate::build_slots::TurnPortEnv;
    let port = if in_worktree { None } else { lease() };
    let turn_port = match &port {
        _ if in_worktree => TurnPortEnv::Worktree,
        Some(lease) => TurnPortEnv::Leased(lease.port()),
        None => {
            eprintln!(
                "[build-slots] every turn port is taken; this turn gets no Hollow-assigned PORT"
            );
            TurnPortEnv::Unavailable
        }
    };
    let mut env = crate::build_slots::chat_env(
        build.as_ref(),
        agent,
        perm,
        tool.as_deref(),
        Some(path.as_str()),
        turn_port,
    );
    // Settings, Devices, Builds and tests (smart routing 3), auto
    // included, so an inherited value never stands in for it.
    if !env.is_empty() {
        let builds_on = crate::agent_settings::get_settings()
            .ok()
            .and_then(|settings| settings.builds_on);
        env.push(crate::build_slots::route_env(builds_on.as_deref()));
    }
    ToolchainEnv { env, port }
}

/// [`build_workspace_env`]'s answer: the turn's environment and the port it
/// leased, freed when this is dropped.
#[derive(Debug, Default)]
struct ToolchainEnv {
    env: Vec<(String, String)>,
    port: Option<crate::build_slots::TurnPort>,
}

/// The PATH a turn's `hollow-slot` folder goes in front of: the same one
/// every agent gets ([`crate::agent::cli_path`]), never the bare PATH a
/// Dock-launched Hollow inherits, which has no Homebrew or cargo (B02).
fn build_workspace_base_path() -> String {
    crate::agent::cli_path()
}

/// The vault folder of the build workspace whose worktree is `cwd`, if any.
fn build_workspace_folder(vault: &Path, cwd: &Path) -> Option<PathBuf> {
    let workspace = crate::workspaces::workspace_for_cwd(vault, cwd).ok()??;
    workspace.build.as_ref().filter(|b| b.exists)?;
    Some(vault.join(&workspace.path))
}

/// Agent effectiveness F11 (wave 1 · lane R): a chat turn's context
/// manifest, assembled from the same preparation steps `execute_turn` runs
/// (retrieval, connector selection, the appended prompt and the wrapper),
/// with Jev answered by stubs so nothing leaves the machine.
#[cfg(test)]
mod f11_manifest_tests {
    use super::*;
    use crate::context_rank::{select_connectors_via, Basis, ConnectorCandidate};
    use crate::run_manifest::{sha256_hex, RunManifest};

    const REQUEST: &str = "Summarise the zircon kestrel memo and ask Nameword25x";

    /// The vault is `Vault/` inside the temp folder: the vault walker skips a
    /// hidden root, and tempfile's folders are `.tmp…`.
    fn vault() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("Vault").join("notes");
        std::fs::create_dir_all(&notes).unwrap();
        std::fs::write(
            notes.join("memo.md"),
            "The zircon kestrel memo is due Friday.\n",
        )
        .unwrap();
        std::fs::write(
            notes.join("brief.md"),
            "Kestrel planning brief for the zircon launch.\n",
        )
        .unwrap();
        dir
    }

    /// One turn's manifest, as `execute_turn` builds it.
    async fn turn_manifest(vault: &Path) -> RunManifest {
        let composed = format!(
            "<operator-context>\nI like strong tea.\n</operator-context>\n\n<hollow-chat-intent>\nAnswer plainly.\n</hollow-chat-intent>\n\n{REQUEST}"
        );
        let asked = crate::sessions::strip_injected_context(&composed);
        let mut manifest = RunManifest::new();
        manifest.run_id = "run-f11".into();
        let prepared = crate::directory_connections::prioritized_context_via(
            vault.to_path_buf(),
            vault.to_path_buf(),
            asked.clone(),
            |items| async move { Some(Ok(vec![2.0; items.len()])) },
        )
        .await
        .unwrap();
        manifest.set_retrieval(prepared.retrieval, prepared.jev);
        let owned: Vec<(String, String)> = (1..=30)
            .map(|i| (format!("bed-{i:02}"), format!("Nameword{i}x")))
            .collect();
        let list: Vec<ConnectorCandidate> = owned
            .iter()
            .map(|(id, name)| ConnectorCandidate {
                id,
                name,
                description: "",
            })
            .collect();
        let query = crate::context_rank::ConnectorQuery::plain(&asked);
        let selection = select_connectors_via(&query, &list, |_, items| async move {
            Some(Ok(vec![Some(0.0); items.len()]))
        })
        .await;
        manifest.set_connectors(selection);
        let append = chat_append_layers(
            false,
            vault,
            vault,
            AttachedCaps::default(),
            crate::visual_prompt::CanvasFrequency::Helps,
            "acceptEdits",
        );
        let note =
            "<hollow-connectors>\nNameword25x (tools: mcp__bed-25__*)\n</hollow-connectors>\n";
        let evidence = prepared.text;
        let wrap = |body: &str| format!("{note}{evidence}{body}");
        record_turn_layers(
            &mut manifest,
            &TurnLayers {
                append: &append,
                configured_held: false,
                workspace: None,
                recall: "",
                clock: "",
                context_note: "",
                mode: "",
                connectors_note: note,
                skills: "",
                evidence: &evidence,
                wrapped: &wrap(""),
            },
            &wrap(&composed),
        );
        assert!(
            evidence.contains("zircon kestrel"),
            "the prompt's evidence does carry the passage: {evidence}"
        );
        manifest
    }

    #[tokio::test]
    async fn a_turn_manifest_holds_no_request_or_passage_text() {
        let dir = vault();
        let manifest = turn_manifest(&dir.path().join("Vault")).await;
        let json = serde_json::to_string(&manifest).unwrap();
        for text in [
            "zircon",
            "kestrel",
            "Summarise",
            "due Friday",
            "strong tea",
            "Answer plainly",
        ] {
            assert!(!json.contains(text), "the manifest holds {text:?}: {json}");
        }
        let names: Vec<&str> = manifest.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "visualPrompt",
                "capabilityIndex",
                "environmentBrief",
                "policy",
                "connectorsNote",
                "retrievalEvidence",
                "operatorProfile",
                "chatIntent",
                "request"
            ]
        );
        let request = manifest
            .layers
            .iter()
            .find(|l| l.name == "request")
            .unwrap();
        assert_eq!(request.sha256, sha256_hex(REQUEST.as_bytes()));
        assert_eq!(request.bytes, REQUEST.len() as u64);
    }

    #[tokio::test]
    async fn retrieval_and_connector_decisions_appear_with_reasons() {
        let dir = vault();
        let manifest = turn_manifest(&dir.path().join("Vault")).await;
        let retrieval = manifest.retrieval.as_ref().unwrap();
        assert_eq!(retrieval.engine, "jev");
        let paths: Vec<&str> = retrieval
            .candidates
            .iter()
            .map(|c| c.path.as_str())
            .collect();
        assert!(
            paths.contains(&"notes/memo.md"),
            "vault-relative: {paths:?}"
        );
        assert!(paths.contains(&"notes/brief.md"), "{paths:?}");
        for c in &retrieval.candidates {
            assert!(
                c.kept && c.basis == "jev" && c.jev_score == Some(2.0),
                "{c:?}"
            );
            assert!(c.local_score >= 1 && c.rank >= 1, "{c:?}");
            assert_eq!(c.sha256.len(), 64);
        }
        // R2's decisions, one per candidate, the named 25th kept as named.
        let connectors = manifest.connectors.as_ref().unwrap();
        assert_eq!(connectors.decisions.len(), 30);
        let named = &connectors.decisions[24];
        assert_eq!(
            (named.id.as_str(), named.attach, named.basis),
            ("bed-25", true, Basis::Named)
        );
        assert_eq!(connectors.attached().collect::<Vec<_>>(), ["bed-25"]);
        let beyond: Vec<_> = connectors
            .decisions
            .iter()
            .filter(|d| d.basis == Basis::BeyondLimit)
            .collect();
        assert_eq!(beyond.len(), 5, "29 open, 24 sent");
        assert!(beyond
            .iter()
            .all(|d| d.reason.as_deref().unwrap().contains("at most 24")));
        // Both Jev uses, with outcomes, and a cost nobody knows.
        let uses: Vec<(&str, &str)> = manifest
            .jev
            .iter()
            .map(|j| (j.kind.as_str(), j.outcome.as_str()))
            .collect();
        assert_eq!(uses, [("passages", "ok"), ("connectors", "ok")]);
        assert!(manifest.jev.iter().all(|j| j.cost_usd.is_none()));
        let wire = serde_json::to_value(&manifest).unwrap();
        assert!(wire["jev"][0]["costUsd"].is_null());
        assert_eq!(
            manifest.fallbacks.len(),
            1,
            "the 5 beyond Jev's 24: {:?}",
            manifest.fallbacks
        );
    }

    #[tokio::test]
    async fn identical_inputs_give_identical_sizes_and_hashes() {
        let dir = vault();
        let v = dir.path().join("Vault");
        let one = turn_manifest(&v).await;
        let two = turn_manifest(&v).await;
        assert_eq!(one.layers, two.layers);
        // The broker's own timings are the run's, not its inputs'.
        let timeless = |r: &Option<crate::run_manifest::RetrievalRecord>| {
            r.clone().map(|mut r| {
                if let Some(stats) = r.stats.as_mut() {
                    stats.ms = 0;
                    stats.semantic_ms = None;
                }
                r
            })
        };
        assert_eq!(timeless(&one.retrieval), timeless(&two.retrieval));
        assert_eq!(one.connectors, two.connectors);
    }

    /// The wrapper `execute_turn` builds is what [`record_turn_layers`]
    /// strips: with recall and a workspace too, the body's layers are
    /// found.
    #[test]
    fn the_body_is_found_after_the_full_wrapper() {
        let append = AppendLayers {
            visual: "v".into(),
            index: "i".into(),
            brief: "b".into(),
            policy: "\n\np".into(),
        };
        let wrapped = "<hollow-workspace>w</hollow-workspace>recall\n\nnote evidence ";
        let mut m = RunManifest::new();
        record_turn_layers(
            &mut m,
            &TurnLayers {
                append: &append,
                configured_held: false,
                workspace: Some("<hollow-workspace>w</hollow-workspace>"),
                recall: "recall",
                clock: "",
                context_note: "",
                mode: "",
                connectors_note: "note ",
                skills: "",
                evidence: "evidence ",
                wrapped,
            },
            &format!(
                "{wrapped}{}hello there",
                crate::visual_prompt::guide_block("# The visual guide")
            ),
        );
        let names: Vec<&str> = m.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "visualPrompt",
                "capabilityIndex",
                "environmentBrief",
                "policy",
                "workspace",
                "recall",
                "connectorsNote",
                "retrievalEvidence",
                "visualGuide",
                "request"
            ]
        );
        assert_eq!(append.joined(), "v\n\ni\n\nb\n\np");
    }
}

#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;

/// Agent effectiveness F2 (wave 1 · lane C): a chat turn's recall,
/// connector selection and learning read the admitted request, not the
/// composed prompt.
#[cfg(test)]
mod f2_request_tests {
    use super::*;
    use crate::turn_request::fixtures::{self, ATLAS, BOREALIS, OUTLOOK};

    fn send(vault: &Path, key: &str, composed: &str) -> ChatTurnPayload {
        send_as(vault, key, composed, false)
    }

    fn send_as(vault: &Path, key: &str, composed: &str, typed: bool) -> ChatTurnPayload {
        let accepted = accept_turn(
            vault,
            ChatSendRequest {
                prompt: composed.to_string(),
                session_id: None,
                session_key: Some(key.to_string()),
                cwd: vault.to_path_buf(),
                model: None,
                permission_mode: None,
                agent: AgentKind::Claude,
                client_turn_id: None,
                app_depth: 0,
                ultracode: false,
                canvases: None,
                continued_from: Vec::new(),
                unattended: false,
                typed,
                host_asker: None,
                canned: None,
                worker: false,
            },
            Utc::now(),
        )
        .unwrap();
        let row = run_queue::get(vault, accepted.queue_id).unwrap().unwrap();
        serde_json::from_value(row.payload).unwrap()
    }

    fn composed(profile: &str, user: &str) -> String {
        format!(
            "{}{}{user}",
            fixtures::operator_block(profile),
            fixtures::INTENT_BLOCK
        )
    }

    #[test]
    fn unrelated_profile_prose_changes_neither_recall_nor_connectors() {
        let dir = fixtures::learned_vault();
        let v = dir.path();
        let ask = "Draft the Atlas weekly report";
        let plain = turn_inputs(
            v,
            &send(
                v,
                "s-20261006-plain",
                &composed(fixtures::PLAIN_PROFILE, ask),
            ),
            "s-20261006-plain",
            false,
        );
        let noisy = turn_inputs(
            v,
            &send(
                v,
                "s-20261006-noisy",
                &composed(fixtures::NOISY_PROFILE, ask),
            ),
            "s-20261006-noisy",
            false,
        );
        assert_eq!(
            fixtures::statements(&plain.recall),
            vec![OUTLOOK.to_string(), ATLAS.to_string()]
        );
        assert_eq!(
            fixtures::statements(&noisy.recall),
            fixtures::statements(&plain.recall),
            "profile prose chose memories"
        );
        assert!(!fixtures::statements(&noisy.recall).contains(&BOREALIS.to_string()));
        assert_eq!(
            fixtures::local_selection(&noisy.connector_query.local),
            fixtures::local_selection(&plain.connector_query.local),
            "profile prose chose connectors"
        );
        assert!(fixtures::local_selection(&noisy.connector_query.local).is_empty());
    }

    #[test]
    fn a_long_profile_cannot_hide_an_explicit_connector_request() {
        let dir = fixtures::learned_vault();
        let v = dir.path();
        let profile = "I like tidy notes and calm mornings. ".repeat(200);
        let inputs = turn_inputs(
            v,
            &send(
                v,
                "s-20261006-long",
                &composed(&profile, "Summarize yesterday's Plaud call"),
            ),
            "s-20261006-long",
            false,
        );
        assert_eq!(
            fixtures::local_selection(&inputs.connector_query.local),
            vec!["plaud"]
        );
        // Jev scores the first 2,000 characters of the query (`jev.rs`).
        let head: String = inputs.connector_query.request.chars().take(2000).collect();
        assert!(head.contains("Plaud"), "{head}");
    }

    /// A resumed conversation: the earlier turns are in the session store,
    /// the composer sends only the profile and the new words.
    #[test]
    fn a_resumed_reference_resolves_the_earlier_project() {
        let dir = fixtures::learned_vault();
        let v = dir.path();
        let key = "s-20261006-resume";
        let cwd = v.to_string_lossy().to_string();
        crate::session_store::start_turn(
            v,
            key,
            "claude",
            &cwd,
            "run-1",
            "Summarize the Borealis project",
        )
        .unwrap();
        crate::session_store::start_turn(
            v,
            key,
            "claude",
            &cwd,
            "run-2",
            "Now summarize the Atlas project status",
        )
        .unwrap();
        crate::session_store::start_turn(v, key, "claude", &cwd, "run-3", "Thanks, that helps")
            .unwrap();
        let payload = send(
            v,
            key,
            &composed(fixtures::NOISY_PROFILE, "Do the same for that project"),
        );
        let request = payload.request.clone().expect("admitted fields");
        let paths: Vec<&str> = request
            .antecedents
            .iter()
            .map(|a| a.path.as_str())
            .collect();
        assert_eq!(paths, vec!["projects/active/Atlas.md"]);
        assert_eq!(request.antecedents[0].from_turn, 2);
        let inputs = turn_inputs(v, &payload, key, false);
        let recalled = fixtures::statements(&inputs.recall);
        assert!(recalled.contains(&ATLAS.to_string()), "{recalled:?}");
        assert!(!recalled.contains(&BOREALIS.to_string()), "{recalled:?}");
        // The resolved name informs local selection only; Jev gets the
        // request as typed.
        assert!(inputs.connector_query.local.contains("Atlas"));
        assert_eq!(
            inputs.connector_query.request,
            "Do the same for that project"
        );
        assert_eq!(
            inputs.learn_from.references,
            vec!["projects/active/Atlas.md"]
        );
    }

    /// Jev is sent only what the person typed this turn: a resolved
    /// antecedent's name may inform local selection, but it is never new
    /// content for Jev (wave 1 follow-up to C1).
    #[test]
    fn jev_is_sent_only_the_request_when_an_antecedent_resolves() {
        let dir = fixtures::learned_vault();
        let v = dir.path();
        let key = "s-20261006-jev-only";
        let cwd = v.to_string_lossy().to_string();
        crate::session_store::start_turn(v, key, "claude", &cwd, "run-1", "Plan the Atlas project")
            .unwrap();
        let ask = "Do the same for that project";
        let prompt = composed(fixtures::NOISY_PROFILE, ask);
        let payload = send(v, key, &prompt);
        assert_eq!(payload.request.clone().unwrap().antecedents.len(), 1);
        let inputs = turn_inputs(v, &payload, key, false);
        assert!(inputs.connector_query.local.contains("Atlas"));
        // A "when relevant" connector whose name only the antecedent says:
        // local selection still sees it, Jev is sent only the request.
        let candidates = [crate::context_rank::ConnectorCandidate {
            id: "atlas-tracker",
            name: "Atlas tracker",
            description: "Issues for the Atlas project",
        }];
        let sent = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
        let log = sent.clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let selection = runtime.block_on(crate::context_rank::select_connectors_via(
            &inputs.connector_query,
            &candidates,
            |text, items| async move {
                *log.lock().unwrap() = Some(text.to_string());
                Some(Ok(vec![Some(0.0); items.len()]))
            },
        ));
        assert_eq!(selection.decisions.len(), 1);
        assert_eq!(
            sent.lock().unwrap().as_deref(),
            Some(crate::sessions::strip_injected_context(&prompt).as_str()),
            "the text handed to the Jev scorer is the stripped request"
        );
    }

    /// The one earlier request "yes, merge it" follows (overnight OB25).
    const NOTION_ASK: &str = "Fix the Notion sync retry backoff in notion_sync.rs";

    /// A conversation whose earlier turns asked `asks`, oldest first.
    fn asked_before(vault: &Path, key: &str, asks: &[&str]) {
        let cwd = vault.to_string_lossy().to_string();
        for (i, ask) in asks.iter().enumerate() {
            let run = format!("run-{key}-{i}");
            crate::session_store::start_turn(vault, key, "claude", &cwd, &run, ask).unwrap();
        }
    }

    fn wake_with(title: &str) -> String {
        format!(
            "{}\n\n- “{title}” in Launch (claude, session s-ob18-child): done",
            crate::main_agent::WAKE_MESSAGE
        )
    }

    /// Overnight OB18: a wake's retrieval, recall, attached sources and
    /// connectors read the chats it reports, never its template; a check-in
    /// with nothing new retrieves nothing and keeps the last connector
    /// decision, so neither half calls Jev.
    #[tokio::test]
    async fn a_host_turn_reads_its_subject_and_an_empty_one_skips() {
        use crate::turn_request::QueryBasis;
        let dir = fixtures::learned_vault();
        let v = dir.path();
        let wake = wake_with("Fix the Notion sync");
        let inputs = turn_inputs(v, &send(v, "s-ob18-wake", &wake), "s-ob18-wake", true);
        let basis = &inputs.retrieval_basis;
        assert_eq!(basis.basis, QueryBasis::HostSubject);
        let query = basis.query.clone().expect("a subject");
        assert!(query.contains("Notion sync"), "{query}");
        for template in ["report", "PLAN.md", "finished"] {
            assert!(!query.contains(template), "{template}: {query}");
        }
        assert_eq!(inputs.source_query, query);
        assert_eq!(inputs.connector_query.request, query);
        assert_eq!(
            fixtures::local_selection(&inputs.connector_query.local),
            vec!["notion"]
        );
        // What the turn learns from is still the whole message.
        assert!(inputs
            .learn_from
            .user
            .contains("what it found and what is next."));

        let quiet = format!(
            "{} Nothing new came up since the last check-in.\n\nDecide what needs the person.",
            "⟲ Hollow: check-in at 10:00."
        );
        let payload = send(v, "s-ob18-quiet", &quiet);
        let inputs = turn_inputs(v, &payload, "s-ob18-quiet", true);
        assert_eq!(inputs.retrieval_basis.basis, QueryBasis::SkippedHost);
        assert!(inputs.retrieval_basis.basis.skips());
        // Nothing of the template reaches recall, sources or connectors.
        assert_eq!(inputs.retrieval_basis.query.as_deref(), Some(""));
        assert_eq!(inputs.source_query, "");
        assert_eq!(inputs.connector_query.request, "");
        assert_eq!(inputs.connector_query.local, "");
        // execute_turn retrieves on an empty query: no passages, so Jev's
        // rerank is never asked.
        let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = asked.clone();
        let prepared = crate::directory_connections::prioritized_context_via(
            v.to_path_buf(),
            v.to_path_buf(),
            String::new(),
            move |items| {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                async move { Some(Ok(vec![1.0; items.len()])) }
            },
        )
        .await
        .unwrap();
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(prepared.jev.is_none());
    }

    /// Overnight OB18: the person's own turn and a night-queue item read
    /// their request, as before.
    #[test]
    fn a_person_turn_and_a_night_item_read_their_request() {
        use crate::turn_request::QueryBasis;
        let dir = fixtures::learned_vault();
        let v = dir.path();
        let night = format!(
            "{} item 3 added by you at 01:00. Plan the Q3 offsite agenda.",
            crate::night_queue::NIGHT_PREFIX
        );
        for (key, ask) in [
            ("s-ob18-person", "Draft the Atlas weekly report"),
            ("s-ob18-night", night.as_str()),
        ] {
            let inputs = turn_inputs(v, &send(v, key, ask), key, false);
            assert_eq!(inputs.retrieval_basis.basis, QueryBasis::Request, "{key}");
            assert_eq!(inputs.retrieval_basis.query, None, "{key}");
            assert_eq!(inputs.source_query, ask, "{key}");
            assert_eq!(inputs.connector_query.request, ask, "{key}");
        }
    }

    /// Overnight OB25: a resumed "yes, merge it" adds nothing to the topic
    /// its native session holds, so it skips retrieval (no broker, no Jev)
    /// and keeps the last connector decision; today it queried "yes merge".
    /// Recall and attached sources read the follow-up with its prior.
    #[test]
    fn a_resumed_follow_up_that_adds_nothing_skips_retrieval() {
        use crate::turn_request::QueryBasis;
        let dir = fixtures::learned_vault();
        let v = dir.path();
        let key = "s-ob25-merge";
        asked_before(v, key, &[NOTION_ASK]);
        let inputs = turn_inputs(v, &send(v, key, "yes, merge it"), key, true);
        let basis = &inputs.retrieval_basis;
        assert_eq!(basis.basis, QueryBasis::SkippedFollowup);
        assert!(basis.basis.skips());
        let joined = "yes merge fix notion sync retry backoff";
        assert_eq!(basis.query.as_deref(), Some(joined));
        assert_eq!(inputs.source_query, joined);
        // Jev is sent only what was typed this turn.
        assert_eq!(inputs.connector_query.request, "yes, merge it");
    }

    /// Red first (OF26 review, B1): a turn that retrieves nothing still
    /// embedded its query for recall's meaning, a vector nothing reused.
    /// Now its recall skips meaning; the same follow-up not resumed
    /// retrieves, and its recall asks for meaning. Semantic search is off
    /// in tests, so both answer `Off` and the manifest records nothing
    /// (second review: the skipped one said `skipped` even then; that it
    /// says so when semantic search is on is
    /// `turn_request::tests::a_skipped_recall_says_skipped_only_when_meaning_was_available`).
    #[test]
    fn a_turn_that_skips_retrieval_skips_recalls_meaning() {
        use crate::vault_learning::RecallMeaning;
        let dir = fixtures::learned_vault();
        let v = dir.path();
        let key = "s-of26-skip";
        asked_before(v, key, &[NOTION_ASK]);
        let skipped = turn_inputs(v, &send(v, key, "yes, merge it"), key, true);
        assert!(skipped.retrieval_basis.basis.skips());
        assert_eq!(skipped.recall.meaning, RecallMeaning::Off);
        assert_eq!(skipped.recall.meaning.recorded(), None);
        let key = "s-of26-asked";
        asked_before(v, key, &[NOTION_ASK]);
        let asked = turn_inputs(v, &send(v, key, "yes, merge it"), key, false);
        assert!(!asked.retrieval_basis.basis.skips());
        assert_eq!(asked.recall.meaning, RecallMeaning::Off);
        assert_eq!(skipped.recall.prompt_block, asked.recall.prompt_block);
    }

    /// Overnight R1 review (B1): a follow-up that asks to resume but whose
    /// plan starts a fresh native session (here: the stored configuration
    /// changed) is not skipped; it retrieves with its prior's terms. The
    /// exact resume still skips.
    #[test]
    fn a_follow_up_whose_resume_resets_retrieves_its_priors_terms() {
        use crate::turn_request::QueryBasis;
        let dir = fixtures::learned_vault();
        let v = dir.path();
        let key = "s-ob25-reset-plan";
        asked_before(v, key, &[NOTION_ASK]);
        crate::session_store::record_harness_id_with(v, key, "sess-old", Some("v1:old")).unwrap();
        let inputs = turn_inputs(v, &send(v, key, "yes, merge it"), key, true);
        assert_eq!(inputs.retrieval_basis.basis, QueryBasis::SkippedFollowup);
        let state = crate::session_store::resume_state(v, key).unwrap();
        let plan = crate::chat_resume::plan_resume_with(Some("sess-old"), &state, "v3:new", false);
        assert_eq!(
            plan,
            ResumePlan::Reset(crate::chat_resume::ResetReason::ConfigChanged)
        );
        let resumes = matches!(plan, ResumePlan::Resume(_));
        let basis = inputs.retrieval_basis.clone().on_plan(resumes);
        assert_eq!(basis.basis, QueryBasis::FollowupPrior);
        assert_eq!(
            basis.query.as_deref(),
            Some("yes merge fix notion sync retry backoff")
        );
        let same = crate::chat_resume::plan_resume_with(Some("sess-old"), &state, "v1:old", false);
        let resumes = matches!(same, ResumePlan::Resume(_));
        assert!(inputs.retrieval_basis.on_plan(resumes).basis.skips());
        // An empty native id asks for no resume at all.
        let blank = crate::chat_resume::plan_resume_with(Some(" "), &state, "v1:old", false);
        assert_eq!(blank, ResumePlan::New);
    }

    /// Overnight OB25: on a fresh native session (a reset) the follow-up
    /// retrieves with its prior's topic; a new word leads the query and
    /// keeps retrieval on, resumed or not.
    #[test]
    fn a_follow_up_queries_its_own_terms_then_its_priors() {
        use crate::turn_request::QueryBasis;
        let dir = fixtures::learned_vault();
        let v = dir.path();
        asked_before(v, "s-ob25-reset", &[NOTION_ASK]);
        let reset = turn_inputs(
            v,
            &send(v, "s-ob25-reset", "continue"),
            "s-ob25-reset",
            false,
        );
        assert_eq!(reset.retrieval_basis.basis, QueryBasis::FollowupPrior);
        assert_eq!(
            reset.retrieval_basis.query.as_deref(),
            Some("continue fix notion sync retry backoff")
        );
        asked_before(v, "s-ob25-dana", &[NOTION_ASK]);
        let dana = turn_inputs(
            v,
            &send(v, "s-ob25-dana", "what about Dana?"),
            "s-ob25-dana",
            true,
        );
        assert_eq!(dana.retrieval_basis.basis, QueryBasis::FollowupPrior);
        assert_eq!(
            dana.retrieval_basis.query.as_deref(),
            Some("dana fix notion sync retry backoff")
        );
    }

    /// Overnight OB25: a request with four terms or more reads exactly as
    /// before, whatever came earlier; a wake (or a turn Hollow asked) is
    /// never a follow-up's prior.
    #[test]
    fn a_full_request_is_unchanged_and_a_wake_is_never_the_prior() {
        use crate::turn_request::QueryBasis;
        let dir = fixtures::learned_vault();
        let v = dir.path();
        let ask = "Draft the Atlas weekly report";
        asked_before(v, "s-ob25-full", &[NOTION_ASK]);
        let full = turn_inputs(v, &send(v, "s-ob25-full", ask), "s-ob25-full", true);
        let fresh = turn_inputs(v, &send(v, "s-ob25-fresh", ask), "s-ob25-fresh", true);
        assert_eq!(full.retrieval_basis.basis, QueryBasis::Request);
        assert_eq!(full.retrieval_basis.query, None);
        assert_eq!(full.source_query, fresh.source_query);
        assert_eq!(full.connector_query.request, fresh.connector_query.request);
        assert_eq!(
            fixtures::statements(&full.recall),
            fixtures::statements(&fresh.recall)
        );

        // Only Hollow's turns before it: no prior, the request as typed.
        let wake = wake_with("Tidy the invoices ledger");
        let key = "s-ob25-woken";
        asked_before(v, key, &[wake.as_str()]);
        let woken = turn_inputs(v, &send(v, key, "continue"), key, true);
        assert_eq!(woken.retrieval_basis.basis, QueryBasis::Request);
        // A person's turn, then a wake: the person's is the prior.
        let key = "s-ob25-between";
        asked_before(v, key, &[NOTION_ASK, wake.as_str()]);
        let between = turn_inputs(v, &send(v, key, "continue"), key, false);
        let query = between.retrieval_basis.query.unwrap_or_default();
        assert_eq!(query, "continue fix notion sync retry backoff");
        // A turn recorded as Hollow's (`asked_by` `hollow:*`) is skipped too.
        let key = "s-ob25-hollow";
        asked_before(v, key, &[NOTION_ASK, "Tidy the invoices ledger today"]);
        crate::session_store::record_turn_asker(v, key, 2, "hollow:pulse").unwrap();
        let skipped = turn_inputs(v, &send(v, key, "continue"), key, false);
        let query = skipped.retrieval_basis.query.unwrap_or_default();
        assert_eq!(query, "continue fix notion sync retry backoff");
    }

    /// J3a review (red first): the prefetch ranked recall through
    /// `recall_ranked`, which recorded usage every time, so a prefetch
    /// thrown away by a deferred admission (retried every few seconds) or
    /// replaced past its cutoff counted the same notes again. Now the chat
    /// path ranks without recording and the turn records the recall it
    /// uses, once.
    #[test]
    fn a_turn_records_its_recall_once_and_a_dropped_prefetch_never() {
        use std::time::Duration;
        let dir = fixtures::learned_vault();
        let v = dir.path();
        std::fs::create_dir_all(v.join("learnings")).unwrap();
        std::fs::write(
            v.join("learnings/csv-j3a.md"),
            "---\ntype: learned-context\ntitle: CSV delimiter\ntarget_kind: goal\ntarget_path: goals/report.md\nsubject: CSV reports\ntrust: approved\nproposal_id: j3a-fixture\nlearned_at: 2026-10-01T00:00:00Z\n---\n\nCSV reports preserve their declared delimiter.\n",
        )
        .unwrap();
        let uses = || -> u64 {
            crate::memory_maintenance::view(v)
                .unwrap()
                .entries
                .iter()
                .filter(|e| e.path.ends_with("csv-j3a.md"))
                .map(|e| e.uses)
                .sum()
        };
        let key = "s-j3a-once";
        let payload = send(v, key, "How do CSV reports keep their declared delimiter?");
        let long = Duration::from_secs(20);

        // A prefetch a deferred admission throws away records nothing.
        let (dropped, _) = chat_queue::prefetch_inputs(v, &payload, key, false).take(long, long);
        let dropped = dropped.expect("the prefetch answers");
        assert!(
            dropped
                .recall
                .items
                .iter()
                .any(|i| i.path.ends_with("csv-j3a.md")),
            "the fixture is recalled"
        );
        assert_eq!(uses(), 0, "a dropped prefetch counts nothing");

        // The turn that uses it records once.
        let prefetch = chat_queue::prefetch_inputs(v, &payload, key, false);
        let _ = chat_queue::turn_inputs_prefetched(v, &payload, key, false, prefetch);
        assert_eq!(uses(), 1, "recorded once");

        // Past its cutoff the turn ranks again itself and still records
        // once; the late prefetch, when it ends, records nothing.
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let (done_tx, done) = std::sync::mpsc::channel::<()>();
        let (lv, lp) = (v.to_path_buf(), payload.clone());
        let late: chat_queue::InputsPrefetch = crate::recall_prefetch::Prefetch::start(move || {
            let _ = gate.recv();
            let inputs = chat_queue::turn_inputs_by(&lv, &lp, "s-j3a-once", false, true);
            let _ = done_tx.send(());
            inputs
        });
        let (inputs, pickup) = chat_queue::turn_inputs_prefetched(v, &payload, key, false, late);
        assert_eq!(pickup.outcome, crate::recall_prefetch::Outcome::Late);
        assert!(inputs
            .recall
            .items
            .iter()
            .any(|i| i.path.ends_with("csv-j3a.md")));
        assert_eq!(uses(), 2, "the late turn recorded once");
        release.send(()).unwrap();
        done.recv_timeout(long).unwrap();
        assert_eq!(uses(), 2, "the late prefetch recorded nothing");
    }

    /// Fix-up 10-17 (red first): a Main agent check-in never loads the visual
    /// guide. Its own guidance says "options", a cue word, so every check-in
    /// carried the ~11 KB guide; the same words from the person, or in a
    /// child wake, still cue it.
    #[test]
    fn a_check_in_never_cues_the_visual_guide() {
        use crate::main_agent::{ASKED_BY_CHECKIN, ASKED_BY_WAKE, PULSE_GUIDANCE};
        let text = format!("Check-in.\n\nSignals: 4 tasks due, 2 PRs open, 3 chats ended, 9 notes.\n\n{PULSE_GUIDANCE}");
        assert!(
            crate::visual_prompt::visual_cue(PULSE_GUIDANCE, &[]).is_some(),
            "the guidance alone reads as visual (why this rule exists)"
        );
        let cue = |asker| chat_queue::turn_visual_cue(&text, &[], asker);
        assert_eq!(cue(Some(ASKED_BY_CHECKIN)), None);
        assert!(cue(None).is_some(), "the person's words");
        assert!(cue(Some(ASKED_BY_WAKE)).is_some(), "only check-ins");
    }

    /// Fewer learnings (10-11): only the composer's send is the person's
    /// words; an agent's packet, a wake, a night item or a bare run never
    /// teaches, and a row queued without the flag doesn't either.
    #[test]
    fn only_the_persons_typed_turns_teach() {
        assert!(turn_learns(true, 0, false, false));
        assert!(
            !turn_learns(false, 0, false, false),
            "Hollow's or an agent's message"
        );
        assert!(!turn_learns(true, 1, false, false), "a delegated chat");
        assert!(!turn_learns(true, 0, true, false), "an unattended turn");
        assert!(!turn_learns(true, 0, false, true), "a bare persona run");

        let dir = fixtures::learned_vault();
        let v = dir.path();
        let packet = send(
            v,
            "s-20261011-packet",
            "From now on never install; don't push.",
        );
        assert!(!packet.typed, "an agent's send carries no typed flag");
        let row = serde_json::to_value(&packet).unwrap();
        assert!(
            row.get("typed").is_none(),
            "absent on the row, so old rows read false"
        );
        let old: ChatTurnPayload = serde_json::from_value(row).unwrap();
        assert!(!old.typed);
        let typed = send_as(v, "s-20261011-typed", "Always write dates as ISO.", true);
        assert!(typed.typed, "the composer's flag rides the queue row");

        // The composer's command is the only sender that sets it.
        let commands = include_str!("commands.rs");
        let start = commands
            .find(concat!("pub async fn ", "chat_send("))
            .unwrap();
        let wrapper = &commands[start..start + commands[start..].find("\n}\n").unwrap()];
        // Unless the person sent a request Hollow wrote unchanged (A1).
        assert!(wrapper.contains(concat!("typed: ", "canned.is_none(),")));
        assert_eq!(
            commands.matches(concat!("typed: ", "canned")).count()
                + commands.matches(concat!("typed: ", "true")).count(),
            1,
            "no other send in commands.rs claims the person's words"
        );
    }

    /// Overnight 9 A1: a canvas view's request the person sent unchanged
    /// rides the queue row with its tag and teaches nothing.
    #[test]
    fn a_canned_send_is_not_typed() {
        let dir = fixtures::learned_vault();
        let v = dir.path();
        let accepted = accept_turn(
            v,
            ChatSendRequest {
                prompt: "Please make a canvas view. Do not change any files.".into(),
                session_id: None,
                session_key: Some("s-20261019-canned".into()),
                cwd: v.to_path_buf(),
                model: None,
                permission_mode: None,
                agent: AgentKind::Claude,
                client_turn_id: None,
                app_depth: 0,
                ultracode: false,
                canvases: None,
                continued_from: Vec::new(),
                unattended: false,
                typed: false,
                host_asker: None,
                canned: crate::task_state::canned_asker(Some("canvas")),
                worker: false,
            },
            Utc::now(),
        )
        .unwrap();
        let row = run_queue::get(v, accepted.queue_id).unwrap().unwrap();
        assert_eq!(row.payload["canned"], serde_json::json!("hollow:canvas"));
        assert!(row.payload.get("typed").is_none());
        assert!(row.payload.get("hostAsker").is_none(), "never host-opened");
        let payload: ChatTurnPayload = serde_json::from_value(row.payload).unwrap();
        assert!(!turn_learns(payload.typed, 0, false, false));
        let plain = send(v, "s-20261019-plain", "Draft it.");
        assert!(
            serde_json::to_value(&plain)
                .unwrap()
                .get("canned")
                .is_none(),
            "absent on every other row"
        );
    }

    #[test]
    fn injected_old_preferences_alone_never_trigger_extraction() {
        let dir = fixtures::learned_vault();
        let v = dir.path();
        let history = "<conversation-context>\nuser: We decided to always use Outlook.\n</conversation-context>\n\n";
        let injected = format!(
            "{history}{}",
            composed(
                "Never use Gmail. Always use Outlook.",
                "Can you fix the button?"
            )
        );
        let quiet = send(v, "s-20261006-quiet", &injected);
        let inputs = turn_inputs(v, &quiet, "s-20261006-quiet", false);
        assert_eq!(inputs.learn_from.user, "Can you fix the button?");
        assert!(!crate::vault_learning::should_extract(
            &inputs.learn_from.user,
            true
        ));
        let job = learning_job(
            Some(inputs.learn_from),
            "success",
            "Fixed.",
            "run-1".into(),
            true,
        )
        .expect("a successful turn has a job");
        assert!(!crate::vault_learning::should_extract(
            &job.user,
            job.changed
        ));

        // The user's own words still open the gate.
        let said = format!(
            "{history}{}",
            composed(
                "Never use Gmail.",
                "We decided the report ships on Fridays."
            )
        );
        let told = send(v, "s-20261006-said", &said);
        let inputs = turn_inputs(v, &told, "s-20261006-said", false);
        assert!(crate::vault_learning::should_extract(
            &inputs.learn_from.user,
            false
        ));
    }

    /// The wiring `turn_inputs` feeds: no recall, connector or learning input
    /// in `execute_turn` comes from the composed prompt.
    #[test]
    fn execute_turn_takes_recall_connectors_and_learning_from_the_request() {
        let whole = include_str!("chat.rs");
        let start = whole
            .find(concat!("pub(crate) async fn ", "execute_turn("))
            .unwrap();
        let turn = &whole[start..start + whole[start..].find("\n}\n").unwrap()];
        assert!(turn.contains(concat!(
            "turn_inputs_prefetched(",
            "&vault, &payload, &session_key, resumed, recall_prefetch)"
        )));
        assert!(!turn.contains(concat!("vault_learning::", "recall(")));
        assert!(crate::source_pins::contains(
            turn,
            concat!(
                "connectors::attach(agent, &connector_perm, false, ",
                "&connector_query, &deadline, &jev, ",
                "Some((vault.as_path(), session_key.as_str())))"
            )
        ));
        // A bare scratch turn (overnight S7) learns nothing, and only the
        // person's typed words teach (fewer learnings, 10-11).
        assert!(turn.contains(concat!("learns.then_", "some(learn_from),")));
        assert!(!turn.contains(concat!("Some(", "original_prompt)")));
        // Attached sources are ranked against the request too (F3 part 2).
        assert!(turn.contains(concat!("query: ", "&source_query,")));
        // Both routes' lifecycle decisions reach the run's manifest (F7).
        assert!(turn.contains(concat!(
            ".add_lifecycle(recall.history, ",
            "recall.lifecycle.clone());"
        )));
        // So does whether recall ranked by meaning (OF26 review).
        assert!(turn.contains(concat!(".recall_meaning = ", "recall.meaning.recorded();")));
        assert!(turn.contains(concat!(
            "manifest.add_lifecycle(prepared.history, ",
            "prepared.lifecycle);"
        )));
    }
}

/// F6 (agent-effectiveness wave 1, PR C2): the permission wording a chat
/// agent reads is generated from the mode contract (`mode_contract`, owner
/// decision Q1), and each mode's text says what that mode really does.
#[cfg(test)]
mod mode_wording_tests {
    use super::*;
    use crate::mode_contract::{Mode, EXTERNAL_CONFIRM, HOLLOW_INTERNAL_CONFIRM};

    fn rich_caps() -> AttachedCaps {
        AttachedCaps {
            computer: true,
            vault: true,
            questions: true,
            tasks: true,
            app: true,
            ..Default::default()
        }
    }

    fn mail(auto_approve: bool) -> crate::connectors::Attachment {
        crate::connectors::Attachment {
            id: "mail".into(),
            name: "Mail".into(),
            entry: serde_json::json!({}),
            auto_approve,
            claude_login: false,
            remedy: None,
        }
    }

    /// A vault chat's Claude turn line in mode `perm`, with the folders it
    /// reads (OB02): its working directory and scratch folder.
    fn claude_turn_line(perm: &str, app_changes: bool) -> String {
        let reads = crate::mode_contract::Reads {
            scratch: true,
            extra: &[],
            app: true,
            vault: true,
        };
        crate::mode_contract::turn_line(
            perm,
            true,
            app_changes,
            true,
            crate::capability_contract::reads_only_its_roots(AgentKind::Claude).then_some(&reads),
        )
    }

    /// Everything a vault chat's Claude agent is told about permissions in
    /// mode `perm`: the appended prompt, the adapter's policy, the turn's
    /// `<hollow-mode>` line (HB01) and the connector note.
    fn fixture(perm: &str) -> String {
        let vault = Path::new("/Users/x/Second-brain");
        let append = super::chat_append_prompt(
            false,
            vault,
            vault,
            rich_caps(),
            crate::visual_prompt::CanvasFrequency::Helps,
            perm,
        );
        let policy = crate::agent::chat_policy(AgentKind::Claude, perm);
        let mode = claude_turn_line(perm, true);
        let note = crate::connectors::prompt_note(&[mail(false)], perm, false);
        format!("{append}\n\n{policy}\n\n{mode}{note}")
    }

    #[test]
    fn a_bypass_chat_drops_hollow_confirm_wording_but_keeps_the_external_one() {
        let text = fixture("bypassPermissions");
        let mut wrong: Vec<String> = HOLLOW_INTERNAL_CONFIRM
            .iter()
            .filter(|phrase| text.contains(*phrase))
            .map(|phrase| format!("bypass still says {phrase:?}"))
            .collect();
        if !text.contains(EXTERNAL_CONFIRM) {
            wrong.push("the external confirmation is missing".into());
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
        assert!(
            text.contains("Hollow shows no approval or confirm cards"),
            "{text}"
        );
        assert!(text.contains("In bypass they all run without a prompt."));
    }

    /// Question cards and task suggestions still reach the person in a
    /// bypass chat; only approval and confirm cards are gone (wave 1
    /// follow-up to C2). An agent without owned MCP still posts question
    /// cards through a fenced block, so its policy can't say Hollow attaches
    /// no cards.
    #[test]
    fn bypass_and_no_owned_mcp_wording_match_the_cards_that_still_show() {
        let text = fixture("bypassPermissions");
        assert!(
            text.contains(crate::interaction_schema::GUIDANCE_MCP),
            "a bypass chat still asks on question cards"
        );
        assert!(!text.contains("shows no cards"), "{text}");
        assert!(text.contains("no approval or confirm cards"), "{text}");
        for mode in Mode::ALL {
            let policy = crate::mode_contract::policy(mode.perm(), false);
            assert!(!policy.contains("attach its cards"), "{mode:?}: {policy}");
        }
    }

    /// The auto fixture names exactly the classes that still show a card
    /// in auto (`app_actions::asks_in_mode`), and that auto preapproves no
    /// library connector (`connector_preapproved`).
    #[test]
    fn an_auto_chat_names_the_classes_that_still_ask() {
        let text = fixture("auto");
        let missing: Vec<&str> = crate::app_actions::AUTO_ASK_CLASSES
            .iter()
            .map(|class| class.label)
            .chain(crate::mode_contract::AUTO_ASK_EXTRAS)
            .filter(|label| !text.contains(label))
            .collect();
        assert!(missing.is_empty(), "auto doesn't name {missing:#?}");
        assert!(text.contains("Auto preapproves none of them"), "{text}");
        assert!(text.contains(EXTERNAL_CONFIRM));
        assert!(
            text.contains("allows them on a card"),
            "computer tier 2 asks in auto"
        );
    }

    /// Every mode keeps the external confirmation once, in the connector
    /// note (the policy only points at it), and only bypass loses the card
    /// wording.
    #[test]
    fn every_mode_keeps_the_external_confirmation() {
        for mode in Mode::ALL {
            let text = fixture(mode.perm());
            assert_eq!(
                text.matches(EXTERNAL_CONFIRM).count(),
                1,
                "{mode:?}: the note's"
            );
            let carded = text.contains("on a card");
            assert_eq!(carded, mode != Mode::Bypass, "{mode:?}");
        }
    }

    /// HB01 (evening 10-16): what Claude's system prompt holds (the
    /// appended prompt and the policy after it) is byte-identical in every
    /// mode that acts, for a vault chat, a workspace and the Main agent, so
    /// a wake (acceptEdits) between two of the person's bypass turns keeps
    /// both the native session and its prompt cache. Only plan differs.
    #[test]
    fn the_configured_half_is_the_same_in_every_mode_that_acts() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let main = crate::workspaces::ensure_main_agent(vault).unwrap();
        let ws = vault.join("workspaces/w1");
        let helps = crate::visual_prompt::CanvasFrequency::Helps;
        for (is_workspace, cwd) in [(false, vault), (true, ws.as_path()), (true, main.as_path())] {
            let half = |perm: &str| {
                let append =
                    super::chat_append_prompt(is_workspace, vault, cwd, rich_caps(), helps, perm);
                let policy = crate::agent::chat_policy(AgentKind::Claude, perm);
                format!("{append}\n\n{policy}")
            };
            let bypass = half("bypassPermissions");
            for mode in [Mode::AcceptEdits, Mode::Auto, Mode::Default] {
                assert_eq!(half(mode.perm()), bypass, "{cwd:?} {mode:?}");
            }
            assert_ne!(half("plan"), bypass, "{cwd:?}");
        }
    }

    /// OB02: the headless wording and the readable roots ride only in the
    /// turn's `<hollow-mode>` line. The configured half (what Claude's
    /// fingerprint hashes, `chat_resume::config_fingerprint`) holds neither,
    /// so the change resets no native session by itself.
    #[test]
    fn the_headless_wording_and_roots_stay_out_of_the_configured_half() {
        let vault = Path::new("/Users/x/Second-brain");
        let helps = crate::visual_prompt::CanvasFrequency::Helps;
        for perm in ["acceptEdits", "default"] {
            let append = super::chat_append_prompt(false, vault, vault, rich_caps(), helps, perm);
            let policy = crate::agent::chat_policy(AgentKind::Claude, perm);
            let half = format!("{append}\n\n{policy}");
            for absent in [
                crate::mode_contract::HEADLESS_FORMS,
                "refused, not asked",
                "Reads stop at",
            ] {
                assert!(!half.contains(absent), "{perm}: {absent}");
            }
            let line = claude_turn_line(perm, true);
            assert!(
                line.contains(crate::mode_contract::HEADLESS_FORMS),
                "{line}"
            );
            assert!(
                line.contains("Reads stop at the working directory and scratch folder"),
                "{line}"
            );
        }
    }

    /// The brief keeps only what's true of the current app: no retired
    /// tabs, and one consistent Notion story (no sync; Notion's official
    /// connector is an ordinary connector; 2026-11-11).
    #[test]
    fn the_brief_drops_legacy_navigation_and_stale_notion_guidance() {
        let vault = Path::new("/Users/x/Second-brain");
        let brief = super::environment_brief(vault, vault, rich_caps());
        for stale in [
            "Today, Tasks, Projects, Calendar, Vault",
            "Your work, Review, Tasks",
            "Hollow's tabs",
            "Meetings and projects are not stored",
            "explicit import connector only",
            "Local edits remain local",
            "Obsidian is the source of truth",
            "recordings, and Calendar",
            "library/atlas",
            // P1-20: meetings aren't Notion transcripts; Plaud and calendar name their own sources.
            "Notion meeting transcripts",
        ] {
            assert!(!brief.contains(stale), "{stale}");
        }
        for current in [
            "Projects, Tasks, Library, Automations",
            "Needs you opens from the bell, then See all",
            "tracked projects keep their PROJECT.md",
            "Linear is the source of truth",
            "Hollow no longer syncs with Notion",
            "notion-official",
            "Never run Hollow's binary without a known command",
            // The shared meetings wording (src/meetingsCopy.ts's PLAUD_FOR_AGENTS and CALENDAR_SOURCE).
            "Plaud tools for agents come from the Plaud connector, with its own sign-in. Calendar comes from the source you connect in Settings → Connectors.",
        ] {
            assert!(brief.contains(current), "{current}");
        }
        // The named destinations are the rail's labelled rows, which the
        // rail renders from navigation.ts's NAV_PAGES since P1-05.
        let rail = include_str!("../../src/components/Rail.tsx");
        assert!(rail.contains("<For each={RAIL_PRIMARY}>"));
        let nav = include_str!("../../src/navigation.ts");
        // Primary destinations retain their rail and Go-menu routes.
        // Needs you keeps its stored review id and Go item, behind the bell.
        let menu = include_str!("app_menu.rs");
        let go = &menu[menu.find("\"Go\",").expect("the Go submenu")..];
        let go = &go[..go.find("Submenu::with_items").unwrap_or(go.len())];
        for (label, item) in [
            ("Projects", "go-work"),
            ("Tasks", "go-tasks"),
            ("Library", "go-library"),
            ("Automations", "go-automations"),
        ] {
            assert!(nav.contains(&format!("label: '{label}'")), "{label}");
            assert!(
                go.contains(&format!("item(app, \"{item}\")")),
                "{label}: {item}"
            );
        }
        assert!(crate::source_pins::contains(nav, "label: 'Needs you'"));
        assert!(crate::source_pins::contains(nav, "page.id !== 'review'"));
        assert!(crate::source_pins::contains(go, "item(app, \"go-review\")"));
    }

    /// The generated wording per mode, checked in so a reviewer sees what
    /// each mode says: `tests/fixtures/prompts/mode-wording.json`.
    /// Regenerate with `HOLLOW_REGENERATE_PROMPTS=1 cargo test --lib
    /// mode_wording` (from `src-tauri/`).
    #[test]
    fn mode_wording_fixture_is_current() {
        let mut modes = serde_json::Map::new();
        for mode in Mode::ALL {
            let perm = mode.perm();
            modes.insert(
                perm.to_string(),
                serde_json::json!({
                    "policyOwnedMcp": crate::mode_contract::policy(perm, true),
                    "policyNoOwnedMcp": crate::mode_contract::policy(perm, false),
                    "appCardsChat": crate::mode_contract::app_cards(perm, false),
                    "appCardsAutomation": crate::mode_contract::app_cards(perm, true),
                    "connectorNote": crate::connectors::prompt_note(&[mail(true), crate::connectors::Attachment { id: "drive".into(), name: "Drive".into(), ..mail(false) }], perm, false),
                    "connectorNoteAutomation": crate::connectors::prompt_note(&[mail(false)], perm, true),
                    "computerBrief": crate::mode_contract::computer_brief(perm),
                    "chatPolicy": crate::mode_contract::chat_policy(perm, true),
                    // A plan turn's app tools only read: no card sentence.
                    "turnLine": claude_turn_line(perm, mode != Mode::Plan),
                }),
            );
        }
        let fixture = serde_json::json!({
            "_doc": "Generated by chat.rs mode_wording_fixture_is_current from mode_contract (F6, owner decision Q1). Do not edit; regenerate with HOLLOW_REGENERATE_PROMPTS=1 cargo test --lib mode_wording.",
            "computerTier2": {
                "bypass": crate::mode_contract::computer_tier2(true),
                "otherModes": crate::mode_contract::computer_tier2(false),
            },
            "modes": modes,
        });
        let text = format!("{}\n", serde_json::to_string_pretty(&fixture).unwrap());
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/prompts/mode-wording.json");
        if std::env::var_os("HOLLOW_REGENERATE_PROMPTS").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &text).unwrap();
        }
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            current == text,
            "tests/fixtures/prompts/mode-wording.json is stale; from src-tauri/ run HOLLOW_REGENERATE_PROMPTS=1 cargo test --lib mode_wording"
        );
    }

    /// Byte sizes of the permission texts, for the evidence doc
    /// (`cargo test --lib mode_wording_sizes -- --nocapture`).
    #[test]
    fn mode_wording_sizes() {
        let vault = Path::new("/Users/x/Second-brain");
        let ws = vault.join("workspaces/w1");
        let helps = crate::visual_prompt::CanvasFrequency::Helps;
        for mode in Mode::ALL {
            let perm = mode.perm();
            let policy = crate::agent::chat_policy(AgentKind::Claude, perm).len();
            let note = crate::connectors::prompt_note(&[mail(false)], perm, false).len();
            let app = crate::app_mcp::chat_prompt_guidance(false).len();
            let brief = super::environment_brief(vault, vault, rich_caps()).len();
            let mode = claude_turn_line(perm, true).len();
            let vault_chat =
                super::chat_append_prompt(false, vault, vault, rich_caps(), helps, perm).len();
            let ws_chat =
                super::chat_append_prompt(true, vault, &ws, rich_caps(), helps, perm).len();
            eprintln!("SIZES {perm}: policy={policy} note={note} policy+note={} app_guidance={app} env_brief={brief} turn_line={mode} vault_chat_append={vault_chat} workspace_chat_append={ws_chat}", policy + note);
        }
    }
}

/// Agent effectiveness F6, budget part (wave 3 · lane G, G1): what Hollow's
/// configured half and visual guidance cost per turn, measured for three
/// representative turns per provider before anything changes, and kept as a
/// generated fixture so a later change shows up as a diff
/// (`tests/fixtures/prompts/context-budget.json`; regenerate with
/// `HOLLOW_REGENERATE_PROMPTS=1 cargo test --lib context_budget`). Bytes of
/// Hollow's own text only: the workspace envelope, recall, evidence and the
/// compiler's blocks depend on the vault and are measured per run in the
/// manifest, and none of them is changed here.
#[cfg(test)]
#[path = "chat_g2_budget_tests.rs"]
mod g2_budget_tests;

#[cfg(test)]
mod g1_budget_tests {
    use super::*;
    use crate::visual_prompt::CanvasFrequency;

    /// A turn type: the surface it runs in and the person's words.
    struct Turn {
        name: &'static str,
        workspace: bool,
        request: &'static str,
    }

    const TURNS: [Turn; 3] = [
        Turn {
            name: "simpleText",
            workspace: false,
            request: "What time zone is the Lisbon office in?",
        },
        Turn {
            name: "visual",
            workspace: false,
            request: "Compare the three pricing plans in a chart.",
        },
        Turn {
            name: "workspaceResearch",
            workspace: true,
            request: "Summarize what the attached sources say about churn, with citations.",
        },
    ];

    const AGENTS: [AgentKind; 4] = [
        AgentKind::Claude,
        AgentKind::Codex,
        AgentKind::Antigravity,
        AgentKind::Grok,
    ];

    /// What a chat with `agent` typically has attached: Claude and Codex the
    /// owned servers a default install turns on for chats (question cards,
    /// task tools, app tools, vault search), the others the fenced
    /// question block only.
    fn caps_for(agent: AgentKind) -> AttachedCaps {
        if crate::agent::capabilities(agent).owned_mcp {
            AttachedCaps {
                questions: true,
                tasks: true,
                app: true,
                vault: true,
                agent: Some(agent),
                ..Default::default()
            }
        } else {
            AttachedCaps {
                questions_fenced: true,
                agent: Some(agent),
                ..Default::default()
            }
        }
    }

    /// The per-turn visual guide block a turn carries (G1), in a
    /// conversation of turns like it: the first on a fresh native session,
    /// the next resumed. The guide file is taken as written.
    fn turn_guide_bytes(agent: AgentKind, turn: &Turn, first: bool) -> u64 {
        use crate::visual_prompt::{
            delivery, guide_block, visual_cue, visual_guide, Asking, Delivery,
        };
        let caps = caps_for(agent);
        let guide = visual_guide(
            CanvasFrequency::Helps,
            Asking::of(caps.questions, caps.questions_fenced),
        );
        let sha = crate::run_manifest::sha256_hex(guide.as_bytes());
        let cue = visual_cue(turn.request, &[]);
        let opening = delivery(CanvasFrequency::Helps, cue, true, true, None, &sha);
        let now = if first {
            opening
        } else {
            let given = (opening == Delivery::Turn).then_some(sha.as_str());
            delivery(CanvasFrequency::Helps, cue, true, false, given, &sha)
        };
        if now == Delivery::Turn {
            guide_block(&guide).len() as u64
        } else {
            0
        }
    }

    fn row(agent: AgentKind, turn: &Turn, first: bool) -> serde_json::Value {
        let vault = Path::new("/Users/you/Hollow");
        let cwd = if turn.workspace {
            vault.join("workspaces/release-review")
        } else {
            vault.to_path_buf()
        };
        let perm = "acceptEdits";
        let mut layers = chat_append_layers(
            turn.workspace,
            vault,
            &cwd,
            caps_for(agent),
            CanvasFrequency::Helps,
            perm,
        );
        layers.brief = super::g2_budget_tests::normalize_downloads(&layers.brief);
        let policy = crate::agent::chat_policy(agent, perm);
        let append = layers.joined();
        // Claude: `--append-system-prompt` (append, blank line, policy),
        // every process, cached by the provider. Others: the policy, the
        // append and the turn's prompt, as one prompt, every turn.
        let configured = (append.len() + 2 + policy.len()) as u64;
        let guide = turn_guide_bytes(agent, turn, first);
        serde_json::json!({
            "agent": agent.as_str(),
            "turn": turn.name,
            "session": if first { "first" } else { "resumed" },
            "visualPrompt": layers.visual.len(),
            "capabilityIndex": layers.index.len(),
            "brief": layers.brief.len(),
            "guidance": layers.policy.len(),
            "workspacePolicy": policy.len(),
            "configured": configured,
            "turnGuide": guide,
            "perTurn": configured + guide,
        })
    }

    /// Per-turn bytes at base `c6035e93` (the measured baseline above),
    /// by provider and turn: Claude and Codex, then Antigravity and Grok.
    fn before(agent: AgentKind, turn: &Turn) -> u64 {
        match (crate::agent::capabilities(agent).owned_mcp, turn.workspace) {
            (true, false) => 23_790,
            (true, true) => 20_204,
            (false, false) => 20_496,
            (false, true) => 17_774,
        }
    }

    fn per_turn(row: &serde_json::Value) -> u64 {
        row["perTurn"].as_u64().unwrap()
    }

    /// F6 acceptance: a turn that needs no visual answer (a simple text
    /// question, a workspace research request) gets materially less of
    /// Hollow's text, first turn and resumed, with every provider.
    /// [`SCREEN_JOBS_ALLOWANCE`] and [`DEPENDENCY_RULE_ALLOWANCE`] are the
    /// only growth past that 10 KB margin.
    #[test]
    fn a_simple_text_turn_gets_materially_less_injected_context() {
        for agent in AGENTS {
            for turn in TURNS.iter().filter(|t| t.name != "visual") {
                for first in [true, false] {
                    let now = per_turn(&row(agent, turn, first));
                    let was = before(agent, turn);
                    eprintln!(
                        "{} {} first={first}: {was} -> {now} bytes",
                        agent.as_str(),
                        turn.name
                    );
                    assert!(
                        now + 10_000
                            <= was
                                + (super::SCREEN_JOBS_ALLOWANCE + super::DEPENDENCY_RULE_ALLOWANCE)
                                    as u64,
                        "{} {}: {now} bytes, was {was}",
                        agent.as_str(),
                        turn.name
                    );
                }
            }
        }
    }

    /// The workspace and quick-chat briefs named only computer, teamwork,
    /// questions and tasks (F6): an attached vault server or app server
    /// was never mentioned.
    #[test]
    fn a_workspace_chat_is_told_about_its_vault_and_app_tools() {
        let vault = Path::new("/Users/you/Hollow");
        let cwd = vault.join("workspaces/release-review");
        let caps = AttachedCaps {
            questions: true,
            vault: true,
            app: true,
            ..Default::default()
        };
        let prompt = super::chat_append_prompt(
            true,
            vault,
            &cwd,
            caps,
            CanvasFrequency::Helps,
            "acceptEdits",
        );
        for name in [
            "hollow_vault",
            "search_vault",
            "read_source",
            "hollow_app",
            "list_actions",
        ] {
            assert!(prompt.contains(name), "{name} missing");
        }
    }

    /// Every combination of the attached-server flags (the manifest's
    /// `capabilities.ownedServers` are the same names), at every mode.
    fn every_caps() -> Vec<AttachedCaps> {
        (0u32..1 << 10)
            .map(|bits| {
                let on = |bit: u32| bits & (1 << bit) != 0;
                AttachedCaps {
                    questions: on(0),
                    questions_fenced: !on(0) && on(1),
                    vault: on(2),
                    app: on(3),
                    app_read_only: on(3) && on(4),
                    tasks: on(5),
                    tasks_read_only: on(5) && on(4),
                    computer: on(6),
                    team_codex: on(7),
                    services: on(8),
                    team_grok: on(9),
                    ..Default::default()
                }
            })
            .collect()
    }

    /// OF07 review: the build-slot line follows the agent and mode exactly
    /// as the environment does (`build_slots::can_take_slot`): never in plan,
    /// Antigravity only in bypass (its sandboxed modes run no commands),
    /// Grok's `default` (read-only, which writes `/tmp`) included.
    #[test]
    fn the_slot_line_is_there_only_where_the_turn_can_take_a_slot() {
        for agent in [
            AgentKind::Claude,
            AgentKind::Codex,
            AgentKind::Antigravity,
            AgentKind::Grok,
        ] {
            let caps = AttachedCaps {
                agent: Some(agent),
                ..AttachedCaps::default()
            };
            for perm in [
                "plan",
                "default",
                "acceptEdits",
                "auto",
                "bypassPermissions",
            ] {
                let index = capability_index(caps, CanvasFrequency::Helps, perm);
                assert_eq!(
                    index.contains(SLOT_INDEX_LINE),
                    crate::build_slots::can_take_slot(agent, perm),
                    "{agent:?} {perm}"
                );
            }
        }
        let grok = AttachedCaps {
            agent: Some(AgentKind::Grok),
            ..AttachedCaps::default()
        };
        let index = capability_index(grok, CanvasFrequency::Helps, "default");
        assert!(
            index.contains(SLOT_INDEX_LINE),
            "its read-only profile writes /tmp"
        );
    }

    /// F6 acceptance: the index lists every capability the run has, and
    /// nothing it lacks: exactly the owned servers the run's manifest
    /// records, connectors only where an agent can have them, and the
    /// answer forms for the Canvases choice with where their formats are.
    #[test]
    fn the_index_lists_every_attached_capability_truthfully() {
        for caps in every_caps() {
            for perm in [
                "plan",
                "default",
                "acceptEdits",
                "auto",
                "bypassPermissions",
            ] {
                for canvases in CanvasFrequency::ALL {
                    let index = capability_index(caps, canvases, perm);
                    let named: Vec<&str> = crate::mcp_attach::OWNED_SERVER_NAMES
                        .into_iter()
                        .filter(|name| index.contains(name))
                        .collect();
                    let mut attached = caps.server_names();
                    attached.sort_by_key(|name| {
                        crate::mcp_attach::OWNED_SERVER_NAMES
                            .iter()
                            .position(|n| n == name)
                    });
                    assert_eq!(named, attached, "{caps:?} {perm}");
                    assert_eq!(
                        index.contains("Library connectors"),
                        !caps.questions_fenced && perm != "plan",
                        "{caps:?} {perm}"
                    );
                    // OF07: `hollow-slot` is on PATH wherever Bash runs.
                    assert_eq!(
                        index.contains("`hollow-slot <cmd>`"),
                        perm != "plan",
                        "{caps:?} {perm}"
                    );
                    assert_eq!(
                        index.contains("(read-only)"),
                        (caps.app && caps.app_read_only) || (caps.tasks && caps.tasks_read_only)
                    );
                    assert!(!index.contains("hollow-whiteboard"), "boards are gone");
                    assert_eq!(
                        index.contains("`hollow-canvas`") && index.contains("ask-canvases"),
                        canvases != CanvasFrequency::Off
                    );
                    assert_eq!(
                        index.contains(crate::visual_prompt::GUIDE_PATH),
                        canvases == CanvasFrequency::Helps,
                        "names where the formats are (Off has none)"
                    );
                    assert!(index.contains("Skills:") && index.contains("$HOLLOW_SCRATCH_DIR"));
                    assert!(
                        index.len() <= 1_600,
                        "a compact index: {} bytes",
                        index.len()
                    );
                }
            }
        }
        // The real attach for a Claude chat: the index names what the
        // server map (and so the manifest) holds.
        let dir = Path::new("/tmp");
        let parent = DelegateParent {
            surface: Surface::Chat,
            scope: "scope".into(),
            run_id: "run-1".into(),
            scratch: None,
            tool_limits: Default::default(),
        };
        for agent in AGENTS {
            let (servers, caps) = crate::mcp_attach::server_map_for(
                agent,
                "acceptEdits",
                dir,
                dir,
                &parent,
                crate::tasks_mcp::TasksAccess::ReadWrite,
                None,
            );
            let index = capability_index(caps, CanvasFrequency::Helps, "acceptEdits");
            // It ends with this agent's line of the contract (F9).
            let line = crate::capability_contract::guidance_line(agent);
            assert!(
                !line.is_empty() && index.ends_with(&format!("\n{line}")),
                "{}: {index}",
                agent.as_str()
            );
            assert!(
                index.len() <= 1_900,
                "{}: {} bytes",
                agent.as_str(),
                index.len()
            );
            let manifest = crate::chat_resume::mcp_server_names(&servers);
            for name in crate::mcp_attach::OWNED_SERVER_NAMES {
                assert_eq!(
                    index.contains(name),
                    manifest.iter().any(|m| m == name),
                    "{} {name}",
                    agent.as_str()
                );
            }
        }
    }

    /// The wiring, through a real session store: a visual turn on a fresh
    /// session carries the guide just ahead of the caller's text; the next
    /// resumed visual turn doesn't (it was given); a plain turn leaves it to
    /// the file; a session reset gives it again; and without a file the
    /// turn always carries it on a fresh session. The manifest says which.
    #[test]
    fn a_visual_turn_carries_the_guide_once_per_native_session() {
        use crate::context_compiler::Session;
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let scratch = tempfile::tempdir().unwrap();
        let key = "s-20261007-g1000001";
        let composed =
            "<hollow-chat-intent>i</hollow-chat-intent>\n\nCompare the plans in a chart.";
        let compile = |session, guide: &TurnGuide, ord: i64| {
            let compiled = crate::context_compiler::compile(
                crate::context_compiler::Entry::Composer,
                session,
                &[],
                &crate::context_compiler::Profile::default(),
                composed,
            );
            // Each of these turns reached its agent: what it gave is
            // recorded, as execute_turn does after the spawn.
            let (compiled, given) = guide.apply(vault, key, session, compiled, composed, true);
            if let Some(given) = given {
                given.record(vault, key, ord);
            }
            compiled
        };
        let asking = crate::visual_prompt::Asking::Cards;
        let visual = TurnGuide::new(
            CanvasFrequency::Helps,
            asking,
            Some("chart"),
            Some(scratch.path()),
        );
        assert!(scratch
            .path()
            .join(crate::visual_prompt::GUIDE_FILE)
            .is_file());
        let block = crate::visual_prompt::guide_block(&visual.guide);

        let first = compile(Session::New, &visual, 1);
        assert_eq!(first.body, format!("{block}{composed}"));
        let record = first.record.visual_guide.clone().unwrap();
        assert_eq!(
            (record.delivery.as_str(), record.cue.as_deref(), record.file),
            ("turn", Some("chart"), true)
        );
        assert_eq!(record.bytes, visual.guide.len() as u64);
        let layers = crate::run_manifest::composer_layers(&first.body);
        assert_eq!(layers[0].name, "visualGuide");
        assert_eq!(
            crate::sessions::strip_injected_context(&first.body),
            "Compare the plans in a chart.",
            "a title never shows the guide"
        );

        let again = compile(Session::Resume, &visual, 2);
        assert_eq!(again.body, composed);
        assert_eq!(again.record.visual_guide.unwrap().delivery, "given");

        let plain = TurnGuide::new(CanvasFrequency::Helps, asking, None, Some(scratch.path()));
        let simple = compile(Session::Resume, &plain, 3);
        assert_eq!(simple.body, composed);
        assert_eq!(simple.record.visual_guide.unwrap().delivery, "onDemand");

        let reset = compile(
            Session::Reset(crate::chat_resume::ResetReason::ConfigChanged),
            &visual,
            4,
        );
        assert!(reset.body.contains(&block), "a fresh session gets it again");
        let plain_fresh = compile(Session::New, &plain, 5);
        assert_eq!(
            plain_fresh.record.visual_guide.unwrap().delivery,
            "onDemand"
        );
        let after = compile(Session::Resume, &visual, 6);
        assert!(
            after.body.contains(&block),
            "that fresh session was never given it"
        );

        let no_file = TurnGuide::new(CanvasFrequency::Helps, asking, None, None);
        assert!(!no_file.file);
        let fallback = compile(Session::Retry, &no_file, 7);
        assert!(
            fallback.body.starts_with(&block),
            "no file: the turn carries it"
        );
        assert_eq!(fallback.record.visual_guide.unwrap().delivery, "turn");

        let most = TurnGuide::new(
            CanvasFrequency::Most,
            asking,
            Some("chart"),
            Some(scratch.path()),
        );
        let in_prompt = compile(Session::New, &most, 8);
        assert_eq!(in_prompt.body, composed);
        assert_eq!(in_prompt.record.visual_guide.unwrap().delivery, "prompt");
    }

    /// Wave 3 review (G1): composing a turn records nothing about the
    /// guide. A resumed turn whose block was compiled but never reached
    /// its agent (a stop before spawn, a spawn failure) leaves the native
    /// session without it, so the next resumed visual turn carries it.
    #[test]
    fn a_guide_compiled_for_a_turn_that_never_spawned_is_not_recorded_as_given() {
        use crate::context_compiler::Session;
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let scratch = tempfile::tempdir().unwrap();
        let key = "s-20261007-v1000001";
        let composed = "Compare the plans in a chart.";
        let visual = TurnGuide::new(
            CanvasFrequency::Helps,
            crate::visual_prompt::Asking::Cards,
            Some("chart"),
            Some(scratch.path()),
        );
        let block = crate::visual_prompt::guide_block(&visual.guide);
        let compile = || {
            let compiled = crate::context_compiler::compile(
                crate::context_compiler::Entry::Composer,
                Session::Resume,
                &[],
                &crate::context_compiler::Profile::default(),
                composed,
            );
            visual.apply(vault, key, Session::Resume, compiled, composed, true)
        };
        let (unsent, given) = compile();
        assert!(unsent.body.contains(&block));
        assert_eq!(
            crate::visual_prompt::given(vault, key),
            None,
            "nothing is recorded before the agent spawned"
        );
        let (next, given_next) = compile();
        assert!(
            next.body.contains(&block),
            "the session never got the guide, so the next turn carries it"
        );
        assert_eq!(given, given_next);
        // Once a turn's agent spawned, its session holds it.
        given_next.unwrap().record(vault, key, 2);
        let (after, _) = compile();
        assert_eq!(after.body, composed);
        assert_eq!(after.record.visual_guide.unwrap().delivery, "given");
    }

    /// A durable child task's visual turn (F10 D1 review): the compiler
    /// moves the packet first, so the guide goes just ahead of the task
    /// text, after the packet and the profile, not at the front.
    #[test]
    fn a_child_tasks_visual_guide_goes_after_its_packet() {
        use crate::context_compiler::Session;
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let scratch = tempfile::tempdir().unwrap();
        let key = "s-20261007-d1000001";
        let task = "Compare the plans in a chart.";
        let packet = crate::delegation::build(
            crate::delegation::Contract::ChildTask,
            crate::delegation::Request {
                constraints: vec!["Keep it to one chart.".into()],
                ..Default::default()
            },
            crate::delegation::Parent::default(),
        )
        .render()
        .text;
        let composed = crate::delegation::prepend(&packet, task);
        let profile = crate::context_compiler::Profile {
            enabled: true,
            body: "Identity: Synthetic Tester".into(),
            files: 1,
            ..Default::default()
        };
        let guide = TurnGuide::new(
            CanvasFrequency::Helps,
            crate::visual_prompt::Asking::Cards,
            Some("chart"),
            Some(scratch.path()),
        );
        let block = crate::visual_prompt::guide_block(&guide.guide);
        let off = crate::context_compiler::Profile::default();
        for with_profile in [true, false] {
            let session = Session::New;
            let compiled = crate::context_compiler::compile(
                crate::context_compiler::Entry::AppSession,
                session,
                &[],
                if with_profile { &profile } else { &off },
                &composed,
            );
            let (compiled, _given) = guide.apply(vault, key, session, compiled, &composed, true);
            let body = &compiled.body;
            let at = |needle: &str| body.find(needle).unwrap_or(usize::MAX);
            assert_eq!(at(crate::delegation::OPEN), 0, "{body}");
            assert!(body.ends_with(&format!("{block}{task}")), "{body}");
            let layers: Vec<_> = crate::run_manifest::composer_layers(body)
                .into_iter()
                .map(|layer| layer.name)
                .collect();
            let expected: &[&str] = if with_profile {
                &["taskPacket", "operatorProfile", "visualGuide", "request"]
            } else {
                &["taskPacket", "visualGuide", "request"]
            };
            assert_eq!(layers, expected, "{body}");
        }
    }

    /// F6 acceptance: a visual turn still gets the whole canvas guide (with
    /// the kit reference, `hc-flow` included, and the ask-canvas guide for its
    /// channel), every provider. Boards are gone (2026-10-07).
    #[test]
    fn a_visual_turn_still_gets_the_canvas_guide() {
        for agent in AGENTS {
            let caps = caps_for(agent);
            let asking = crate::visual_prompt::Asking::of(caps.questions, caps.questions_fenced);
            let turn = &TURNS[1];
            let cue = crate::visual_prompt::visual_cue(turn.request, &[]);
            assert!(cue.is_some());
            let guide = crate::visual_prompt::visual_guide(CanvasFrequency::Helps, asking);
            for part in [
                crate::visual_prompt::CANVAS_GUIDE,
                crate::visual_prompt::KIT_REFERENCE,
                crate::visual_prompt::ASK_GUIDE,
            ] {
                assert!(guide.contains(part.trim_end()), "{}", agent.as_str());
            }
            assert!(guide.contains("<hc-flow"), "{}", agent.as_str());
            assert!(!guide.contains("hollow-whiteboard"), "{}", agent.as_str());
            assert_eq!(
                turn_guide_bytes(agent, turn, true),
                crate::visual_prompt::guide_block(&guide).len() as u64
            );
        }
    }

    /// The billed probe's request (`scripts/canvas-real-reply.mjs`) is a
    /// visual turn, so its fixture carries the guide block Hollow would
    /// send with it.
    pub(super) fn probe_request() -> String {
        let script = include_str!("../../scripts/canvas-real-reply.mjs");
        let start = script.find("export const REQUEST = [").expect("REQUEST")
            + "export const REQUEST = [".len();
        let end = start + script[start..].find("].join('\\n');").expect("join");
        script[start..end]
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| {
                line.trim_end_matches(',')
                    .trim_matches('\'')
                    .replace("\\'", "'")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_probe_request_is_a_visual_turn() {
        let request = probe_request();
        assert!(
            request.starts_with("Where does the Q4 launch stand"),
            "{request}"
        );
        assert!(request.ends_with("fix both bugs."), "{request}");
        assert!(crate::visual_prompt::visual_cue(&request, &[]).is_some());
    }

    #[test]
    fn context_budget_fixture_is_current() {
        let mut rows = Vec::new();
        for agent in AGENTS {
            for turn in &TURNS {
                for first in [true, false] {
                    rows.push(row(agent, turn, first));
                }
            }
        }
        let fixture = serde_json::json!({
            "_doc": "Generated by chat.rs g1_budget_tests::context_budget_fixture_is_current: bytes of Hollow's configured half (Claude: --append-system-prompt; Codex, Antigravity, Grok: the prompt preamble, every turn) and of the per-turn visual guide, for three representative turns per provider at Canvases: when it helps, accept edits. Do not edit; regenerate with HOLLOW_REGENERATE_PROMPTS=1 cargo test --lib context_budget.",
            "requests": TURNS.iter().map(|t| serde_json::json!({ "turn": t.name, "workspace": t.workspace, "request": t.request })).collect::<Vec<_>>(),
            "rows": rows,
        });
        let text = format!("{}\n", serde_json::to_string_pretty(&fixture).unwrap());
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/prompts/context-budget.json");
        if std::env::var_os("HOLLOW_REGENERATE_PROMPTS").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &text).unwrap();
        }
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            current == text,
            "tests/fixtures/prompts/context-budget.json is stale; from src-tauri/ run HOLLOW_REGENERATE_PROMPTS=1 cargo test --lib context_budget"
        );
    }
}

/// The relevant skill index in a chat turn (agent effectiveness F9, wave 4 ·
/// V2).
#[cfg(test)]
mod v2_skill_tests {
    use super::*;
    use crate::skill_index::tests::{use_skill_prompt, write_skill, KESTREL};

    fn caps_for(agent: AgentKind) -> AttachedCaps {
        let owned = crate::agent::capabilities(agent).owned_mcp;
        AttachedCaps {
            vault: owned,
            app: owned,
            questions: owned,
            questions_fenced: !owned,
            agent: Some(agent),
            ..AttachedCaps::default()
        }
    }

    /// The pilot's selected-skill task (red first): a workspace chat in
    /// accept edits whose agent may not read `{vault}/.agents/skills`. On
    /// main the agent got only the path in the person's text; now the
    /// turn's message carries the skill's instructions, for every provider.
    #[test]
    fn a_selected_skill_reaches_an_accept_edits_agent_not_just_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("Vault");
        let home = dir.path().join("home");
        std::fs::create_dir_all(vault.join("workspaces/eval-outcome/outputs")).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let skill = write_skill(&vault, "kestrel-status-report", KESTREL);
        let asked = use_skill_prompt(
            "kestrel-status-report",
            &skill.to_string_lossy(),
            "Write this week's Kestrel status report to outputs/status-report.md.",
        );
        let roots = crate::skill_index::Roots::new(Some(&vault), Some(&home));
        for agent in [
            AgentKind::Claude,
            AgentKind::Codex,
            AgentKind::Antigravity,
            AgentKind::Grok,
        ] {
            let caps = caps_for(agent);
            let skills = turn_skills(&roots, &asked, &caps);
            let guide = TurnGuide::new(
                crate::visual_prompt::CanvasFrequency::Helps,
                crate::visual_prompt::Asking::of(caps.questions, caps.questions_fenced),
                None,
                None,
            );
            let rebuild = |_: usize| None;
            let composer = TurnComposer {
                vault: &vault,
                session_key: "s-20261007-v2",
                continued_from: &[],
                host: false,
                turn_ord: 1,
                entry: crate::context_compiler::Entry::Composer,
                operator_enabled: false,
                reader: crate::task_state::Reader::None,
                composed: &asked,
                guide: &guide,
                agent,
                append: "",
                policy: "",
                workspace: None,
                rebuild: &rebuild,
                native: None,
                recall: "",
                connectors_note: "",
                skills: &skills.block,
                evidence: "",
                clock: "<hollow-clock>T</hollow-clock>\n\n",
                context_note: "",
                mode: "<hollow-mode>M</hollow-mode>\n\n",
            };
            let prompt = composer
                .compose(crate::context_compiler::Session::New)
                .prompt;
            // Overnight A2: the turn's clock line rides in the message.
            assert!(
                prompt.contains("<hollow-clock>T</hollow-clock>"),
                "{prompt}"
            );
            // HB01: so does its mode line, right after the clock.
            assert!(
                prompt.contains("<hollow-clock>T</hollow-clock>\n\n<hollow-mode>M</hollow-mode>"),
                "{prompt}"
            );
            for line in [
                "The first line is exactly `KESTREL STATUS · <date>`",
                "`## Done`, `## Next`, `## Risks`",
                "Prepared for the Kestrel steering group.",
            ] {
                assert!(
                    prompt.contains(line),
                    "{agent:?}: the skill's instructions don't reach the agent, only its path: {prompt}"
                );
            }
            let record = skills.record.expect("the manifest records the index");
            assert_eq!(record.indexed[0].name, "kestrel-status-report");
            assert_eq!(record.indexed[0].reason, "selected");
            assert!(record.indexed[0].inline);
            assert_eq!(
                record.reader,
                if caps.vault { "readSkill" } else { "ownTools" },
                "{agent:?}"
            );
        }
    }

    /// The capability index names how this run reads a skill whole, and
    /// stays fixed text (Claude caches it): the relevant skills
    /// themselves are per turn.
    #[test]
    fn the_index_says_how_this_run_reads_a_skill() {
        use crate::visual_prompt::CanvasFrequency;
        for agent in [AgentKind::Claude, AgentKind::Grok] {
            let caps = caps_for(agent);
            let index = capability_index(caps, CanvasFrequency::Helps, "acceptEdits");
            assert!(index.contains("<hollow-skills>"), "{index}");
            assert_eq!(
                index.contains("hollow_vault read_skill"),
                caps.vault,
                "{agent:?}: {index}"
            );
            assert_eq!(index.contains("your own file tools"), !caps.vault);
            assert!(
                !index.contains("kestrel"),
                "no skill names in the fixed text"
            );
        }
        let app_only = AttachedCaps {
            app: true,
            agent: Some(AgentKind::Claude),
            ..AttachedCaps::default()
        };
        assert!(
            capability_index(app_only, CanvasFrequency::Helps, "acceptEdits")
                .contains("skills.read_skill reads one whole by its path")
        );
    }

    /// The manifest records which skills were indexed, why and whether
    /// inline, and which the agent read, by name: never the request's words
    /// or a skill's text.
    #[test]
    fn the_manifest_records_indexed_selected_and_read_skills() {
        use crate::skill_index::tests::meta;
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("Vault");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&vault).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let skill = write_skill(&vault, "kestrel-status-report", KESTREL);
        write_skill(
            &home,
            "release-notes",
            &meta("release-notes", "Draft release notes."),
        );
        let asked = use_skill_prompt(
            "kestrel-status-report",
            &skill.to_string_lossy(),
            "Also draft the release notes for quarterly zircon.",
        );
        let roots = crate::skill_index::Roots::new(Some(&vault), Some(&home));
        let turn = turn_skills(&roots, &asked, &caps_for(AgentKind::Claude));
        let mut c = crate::evidence::EvidenceCollector::new("chat", None, "run-v2-skills");
        c.context_mut().skills = turn.record.clone();
        c.record_tool_use(
            "t1",
            crate::run_manifest::READ_SKILL_TOOL,
            &json!({ "name": "release-notes" }),
        );
        c.record_tool_result("t1", "Skill \"release-notes\" …", false);
        c.record_tool_use(
            "t2",
            crate::run_manifest::READ_SKILL_TOOL,
            &json!({ "name": "nope" }),
        );
        c.record_tool_result("t2", "no skill named", true);
        c.record_tool_use(
            "t3",
            crate::run_manifest::RUN_ACTION_TOOL,
            &json!({ "id": "skills.read_skill", "args": { "path": skill.to_string_lossy() } }),
        );
        c.record_tool_result("t3", "---", false);
        c.finish(&vault, "success", vec![]).unwrap();
        let view = crate::evidence::context_manifest(&vault, Some("run-v2-skills"), None)
            .unwrap()
            .unwrap();
        let json = serde_json::to_value(&view.manifest).unwrap();
        let skills = &json["skills"];
        assert_eq!(skills["reader"], json!("readSkill"), "{json}");
        assert_eq!(skills["indexed"][0]["name"], json!("kestrel-status-report"));
        assert_eq!(skills["indexed"][0]["reason"], json!("selected"));
        assert_eq!(skills["indexed"][0]["inline"], json!(true));
        assert_eq!(skills["indexed"][1]["name"], json!("release-notes"));
        assert_eq!(skills["indexed"][1]["reason"], json!("match"));
        assert!(
            skills["indexed"][0].get("path").is_none(),
            "paths aren't stored"
        );
        assert_eq!(
            skills["read"],
            json!([
                { "name": "release-notes", "via": "readSkill", "ok": true },
                { "via": "readSkill", "ok": false },
                {
                    "name": "kestrel-status-report",
                    "source": "vault",
                    "via": "appAction",
                    "ok": true
                }
            ])
        );
        let text = json.to_string();
        for words in ["zircon", "quarterly", "KESTREL STATUS", "steering group"] {
            assert!(
                !text.contains(words),
                "the manifest holds {words:?}: {text}"
            );
        }
    }
}

/// Agent effectiveness F10 (D2): a stopped turn's stop reaches the child
/// task turns it started, and its receipt's roll-up says what it did.
#[cfg(test)]
mod d2_child_stop_tests {
    use super::*;
    use crate::descendants::tests::{child_turn, record_child};

    /// Morning M3: stop cascades to a handover's successor (its record is
    /// the caller's child task, tool `handover`), as the action says.
    #[test]
    fn a_stopped_turn_stops_its_handover_successor() {
        let _turns = TURN_TEST_LOCK
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let heir = child_turn(v, "s-m3-stop-heir", "", RowStatus::Queued);
        let record = crate::run_manifest::DelegationRecord {
            kind: "childTask".into(),
            tool: crate::app_sessions::HANDOVER_TOOL.into(),
            agent: "claude".into(),
            child_session: Some("s-m3-stop-heir".into()),
            child_queue_id: Some(heir),
            at: crate::session_store::db_time(Utc::now()),
            ..Default::default()
        };
        crate::delegation::store(v, "R-m3-stop-caller", &record);
        let stops: Vec<(i64, &str)> = stop_child_turns(v, "R-m3-stop-caller", Utc::now())
            .into_iter()
            .map(|(id, outcome, _)| (id, outcome))
            .collect();
        assert_eq!(stops, [(heir, "cancelled")]);
        assert_eq!(
            run_queue::get(v, heir).unwrap().unwrap().status,
            RowStatus::Cancelled
        );
    }

    #[test]
    fn a_stopped_turn_stops_the_child_turns_it_started() {
        let _turns = TURN_TEST_LOCK
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let queued = child_turn(v, "s-stop-queued", "", RowStatus::Queued);
        let running = child_turn(v, "s-stop-running", "R-stop-running", RowStatus::Running);
        let ended = child_turn(v, "s-stop-ended", "R-stop-ended", RowStatus::Done);
        for (key, id) in [
            ("s-stop-queued", queued),
            ("s-stop-running", running),
            ("s-stop-ended", ended),
        ] {
            record_child(v, "R-stop-parent", key, id);
        }
        // The ended child's conversation has a later turn of its own.
        let later = child_turn(v, "s-stop-ended", "", RowStatus::Queued);
        let registration = register_stop("s-stop-running");
        let stops: Vec<(i64, &str)> = stop_child_turns(v, "R-stop-parent", Utc::now())
            .into_iter()
            .map(|(id, outcome, _)| (id, outcome))
            .collect();
        assert_eq!(
            stops,
            [
                (queued, "cancelled"),
                (running, "signalled"),
                (ended, "finished")
            ]
        );
        let row = |id| run_queue::get(v, id).unwrap().unwrap().status;
        assert_eq!(
            row(queued),
            RowStatus::Cancelled,
            "the queued child ends cancelled"
        );
        assert!(
            registration.signal().requested(),
            "the running child is stopped"
        );
        assert_eq!(
            row(later),
            RowStatus::Queued,
            "never a later turn of that conversation"
        );
        // The roll-up the receipt seals: the cancelled child cost nothing.
        let rollup = crate::descendants::rollup(v, "R-stop-parent");
        let cancelled = rollup
            .descendants
            .iter()
            .find(|d| d.child_queue_id == Some(queued))
            .unwrap();
        assert_eq!(cancelled.status, "cancelled");
        assert_eq!(cancelled.cost_usd, Some(0.0));
        let running_child = rollup
            .descendants
            .iter()
            .find(|d| d.child_queue_id == Some(running))
            .unwrap();
        assert_eq!(running_child.cost_usd, None, "unknown until its turn ends");
        // Nothing is left to stop the second time.
        let again = stop_child_turns(v, "R-stop-parent", Utc::now());
        assert!(
            again
                .iter()
                .all(|(id, outcome, _)| *id == running && *outcome == "signalled"
                    || *outcome == "finished"),
            "{again:?}"
        );
    }
}

/// COO rebuild L7: the CFO's decision for a chat turn lands in its run
/// manifest (`cfo`) and on its receipt, which the footer reads.
#[cfg(test)]
mod cfo_turn_tests {
    use super::*;

    #[test]
    fn the_decision_lands_in_the_run_manifest_of_a_chat_turn() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let origin = turn_origin(true, None, 0, false);
        assert_eq!(origin, crate::cfo::Origin::Person);
        let (decision, account) = turn_cfo(
            dir.path(),
            AgentKind::Claude,
            Some("claude-sonnet-5-5"),
            "Fix the flaky test in chat.rs",
            origin,
            false,
            None,
            Some("xhigh".into()),
            None,
        );
        assert_eq!(account, None, "the person's prompt keeps the spread");
        let mut evidence = crate::evidence::EvidenceCollector::new("chat", None, "run-cfo");
        evidence.set_cfo(decision);
        let recorded = evidence.context_mut().cfo.clone().expect("manifest cfo");
        assert_eq!(recorded.kind, crate::cfo::JobKind::Code);
        assert_eq!(recorded.effort.as_deref(), Some("high"));
        assert_eq!(recorded.effort_source, crate::cfo::EffortSource::Cfo);
        assert_eq!(recorded.model.as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(recorded.reason, "code change");
        assert_eq!(evidence.cfo(), Some(&recorded), "and on the receipt");
        let json = serde_json::to_value(evidence.context_mut().clone()).unwrap();
        assert_eq!(json["cfo"]["effortSource"], "cfo");
        assert_eq!(json["cfo"]["kind"], "code");
    }

    #[test]
    fn a_turn_says_who_asked() {
        use crate::cfo::Origin;
        assert_eq!(
            turn_origin(false, Some("checkin"), 0, false),
            Origin::CheckIn
        );
        assert_eq!(turn_origin(false, Some(""), 1, false), Origin::Subagent);
        assert_eq!(turn_origin(false, None, 0, true), Origin::Subagent);
        assert_eq!(turn_origin(false, None, 0, false), Origin::Automation);
    }

    /// Settings plan PR 12 (K2): the composer's per-chat effort rides the
    /// queued turn into the CFO and wins over a fixed level and Automatic;
    /// with none, Fixed keeps the saved level and Automatic picks by job.
    /// Each decision records the level used and who set it.
    #[test]
    fn a_chats_own_effort_wins_then_a_fixed_level_then_automatic() {
        use crate::cfo::settings::{self, EffortMode};
        use crate::cfo::EffortSource;
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let queued = |key: &str, effort: Option<&str>| {
            let accepted = accept_turn_with_effort(
                v,
                ChatSendRequest {
                    prompt: "Fix the flaky test in chat.rs".into(),
                    session_id: None,
                    session_key: Some(key.into()),
                    cwd: v.to_path_buf(),
                    model: None,
                    permission_mode: None,
                    agent: AgentKind::Claude,
                    client_turn_id: None,
                    app_depth: 0,
                    ultracode: false,
                    canvases: None,
                    continued_from: Vec::new(),
                    unattended: false,
                    typed: true,
                    host_asker: None,
                    canned: None,
                    worker: false,
                },
                Utc::now(),
                effort.map(str::to_string),
            )
            .unwrap();
            let row = crate::run_queue::get(v, accepted.queue_id)
                .unwrap()
                .unwrap();
            serde_json::from_value::<ChatTurnPayload>(row.payload).unwrap()
        };
        let decide = |payload: ChatTurnPayload| {
            let origin = turn_origin(payload.person_sent(), None, 0, false);
            let (decision, _) = turn_cfo(
                v,
                AgentKind::Claude,
                None,
                "Fix the flaky test in chat.rs",
                origin,
                false,
                payload.effort,
                Some("xhigh".into()),
                None,
            );
            let mut evidence = crate::evidence::EvidenceCollector::new("chat", None, "run-k2");
            evidence.set_cfo(decision);
            let recorded = evidence.context_mut().cfo.clone().expect("manifest cfo");
            (recorded.effort, recorded.effort_source)
        };
        let used = |effort: &str| Some(effort.to_string());
        for mode in [EffortMode::Automatic, EffortMode::Fixed] {
            settings::save(
                v,
                crate::cfo::CfoSettings {
                    effort_mode: mode,
                    ..crate::cfo::CfoSettings::default()
                },
            )
            .unwrap();
            assert_eq!(
                decide(queued(&format!("s-k2-own-{mode:?}"), Some("max"))),
                (used("max"), EffortSource::You),
                "the chat's own choice wins in {mode:?}"
            );
        }
        // Fixed (still saved): no per-chat choice keeps the saved level.
        assert_eq!(
            decide(queued("s-k2-fixed", None)),
            (used("xhigh"), EffortSource::Fixed)
        );
        settings::save(v, crate::cfo::CfoSettings::default()).unwrap();
        assert_eq!(
            decide(queued("s-k2-auto", None)),
            (used("high"), EffortSource::Cfo),
            "Automatic picks by job kind (code)"
        );
    }
    /// 2026-11-12: a chat's own effort lives in the session store, so every
    /// run of the chat resolves it: the send's own effort first, then the
    /// chat's, then none (the CFO's Fixed or Automatic level).
    #[test]
    fn a_turn_asks_for_the_named_effort_then_the_chats_own_then_none() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let key = "s-20261112-own00001";
        assert_eq!(resolve_requested_effort(v, key, None), None, "no choice");
        crate::session_store::set_chat_effort(v, key, Some("max")).unwrap();
        assert_eq!(resolve_requested_effort(v, key, None), Some("max".into()));
        assert_eq!(
            resolve_requested_effort(v, key, Some("low".into())),
            Some("low".into()),
            "a send that names a level wins"
        );
        assert_eq!(
            resolve_requested_effort(v, key, Some(" ".into())),
            Some("max".into()),
            "a blank name is none"
        );
        assert_eq!(
            resolve_requested_effort(v, "s-20261112-other001", None),
            None
        );
        // Default (and a reset) clears it.
        crate::session_store::set_chat_effort(v, key, None).unwrap();
        assert_eq!(resolve_requested_effort(v, key, None), None);
    }

    /// Every send the backend starts itself queues with no effort of its own
    /// (an automation in the chat, a wake-up, a continuation, a restart
    /// resume, a phone send); each one runs at the chat's own level.
    #[test]
    fn runs_the_backend_starts_use_the_chats_own_effort() {
        use crate::cfo::EffortSource;
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let key_of = |n: usize| format!("s-20261112-own1000{n}");
        for n in 0..5 {
            crate::session_store::set_chat_effort(v, &key_of(n), Some("max")).unwrap();
        }
        // (typed, unattended, host_asker): the composer's own text; a
        // chat-bound automation (unattended); a wake-up (a host turn); a
        // continuation and a resume (attended, not typed); a phone send.
        let paths: [(&str, bool, bool, Option<&'static str>); 5] = [
            ("composer", true, false, None),
            ("automation in chat", false, true, None),
            (
                "wake_me",
                false,
                true,
                Some(crate::main_agent::ASKED_BY_WAKE_ME),
            ),
            ("continue_after / resume", false, false, None),
            ("phone", true, false, None),
        ];
        for (n, (name, typed, unattended, host_asker)) in paths.into_iter().enumerate() {
            // One queued turn per chat, so each path gets its own.
            let key_owned = key_of(n);
            let key = key_owned.as_str();
            let accepted = accept_turn_with_effort(
                v,
                ChatSendRequest {
                    prompt: "Continue".into(),
                    session_id: None,
                    session_key: Some(key.into()),
                    cwd: v.to_path_buf(),
                    model: None,
                    permission_mode: None,
                    agent: AgentKind::Claude,
                    client_turn_id: None,
                    app_depth: 0,
                    ultracode: false,
                    canvases: None,
                    continued_from: Vec::new(),
                    unattended,
                    typed,
                    host_asker,
                    canned: None,
                    worker: false,
                },
                Utc::now(),
                None,
            )
            .unwrap();
            let row = crate::run_queue::get(v, accepted.queue_id)
                .unwrap()
                .unwrap();
            let payload = serde_json::from_value::<ChatTurnPayload>(row.payload).unwrap();
            assert_eq!(
                payload.effort, None,
                "{name}: queued with no effort of its own"
            );
            let requested = resolve_requested_effort(v, key, payload.effort.clone());
            let (decision, _) = turn_cfo(
                v,
                AgentKind::Claude,
                None,
                "Continue",
                turn_origin(
                    payload.person_sent(),
                    payload.host_asker.as_deref(),
                    0,
                    false,
                ),
                false,
                requested,
                Some("xhigh".into()),
                None,
            );
            assert_eq!(
                decision.effort.as_deref(),
                Some("max"),
                "{name}: the chat's own level"
            );
            // A named level is a request: the person's own turn records it as
            // theirs (`You`), another origin's as an agent's (`Agent`); never
            // the CFO's pick or the Fixed level.
            assert!(
                matches!(
                    decision.effort_source,
                    EffortSource::You | EffortSource::Agent
                ),
                "{name}: a requested level, not the CFO's or Fixed ({:?})",
                decision.effort_source
            );
        }
    }

    #[test]
    fn preview_file_scoped_allows_files_inside_roots_and_rejects_outside_or_escaped() {
        let root_dir = tempfile::tempdir().unwrap();
        let outside_dir = tempfile::tempdir().unwrap();
        let inside = root_dir.path().join("notes.md");
        let outside = outside_dir.path().join("secret.txt");
        std::fs::write(&inside, "hello preview").unwrap();
        std::fs::write(&outside, "top secret").unwrap();
        let roots = vec![root_dir.path().canonicalize().unwrap()];

        let ok = preview_file_scoped(&inside.to_string_lossy(), &roots).unwrap();
        assert_eq!(ok.kind, "text");
        assert_eq!(ok.text.as_deref(), Some("hello preview"));

        let err = preview_file_scoped(&outside.to_string_lossy(), &roots).unwrap_err();
        assert!(err.contains("outside"), "{err}");

        assert!(preview_file_scoped("relative/notes.md", &roots).is_err());

        #[cfg(unix)]
        {
            let link = root_dir.path().join("escape.txt");
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            let err = preview_file_scoped(&link.to_string_lossy(), &roots).unwrap_err();
            assert!(err.contains("outside"), "{err}");
        }
    }
}
