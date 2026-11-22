# Scheduler v2 (master plan 2026-10, F1)

> **Removed 11-10 (S6).** The Tonight (night) queue: its Tauri commands (`night_queue_*`), agent actions (`automations.night_queue_*`), the drain and the morning report are gone, with `runbooks.rs` and the bake-off runner. `night_queue.rs` keeps only `NightSpend`/`SPEND` (the mid-turn guard, inert without a published reading), `spent_tonight`/`night_budget`/`budget_read` (read by `budgets` and `wake_me`), `left_on_runner`, and the `night_queue` table's schema (still created, inert). The sections below on queueing, draining and the morning report describe the removed feature.


What runs on its own, and when. The modules' `//!` docs are the authority:
`schedule_windows.rs` (windows, catch-up, window jobs, the minute timer) and
`night_queue.rs`. Cron automations, triggers, the heartbeat and the Main
agent's check-ins are unchanged; see the main [AGENTS.md](../AGENTS.md).

## Named windows

Defined once in `<app support>/Scheduler/windows.json` (defaults until the
person saves; `automation_windows` reads them, `set_automation_windows` is the
person's only). Wall-clock time in this Mac's zone; `automation_windows`
names it (`timeZone`).

| Window | Days | Hours | One occurrence counts as |
|---|---|---|---|
| `tonight` | Mon–Fri | 18:00–07:00 (next day) | a weekday |
| `weekday-evening` | Mon–Fri | 18:00–22:00 | a weekday |
| `friday-pm` | Fri | 15:00–18:00 | an ISO week |
| `nightly` | every day | 02:00–05:00 | a day |

DST: a start inside the spring-forward gap moves to the first minute that
exists (`nightly` on 2026-03-08 starts 03:00 EDT); a start inside the repeated
autumn hour takes the first. Tests use a hand-written US Eastern zone
(`schedule_windows::tests::Eastern`), not the process's `TZ`.

The overnight hard stop (`overnightCapUsd`, $40, decision 7) lives in the same
file. `$0` refuses every night run.

**Editing (overnight S10).** Automations → **Automation windows and the run queue**
(`src/pages/workflows/AutomationWindowsPanel.tsx`, lazy; model in
`src/automationWindows.ts`; moved from Settings in PR 4) reads `automation_windows` when the fold opens
(never at launch) and saves with `set_automation_windows`. Each window shows its
label, days, start and end in the zone `automation_windows` names; the editor
checks what `WindowDef::problem` / `WindowSettings::validated` would refuse
before sending, and puts a refusal the backend still returns next to the window
or the cap it names (unsaved edits stay; a refetch never replaces a dirty
draft). The four built-ins keep their names and can't be removed in the editor,
since `JOBS`, the night queue and `wake_me` look them up by name (the backend
itself accepts any valid set); a custom window gets its name from its label
until typed, and the name is fixed once saved because automations say
`window:<name>`. A changed window's label is composed again as
`Title (days HH:MM–HH:MM)` so automations never show old hours; an unchanged one
keeps its saved label byte for byte. The overnight cap is edited in Settings →
Usage → Budgets since P1-07 (its row there saves through the same
`set_automation_windows` with the stored windows unchanged); it is $0–$1,000 and
the row says when it is above the $40 hard stop (`night_budget` uses the lower).
A window automations run on shows how many (`list_workflows`, read when the
panel opens) and Remove asks inline first. Edits live in the panel only, so
leaving the section drops what wasn't saved; edits typed while a save runs are
kept, and a read that started before the last save is ignored. Automations name
window schedules with the saved labels (`src/windowLabels.ts`, a signal) once the
editor, the Automations page (read when it opens) or the COO's queue line
has read them.

## Automations on a window

`schedule: window:friday-pm` instead of a cron. The schedule string is a
stamped permission field, so a run that rewrote its own window is refused like
any other schedule edit; `set_automation_window` (confirm) uses the owned
`set_schedule` write, which keeps an approved automation approved.
`next_run_at` is the window's next start; the frontend shows the window's
plain-English name (`workflowSchedule.ts::describeSchedule`).

The catch-up is part of the same stamped string: `window:<name>` (run-once-on-wake,
the default), `window:<name>/run-all` or `window:<name>/skip`. It decides what an
occurrence missed while Hollow was closed does: one late run, each missed one
(oldest first, at most 5 runs, `RUN_ALL_MAX`), or a skip after the misfire grace.
A cron automation has no catch-up and keeps its 120-minute grace. Catching up
applies only to an occurrence that may run: one held by a permission review, an
invalid workflow or unreadable usage keeps its grace and is skipped, so a hold
never turns into a run at the wrong hour.

## Window jobs

Host jobs (`schedule_windows::JOBS`) that run once per occurrence of a
window, decided by `decide_job` with `period.rs` keys: missed occurrences
collapse to one run under `run-once-on-wake`; keys compare only within one
shape (a window edited from one day a week to several starts afresh); a job seen for the first time
stamps occurrences that already ended instead of running them. State:
`<app support>/Scheduler/jobs.json`. The timer (`start_timer`, every minute,
queue owner only, not in a candidate unless asked) drains the night queue,
then the jobs.

Jobs, in order: `weekly-update` and `friday-status` (A18, each tracked
project's Friday status into Hollow; does nothing while `tracker.fridayStatus`
is off, the default; [Tracker sync](TRACKER_SYNC.md#status-and-canvas-copies))
on `friday-pm`, then on `nightly` the
`gardener`, the `as-of-snapshot` (F5, `asof::nightly_job`), the
`friction` recount (#13) and the `org-digest` (CX8b, `context_org_digest.rs`:
org changes merged as the owner, a note in `workspaces/main/outputs/org-changes/`
and one notification), all run-once-on-wake.

## The night queue

`night_queue` table in the session database. `night_queue_add` (write) from
the UI or any chat; the COO's own calls and automations are refused (it
may only suggest). Inside `tonight` (the overnight shift, master plan #1), up to
`NIGHT_PARALLEL` (3) items at once and at most `CODE_SLOTS` (2) taking a slot
(`uses_slot`: a code item, or one whose agent runs shell commands without asking,
i.e. anything but Claude), each an ordinary chat turn at `acceptEdits`, registered with
`main_agent::unattended`, so its Hollow app tools get the M4 limits (no builds,
installs, publishing or sends). Its turn is queued with `unattended: true`
(`chat::ChatSendRequest::unattended`): the COO's limits hold from
admission (`chat::queued_unattended`, read from the row, so a first tool call
before `register` or a restart can't escape them), and its Claude account is
picked like long work (#16: away from one at 80% or more) and fails over
inside the turn through R1. An item is claimed (`queued` → `running`) before its turn is sent, then given
its run-queue row; a claimed item never sent is failed after two minutes.
Spend per `tonight` occurrence (`night_queue::night_budget`) is the roll-up of
its turns plus what unattended runs (automations, the heartbeat and their
teammates, and turns `sessions.wake_me` woke; `budgets::unattended_spend`) spent since the occurrence began, or
the overnight window if that began earlier, so a window running past 07:00
keeps counting them (night turns are chat rows, so nothing counts twice). The
cap is the lower of `overnightCapUsd` and $40, always. At the cap nothing more
starts that night, an item whose `budgetUsd` exceeds the cap is refused, one
that doesn't fit tonight waits. Fail closed: an unreadable ledger or night
queue, or a night turn whose cost nobody reported, starts nothing more that
night (automations' unknown costs are reported, not blocking).

**Mid-turn cost guard (turn guards, 2026-10-09).** A turn's real cost is known
only when it ends, so one long night turn could overshoot the stop by its own
cost. Each running night turn now publishes an estimate (its own messages'
tokens × `usage_prices`, deliberately high: input at the cache-write rate, an
unpriced model at the dearest row) to `night_queue::SPEND`, which the drain
refreshes each tick with the committed spend, the cap, the running items'
conversations and the runs it has counted (`finished_runs`, read before the
spend, so a run ending in between counts twice for a minute, never zero
times). Entries are per run id, so a runbook's step turns (one conversation)
are each weighed. At each of its safe points (the line starting its next
main-thread model request, no tool call open, a tool result since the last
one) the turn weighs committed + every running turn's estimate; at the cap it
ends there (`Ended::LimitReached`; only the request just started is lost), its
usage row keeps the estimate as a partial cost (so the night never reads it
as unknown), its item becomes `stopped` with the note "Stopped at a safe point
mid-turn…", and the morning report is partial. At the turn's end the real cost
replaces the estimate. A running item holds back at least what it has cost so
far. Claude only: Codex reports no usage mid-turn and other agents aren't in
the price table; their unknown cost still stops the night once they end.

**The overnight shift (#1).** A code item (`kind: code`, only from a build
workspace: `night_queue_add` refuses otherwise) gets a new build workspace on
the local branch `night/<night>-<id>`, cut from its workspace's branch; the turn
runs there and nothing is pushed, opened as a PR or installed. A waiting code
item lets later non-code items start. Each running item holds back its own
`budgetUsd`, never less than `RESERVE_USD` ($5) or what it has cost so far,
from what the next may start with. Spend counts finished turns only (a running turn's cost is unknown until
it ends; its reserve holds its place). When the night's spend reaches the cap
every running item is stopped (`stopped`, its turn cancelled; a runbook run
stops before its next step; a cancel that fails is retried next tick), also
after the window has closed. A night Claude turn also weighs its running
estimate against the stop at its safe points (`NightSpend`, turn guards
2026-10-09), so one long turn no longer passes the cap by its own cost. The automations'
`overnight` scope (`budgets::overnight_spend`) counts the queue's finished
turns too, so both share one $40. An item naming a
runbook (`runbook`, a vault path; checked when added; never a build workspace)
runs its approved version for that workspace unattended (`runbooks::open_checked`), stopping at any step
that waits for the person; its step turns count toward the night's spend.
Unattended turns get no shared connectors (`connectors::for_turn`), teammates,
computer control or services (`mcp_attach::for_unattended`), so nothing a source
says can send, publish, delete or spend through one, and the Hollow app tools
keep the M4 limits whatever their arguments say; an unattended turn can't start
or steer a chat in a build workspace, and a non-code item can't name one (it
must be a code item, on its own branch). A code item whose start fails has its
new build workspace and branch removed again. **Morning:** once the
`tonight` window has closed and nothing of that night runs, the drain writes
`morning_report` to `workspaces/main/outputs/night/<night>.md` (each item's
answer, receipt files and checks, a code item's branch and `git diff --stat`,
its cost, what didn't start, and a **Partial** line when the budget cut the
night short; queued items are listed only for the latest night) and queues one
isolated COO turn (`queue_isolated_hollow_turn`: unattended, so no
connectors or teammates, since the report quotes unwatched runs) to present it;
stamped per vault as window job `night-morning-review:<vault>`; a night missed
while the thread was busy is reported before a later one. Refused items carry
the night that refused them. A runbook item stopped at a step for the person
ends `waiting`. `night_queue_morning_report`
(read; action `automations.night_queue_morning_report`) returns it any time.
The orb's label adds "N queued for tonight · $40 cap" (it did before consolidation D, and again since the orb was
restored on 2026-10-14). The
line (`store/mainAgent.ts` `nightQueueLine`) leads Automations → Tonight, with the rules every item runs
under (plan-completion matrix O8; the Main agent's side band showed it too until COO rebuild L3).

## The weekly update

`digest.rs` (master plan #4). The `weekly-update` window job runs on
`friday-pm` (decision 2: Friday 15:00), run-once-on-wake:

1. **Gather** from records Hollow already keeps: each workspace's Activity
   (`workspace_activity`: conversations, runs with cost, results and their
   reviews, installs) and the tasks. Shipped = accepted results, installs and
   tasks done this week; in progress = draft results and runs per workspace;
   blocked = failed runs and tasks on hold; next week = open tasks due then.
   Every line names its evidence (a vault link or a record id). A line with
   none, or whose date only a file's modified time gives (a task marked done),
   says *not verified*.
2. **Numbers**: conversations, runs, result changes; then from the usage
   ledger's own columns (run ledger v2): model spend over every run of the
   week (folded by run id, the COO's and automations' included, unpriced
   runs counted apart) and spend by the department each run's workspace had
   when it launched ("not available" when none had one). Hours saved come from
   the value ledger (#6): accepted runs only, from the person's baselines,
   labelled an estimate; "not available" until a baseline is set.
3. **Draft** into `workspaces/main/outputs/weekly/<Friday>.md` from the
   person's template `workspaces/main/templates/weekly-update.md` (written with
   the default on first use; `{{title}}`, `{{week}}`, `{{range}}`,
   `{{drafted}}`, `{{shipped}}`, `{{in_progress}}`, `{{blocked}}`,
   `{{numbers}}`, `{{next_week}}`). An existing draft is never overwritten.
   Written under the scheduler's bookkeeping lease; a busy vault retries.
4. **Notify** "Your weekly update is ready" (opens the draft), then queue one
   COO turn (`main_agent::queue_hollow_turn`: today's thread, the
   configured agent, unattended limits, never past the day's cap) that
   rewrites the prose from those facts only. It runs once per week
   (`prose:<week>` in `jobs.json`), and only on a draft this job created
   (`draft:<week>`): a draft the person made earlier with `digest_weekly`, and
   may have edited, is left alone.

Nothing is sent. `digest.digest_weekly {week}`
(write) drafts any week on request. Needs the COO workspace; without
it the job logs and skips.

## The gardener

`gardener.rs` (master plan #19, decision 5). The `gardener` window job runs on
`nightly`, run-once-on-wake (a laptop asleep at 02:00 gets it on waking). It
**only scans** (and not while a run holds the vault): it writes `workspaces/main/outputs/gardener/<date>.json` (when
the COO workspace exists) and keeps one live review card, kind
`gardener` ("N things can be tidied", in Waiting on you → Tidy up), superseding
yesterday's; with nothing to tidy the card goes.

| Line | Offered when | Do it | Undo |
|---|---|---|---|
| `worktree:<ws>` | a build worktree is clean, nothing unpushed, no ignored files beyond rebuildable output (`.env.local` and local databases count), idle ≥ 7 days (git, read-only), and not held by `worktree_hygiene`'s holds (`Holds::read` + `held_for`, folders canonical: a build running, waiting to install or installing still needs it, or the installed app or rollback copy was built in it or at its HEAD; overnight S01e) | `build_workspaces::remove_for` (refuses unsaved work again), after reading the holds again (`worktree_hygiene::held_now`, overnight R2: a worktree a Build or Install has needed since the scan is skipped, "Kept: <reason>, since the list was shown", and the other lines still apply), and once more right before `git worktree remove`, after its services stopped (`remove_for_unless`, overnight R4) | the branch is kept; create the worktree again |
| `idle-workspace:<ws>` | idle ≥ 30 days (brief mtime and its conversations' last activity) and no real open PLAN.md rows (the two seed rows don't count; overnight B2, `workspace_shape::open_plan_rows`); never built-in, tracker-managed or with a live worktree | `workspaces::update_meta(status=archived)` | set it active |
| `duplicate-learning:<path>` | its statement exactly repeats an older learning (`statement_key`) | the newer copy to `.trash/` | restore from the trash |
| `test-task:<path>` | a task titled `[Hollow test]…` | local (a legacy `notion_id` alone no longer protects one, 2026-11-10): to `.trash/`; tracker-linked (`Task.tracker`): **manual**, listed for the person to review in the tracker | restore from the trash / the tracker's recovery |
| `conflict-copy:<path>` | an iCloud/Finder conflict copy (`Note 2.md`, `Note (1).md`, "conflicted copy") beside its original (`conflict_copies::scan`, overnight A10; a pair marked Not a copy is skipped, morning M7) | **resolve**: listed, never ticked or applied; the person keeps one in `ConflictCopiesDialog` (Waiting on you, kind `copies`) | the other copy is in `.trash/` |

Applying: the person's **Do it** on the card (`resolve_review_item`, the ticked
lines; manual lines can't be ticked) or `gardener.apply {ids}`, which
is in `ALWAYS_ASK` (a card in every mode, bypass included; refused from
automations and check-ins). Either way a fresh scan runs first and only ticked
ids it still finds are applied, so a stale or edited card can't widen what
happens. If nothing ticked could be tidied the card stays open and says why.
There is no other apply path (no raw command). Nothing is deleted; the tracker and remote branches are never touched.
`gardener.gardener_scan` (read) shows the lines without changing anything.

## Not built yet

- A night-queue panel in the UI (agents and the commands can use it now), and
  "Do this tonight" on a message, task or workspace (it exists on Plan rows and
  runbooks since overnight A7); one-click accept / open / queue a revision in
  the morning review (the COO presents it in chat).
- The bed row "closed through a Friday window, catch-up once" (bed runs once
  per phase, decision 8).
- Gardener: stale local branches (deleting a ref is a hard delete, so not
  offered), Jev-assisted near-duplicate learnings (exact repeats only), and
  leftover test pages outside `tasks/`.

## Friction recount

`friction.rs` (master plan #13). The `friction` window job runs on `nightly` (catch-up once on wake) and only
recounts the friction patterns into the session store's `friction` table; it never starts a fix draft. `friction.list`
recounts on read too. A draft starts only from a card the person confirms (`friction.draft_fix`, always asks); the
Settings → Usage → Friction page was cut (G2, 11-08). See docs/reference/runs.md "Friction → fix loop".
