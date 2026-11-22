# Glossary

Part of the agent manual. [AGENTS.md](../../AGENTS.md) is the entry point and
links every reference doc; this one holds CLAUDE.md's "Glossary" with its
"Artifact-to-session continuity" note, moved there word for word on 2026-10-13
(only relative links were re-based). Read it when a project-specific term is
unclear. Keep it current with the code in the same change, as AGENTS.md asks. The everyday product model above the
glossary was added on 2026-10-14 (plan completion M5).

## The everyday product model

What each everyday concept is for, and where it lives. This is the 2026-10-13 UI/UX review's "concept
responsibilities" table (UI_UX_REVIEW), stated as
the app now implements it (matrix row M5). The one-line model: *a workspace holds an outcome; you give the agent
sources, work through its plan, and review the results.* For the click path to each home, see
[ROUTE_LEDGER](../ROUTE_LEDGER.md).

| Concept | What it is for | Where it lives | Not to be confused with |
|---|---|---|---|
| **Work** (Home) | Resume active outcomes and handle the most important decision first | Rail → Work; its views are Home (the default: Ask and recent projects), Today (a Day / Week switch: the day's brief and the weekly review), Map and Running | A briefing: Today's Day and Week are built from existing evidence. Today's brief says when it was written; the weekly take sits under its week |
| **Workspace** | A durable outcome: its brief, plan, sources, conversations and Results | Open a workspace; its chat fills the page, and the top bar's Project files opens WORKSPACE.md, PLAN.md, SOURCES.md and AGENTS.md in the Preview pane (a Result there gets its review bar) | The same name can be a local project, a tracker project, a legacy Notion folder or a build checkout. The origin label tells them apart, and Archive… (off the active list) is not Hide from sidebar |
| **Conversation** | One thread of agent work inside an explicit context | A workspace's conversation, a chat in Chats, or the Main agent; Conversation history names its owner and resume behavior | Conversation history: the open conversation's header names its kind, and "About this conversation" says who owns it |
| **Plan** | Steps toward the workspace's outcome (`PLAN.md`) | Workspace → Plan | A task. Making tasks from a plan copies the text and keeps a link back: ticking the plan item doesn't finish the task, and finishing the task doesn't tick the plan item |
| **Task** | A schedulable action tracked on its own | Rail → Tasks (List, Board, Agenda with its Day and Week; the **Due today** scope; saved filters in **Filters**) | A plan item or a promise. A task shows where it came from (plan item, recording, Incoming item or promise) and opens that exact item; an Incoming origin opens Incoming, because filing used the item up |
| **Commitment** | A promise to or from a named person, with the evidence it came from | Rail → Tasks → Commitments | A task. "Make a task" from a promise you owe creates a linked task, and the promise stays open until you close it |
| **Result** | An output in a workspace's `outputs/`, reviewed and accepted by exact version | A workspace's Results; every Result is under Library → Results (canvases under Canvases) | "Created" or "finished": Accept applies to the version on screen, and Ask for a revision drafts a request without sending it |
| **Waiting on you** | Decisions and interventions that need you | Rail → Review (⌘5): All waiting, Decisions, Results to review, Failures, Incoming | Incoming, which is a view inside Review |
| **Incoming** (was Inbox) | Newly arrived items to classify and file | Rail → Review → Incoming | Approval: filing organises an item and approves nothing |
| **COO** (was Main agent) | Help across all your work, and the ongoing work you set up for it (standing orders, check-ins); it hands building, research and writing to a project's lead or a subagent | Top bar → the COO orb, or ⌘⇧M (its own page); Work → Ask anything → Across all my work to put a question in it | A workspace conversation. Check-ins run unattended under the rules in "How background work runs"; it runs on any installed agent |
| **Lead** | A project's own conversation, the first row under the project in the sidebar | Sidebar → a project → Lead | A chat you started in the project |

## Words we use

One meaning per word in what Hollow says (P1-03, 2026-10-24). `scripts/check-plain-words.mjs` holds the list: its
engineering words (receipt, stdout, session key, IPC, backend call) fail the gate in shown text, and the rest record
which word won a collision. Change the list there, then run `node scripts/check-plain-words.mjs --glossary` to
rewrite this table; `check-plain-words.test.mjs` fails while the two disagree.

<!-- words:start (scripts/check-plain-words.mjs --glossary writes this table) -->
| Say | Don’t say | Means |
|---|---|---|
| run record | receipt | what a run changed and did |
| output / error output | stdout, stderr | what a command printed |
| the conversation | session id, session key | the thread the agent works in |
| request | IPC | what the window asks Hollow to do |
| request to Hollow | backend call | the same, from an agent or a view |
| automation | workflow | a saved prompt Hollow runs on a schedule or a trigger |
| Import | ingest, Quick ingest | bring a file or a link into the Library |
| Tonight (the night window) | night shift, overnight shift | the hours Hollow runs queued work unattended, within the overnight cap |
| History & recovery | Vault History | every change Hollow and its agents made, and how to undo it |
| Draft for the COO | Do it (Today, Weekly review) | puts the suggestion in the COO’s message box; you send it |
| Apply ticked | Do it (Tidy up) | applies the ticked lines of a tidy-up and nothing else |
| Do it | Do it on a button that only drafts or applies | your answer to a COO suggestion (Do it / Not now / Never); Ask’s scopes say what their button does (“Ask the COO”, “Start the chat”; P1-11) |
| Running | Activity (Work’s view) | every running and queued agent turn, and what just finished |
| History | Activity (a workspace’s section) | a workspace’s runs, changes and conversations over time (History & recovery and Conversation history are their own places) |
| Background tasks | Activity, Background activity (the dock) | long operations in the corner dock |
| Recent notifications | Recent activity (Review) | the notification history under Review |
| Last active | Recent activity (Work’s sort) | workspaces in the order you last used them |
| Log | Activity (a task’s detail) | the dated entries in a task note |
| Summary | Visual (a section’s toolbar) | a section’s own layout, as opposed to its canvas |
| Blocked | Waiting (Tasks) | a task held by its status or an open blocker |
| Library | vault (for where notes live) | the notes, imports and results Hollow shows; the Hollow folder is where they sit on disk |
| Silent heartbeat | Heartbeat, background check-in | the quiet agent pass over what changed in your notes, calendar and tasks |
| the chat’s agent’s own mode names (Accept edits, Ask for approval, Skip permissions) | Edits only, Full access in a chat’s menus | a chat names its permission mode as its agent does (per-agent menus, 2026-11-12); Edits only and Bypass all stay for automations, which pick no agent’s words |
| Waiting on you | Waiting for a blocked task | Review: decisions and interventions that need you (a plain “waiting for…” loading line is fine) |
<!-- words:end -->

## Glossary (acronyms and project-specific terms)

- **Hollow** — the app. "Alex Morgan's operating system."
- **The vault** — Alex's Obsidian vault. Filesystem-backed markdown notes.
- **Stable URL** — every Task or Project has a `url` field used by
  the frontend as its drag-drop key. It is always a synthetic
  `vault:{rel-path}` URI that `vault_tasks::find_by_url` /
  `vault_projects::find_by_url` resolve back to the file on edit.
- **Background tasks** (the activity dock, `ActivityDock.tsx`) — the bottom-right rolling log of long-running operations.
- **Snackbar** — toast with optional Undo button; focus or the pointer holds its countdown.
- **Environment brief** — `chat.rs::environment_brief()`, a non-workspace
  (vault) chat's shared context (all adapters): vault root + folder map, the
  local-files story (Notion as a read-only archive), the user's Downloads folder, and which
  Hollow MCP capabilities this session attached (`hollow_questions` included
  since PR 13, or the fenced-block fallback for Antigravity). It doesn't
  repeat `WORKSPACE_POLICY`, which every adapter appends (F6). Every chat
  (vault, quick, workspace) also gets the compact capability index
  (`chat::capability_index`, G1) ahead of its brief. Three context layers now: environment brief (*where am I*),
  operator context (*how the user works*), page context (*what's on screen*).
- **Connector** — anything that reaches outside Hollow: a shared MCP server
  from the connectors library (Notion's official connector among them), a
  built-in connector (Microsoft 365, Plaud), or one of Hollow's own sources
  (tracker sync, Plaud).
- **Permission mode** — `plan`, `default`, `acceptEdits`, `auto` or
  `bypassPermissions`, the same meaning in every agent (see Permission modes
  across agents).
- **Workflow** — an automation stored at `vault/workflows/{slug}.md`
  (schedule, optional `output_glob`, run metadata in frontmatter; prompt body;
  `## Run log`). Every run is a row on the run queue.
- **Result** — a file in a workspace's `outputs/`, with its review state.
- **Scoped run** — a workspace or build-worktree turn that holds no
  full-vault lease: shared in a workspace folder (several at once); a
  build worktree's turn holds the folder alone, or once the workspace has
  lanes is shared there and holds only its worktree's lock (F6 part B).
- **Custom theme** — a kitty-format `.theme` palette file under
  `~/Library/Application Support/Hollow/themes/`. Parsed by `themes.rs`,
  applied as inline CSS variables on `<html>` by `applyCustomTheme()`.
- **Token system** — the one set of design values in `styles/tokens.css`
  (see `## Styling`): neutral palette, Acme-red accent, hairline borders,
  three elevation levels. It replaced **Carved Light** (the June 2026
  "one slab, recessed content, amber glow" language) in frontier F2.

### Artifact-to-session continuity

Vault notes and existing project editors expose a lazy **Sessions** disclosure.
Projects use the project's `vaultPath`: this is history for the project note,
not inferred membership across every task or file in a project. Rows group
retained chat runs by Hollow session key; unlinked older runs remain reviewable
without a resume action. Only exact changed paths in automated history establish
a link. A successful structured write receipt labels an edit **confirmed**;
otherwise it is **observed during the run**, not attributed or verified by guess.
Coverage is limited by history retention (200 runs / 90 days / 512 MiB); empty
results do not establish that an agent never touched a file. Renames are separate
paths, and external sessions without a recorded run are not imported here.
