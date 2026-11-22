# How context, tools and delegation reach an agent

## Accepted-send preparation (SE6)

After enqueue commits, `chat::send_prep` may prepare read work in at most two
blocking workers, one per session. It has no waiting queue and skips payloads
over 128 KiB. Optional stages stop starting after five seconds, cancellation
or a full-payload/row-identity mismatch; a blocking read keeps its permit
until it actually exits. Stop marks preparation cancelled. Terminal rows and
replaced payloads fail the next checkpoint even through other queue paths.
The reserved job must still be Queued before its worker starts. This prevents
a late registration from missing an already completed running-row Stop.
Exact-row Stop latches cancellation while its stop slot is still registered.

Retrieval and connector relevance warm exact vector/score caches, never an
authoritative prompt, evidence envelope, attachment or approval. Final
admission resolves current scope and candidates and rebuilds authorization.
Speculative ranking sends are metered as standalone Jev usage; identical
concurrent successful requests share their score work and individual deadlines.
Settings resets separate generations and reject late obsolete cache writes.
Compiler profile, derived-state and baseline reads may warm the OS/SQLite
caches. Full compilation remains after admission because its turn ordinal,
resume choice, arrivals and final context budget are not known at send.
Timing is a separate enqueue-span log, not a zero admission-relative mark.
The existing short-follow-up skip and failed-resume fresh fallback remain.
Semantic preparation runs inline on its capacity-owning worker, including
time waiting for the model. Stop and expiry gate model loading/inference,
ranking lock waits and HTTP transport. Warm connector selection first checks
the conversation's existing reuse eligibility without changing it; admission
may adopt warmed scores with their original decision time and unchanged TTL.

Enabling semantic search, including adopting its saved opt-in at boot, starts
one model warm worker. Disable and unload epochs still win; off never loads.
Optional preparation expiry cancels only that request, preserving shared
models and query vectors. A model retained after a cancelled load has an idle
timestamp so ordinary eviction still releases it.

The one current reference for what an agent run in Hollow is given, in what order and size, where each
part is composed, and which tools each provider gets. It describes main after wave 1 of the agent
effectiveness programme (R1–R3, C1–C2, E1–E2 merged, 2026-10-06), plus wave 2's lane K (K1, the host
context compiler; K2, the task-state record and profile revision deltas) and S1, S2 and S3 (the host
retrieval broker, F3: ranked evidence, then query-centred attachment chunks and the `read_source`
reader; F7: one lifecycle policy for recall and retrieval), the wave 2 follow-ups (2026-10-07), and
wave 3's G1 (F6 budget part: the compact capability index and the visual guide delivered when a turn
needs it, 2026-10-07), G2 (one total context budget per provider, the profile digest that keeps
Decisions and Guardrails, and deltas of the stable layers on resumed turns, 2026-10-07), O1 (F8: every
preparation step timed on the run manifest, 2026-10-07), O2 (F8: retrieval and connector selection
prepared at once under one preparation deadline, Jev rankings reused and a failing Jev paused, the
Microsoft 365 refresh only after selection, 2026-10-07) and O3 (F8: Codex's native MCP inventory cached
per binary, config and folder; F11: duplicate-check reports kept per run, a run's Jev calls reconciled
in its manifest view, a manifest for the heartbeat, 2026-10-07), and wave 4's V1 (F9: the
per-provider capability contract, truthful skill copy, the Codex helper probe, 2026-10-07), V2 (F9:
`list_actions` query, filters and paging; each chat turn's relevant skill index with `read_skill`,
2026-10-07), D1 (F10: the two delegation contracts named everywhere, a task packet for each child,
delegations on the parent's manifest view, 2026-10-07) and D2 (F10: a child task's end
reported once without polling, every waiting tool inside its caller's tool timeout, a stopped turn
stopping its child turns, descendant cost rolled up to the parent, 2026-10-07).
Every claim was checked against the code; files are cited, not line numbers. The per-feature contracts are in the agent manual's [reference docs](../AGENTS.md#reference-docs)
([agent integration](reference/agents.md#chat-how-the-headless-agent-integration-works),
[permission modes](reference/agent-tools.md#permission-modes-across-agents-2026-10-02),
[connectors and prioritizer](reference/connectors.md#connectors-library-and-context-prioritizer)). What each wave-1
change proved is in [AGENT_EFFECTIVENESS.md](history/AGENT_EFFECTIVENESS.md). The plan (findings F1–F11) lives in
the Hollow vault's workspace outputs as `agent-effectiveness/README.md`. Gaps the plan still owns are
marked with their wave in the last section.

Update this file in the same change as any layer, bound, entry point or capability it describes.

## Project main conversations (2026-10-30)

Each non-built-in project exposes a stable `s-project-` conversation key from
`project_agents.rs`. `chat_queue` validates its owning project/folder both on
acceptance and admission. `chat::send` adds its fixed main-agent role paragraph
to the brief returned by `chat_append_layers`, for every provider: answer and do
straightforward work directly, consult only relevant project context as needed,
and create workers only for explicit workers/subagents or task-batch requests.
Both main roles share `project_agents::WORKER_GUIDANCE`: five Portal tasks
means five separate local Hollow chats; project lookups and status questions
stay direct. `sessions.start_workers` checks the count and every task before
enqueueing, accepts only attended depth-zero global/project coordinators,
inherits the caller's provider/model/mode, and reports partial failures and
queue state. Workers use normal delegation records, context packets and the
existing run queue. The ordinary three-start limit remains for `start_session`;
an explicit batch is separately bounded at 25, once per run, with retry results.
It adds no classifier call, mandatory
history scan or autonomous wake. Existing project documents, task state, resume,
permission modes and provider adapters remain the source of continuity.
Reopening an unsent main conversation restores the mode chosen for its key
(new-chat default otherwise). After Remove worktree, a main conversation still
on the project's own removed worktree path stays valid, unless another
workspace runs or ran at that path (ambiguous, refused). One moved to another
project is refused and its history left in place.

App guidance distinguishes sidebar projects (local workspaces) from the
legacy task-project notes and legacy `notion-*` folders served by `hollow_tasks`.
Missing `projects/` points recovery to `list_workspaces` / `create_workspace`.
The chat prompt says: "Hollow no longer syncs with Notion; its official
connector (notion-official) is an ordinary connector." (`chat.rs`, since
2026-11-11; Hollow does not hide or deny any of its tools.)

It also says never to run Hollow's binary without a known command (`chat.rs`,
2026-11-11; the prompt has no room for more, F6's budget). The binary answers
`--version` and `--help`, refuses anything else with exit 2, and a second launch
hands off to the running app ([`launch.rs`](reference/backend-module-map.md)), so a
probe like `hollow --version` no longer starts a second app. **Apple's Tk `Wish.app`
crash (2026-11-11 2:10 PM):** an agent probed for Tcl/Tk with `wish 2>&1 </dev/null`;
`/usr/bin/wish` execs `Wish.app`, which macOS killed on launch ("Launch Constraint
Violation", SIGKILL) from that chat's shell, and the crash report names Hollow
because the shell is in Hollow's coalition (responsible pid 15000). No tkinter or matplotlib was
involved, so `MPLBACKEND=Agg` would not have helped. Hollow can't stop a command an
agent types and the prompt budget has no room for a line about it, so this is
documentation only: check Tcl with `tclsh` (`info patchlevel`), never `wish`,
`open -a` or a GUI toolkit's demo.

The [installed-client comparison and benchmark](research/claude-client/technical_specification.md)
separates observed Claude behavior, implemented Hollow changes and future runtime
experiments. The local reducer speedup is not an inference-latency claim.

## The two halves of a run

Every agent run gets two kinds of text:

- **The configured half.** For chats: the appended prompt (`chat::chat_append_layers`, four parts:
  "Visual answers in Hollow", the capability index, the brief, guidance) plus the workspace policy and
  a mode-free permission pointer (`agent::chat_policy` = `WORKSPACE_POLICY` + `mode_contract::chat_policy`;
  plan keeps its own paragraph). Since HB01 (evening 2026-10-16) nothing in a chat's configured half
  depends on the mode outside plan: the mode is the per-turn `<hollow-mode>` line (layer 7a). Automations,
  teammates and other runs keep `agent::workspace_policy` (`WORKSPACE_POLICY` + `mode_contract::policy`).
  The configuration fingerprint (`chat_resume::config_fingerprint`, `v3:` since OB04) hashes agent, model, whether
  the mode is plan (`chat_resume::mode_class`: `plan` or `act`; for Grok its sandbox profile, since a Grok
  session refuses to resume under another), cwd, the attached owned server names, what selects the
  appended prompt's text (the `AttachedCaps` bits, the Canvases choice and the prompt kind:
  `main`, `quick`, `workspace`, `vault` or `bare`), `chat_resume::PROMPT_CONTRACT` and a skills list that
  is always empty. Change any of these and the native session starts fresh once. Each part's own hash
  is stored beside the fingerprint (`stints.config_parts`, `chat_resume::config_parts`, OB03), so a
  config-changed reset names what changed: the `sessionReset` event's `changed` and the manifest's
  `compiler.resetParts` list `chat_resume::PARTS` names (`contract`, `agent`, `model`, `mode`, `cwd`,
  `mcp`, `caps`, `canvases`, `workspace`, `skills`), or `unknown` for a stint stamped before OB03. The
  chat's notice says them in plain words
  (`chatContinue.ts` `configResetClause`, OF03: "The model and the permission mode changed, so Hollow
  started a fresh Claude session…"). `mcp` and `caps` share one phrase, "Hollow's tools for the agent"
  (`mcp` is only Hollow's own servers, not the person's connectors), and `caps` isn't said beside
  `agent`, which always changes it; `workspace` is "the kind of conversation" (the prompt kind, not a
  move); `mode` names neither mode, since the event carries no values. `unknown`, unrecognised or
  absent names keep the generic text ("This conversation's agent settings changed…"). An `unknown` stint's
  parts are filled after its next successful resume (`session_store::stamp_config_parts`). A pending
  reset names against the last closed stint. The parts' JSON holds 16-hex-digit hashes, never the
  folder or model. The servers and caps come from one builder, `chat::turn_servers`, then `for_bare`; a test drives it
  for a Main-agent wake and the person's bypass turn (identical parts). The appended prompt's
  bytes are not hashed: a guidance edit reaches a resumed session (Claude gets `--append-system-prompt`
  on every process; the other adapters get it resent when its stable-layer SHA changes). Bump
  `PROMPT_CONTRACT` in a change that should reset every session (guidance a resumed session would act
  against, not wording). A stint stamped `v1:`/`v2:` resumes and is restamped `v3:` after a successful
  turn when the open stint is this agent's, the last turn that ran on its native session (its usage
  row names that session, so a turn that failed before its resume plan never counts) ran in the same
  mode class (`turns.permission_mode`) and asked for the same model, and `sessions.cwd` is the turn's
  folder (`chat_resume::may_restamp`, `session_store::previous_turn`); otherwise, or on Grok, it starts
  fresh once. So the install still resets, once, a conversation with no earlier turn whose usage row
  names that session, or whose matched turn has no recorded `permission_mode`. The cost: a resumed Claude session whose appended prompt changed rewrites its whole
  prompt-cache prefix on its first call, which for a chat at 300K tokens or more can cost more than a
  reset would, though it keeps the native context (J01 records first-call cacheWrite on post-install
  resumes). Claude (`--permission-mode`), Codex (its sandbox override) and Antigravity (`--sandbox`
  or its bypass flag) take the mode on each process, resumes included, so a flip between acting modes, such as a Main-agent wake in a lower mode (capped at acceptEdits until 2026-11-12) between the person's bypass turns, keeps
  the native session. Before HB01 those flips were 74 of 99 config-changed resets (2026-10-09 to 10-16).
  The composer follows the same classes (`permModes.ts` `modeClass`, 2026-11-04): from the 10-27 permission
  port until then it resumed only an exact mode match and dropped an adopted turn's session, so each turn
  after a Main check-in (acceptEdits) in a bypass chat started fresh and the check-in's question card couldn't be sent.
  Claude Code's own system text is the same in acceptEdits and bypass, but its Bash tool description is
  not, and tools lead the prompt-cache prefix: such a flip keeps the session and its context but still
  rewrites Claude's prompt cache (a $0 probe against a local endpoint, HB01). Library connectors, effort, profile text and the per-turn layers are
  not in the fingerprint. That is deliberate for the profile and the skills (K2): a profile edit reaches a
  resumed session as a delta (below) instead of a restart, and Hollow attaches no skills per run (each
  turn is a new CLI process, which loads its own). The visual guide a visual turn carries (G1) is per-turn
  text, not in the fingerprint, so whether a request looks visual never restarts a session.
- **The per-turn half.** The message: the backend's wrapper (workspace envelope, recall, the clock and
  mode lines, connectors note, retrieved evidence) around the body the context compiler built (`context_compiler.rs`: on a fresh
  native session the conversation bundle, the task-state record and the operator profile; on a resumed
  one the profile's revision delta when the profile changed; then, when the turn needs it, the visual
  guide; then the caller's composed text). `chat::TurnComposer` builds it, for the first attempt and a
  failed resume's retry alike. The connectors note and the evidence are rebuilt and sent on every turn;
  recall, the composer's chat intent, the skill index and the mode line on every turn too, except that an
  exactly resumed one leaves out each its session was given as is (S02, below); the envelope in full on
  a fresh session and, on an exactly resumed one, as an update (G2, below); the bundle, the task state and the profile only when the native session starts fresh; the
  visual guide once per native session, on a turn that looks visual (G1, below).

One total budget bounds both halves of a chat turn (G2, [The budget](#the-budget-and-resumed-turns-g2)).

How the adapters place them (`agent.rs`):

| Provider | Configured half | Per-turn half |
|---|---|---|
| Claude | `--append-system-prompt`: appended prompt, blank line, workspace policy (`isolate_claude`). The policy comes last. | The `-p` prompt. |
| Codex, Antigravity, Grok | No system-prompt flag. `contextual_prompt` puts the workspace policy first, then the appended prompt, then the turn's prompt, as one prompt. | Same prompt. |

So for Codex, Antigravity and Grok the configured half is part of the prompt. Since G2 an exactly
resumed session that already holds it gets a one-line reference instead (`agent::CONFIGURED_HELD`), and
since V1 only when the command really resumes (`agent::resumed_id`: a safe id on an interactive spec,
which is when every builder passes it); the workspace policy still goes every turn. Claude's appended prompt is its system prompt, which every
process needs, so it goes every turn.

## Entry points at a glance

| Entry point | Composed where | Working folder | Configured half | Message blocks | Owned MCP | Manifest |
|---|---|---|---|---|---|---|
| Vault chat (Conversation history, Library → Conversations, `components/ConversationHistory.tsx`) | `ChatBody.tsx` `send` (page context, intent, request, images), then `chat::execute_turn` and `context_compiler` | vault root (or a chosen folder) | visual answers, capability index, environment brief + preview-pane note, guidance | conversation, task state, operator (fresh only), or the operator update (resumed, on a change); page context, intent, request, images | Claude, Codex | yes |
| Chats (`quick-chats` workspace) | same | `workspaces/quick-chats` | visual answers, capability index, quick-chat brief + preview-pane note, guidance | conversation, task state, operator (or update), intent, request, images | Claude, Codex | yes |
| COO (`main` workspace, M1–M6; the Main agent until L6, 11-10) | same | `workspaces/main` | visual answers, capability index, `main_agent_prompt` (delegate by default to a project's lead or a subagent, notepad, standing orders, `SCHEDULE_NOTE`) + preview-pane note, guidance; a wake turn (M2) carries `<hollow-child-finished>` | same as a chat in Chats; envelope adds `STANDING_ORDERS.md` | Any agent (L6): the standing-orders deny rule (`chat::coo_denied_tools`) is added only for an agent that holds deny rules (`agent::holds_deny`); the others get it from the prompt | yes |
| Workspace chat | same | `workspaces/<id>` | visual answers, capability index, one-paragraph workspace brief, guidance | same as a chat in Chats | Claude, Codex | yes |
| Build-workspace chat | same | the git worktree | as workspace chat, plus services guidance (not in plan) | same | Claude, Codex, plus `hollow_services` | yes |
| Child started by an agent (`hollow_app` `start_session` / `send_to_session`: a durable child task) | `app_sessions::start` / `send` → `child_prompt` → `queue_turn` → `commands::send_chat`, then `context_compiler` (entry `appSession`); no frontend | the target workspace's folder or the vault | as the chat for that folder, but a subagent (started with a parent, or `fork_kind = 'subagent'`) gets the lean profile (COO rebuild L10, `chat::lean_subagent`): `LEAN_VISUAL` for the visual layer (no canvases, no guide in its message), `LEAN_INDEX` for the index, and the guidance without the question-card and next-step blocks; the brief and the rest stay | the task packet (first, when it carries anything), conversation and task state (fresh only; none on a start), operator (fresh only) or update (resumed), the task text | as a chat, at depth + 1 | yes, and the parent's view lists it |
| Automation (workflow) | `workflows::prepare_spawn` | the workflow's `cwd`, else the vault | none appended; the workspace policy only | boundary block, task and app guidance, connectors note, workspace envelope, recall, operator, evidence, request | Claude, Codex (no questions, no computer, no vault search) | yes |
| Heartbeat | `heartbeat::prepare_beat` | vault | none; the workspace policy only | scratch note (tool pass), recall, operator, already-proposed, beat prompt (+ snapshot for Codex) | none | no |
| Teammate (`hollow_team` `ask_agent`: a bounded consultation) | `team_mcp` (inside the parent's server) | `cwd` argument, else the parent's cwd | none; the workspace policy only | task packet, role preamble, task, working-folder directive, scratch note | none | no; the parent's view lists it |

For the chat rows, "message blocks" are what the body holds: the context compiler adds the conversation,
task-state and operator blocks on a fresh native session (the operator update on a resumed one), the
visual guide on a turn that needs it (every chat row, G1), the caller sends the rest; the backend wraps the body in the workspace envelope, recall, connectors note and
evidence (layers 5–8 below).

## UI chats: what the agent reads, in order

A first turn of a Claude chat, top to bottom. Codex, Antigravity and Grok read the workspace policy first
instead of fifth.

| # | Layer | Composed in | Bound | Notes |
|---|---|---|---|---|
| 1 | "Visual answers in Hollow" | `visual_prompt::visual_prompt` | about 2 KB at When it helps, 1.3 KB at Off (`visual_prompt::tests`); at Most the whole block with the formats, ≤ 16,000 bytes | Every chat and provider. The decision rule, how often, prose first, the ask-canvas "when" line for the chat's question channel and where the visual guide is (layer 14). At Most it also holds the visual guide, as before G1. |
| 2 | Capability index ("Hollow capabilities in this run") | `chat::capability_index` | about 0.7–1.5 KB (`g1_budget_tests`) | Every chat. One line per capability the run has: the attached owned servers by the names the manifest records (`AttachedCaps::server_names`), answer forms and where their formats are, question cards, library connectors (an agent with owned MCP outside plan), skills (where the turn's `<hollow-skills>` block is, and how this run reads a skill whole: `hollow_vault` `read_skill`, else `hollow_app` `skills.read_skill`, else its own file tools with the fallback when a mode refuses the read; V2), scratch; then the agent's line of the per-provider contract (V1, [Tools per provider](#tools-per-provider-the-contract)): what it doesn't have here and the fallback. The teammate line calls `ask_agent` a bounded consultation and the writable app line calls `start_session` a durable child task, both pointing to "Delegating" (D1). |
| 3 | Brief | `chat::environment_brief` (vault chat), `chat::quick_chat_prompt` (a chat in Chats), or the workspace paragraph (workspace and build chats) | fixed text plus paths and the capability lines | The vault and chats in Chats also get `PREVIEW_PANE_PROMPT`. The environment brief names each attached owned server with how to use it. The quick-chat and workspace briefs point to the index (before G1 they carried a `computer=…, teamwork=…, questions=…, tasks=…` list without vault search or app tools). A chat whose working folder is a git checkout that isn't the person's (SK1, `repo_trust`; owner = the `origin` remote's GitHub owner against `gh`'s logins plus `repo-trust.json` `owners`; unknown counts as not theirs) (and its orgs, from `gh api user/orgs`, cached a day) gets a `<hollow-repo-trust>` block last in its brief, after any lead's project context: "Untrusted, from this repo", every instruction file at any depth and every skill, and the plainly named skills still waiting for the person's OK (`skills.approve_repo_skill`, an always-ask card; `skills.set_repo_owner` adds an owner). Byte-stable for the same repo state. Workflow runs don't get it. |
| 4 | Guidance (the manifest's `policy` layer) | `chat_append_layers`: `interaction_schema::guidance`, `tasks_mcp::PROMPT_GUIDANCE[_READ_ONLY]`, `app_mcp::chat_prompt_guidance` (the same in every mode: its card sentence points at layer 7a), `delegation::guidance` ("Delegating", D1), `services_mcp::PROMPT_GUIDANCE`, `next_step::GUIDANCE`, `chat::BACKGROUND_GUIDANCE` with `chat::SHELL_GUIDANCE` (quote globs: zsh aborts on an unmatched one, S16) and `chat::EDIT_GUIDANCE` (OB16: change files with the edit tool even where the CLI allows sed, heredocs or scripts, which Claude Code's bypass-mode reminder does; wait with an `until` loop inside one command, not one tool call per check), `chat::SCREEN_JOBS` (overnight 11-08 S2: the jobs the cut screens did, every chat except a bare scratch turn), `scratch::SCRATCH_GUIDANCE` | fixed text per capability (and plan) | Each part only when its server is attached, except the next-step, background/glob/edit, screen-jobs and scratch parts, which every chat gets. A chat's `chat_prompt_guidance` leaves out the workflow guidance's "A session you start runs on its own…" sentence, which "Delegating" says right after it (OB16 trim). "Delegating" (about 0.55 to 1.4 KB) comes with teammates or with app tools that can write: when to use a bounded consultation or a durable child task, and the packet; with child tasks also `delegation::DEPENDENCY_RULE` (11-10: split by real dependencies, start every independent part at once, past a cap the rest queue). |
| 5 | Workspace policy and permission pointer | `agent::chat_policy` → `mode_contract::chat_policy` (`run_policy`, because the chat's `RunExtras::mode_per_turn` is set) | 716 bytes (`MODE_POINTER`, 172 of them: the latest `<hollow-mode>` line applies, S02); plan its own paragraph | Added by the adapter. Not a manifest layer. Other runs get `agent::workspace_policy` (~640–880 bytes; default and accept edits are the long ones since OB02). |
| 6 | `<hollow-workspace>` envelope | `workspaces::context` | 128,000 bytes after escaping. Documents (`WORKSPACE.md`, `PLAN.md`, `SOURCES.md`, `AGENTS.md`, `NOTION.md`) share 48,000 bytes, the largest cut first. A cut WORKSPACE.md keeps everything outside `## Current context` (frontmatter, Outcome, Decisions) and that section's newest dated entries, with one "[Hollow: N older entries in Current context left out; …]" line, and always keeps context_compact's archive pointer line (`context_compact::POINTER`, OF24); a cut PLAN.md keeps the pointer line an earlier Archive older entries wrote (`context_compact::PLAN_POINTER`, whole-line shape only, overnight 10 A9; since V5a, 2026-11-15, written by `workspaces.archive_shared_docs` and the workspace menu's Archive older entries…) and every heading (a `#` line inside a ``` or ~~~ fence is not one; a fence closes only on a run of its own character at least as long with nothing after it, and an opener nothing closes is text, `workspaces::heading_lines`) and each section's first lines, with a "left out" line per section (`workspaces::brief_selection`, `plan_selection`, HB02). Without dated entries or headings (or when a plan's headings alone don't fit) a document is cut to its prefix, as before; a brief whose rest alone is over its share, or a document the 128 KB trim halves, is cut from its end and its note says "the first bytes". Attached sources: ≤ 100 listed; ≤ 24,000 bytes of text in total, as the chunks (≤ 40 lines, ≤ 3,000 bytes; one longer line cut to the window around its best match) that best match the request across files and pass the relevance floor (OF06; when none does, the 3 densest within 8,000 bytes), with no backfill unless the request asks for the whole document or all attached text fits; a one-chunk file goes whole; each file with a coverage note. | Workspace and build chats. Optional `goalAlignment`, added only when it fits. A build workspace adds its worktree note (`build_note`). A workspace with a connected repository or a card adds `projectCard` (PC1, `project_card.rs`): the architecture card's checked status (fresh, stale, invalid, missing) and its text, or, when not fresh, the versioned prompt and the frontmatter to rewrite it with; a card that links a design brief (`design:`, SD1) adds its status to the first line and, when fresh, its text (at most 2 KB); never in the slim envelope. Chats get the fixed `QUICK_CHAT_ENVELOPE`; vault chats get none. |
| 7 | `<trusted-recall>` | `vault_learning::recall_with_scope`, called through `TurnRequest::recall_cued_by` | ≤ 5 items, ≤ 3,000 characters | Approved learnings only, under the lifecycle policy ([Lifecycle](#lifecycle-s3)): a superseded one only for a historical request, labelled. Global learnings rank by the stems they share with the request (stop words dropped after stemming), then recency; when semantic search is on and the turn's query embeds (the same text retrieval embeds, `TurnRequest::meaning_query`; an empty host subject embeds nothing), a global learning whose statement (its indexed chunks before the first heading, not the "Why Hollow proposed this" or "Source excerpt" sections: `EmbedIndex::statement_cosine`) reaches cosine 0.6 (`MEANING_FLOOR`, which is retrieval's `SEMANTIC_MIN`) also scores 5–15 by meaning (OF26): a shared stem (10) beats a cosine under about 0.78, two always win. Recall waits 100 ms at most for the vector and every global learning's cosine, read under one index lock (`RECALL_MEANING_WAIT`; 25 ms after three late recalls in a row, `RecallPace`, J4; all on a single `hollow-recall-embed` thread; the turn's own thread takes no lock that can block, and while one such thread still runs the next recall doesn't wait), then ranks by stems while the work runs on. Since J3a (11-15) a chat turn's recall is a prefetch: admission starts the turn's inputs on a `hollow-recall-prefetch` thread before it takes the lease (`recall_prefetch`, `chat_queue::prefetch_inputs`), and the turn takes them if ready within 25 ms (`RECALL_PREFETCH_CUTOFF`; no wait at all during a late stretch; never past 100 ms after admission began), else ranks by stems itself (`TurnRequest::recall_past_cutoff`, `timedOut`) and drops the late answer; each pickup logs `[recall] own|agent turn: prefetch ready|late|failed`. A turn that retrieves nothing (bare, or a skipped basis) doesn't embed. `embed_model` keeps the last query vector and reads it again once it holds the model, so retrieval reuses it instead of embedding again, even behind a timed-out recall. Without a vector the ranking is the stem one. The manifest's `recallMeaning` says which: `used`, `skipped`, `timedOut` or `noVector` (absent when semantic search is off). |
| 7a | `<hollow-clock>`, then `<hollow-mode>` | `chat::clock_line`; `mode_contract::turn_line` (HB01) | clock one line; mode about 0.76–1.0 KB outside plan (`tests/fixtures/prompts/mode-wording.json` `turnLine`; accept edits the longest since OB02, 996 bytes) | The clock every chat turn; the mode line every turn of a fresh session, and on an exactly resumed one only when its session doesn't hold this exact line (S02: a changed mode, such as a permission flip, is always sent; the latest line in the conversation applies). Never cut (`Tier::Fixed`). A plan turn's line repeats plan's paragraph, which its configured half also keeps. The mode line is `mode_contract::policy`, then (default and accept edits, for an agent whose reads stop at its folders: `capability_contract::reads_only_its_roots`, Claude today; OB02) `mode_contract::Reads`' sentence "Reads stop at the working directory and scratch folder" (plus a build workspace's vault folder by path) and `read_session`/`read_source` when attached, then the Hollow app card sentence (`app_cards`) when the app tools can change things, then computer control's tier-2 sentence when it is attached, and ends "This replaces any mode an earlier turn named." The working directory and scratch folder are named, not spelled out: the brief already gives their paths, and at about 90 bytes each in production the paths pushed the heavy G2 turn's workspace layer under its want. Manifest layers `clock` and `permissionMode`. |
| 7b | `<hollow-context>` (repo features R2b) | `chat::turn_context_note` → `context_gauge::start_note` / `start_line` | one line, ≤ 220 bytes (about 150) | Between the clock and the mode line, only on an exactly resumed session (`Session::Resume`; never a retry or a fresh one) of a turn that isn't plan mode, bare or unattended (`chat::gauge_quiet`), and only when the conversation's stored figure (the `contextSteps` row's `tokens`, written by the previous turn's end) passed a growth step it wasn't told: "This conversation's context is at 412K of 1M. On a long job, finish the step and hand over (sessions.handover)." (the window clause only when known; no subagent clause, since only the main thread reads a turn's message). The step is then stored as told, so neither this channel nor the in-turn note repeats it ("Context gauge and growth steps" below). Never cut (`Tier::Fixed`, a `contextNote` budget ask only when present). Manifest layer `contextNote`; the composer's What your last message used shows "Context note". Off the always-on path: absent below 300K and between steps. |
| 8 | `<hollow-connectors>` note | `connectors::prompt_note` | one entry per attached connector (the library holds ≤ 50) | Only when a connector is attached. Carries the mode's preapproval sentence and the external confirmation. |
| 8a | `<hollow-skills>` index | `skill_index::for_turn` through `chat::turn_skills` (V2) | ≤ 5 lines (description ≤ 160 characters), plus the selected skills' `SKILL.md` text inline while it fits 8,192 bytes in total; the pilot's selected skill alone makes about 0.9 KB | Only when a skill fits the request or one was selected ([Skills](#skills-v2)). Every provider; never cut (a `skillIndex` budget layer, fixed tier). The manifest layer `skillIndex`. |
| 9 | `<retrieved-evidence>` | `directory_connections::prioritized_context` over `retrieval_broker` | ≤ 8 passages (the one evidence budget, after reranking a pool of ≤ 24), ≤ 900 characters each, plus coverage notices | Untrusted evidence; cite paths and lines. A file's third passage ranks behind every other file's first two. |
| 10 | `<conversation-context>` | `context_compiler::conversation_bundle` from `session_store::conversation_entries` | last 24 user/assistant entries, last 6,000 characters of each, JSON body ≤ 32,000 characters (Unicode scalar values) | Only on a fresh native session (new, reset, retried). |
| 11 | `<task-state>` | `task_state::render` from the record `task_state::refresh` folds (K2), merged after the continued conversations' records `task_state::peek` reads (OB14) | 8,000 characters, a hard ceiling after escaping (past the drops below, item texts are cut shorter, then items go); 12 constraints (the first 4 always kept), 6 corrections, 16 steps, 4 open questions, 12 files, 280 characters an item | Only on a fresh native session with a non-empty record, or once on a resumed one that re-grounds the agent (repo features R1, `stable_layers::reground`): the next resumed turn after a turn whose stream reported a `compact_boundary` (any agent; `Baseline::compacted_turn`, cleared by the next turn that finishes, so a stopped re-grounding turn sends it again; a compacting turn stopped before any finished turn recorded its session, such as a first turn, leaves a baseline that names the announced id and holds nothing else, `stable_layers::compacted_mark`), and the refresh of every agent but Claude (the resumed turn after 8 resumed turns of deltas; Codex, Antigravity, Grok: Hollow can't see them compact). That block is the conversation's own record (no continued ones) with one line inside it first (`context_compiler::reground_note`: "Your session compacted its history during turn N…" or "Hollow resends this record after every 8 resumed turns…"), so it can run a little past the 8,000 characters; mid-turn the agent reads its own record, the running turn included, with `hollow_app` `sessions.task_state`, and its next `hollow_app` result after a compaction its stream reported points it there (repo features R2a; "Context gauge and growth steps" below). A header (the user's words; or, when another agent asked a cited turn through `hollow_app`, who asked: those items carry `"by":"agent"`, and a rule the conversation's own task packet's sender added carries `"by":"parent"`, RF-797-F3), then one JSON object: `goal` (the first request that isn't a Main agent wake or check-in, `main_agent::is_wake_message`, which is the goal only when every turn is one; a step retry's or friction fix's `⟲ Hollow:` opening is a request), `constraints` (never a wake's, nor a request Hollow wrote that the person sent unchanged: `chat_send`'s `canned` argument, `canvas`/`canvasRefresh`/`explain`, recorded as `asked_by` `hollow:canvas`…, which also makes the send untyped so it teaches nothing; nor fenced blocks, `>` lines or quoted spans over 40 characters, `task_state::settling_sentences`; schema 4, overnight 9 A1; a sentence that settles something without such a span keeps each one that is a single phrase and a `code` span or settles nothing itself, a path, a command or a commit trailer, schema 5, RF-797-F2; a rule's dedupe key is its clipped stored text), `corrections`, `steps` (`done`/`failed`/`unconfirmed`; a check run takes its receipt's outcome, read from its full output (the step spine's `checkOutcome`): `failed` when its output shows its runner's failure text, even when piped into `tail`, behind `|| true` or exiting 0, `done` when it passed, `unconfirmed` when masked and nothing confirms it, OF16; each target through `claude_runner::redact`, as `read_session` gives tool inputs; a shell command `run_effects::classify_shell` calls read-only, such as `grep`, `cat` or `git status`, is no step, while a test, build or lint run and an unknown command are), `open` (a question card's only while the card is pending or stale; an answered one is left out), `artifacts` (path, change, History's after-hash), each with its `turn` (and `run`), then `transcript` (the key, the earlier turns' count and how to read them: `hollow_app` `read_session` with `lastTurns` = the stored turns, this one included, so it reaches turn 1, capped at `read_session`'s maximum of 100 with a note that older turns are covered only by the record; or "can't be read from here" without app tools). `<` escaped. |
| 12 | `<operator-context>` | `context_compiler::operator_block` from `operator::compile`, then `Profile::add_layers` with `context_layers::sections` | 6,000 bytes per `me/` file, 16,000 in total, read in the order Identity, Communication, Anti-patterns, Guardrails, Decisions; on overflow Decisions and Guardrails are kept whole and prose gives way, Identity first (G2); less prose when the turn's budget needs it. The four layer sections: home 4,000 bytes (`HOME_CAP`, lead line included), pinned memories 1,500 (`PINNED_CAP`, PX3), the generated personal index 2,000 (`INDEX_CAP`, PX2), org 2,000 (`ORG_CAP`), each cut to whole lines (home and org with `CUT_MARK`, the index with a closing line that counts the files left out), never cut by the turn budget | Only on a fresh native session, while About you is on (the backend's persisted switch, `operator::Switch`, the source of truth: every turn reads it from disk, the window adopts it at launch and sends only an explicit change) and the digest has text. Five blocks ([CONTEXT_LAYERS.md](CONTEXT_LAYERS.md)): the personal `me/` digest; `## home/<slug>.md` (this machine's home file from the personal clone, while the personal root and the machine slug are set and the file has a body); `## personal/pinned.md` (PX3: memories marked `pinned: true`, whole, within 1,500 bytes; one over the budget is named, not loaded); `## personal/INDEX.md` (generated, PX2: one line per memory and learning in the personal clone, from its `description:` or a learning's `title:`, grouped by type, while the personal root is set and holds a file); `## org/INDEX.md` (the org clone's `INDEX.md`, or a list of its top-level folders and Markdown files while it has none, while the org root is set). A layer section alone (no `me/` text) still sends the block. Each section's changed lines reach a resumed session through layer 13. |
| 13 | `<operator-update>` | `context_compiler::profile_delta` (K2) | 2,000 characters, 300 a line, then the files to read | Only on a resumed native session, once, when the lines of a `me/` file's digest section (as layer 12 would send it) changed since the session's stored baseline (the sections it was sent); first in the body. A line is `-` only when it is gone from the file itself, never when the digest's caps merely pushed it out. |
| 14 | `<hollow-visual-guide>` | `chat::TurnGuide` with `visual_prompt::visual_guide` (G1) | about 8 KB (none at Off: delivery `off`, no file; boards removed 2026-10-07) | Not at Most (layer 1 has it). When the request looks visual (`visual_prompt::visual_cue`: Hollow's cue words, four or more numbers, a named data file; never a Main agent check-in, `chat_queue::turn_visual_cue`, whose own guidance says "options") or the guide file couldn't be written, once per native session (`derived_state` `visualGuide`: a resumed session given this guide isn't sent it again; since V1 recorded only once the agent spawned and printed, so a turn stopped before spawn or failing to spawn leaves it unsent). Every turn at When it helps also writes it to `$HOLLOW_SCRATCH_DIR/hollow-visual-guide.md`, which layers 1 and 2 name (none at Off). Not fingerprinted. |
| 15 | `<page-context>` | `ChatBody.tsx` (`ctxPart`) from `store/runtime.ts` `pageContext` | a few lines | Vault chat only, while its toggle is on: the Chat tab's agent, session, cwd, model and mode. |
| 16 | `<hollow-chat-intent>` | `ChatBody.tsx` (`intentPart`) | 327 bytes, fixed | Every send from the composer. |
| 17 | The request | the user's text | | |
| 18 | `<attached-images>` | `ChatBody.tsx` (`imgPart`) | one path per image | The agent opens them with its own tools. |

Layers 6–9 (with 7a and 8a) are the backend's wrapper (`execute_turn`'s `wrap`). Layers 10–14 are the context compiler's
(`context_compiler::compile_turn`, then `TurnGuide::apply`): 10–12 on a fresh session, 13 on a resumed
one, 14 when the turn needs it. Since D2 the compiler also puts a `<hollow-child-finished>` note after
layer 13's place and before layer 10 on any session, when child task turns this conversation started have
ended since ([Completion, stop and cost](#completion-stop-and-cost-d2-f10)), followed (2026-10-07) by a
`<hollow-workspace-shared>` note on every turn in a workspace's own folder (since 2026-10-20, whether or not another
conversation is live there; not on an exclusive run's turn; a build worktree's turn in a workspace with lanes gets it too since F6 part B, its log named by absolute path in the workspace folder, `shared_workspace_note_in`; `context_compiler::shared_workspace_note(session_key)`, its own
`workspaceShared` ask): append to your own `log/<session key>.md` (V5b, 2026-11-15; one writer per file, `shared_docs::chat_log_path`; the workspace system prompt says the same and WORKSPACE.md/PLAN.md take only workspace-wide changes; the Main agent keeps PLAN.md for its follow-ups, so its note names no log (since T3a its prompt says not to list started chats there and its wake message no longer says "log it in PLAN.md"); the workspace system prompt carries the chat's real key), re-read a shared file such as WORKSPACE.md, PLAN.md or SOURCES.md right before editing it, change
only your part with the edit tool, never write it back from an earlier copy or with a shell redirect. Layers 15–18 are the composed
message the frontend sends with `chat_send`.

**Measured sizes** (G1, `tests/fixtures/prompts/context-budget.json`, accept edits, When it helps; Claude
and Codex with question cards, task, app and vault tools, Antigravity and Grok with the fenced question
block). Hollow's configured half per turn, before G1 → after: a vault chat 23,790 → 12,345 bytes (Claude,
Codex) and 20,496 → 8,639 (Antigravity, Grok); a workspace chat 20,204 → 8,735 and 17,774 → 5,892. A
visual turn adds the 13.4 KB guide block on its session's first visual turn (25,719 and 22,130 that turn),
then nothing. The probe fixture (a workspace chat with no tools): Claude's append 17,017 → 5,143 bytes,
Codex's preamble 17,019 → 5,145, plus the 13,324-byte block its visual request carries
(`tests/fixtures/prompts/canvas-real-reply.json`). Claude reads the configured half as a cached system
prompt in every process; Codex, Antigravity and Grok read it in the prompt of every turn that isn't an
exactly resumed one holding it (G2, below). Real provider tokens and cache hits are not measured yet.
These are G1's figures. Since V1 each index ends with its agent's contract line, which adds 110
(Claude), 127 (Codex), 504 (Antigravity) and 497 (Grok) bytes to the configured half (the regenerated
`context-budget.json`: a vault chat 12,455 / 12,472 / 9,143 / 9,136 bytes). V2's skills line, and the
`read_skill` sentence in the vault brief Claude and Codex get, add 139, 139, 40 and 40 more (12,594 /
12,611 / 9,183 / 9,176). D1's "Delegating" guidance and the contract names in the teammate and app
lines add 557 more for Claude and Codex, and D2's wait and stop wording 128 more (13,279 / 13,296;
Antigravity and Grok unchanged). G2's figures below are from before V1: a turn that carries the
configured half is larger by the same amount (the regenerated `turn-budget.json`). A turn with a skill index carries
that block too (layer 8a).

### The budget and resumed turns (G2)

**One total per turn** (`context_budget.rs`): 128,000 bytes of Hollow's text for every provider. Claude is
charged its appended system prompt on every turn; Codex, Antigravity and Grok what their prompt carries.
`chat::TurnComposer` allocates by priority and the manifest records each layer's ask and grant
(`compiler.budget`):

| Priority | Layers | When over |
|---|---|---|
| never cut | the configured half, the connectors note, the request (the caller's composed text), the task state, recall, the profile's Decisions and Guardrails, the profile delta | the turn still goes; the record says `over` |
| evidence | the retrieved evidence whole; the workspace envelope and the conversation bundle share what is left | the smaller whole, the larger cut to an equal share: the envelope rebuilt within it (`workspaces::context_for_within`: excerpts, then the source list, then the largest document halved), the bundle dropping its oldest entries |
| profile prose | Identity, Communication, Anti-patterns | cut to what evidence left (`operator::compile_within`) |
| guidance | the visual guide block (layer 14) | left in its scratch file (`onDemand`); a guide with no file is never cut |

Measured (`tests/fixtures/prompts/turn-budget.json`, `chat_g2_budget_tests`): a heavy fresh turn (70 KB
plan, 63 attached sources, 30 turns of history, a 17 KB profile, a visual request) went from 148,374–150,329
bytes to 127,883–127,920, with the guide in its file and the prose cut.

Per layer (OB15, `tests/fixtures/prompts/envelope.json`, `envelope_fixture_is_current`): five real-shaped turns
per provider (the day's first COO check-in, a new native session on a 51 KB sectioned brief; a later check-in
resuming it; a resumed composer turn; an app-session packet child in a 75 KB workspace, which gets the slim
envelope (S01); a config-changed reset), each
layer's bytes as the run's manifest records them.
`node scripts/agent-eval/envelope-diff.mjs` prints a change's per-layer delta against `origin/main`.

A fresh turn whose budget cut the profile prose stores that prose room with the session's profile baseline;
its resumed turns compare the profile within the same room, so prose the budget withheld is never reported
as an `<operator-update>` change (only a real `me/` edit is).

**Resumed turns** (`stable_layers.rs`): once the agent took the prompt, the conversation's `derived_state`
row `stableLayers` records what its native session holds: the native id this turn's stream announced, the
appended prompt's SHA-256 when the prompt carried it, the envelope's documents, each attached source's
entry and the excerpts given. "Took the prompt" means a finished reply (a success, or denied actions only)
and an id the agent announced in this turn, not the resumed id Hollow passed in; a delta is recorded only
under the id it was composed against (`chat::stable_layers_holder`). A failed, stopped or silent turn
leaves the previous baseline (less the per-turn layers it changed, below), so the next turn resends what it
carried. A turn that resumes exactly that native id, within 8 resumed turns of the last full send, gets:

- the envelope as an update (`DELTA_PREAMBLE`): an unchanged document as a one-line marker, a changed one
  as its `+`/`-` lines (or whole when that isn't smaller), excerpts the session holds named by range in
  `givenEarlier` with only new ones in full, sources with no excerpt for this request counted in
  `unchangedSourceFiles`; an unchanged quick-chat envelope is left out;
- for Codex, Antigravity and Grok, the one-line reference instead of the appended prompt;
- none of the per-turn layers its session holds as is (S02, `stable_layers::TURN_LAYERS`): approved
  recall, the composer's `<hollow-chat-intent>`, the `<hollow-skills>` index and the `<hollow-mode>` line.
  The baseline keeps each one's SHA-256 as the latest turn carried it (`layers`); one whose text changed
  is sent, so a permission flip's new mode line always goes, and an empty one is dropped, so it is sent
  when it comes back. A layer left out counts toward the 8-turn refresh like a delta. These used to go on
  every turn, and every copy stayed in the session's history, cache-read on every later call: on resumed
  turns recall matched the previous turn's 41% of the time and the chat intent 95%, and the mode line is
  663–929 bytes. The agent keeps the earlier copy in its history, and the appended prompt says the latest
  mode line applies (`MODE_POINTER`; the skills guidance says "the latest `<hollow-skills>` block", which
  is left out only when its picks are identical). A resumed turn with no fitting skill, whose session
  holds an earlier block (`stable_layers::holds_skills`: the baseline's native id and a recorded
  `skillIndex`, held or due for the refresh), sends `<hollow-skills></hollow-skills>` (`skill_index::NONE_FIT`,
  33 B) so the stale picks stop being the latest; it is recorded like any layer, so later empty turns
  leave it out, and a new session never gets it (overnight A15). An unsettled turn (`unsettled`) or a
  compaction (`forget_layers`) keeps a `skillIndex` entry with no SHA-256, so the block is still due
  after it. Two cases unsettle the record (S02 review):
  - a turn that resumes the baseline's session may leave its message there whatever ends it (a stop, a
    stall, an API error, a failover whose spawn fails, Hollow quitting), so before the first spawn
    (`stable_layers::unsettle`, against the resumed id) the baseline keeps only the layers that turn
    carries identically, and once the agent ran it does so again with what the run knows (the announced
    id, a compaction, a fresh retry's prompt); the envelope, excerpts and refresh count stay as they were.
    A finished turn then writes its own record over it; a turn that never ran only costs a resend. A
    bypass turn stopped mid-way, then an accept-edits turn: the accept-edits line is sent again;
  - Claude's own compaction (the stream's `system` `compact_boundary`, `ParseState::compacted`, carried
    across a quota failover's attempts) may summarize a held layer away, so that turn records none and the
    next turn sends them all. The compacting turn itself may run without the mode line until then. The
    envelope's baseline and refresh count are left as they are: a compaction can summarize workspace
    documents away too, and for those the 8-turn refresh is still the only bound (a known limitation;
    Hollow sees no compaction for Codex, Antigravity or Grok either). The compaction also marks the
    baseline with its turn (`compacted_turn`, repo features R1), so the next resumed turn carries the
    `<task-state>` record again, once (layer 11).

Everything goes in full on a new, reset or retried session, for another native id, and on the resumed turn
after every 8 resumed turns of deltas (Hollow can't see a provider's own compaction). The envelope also goes in full when its preamble
changed or the budget cut it; the per-turn layers are decided on their own, so those two leave them as
above. Measured: a resumed workspace turn on the same sources 41,414 → 10,063 bytes (Claude),
41,416 → 2,361 (Codex), 38,648 → 2,345 (Antigravity, Grok); a resumed vault chat 12,419 → 1,105 (Codex),
8,713 → 1,051 (Antigravity, Grok); a resumed turn on a new topic still carries its new excerpts. None of
this is fingerprinted.

**What differs by surface.**

| | Vault chat | Chat in Chats | Workspace chat | Build-workspace chat |
|---|---|---|---|---|
| Lease | full vault | scoped (`chat::run_scope`) | scoped | scoped |
| Retrieval scope (`retrieval_broker::resolve_scope`) | all connected projects | all | its project when the workspace id is a connected project's (`workspace`); a project's workspace (`local-…`) whose project left the index keeps that id, so no project evidence (`workspace-disconnected`); else all | the project its worktree was created from (`build-workspace`), whatever the worktree is called; when no connected project holds that repository (its project left the index), the workspace's id, so no project evidence (`workspace-disconnected`) |
| Services, extra dirs, build env | no | no | no | `hollow_services` (not in plan), the workspace folder as an extra dir, `CARGO_TARGET_DIR`/`PORT`/`PW_PORT` and `hollow-slot` (`build_workspace_env`) |
| Toolchain caps and `hollow-slot` (OF07) | not in plan | not in plan | not in plan | every mode |
| Own `PORT`/`PW_PORT` (F6 part A) | leased per turn, not in plan | leased per turn, not in plan | leased per turn, not in plan | the worktree's stable one |

**Build slots in every chat (OF07).** Every chat turn that can take a slot (`build_slots::can_take_slot`:
every mode but plan, and for Antigravity only bypass, since its sandboxed accept-edits runs no commands)
runs with `hollow-slot` first on PATH, `HOLLOW_BUILD_SLOT_DIR` and the toolchain caps
(`build_slots::CAPS`: `CARGO_BUILD_JOBS=8`, `VITEST_MAX_*=4`, vitest minimums 1), and its capability index
carries the 32-byte line `- Big jobs: \`hollow-slot <cmd>\`` (the simple-text and G2 budgets leave no room for
more; Codex's simple-text turn is 1 byte under its bar). The rest reaches the agent from the tool: every
build call's first stderr line ("got a build slot; running it now …") and its usage name the dev-server form,
`hollow-slot --dev-server <cmd>` (one machine-wide, with `PORT`), so a dev server isn't run in a build slot;
a build workspace's note says it too. The index line follows the same rule and
is dropped when `hollow-slot` couldn't be written. The locks live in `/tmp/hollow-build-slots-<uid>`, which
Codex's `workspace-write` and Grok's `read-only` (its `default`) and `workspace` profiles can write.
Environment only: no permission changes. Chat turns only: workflow steps, heartbeat passes and `ask_agent`
delegates aren't given it yet. `build.slots_status` (read) lists the two slots, the cargo lock (one Rust build at a time: a cargo, rustc
or tauri command holds it beside its slot, and a second one waits, saying so), `cargoLane1` (a runner's second
Rust lane, free on this Mac unless `HOLLOW_SLOT_RUST_LANES=2` is exported) and the dev-server lock with
their holders. See [WORKSPACES.md](WORKSPACES.md) (Build settings).

`HOLLOW_THIS_MAC` points to the generated `this-mac.md` in the slot tool's
folder on the machine running the session. Remote preparation installs its own
note with runner environment paths and explicitly unknown hardware, and the
supervisor sets that path rather than forwarding this Mac's app-data path.
The note is shared by every provider. It reports the machine, runner, folders and
current caps, refreshed only when content changes. A turn gets the variable only for a note
the last generation wrote or confirmed, never a stale file it couldn't remove. `this_mac.rs` owns the
rendering; no provider's global instruction file is edited. The capability
index's budgeted text is unchanged. No provider is told to read the note yet:
that pointer lands when Alex moves his own machine notes into it.

**Remote runs (2026-11-01) and smart routing (2026-11-03).** The rule for agents is "use `hollow-slot`; it
routes": with a remote runner set up for this Mac (an SSH host in `~/.config/hollow/slot-remote` or
`HOLLOW_SLOT_REMOTE`), a plain `hollow-slot <cmd>` decides whether the command runs on the runner or here and
says where and why in its first line (gates, Rust builds and whole test suites go there; quick checks, writers
and anything needing this Mac stay here; [WORKSPACES.md](WORKSPACES.md), Smart routing). `--here` and `--remote`
force it, `--postgres` gives a test suite a throwaway database wherever it runs, and Settings → Devices →
Builds and tests sets each turn's default (`HOLLOW_SLOT_ROUTE`). When the runner can't take it, it runs here.
No prompt text changed for it: the index line stays `hollow-slot <cmd>`, and the tool's own line tells the
agent. Same tool, same turns: nothing new for workflow steps, heartbeat passes or `ask_agent` delegates. A
Codex turn in plan, default or accept edits has no network in its sandbox, so its calls run on this Mac (the tool says
so at once); in auto and bypass it reaches the runner.
Since the close-out the tool also holds this Mac's memory floor for every agent: under 30% free memory a
build or dev server waits once it has its slot, then refuses with exit 69 after 5 minutes (the Claude hook
applied it to Claude alone, and never to `hollow-slot`). See [WORKSPACES.md](WORKSPACES.md) (Remote runs).

**Plan mode** attaches no teammate, no services and no library connectors; task and app tools are
read-only.

### Videos an agent makes (2026-10-17)

An mp4, m4v or mov plays inside Hollow, with no special markup, when it sits in the vault (a workspace's `outputs/`
included), a connected project, a live build worktree, or the conversation's own folder or a repository its receipts
name. A video the turn made or changed (its receipt lists it, so a render through Bash counts) or that the reply names
(a link, an inline-code path, a bare absolute path) plays under the reply, three at most; it also opens in the side
panel's player, and a Result plays in Results. Files in `/tmp`, `~/Movies`, `~/Downloads` or the run's scratch folder don't play (the player says "Outside
Hollow's folders" and offers QuickTime). Render into `outputs/` with `-movflags +faststart` and H.264 or
`hvc1`-tagged HEVC: `hev1`-tagged HEVC and WebM don't play in WKWebView here. Don't write raw `<video>` tags.

## Child sessions started by an agent

`hollow_app`'s `start_session` and `send_to_session` (`app_sessions::start` / `send`) are the
**durable child task** contract (D1, below). They queue an
ordinary chat turn through `commands::send_chat`, the composer's backend path. The child gets
everything a chat in its folder gets from the backend: layers 1–8, its own retrieval, recall and
connectors chosen on its own text, and the context compiler's layers 9–12 (entry `appSession`). A
start has no native session, so its first turn gets the enabled operator profile once (and no
conversation or task state yet); the prompt after it is the caller's task text as written, with no
intent block, after the task packet. A send to an existing conversation resumes its native session and gets none of those
blocks, only the operator update if the profile changed; if that resume resets or fails, the compiler
adds the bundle, the task state and the profile (see below). Each turn an agent sends is recorded as the
agent's (`turns.asked_by`), so the child's task-state record says the agent asked for its constraints. A send to a conversation with no native id
(a Grok or Antigravity one, say) starts fresh, with all three.

**The status line (repo ideas 2026-10-27 #7).** The child task packet's header asks every child
to end its answer with one last line on its own: `Done`, `Partial: what is left` or `Blocked: what
would unblock it` (`delegation::CHILD_TASK_HEADER`). `child_status::parse` reads it back from the
answer's last line (trailing fenced blocks such as `hollow-next` skipped; `Status:`, list and
emphasis marks allowed; detail at most 200 characters); a missing or prose line is `unknown`,
never done. The parent's `<hollow-child-finished>` line says "it says Blocked: …" (or "no status
line"); the person reads it at the end of the subagent's answer in the chat's Subagents tab (the COO page's Chats I started chip went with COO rebuild L3).
The parser is Hollow's one reader of the line, for any surface that shows a child (#1047's task
cards included).

**The slim workspace envelope (S01).** A child turn (depth > 0) whose text leads with a task packet
(`delegation::child_packet_sources` → `SourceRequest::child_packet`) gets a slim `<hollow-workspace>`
instead of the full one, 12,288 bytes at most with its tags and preamble (`workspaces::CHILD_SLIM_ENVELOPE`;
the JSON gets what the preamble, with every note it may carry, leaves): WORKSPACE.md's frontmatter, its
lead and its Outcome and Decisions sections (any section whose name starts with "Decisions", such as
"Decisions and constraints": `kept_brief_section`, RF-806-F1), its other sections by heading only
(`brief_outline`, 3 KB);
PLAN.md's headings only (`plan_headings`, 1.5 KB; `#` lines in code fences don't count); no SOURCES.md,
AGENTS.md or goal alignment; and every attached source listed, with passages (a 3 KB budget) only from
those the packet's `sources` paths name (vault-relative, workspace-relative or absolute); the rest say
`budget_exceeded`, "not searched" (the SOURCES_NOTE meaning; `not_selected` would claim a search). Over
its caps the brief is cut at whole lines: the Outcome and Decisions headings always stay, their bodies
share the room (`context_shares`: a short one whole, so a long Outcome can't starve Decisions), then the
other headings, then the lead; each cut is marked ("N more lines of Decisions left out") and the
document marked cut, so the preamble gets the cut note. Over 12 KB, the text files the packet doesn't
name give way to their paths alone (`listedSourceFiles`, with a preamble line saying so), then those
paths from the end into `omittedSourceFiles`, before the named passages; named and unreadable files keep
their entries and notes. The preamble's slim note names the full files present and their folder ("read
them with your own tools when you need more"); only the `listedSourceFiles` line may follow it. The manifest records the
layer as `stableLayers.workspace` `child-slim`; a resumed child turn whose session already holds a slim
envelope records `delta`, as any resumed turn does. Before, a packet child
got the whole layer (75,201 bytes for Claude in OB15's `packetChild`, 24 KB of it excerpts). The
person's turns and a child turn without a packet get the full envelope as before. A handover's successor
with a packet gets the slim one too (`delegation::child_envelope`, overnight 9 A6; S01 had kept it full,
and eight lane successors' first turns carried 74–75 KB of brief and plan); a packet whose `expected`
asks for the "full brief" keeps the full envelope, successor or not. `delegation::asks_for` matches whole
words in `expected`'s lowercased text (a schema's JSON text too), and a "no", "not", "without", "never"
or "don't" among the three words before cancels it: a heuristic, not a parser.

**Child turns' visual block (overnight 9 A6).** A turn below the person (depth > 0) at Most replies runs
at When it helps (`visual_prompt::for_turn`): the short block in the appended prompt and the guide only
with a visual turn, instead of Most's whole guide (about 12 KB) every turn. A packet whose `expected`
asks for "canvas", "canvases" or "hollow-canvas" (`delegation::child_wants_canvas`) keeps Most; Off and
When it helps are never changed. The fingerprint hashes the level a turn ran at, so `visual_prompt::steady`
keeps a child turn at its open native session's level when that is the other of Most and When it helps:
alternating sends don't reset the session. The person's turns run at their own choice, so the person
typing into a child chat whose session runs at When it helps starts one fresh session, and the agent's
later turns then stay at Most.

Guards: depth (`HOLLOW_APP_DEPTH` + 1; a caller at `app_mcp::MAX_START_DEPTH` = 2 can't start or send,
counted from the COO or a project lead since L1: their turns start from depth 0 whoever messaged them, so
their subagents are depth 1 and may start helpers, and helpers can't; the server lists the starters at
`AppGrant::list_depth` (`app_sessions::start_depth`, env `HOLLOW_APP_LIST_DEPTH`) and the broker checks
`app_sessions::effective_depth`, while `caller.depth`, which "a chat the person started" checks read, is
unchanged), at
most Settings' "Subagents per chat" (`chat_limits::ChatLimits::subagents_per_chat`, 1 to 20 since 2026-11-16, default 6, was
the fixed `MAX_RUNNING_PER_PARENT` before 2026-11-12) of one conversation's subagents running at once (L1: a
further start is queued, `run_pump::Hold::ParentLimit`, "a free subagent slot: this chat is at its limit of 6
subagents at once (Settings → Chat → Chats at once)"; the COO changes it with `settings.set_chat_limits`), at most
`MAX_STARTS_PER_RUN` = 25 starts per calling run as a runaway guard (messages to existing conversations and
`sessions.handover` don't count, `app_sessions::take_start`; a handover stays once per turn and per
conversation, and a chain stops at `MAX_HANDOVER_CHAIN` = 24 successors), and a mode no stronger than the
caller's without the user's Allow (`escalation`). A send runs in the target conversation's own mode (since 2026-11-12;
`app_sessions::send_mode`): a `permission` argument may only lower it (stronger is refused), and a caller in plan sends in plan. **Since 2026-11-20 the sender's mode carries down the chain** (`send_mode_lifted`): a chat sender (the COO or any chat the person ran; not an unattended turn, not an automation, not plan, and never into a conversation stored in plan) whose turn mode is stronger than the target's stored one sends the turn in the SENDER's mode, never above it, with no card, so a project lead stored in acceptEdits that the bypass COO messages runs in bypass and starts or messages its own subagents up to that mode without a card (`escalation` compares with the turn's mode). A chain from an acceptEdits chat stays acceptEdits; hard-coded always-ask actions are unchanged. The receipt (`inheritedMode`, `modeNote` in the send's result, so in History) names the mode, the stored one and the sender; a held send keeps it (`HeldInput.mode_lift`). It runs
on the conversation's own agent with the model its newest turn used unless the caller names
`model` (L1, `app_sessions::follow_up_for`: the live row's payload, else the usage ledger's
model; before L1 a follow-up ran on the saved default), and at the `effort` the caller names, else the
chat's own read when the turn runs, else Settings' (a turn's recorded effort is never reused, 2026-11-13). `start_session` takes `effort`
too (`auto`, the default, asks the CFO: `docs/CFO.md`). A send to a conversation
whose turn is still queued or running is held behind it like the person's Queue for after
(`commands::hold_agent_send`, at the agent's depth and effort, with the asker's delegation record stored
once it is queued) and the reply says `held: true` and its place; `busy_send` words any refusal left. The
composer's refusal (`CHAT_ALREADY_QUEUED`) keeps its words. Every start records its conversation's parent
(`sessions.parent_key`, `fork_kind = 'subagent'`, from the queued payload's `parent` when the turn
starts; older ones are backfilled from the child-task delegation records when the store opens; a
handover's successor stays the same chat). Overhaul K3 (11-12, `fresh_brief.rs`): a `send_to_session` to a
subagent whose stored context figure (`context_gauge::stored_figure`) passed the first growth step, with a
message of at most 1,500 characters, starts a fresh subagent of the caller instead (a `start_session` in the
same folder, agent and model) whose first message is the follow-up and a short brief (title, goal, last
answer, settled constraints and corrections, changed files, `read_session` pointer, one edit pass per file);
the reply carries `freshFrom`, and the new row's `freshFrom` makes the Subagents tab say "Fresh chat with a
brief". `fresh: true` asks for it on a subagent below the threshold (refused on any other chat); `fresh: false`
always continues. Either fresh path needs the caller to be the subagent's parent (`parent_key`, or its chain's
current conversation) and the subagent idle (no live `run_queue` chat row): otherwise the automatic path is an
ordinary send (held behind a running turn) and `fresh: true` is refused (`fresh_brief::NOT_PARENT`, `BUSY`). Idle is
checked under the admission gate and the subagent's keys stay reserved until the fresh start is queued
(`chat_rows::reserve_if_idle`, 11-13 R1), so a send admitted in between is refused as busy and held behind. Since
11-13 R3 the subagent kind and parent are read on the chain's root (a handover's successor of a subagent goes
fresh too), the fresh chat runs in the chain's current conversation's folder and agent (where an ordinary send is
routed), and that agent and model are checked against a non-empty CFO allowlist (refused: `fresh: true` fails,
the automatic path continues the conversation). The child's mode is clamped by the current conversation's last mode too
(`fresh_brief::clamp_mode`), and copied fields are redacted (`claude_runner::redact`) before they are cut.
The COO's and each lead's role carry `fresh_brief::FOLLOW_UP_RULE`. `sessions.subagents` (L2, `subagents.rs`) reads a conversation's
subagents (Active across its chain; Done the current conversation's own, the rest as `done_older` via `older`) with status, model, effort, working time, tokens and cost;
`sessions.subagent_budget` (L9) raises one subagent's CFO budget after it was paused at it. The
caller learns that the child's turn ended without polling (D2, [below](#completion-stop-and-cost-d2-f10))
and reads the answer with `read_session`. Any conversation `read_session` reads, the caller's own
included, can also be searched with `query` (at most 12 redacted snippets with their turn numbers and
tool ids) and read by number with `fromTurn`/`toTurn` (at most 20 turns), instead of reading 100 turns
with `tools: full` (repo features R4; `app_actions::search_conversation`, `slice_turns`; arguments in
[agent tools](reference/agent-tools.md)). The `<task-state>` transcript hint still names `lastTurns`:
agents learn `query` from `read_session`'s always-loaded schema (+291 bytes).

### The two contracts and the task packet (D1, F10)

`delegation.rs`. Every text an agent reads names the two the same way: `ask_agent`'s description,
`start_session`'s, `send_to_session`'s and `read_session`'s, the capability index (layer 2) and the
"Delegating" guidance (layer 4).

| | Bounded consultation | Durable child task |
|---|---|---|
| Tool | `hollow_team` `ask_agent` | `hollow_app` `start_session`, `send_to_session` |
| Waits | yes, in the tool call | no; `wait_for_session` waits up to 300 s for its turn (or the first of several) to end, or the next turn's `<hollow-child-finished>` note reports it; read the answer with `read_session` |
| Deadline | 180 s (`delegation::CONSULTATION_SECONDS`), or the packet's `budget.seconds` when lower (`delegation::build`), after up to 30 s for a run permit, counted from the asking agent's send (D2); Codex waits 240 s for the tool (`team_mcp::CODEX_TOOL_TIMEOUT_SECS`). Cut off at the deadline with text written, it returns that text flagged `Incomplete:` (OF04, [Teammates](#teammates)); a cancel returns nothing; a timed-out Claude consultation under a budget below 90 s draws a note | none Hollow enforces; the budget is the asking agent's request |
| The parent's stop | ends it (the stop sweep's SIGTERM drops every consultation in flight; its usage row says `cancelled`) | a stopped chat turn cancels the child turn it queued or stops the running one (D2); a normal finish leaves it running |
| Cost | its `delegate` usage row, under the parent's run id | its turn's own roll-up, listed on the parent's (D2) |
| Durable state | none: no conversation in History | its own conversation in History |
| Context | the packet, the task, the working folder (no Hollow tools, profile or conversation) | a chat's: layers 1–14 (the slim workspace envelope with a packet, S01), the profile once per new native session |
| Packet | always (its budget has the deadline) | when it carries anything |

The packet (`<hollow-task-packet>`, `Packet::render`): a header for the contract, then one JSON object
(`<` escaped): `contract`, `askedBy`, `constraints` (`{text, from}`: `parent` = the asking agent passed
it, `user` or `agent` = settled in the asking conversation), `sources` (`{path, from, excerpt?}`: `parent`,
`named` by the asking conversation's newest request, or `changed` by its turns), `expected`, `budget`
(`seconds`, `usd`) and `omitted`. The parent passes the `packet` argument (`delegation::parse`; a bad one
runs nothing, and the error shows the right shape). `sessions.start`, `.send` and `.handover` publish its
full schema (`delegation::packet_schema` through `Param::shaped`), not a bare object, and `constraints` or
`sources` given as one string is taken: a JSON array as text, else one item per line with list markers
dropped (HB04; the same `parse`, so `ask_agent` takes string lists too). The host fills the rest only for a
chat parent (`Parent::of_conversation`): its stored task-state record (K2) plus the turns not yet folded
into it, the running one included, folded into a copy (never written), corrections first, and the explicit
entities of its newest request (F2); deleted files are left out. Only constraints and corrections from the
parent's newest 8 asks by a person or an agent reach the packet (`session_store::person_ask_ords`; a turn
Hollow tagged `hollow:*` or a wake isn't one); the record keeps the rest (overnight 9 A1). A child's own
packet's constraints are in its record too (RF-797-F3): `execute_turn` stores them on the turn
(`delegation::record_child_packet`, `turns.packet_constraints`) and `task_state::fold` adds them before the
turn's own rules, citing that turn (so the same 8-ask window applies; a rule a later packet carries or the person
types again moves to that turn, and to the end past the first 4, and becomes the person's when the person said it) and keeping the packet's `from`
(`user` unmarked, else `"by":"agent"` or `"by":"parent"`; the agent header names both). So the child's own
children inherit them: `parent` reaches them as `agent` (an agent asked it in the child), `user` as
`user`. An automation parent has no conversation,
so only what it passed. Bounds: 10 constraints (280 characters each), 12 sources (paths ≤ 300 characters; an
excerpt only when ≤ 600), `expected` ≤ 1,200 (refused above), the block ≤ 6,000 characters; over it,
excerpts go, then the host's sources and constraints, then the parent's, each from the end, all counted in
`omitted`.

Where it goes: a consultation's prompt starts with it (then the role preamble, the task, the folder
directive). A child task's turn carries it at the front of its composed text; the context compiler keeps
it first in the body (`compile_with`: before the operator update, the bundle, the task state and the
profile, and a visual turn's guide, which `TurnGuide::apply` places just ahead of the task text past
the packet), after the backend's wrapper (layers 6–9). It is an injected block
(`sessions::split_injected_context`), so the child's request, title, retrieval and connector choice use
the task text alone, and its manifest measures a `taskPacket` layer. The asking agent's own task text
can't forge one: a leading packet tag in it is escaped (`delegation::neutralize`).

### Completion, stop and cost (D2, F10)

`descendants.rs`, `delegation::WAITING_TOOLS`. A child task's delegation row keeps its turn's run-queue
row (`childQueueId`), which says whether it is queued, running or ended, how, and as which run.

- **Deadlines.** Each tool that waits on a child or teammate ends at least 15 s before the calling agent's
  own tool timeout for its server (`delegation::caller_tool_timeout`): `ask_agent` 210 s (permit and
  deadline) against Codex's 240 s, `wait_for_session` 315 s (the `hollow_app` server's wait for the GUI's
  answer, `app_mcp::broker_wait_secs`) against Codex's 480 s. Claude Code's `MCP_TOOL_TIMEOUT` is unset,
  so it has none shorter; Antigravity and Grok get no owned MCP. The bounds count from the agent's send
  because the team and app servers run each tool call as it arrives (`team_mcp::serve_lines`,
  `app_mcp::serve_lines`, a thread per `hollow_app` call): each used to answer one at a time, so a second
  consultation waited for the first's 210 s before its own began, and a second `wait_for_session` (or
  any `hollow_app` call sent beside one) for the first's 315 s, past Codex's 480 s.
- **Completion, reported once.** `wait_for_session {key | keys, timeoutSeconds?}` (a read; default 60 s, at
  most 300; `keys`, at most 12, returns when any one ends, each one's answer under `sessions`, and waits past
  an ending already reported to the caller while others still run: only the caller's own children are ever
  marked reported, so another conversation's ended key still answers at once, overnight OB10a)
  returns when the session's newest turn has ended (status, end time, run id, cost), at the deadline (still
  running, with each turn's `runningSeconds` or `queuedSeconds`; a chat whose live keys are all its own
  children is told their ends come as its next turn's note, so end the turn, calling `sessions.wake_me` first
  where offered (not in plan mode, an unattended or woken turn, or a build workspace) to come back on its
  own, since only the COO is woken; anyone else is told to wait again only if it
  needs the answer), at once when it is queued for the vault the caller's own turn holds, or for a build
  worktree the caller's turn holds alone, or for a chat slot only the caller's turn can free (its workspace's
  limit or the app-wide one, filled by that turn alone; `run_pump::waits_only_for_slot_of`, 2026-10-20) (it
  can't start before that turn ends; a child in the caller's own workspace folder otherwise starts beside it
  since 2026-10-07), or when the caller's run has ended. Otherwise the asking conversation's next
  turn opens with `<hollow-child-finished>` (`descendants::notes_for_turn`, compiled in
  `compile_turn_within` for every session kind, after a packet and an `<operator-update>`, before the
  bundle; budget tier reserved; manifest layer `childFinished`): one line per child turn that ended since
  (key, agent, status, end time, a failure's error cut to 160 characters), at most 8, saying it is news, not
  a request. Since OB17 each line also carries `descendants::child_summary`: from the turn's receipt its files
  (count, up to 5 paths), PRs (number and action), failure count and `verificationState` (since overnight A8
  followed, when S27's flags say its reply claims a pass the receipt doesn't back, by `; says it passes; its last
  check failed` or `; says it passes; no check after its last edit`, ≤ 60 B, the failed check winning), then the end of its
  answer (redacted, cut to a line start, indented, `<` escaped), the lines sharing
  `clamp(3600 / lines, 300, 1200)` answer characters each; `read_session` is for more. Each child turn is
  reported once (`session_store` `child_deliveries`, by queue row): a wait that returns it marks it, so no
  note follows and a second wait says `reportedBefore`; a note marks it for its turn, so only a retry of that same turn repeats it.
- **Stop.** A chat turn that ends stopped (`cancelled`) stops the child task turns it queued, before its
  lease is released (`chat::stop_child_turns`): a queued one leaves the queue and ends `cancelled` (its
  window gets the same events a keyed `chat_cancel` sends), a claimed one is flagged so the pump ends it,
  a running one is stopped like the composer's Stop; one that already ended, and any later turn of that
  conversation, is left alone. A normal, failed, stalled or timed-out finish leaves them running: a child
  task outlives the turn that started it.
- **Cost.** `descendants::rollup(run)`: the run's own usage rows (every attempt), each `delegate` row
  (a consultation) and `oneshot` row (a helper one-shot run for it: the learning check carries its turn's
  run id since D2) with that run id, and each child turn's own roll-up (a grandchild counts inside its
  child, once). A cost nobody reported stays `null` and makes every sum it is in `null`; `knownUsd` adds
  what is known; a child turn that never ran cost 0. The receipt seals it at the turn's end
  (`EvidenceReceipt.descendants`, with each child's `stop`); the manifest view reads it live
  (`costRollup`).

## Automations

`workflows::prepare_spawn`. What the agent reads, top to bottom:

1. The workspace policy and permission paragraph (Claude: as its whole appended system prompt; others: at
   the top of the prompt). There is no visual guide or environment brief: an automation's prompt decides
   its output.
2. `<hollow-workspace-boundary>`: `WORKFLOW_BOUNDARY`, the prompt-variable note when a variable carried
   external values, and the scratch guidance.
3. `<hollow-tasks>` and `<hollow-app>` guidance, when attached. The app guidance uses the automation rule:
   nothing can ask.
4. The `<hollow-connectors>` note, with the automation form of the external rule.
5. The workspace envelope (as for chats, when `cwd` is a workspace).
6. `<trusted-recall>`.
7. `<operator-context>` (`compose_run_context`, every run while About you is on).
8. `<retrieved-evidence>`.
9. The request: the workflow's prompt after variable substitution.

Owned servers (`mcp_attach::server_map_for`, workflow surface): teamwork (not in plan), task tools and app
tools. No question cards, computer control or vault search. A tool-restricted automation gets the app
server only when its allowed tools name it. It gets a library connector only when they list
`mcp__<id>`. Codex, Antigravity and Grok can't run an automation with explicit tool restrictions
(`agent::validate_run_spec` refuses it).

## The heartbeat

`heartbeat::prepare_beat`. The prompt, in order: the scratch guidance (tool pass only), `<trusted-recall>`
(recall on the beat's own prompt), the `<operator-context>` block, the already-proposed list and the beat
prompt. The adapter adds the workspace policy. Claude runs a tool pass with `Read`, `Glob`, `Grep` (and
`Write` when drafts are on), no Bash and no MCP. Codex runs a tool-free pass through the Codex transform,
with a host-read snapshot (≤ 60 notes, ≤ 60,000 characters) appended to the prompt. Antigravity and Grok can't honour the tool list, so `validate_run_spec` refuses the
beat. Since O3 the beat writes a run manifest with its receipt: its prompt's layers (`toolGuidance`,
`recall`, `operatorProfile`, `alreadyProposed`, `request`, `snapshot`) and its Jev duplicate check
(`dedup`, its request among the manifest's `jev`), read by the beat's run id.

## Teammates

`hollow_team` `ask_agent` (`team_mcp.rs`), on a Claude or Codex primary, outside plan mode: the
**bounded consultation** ([above](#the-two-contracts-and-the-task-packet-d1-f10)). The delegate's
prompt is the task packet, the role preamble from Settings, the `task` argument, a working-folder
directive (Antigravity also gets an image-generation directive) and, when the parent has a scratch
folder, the scratch guidance. The adapter adds the workspace policy. Nothing else reaches it: no
appended prompt, profile, recall, retrieval, workspace envelope, owned MCP or connectors. It runs at the
parent's mode with the parent's tool limits intersected (`delegate_grant`, unchanged by D1),
synchronously, until its deadline (180 s, or the packet's `budget.seconds` when lower) after up to 30 s
waiting for a run permit, both counted from the asking agent's send: the server handles requests as
they arrive (D2). Each consultation is recorded for the parent run (the session store's `delegations`
table) before the teammate starts; its cost is on the parent's roll-up, and the parent's stop ends it.

**Cut off at the deadline (OF04).** 12 of 63 measured consultations (19%) timed out, and the timeout used
to throw away what the teammate had written. Now `collect_command` keeps it in `Collected::partial_text`
(a non-error result envelope's text, else the deltas with a blank line between messages, else every prose
text block joined; never an error result's or API-error block's text), whichever provider ran; `result`
stays the timeout's `Err`, so `claude_runner`'s helpers and the heartbeat still treat a timeout as a
failure. `ask_agent` returns it as a tool error (the call did not complete) whose first line is
"Incomplete: <Teammate> was stopped at the <N> s deadline; what it wrote so far is below and is not a
finished answer. For longer work, continue with start_session (a durable child task) instead.", then any
note, the text (over 32 KB, only its last 32 KB after an "[earlier N bytes omitted]" line,
`team_mcp::capped_partial`), the created files and a closing "[End of <Teammate>'s incomplete output …]"
line. A teammate cut off before writing anything returns the old "<Teammate> timed out after <N>s" error.
Only a timeout returns partial text: a cancel (the stop sweep's SIGTERM) drops the call with no reply, so
the asking agent gets nothing back. A timed-out Claude consultation whose packet budget set a deadline under
90 s gets a note in its result, the incomplete one or the timeout error (`team_mcp::short_budget_note`); a
finished answer never does. The record is then rewritten with how it ended
(`delegation::Consultation`): `outcome` `ok`, `timeout`, `error` (a failure or budget stop) or
`cancelled` (a call dropped mid-run by the stop sweep's SIGTERM, written from the guard's `Drop`),
`durationMs` (teammate start to end, permit wait excluded), `returnedBytes` (the teammate text
returned, trimmed, and capped on a timeout; 0 on an error or cancel) and `incomplete`. A finished run whose
consultation record has no `outcome` means the consultation process was killed outright (SIGKILL, a crash)
before even the guard could write: read it as interrupted.

## Resume, reset, switch and Continue

| Case | What the next turn carries |
|---|---|
| A Main-agent wake (acceptEdits) or a `send_to_session` turn at a weaker mode, between bypass turns (HB01) | Native resume: the fingerprint holds only plan or not (Grok: its sandbox profile), so the session, its context and the stable-layer deltas carry on; the turn's `<hollow-mode>` line names its mode and the CLI flag enforces it. Claude rewrites its prompt cache on the flip (its Bash tool text differs by mode). |
| Native resume (same fingerprint) | The CLI's own session holds earlier turns. The compiler adds no conversation bundle, task state or profile (manifest `compiler.session` `resume`); it still folds the task-state record forward (the manifest records its revision). Recall, the connectors note and the evidence are rebuilt and sent again; the envelope goes as an update and Codex, Antigravity and Grok get the one-line reference for the appended prompt, when the session's `stableLayers` baseline names this native id (G2; `compiler.stableLayers`). |
| Profile or `me/Decisions.md` edited during a resumed conversation | The next resumed turn gets `<operator-update>` once: the changed lines of each `me/` file's digest section since what the session was given (its `profileBaseline`: the digest's sections as sent, stored on every fresh session; a line the digest cut was never given, so it arrives as added once it fits), not the whole profile. A given line pushed out of the digest by its caps (lines added above it, or an earlier file growing until a later one no longer fits) still holds: it isn't reported, stays in the baseline, and is reported once it is deleted from the file. No restart: the fingerprint doesn't hash the profile. A session from before K2 has no baseline, adopts the current profile and gets nothing; with About you off nothing is compared; a session started with it off gets the whole profile as added lines once it is switched on. |
| Agent or mode switch in the composer, or a new conversation | The frontend clears the native id; the plan is `new`, so the compiler adds the conversation bundle from the session store, the task-state record and, if enabled, the profile (layers 9–11). |
| Reset by the backend (`chat_resume::plan_resume`: configuration changed, resume failed, conversation moved, invalid id; since overnight 9 A9 also `rested`) | The compiler adds all three (`reset`, with its reason), skipping a bundle or profile already at the front of the composed text. The task state carries the constraints and corrections the recent-turn tail may have dropped and marks finished steps done. The manifest records a `session` fallback. A configuration change also names the changed fingerprint parts (`sessionReset.changed`, `compiler.resetParts`; OB03), or `unknown`. **Rested (A9):** a Claude turn in the Main agent workspace, not plan mode, bare or unattended (a wake or check-in never rests: a fresh stint expires open question cards), nor sent by another agent (`app_depth` above 0, `send_to_session`; `may_rest` refuses it, RF-810-F1), that would resume after more than 60 minutes since the conversation's last turn ended (`session_store::last_turn_ended`) at a stored figure of at least 350,000 tokens (`context_gauge::stored_tokens`, `ADVISORY_TOKENS`) starts fresh instead (`chat_resume::rests`, `with_rest`), because the expired prompt cache made each such resume cost $5–6. Settings → Background work → Main agent → "Start fresh after an hour away" (`MainAgentSettings.freshAfterAway`, on by default) turns it off. `sessionReset.away` and the receipt's `rested` switch row say "away 3 h at 714K". |
| A failed resume retried inside the turn | The compiler again, fresh (`retry`), with the task state, so the steps earlier turns finished are marked done and not repeated; the manifest's `compiler` record is replaced and a `session` fallback added. |
| Sessions -> Continue from summary | A new conversation (new key, no native id) whose window shows the earlier session's turns. The send names that session in `continuedFrom`; on the first fresh turn the compiler bundles its recorded turns (all of them, from this vault's store) ahead of the new conversation's own, merges its task-state record ahead of the new one's (items tagged with its key), then the profile. Later fresh turns of the same window (a switch, a reset) do the same, as the old composer's window-built bundle did. The composer text still carries `artifactContinuationPrompt`'s last 3 summarised turns. |
| `sessions.handover` successor (HB06) | `app_sessions::handover` queues the successor's first turn with `continuedFrom` = the caller's key, and its header names that key ("read_session <key> for its last answer"). Being an app-session entry, the fresh turn merges the predecessor's task-state record (items tagged with its key) but gets no `<conversation-context>` of its turns (manifest `continuedFrom` and `seeded` 0, `taskState.merged` 1); `start_session` and `send_to_session` continue from nothing. The handing-over turn is usually still running (25 of 30 handovers in the sessions DB): a continued conversation's record is read with `task_state::peek` (OB14), the stored record plus every turn not yet folded into it, the running one included, folded into a copy that is never written, so the predecessor's own `refresh` still folds that turn once it ends. The running turn's finished calls are steps marked `unconfirmed`; a call still waiting on its result (the handover itself) is none. `Parent::of_conversation` shares it (`peek_turns`, its newest 8 pending turns, without receipts). |
| A new day's COO thread opened by Hollow (OB19) | A wake, check-in, weekly update, morning review or swarm synthesis that finds no thread today queues with `continuedFrom` = the newest earlier COO thread (`main_agent::continued_for`) and its `asked_by` tag (`hostAsker` in the row, OB20). The compiler treats a host-tagged turn like a handover's successor though its entry is `composer`: it merges that thread's task-state record (items tagged with its key) with no `<conversation-context>` (manifest `continuedFrom` and `seeded` 0, `taskState.merged` 1), and the `<task-state>` block opens with one line naming the key and how to read it (`context_compiler::continues_note`: "hollow_app read_session with {"key":…} reads its turns", or that this run can't). A thread with no record still gets that line in a block of its own. |
| The model of a Hollow turn in a COO thread (fix-up 10-17) | Wakes, check-ins, the weekly update, the morning review and swarm syntheses ask for the model the basis thread's newest turn asked for (`main_agent::thread_basis` → `wake_me::wake_model`: today's thread, else the newest one; the CLI default stays `""`; another agent's model, or no turn yet, gives `None`, the saved default). Before, they all sent `model: None`, so a thread on a non-default model changed its resume fingerprint and got a fresh native session on every wake. So a new day's thread opened by Hollow starts on yesterday's thread's model, as it already took its agent and mode, not the saved default. The isolated morning review keeps the model but still starts a fresh session (its tools differ). |
| Working folder vanished (`recoverChatContext`) | The key is dropped and the window keeps its turns; the left key is appended to `continuedFrom`, so the next turn (a new key, fresh) gets those turns bundled. Before the K1 review fix this path, and the one above, lost the bundle. |
| Continue (`chatContinue.ts`) | Exactly `CONTINUE_PROMPT` from the composer, on the same native session, so nothing is added; were the session to reset, the compiler adds both blocks as for any turn. The backend wrapper still applies. |
| Retry under a failed last turn (`chatContinue.ts` `retryIndex`, P1-27) | Offered from the run's own status (a stop or failed checks never offer it; a stall is Continue's; a Main agent wake is never resent). The failed message's own text, sent again as an ordinary send (the draft stays), so the compiler treats it like any turn. Retry with another agent switches the conversation's agent first (`setChatAgentPersistent` clears the native id), so that turn starts fresh with the written conversation. Retry with another model (D1, 11-07) lists the agent's other model families (`retryModelChoices`, four at most; the model menu's cached `loadCatalog`) and sets the chat's model (`setChatModelPersistent`) before the send, so the chat keeps that model; the native id stays, but the model is a resume-fingerprint part, so that send starts a fresh native session, as a model-menu change does. An image-only message isn't offered (its path isn't kept). |
| Quota failover (Claude, R1) | The same native session resumed on another account with `FAILOVER_PROMPT`, at most once per turn. |
| Provider outage backoff (overnight 9 A3, not Claude) | A failed attempt that `result_kind::is_backoff_outage` calls an outage (Transient and an overload, 5xx or network error; for Codex also `server_overloaded` / "at capacity", which `envelope_class` now checks before `QUOTA_RE`) waits `chat::backoff_wait` (60–120 s, jittered; a Stop ends the wait) and resumes the same native session with `BACKOFF_PROMPT`, once per turn, only if 60 s of the turn's deadline would still remain. The outage's error envelope is held (`TurnStream::defer`) and dropped when the retry runs; the receipt gets a `retry` switch row ("Retried once after the provider was overloaded (N s later, same session)"), which the chat timeline shows as "Codex resumed once after an outage". Claude is left out: its CLI retries its own overloads, and R1 owns its accounts. A plan limit, a sign-in failure or a bare 429 is never retried. |
| Continue in a new conversation (after an interrupted run, or from the usage ring past the hand-over point; R6) | `ChatBody.branchFrom`: a new conversation (new key, no native id) whose composer holds `handoffText`'s written handoff, sent as an ordinary first message, so nothing else is added. The ring offers it at 75% of a known window or 350K on a 1M window (`handoverOffered`, mirroring `sessions.context`'s advisory). An agent can read the same figures the ring shows, sizes only, with the read action `usage.conversation_context_use` (tool results by size since its stint began, cache share per turn, compactions); nothing pushes them into a prompt. |

Every character bound counts Unicode scalar values (Rust `chars()`, JavaScript `[...text]`); no frontend
code counts against these limits any more. The shared fixture is
`src-tauri/fixtures/continuity/char-counting.json` (`context_compiler::tests`, `src/textCounting.test.ts`).

## How retrieval and connectors choose

### The request (C1)

`chat::accept_turn` splits the composed prompt once (`turn_request::TurnRequest::admit`, using
`sessions::split_injected_context`) and the queued payload carries the parts:

| Field | What | Bounds |
|---|---|---|
| `request` | the user's own words (= `asked`) | |
| `entities` | paths the request names (vault-relative when inside the vault), attached image paths, `projects/`, `people/`, `goals/` notes named by file name (whole words of the original text; a combining mark continues a word) | ≤ 8; ≤ 500 notes per folder indexed |
| `antecedents` | what "that project", "the same file" resolve to, from earlier requests of the same conversation; a kindless "that one", "same one" or "as before" only with a kind word in the request ("the project, as before"), and only to that kind | ≤ 3; reads ≤ 8 earlier requests, 4,000 characters each; user requests only, never replies; never a model call |
| `contextBlocks` | the injected blocks, kept apart: tag, size and SHA-256 (the text stays in the prompt only) | |
| `taskState` | the conversation's task-state record as admitted: `revision`, `throughTurn`, `items` (K2); absent before the conversation has one | a reference; the compiler folds anything newer and sends the record |

Who reads what:

| Stage | Reads |
|---|---|
| Retrieval (chat) | `asked`, stripped again inside retrieval (idempotent), unless the turn's query basis (below) gives another query or skips |
| Recall | `selection_query` (request + antecedent display names) and `subject_paths` (entities + antecedents) |
| Connector selection | `connector_query`: the local rules read `selection_query`; Jev is sent the request alone (antecedent names are text the person didn't type this turn) |
| Post-turn learning | the request (without hidden answer blocks) and the reply, with subject paths as targets only |
| Workflows | the prompt after variable substitution (`TurnRequest::from_text`) for retrieval, recall and connectors |
| Heartbeat recall | the beat's own prompt, always current-state |
| History intent (S3) | the request's own words (`note_lifecycle::history_cue`), never antecedent names; retrieval reads the same stripped text |

**The query's basis (overnight OB18, OB25, 2026-10-17).** A chat turn's retrieval, recall and attached
sources read `TurnRequest::basis_in` (`turn_request::RetrievalBasis`) in place of the request when it gives one:

- **Host turns:** a wake or check-in reads its subject (`main_agent::host_subject`): a wake's chat
  titles and workspace names, a check-in's "New since" signal lines; never the template ("report",
  "PLAN.md", "finished"). Its connectors are chosen from the subject too. An empty subject (a check-in
  with nothing new) skips. Hollow's other messages (swarm, `wake_me`, weekly update, morning review),
  night items and runbooks read their request.
- **Short follow-ups:** a request with fewer than 4 retrieval terms, in a conversation with an earlier
  non-host turn (`asked_by` not `hollow:*`, not a wake; `session_store::person_asks`), queries its own
  terms then its prior's, capped at 16 by the same tokenizer. The prior is the newest of the last 8
  such requests with 4 or more terms, else the task-state goal. Connector selection sends Jev only the
  typed request; the evidence rerank sees the basis query. A turn whose resume plan really resumes its
  native session (`ResumePlan::Resume`, settled before retrieval) and adds no term the prior lacks
  (go-ahead words like "yes", "merge", "continue" aside) skips; on a fresh or reset session it
  retrieves on the basis query (`followup+prior`).
- **A skip** retrieves nothing (no broker passages, so no Jev rerank) and puts no evidence in the
  prompt, like a bare turn, and keeps the conversation's last connector decision
  (`evidence::last_connector_selection`, its newest chat manifest → `connectors::attach_kept`): no Jev
  call; a connector that decision didn't cover is decided locally, and one the request names is
  attached. A skipped follow-up with no decision to keep selects connectors as usual. If its resume
  fails and the turn retries in a fresh session, the retry retrieves then
  (`directory_connections::local_context`: broker order, no Jev) and its record replaces the skipped
  one. A request with 4 or more terms reads exactly as before. The manifest records the basis as
  `retrieval.queryBasis`, also when retrieval fails. Recall's history cue reads a follow-up's own
  words and a host turn's subject; vault retrieval's own history cue reads the basis query (prior terms included).

### Retrieval

One host retrieval broker (`retrieval_broker.rs`, S1/F3) serves automatic retrieval
(`directory_connections::prioritized_context`, chats and automations), `search_vault` (the
`hollow_vault` subprocess) and the app actions `vault.vault_search`, `vault.semantic_search` and
`vault.retrieve`, so candidates, ranking, scope and paths can't drift between them. A test pins the
parity: for one request and scope, the manifest's pool, `search_vault`'s passages and `vault.retrieve`
list the same paths in the same order.

- **Scope** (`resolve_scope`, recorded as `scopeBasis`): a workspace folder whose id is a connected
  project's (`workspace`); a build workspace's worktree, any name: the connected project that is or
  contains the repository it was created from (`build-workspace`); a folder inside a connected project
  (`project-folder`); a linked git worktree of a repository in one (`git-worktree`). A project's
  workspace (`workspaces/local-…`) whose project left the index keeps its id, which no project matches
  (`workspace-disconnected`: no connected-source evidence; scope never widens by accident); a build
  workspace whose repository no connected project holds is scoped the same way. Anything
  else, and every other folder in the vault, sees every connected project. Never from a folder's name.
  The vault half is never scoped.
- **Terms:** up to 16 distinct words longer than two characters, minus stop words (`retrieval_terms`),
  from the request alone (a chat turn: from its query basis, above).
- **Connected projects:** from the stored inventory, name matches first, then the most recently
  changed; at most 4,000 files and 32 MB read per request; up to 3 lines per file. Then PDF and
  Office files whose name matches a term (repo features R14a; at most 40 files and 16 MB per
  request), read by the host (`doc_text`: `## Page N` per PDF page, `## Slide N` per slide, one
  paragraph per line, workbook cells; the 50 MB uncompressed-XML cap cuts on a whole character,
  overnight 10 A3) and cited as `excerpt` passages with line numbers counted in
  that extracted text, which `read_source` shows too; so Claude, Codex and Grok get the same
  evidence. A failed read (an image-only or locked PDF) keeps the line-0 `requires_reader`
  passage with the reason as its excerpt. No prompt text changed: the envelope, the
  `read_source` and `search_vault` descriptions and the capability index are byte-identical.
  **By contents (repo features R14b):** with the project content index on (off by default) and
  holding at least 95% of the in-scope text, PDF and Office files as listed now, only candidates
  are read: the best 200 by `bm25(words, 0, 2.0, 1.0)` over that index (terms ORed as prefix
  queries, OB12's rule), name matches, the 20 newest and any file the index doesn't hold as it is
  now; name matches first, then BM25 rank, then newest, in the same budgets, and a PDF or Office
  candidate is read without a name match. The app and the `hollow_vault` subprocess open the
  index read-only per request, so `search_vault`'s project half ranks the same way. A term that
  isn't one word, a partial index or the switch off means the path above. The budget warning names
  which path ran; files in `restricted` folders and secret-named files are never in the index (a
  name match still reads them as before), and a document whose parse never returned twice
  running (two strikes) is never read again until it changes; retrieval and the index share those
  strikes (one marker per file), so turning the index off and on buys no more parses. A read that
  finds a parse of the file in flight (in either process, or the index's pass) waits up to 5 s
  for it rather than parse beside it, then answers "still being read", which isn't cached; the
  index's pass waits on a parse at most two minutes and only while the index is on, so a parse
  that never returns can't keep the switch from turning off; one that returns later (a slow,
  healthy file) is stored by the next pass, never counted as a crash. A document the index holds but
  didn't rank isn't parsed for being among the newest. Evidence stays inside the same envelope and budgets; no prompt or
  tool-description byte changed (the g1/g2 fixtures are identical).
- **Vault:** eligible notes (not `me/`, `sessions/`, `workflows/`, or workspace folders other than
  their `outputs/`), newest first. With the FTS index (OB12; the app uses its engaged index, the
  subprocess reads the same database read-only) only the candidates are read: the best 200 by
  `bm25()` over a word-tokenized table (`unicode61 remove_diacritics 2`, the file stem as a
  weight-2 column) with the terms ORed (a prefix match from 3 characters; when fewer than 200 rank,
  the trigram matches of every term of 3+ characters join too, so `auth` still reads `OAuth`
  and `api` still reads `会議のapi`, one `unicode61` token; RF-737: at most the rest of the 200 and
  4 MB of them, best trigram `bm25()` first), plus up to 50 notes whose
  stem names a term, the 20 newest, any note changed since it was indexed, and trigram matches of a
  term that isn't one word (a CJK run; under 3 characters there means a linear scan). Without an
  index (cold, or no database), or with a kinds filter (a kind needs the frontmatter), every
  eligible note is read: the linear scan, also the test oracle.
  Notes over 1 MB are skipped; the "not scanned" notice counts only those the request names (a term
  in the file's stem, or a dotted term such as `preview.html` in its name; never a folder; HF05). An `.html`/`.htm` note is matched and cited by its
  text: comments, tags, scripts and styles are stripped in place, line by line (one left-to-right
  pass, so whichever of comment/script/style opens first wins; an unclosed script or style runs to
  the end of the file; OF25)
  (`vault_embed_index::strip_html_lines`), so a cited line number is still the file's. The 32 MB
  scoring budget is charged only for notes that match, after the candidate choice, so newer unrelated
  notes can't starve an older exact match; a 256 MB read limit bounds a linear scan. Up to 3 lines
  per note.
- **Semantic (optional):** only in the app, when Settings' semantic search is on and its index is
  open with embedded notes: the request (≤ 2,000 characters) is embedded with the bundled bge-small
  model, on this Mac, and the best chunk of up to 24 eligible notes at cosine ≥ 0.6 joins. Never a
  network call or a model download; otherwise lexical only, and the manifest says why (`off`,
  `unavailable` with the reason). The subprocess has no embedder, so `search_vault` is always lexical.
  A note is chunked by its body's sections; a task, project, person or goal, and any note whose body
  is empty, also gets one chunk of its frontmatter `title`, `description`, `project`, `status` and
  `due` lines (HF03, `vault_embed_index::frontmatter_chunk`), cited at its first such line, never a
  body line. Embed schema v3 rebuilds the index once to add them. An index at a newer schema than
  the build (an older build installed over it) is left as it is, never dropped or written: semantic
  is `unavailable` and the keyword index likewise falls back to the scan (OF25). Versions are plain
  integers, so a branch build that bumped one leaves main with that index off for good; stderr says
  so (`[search index]` / `[embed index] … is newer than this build's …`). Recovery: quit Hollow and
  delete that vault's `<vault name>-<hash>.db` (and any `-wal`/`-shm`) under the app support
  folder's `SearchIndex/` or `EmbedIndex/`; the next launch rebuilds it (a full re-embed for the
  latter). Other vaults' indexes (never this vault's) are the person's Settings → Advanced → Disk space →
  Remove old indexes…, or `system.remove_stale_indexes` behind a card (repo features R7b): it offers
  only those named `<name>-<16 hex>.db` whose `.db` and non-empty `-wal` are over an hour old and
  that no other process has open (a lock on the `-shm`'s byte 128) and that isn't the file this
  Hollow's search or meaning-search `engage` opened (whatever its stem), plus any `<db>.removing`
  tombstone an interrupted removal left over an hour ago, refuses when this vault's folder
  can't be resolved, and removes each by renaming the `.db` aside first, then its `-wal`/`-shm` (the
  `.db` goes back if either can't, by a hard link that never replaces a database made there meanwhile:
  then the old one stays `<db>.removing` and the outcome says so). Just before each rename it probes
  again (the `-shm` lock, an hour's use), so a file opened since the list was made is kept as failed
  "opened since the list was made; kept". `system.hollow_databases` lists every SQLite file there with its
  owner, size, WAL and `user_version` (`check: true` adds a read-only `quick_check` and row counts,
  stopped after 30 s; FTS5's inverted-index check needs a write, so such a file reads
  `ok (FTS index not validated read-only)`);
  `system.stale_indexes` is the removal's preview. Both read only; a closed WAL-mode file's version
  comes from its header, so measuring creates no `-wal`/`-shm`.
  Moving the vault changes its stem, so `system.move_hollow_home` (2026-11-12) renames every store
  named with the old stem and rewrites the Sessions DB's path and workspace columns while Hollow is
  quit (a folder in iCloud is copied out, not renamed: iCloud-only files first with `move-home --download`);
  `system.move_hollow_home_plan` previews it, `system.rollback_move_home` undoes it
  ([DIRECTORIES.md](DIRECTORIES.md#move-hollow-home-2026-11-12)).
  Since O2 (F8) the half runs on its own thread beside the lexical scan (`retrieval_broker::retrieve_by`);
  a run's automatic retrieval waits for it until the preparation deadline at most, then ranks lexical
  only (`deadline`, below). The lexical half always completes.
- **Ranking:** the whole candidate set. With no project passage (vault-only retrieval) a note's
  BM25 band leads (OB12: one band per factor of 1.25 below the best score; a note the ranking can't
  speak for, edited since indexing or a CJK match, is band 0; name matches, the newest notes and
  mid-word matches come last; a note's second and third lines rank one band lower, never below the
  lowest band present; a linear scan is all band 0), then coverage. With ANY project passage the
  order is the pre-OB12 one: coverage (distinct terms in the line plus the file's name), then terms
  in the line, the band only a tiebreak after them, then inside a tie vault and project passages
  alternate, vault first and newest first (OB12's review found every way of keeping bands across
  sources could push a strong vault line behind dozens of weak project lines). So in a chat with a
  connected project, one project match is enough for coverage order: there OB12 narrows what is read
  but no longer reorders the vault. All band 0, both are the pre-OB12 order exactly. With semantic hits, reciprocal-rank fusion `1/(60 + lexical rank) + 1/(60 + semantic
  rank)`, a missing half adding 0, ties to the passage with a lexical rank, then lexical rank,
  semantic rank, path and line. Diversity is a ranking rule: a file's third and later passages move
  behind every other passage. Nothing is cut before ranking. The weights and the 0.6 floor are tuned
  on a synthetic corpus; `scripts/agent-eval/retrieval-labels.mjs` checks them against Jev's keeps in
  real run manifests (AUCs, keep rate by band, a weight × floor sweep; HF06), and against the agent's
  own behaviour (S26: citation rate and AUC against `cited`, need-recall from `notesOpened`).
- **Pool, rerank, budget:** the first 24 (`jev::MAX_CANDIDATES`) go to the context prioritizer
  (`context_rank::prioritize_passages_via`): with Jev on and at least two passages, one call scores them,
  a score below 1 drops a passage and the rest are reordered; off or failing, the broker's order stands.
  Then the one evidence budget: the diversity rule again (a reranker can't lift a file's third passage
  past the others), a historical passage's current version back ahead of it, then the first 8. When it
  leaves ranked passages out, a coverage notice says how many and points to `search_vault`.

### Lifecycle (S3)

One policy for recall and retrieval (`note_lifecycle.rs`, F7). The broker applies it to the whole ranked
candidate set before the pool cut; recall applies it to the approved learnings it lists. Only a learning
has a lifecycle: a `learnings/<name>.md` note with `type: learned-context`, the same notes recall lists.
Any other note is ordinary evidence whatever its frontmatter says, so a project under
`projects/superseded/` with `status: superseded`, or an ADR or Notion page marked Superseded, is still
cited when a request names it. Both routes read the same metadata, from the frontmatter each already
parses: whether the learning is approved (`trust: approved`), superseded (a `superseded_by:` link, as
`supersede_context` writes it, followed along a chain of at most 8, and only to another learning; or
`status: superseded`), `supersedes:`, its date (`learned_at`, else `date`/`updated`) and a key of its
exact statement. A note read by path to resolve a current version is refused when the vault walk would
skip it (`vault::walk_skips`: a hidden component such as `.trash/`, or the retired `emails/`, `teams/`,
`slack/`, `inbox/` and `_signature/` folders), so the stage never cites what retrieval keeps out.

- **Current-state request** (the default): a superseded learning is left out. In retrieval its current
  version takes its place in the ranking, read from disk when it matched nothing itself (at most 64
  reads per request), labelled `current: supersedes <path>`. So the correction outranks the old
  statement even when the old one shares more words with the question.
- **Historical request** (`history_cue`: "used to", "history", "historical", "previously",
  "formerly", "originally"; "before" only ending a sentence ("what was the config before?"), in
  "before it changed" or followed by a date on or before today (a later one is a deadline), a version, "the change"/"the switch", "last week" and the
  like; "changed" only as "changed since" one of those (HF04, 2026-10-16); or an explicit past date: an
  ISO date, a month with a year once it has ended, a past year after "in", "since" and the like): the
  superseded learning stays, labelled
  `historical: superseded by <path> (current since <date>)`, directly behind its current version, in
  both routes. Recall keeps it only when it matches the request or a subject, not for being global.
- **Exact duplicates** (the same statement, whitespace and case aside) collapse to the newest. Nothing
  semantic is compared.
- **An unapproved learning** is labelled in retrieval and never recalled.

A labelled passage has a `lifecycle` field in the evidence JSON, and the envelope then explains it.
Recall's block prints the historical label before the statement. `search_vault`'s keyword list labels a
replaced learning with its current version. The side panel card's Recall section shows the label (an automation's run record still lists it). Nothing
edits or deletes a learning, so the audit history stays. The manifest records each decision
(`lifecycle`, below).

### Attached sources and the reader (S2)

A workspace's attached sources go through the broker too (`retrieval_broker::select_attached`, in
`source_evidence.rs`), on the same request: `TurnRequest::source_query`, the request plus resolved
reference names (a workflow: its prompt after substitution).

- **Read:** each readable file under `sources/`, at most 1 MB, 16 MB per request; past that a file
  is `budget_exceeded` ("not searched").
- **Chunks:** at most 40 lines and 3,000 bytes, 8 lines shared with the next. A single longer line is
  its own chunk; included, it is cut to the 3,000-byte window around its best match, with "…" marks.
- **Score:** the broker's terms and coverage rule (distinct terms in the chunk plus terms in the file
  name). A chunk matches when a term is in its text.
- **Relevance floor (OF06):** a matching chunk is sent only when its text holds at least 2
  *informative* terms, or at least one and 40% of the best chunk's count. A term is not informative
  when it is generic (`context_rank::STOP`, `source_evidence::GENERIC`: everyday request words such
  as "tell", "look", "next", "ones", "summarise", and `ATTACHMENT_WORDS`, the words for the
  attachment itself, "file", "document", "attached", "export", "report", "notes"…, even when a file
  is named for one; plus their plurals, and -ing/-ed forms of the first two lists, folded only onto
  words of four or more letters, since retrieval terms have no stemmer) or common (in more than half
  the chunks of some file holding it that has at least 8 chunks, judged per file so a second file
  mentioning the word once can't dilute it; exempt only when every file holding it has it as a word
  of its name, as churn.md does "churn"). Chunks below the floor are recorded in the manifest ("Below the relevance floor…") and not
  sent. 40% keeps a one-term chunk beside a best of two and drops it beside a best of three. Before
  OF06 any one term made a chunk relevant and leftover budget was backfilled with chunks that matched
  nothing, so every run with attached sources filled the 24 KB.
- **Capped fallback (OF06 review):** when the request has terms but no chunk passes the floor (every
  term generic, every specific one common, or none matching), the 3 densest chunks go, within
  8,000 bytes (`FOCUSED_CHUNKS`, `FOCUSED_BUDGET`): densest by lines holding a non-generic term, or,
  when every term is generic, a term outside the stopwords ("review", "summary" are often the
  subject then). Chunks holding none of them follow in document order when every term is generic,
  the request names the attachment ("attached", "sources", "export", "notes", "transcript",
  "report", "pdf", a file's name word), or a term is a whole-document verb ("summarise", "review",
  "read"…: "Summarise it for the board"). So "Why is churn up?" over an export with churn on every page
  gets its densest pages, "Show me something new" gets the opening 8 KB, and an **automation's**
  standing prompt ("Each morning check the attached sources and post a digest") gets a capped fill
  every run, not 24 KB and not nothing. A request with terms that match nothing and doesn't name the
  attachment ("What is the budget ceiling?") still gets pointers only.
- **One 24,000-byte budget:** relevant chunks best first (informative terms, then coverage), a file's
  third and later behind every other file's first two. Overlap with a kept range is trimmed; a chunk
  that doesn't fit is cut to whole lines (lines without a term go first); a piece that fits whole is
  always taken. Name order decides nothing. **No backfill**, except: when the request asks for the
  whole document (in one sentence, a verb, "summarise", "summarize", "summary", "review", "read",
  "go through" or "go over", followed within four words, with only "the", "this", "all", "of" and the
  like between, by "attached", "file", "document", "doc", "source", "export", "notes", "transcript",
  "report", "pdf", "whole", "entire", "everything", their plurals, an attached file's name word, or
  "it", "this", "these", "them" ending the sentence: "summarise this")
  or all attached text fits the budget, every matching chunk counts and the rest fill in document
  order (every file's first chunk, then every second, name matches first), as before; and a file that
  is one chunk (≤ 3,000 bytes, read whole) goes whole when it fits. A request with no terms therefore
  gets only pointers, unless one of those applies.
- **Per file** in `attachedSourceEvidence`: `coverage` (`complete`, `excerpt`, `not_selected`,
  `budget_exceeded`, `requires_reader`), a one-line `note` ("excerpt: lines 120–160 of 400; read_source
  reads the rest", the sent lines only, "present: no passage matched this request (300
  lines, 12000 bytes)", "… matched this request closely enough to send …" when only chunks below the
  floor matched) and `excerpts` `[{from, to, text}]`.
- **The reader:** where `hollow_vault` attaches (Claude and Codex chats with Ask your vault on,
  `mcp_attach::chat_has_vault_reader`), `read_source {path, from_line?, to_line?}` returns numbered
  lines, at most 600 lines or 24,000 bytes per call, with a pointer to the next range. It reads the
  run's workspace folder (`HOLLOW_VAULT_WORKSPACE`; relative paths try it first), vault notes
  retrieval may cite (`vault_eligible`) and connected project files in `HOLLOW_VAULT_SCOPE` (any
  connected project's when it is empty; the tool description says which); a PDF or Office file
  there reads as its extracted text (R14a), attached `sources/` ones included. It refuses
  hidden and secret-looking names, `me/`, `sessions/`, `workflows/`, other workspaces' files outside
  `outputs/`, and any path whose resolved target breaks those rules (a link out of the vault or into
  `me/`). Without it (Antigravity, Grok, automations) the envelope's preamble names the workspace folder
  for the agent's own file tools.
- **Asking by intent (repo features R3):** `read_source {path, query}` (`from_line`/`to_line` are then
  ignored) answers with `source_evidence::query_source` instead of a range. The query follows
  `retrieval_terms`' one rule: words of 3 or more letters or digits outside its stopwords, the first 16
  searched; a query with none is a tool error saying which (too short, or only common words). The reply
  is a header `{path} · N lines · best matches for "q"` (the query echoed to 200 characters), the
  best-matching line ranges (`select` over the whole file, at most 20 MB and 2,000,000 lines, within
  8,000 bytes of text: each `-- lines a–b --` then numbered lines), `-- last 40 lines (a–N) --` (the
  lines after the last one shown, at most 3,000 bytes; a single longer last line is cut), `Not in any
  line: x, y` for searched terms no line holds, `Not searched (only the first 16 words count): …`, and a
  `[More: …]` pointer, all within the range reader's 24,000-byte cap (matches are cut first, and say
  so). A file that fits the budget comes back whole. The reader also reads **this run's scratch
  folder**: `server_entry` always writes `HOLLOW_SCRATCH_DIR` and `HOLLOW_RUN_ID` (empty when the run
  has none; explicit because Codex forwards only a server's own environment), and the server reads it
  only when `scratch::inherited` validates it against its marker and run id. An absolute path that
  resolves inside the canonical folder, or `$HOLLOW_SCRATCH_DIR/…` as written, is readable and shown as
  `$HOLLOW_SCRATCH_DIR/…`; every component, as written and as resolved, must pass `forbidden_component`
  (so the `.hollow-scratch` marker, hidden files and links out stay refused), a file with a second hard
  link is refused, saying so (one into `me/` would otherwise read as scratch), a missing file is "No such
  file" however the folder is written (`/tmp/…` too), and the read is capped at 20 MB as it happens, so a
  log still being written can't grow past it. Every reader file is opened once (`O_NOFOLLOW`,
  `O_NONBLOCK`), and its type, link count and size come from that handle, which is what is read. The
  text goes through `source_evidence::redact_log`, which keeps every line (line numbers match the file):
  a PEM block is masked line by line to its END line or at most 100 lines (a BEGIN inside restarts the
  window), then `claude_runner::redact`, AWS access keys, Slack tokens, and values of keys named like a
  password, API key, secret or `*_TOKEN` (the key at a line's start, after whitespace, `--` or a quote,
  then `=` or a single `:`, so `secret_store::tests::reads_key` and other Rust paths survive). The tool
  description gains the sentence "Also reads this run's scratch
  folder: save long output there (cmd > $HOLLOW_SCRATCH_DIR/o.log 2>&1) and query it." only when the
  folder validated; `SCRATCH_GUIDANCE` is unchanged. The schema grew by 138 bytes (the `query`
  property) or 252 with the scratch sentence, under a 260-byte cap
  (`vault_mcp::read_source_schema_stays_within_its_growth_cap`), since Codex gets every MCP schema on
  every turn. Claude's own tool-results folder is not readable.

The Sources panel's listing (no request) says `complete` only for a file a request gets whole: when the
attached text fits 24 KB together, it fills them as a request with no terms would; otherwise every text
file is `excerpt`. Each `read_source` call shows in the chat as "Read a source" (the file and lines, or
`find “…”` with a query's first 40 characters), and the run manifest counts them (`agentReads`,
`readSourceQueried` for the queried ones).

`search_vault` returns the keyword note list (every word must match) and the pool cut to its `limit`
(default 20), with a `ranking` field (`lexical only: …` or `lexical and semantic`); a lexical-only answer
in a run that also has `hollow_app` (`HOLLOW_VAULT_APP=1`), the empty "No vault notes matched" one included,
says "This search is lexical only" and names `run_action vault.retrieve {"query": …}` "for ranking by meaning
when the app's semantic search is on" (S03; hedged, since the app ranks lexically too when semantic search is
off or not ready, without search terms, or for connected-project passages alone). `vault.retrieve`
returns the pool cut to `limit` (default 8, at most 24) for the calling conversation's or automation's
own scope.

### Connectors (R2)

`connectors::attach(agent, perm, automation, request, session)`:

1. **Eligible** (`connectors::eligible`): none in plan mode and none for an agent without owned MCP. Of
   the rest: enabled, offered to this agent, switched on for chats or for automations, claude.ai
   connectors only for Claude, built-ins only when signed in.
2. **Always** connectors attach.
3. **When relevant** connectors go through `context_rank::select_connectors`, which returns exactly one
   decision per candidate, in order (`ConnectorSelection`):
   - one the request names, by id or name, attaches as `named` and is never sent to Jev;
   - with Jev on, one request carries at most `jev::MAX_CANDIDATES` = 24 of the rest, preselected
     locally: the locally relevant first, then library order. Jev's query is `ConnectorQuery::request`,
     the request alone; the local rules read `local`, which adds resolved antecedent names. A score of 2
     or more attaches (`jev`);
   - the rest keep local relevance as `beyond-limit`. A failed call falls back for every sent candidate
     (`local-fallback`), and a missing score for that candidate (`unscored`), each with its reason;
   - with Jev off: local relevance, meaning the request names it or shares a meaningful word with its
     name or description;
   - a chat turn first tries its conversation's last Jev decision (S28,
     `context_rank::select_connectors_in_session`, in-process, ≤ 256 conversations): reused with no call
     (`cached`, engine `cached`, a `cached` Jev use on the manifest; "Jev (reused)" in Settings →
     Context) when it is under 30 minutes old (counted from Jev's own decision: a ranking `jev::Guard`
     reused is never remembered), keyed by vault path and session key, the inventory (endpoint and each
     candidate's id, name and description) is unchanged, the stored decision covers every candidate,
     the request names no candidate and shares no word with one it left out (a trailing `s` stripped on
     both sides). Only relevance is reused: the library, and with it `auto_approve` and every
     preapproval, is read again each turn. Automations pass no session.
4. Attachments are matched to decisions by id, never position.
5. The Microsoft 365 token is refreshed (`m365_auth::warm`) only when that connector is among the
   attachments, after selection, until the preparation deadline at most (O2; the refresh finishes on
   its own if cut). An unrelated request triggers no refresh.

Contract: [JEV.md](JEV.md#connector-cardinality-contract).

### Skills (V2)

`skill_index.rs` (F9, wave 4 · V2), run in `execute_turn` once the owned servers are attached, from the
person's own words (`asked`, the request `TurnRequest` admitted):

1. **Roots.** The vault's `.agents/skills`, then the shared `~/.agents/skills`; nothing else (not the
   provider roots, not plugins). `.agents` and `.agents/skills` must be real folders, each `SKILL.md`
   must resolve inside its root (a link out drops it), ≤ 256 folders a root, the frontmatter read from
   the first 16 KB. A vault skill wins a name clash.
2. **Selected.** The text Skills → Use in workspace writes (`useSkillPrompt`: `Use the skill "<name>"
   for this task. … Then follow them for this task:`) is parsed out of the request (≤ 3); by its path or
   name in the roots, else, for a provider or plugin skill Skills lists, through
   `skills::read_skill_in` (the registered set). One found nowhere gets a line saying so.
3. **Matched.** The rest of the request's words (≥ 3 letters, filler left out, a plural `s` dropped)
   against each skill: named outright 10, a word of its name 3, a word only in its description 1; at
   least 3 to be listed; ties by root then name. No model and no Jev; the request never leaves the Mac.
4. **The block.** Selected first, then the best matches, ≤ 5 in all (`leftOut` counts the rest); the
   selected skills' text inline while it fits 8,192 bytes together. Each line names how this run reads a
   skill whole: `hollow_vault` `read_skill {name, source?}` (≤ 64 KB), else `hollow_app` `run_action
   skills.read_skill` with its path (which since V2 also reads a vault skill), else the agent's own file
   tools at its path. In the turn's message, never the appended prompt, so a different index never
   restarts a native session or rewrites Claude's cached prefix. Workflows and the heartbeat get none.

### Preparing both at once, under one deadline (O2)

A chat turn (`chat::execute_turn`) and a workflow run (`workflows::prepare_spawn`) run retrieval and
connector selection concurrently (`prep_deadline::concurrently` in a workflow; a chat turn joins
connector selection with the repository snapshot and polls retrieval beside them, then starts a
steerable Claude turn's agent while retrieval finishes: run 15 S2, [Starting the agent beside
preparation](#starting-the-agent-beside-preparation-run-15-s2)): neither reads the other. One
`PrepDeadline` (3 s from the start of that stage) bounds their optional parts: the passage rerank, the
connector call, the semantic half and the Microsoft 365 refresh. When it passes each falls back
locally (Jev → local decisions, outcome `deadline`; semantic → lexical only; the refresh continues in
its own task), and `preparation.deadline.cut` names what was cut. A Jev call is never sent once it has
passed. Required local context (the lexical scan, recall, attached sources, named and "always"
connectors) is never cut. Both Jev calls go through one guard per process (`jev::Guard`): an
identical request reuses its scores for 120 s (`cached`), and three failed calls in a row pause Jev
for 60 s (`circuit-open`); saving Jev's settings resets it. [JEV.md](JEV.md#preparations-two-calls-one-deadline-reuse-and-a-breaker-agent-effectiveness-f8).

### Starting the agent beside preparation (run 15 S2)

Claude Code spends most of a second on its own start-up (its settings, the person's SessionStart
hooks, plugins, its MCP servers) before it reads its first stdin line, and a steerable Claude turn's
prompt is exactly that line (`agent::live_claude_input`), so its command never holds the prompt. Once
connector selection and the repository snapshot have settled (they decide its MCP config and must
precede it), `execute_turn` builds the turn's spec less its prompt (`template`), builds and spawns
its command (`chat::EarlyStart`, adopted into the run's `GroupGuard` at once, like any spawn), and
only then awaits retrieval and composes the message. The first attempt of `drive_turn_attempts`
takes the early agent instead of building and spawning, and writes the prompt to it; a retry or a
failover builds as before. No early start once a stop was requested; a stop while retrieval finishes,
or a failed retrieval, ends the early agent and waits until its process tree is gone
(`EarlyStart::end`, the guard's `settle`) before the turn is finished, and the repository baseline
is armed at the early spawn so anything its start-up changed is captured. Codex and non-steerable
turns are unchanged. On the receipt, Preparation (`spawnMs`) runs to the prompt's hand-over (`drive`
stamps the spawn then) and Model started from it; the first output may come before the prompt and
is read only at the hand-over, so its real time can't be told and First output and First program
output are not reported for such a turn. The run manifest's `spawn` step spans the early start to the hand-over.

### Jev's role and its bounds

Jev (`jev.rs`, TypeSafe `jev-1.13.0`) is optional: a key and two switches (ranking, duplicate check) in the Keychain, both off by
default. Off, missing or failing, Hollow behaves as it does without it.

| Use | Where | Decision |
|---|---|---|
| Connector relevance | `context_rank::select_connectors` | score ≥ 2 attaches |
| Passage reranking | `context_rank::prioritize_passages_via` | score < 1 drops |
| Proposal duplicate check | `jev_decide.rs` (heartbeat and learning proposals) | ≥ 0.85 drops (flags instead when the matched record has values and the proposal brings one it doesn't have), ≥ 0.5 flags |

Transport bounds: one request per use, 4 s for the whole request (in a run's preparation, the 3 s
preparation deadline comes first), no retry, a 64 KiB response limit, a
query of at most 2,000 characters, candidate titles of 200 characters and excerpts of 700, at most 24
candidates (`score_with` errors past that rather than truncating). Jev never picks a model, delegates,
writes tool arguments, grants permissions or sizes context. Its cost is unknown and recorded as `null`.

Whether each use stays is the Jev study's call (E3, decision Q3). `hollow jev-study` runs the judged
fixtures (`scripts/agent-eval/jev-fixtures/`) through these same paths with Jev off and on, against a
loopback fake or, only when the orchestrator says `--typesafe`, TypeSafe, and
`scripts/agent-eval/jev-study.mjs` compares the two with intervals ([JEV.md](JEV.md)). The runner is built
and tested against fakes; the real study hasn't run yet.

## The run manifest (R3)

`run_manifest.rs`: one record per chat turn or workflow run of what prepared its context and why. It is
built on the run's `EvidenceCollector` (`context_mut()`). The receipt carries `contextManifest` (schema,
bytes, SHA-256), and the manifest is the sidecar `{vault}/.evidence-manifests/{runId}.json`, kept and
pruned with its receipt. It holds hashes and metadata only, never prompt, request or passage text. Nothing
decides on it.

**How to read one.**

- IPC `run_context_manifest({runId?, sessionKey?})`, or the `history.run_context_manifest` app action. With `runId` it answers that run's manifest or none; else with `sessionKey`, that
  conversation's newest or none (never another conversation's); with neither, the newest run's. Check that
  the returned `runId` is the one you asked for.
- Settings → Context's "Last request" line shows the newest run's and names the run.
- `layers`: bytes and hash per layer. Compare a turn's layers across runs to see what grew. A chat's
  `environmentBrief` holds whichever brief the surface got.
- `agentReads` (wave 2 follow-ups): the agent's own `hollow_vault` calls, counted from the run's tool
  calls: `readSource`, `readSourceFailed`, `readSourceQueried` (R3: reads with a non-empty `query`;
  0 on older manifests), `searchVault`. Present when the server was attached (zero
  included) or called. `notesOpened` (S26): up to 32 vault-relative `.md` paths the agent opened outside
  its folder (`Read`, `read_source`, shell `cat`/`sed`/`head`/`tail`), first-read order, never a
  restricted folder's, by the name read or its real path (a symlink into one); chat and workflow runs
  (they record the cwd), absent when none. A vault-root chat (its folder is the whole vault) and a run
  with no cwd record none.
- `retrieval`: `scope`, `scopeBasis` (`workspace`, `workspace-disconnected`, `build-workspace`,
  `project-folder`, `git-worktree`, `explicit`, `none`), `workspace`, `engine` (`local`, `jev`, `jev-fallback`), the
  broker's whole pool as candidates (path, rank, local score, semantic score, Jev score, `kept`, `basis`
  and reason; `kept` is false past the 8-passage budget, with that reason), the coverage notices, and
  `stats`: prefilter (`fts`, `fts-read-only`, `linear`), `candidateSource` (`bm25` or `linear`) and
  `candidates` (OB12), vault notes eligible, read and matched, bytes
  read and scored, why the scan stopped, project files read, pool and budget, the semantic state with its
  reason, hits and time, and the broker's own `ms`. On a chat turn, `queryBasis` (OB18, OB25):
  `request`, `hostSubject`, `followup+prior`, `skipped-host` or `skipped-followup`; absent on other
  runs, a bare turn and older manifests. `citedChecked` (S26): the final reply (a chat's assistant
  text after its last tool call, S27's `reply_start`; a workflow's output) was read, and each kept
  candidate it names gets `cited: true`: its path, or its file name with or without `.md` when that is
  4+ characters (a generic name such as `README`, `plan`, `index`, `summary` or a date-shaped one only
  with its `.md`), case-insensitive, whole word (`run_manifest::cites`). Flags only, never the reply.
  Separate from the receipt's `cites` (repo features R5, `cite_check.rs`), which checks each `path:line`
  the same final reply names against the file and the run's reads ([vault.md](reference/vault.md)). It is
  recorded after the run and sent to no agent: 0 prompt bytes.
- `attachedSources` (S2): the 24,000-byte budget and what was used, the `reader` (`read_source` or
  `own-tools`), the request's term count, bytes searched, chunk counts, whether the 128 KB envelope trim
  dropped the excerpts, each file's coverage, lines, sizes and included bytes (hashed), and each kept or
  matching chunk's path, line range, score, `kept`, bytes and reason (at most 96; the rest counted).
- `lifecycle` (S3): `history` and the `cue` that made the request historical (one of Hollow's own words
  or `a past date`, never the request's text), and per decision the `route` (`recall` or `retrieval`),
  `path`, `status` (`superseded`, `current`, `duplicate`, `unapproved`), `action` (`excluded`,
  `historical`, `promoted`, `current`, `collapsed`, `labelled`), the `current` version and the reason;
  at most 32 per run, the rest counted. Absent when nothing had a lifecycle.
- `connectors`: one decision per "when relevant" candidate, with basis and reason.
- `skills` (V2): the turn's skill index: `reader` (`readSkill`, `appAction`, `ownTools`), `available`
  skills in the roots, `indexed` (name, `source` `vault`/`shared`/`installed`, `reason`
  `selected`/`named`/`match`, `score`, `bytes`, `inline`), `leftOut`, `selectedMissing`, and `read`: the
  agent's `read_skill` and `skills.read_skill` calls from its tool calls, each with the indexed `name` it
  read (absent for another), `via`, `ok` and optional proven `source`.
  Name-only/default reads leave source unknown; an explicit source or matched
  indexed path can prove it. Shared-skill lesson capture refuses an unknown
  home and never substitutes a vault/provider namesake. Names, homes and sizes
  only, no stored skill path or text. Absent when the turn had no index.
- `jev`: one entry per use with outcome (`ok`, `timeout`, `malformed`, `error`, `off`, and since O2
  `cached`, `deadline`, `circuit-open`), items, latency, `costUsd: null`. Since O3 a beat's duplicate
  check is one too (use `proposals`), with its report in `dedup`.
- The view (D1, F10) adds `delegations`: each bounded consultation and durable child task the run
  started (`kind`, `tool`, `agent`, `childSession`, `packetBytes`, `packetSha256` (empty when a child
  task carried no packet), `constraints`, `sources`, `fromTaskState`, `omitted`, `budgetSeconds`,
  `budgetUsd`, `deadlineSeconds`, `at`, since D2 a child task's `childQueueId`, and since OF04 a
  finished consultation's optional `outcome`, `durationMs`, `returnedBytes` and `incomplete`, absent on
  older rows: no migration), from the session store (outside the vault; written by the GUI
  for a child task and by `hollow team-mcp` for a consultation; at most 32 per run, kept 90 days). A
  child's own manifest measures its packet as the `taskPacket` layer.
- The view (D2, F10) adds `costRollup` `{ownUsd, descendantsUsd, totalUsd, knownUsd, unknown,
  descendants}`, read when the view is read: each descendant's `kind` (`consultation`, `childTask`,
  `helper`), `agent`, `status`, `costUsd` (`null` when unknown or still running), and a child's
  `childSession`, `childQueueId`, `childRunId` and the `stop` its parent's stop gave it (from the receipt,
  which seals the same roll-up as `descendants`). A layer `childFinished` measures the
  `<hollow-child-finished>` note.
- The view (O3) adds `afterRun`, the run's duplicate checks that ran after its receipt (a chat turn's
  learning proposals; from memory, so lost on restart), and `jevCalls` `{sent, notSent, latencyMs,
  outcomes, costUsd: null}` over `jev` and `afterRun`: a run's Jev use in one place, which the bed row
  `ae-jev-calls` checks against the requests the fake Jev received.
- `capabilities`: owned servers and library connectors given, and `bashOutputMaxChars` (OB23: 16000 on a
  Claude run that has Bash, absent otherwise), `compactWindow` and `compactions` (OB22, below; absent when
  off or none; counted for chat and automation runs on Claude-format streams only). `fallbacks`: Jev failures, partial scores
  and a fresh native session. `preparation`: `retrievalMs`, `connectorsMs`, and `timeline` (O1, F8,
  `prep_timing.rs`): `totalMs` (admission's start → the spawn) and one `{name, startMs, durationMs}` per
  step, offsets from admission's start, both `null` for a step that didn't run: `admission` (the vault
  snapshot), `recall`, `attachedSources`, `retrieval` (`retrievalSemantic`, `retrievalJev` inside),
  `connectors` (`microsoftAuth`, `connectorsJev` inside), `contextCompile`, `promptAssembly`,
  `repoSnapshot`, `command` (`codexInventory` inside), `spawn`, `firstStdout`, `firstText`. The ones that
  ran come first, by start. `scripts/agent-eval/prep-bench.mjs` gives p50/p95 per step over a folder of
  manifests or the deterministic bench's samples (`prep_bench.rs`). Since O2 `preparation.deadline`
  `{budgetMs, cut}` lists what the preparation deadline cut (`retrievalJev`, `connectorsJev`,
  `retrievalSemantic`, `microsoftAuth`); retrieval and connectors start together. Since S29 a chat turn's
  `repoSnapshot` (`repo_diff::begin`) starts with them too, a third branch of the same join on its own
  pre-run budget, so it is listed before `promptAssembly`; its baseline is still armed only after the
  spawn. A workflow run still snapshots after its spec. Since O3
  `preparation.codexInventory` is `listed` (`codex mcp list` ran) or `cached` on a Codex run.
- `compiler` (chat turns, F4): `entry` (`composer` or `appSession`), `session` (`new`, `resume`, `reset`
  with `resetReason`, `retry`), `profile` (`enabled`, `sent`, the digest's `bytes` and `sha256`, which is
  its revision and is recorded even when not sent, `files`, `truncated`) and `continuity` (`sent`,
  `available` entries, `entries` kept, `bytes`, and `continuedFrom` and `seeded`: the earlier
  conversations a continued one read and the entries they gave), `taskState` (`revision`, `throughTurn`,
  `items`, `sent`, `bytes`, `merged`: folded every turn, sent on a fresh session, and `reground`
  `compacted`/`refresh` when a resumed one got it again, repo features R1, layer 11) and
  `profileDelta` (`baseline` `fresh`/`stored`/`adopted`/`off`, `sent`, `added`, `removed`, `files`,
  `bytes`) and `visualGuide` (G1: `delivery` `prompt`/`turn`/`given`/`onDemand`/`off`, the `cue` in Hollow's
  own words, whether the scratch `file` was written, the guide's `bytes` and `sha256`), `budget` (G2:
  `provider`, `total`, `configuredChannel` `systemPrompt`/`prompt`, `used`, `over`, and per layer `name`,
  `tier`, `wanted`, `granted`) and `stableLayers` (G2: `configured` `systemPrompt`/`full`/`held`,
  `workspace` `none`/`full`/`delta`/`child-slim` (S01, above), the `reason` a resumed turn went full: `noBaseline`, `otherSession`,
  `refresh`, `changed`, `budget`; `fullBytes`, `sentBytes`, `unchanged`, `changed`, `excerptsReferenced`,
  `resumedSinceFull`, and `heldLayers`, the per-turn layers a resumed turn left out as given, S02). A
  layer left out is no layer of that turn's manifest. A `configuredHeld` layer replaces the appended prompt's four when the prompt
  carried the reference. Layers
  `taskState`, `operatorUpdate` and `visualGuide` are measured like the other composed blocks, and the
  appended prompt's `capabilityIndex` like its other layers.
- `nativeMemory` (S13, chat turns and workflow runs on Claude; absent for other agents and older
  manifests): what Claude Code loads by itself, outside Hollow's envelope and so in no layer. Hollow passes
  no `--setting-sources` and never overrides `HOME` or `CLAUDE_CONFIG_DIR` (`agent::isolate_claude`;
  accounts share the config folder), so all of it loads into every main session and subagent.
  `configDir` (`CLAUDE_CONFIG_DIR`, else `~/.claude`); `instructions`, each `{scope, path, bytes}` in load
  order: `user` (`<configDir>/CLAUDE.md`), then from `/` down to the cwd's real path each folder's
  `CLAUDE.md` and `.claude/CLAUDE.md` (`project`) and `CLAUDE.local.md` (`local`), with
  `instructionBytes` their sum (`@` imports and `.claude/rules/` not followed); `autoMemory` `{dir,
  exists, memoryMdBytes, memoryMdLines, notes}` for `<configDir>/projects/<slug>/memory`, keyed like
  Claude Code by the project root, not the cwd (`run_manifest::claude_memory_root`): walking up the cwd's
  real path to the first `.git`, a `.git` folder's parent, or for a linked worktree (`.git` file →
  `gitdir:` → `commondir`) the main working copy, so all worktrees of a repo share one folder; as
  Claude Code does, a bare repo's worktrees key the common dir itself (`repo.git`, `.bare`) and a
  worktree whose gitdir isn't under `<common>/worktrees` or whose back-pointer (`<gitdir>/gitdir`)
  doesn't name its `.git` keys its own folder; the cwd when there's no `.git`. The slug is that path with every character but an ASCII letter or digit as `-`
  (`run_manifest::claude_project_slug`); `notes` the other `.md` files in it. `memoryMdBytes`/`Lines`
  are the whole `MEMORY.md`, though Claude Code loads only about its first 200 lines / 25 KB. With a
  personal context root set, a local Claude turn carries `--settings {"autoMemoryDirectory":"<personal clone>/memory/<slug>"}`
  (CX4b, `context_layers::auto_memory_dir`, merged into the self-hosted settings, never a second flag), and
  `dir` names that folder; a runner turn, a turn with no usable root and a per-run scratch working folder (cwd under
  `$TMPDIR`, `/tmp` or `/private/tmp`, so no `hollow-run-*` memory folders) keep the default. **Decision: no copy.** Once
  the key is set the CLI no longer loads the old `~/.claude/projects/<slug>/memory` notes of that project; nothing is
  copied (the 32 flat `memory/*.md` files stay in `personal-context`), a seed is a follow-up. Two paths differing
  only in punctuation share one slug, as they do for the CLI. Not modelled: a
  person's own `autoMemoryDirectory`, `CLAUDE_CODE_REMOTE_MEMORY_DIR` and the settings that turn auto memory off. And `skillListing` `{count, bytes}`, user skills only: the
  folders under `<configDir>/skills` holding a `SKILL.md` (symlinks followed), and their frontmatter
  `name` + `description` bytes summed (the first 500 read; any past that counted without bytes).
  Project skills (`.claude/skills`), plugin and built-in skills aren't counted. Paths and sizes only;
  recorded when the command is built (`set_native_memory`).
  The receipt's `memoryWrites` counts the run's completed, non-error `Write`/`Edit`/`MultiEdit` calls whose
  `file_path` is inside `autoMemory.dir` (present, 0 included, whenever `nativeMemory` is).
- `recallMeaning` (OF26, chat turns and workflow runs): whether recall ranked global learnings by meaning
  (`vault_learning::RecallMeaning`): `used` (a query vector was read), `skipped` (a bare turn or a
  skipped basis embeds nothing), `timedOut` (no cosines within the wait: `RECALL_MEANING_WAIT`, 100 ms since J4 (11-08; it was
  250), or `RECALL_SLOW_WAIT`, 25 ms, after three late recalls in a row (`RecallPace`); or an earlier
  recall's thread still running; or, for a chat turn since J3a, its prefetch not ready within its cutoff; stems only, and the embedding runs on beside the turn) or `noVector` (nothing to embed, or the embedding failed). Absent when semantic search is off,
  the model isn't installed, and on older manifests (an index that isn't ready yet still records `noVector` or
  `timedOut`: the check before the wait takes no lock).

**Limits.** No manifest for ingest, teammates or one-shots (the heartbeat has one since O3, without
`nativeMemory`; a
teammate's consultation is listed on its parent's view since D1, its own context isn't measured). The adapters' workspace policy
and permission paragraph is not a layer. A chat's layers are in reading order. A workflow's are in
preparation order (`request`, `retrievalEvidence`, `operatorProfile`, `recall`, `workspace`,
`connectorsNote`, `policy`), although `RunManifest.layers` says reading order. A workflow's `policy` is its
boundary and tool blocks. `chatMode` appears only for a legacy `<hollow-chat-mode>` block.
A chat turn's learning check lives in this process's memory only (the view's `afterRun`), not in the
sealed manifest.

Host capture waits for ten idle minutes, four successful attended turns,
a handover or a near-compaction boundary (`learning_capture.rs`); an
explicit remember, correction or decision is due at once. Its pending
input is in memory: the newest finished turn, kept apart from the turn in
flight, which a cancelled, failed or unstarted turn ends. A sleeping self-hosted model keeps an optional
`capture_session` on its queued Learning job, coalesced by vault and chat;
the drain reconstructs bounded grounding from the host transcript.
Exact positive Know-how grounded in the person's own messages can save
automatically with a logged host-capture actor and durable Undo; a tool-result
quote, a negative statement, a quote from a negative sentence of the person's
message (or from a list under a negative lead-in such as "Do not:") or an
email address waits for Save. You and Project rules,
private-memory promotions and edits of existing facts stay pending for Save.
The review home is Library → Learnings, with a line at the source reply.

`turns.capture_scope` records eligibility and privacy at turn start. It is
`cloud:<provider>` or `private:<entry id>` only for an admitted turn. Grounding
requires the source scope and excludes unmarked history and different scopes;
private models can share a CLI agent with cloud models, so agent names alone
never establish that boundary.

## The permission wording contract (C2, Q1)

Every sentence an agent reads about asking, cards or preapproval is generated in `mode_contract.rs` from
the mode and the agent's capability. Behaviour is decided elsewhere, and tests pin the words to it:
`app_actions::asks_in_mode`, `computer_mcp::server_entry`, `connector_preapproved`,
`connectors::eligible`.

| Mode | Policy paragraph (`policy`) | Hollow app cards (`app_cards`) | Library connectors (`connector_sentence`) | Computer tier 2 |
|---|---|---|---|---|
| plan | read and plan only | none (read-only tools) | none attach | asks |
| default, acceptEdits | the run is headless, so what the CLI must approve is refused, not asked; ask only for a shared connector's external effects; then the forms that pass (`HEADLESS_FORMS`: one command per call, `git -C` instead of `cd X &&`, no `$VARS`, no pipes into interpreters, suggest changes outside the workspace) (OB02) | every confirm and external action | the auto-approve ones run; the rest are refused | asks |
| auto | approval is Hollow's or the CLI's job; ask only for a shared connector's external effects (unchanged by OB02) | only the sensitive list: external actions, `AUTO_ASK_CLASSES`, sensitive settings, a session in a stronger mode | none preapproved; each call goes to the agent's own check | asks |
| bypass | approved up front, don't stop to ask | no approval or confirm cards (question cards and task suggestions still show), except `app_actions::ALWAYS_ASK` (`gardener.apply`, `records.records_supersede_decision`, `records.records_set_decision_status`, `friction.draft_fix`, since P0-27 `build.install_built_app`, since the 11-10 sweep `settings.cfo_approve_extra_usage`, since G1 `workspaces.edit_standing_orders`, since U8 `build.publish_main`, since SK1 `skills.approve_repo_skill` and `skills.set_repo_owner`, and since OC15 `build.approve_setup_hook`) | all run | runs at once |

Every chat mode keeps `EXTERNAL_CONFIRM` in the connectors note: before a shared connector sends,
publishes, deletes or spends anything outside this Mac, ask the person and wait (Q1, 2026-10-06). An
automation says it does that only when its own prompt asks for exactly that. Agents without owned MCP get
the "Hollow can't attach its app tools, approval cards or shared connectors…" form (their chats still post
question cards through a fenced `hollow-questions` block). The generated text per mode is checked in at
`tests/fixtures/prompts/mode-wording.json`. Outside plan a chat reads its mode wording only in the
turn's `<hollow-mode>` line (HB01), so changing it (as OB02 did) restarts no chat session; it does change
`agent::workspace_policy`, which other runs (automations, teammates, the heartbeat) carry.

Creating a task or project in Notion (`notion.create_notion_task`,
`notion.create_notion_project`, N3a/N3b) was removed with Notion sync on
2026-11-10 ([history](history/NOTION_SYNC.md)); new tracked work goes through
the tracker ([Tracker sync](TRACKER_SYNC.md)).

## Tools per provider (the contract)

The per-provider capability contract (F9, wave 4 · V1). The table below is generated from one table in
code, `capability_contract.rs`, which is the source of truth: a test fails when this block differs
(`the_doc_holds_the_contract_table`; regenerate with `HOLLOW_REGENERATE_CONTRACT=1 cargo test --lib
capability_contract` from `src-tauri/`), and another checks the cells that restate a runtime decision
against the code that makes it (owned MCP, connectors, tool restrictions, modes, the verified Codex
versions). Each chat's capability index ends with its agent's line from the same table: what this agent
doesn't have in Hollow and the fallback (`capability_contract::guidance_line`), for example Antigravity
and Grok are told that Hollow can't attach its tools or connectors to them, to use their own file tools
in the folders the brief names, and to ask the person for the rest.

<!-- capability-contract:begin (generated from src-tauri/src/capability_contract.rs; edit the table there) -->
| | Claude | Codex | Antigravity | Grok |
|---|---|---|---|---|
| Owned MCP (`hollow_questions`, `hollow_tasks`, `hollow_app`, `hollow_vault`, `hollow_code_context`, `hollow_computer`, `hollow_team`, `hollow_services`), per surface as above | yes: private `--mcp-config` file, `--strict-mcp-config` | yes: each stdio server through `hollow mcp-launch`, `-c mcp_servers` overrides | yes in a chat, over `agy acp` (`session/new` `mcpServers`, per chat, never agy's own config); a one-shot runs `agy -p` and gets none | no (no per-run MCP flag, 1.0.46) |
| Preapproval of owned tools | `--allowed-tools mcp__<server>` (`hollow_computer`, `hollow_services` only in auto and bypass) | `default_tools_approval_mode="approve"`, same rule; `hollow_app` gets a longer tool timeout | none needed: each call asks (an approval card in a chat); bypass starts agy with `--dangerously-skip-permissions` | n/a |
| Hollow app tools (`hollow_app`: workspaces, sessions, automations, `list_actions` / `run_action`) | yes, while Settings → Connectors → Built into Hollow → Hollow app is on (read-only in plan) | same | yes in a chat, as Claude | no: ask the person, or use its own file tools |
| On-demand reader for attached and retrieved files | `hollow_vault` `read_source` (while Ask your vault is on): by range or by `query`, and the run's scratch folder too | same | same, in a chat | no: the envelope names the workspace folder for its own file tools |
| Library connectors | yes, claude.ai ones included | yes, not claude.ai | yes in a chat, not claude.ai; the person shares each connector with Antigravity | no |
| The CLI's own MCP servers, apps and plugins | its MCP servers excluded (strict config); its own skills and plugins are Claude Code's, not managed or verified by Hollow | each configured MCP server disabled (named by `codex mcp list`, cached per binary, config and folder); `apps`, `plugins`, `remote_plugin` disabled | its own global configuration still loads, not managed by Hollow; Hollow's servers are per chat and never written there | `~/.grok` servers still load |
| Question cards | `hollow_questions` in chats | `hollow_questions` in chats | `hollow_questions` in chats | a fenced `hollow-questions` block at the end of a reply |
| Hollow confirm cards (app, computer) | per mode | per mode | per mode, and an approval card for each request agy itself asks | none |
| Skills | a turn's `<hollow-skills>` block lists up to 5 skills from the vault's `.agents/skills` and `~/.agents/skills` that fit its request (a skill chosen with "Use in workspace" first, its text inline up to 8 KB; `skill_index.rs`), and `hollow_vault` `read_skill` reads one whole (else `hollow_app` `skills.read_skill`). Claude Code may also load its own `~/.claude/skills` and plugin skills (not verified per run) | as Claude; Codex may also load its own `~/.codex/skills` (not verified per run); plugin skills don't load (plugins off) | the same block, so a selected skill's text arrives inline when small; any other it opens at the listed path with its own file tools, which its mode may refuse outside the working folder (then the person pastes it) | same as Antigravity |
| Visual guide on demand (G1) | `$HOLLOW_SCRATCH_DIR/hollow-visual-guide.md`, an `--add-dir` folder | the same path (`HOLLOW_SCRATCH_DIR`), read with its own tools | `--add-dir` folder, as Claude | the same path, as Codex |
| Workflow tool restrictions | yes | refused | refused | refused (only the `osascript`/`open` pair, as `--deny`) |
| Tool-free mode (one-shot helpers) | `--tools "" --disable-slash-commands` (`claude_runner::NO_TOOLS`) | `codex_transform.rs` on a verified codex-cli (0.146.0, 0.153.4, 0.162.1), its version read once per binary (`codex_probe`); any other version doesn't run and says why (Settings → Models, the usage row's note) | none verified | none verified |
| Heartbeat | tool pass | tool-free snapshot | refused | refused |
| Permission modes | all | all | all; its own reviewer in auto | all; its sandbox is the limit |
| Reads outside its folders in default and accept edits (OB02) | refused, not asked (headless `-p`): only the cwd and its `--add-dir` folders (scratch, a build workspace's vault folder), which the turn's `<hollow-mode>` line names | yes: its sandbox limits writes, not reads | not verified; its mode may refuse outside the working folder | not verified; its sandbox is the limit |
| Delegates through `hollow_team` (bounded consultation) | yes (not in plan) | yes (not in plan); the tool waits 240 s (D1; was Codex's 60 s default) | yes in a chat (not in plan) | no |
| Starts durable child tasks (`hollow_app` `start_session`) | yes (writable app tools, depth < 2) | yes (same) | yes in a chat (same) | no |
| Waits for a child task (`hollow_app` `wait_for_session`, D2) | yes (≤ 300 s; no shorter client timeout) | yes (≤ 300 s; the server waits 315 s, inside its 480 s tool timeout) | yes in a chat (same) | no |
| Budget metering mid-run | yes | no | no | no |
<!-- capability-contract:end -->

**Claude's Bash output cap (OB23).** Every Claude run that has Bash (chat, delegate, automation) gets
`BASH_MAX_OUTPUT_LENGTH=16000` in its env (`agent::CLAUDE_BASH_MAX_OUTPUT_CHARS`, set in `build_claude`); a
tool-free run (`--tools ""`), one that disallows Bash (the heartbeat) or whose `--tools` list leaves it out doesn't.
Claude Code keeps a Bash result inline up to that many characters (its default is 30,000) and re-reads it on every
later call. Past the cap it gets only the first 2 KB plus the path of a file holding the whole output, so the tail
(a test runner's failure summary) is dropped until the agent reads the file; the file is readable in acceptEdits
too. Probe: `scripts/agent-eval/cli-probe.mjs bash-output` ($0: temp config dir, dummy key, local stub, tripwire
proxy). The manifest records the cap as `capabilities.bashOutputMaxChars`. It is env, not a fingerprint input, so it
resets no native session.

**Claude's fixed run settings (NV9).** Every Claude run (chat, delegate, automation, teammate, heartbeat tool pass,
runner run) gets `CLAUDE_CODE_DISABLE_BACKGROUND_TASKS=1` and `CLAUDE_CODE_DISABLE_REFUSAL_FALLBACK=1` in its env
(`agent::apply_claude_run_settings`, at the end of `build_scoped`'s env step, after `RunExtras::env`, so no extra can
turn either off). No setting: a deterministic default, because a toggle would put two runs of one prompt on
different footing. The first stops the CLI's `run_in_background` shells and agents, which it kills at exit anyway
(`evidence::stopped_background_task` reads that notice), so work stays in the turn that owns it. The second stops a
refused turn silently retrying on another model, so the usage ledger names the model that answered and the refusal
reaches the run. The runner path needs no second edit: `remote_session::into_leader_with` copies the command's env
(minus `LOCAL_ONLY_ENV`) into the leader's header, so the mini gets both. Probes (`--help`, the model list, the
effort check) build their own commands and carry neither; Codex, Grok and Antigravity get neither. Checked on Claude
Code 2.1.325 by reading the installed binary (`strings`): both names are read as env switches
(`CLAUDE_CODE_DISABLE_REFUSAL_FALLBACK` gates the model switch, `CLAUDE_CODE_DISABLE_BACKGROUND_TASKS` the
background launch rules); the separate `CLAUDE_CODE_DISABLE_REFUSAL_RETRY` is left alone. Behaviour on a real refusal
is not exercised (it costs a model call). Env only, so it resets no native session.

**Compaction window (OB22).** `AgentSettings.compactWindow` (`agent-settings.json`): absent is off, the default;
otherwise 300000, 400000 or 600000 tokens (`agent_settings::COMPACT_WINDOWS`). Any other size saves as off (logged;
`set_agent_settings`' confirm card shows it as off too) rather than failing the save, since every save path sends the
whole object and a hand-edited file would otherwise block every agent setting; `agentSettingsPayload` likewise sends
only an offered size, and a run ignores any other. When set, chat turns, automations, delegates and the heartbeat's
tool pass get it through `RunExtras::env` (`agent_settings::compaction_env`): Claude as
`CLAUDE_CODE_AUTO_COMPACT_WINDOW=<n>`, Codex as `-c model_auto_compact_token_limit=<n>` before `exec` (codex-cli
0.163.0), carried by the Hollow-only `HOLLOW_CODEX_AUTO_COMPACT_LIMIT` entry that `build_codex` reads and keeps off
the child's env. Antigravity and Grok get nothing. Env and argv only, so it resets no native session. Two caveats
before J07 turns it on: the Claude variable's name comes from the item spec (Claude Code 2.1.301) and has not been
probed, so confirm it with the $0 request-capture probe first; and Codex's key is a token-count trigger, not a
window, sent as is (Hollow knows no Codex model's context size, `context_gauge::context_window`), and it replaces the
model's default limit, so an oversized value may turn off Codex's proactive compaction (unverified against codex-cli
0.163). The manifest records the window as `capabilities.compactWindow`. The run's `compact_boundary` events
(`ParseState::compactions`, carried across a quota failover) are the receipt's `compactions` and
`capabilities.compactions`, absent when none. They are counted only where Hollow parses a Claude-format stream
(Claude, and Grok, which shares the parser) in a chat turn or an automation run; heartbeat and delegate runs
(`run_collect`) and Codex record no count. A context drop after a boundary is not read as a reset: `context_tokens`
is the post-compaction message's. The receipt shows the count as a "Compacted · N time(s)" row (repo features R1;
`compactionsLine`), and in a chat the compaction re-grounds the next resumed turn with the `<task-state>` record
(layer 11; the composer's What your last message used shows "Task state · Re-sent after the agent compacted").
No Settings control yet (S30); turning it on is Alex's call (J07).

**Context gauge and growth steps (OB11, repo features R2a).** `context_gauge` keeps each running chat turn's own
context reading by history run id: `chat::drive_turn_attempts` registers it (`register`) and publishes the ledger's
`context_tokens` (the main thread's last call; a subagent's never moves it) and call stats at every `MessageUsage`;
the guard removes the entry on every return. `hollow_app` `run_action sessions.context` (read, not first-class, 0
schema bytes) answers the calling turn's `contextTokens`, `contextWindow` (Claude only: 1M for a `[1m]` model or a
turn past 200K, else 200K; Codex the window its rollout reported; other agents `null`), `advisoryTokens` (350000,
the hand-over point, on a 1M window only), `pastAdvisory` and `calls`, or `live: false` outside a running chat turn.
Antigravity's stream gives no figure; Codex's figure comes from its rollout file (below). On top of that read, the
gauge tells a turn once per **growth step** (300K, 350K, 400K, then every 100K, `step_of`; on a window under 1M
that the agent reported itself, Codex's, 70% and 85% of it instead, `step_for`; Claude's window is inferred from
the model id, so Claude keeps `step_of`): when a chat caller's `hollow_app` call ran, the broker (`app_broker::answered`) adds a `note`
from `take_note` to its answer, and `app_mcp` sends it as a second `{type:"text"}` block after the tool's own result
(`parse_app_answer`; the `SocketClient` holds it until the next result that finishes, since calls run on their own
threads). The note is up to two lines, each at most 200 B (both together can be longer): `note_line` ("[Hollow:
this conversation's context is at 412K of 1M. Main thread on a long job: finish the step and hand over
(sessions.handover). A subagent: just mention it in your result.]", the window clause only when known) and, once
per compaction the stream reported (`GaugeGuard::set_compactions`, `ParseState::compactions`), R1's pointer
`COMPACTED_NOTE` at `sessions.task_state`. The step line addresses the conversation's main thread because a Claude
Code subagent shares the parent's MCP connection, so a subagent's `hollow_app` result may be the one that carries
it, and a subagent must not hand the parent's work over. The step told is stored on the conversation as the
`derived_state` row of kind `contextSteps` (`{announced}`; `store_told` writes the live reading's value under one
lock, so a slower parallel call never writes an older step), and the next turn's `register` is seeded with it
(`announced_step`), so a resumed turn already past 300K doesn't announce its step again on its first call; each
reading lowers it to its own step, which re-arms the steps after a compaction. A step is marked told when the broker
builds the answer, before it is delivered: an answer the client never reads loses that step's note (the next step
still fires). Plan-mode, bare scratch and unattended turns (the night queue's, or a COO wake or check-in,
`main_agent::unattended::is_unattended_run`; `chat::gauge_quiet`) are quiet: no note. Tool-result bytes only, sent only past a step: 0
appended-prompt and 0 schema bytes. **Stored figure (R2b):** the reading dies with its turn, so
`chat::finish_observed_run`, after `ledger.finish`, writes the turn's last `context_tokens` and its window
(`context_window(agent, usage.model, tokens)`) into the same row (`chat::store_context_figure` →
`store_figure`: `{tokens, window, announced}`). The told step is the one the turn's reading ended holding (the guard
leaves it behind when it drops; the row's only for a run that never registered), lowered to the figure's own: a
compaction lowered the reading even when no later call carried a note, so the steps re-arm. Quiet turns store it too.
Antigravity has no figure, so it stores nothing and is never told. **Codex (overnight 9 A10):** `codex exec --json`
streams only the turn's summed usage and no compaction event, so `codex_rollout::Watch` reads the thread's rollout,
`$CODEX_HOME/sessions/YYYY/MM/DD/rollout-…-<thread id>.jsonl` (`CODEX_HOME` defaults to `~/.codex`), found by the
`thread.started` id and read then as the baseline (only its tail, since a rollout reaches tens of MB and this runs on
the stream loop: the last MiB from its first whole line, widened 2, 4, 8 MiB while it holds no figure or no model;
RF-812-F1). Later it reads what was appended since its offset (at each
completed item, and once more after the process exits) and parses only lines whose top-level `type`, found by bytes
in the line's head, is `event_msg` naming `token_count`, or `compacted`, or `turn_context` (only its model id is kept, for
A13's Codex price estimate: `Watch::model` → `UsageRun::set_turn_model`) (never the transcript); it gates on each line's own shape,
not `codex_probe::VERIFIED`: a `token_count` counts only with a numeric `last_token_usage.input_tokens` and
`model_context_window`, a `compacted` only with an object payload; anything else is ignored. At each read (the
thread's start, every completed item or turn, and after exit) the figure goes to the gauge (`publish_figure`) and the ledger (`UsageRun::set_context_figure`, so
`RunUsage` carries `context_tokens` and `context_window`, the `usage` event's `contextTokens`/`contextWindow`, and
the composer's ring shows "231k/258.4k"); compactions written during the turn add to `ParseState::compactions` and
set `compacted` (`chat::apply_codex_rollout`), so the receipt counts them, the in-turn `COMPACTED_NOTE` fires and R1
re-grounds the next resumed turn. What the file held when `thread.started` arrived sets the figure but not the
compaction count; a new thread's file that appears later is all this turn's. The stored row keeps `reported: true`
for a Codex figure, so the next turn's `<hollow-context>` line uses the same window-relative steps. The next resumed turn reads it at its
start (`<hollow-context>`, layer 7b; `start_note` stores the step before the gauge registers, which is what keeps the
two channels to one announcement per step); that is the only channel for Grok, which has no `hollow_app`. A fresh
native session (a new or reset plan in `turn_context_note`, or a failed resume's fresh retry in
`drive_turn_attempts`, which also drops the first prompt's `contextNote` layer from the manifest) clears the row to
`{announced: 0}` (`reset_steps`). Best effort: a step is marked told when the prompt is composed, so a turn stopped
before its spawn loses that line (the next step still fires).

**Codex helpers.** The tool-free transform runs only on a codex-cli version whose boundary Hollow's
local check proved: the ignored `installed_codex_advertises_zero_tools_to_local_responses_fixture` test
runs the installed CLI against a loopback Responses fixture and checks that its request advertises no
tools, for the default, an explicit and a custom model, before any inference (no provider call).
`codex_probe::VERIFIED` holds 0.146.0, 0.153.4 (since 2026-10-07) and 0.162.1 (since 2026-10-14, night
ST12, which added a second local check: the fixture answers and the CLI's real `--json` output must parse
through the helper, `installed_codex_json_output_parses_from_a_local_fixture_answer`). `codex_probe` reads the installed
CLI's version once per binary (canonical path, size, modification and change times, inode, and the npm
`package.json` beside a node wrapper; a failed read is never kept). Any other version, a missing CLI or
an unreadable version stops the helper before a model turn with one sentence naming the version, the
verified ones and the fallback: nothing ran, and Hollow doesn't switch agents by itself (choose Claude
Code as the default agent, or install a verified Codex). The usage row keeps it as its note, and
Settings → Models shows the same sentence (IPC and app action `codex_helper_status`). A newer
CLI is verified by running that test against it and adding its line to `VERIFIED`.

### Owned MCP for Grok and Antigravity: not built

Neither CLI takes Hollow's servers today, and building that transport is a separate decision (the plan's
out-of-scope list). What it would take, per CLI:

- **Antigravity (agy 1.2.15).** No per-run MCP flag: `agy mcp add` writes its one global configuration.
  Injection would mean writing Hollow's servers into that file for the run and removing them after, or
  a per-run config home if a later agy gets one. Permissions: agy has no tool allowlist, so the owned
  servers' preapprovals (`--allowed-tools` / `default_tools_approval_mode`) have no equivalent; its
  modes would decide each call, and headless agy denies whatever its mode would ask, so in Edits only
  every Hollow tool that asks would fail. Risks: a global file shared with the person's own agy
  sessions (a crash leaves Hollow's servers, with their run grants, in it; two Hollow runs race on it),
  the run's broker proofs would sit in a file other processes read, and its file tools already ignore
  the working-folder boundary (why it has no `auto`).
- **Grok (Grok Build 1.0.46).** No per-run MCP flag either; it loads `~/.grok` servers. Injection would
  mean a temporary `GROK_HOME`-style config root (not verified to exist) or editing `~/.grok`, with the
  same shared-file and crash risks. Its permission modes take Claude's names, but there is no verified
  per-tool preapproval in Hollow's tool names, so each owned tool's approval would rest on the mode and
  the OS sandbox (`read-only` / `workspace` / `off`). Risk: `~/.grok` servers keep loading beside
  Hollow's, so the run would not be isolated the way Claude's `--strict-mcp-config` and Codex's disabled
  inventory make it.

Either would need a live probe matrix (as the 2026-10-02 permission probes) before anything ships, and
the per-mode wording (`mode_contract`) would gain a third form. Until then the contract's fallbacks
above stand.

## Known gaps, by wave

Wave 4 · D1 named F10's two contracts and gave each child a task packet, and D2 reported a child's end
without polling, aligned every wait with its caller's tool timeout, carried a stop to the child turns and
rolled descendant cost up to the parent (below).
Wave 1 fixed F1, F2, F5, F6 (wording), F11 (first slice). Wave 2 · K1 gave F4 one context compiler, and
K2 the task-state record and profile revision deltas; S1 and S2 fixed F3 and S3 fixed F7. Wave 3 · G1
gave F6's budget part the capability index and the on-demand visual guide, and G2 the total budget, the
digest that keeps Decisions and resumed-turn deltas. Wave 3 · O1–O3 measured F8 and bounded it (one
preparation deadline, the Jev guard, the Codex inventory cache) and narrowed F11. Wave 4 · V1 published
F9's per-provider contract, corrected the skill copy and probes the Codex helper CLI; V2 gave
`list_actions` a query, filters and paging and every chat turn a relevant skill index. Still open:

- **F3 remainder · wave 2, lane S.** S1 (the broker) fixed the pre-rank cap, the scan budget, the
  `local-` scope guess and the missing semantic blend; S2 the name-ordered attachment prefixes and the
  missing reader (above). Still open: `search_vault` can't blend semantic candidates (the subprocess has
  no embedder), so with semantic search on it may rank differently from automatic retrieval. Attached
  chunks are lexical only (no semantic half). A single line longer than 3,000 bytes reaches the
  envelope as the window around its best match (the reader returns up to 24,000 bytes of it). The
  manifest counts the agent's rereads (`agentReads`) but not which ranges it read.
- **F4 · wave 2, lane K (continuity), what remains.** K1 made one host context compiler for every chat
  entry point and K2 added the task-state record to every fresh handoff and the profile delta to resumed
  sessions. Left: the record reads constraints and corrections by fixed cue words (no model), so a
  constraint phrased without one ("dates in ISO 8601, please") is missed and some chatter is kept; recall
  and connector selection don't read the record. Since G2 the digest keeps `Decisions.md` and
  `Guardrails.md` on overflow and cuts prose (Identity first). A turn an agent sent before 2026-10-07 has no `asked_by` and reads
  as the user's. A conversation's bundle still labels an agent's request `user`. A delta compiled for a turn that then never reaches its agent (a spawn failure, a stop
  before spawn) is lost until the next fresh session. The `continuedFrom` link lives in memory, so after
  a reload a continued conversation's fresh turns read only their own key. Automations and the
  heartbeat still build their own operator block every run.
- **F6, budget part · wave 3, lane G, what remains.** G1 gave every chat a compact capability index and
  moved the visual guide out of the configured half below Most replies (a turn gets it when its request
  looks visual, once per native session; every turn can read it from the scratch folder). G2 bounds the
  total (128,000 bytes per turn, by priority), keeps Decisions and Guardrails on profile overflow, and sends
  the envelope and (Codex, Antigravity, Grok) the configured half as deltas on exactly resumed turns. Left:
  bytes, not provider tokens or cache hits (the pilot measures those); the same total for every provider
  until real token counts justify a difference; Hollow can't see a provider's own compaction, so a delta
  within the 8-turn refresh assumes the session still holds what it was given; a changed document goes as
  whole-line `+`/`-` lines without positions; retrieved evidence is resent every turn (it follows the
  request; recall only when it changed, S02), and a new topic's excerpts are new; automations and the heartbeat have no total
  budget (they get the digest order). The visual cue is lexical (Hollow's word list, numbers, data-file
  names): a visual request without one gets the guide only if the agent reads the file, and a false cue
  costs one guide per session. (A guide block compiled for a turn that never reached its agent was
  recorded as given until V1; it is now recorded only after the spawn.) Whether agents read the file when
  told to, and visual-answer quality without the guide in the prompt, are for the pilot's visual slice
  (no live run yet). (The index's skills line was a pointer until V2, which added each turn's
  relevant skill index.)
- **F7 remainder.** S3 gave recall and retrieval one lifecycle policy (above). Still open: history
  intent is lexical: since HF04 an imperative "before you start" no longer makes a request historical,
  but a clause-final one ("format it as before") still does (the old version then comes labelled,
  behind the current one). Two current learnings that contradict each other
  without a `superseded_by` link are both current: only exact duplicates collapse, and nothing compares
  meaning. Jev's passage rerank may drop a promoted current version (the label on the historical one
  still names it). The `vault.vault_search` and `vault.semantic_search` note lists aren't labelled. Only
  `learnings/<name>.md` notes are judged: a learned-context note filed elsewhere is ordinary evidence (recall
  never lists one either), and a non-learning note marked Superseded is deliberately left as it is.
- **F8 · wave 3, lane O.** O1 timed every preparation step (the manifest's `preparation.timeline`,
  above) and recorded the baseline ([AGENT_EFFECTIVENESS.md](history/AGENT_EFFECTIVENESS.md#wave-3-baseline)):
  two Jev timeouts cost 8.0 s, serially. O2 prepares retrieval and connectors at once under one
  3 s deadline, reuses identical Jev rankings, pauses a failing Jev and refreshes the Microsoft 365
  token only after selection (timeout mode 8,294 → 3,237 ms p50). O3 caches Codex's `codex mcp list`
  per binary, config and folder (the stand-in's 47 → 0 ms p50; the real CLI's cost is unmeasured, no
  Codex run here). Still open: each Jev request still builds its own client (reuse measured at
  33 µs and left out; a kept-alive TLS connection to TypeSafe is unmeasured); the real semantic half's
  cost and the real TypeSafe latency aren't measured (no embedder in tests, no paid calls), so the 3 s
  budget isn't tuned against them yet.
- **F9 · wave 4, lane V, what remains.** V1 made the per-provider contract one table in code
  (`capability_contract.rs`), rendered above and into each chat's capability index, corrected the
  skill copy and probes the Codex helper's version once per binary (0.153.4 verified). V2 gave
  `list_actions` a query, tier filter and paging with one-line summaries, listing only what the run may
  use, and every chat turn a relevant skill index with the selected skill's text inline and
  `hollow_vault` `read_skill` (the pilot's selected-skill task failed because an accept-edits agent
  couldn't read a `SKILL.md` outside its workspace; the bed row `ae-skill-index` checks the fix with
  the fake agent, and the paid pilot hasn't rerun). Still open: the skill match is lexical (a request
  that paraphrases a skill without sharing a word misses it, and a common word can list an unrelated
  one, at most 5); the Skills page lists neither the vault's `.agents/skills` nor offers Use in
  workspace for those (the pilot writes the composer text itself); the `list_actions` query is
  lexical too; whether Claude Code and Codex load their own skills in a Hollow run is unverified;
  Antigravity and Grok get no owned MCP or connectors (what it would take is above; building it is a
  separate decision), so only a selected skill's inline text reaches them for sure; Codex keeps native
  apps and plugins off; the native-default comparison (what Hollow adds and removes against each CLI's
  own default) is unmeasured. A Codex version past 0.162.1 needs both local checks rerun before helpers
  run on it.
- **F10 · wave 4, lane D, what remains after D1 and D2.** D1 named the two contracts everywhere an agent
  reads about them, gave each child a bounded task packet (the parent's constraints, sources, expected
  shape and budget, plus the asking chat's settled constraints and changed files), let a packet lower a
  consultation's deadline, gave Codex a 240 s `hollow_team` tool timeout, and lists delegations on the
  parent's manifest view. D2 reports a child turn's end once (`wait_for_session` or the next turn's
  note), keeps every wait 15 s inside its caller's tool timeout and times a consultation from its send
  (the team and app servers now run calls at once), stops a stopped chat turn's child turns, and rolls descendant cost up to
  the parent (view and receipt). Still open: a child task's budget and a consultation's `usd` are the
  asking agent's request, not enforced; a child task has no deadline of its own; only a chat turn's stop
  reaches its children (an automation's cancel, a stall or a deadline leaves them running, and its
  receipt has no roll-up, though its manifest view does); a note compiled for a turn that never reaches
  its agent is lost (the delivery is marked at compile, as for an `<operator-update>`); a chat with no
  next turn hears of a child's end only by waiting or reading; the MCP `notifications/cancelled` a client
  may send when it gives up on a call is ignored (the call then runs to its own deadline); concurrent
  consultations in one folder may each list the other's new files; a child beyond the 32 delegations
  recorded per run is neither stopped nor rolled up; the roll-up reads usage rows, which Hollow keeps 90
  days; Claude's `MCP_TOOL_TIMEOUT` is not set by Hollow, so its bound is Claude Code's default; the
  host's constraints come from the task-state record's cue words (F4's limit); a child task's packet
  comes after the backend's wrapper (the workspace envelope can precede it); the bed's packet row can't
  read the parent's view (the driver's own caller has no receipt; D2's cancel row starts its child from a
  real parent turn and does). The asking chat's settled constraints (the user's own sentences) reach a
  teammate of another provider in its packet, as the asking agent's own task text already could.
- **F11 remainder.** O3 keeps each duplicate check's report with its run, reconciles a run's Jev
  calls in its manifest view (`jevCalls`, cost `null`) and gives the heartbeat a manifest. Still open:
  Jev stays out of the usage ledger (one row per agent invocation; its calls are in the manifest view
  instead); a chat turn's after-run learning check is in memory only, so a restart loses it; there are no
  manifests for ingest, teammates or one-shots (`hollow team-mcp` is its own process with no evidence
  collector); the workflow layer order differs, and the adapters'
  policy isn't measured.
### Profile promotion and skill lessons (2026-11-08)

An explicitly general rule in the user's own source quote can become a pending
profile proposal. A task procedure belongs in a skill. Capture reads the source
run's successful skill-read/inline manifest and the registered shared skill
fresh before proposing an append. A missing fitting skill may become a new
skill proposal. Library Learnings and the source reply show Save and Reject;
neither route accepts automatically. The saved snapshot hash is checked again
at Save; a changed skill closes the lesson as stale, re-proposable from a
later chat. Save scans the whole resulting file and writes through the skills
module's protected writer. Undo skill lesson restores the pinned bytes, or
removes a new skill, while the skill is still what Save wrote and the memory
log is sound and appendable. Existing
frontmatter and instructions are preserved, and new names refuse collisions.
Provider and plugin skills remain read-only.


Company actions: none. The company's actions went in L5 and its code in L11
(11-10); the COO's prompt names no `company.*` action. Record:
[company assignments](history/COMPANY_ASSIGNMENTS.md).
