# Automation

SE10 (2026-11-16): admission checks macOS free plus inactive bytes against
a 2 GB floor plus one estimated footprint per live turn and the next turn.
Adapter estimates are 0.25 GB for Claude and unmeasured providers, 0.18 GB
for Codex. Reserving live estimates in addition to measured allocation is
deliberately conservative: quick successive claims cannot spend headroom
before their processes allocate it. Low headroom reads `memory headroom`
and rechecks each second. Measured headroom replaces the saved overall count,
with a hard ceiling of 60 turns. Unknown or unsupported samples fall back to
the saved overall count. Workspace and subagent limits still apply. A waiting
vault-lane row gets the last memory reservation as well as the last count slot.
The three-turn
pressure allowance never bypasses this physical floor. Toolchain slots stay
separate and unchanged.

SE2 (2026-11-16): while any row remains queued, Idle and scheduled pump
sleeps recheck within five seconds, even if a notification was lost. Empty
queues retain the 60-second idle poll. Terminal row commits wake the pump
after the database lock is released; detached scoped execution wakes on
normal completion, panic or cancellation. Both chat-limit settings save
routes already notify. The fallback logs when queued rows cap a sleep.

Part of the agent manual. [AGENTS.md](../../AGENTS.md) is the entry point and
links every reference doc; this one holds CLAUDE.md's "Automation" section,
moved there word for word on 2026-10-13 (only relative links were re-based).
Read it before changing launch at login, routines, workflow permission stamps,
the automation queue, failure streaks, skips and retries, the heartbeat or the
Settings sections. Keep it current with the code in the same change, as
AGENTS.md asks.

## Automation — background runner, routines, and the heartbeat

Hollow runs as a resident background coworker. Settings → Background work
(startup, run queue, heartbeat) and the Automations page are the control
surfaces.

### Background work clock

Tonight also shows scheduled automations whose `nextRunAt` falls in the host's
`AutomationWindows.nightPlan` interval and the pending `nightPlan.jobs`. That
read uses the scheduler's configured Tonight occurrence and local timezone;
it never starts or stamps a job. Missing interval/read failures are explicit.
The section says "Scheduled in this window" beside the dated host interval:
on a weekend the next configured occurrence can be Monday, not tonight.
Only each automation's next scheduled run is included; later repeats are
explicitly outside this list's coverage.
Failed refreshes label retained times as previously read and never claim that
no runs or jobs are scheduled from that stale read.
The scheduled-work component loads separately with a local Suspense placeholder;
a cold chunk never hides Tonight's queue or its dry-run controls.
While Tonight is visible, queue, workflows and windows refresh every 30 seconds;
returning to a visible document refreshes once. Queue rows retain id identity.
The list groups Needs attention, Scheduled or triggered, and Manual. Next run
includes weekday, time and zone as well as relative time. Window jobs are the
fifth background mechanism, with a visible Automation windows Settings route.
Runbooks → New runbook opens the Library template with two editable checklist
steps; it creates nothing until the person chooses Create note.

Automations keeps its civil-day dial and native window buttons behind the
Background work windows disclosure on every view, folded initially. Read
failures remain named on the disclosure; open it for Retry and full details.
`backgroundClock.ts` uses `AutomationWindows.timeZone`, never the browser zone;
weekdays belong to starts, so yesterday's overnight tail stays visible. Equal
start/end means 24 hours. Configured clock hours may skip or repeat at DST;
these are neither elapsed durations nor proof a run started. The first six
windows appear on the dial; every configured window remains in the scrollable
list. Tonight opens Tonight; shared windows open Automations without claiming
an individual automation owns them. Unknown/failed reads offer Retry.
Short windows cap the clock at 110px with an internally scrollable legend;
the diagram keeps readable clock labels and leaves the automation list operable.
The Runbooks view scrolls its existing minimum-height card when space is short.

Workflows shares its non-suspending read with schedule labels and Tonight's cap,
discarding superseded replies. Refresh windows rereads it. The minute clock
updates only while the document is visible and cleans up on unmount.
How background work runs keeps its original disclosure and external opener;
Settings has the explanation only. Tonight's queue/removal and authority rules
are under About tonight’s queue; the count/cap line and errors remain visible.

### Automation run review

The compact header groups Run now and status/check pills in a wrapping action
row, with Schedule/Next/Last in a separate definition row. Automation options
(⋯) owns Preview configuration and Edit. The static state edge shares their
status words. Next runs retain their minute refresh and exact times, shown as
a labelled timeline. Loaded history supplies a bounded duration sparkline;
unavailable durations break its line and the dated reading list keeps exact
values and outcomes. Neither decoration adds reads or changes scheduling.
Keep `workflow-run-note` directly below actions, before the schedule row, and
as Run now's accessible description. Next run is the strongest schedule value;
warning colors must override the base value rule. The note
names the target, model allowance, usage and budget. Preview starts nothing.
The failure's What to fix and the activity row's Stop remain reachable.
About these results discloses the full receipt-derived file explanation;
opening it does not read or write more files.

Edit → Save as skill uses only a saved form: any unsaved field, including title,
description or notifications, disables it. Existing skill names offer Replace
(outside History), Save under a new name, or Cancel; creation never silently
replaces a skill. See [Skills](agents.md).

### Automation run state and Last night

The editor title and action group wrap within the detail pane, including at
720px with Largest text, so Stop run stays reachable without horizontal scroll.

The Automations page and editor combine workflow events with `runQueue()` and
`liveItemForSlug`: claimed/running rows are active even when they predate mount;
queued rows stay queued. Both run review and editor expose Stop run, using
`pages/workflows/workflowStop.ts` to block duplicate pending requests and report
failures. Stop requested does not mean stopped: queue/events own completion.
The native cancellation also cancels queued follow-ups. A command can stop the
current workflow before a later queue step fails, so errors say the stop request
could not be confirmed and ask the person to check the run status.

The Last night screen was cut overnight 11-08 S4, with Runbooks and the Tonight
queue. `api.nightMorningReport` still wraps the read-only
`night_queue_morning_report` command. The Rust night queue stays until the
later removal. Stop run on the Automations page is unchanged.

### Automation drafts

### Window and trigger editor

Schedule offers saved windows and Skip, Once on wake or Every missed run
catch-up. The host supplies three future starts in `WindowView.nextStarts`,
using window DST rules (first repeated time, first valid minute after a gap).
The editor and configuration preview format those instants in the host zone;
missing times remain unavailable. Cron retains its own DST/grace policy.
Visible Automations refreshes host times after an occurrence passes and on
focus/visibility return. Settings-prefilled windows survive template picks.
Triggers offers file-changed, task-overdue and task-unblocked. Additions use
the existing Save and approve dialog; removals narrow access. Unknown tokens
remain until removed. `WorkflowInput.triggers` omitted preserves the saved
list; empty clears it. Triggers participate in edit-base conflicts and stamps.
Comma/newline tokens are refused before writing the line-patched file.
Settings lists automations per saved window and Add automation prefills that
window. These navigation actions are disabled while windows are dirty/saving.
Failed workflow coverage says unavailable and offers Retry automations.

### Event triggers (F2)

COO rebuild F2 (2026-11-10, `event_triggers.rs`, commands in
`commands/event_triggers_api.rs`). Three provider-neutral events, each with a
stable id:

| Kind (token) | Event | Event id | Per project |
|---|---|---|---|
| `tracker-issue` | A new issue in team AI, from the tracker sync ledger (created after the trigger was turned on) | the provider's issue id | yes: the workspace whose `PROJECT.md` names the issue's tracker project |
| `pr-merged` | A merged PR in a project's connected repository (its build binding's repo, `gh pr list --state merged`) | `owner/repo#N` for each bound project | yes: build workspaces |
| `recording-ended` | A Plaud recording listed, or a calendar meeting's end (last 6 hours) | `recording:<id>`, `meeting:<event id>@<end>` | no |

- **Off until turned on**, per kind and, for the first two, per project.
  Turning one on stamps `since`; an event from before never fires, and off
  then on moves `since`. Recordings have only a day, so the first clean
  Plaud read after turning the kind on records every listed one as seen
  without firing; until then no recording fires (meetings have exact ends).
- **Deduplicated** by event id in a seen-set saved in app support
  (`<support>/EventTriggers/<vault>.json`, the newest 500 per kind), so a
  restart never fires one twice. An evicted entry raises the kind's
  `evictedThrough` mark, and nothing at or before it is decided again. An
  issue outside team AI is never decided.
- **Capped** at `capPerHour` fires per kind in a rolling hour (default 6,
  1 to 60). An event over the cap is recorded `capped` and dropped. Only an
  event something is bound to counts (`unbound` otherwise), and a merged PR
  bound to two projects counts once.
- **What runs.** An automation whose `triggers:` names the kind
  (`tracker-issue:<workspace>`, `pr-merged:<workspace>`, the bare kind for
  every project turned on, `recording-ended`) gets a trigger row keyed
  `event:<slug>:<kind>:<id>`, `{{trigger_reason}}` quoting the event; the
  evaluator parses these tokens (`TriggerSpec::is_event`) but never polls
  them. The default binding is a project's **Wake the lead for new issues**
  (its right-click menu, tracked projects only, never a build workspace; its
  own switch and time, apart from the project's trigger, so the editor's
  Turn off leaves it and it takes only issues created after it was turned
  on): an unattended turn in the lead's current conversation quoting the issue's
  key, title, Hollow note and link, `host_asker` `hollow:event`. It queues
  behind an open question card and never closes one; the pump's CFO budget
  check holds it like any unattended turn. A busy lead or a passing failure
  keeps the wake pending (several issues join one turn); a wake pending 6
  hours (its own age), whose switch went off, or permanently refused (no
  lead, a moved conversation, an unsafe folder) is dropped and logged.
- **Where.** The automation editor's **When: Schedule or Event** (kind and
  project; saving turns that trigger on, Turn off beside its state), the
  project menu's toggle, and the agent actions
  `automations.event_triggers_status` (read) and
  `automations.set_event_trigger` (write; refused from an automation's run).
- **Pass.** Each minute from the queue's owner: the ledger when it changed;
  `gh`, recordings and the calendar every 5 minutes, and only for a kind
  that is on.

### Run inside this chat (overhaul F1)

An automation can be bound to one conversation (`in_chat: <session key>` in
its frontmatter; `automation_in_chat.rs`). Each run is then a new turn in that
conversation instead of a new chat:

- **Where it goes:** the bound key's handover chain's current conversation
  (`chat_chain::current_of`), so a chat that handed over keeps its schedule.
- **How it runs:** the run is admitted, stamped and budgeted like any run, then
  queues one chat turn and ends (`workflows::execute`'s first branch). The turn
  is the prompt under the line "Scheduled run of the automation “Title”.", sent
  as Hollow's own unattended turn (`asked_by` `hollow:automation`), by the
  chat's own agent and model in the chat's own last mode, lowered when the
  automation's file names a weaker mode, acceptEdits included; a file naming
  none never lowers it (`automation_in_chat::turn_mode`, `named_mode`; capped
  at acceptEdits until 2026-11-12).
  The automation's agent, model and folder are not used. The run log's note says it was queued or is waiting.
- **Prompt variables:** `{{date}}`, `{{now}}`, `{{last_run_at}}`,
  `{{workspace_title}}` and `{{trigger_reason}}` expand before the turn is queued,
  with the same `template_vars` and `compose_prompt` a normal run uses
  (`automation_in_chat::expand_prompt`), so the turn reads like an unbound run's
  prompt. Outside text is tagged `<hollow-var>` and the one-line data note follows
  the prompt. A message that waits keeps the text it was queued with.
- **Behind a card or a turn:** being Hollow's turn it never closes an open
  question card; the pump holds it behind one (E1). A chat with a turn queued
  or running keeps the message pending in `<support>/AutomationChats/` and the
  minute pass sends it when that turn ends; one pending per automation (a newer
  run replaces it), dropped after 6 hours. A budget hold's notice is kept with
  it and is the reason it drops with; a database read that fails (busy,
  unreadable) keeps it for the next minute. A new run whose chat read fails
  the same way parks its message pending too (run log: "Waiting: Hollow
  couldn't read the chat ..."), instead of failing as a never-retried
  Prepare failure (2026-11-13).
- **Fails visibly:** a chat that was deleted or archived, or that sits in a
  build workspace, fails the run with that reason in its run log and the usual
  failure notification; a pending message whose chat went the same way is
  dropped with an `error` run-log row. One whose automation was deleted or
  unbound is dropped quietly.
- **Tools:** a chat turn uses the chat's own tools, so a bound automation can't
  keep an allowlist or denylist: the save refuses one, and a run of a
  hand-edited file fails with the same reason.
- **Budget:** the turns' usage counts toward the automation's own usage budget
  (`automation_in_chat::turn_runs`, read by `budgets::scope_rows`); each usage
  row stays the chat's for every other figure. A hard-stop budget fails the
  run before it sends and keeps a waiting message waiting.
- **Never twice:** each send carries its run's id as the chat row's
  `clientTurnId` (`auto-<run id>`), which the row keeps after its turn ends
  (`run_queue`'s terminal update keeps it beside `unattended` for `auto-` ids
  only), so a waiting message whose entry outlived its send (Hollow stopped in
  between) is recognised and dropped, not resent, even once that turn finished.
  The check searches every chat row since the message started waiting
  (`run_queue::chat_row_with_turn_id`), so a send whose chat handed over since is
  still found, and runs before the 6 hour cap, so a send that landed before a
  long outage isn't logged as late. If the queue can't be read for that check,
  the message is kept until 12 hours (`CHECK_CEILING_HOURS`), then dropped with
  an `error` row saying so. The queue's 500-row cap never prunes an `auto-` chat
  row requested within those 12 hours.
  The minute pass re-reads a message under its automation's lock, so a Run now
  that replaced it meanwhile wins. A waiting message is dropped quietly only once
  its automation file is known gone (`try_exists`) or reads with another
  binding; an unreadable file, or an error checking it, keeps it until the 6
  hours pass, when it drops with its `error` row.
- **Approval:** the binding is in the permission stamp (`inChat`, omitted when
  unbound so older stamps keep their digest); binding to a chat, or moving to
  another one, is a widening change that needs approval like a new trigger.
- **Click paths:** a conversation's ⋯ menu, **Schedule in this chat…**, opens a
  new automation bound to it (no workspace needed); the editor's Inputs &
  scope has **Runs in** (New chat each run / This chat: title). Agents pass
  `inChat` to `save_automation`: a session key, `"this"` for the calling chat,
  or empty to unbind; the card shows Runs in.

### Automation end rules (G2)

An automation can carry one end rule in its frontmatter: `ends:` (an RFC 3339
instant; a save also takes a local `YYYY-MM-DDTHH:MM`, stored as RFC 3339) or
`after_runs: N`. Old files have neither and are byte-unchanged by a save that
doesn't send them. `runs_counted:` (runs finished since the rule was set) and
`ended_note:` are bookkeeping the app writes.

- **Ending** (`vault_workflows::end_if_due`, `apply_end`): `schedule: manual`,
  `next_run_at` cleared, the rule keys cleared, `ended_note: Ended <UTC time>
  because <reason>. …`. It is an owned write under the permission write guard
  that re-stamps the schedule (`after_schedule_write_locked`, like
  `set_schedule`), so the ended automation still runs by hand. Triggers are a
  stamped permission field and are left alone; `ended_note` is what keeps
  `enqueue_due`'s polled triggers and `event_triggers::workflow_bindings` from
  firing it.
- **By time:** step 2b of `workflows::enqueue_due(vault, now)` (after the cron
  step, which still queues an occurrence due at or before the end) ends every
  automation whose `ends` is at or before `now` (the injected clock; tests pass
  it) under the scheduler's bookkeeping lease.
  If the vault is busy, an automation past its end is simply not enqueued
  (`end_passed`) and ends on a later pass. A run already queued or running when
  the end passes is not cancelled.
- **By count:** `finalize_run_schedule` counts every finished run except a cancelled one (scheduled,
  triggered or Run now), inside the locked write and ends the automation when the count reaches
  `after_runs`, in the same write as the run log.
- **Save** (`check_end_input`, `apply_end_input`): a readable date in the
  future, not both rules, `after_runs` at most 10,000, or the save is refused
  with nothing written. Omitted keeps the saved rule, `ends: ""` or
  `afterRuns: 0` clears it, setting one replaces the other, a changed rule
  restarts the count and clears `ended_note`; so does any end field a save
  carries (even cleared), a changed trigger list, or a schedule other than
  `manual` (a trigger-only automation takes a new rule or a
  schedule to wake again). Both keys are in the edit digest, not the permission
  stamp (an end only ever runs less).
- **Surfaces:** `Workflow` carries `ends`, `afterRuns`, `runsCounted` and
  `endedNote` (each omitted when unset); `save_automation` takes `ends` and
  `afterRuns`. The editor's Schedule section has Ends at (datetime-local) and
  Or after this many runs, sent only when changed (`workflowEnds.ts`
  `endsForSave`), and the list row shows `endsLabel` ("ends Fri 6 PM", "ends
  after 3 runs (1 done)", "ended"). The new-automation form has no end fields
  yet: save, then edit.

### Owner Mac (M4, `runs_on`)

`runs_on:` names the one Mac whose scheduler fires an automation, so a
`workflows/` folder synced to two Macs never runs one twice (`runs_on.rs`): a
machine id (`m-` and 16 hex, stable; what Hollow writes) or a machine slug
(`machine.json`'s `slug`, which a person can rename). The rule
(`runs_on::decide`):

1. It names this Mac (its id or slug): it fires here.
2. It names another Mac's id, or a paired Mac's slug: it runs there, never
   here.
3. It names nothing this Mac knows (malformed, or a slug that isn't this
   Mac's or a paired Mac's): it fires nowhere. This stops only that
   automation.
4. No owner: it fires here only while no other Mac has been seen (no Mac
   paired either way over M2's peer link, and no automation in the folder
   owned by another Mac's id or a paired Mac's slug). A malformed or unknown
   owner is not evidence, and neither is a sync remote (it may be a
   backup). Once another Mac is seen, an ownerless automation fires nowhere.
   Two *unpaired* Macs on one synced folder each think they are alone, so
   a lone ownerless automation also goes through a claim tiebreak
   (`runs_on_claim.rs`): each Mac writes only its own
   `workflows/claims/<slug>.<machine id>.json` (`since`, `beat`); a claim counts
   after a 90 s window and while its beat is under 30 min old (renewed every
   5 min); a Mac fires only with a matured claim and no live matured claim of a
   lower machine id. One up (no other live claim): it fires at once. Late joiner: the
   incumbent fires until the newcomer matures, then the lowest id holds. Two unpaired Macs starting
   before either claim has synced can both fire until the claims arrive. A Mac
   with no id, or that can't write its claim, fires as before. Owned and `manual`
   automations never touch a claim; claim files for deleted, owned or manual
   automations, or unrenewed for 3 days, are pruned.

The automations a scheduler pass newly finds in cases 3 and 4 are one Hollow
notification for the pass ("N automations won't run on any Mac"); each is
reported once per run of Hollow (not across restarts). `Workflow.runsOn` is
on the wire; the editor and the list show it (below).

- **Checked by** `enqueue_due` and the startup repair (they list only
  automations that fire here, so heal, cron, end and polled triggers never
  touch another Mac's file), `event_triggers::tick` (bindings), and the claim
  of a cron, trigger or retry row (`run_pump::claim_verdict`, the same rule on
  the folder as it is then: a row whose automation was moved or became
  ownerless in a shared vault since it was queued ends skipped, writing
  nothing). Run now runs where it is clicked. Wakes and in-chat pending turns
  are per Mac and can't double-fire. This Mac (`Seat::current`: its id, slug
  and paired Macs) is read at most once a minute (`runs_on::SEAT`) and
  forgotten on a slug change, a pairing or an unpairing (`peer_machines`
  upsert and forget, `phone_access` store `pair_machine` and `revoke`). A read
  that can't find this Mac's id is not kept and the pass fires as before M4
  (ownerless automations run; no notification). The claim reads the folder
  at most every 10 seconds per vault and seat.
- **Stamped, because the Macs differ in power.** The Mac mini holds the
  person's own GitHub, Google Cloud, Linear and Claude logins, so moving an
  automation there grants it more than it had here. `runs_on` is in the
  permission stamp (`PermissionFields::runs_on`, omitted when unset so older
  stamps keep their digest): a hand or synced edit to it is held for review
  like any stamped change, and gaining or changing an owner counts as an
  escalation. A save through Hollow re-stamps it, and the save card shows
  Runs on.
- **Save:** `save_automation` takes `runsOn` (omitted keeps it, empty
  removes it, `"this"` is this Mac's id (its slug when it has no id), else a
  slug or id; anything else is refused). A new automation with no `runsOn` gets this
  Mac's id only once another Mac shares the vault; alone it has no owner, as
  before.
- **Editor and list** (M4 follow-up, 2026-11-15): the editor's Inputs &
  scope has **Runs on** (`pages/workflows/RunsOn.tsx`): Any Mac that holds it
  (until you pair another Mac), This Mac (sent as `this`), or each paired Mac
  (sent as its machine id; a paired Mac saved by slug keeps the slug), plus
  the saved owner when it is none of those ("Unknown Mac ... (runs
  nowhere)"). It is sent only when changed; empty removes the owner. The
  hint says what the choice means; with no owner in a shared vault it says
  it won't run on any Mac until it has one. A new or changed owner is in the
  editor's stamp (`StampedFields.runsOn`, "Runs on: A → B" in Allowed
  effects) and widens access as the backend counts it (gaining or changing
  an owner, not removing it), so Save asks to approve it. A list row whose
  automation runs elsewhere says "Runs on <Mac>"; one that runs on no Mac
  says why and "Open Edit and set Runs on". The page mirrors the rule in
  `pages/workflows/runsOnChoice.ts` from this Mac and its paired Macs, read
  when Automations opens (`peer_machines`, the store the Macs panel uses,
  plus `phone_access_status`'s `machines` for Macs paired with this one as
  a server, read again on `phone-access-changed`); until they are read a
  row shows no note and the hint for a paired or unknown owner reads
  "Reading your Macs" (never a false warning). The new-automation
  form has no Runs on: the backend's default applies (this Mac once the
  vault is shared); set it after the first save.
- **A paired Mac renames itself** (its `GET /v1/machine` slug differs from the
  record; an absent slug or one another Mac holds is ignored): `peer_machines::refresh_slug` updates the record, keeps the old slug
  as an alias for 7 days (`old_slugs`, at most 4; `Peer::aliases`, so an owner
  stamp or route naming it still resolves), and
  `runs_on::after_peer_rename` re-stamps owners naming the old slug to the
  peer's machine id.
- **Renaming this Mac** (Settings, Context, Home: `set_context_machine_slug`)
  moves every automation whose `runs_on` named the old slug to the new one,
  or to this Mac's id when the slug is cleared (`runs_on::after_slug_change`,
  `vault_workflows::set_owner`), moving only the owner in an otherwise
  matching stamp. An id-named automation never needed it. A `machine.json`
  that can't be read leaves this Mac with no id or slug: its owned
  automations then run nowhere and say so.

### Automation form continuity

New automation initializes defaults once and applies another template only on
its card click. Typing a name never resets Schedule or refills cleared fields;
the chosen schedule is also written beside Create automation. Context-menu Edit
opens the editor; the run review keeps Edit in Automation options.

Both editor forms register a local navigation guard. Back, another automation,
New, view changes, page/history navigation and workspace opening check the full
draft before changing destination state. Keep editing preserves it; Discard
covers the exact form revision shown. A later edit, newer navigation request or
disposed form invalidates that choice. During a save, navigation is refused with
a visible notice; click the destination again after saving. Clean forms navigate
immediately. No new keyboard shortcut is added.

Choosing the current page or form preserves it without a discard prompt.
An automation-open request is consumed before navigation once its row is found;
list refreshes cannot replay it after either acceptance or Keep editing.
An unmatched request waits for the matching automation to become available.

An initial automation-list failure shows an error and Retry in the aside and
viewer, never an empty collection or zero count. A failed later refresh keeps
the earlier list and its existing refresh notice.

### Launch at login + close-to-tray

- **Run at login** (`tauri-plugin-autostart`) and **Keep running on close**
  (`lifecycle.rs`): closing `main` hides it and keeps schedulers alive in the
  tray.

### Scheduled routines (reuse the workflow scheduler)

Routines (morning brief, plan or close the day) are ordinary automations the
user makes; a workflow may carry a `routine:` frontmatter marker, and Hollow
seeds none. `vault_workflows::set_schedule` line-patches `schedule` and
`next_run_at`.

A schedule `vault_workflows::validate_schedule` refuses (a broken cron, an
unknown window or catch-up) never fires: the scheduler and the pump skip it.
`build_workflow` puts that error on the workflow as `scheduleProblem` (omitted
when it reads), so `list_workflows` and `read_workflow` agree, and Automations
says "Schedule can't be read, so this won't run" in the row and the details,
with the error under Details (B04).

### Workflow permission stamps (Paperclip PR 5)

An automated run must not be able to escalate its own automation. The code is
`workflow_permissions.rs` (stamps, verify, approve) and `run_guard.rs`
(host-write attribution, post-run flags).

Each workflow's permission fields (mode, allowed and disallowed tools,
working folder, agent, model, triggers, schedule and the prompt's sha256) are
stamped outside the vault. `workflows::precheck`
calls `workflow_permissions::verify` for every admission, from every entry
path; a workflow changed outside the automation editor is refused until the
change is approved in Waiting on you (Automations → Permission changes).
Existing automations were trusted once at first launch; a file deleted outside
the editor leaves a tombstone. Workflow vault writers hold
`workflow_permissions::write_guard()`. A run's own edits of `me/`, agent
instructions, agent config, watched dot-state or its History record are
flagged on its History record (`run_guard.rs`). Since PER-36 (2026-11-16)
only some flags notify, batched into one notification per run whose body
names the workspace (never a chat's session uuid): an ordinary edit of a
context doc (`CLAUDE.md`, `AGENTS.md`, `BUILD_LOG.md`, `WORKSPACE.md`,
`PLAN.md` at the vault root or directly in a workspace) sends nothing at all,
not even a bell item (History lists it; the one-item batching applies only to
what still alerts); it toasts
only when it lost more than half its bytes in the run (a doc of at least
`CLOBBER_MIN_BYTES`, 256 bytes; a delete loses all) or History holds no
restorable copy (a side History captured without a size counts as none).
Workspace names in the body come from the agent-editable brief, so control
characters are dropped and the name is cut to 60 characters. `STANDING_ORDERS.md`, `me/`, the instruction overrides,
agent settings and hooks, workflows, watched state and the run's own record
always toast (source `run-guard`); shared-doc notes alone are a bell item
(source `run-guard-docs`).

### Automation queue (Paperclip PR 7a)

Every automation that wants the full-vault lease is a row on one durable
queue first, and ONE pump admits rows one at a time. Code: `run_queue.rs`
(storage), `run_pump.rs` (claim and admission), `run_permit.rs` (the
cross-process permit), `workflows::enqueue_due` / `heartbeat::enqueue_if_due`
(the enqueuers), `automated_history::RunLease`.

Workflow request provenance is retained separately by `work_origin.rs` in the
same operational store. New person Run now requests have distinct host identities;
verified Cron/Trigger events are autonomous, with each occurrence retained through
coalescing. Broker requests and retries inherit exact proven roots. Manual source,
priority, permission cards and payload labels do not prove direct origin. Unknown
legacy rows remain unknown. Incompatible roots do not coalesce or absorb siblings.
Proven manual requests preserve unevaluated trigger fires. Completion advances
only an owned or compatibly absorbed Cron occurrence, with an exact saved-due-time
comparison inside the workflow mutation; an old retry cannot consume a newer one.
Legacy Unknown watermark behavior remains, but advancing a schedule still needs
an owned or explicitly absorbed Cron occurrence. Retained terminal receipts let
unconsumed interrupted scheduler events bind replacement attempts after queue
pruning, preserving original request and prior execution identities.
Retained absorption scope/due receipts also let the receiving run's positive
terminal transaction mark Cron work consumed after child pruning. A failed
workflow-file update then uses the existing completed-occurrence healing path;
interruption does not create that marker.
Validated direct workflow lineage also gives advisory Run now-band ordering,
ahead of other rows in that band with FIFO ties. Queue selection, live/list
positions, chat enqueue positions and lane-first choice share that order.
Persisted priority still owns automation-gap eligibility; retry delays, holds,
permission checks, budgets and capacity remain unchanged. Ordinary chat retains
its earlier band, and running work is not preempted. Bulk lineage reads use the
queue snapshot and release SQL before History/eligibility callbacks. Unknown or
damaged evidence grants no promotion and does not block unrelated chat. This
workflow-specific ordering does not extend CFO allocation to other launch paths.

- **Storage:** the `run_queue` table in the Sessions DB (sources `chat`,
  `cron`, `manual`, `trigger`, `retry`, `heartbeat`, `ingest`). **The pump**
  (`run_pump::step`) claims one row at a time; claim rules are the pure
  `decide_claim`, then the permission check, then the budget preflight last.
  A chat row goes first but never preempts a running automation. An open
  question card, or an unanswered ask-canvas in that conversation's workspace,
  holds a wake, check-in, pulse or scheduled turn for 30 minutes
  (`interactions::open_hold_until`). The pump sleeps until then and does not
  stamp `not_before`, so an answer before the cap lets the next step start.
  Only the person's own send closes the card, and that send may queue beside
  the held turn even after Send has already resolved the card. The queue list
  names that wait `an open question card` even when the row already has a
  lease wait or a permission hold, and the composer does not adopt it, so
  the card's Send stays enabled. A card on an earlier chat in the
  same chain holds the current conversation and closes when the person sends
  there. App-data ask-canvases (a chat with no workspace) are not
  scanned. Behind
  another writer's lease it steps again on an enqueue, a stop, a lease
  release (at most once per 250 ms, `LEASE_WAKE_MIN`, S14) or every 5 s
  (`BUSY_POLL`). While a full-vault dispatch runs on its own lane, scoped
  chat rows (build and workspace turns) can still start beside it.
- **One budget (2026-10-20):** every agent turn counts against Settings →
  Chat → Chats at once (`chat_limits.rs`, `chat-limits.json` in the
  app-data folder; the COO changes them with `settings.set_chat_limits`, a
  confirm action: `chatsAtOnce`, `chatsPerWorkspace`, `subagentsPerChat`):
  **Chats running at
  once** (1-25, default 10) across Hollow, and **Chats at once in one
  workspace** (1-10, default 5) for shared turns in one workspace's own
  folder (the ceilings rose from 10 and 5 on 2026-11-07; the defaults and the
  memory floor did not move: `chat_limits.rs` `MAX_*` against `DEFAULT_*`,
  and `src/chatLimits.ts` `CHAT_LIMIT_MAX` keeps in step). A third,
  **Subagents per chat** (1-10, default 6; 2026-11-12), is how many of one
  conversation's subagents (the sessions it starts with `start_session`,
  payload `parent`) run at once; a further start queues
  (`Hold::ParentLimit`, checked first in `QueueFacts::hold_beyond_memory`
  from `ChatLimits::subagents_per_chat`). It was the fixed
  `app_sessions::MAX_RUNNING_PER_PARENT` = 6 before, a hidden limit
  that queued a COO's starts while far fewer than the overall limit ran.
  Swarm parts keep their own count (`swarm_concurrency`, Settings →
  Background work → Swarm parts at once): one `sessions.swarm` call fans one goal
  out on a cheaper model, with two at once in build workspaces, so it is not
  a chat's helper count. It always sits under the app-wide limit. The first number
  counts scoped chat turns, shared and exclusive (live
  `chat` leases, `automated_history::scoped_turn_counts`; a workspace
  archive's lease is not a turn), swarm parts, durable children and
  handover successors (all ordinary chat rows), and the vault lane's one
  run (still one at a time). A swarm part in an ordinary workspace also
  stays within `swarm_concurrency` (`main_agent::swarm_part_limit`); a build
  worktree still takes one turn at a time. `ask_agent` consults keep their
  own two permits and in-process subagents are not turns. While macOS memory
  pressure is warning or worse (`run_permit::memory_pressure_level`,
  `kern.memorystatus_vm_pressure_level` >= 2), SE3 allows the first three
  turns under the other admission rules and holds additional starts.
  It looks again every second (`MEMORY_RECHECK_SECS`) or on a queue wake;
  running turns go on. The selected head row's phase timings retain the latest
  `memoryPressureLevel` and first `memoryHeldSince`, preserved at admission.
  The rule is `run_pump::QueueFacts::hold`, in this order: memory, the
  folder, the workspace limit, the swarm limit, the vault lane, fairness,
  the effective app-wide ceiling. Fairness: a scoped row that would take the app-wide
  limit's last free slot (only when exactly one is free) yields while a
  queued vault-lane row is ready and has waited `LANE_FAIRNESS_SECS` (60 s,
  counted from the later of its request, its retry time and the automation
  gap opening) or is a Run now, no lease is held now (a row's past wait
  episode doesn't count) and no hold or vault-lane run keeps it out
  (`run_pump::lane_first`); otherwise a steady backlog of priority-0 chats
  would starve automations. A wait on a child its caller's turn alone keeps
  out of a limit answers at once (`run_pump::waits_only_for_slot_of`, which
  checks occupancy rather than the displayed reason; `descendants::behind_note`).
- **Stop always stops (R8, 2026-11-14):** a Stop on a chat row the pump has
  claimed but not started sets `cancel_requested`. If that turn then loses
  admission to another run (`ChatAdmission::Deferred`), `admit_chat` ends it
  `cancelled` with `cancelled{queued}` and `done` instead of queueing it
  behind that run (C-S3). A chat row that still goes back to `queued` with
  the flag (a failed write, a stray-claim release) waits for nothing: the
  pump's `startable`, the advisory lease check, the memory gate and
  `next_ready_where`'s not-before and gap all let it through
  (`run_pump::ends_on_claim`), and its claim ends it (`decide_claim`). A
  cron row's skip keeps its holds: its bookkeeping needs the lease.
- **Run permit** (`run_permit.rs`): at most 2 agent CLIs that do not hold the
  lease (one-shots, team-mcp delegates) run at once, machine-wide, and none
  start while `node` processes reach `CLAUDE_NODE_PROC_LIMIT` (default 60) or
  macOS memory pressure is at warning or worse.
- **Only one process owns the queue** (the pump-owner lock); startup ends
  what a dead Hollow's runs left and recovers their rows.
- **UI:** `list_run_queue` / `cancel_queued_run`, `run-queue-event`, the
  dock's `Queued (n)` chip, Settings → Background work → Run queue.
- **Why a queued row waits** (`run_pump::waiting_reason`, overnight A2): when
  no lease wait or hold says so, the list and a send's `enqueued` event carry
  `waitingReason` (the pump's own rule, `QueueFacts::hold`: "memory", what
  holds its folder named by the blocker's kind, then each limit by its number
  and its setting: "a free chat slot: this workspace is at its limit of 5
  chats at once (Settings → Chat → Chats at once)", "a free swarm slot: the
  swarm is at its limit of 6 parts at once (Settings → Background work → Swarm
  parts at once)", "a free subagent slot: this chat is at its limit of 6
  subagents at once (Settings → Chat → Chats at once)", "a free chat slot:
  Hollow is at its limit of 10 chats at once (…)" (the limit, not a count: a
  lowered limit can sit under what still runs), the vault lane's running row, "<automation> to start first" (fairness); a retry time; not
  the automation gap), and
  `SendAccepted.waitingFor` falls back to it, so a queued turn reads
  "Queued · waiting for …" rather than "Queued". Other events leave it unset.
  Working, not Queued (2026-11-16): `RunQueueItem.working` (`Some(true)` or
  absent) is true for a claimed or running row and for a queued chat row with
  no real wait (`run_pump::waiting`: no `Hold`, no retry time, no named lease
  holder; a tracker sync, scheduler, workspace edit, companion or history
  clear lease is the app's own bookkeeping and is not shown); the Queued badge,
  the dock's count, `SubagentRow.status` and `sessions.overview` treat it as
  working, and `chat::queued_live_session` reports phase `preparing`.
  `start_session` and `send_to_session` return `state: "started"` and a note
  that opens "Started." when the row is not held, and `state: "queued"` with
  a note that opens "Queued: waiting for <reason>." only for a real wait
  (`app_sessions::started_state`/`started_note`); the result's other keys are
  unchanged. Since 2026-11-17 the phone API reads the same item: a chat it
  reads Working is `running` (its wire has no Preparing word) and only a real
  wait is `queued` (`phone_access::routes::QueueView`, `accepted_state`). In a
  process that doesn't run the vault's queue (`QueueState::OwnedElsewhere` or
  `NotStarted`) a queued row is never Working: it waits for
  `OWNED_ELSEWHERE_WAIT` ("the Hollow process that runs this vault's queue")
  or `QUEUE_NOT_STARTED_WAIT`. A chat row with a permission `held_reason`
  reads paused in the composer in the dock's words (`queuedTurnLabel`,
  `heldLine`; the working line says Paused), never Preparing.
  A Subagents tab row carries the same words as `SubagentRow.waitingFor`
  (`subagents::queue_places`, from `run_pump::item_with_reason`), shown on
  the row's quiet line and header ("Waiting for a free subagent slot: …").

### Failure streak and notify (Paperclip PR 9)

The third consecutive failed run opens one failing-automation item (Pause
schedule / Dismiss) in Waiting on you; `notify:` (`always` | `failures` |
`never`) filters per-run notifications; prompt variables (`{{date}}`,
`{{workspace_title}}`, `{{trigger_reason}}`, …) are expanded at spawn
(`workflow_hygiene.rs`). Budget stops are not failures.

### Quiet skip, inactive skip and retry (Paperclip PR 9b)

For cron rows only, at the claim: a workflow whose workspace is inactive is
skipped, and `skip_if_quiet` + `watch` skip an occurrence when nothing watched
changed since the last successful run (every doubt runs it). A transient
failure of a cron or trigger run gets ONE bounded retry a minute later
(`workflow_retry.rs`); refusals and denials are never retried, and neither is
a run whose receipt shows it may have acted outside its files or ran calls
Hollow can't see into (plan completion L2). The run review's "Run again
without repeating" is the guarded manual retry: Hollow's own executors refuse
the exact calls the failed run already made. See
[AUTOMATION_RECOVERY.md](../AUTOMATION_RECOVERY.md).

### Precheck command (OC13, `workflow_precheck.rs`)

An automation may carry `precheck:`, a one-line shell command (300 characters
at most, no control characters). Before a SCHEDULED occurrence is admitted
(cron rows only, like the quiet and inactive gates: Run now, trigger and retry
rows are not prechecked) `workflow_hygiene::precheck_gate` runs it, after the
inactive and quiet checks (`cron_gate`) so nothing starts a process for a run
those already skip. **It runs off the claim thread** (`workflow_precheck::poll`,
one thread per occurrence): the claim never waits on it. While it runs the row
goes back to `queued` for `PRECHECK_POLL_SECS` (1 s) and is asked again, other
rows are claimed meanwhile, and the verdict is collected on a later tick. Exit
0 admits the turn. A clean non-zero exit (not 126 or 127, not a signal) skips
it: one `skipped` run-log row whose note starts `skipped_precheck:`
(consecutive skips fold into one row with a `· N skips` count), `next_run_at`
advances, the queue row ends `skipped`, no history run and no agent turn, and
no budget is touched.

- **A broken precheck is a failure, not a skip.** Exit 126 or 127 (misspelt or
  not executable), a signal kill, a spawn failure (its working folder is gone),
  a 20 s timeout, a stop and a changed script write an `error` run-log row
  (`precheck failed: the precheck could not finish (<why>)`, one per occurrence,
  never coalesced), end the queue row `failed`, advance `next_run_at`, and count
  toward the failure streak: three in a row open the Workflows failing card and
  its notification. Each failure also records one in-app notice (not with
  `notify: never`). Only a clean non-zero exit is benign. (Quiet skip is the
  opposite, "any doubt runs it", because it only saves money.)
- **Stop.** Skipping the queued scheduled row (or the pump stopping,
  `drain` ending) kills the precheck child's process group by PID
  (`workflow_precheck::cancel` / `stop_all`) and no new one starts while stopped.
- **The script is part of the approval.** If the command's first word (or the
  word after `sh`, `bash`, `python3`, `node`) is a path to a file inside the
  working folder, the stamp pins that file's path and sha256
  (`Stamp.precheck_script`, set by every approval and editor save). A script
  whose hash differs, or that was never pinned, is refused at the gate as a
  broken precheck; saving the automation in the editor approves the new content.
  Only the person's own editor save (`write_workflow` from the UI, `upsert_input`) or a new file re-pins; the agent's `automations.save` uses `upsert_input_by(.., true)` and `approve_app_save` never pins either: with the flag `put_stamp` never hashes a script, it carries the old pin only while the command and folder are unchanged (a new file, a changed command or a missing stamp leaves it empty), and the gate refuses with "save the automation in the editor to approve this script"; a review-card, app
  card or rename approval keeps the old pin while the command and folder are
  unchanged, so an agent cannot get an edited script approved alongside an
  unrelated change. Flags before the script (`bash -o pipefail ./x.sh`) are
  skipped; `-c`, `-m` and (node, ruby) `-e` mean no script file. The approval card says when
  the command names a path outside the working folder, when no folder is set (the vault is
  used), and when it chains commands or runs inline code with a path in it (only the first
  script is pinned). Still not
  covered: a program found on PATH, a variable or subshell, a script that sources
  another file, and an edit between the hash check and the exec. A panic in the
  precheck thread settles as a failure, and the child's PID is cleared once it is
  reaped so a late stop cannot signal a reused PID.
- **Limits.** Hollow has no per-command tier or sandbox for a shell outside an
  agent turn, so this is not a sandbox. It runs `/bin/sh -c` as the person's
  user with a scrubbed environment (`PATH`, `HOME`, `LANG` only: no keys, no
  proxy or `HOLLOW_*` variables), the automation's working folder as cwd, no
  stdin, its own process group ended on exit, timeout or stop, and output read
  dropped (stdout and stderr go to null; never stored). It has its own bounded
  runner (`workflow_precheck::run_with`, 50 ms poll, cancel flag, published PID).
- **Who may set it.** `precheck` is a stamped permission field
  (`PermissionFields.precheck`, omitted when unset so older stamps keep their
  digest): an agent, a run or a hand edit that sets or changes it refuses the
  automation until the person approves it in Permission changes (an
  escalation; removing one is not). The automation editor's "Check before each
  scheduled run" field is the owned writer: it sends the value only when
  changed, and saving asks for the usual approval. The agent action
  `automations.save` has no such argument (its validator refuses it and
  `workflow_input` strips it), and its card shows the saved command as
  "Precheck". The gate runs the verified workflow's command, the one approved.

### Silent agentic heartbeat (`heartbeat.rs`)

A 10-minute timer (started only by the queue owner) enqueues a heartbeat row
when its gates pass (enabled, work hours, interval, daily budget, a change);
the agent pass runs in the vault, and its outputs are proposals only (tasks as
`heartbeat_task` review items, deduplicated against existing tasks). Its state
lives outside the vault. A beat has a fixed 300 s wall clock
(`RUN_TIMEOUT_SECS`), so its tool pass takes the saved reasoning effort held
at or below `high` (`BEAT_EFFORT_CEILING`, `effort::at_most`); a lower saved
level is kept. At a saved `max`, both beats on 2026-10-27 made 26-37 model
calls and timed out with nothing returned. Settings → Models → Self-hosted
models → Background jobs can send the heartbeat to a self-hosted model
(self-hosted models Q10): `prepare_beat` decides before the lease and the
charge (an asleep model queues the beat; a switched-off one, or a choices
file that can't be read, skips it; each noted in the heartbeat log), and
`run_beat` attaches the forwarder and a key for the beat. A queued beat runs
inside the drain that woke its model (`run_queued_beat`, the scheduled gates
and admission through `gate_queued_beat`), and only while still due
(`still_due`). It is pinned to that model: if the heartbeat was sent back to
As now during the wake, `prepare_beat` holds it, and a beat that couldn't be
prepared is an error, so it stays queued rather than leaving as done; see
[agents](agents.md#self-hosted-models).

### Settings is now true section-switching

`Settings.tsx` renders only the active section (with all its parts, S2);
sections, parts, ids and search terms live in `src/settingsSections.ts`
(pinned by `settingsSections.test.ts`), and `resolveSettingsSection` maps
every older section id to the section that holds it now. Rows put the label
left and the control right.

Archive: automation runner, queue, permission stamps, heartbeat and Settings;
accounts, connectors and onboarding detail are in the
agents archive.

---
