# Tasks in the workspace shell

Implemented in 0.5.3 (2026-09-23). The user asked for a full redesign after the
restored Tasks sidebar entry still opened the old dense task interface.

## Interaction

Since overhaul B1 (2026-11-10) Tasks is laid out like the task tracker's
issue list; the page, its keys and the issue detail are described in
[Tracker sync, Hollow's screens for the tracker](TRACKER_SYNC.md#hollows-screens-for-the-tracker).
The tracked issues come from `tracker_view`; the vault's untracked tasks sit in
a **Local tasks** group (open ones, and done ones only while a search is
typed), with a check button that ticks them done or open and a title button
that opens the local editor below. **New local task** opens the same editor
to create one; **New issue** creates one in the tracker. The Notion-era list,
its scope tabs, the List and Board layouts, the project filter, saved filters
and sort, bulk edits with their outcome report, the completion animation, the
row ⋯ menu and the Unassigned Notion tasks fold were removed with it.
Suggested tasks still show above the list (Waiting on you's tasks link), and a
failed read of the tasks folder says so with Retry. Views, filters, grouping and
order (B2) are in the same section of TRACKER_SYNC.md; the agents' saved-filter
actions (`tasks.list_task_filters` and the rest, `.task-filters.json`) stay, with
no screen of their own.

A local task's pane (`pages/tasks/TaskDetail.tsx`) keeps its edit form,
blockers, its From line and Work on this, as described below.

### Task mutation contract

One contract across every writer (review roadmap A; plan completion row T10,
2026-10-13). Local field writes go through `vault_tasks`' shared writer (one
lock, `vault::mutate_note`), so the compare-and-swap check and the write are
one critical section; a tracker-linked task's edits go through the tracker
seam (`tracker/seams.rs` `user_edit`, [Tracker sync](TRACKER_SYNC.md)) and creates through `create_from_input` under
`CREATE_LOCK`. "Baseline" is the `contentHash` the caller last read; a stale
one is refused with `TASK_CONFLICT: …` and writes nothing. The UI sends
`contentHash || undefined`, and an omitted baseline means no check, so a task
listed without a hash would be written unchecked (vault tasks always carry
one).

| Operation | Callers | Baseline | Returns | Tracker-held fields | Repeating task | Retry safety |
|---|---|---|---|---|---|---|
| Drag / row menu / row tick | board, Agenda's week, menu, the row's tick (`dragdrop.ts` `applyDrop`) | the dragged row's hash | new hash (patched into the list) | a tracker-linked task's edit is queued to the tracker as a durable user edit (`tracker/seams.rs`); a field the issue holds is edited there | Complete advances a local one; no Undo offered | Undo sends the hash the drop returned |
| Task pane edit | `TaskEditor` (`update_task`; a tracker-linked task's fields go through `tracker/seams.rs` `user_edit`) | the hash when editing began (draft kept in sessionStorage) | new hash | held fields are disabled (`Task.tracker.lock`); relations and steps on a tracked task are refused (`TASK_LINKED: …`) | `update_task` writes fields, never rolls | the draft survives a conflict |
| Blockers | Tasks pane (`update_task` `blockedBy`) | the hash when editing began | new hash | `blocked_by` is Hollow-only | n/a | `TASK_BLOCKER` / `TASK_CYCLE` write nothing |
| Create | Tasks, plan preview, recordings, promises, Incoming, agents | n/a | url and `existing` | n/a | n/a | idempotency key: a repeat returns the task it made |
| Agent writes | `hollow_tasks` MCP → `task_broker.rs` (`set_status`, `update_task`, `create_task`) | `expectedHash` required for updates and for completing a repeating task | new hash | fields the tracker issue holds are refused (`TASK_LINKED: …`) unless sent as `update_task`'s tracker fields | rolls forward once per baseline | live lease; idempotency key on create |
| Companion ops | `companion_inbox.rs` `apply_merge` (through the broker) | the op's `baseHash`; a moved file is merged per field and retried | n/a (ledgered once per op id) | as agent writes (same dispatcher) | as the broker | an op applies once (ledger); a true conflict becomes a card; an op sent more than 14 days ago waits for Apply it |
| Delete / restore | Tasks pane (`delete_task`, Undo `restore_task`) | none: delete has no compare-and-swap | the deleted bytes, for Undo | n/a (never deletes remotely) | n/a | restore refuses if another task now holds the path; the original stays in `.trash` |

The GUI's own `update_task` sends a tracked task's fields to the tracker as a
durable user edit; a conflict shows on the issue, never a silent overwrite
([Tracker sync](TRACKER_SYNC.md)). Tests: `vault_tasks` CAS tests, `task_broker`
`update_task_conflicts_when_the_gui_wrote_between_read_and_write`, the
companion merge tests, smoke `next-level-tasks`.

The bulk selection and its outcome report (plan completion item 4) were removed with the old list in overhaul B1.

A task's origin opens the exact source where its identifier allows: the plan
row it was copied from (**Open the plan item**), the recording
(`meeting:<recording id>`, **Open the recording**, `store/originFocus.ts`;
the range widens to 90 days once), a meeting note (`meeting:<path>`, **Open
the meeting note**), the promise (**Open the promise**, selected in
Commitments) and the agent run (**Open the run in History**). An Incoming item
is consumed when filed, so that origin opens Incoming. The detail and editor
also say how the two relate: a plan task is a copy that isn't kept in sync; a
promise's task doesn't close the promise.

As of the 2026-09-24 UI pass, the list uses the full width until a task is
selected or a creation form is open. Close releases that space again.
The context pane shows properties, summary/notes, checklist and a direct Open
workspace action when a stable project ID/path resolves. It never guesses a
workspace by title.
On windows under 1000px the selected detail replaces the list; Close returns to
it. List, Board and Agenda are alternative layouts in a compact selector (`Segmented`); status synonyms
such as `todo` group under Not started (`canonicalTaskStatus`, frontier F3). Only the selector saves the
layout (`hollow.tasksView`, `store/tasksLayout.ts`): Agenda from `g c`, the palette or Today shows for that
visit (`goToPage(page, { tasksView })` → `setActivePage(page, tasksView)`), and the rail brings back the saved
one. Agenda lists the same filtered tasks as the list.

Agenda's **Week** is the one week view (simplification S6, 2026-11-03). Tasks'
own Next 7 days (called Week until P1-16) was merged into it: a saved
`calendar` or `week` layout reads as `agenda` with the range on Week, and is
rewritten once (`readTasksLayout`), so a later choice of Day sticks. Agenda
keeps its Day/Week choice in `hollow.agenda.range` (Day until Week is chosen);
a day's heading in the week opens that day without changing the choice. Each
day takes a task dragged onto it as its due date through Board's path
(`dropTask` → `applyDrop`; a day that
can't take it never lights up), and the week's tasks drag between days
(each is a `<button draggable="true">`; `agenda.css` also sets
`-webkit-user-drag: element` for WebKit, which likely maps `draggable` to
it already; not yet checked by hand in the installed app).
**Not this week** sits under the seven days: the open tasks shown that fall
outside them, as Earlier, Later (folded) and No date, so a task doesn't
disappear because it has no date, and its cards drag onto a day. Relative to
the week on screen, so it follows Previous/Next week. The keyboard path is a
task's ⋯ (Due today / tomorrow / next Monday) or Edit task; ↑/↓ stay within a
Not this week group when one of its rows has focus and otherwise leave Agenda
alone. Done tasks leave the week and Not this week, as in Agenda's Day.

New task opens a small form in the context pane: title, project, date, priority
and notes. An explicitly selected project stores its exact validated vault URL.
This creates a **local-only** task, clearly labeled in the form; **New issue**
creates one in the tracker ([Tracker sync](TRACKER_SYNC.md)). Existing
local tasks retain More details for the advanced editor (labels, schedules,
recurrence, subtasks and AI assistance); its surfaces match the new styling.
Break down on a linked task sends the task's stored project URL (only while the
Project field is unchanged; never matched by name), so the suggestions see that
project's goals as reference evidence (Paperclip PR 10).
The palette sheet (TaskModal) renders the same shared `TaskEditor` as the Tasks
side panel (audit H02): identical fields, vocabulary, validation and names.
It disables and never sends the fields the task's tracker lock
(`Task.tracker.lock`, `taskLocks.ts`) holds, plus the due time when the due is
held and carries one. The lock kinds (`linked`, `remoteMissing`, `moved`,
`unreadable`, `staleRelations`, `outOfScope`) are in
[Tracker sync](TRACKER_SYNC.md). The Notion lock (`Task.notionLock`) was removed
with Notion sync on 2026-11-10: a task with only legacy `notion_id` keys is a
plain vault task.

Everything else (labels, duration, repeat, section, sub-tasks) saves from the
modal.

The old quick-add bar, planning scorecards, group controls, progress meters and
animated task cards are removed from this page. Saved filters remain in Filters;
Task suggestions sit above the layouts (P1-16), so Board and Agenda show them too. This does
not add an Approvals or Inbox route.

## Editing and sync boundaries

A tracked issue's fields, archive (**Archive in Linear**; an agent's
`tracker.archive_task` asks no card since T1, 2026-11-15) and conflicts live in the issue detail
([Tracker sync](TRACKER_SYNC.md#hollows-screens-for-the-tracker)). Plain
*Delete task* on a local task stays local only. "Move to Notion trash", the
Notion-mapped form and "Sync with Notion" were removed with Notion sync
(2026-11-10). *Work on this* says "Available when the running chat turn
finishes" while a chat turn runs. A refused or failed change says why with
Dismiss; Retry sits only on a read that failed.

Local edits send only changed fields through `update_task`, and so does
TaskModal (an untouched sheet sends nothing). Both are compare-and-swap: the
task's `contentHash` (sha256 of the file text) is captured when editing starts —
the pane stores it in the draft cache as `hash` — and sent as `expected`. The
backend compares it under `MUTATE_LOCK`; a changed file returns `TASK_CONFLICT:`
with nothing written, and the draft stays editable. Agenda's week drag-to-schedule
sends the dragged snapshot's hash and refreshes after a refusal. Status and
priority toggles stay last-writer-wins but return the new hash, which the list
records so an editor opened next holds a current baseline. Agents' `hollow_tasks`
writes of a field a tracker issue holds are refused (`TASK_LINKED:`) unless sent
as `update_task`'s tracker fields ([Tracker sync](TRACKER_SYNC.md)). Every Hollow
write line-patches only the changed keys, so unmodelled frontmatter
(`previous_due`, legacy `notion_*` keys, unmodelled lists, custom keys) survives. A YAML-list `labels:` is read and kept until labels are edited,
which rewrites it as the comma scalar. An edit that an indented `key:` line
would shadow is refused rather than reported as saved.

Bodies are section-aware (Paperclip PR 11):
- **Fenced blocks.** A line opening with ``` or ~~~ (3 or more, up to 3
  leading spaces) starts a fence only when a matching close (same character,
  at least as long) follows. Fenced lines are never steps or headings. An
  opener with no matching fence line after it is ordinary text, deliberately
  unlike CommonMark (where it would run to the end of the note), so a stray
  ``` at the end doesn't swallow the steps and sections after it; in Obsidian
  such a note still renders the rest as code. A stray opener still pairs with
  the next matching fence line, even one that opens a later block, and hides
  everything in between (as Obsidian does).
- **Reserved sections.** An H2 outside a fence reading `Done when` or
  `Activity` (any case, trailing `#`s and one `:` ignored) starts a reserved
  section that runs to the next H1 or H2 outside a fence. Its lines, checklist
  lines included, are never steps or description; the pane shows them
  read-only as collapsed **Done when** / **Activity** disclosures
  (`Task.doneWhen` / `Task.activity`). Repeated headings stay separate.
- **Steps** are the unfenced checklist lines everywhere else; the other lines
  are the description (headings such as `## Context` included).
- **Writes.** A steps-only change rewrites the step lines in place: an
  unchanged step keeps its bytes, a changed one keeps its indentation and
  `- ` / `* ` marker. Old step lines are matched to the new steps by text
  first, so deleting a step anywhere removes only its line, and a new step
  goes right after its preceding step (at the front, just before the first
  step; with no steps, after the leading prose, before the first reserved
  section, and never ahead of the body's first line, which in a CRLF file is
  the rest of the frontmatter fence). A toggle is therefore a one-line diff in
  any layout. Only a body-routed description replacement recomposes the canonical
  layout: the new prose, the steps, then every Done when section and every
  Activity section verbatim. A reserved heading typed into the description
  becomes a section there rather than swallowing the steps. Nothing is
  written when neither changes.

**Idempotent create.** `create_task` takes an optional `idempotencyKey`
(≤ 16 KiB, else `TASK_KEY:`) and `origin`. The key's sha256 is stored once as
`fingerprint:`; a later create with the same key returns that task
(`CreatedTask.existing: true`) instead of a `slug-2.md` twin, even after the
task was renamed. Heartbeat and meeting approvals use the same path with a
title-slug fallback for tasks written before fingerprints existed; ProjectModal
keys each drafted task `project-draft:<project>\n<normalized text>` (a retry
after a drafted task failed reuses the project the first attempt created, so
the keys match and the tasks already made are returned), and the
PLAN.md preview keys plan items (see [Workspaces](WORKSPACES.md)). Creates are
serialized in-process by one `CREATE_LOCK`. The `hollow_tasks` MCP
(Paperclip PR 12) never writes from its own process: it proxies writes to the
GUI, which runs them through the same functions.

**Agents and tasks (Paperclip PR 12).** A Claude or Codex chat turn or
automation can list, read, create, update, complete and comment on tasks
through the `hollow_tasks` tools, and suggest tasks for review. Those writes
happen only while that run holds the vault, go through the same writers as the
Tasks page (so an agent's edit of a task you changed meanwhile fails with a
conflict instead of overwriting it), and appear in History & recovery and in
the run's receipt (as `observed`). Agent writes are validated strictly: dates
`YYYY-MM-DD`, times `HH:MM`, durations 1-1440 minutes, priorities P1-P4 (or
High/Medium/Low), recurrences Hollow can read, one-line titles of at most 240
characters. An agent can't change a task's status through a field edit (it
uses the status tool, which rolls a recurring task forward exactly as the
checkbox does), can't move a task to another project, and can't change fields
its tracker issue holds (the time too, when the held due has one); it can still add an
`## Activity` comment. Plan-mode chats and automations whose allowed tools
grant no writes only read. Suggested tasks appear as **Agent-proposed task**
in Tasks → Task suggestions (edit, accept, reject, snooze like the others);
accepting creates the task once, and nothing is created until you do. Turn
the tools off in Settings → Agent tools → Task tools. A run can claim a task
(`claim_task`: the owner is `<surface>:<session or run>`, the task moves to In progress,
another run's claim is refused as `TASK_CLAIMED`, and a repeat by the owner answers
`applied` with `already: true`), offer itself the next unclaimed one (`next_claimable`)
and release it (`release_task`, owner only; the task goes back to Not started), all
through the broker (`task_broker.rs`, `vault_tasks::claim_task`). A `skip_if_quiet` automation that watches
`tasks/` and writes tasks through these tools doesn't wake on its own writes
(2026-11-13): `with_live_run` notes each file a broker write makes for the
run with its hash, the run's finish marks a change it left exactly so as
`hollowWrite` in History and the run's receipt, and the quiet check credits
that receipt mark as the run's own write, like a `confirmed` one. Another
run's or the person's write to `tasks/`, and any edit after the run's own
write (during the run or after it), still wake it.

**Blockers (Paperclip PR 11b).** `blocked_by:` holds `vault:tasks/…` links to
the tasks this one waits on (a comma scalar; a YAML block list or a flow list
such as `[a, b]` or `[]` is read too, and rewritten as the comma scalar only
when the links change); it is Hollow-only. A tracked task's relations come
from the tracker instead (`update_task` refuses `blockedBy` on one). The pane shows **Blocked by** (each
blocker's title, ✓ once it is finished) and **Blocks** (the tasks that wait on
this one), with an **Edit blockers** / **Add a blocker** button that opens the
picker and saves on its own through `update_task` with the `contentHash`
captured when editing began, so an edit made elsewhere while the picker is
open returns the conflict message and keeps the draft. TaskModal has the same picker as a form field; it sends
`blockedBy` only when the list changed. The picker searches open tasks by
title and leaves out the task itself, tasks already chosen, finished tasks and
any task that would close a loop; recurring and cancelled tasks come last with
a note. Under a blocker the pane warns when it will never resolve on its own:
`missing` (no task at that path: renamed, moved, archived or deleted; links
follow the path), `cancelled` (cancelled / canceled / won't do), `recurring`
(Hollow rolls a recurring task forward instead of finishing it) and `cycle`
(the blocker waits on this task, directly or through others). A task is
**ready** when it is not finished, cancelled or on an explicit hold status
and every blocker is finished; readiness looks one level deep and is computed
by the task list only. The backend re-checks every newly added entry on save:
`TASK_BLOCKER:` (names no task, or holds a comma, line break or `..`) and
`TASK_CYCLE:` (the task itself, or a loop, named by title); both write nothing
and show without their code. Stored links that no longer resolve are kept so
other edits still save. Workflows can react with the `task-unblocked` trigger
(see [the Workflows schema](reference/vault.md#per-tab-folder-schemas)).
Edit drafts share `hollow.task-drafts.<vault-path>` in sessionStorage. `vault:`
and `vault://` URLs normalize to the same identity. Local sheet fields and
mapped fields have separate values/baselines inside that envelope: saving one
does not clear the other's labels or steps. Legacy editor caches migrate when
opened, including stale drafts. New-task creation keeps `hollow.task-editor.new`
and its idempotency key until saved or explicitly discarded.

Close and task switching preserve drafts. A changed baseline shows **This task
changed since your draft: Review / Discard**. Review compares the saved draft
with a fresh local read, requires a choice for overlapping edits, and follows
current values for fields the draft never edited. **Use selected values**
rebases to exactly the displayed snapshot; Save still performs compare-and-swap,
so a later change refuses again. No force-overwrite path exists. An unavailable
linked record keeps its draft and offers Review to retry the read.

Ticks reserve only the affected task URL. Local rows remain usable during a
tracker refresh or failure.
Failed optimistic writes restore only their own field while its baseline hash
and optimistic value still match, preserving concurrent task results.

Tasks and sync metadata refresh every 30 seconds while the page is mounted;
timers are disposed on navigation. Date scopes use fresh backend buckets and
Agenda moves to the new day at midnight (`followToday`). Drafts are independent of those reads.
The header exposes the tracker's state and **Sync now** ([Tracker sync](TRACKER_SYNC.md)).

## Implementation

- `src/taskDrafts.ts`: canonical draft envelope and three-way value comparison.
- `src/components/TaskDraftReview.tsx`: shared visible per-field review.

- `src/pages/Tasks.tsx`: the tracker-style list (`pages/tasks/IssueList.tsx`), the issue detail (`pages/tasks/IssueDetail.tsx`), the Local tasks group and the create panes (overhaul B1). `src/store/tasksLayout.ts` still holds the retired layout choice for navigation; B2 replaces it with views.
- `src/pages/tasks/TaskDetail.tsx`: one task's edit pane (local form, blockers, Work on
  this); split out of `Tasks.tsx` by ST11.
- `src/styles/tasks-workspace.css`: scoped workspace styling and advanced-sheet
  styling. Other legacy task consumers retain TaskRow.
- `CreateTaskFields` / `CreateTaskInput` gain optional `projectUrl`. The backend
  validates a project note under `projects/`, resolves its current title, and
  persists that exact relationship. Legacy name-only callers remain compatible.
- `vault_projects::task_project_link` validates those links. No source scope,
  migration baseline, polling preference or remote write contract changes.
- Paperclip PR 3 (2026-09-25): `Task` gains `contentHash` and a lock
  (`notionLock` until 2026-11-10; now only the tracker's, `Task.tracker.lock`,
  filled by `vault_tasks::mark_task_locks`);
  `UpdateTaskFields` / `UpdateTaskInput` gain optional `expected`; `update_task`
  returns `UpdatedTask {url, contentHash}`; `set_task_status` /
  `set_task_priority` return the new `contentHash`. `src/taskEdit.ts` holds the
  pure TaskModal prefill and changed-field diff. `requestOpenTask(url)` /
  `taskOpenRequest` (`store/runtime.ts`) select a task on this page. Errors
  display through `taskErrorText`, which strips the `TASK_CONFLICT:` /
  `TASK_LINKED:` prefix from Tauri's plain-string rejection.
- Paperclip PR 11 (11a, 2026-09-26): `Task` gains read-only `fingerprint`,
  `origin`, `doneWhen` and `activity`; `CreateTaskFields` / `CreateTaskInput`
  gain optional `idempotencyKey` and `origin`; `CreatedTask` gains `existing`.
  `create_task` delegates to `vault_tasks::create_from_input` in
  `spawn_blocking`. `taskErrorText` also strips `TASK_KEY:`, and
  `errorWithPrefix(e, PREFIX)` (`src/utils.ts`) reads any backend error code.
  `src/taskStructure.ts` held `projectDraftKey` / `projectDraftOrigin` for
  ProjectModal's drafted tasks; both went in overnight H22, ProjectModal having
  gone with the old shell.
  Blocked-by, readiness and the task-unblocked trigger are part 11b.
- Paperclip PR 12 (12a, 2026-09-26): the shared backend writers for the
  `hollow_tasks` MCP; no IPC shape or UI changes. `set_task_status` delegates
  to `vault_tasks::set_status_with_recurrence`, which reads the recurrence,
  decides the roll-forward and writes in one `mutate_note` critical section
  (before, the recurrence was read outside the lock), trims the status and
  refuses an empty one. For the MCP: `create_agent_task` (a required key,
  scoped to the conversation or automation, `origin: run:<id>`, and
  `IDEMPOTENCY_MISMATCH:` when the key already made a task with another title
  slug), `append_activity` (one timestamped, escaped `## Activity` entry
  spliced in place, frontmatter untouched, optional compare-and-swap and a
  hidden `<!-- hollow-activity: … -->` marker that makes a retry a no-op; the
  marker is checked before the baseline, so a retry that still carries the
  pre-write hash reads as a duplicate, not a conflict),
  `resolve_task_ref` (refuses a symlinked task file or folder, which could
  otherwise aim a task write at `me/` or dot-state), `held_modal_fields` (the Rust twin of
  `lockedTaskFields`; keep them in step) and the validators `valid_due`,
  `valid_priority`, `valid_clock`, `one_line` (moved from `meeting_loop`).
  Each MCP write runs inside `automated_history::with_live_run`, so it is
  recorded in the run that made it or refused once that run has finished.
- Paperclip PR 11 (11b, 2026-09-26): `Task` gains `blockedBy` and the
  list-computed `blocks`, `ready`, `unresolvedBlockers` and `blockerWarnings`
  (`BlockerWarning {url, kind}`); `CreateTaskFields.blockedBy` and
  `UpdateTaskFields.blockedBy` are optional. `taskErrorText` also strips
  `TASK_CYCLE:` and `TASK_BLOCKER:`. `src/components/BlockedByPicker.tsx` (with
  `styles/blocked-by-picker.css`) is the picker; `src/taskStructure.ts` holds
  `wouldCycle`, `blockerCandidates`, `blockerLabel` and `blockerWarningText`,
  and `src/taskEdit.ts` diffs `blockedBy`. `openModal` also enables the tasks
  resource for the picker's candidates.
- Paperclip PR 12 (12b, 2026-09-26): the `hollow_tasks` MCP server
  (`tasks_mcp.rs`, `hollow tasks-mcp`) and its GUI broker (`task_broker.rs`);
  see [Task tools](reference/agent-tools.md#task-tools-hollow_tasks-paperclip-pr-12). Review kind `agent_task` (label
  "Agent-proposed task", `SUGGESTION_CONTEXTS.tasks`); `ReviewInbox` treats it
  like heartbeat and meeting tasks (summary, the Task / Due date / Priority /
  Context editor, a task refetch after accept). IPC
  `set_tasks_mcp_enabled` / `tasks_mcp_enabled` (`commands/tasks_tools_api.rs`),
  the `tasksMcpEnabled` signal in `src/store/runtime.ts` (seeded from the
  backend, never localStorage), and a `review-queue-changed` listener that
  refreshes the suggestion panels. Smoke:
  `tests/smoke/agent-task-suggestions.spec.ts`.

Browser checks use fixture IPC; signed installation and live sync checks are
recorded separately in [the build log](BUILD_LOG.md).
