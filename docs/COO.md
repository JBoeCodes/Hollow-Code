# COO

The COO is the always-on agent named "Main agent" until COO rebuild L6
(2026-11-10). Only the name people and agents see changed: the `main`
workspace id, `MAIN_AGENT_ID`, the `main-agent` chain root and budget scope,
module, type and settings names, CSS classes and test ids stay, and nothing
on disk is rewritten (an older `workspaces/main/WORKSPACE.md` titled "Main
agent" reads as COO, `workspaces::summary`). Phase names below (M1 to M6) are
the original programme's.

Main's **Actions** link opens Automations and places keyboard focus on the
COO actions disclosure once its lazy view is ready. This reveal is
consumed once; leaving, changing views or closing cancels it. A later load
does not take focus from another control. Local toggles and data refreshes
do not move focus. Empty failed reads show one error plus Retry; retained
rows stay labelled as the last read.

Hollow's always-on agent, reached from the **orb** at the top of the app (one
click; consolidation D folded it into an Ask button on 2026-10-13, and it was
restored on 2026-10-14). The
programme plan (M1–M6, decisions M-Q1–M-Q5) lives in the vault workspace's
`outputs/main-agent/README.md`; this file says what is built.

## Subagent work view

The Subagents tab is a compact side view without a page heading. Active rows
show a stable conversation glyph, a plain-language current step, at most three
distinct recent steps, an explicitly reported plan's count and elapsed time.
Tool names and targets pass through a closed display vocabulary, including old
checkpoints; unknown tools use a generic activity label. Known cost is visible;
model, effort, tokens, unavailable cost and recorded files live in the hover or
transcript header. Failed and stopped runs cannot retain a live current step. Finished
rows show the child's Done, Partial or Blocked status when it reported one;
otherwise they say Finished, which describes the run, not task completion.
Finished rows are one line (title, outcome, duration) under a closed done count.
Recorded file paths remain in the hover; final answers retain reported PR links.
Shell-only outputs are not inferred. A row opens the existing transcript, with
Back, Stop and the message composer.
Queued rows show the pump's specific wait reason once, or Waiting in line
when no limiting reason is recorded; the full reason wraps at narrow widths.

Swarm membership comes from saved swarm records; ordinary child relationships
form Team groups, including handover keys. The current scope includes up to
250 descendants across eight levels, cycle-safe. The walk reads every
descendant's status from its turns first and walks finished subtrees too, so
a running or queued grandchild of any finished part (or of an older chat in
the chain, or of a listed Earlier chat) is Active, counted and stoppable,
and a queued start under any visited conversation is listed; only the newest 200 finished rows
are built, and the done count says how many ended. Mixed groups stay together
in Active; completed groups move to Done. Team and swarm summaries start closed
on one line with their part count. Show older and Earlier chats remain.
Group hover costs sum listed priced rows, labelled recorded, not estimates
for unpriced parts. A swarm's saved task title replaces only its generated
prompt title, never a renamed title. New swarm prompts start with the task title.

`subagents/activity.rs` reads the latest turn's bounded step spine, the last
reported plan event and durable final status. Codex's public plan snapshots
become compact count-only events as they arrive, without adding tool calls.
Claude TodoWrite/update_plan counts are stored before tool payload truncation.
No plan means no progress bar. Finished activity is cached by database, turn,
state and latest event; Earlier chats do not request activity details.
The spine keeps
a bounded display target separately from its truncated JSON checkpoint,
redacted before it is cut. Readers redact tool targets before display. A
re-queued subagent whose new run has no turn row yet shows no current step,
recent steps or plan (the newest turn is the previous run's). Updates share the existing three
second active poll; elapsed ticks pause with a hidden document. The opacity
pulse requires the shared ambient-motion policy, with system and saved Reduce
Motion taking precedence. No network reads or model calls are added.

## Status

The header is one status line and at most two links (2026-11-06): its state
("Waiting on you in 1 conversation", the orb badge's count of its own
conversations, never the bell's every-item count; "Working on Claude" adds the
recorded provider), then its check-ins ("6 check-ins today · next when
something new comes up": a check-in runs only when something new came up or as
the day's first, so a past due time never reads "No upcoming check-in"). Under
it, its day in one quiet line (spend against the hard stop, acts on its own or
suggests only, suggestions taken), a held-back wake's reason, and a failed read's
Retry status. Controls: More and, only when one failed, Open failed conversation.
More holds the links the side band had until COO rebuild L3 (2026-11-10):
"Actions it took on its own · N recent" (the acts log, last 200), Change hard
stop, the check-in setting (Turn check-ins on in Settings while they are off),
How check-ins run, and Edit standing orders (Create default standing orders
while the file is missing). The side band, its Show details toggle and the
"Chats I started" list are gone: the subagents it started are its conversation's
Subagents chip and tab (the Preview pane's Subagents tab, `SubagentsPanel.tsx`).
Change icon… is in the top bar's Actions for COO menu and Settings →
General; the top bar shows no second "Waiting on you" pill here.
Review, through the bell, owns waiting items. Automations → COO
actions is the single action-detail home, with timestamps, full summaries,
retained-read wording and Retry. No animation is introduced; the status is
also re-read at local midnight and, while a check-in is past due, at the next hour.

| Phase | State |
|---|---|
| M1 Presence | Built (2026-10-07): built-in `main` workspace, top-bar orb, system-wide key, tray item, standing orders + notepad, daily thread, own prompt |
| M2 Mission control | Built (2026-10-07): "Chats I started" list with Open/Stop (replaced 2026-11-10 by the conversation's Subagents chip and tab, COO rebuild L3), live count on the orb, a wake turn when a started chat ends while the COO is idle; the prompt says scheduled check-ins aren't on yet. Since repo ideas 2026-10-27 #7 each ended chat ends with its own status line (Done, Partial: …, Blocked: …; `child_status.rs`), shown as a chip until L3 and in its session's answer since |
| M3 Check-ins | Built (2026-10-07): a token-free gate every minute; a check-in turn in today's thread only when something is new or at the day's first; $100/day hard stop (edited in Settings → Usage → Budgets since P1-07); Settings → Background work → COO |
| M4 Acting on its own | Built: unattended turns use only the safe actions (chats ≤ acceptEdits, tasks, its own folder), no cards, every act logged |
| M5 Swarms | Built: `sessions.swarm`, parts on Claude Haiku 5.5 by default, up to 6 parts by default (Settings, up to 100), 6 at once within the app-wide chat limit (Settings → Background work → Swarm parts at once; 2 in build workspaces), one synthesis wake. A chat's own `start_session` subagents are capped separately by Settings → Chat → Subagents per chat (default 6) |
| M6 Learning | Built: suggestion outcomes recorded, 30-day stats, declined-for-good list in every check-in, "never" becomes a learning proposal you approve |

## The `main` workspace

`workspaces::MAIN_AGENT_ID` (`main`), created on first use by
`ensure_main_agent` (`ensure_main_agent_workspace` command and app action).
`kind: main` comes from the id, never from editable frontmatter. Like Chats,
it can't be renamed, archived or deleted, and the frontend keeps it out
of project lists (`isBuiltinWorkspace`). It runs **scoped** to its folder
(`workspaces/main`), so it never holds the vault lane or blocks automations.

| File | Owner | What it is |
|---|---|---|
| `WORKSPACE.md` | agent | Its notepad. "Current context" carries what it is watching across days. |
| `PLAN.md` | agent | What it is tracking for you: subagents it started, follow-ups. |
| `STANDING_ORDERS.md` | you | What it watches and may do unprompted. Seeded once with the brief from `STANDING_ORDERS_SEED`; after deletion it stays gone unless you click Create default standing orders. An instruction file to the run guard (`run_guard::MAIN_STANDING_ORDERS`): Hollow app tools refuse to write it, a COO turn denies Edit, Write and MultiEdit on it (`STANDING_ORDERS_DENY`), and an agent's change is flagged `agentInstructions` in History. The one agent route is `workspaces.edit_standing_orders` (G1, 2026-11-15): add, replace or remove one line (exact `before` text, or `index`; 400 characters a line, 120 lines, 32 KB), ALWAYS_ASK so its card shows the line before and after in every mode, atomic write under the vault lock, noted to the run guard as a host write. |
| `outputs/` | agent | Digests and summaries. |

Its context envelope carries the usual documents plus `STANDING_ORDERS.md`
(`MAIN_CONTEXT_DOCS`, same 48 KB fair share; a long brief keeps its Decisions
and newest Current context entries, HB02). Its appended prompt is
`chat::main_agent_prompt`, not the outcome-workspace paragraph: read across
the vault, do each workspace's work *in* that workspace through `hollow_app`
`start_session` / `send_to_session`, then end the turn rather than wait in
it (Hollow wakes it once when a started chat ends, unless the person stopped
it; a chat that hands over wakes it when its last successor ends; a child's
child's ending doesn't wake it but arrives in its next turn's note; `wait_for_session` only for a short wait
on one chat or several (`keys`, overnight OB10a; an unfinished one on subagents it started says to end the turn
instead of waiting again),
`read_session` to look in (`list_sessions` answers 12 short rows, `mine` only
the subagents it started and their successors, `detail` the old full rows; a
`read_session` of an ended child defaults to its last answer, OB21), and `sessions.wake_me` for a later look at a long
chat, except from an unattended turn, which can't; overnight E7, the prompt
held to `MAIN_AGENT_PROMPT_BUDGET`), keep replies short, don't list started chats in PLAN.md (T3a, 2026-11-15: `list_sessions mine` has them; old rows are still read), never
accept Results or approve learnings, confirm external effects.

## Entry points

Explicit task batches now use `sessions.start_workers` from either global
Main or a project's COO. “Do five Portal tasks” creates five distinct
local worker chats, while project-location and status questions stay direct.
The batch inherits the caller's provider, model and permissions; it does not
choose the legacy swarm's default Claude model. Workers remain subject to the
existing queue and resource limits. The legacy opt-in swarm action still exists.
Project-main results arrive in its next-turn child note; project leads
do not acquire the global Main's autonomous check-in schedule.
Batch ownership checks use the project's current folder and the same main-key
validation as normal sends. History in its own removed worktree remains valid;
a worktree path claimed by another project is refused without moving history.

The Main page header no longer repeats running child chats or the bell's
Review count (a second row did until 2026-11-06); the status line names live
subagents ("2 subagents running") and its Subagents chip and tab list
them (COO rebuild L3). Unread and failed counts never render as zero; retained
counts say last read. The header's day line, Retry status and More keep the
recovery and detail controls the day band had.
Working orb tint follows a complete single-provider observation from listed
live Main sessions and children, with words beside it. Mixed, unknown and
not-yet-listed turns stay neutral; next-message choices never supply identity.
The shared indicator owns one experimental waiting aura across top bar and
page. Idle is still; existing working arcs retain their motion policy.
The combined experimental gate, forced colors and reduced motion win.

- **Orb** (`MainAgentOrb.tsx`, in `TopBar.tsx` after New, the last of the
  global group since P1-14 removed the Review bell). Consolidation D (2026-10-13) replaced it with an
  Ask button whose click opened a panel; restored 2026-10-14 on Alex's ask
  ("the agent button that takes you to COO"). It carries the COO's session
  indicator (`MainAgentIndicator`, asks 10-11 A2): the doorway alone (resting), in
  a still ring (watching), in the turning arc (working, a COO turn or a
  subagent it started is queued or running), with an "!" disc (one of its own
  conversations waits on you) or in a dotted ring (paused: its own spend hit the
  daily cap). The page's header uses the same mark and words
  (`mainAgentNow` in `store/mainAgent.ts`, `presence` in `mainAgentSpace.ts`), then its
  check-ins (`checkInWords`). Clicking it runs `toggleMainAgent`: from anywhere it opens the
  COO; from the COO it reopens the workspace conversation and page you came from, or Work when
  it was opened some other way. Leaving it by any other route ends the trip.
- **⌘⇧M** and **View → COO** open the page (`openMainAgent`; already open, they focus its box). The
  command keeps the id `main-drawer`, so a custom binding in `shortcuts.json` still works. The palette's one row
  for the page is **Open COO** (`act:main-agent`). **Removed in run 15 P3 (2026-10-27):** the COO
  drawer, this conversation docked beside any page with its own chat engine, together with the page's **Open
  beside your page**, ⌥-click on the orb and the palette's Toggle COO drawer, so that one conversation is
  on screen at a time. Its stored `hollow.mainDrawer.open`, `.width` and `.mode.<key>` keys are no longer read.
  Until 2026-10-21 a quiet **Ask ⌄** beside the orb opened an Ask panel; it was removed because each of its
  places had its own button (the workspace's message box, the orb, New → New chat).
- **Across all my work** and New chat stay on Work's Ask field (`askScopes.ts`; One Ask, 2026-11-03, removed
  its my-notes and Plan it in Hollow scopes, so a request those planned or answered comes here). Across
  all my work opens this page (`runAsk` in `store/ask.ts`): Enter sends the person's text once the page's
  conversation is open and idle (D2, P1-11; as typed words, through the composer's pending send, so the box's
  own draft stays), and ⌥-Enter only places it in the box. Asked from the page itself, it fills the box.
- **Its day** (the header line under its status) holds the COO's own status; the bell holds the
  global **Review** count, which opens Review; a failed own
  conversation has an Open failed conversation button. The line shows autonomous
  mode and suggestions taken. More → Actions it took on its own opens the
  Automations disclosure with dates and action labels. Status failures offer Retry and mark retained facts;
  focus, a settings save, a future check-in, local midnight and (while one is past
  due) the next hour trigger fresh reads.
  `api.setMainAgentSettings` emits `hollow-main-agent-settings` after a successful
  save; all settings callers share that refresh. One timer per status, at most one
  read an hour, so stale responses cannot start a polling loop.
  Child/action reads retain last-good rows and distinguish loading/errors from
  a confirmed empty list.
- **Standing orders** reads distinguish missing, unreadable (Retry), empty and
  no Watch bullets. **Create default standing orders** is a person-only click:
  `create_main_standing_orders` uses `workspaces::STANDING_ORDERS_SEED` through
  exclusive `vault::create_note`, returning false for an existing file. There is
  no app action. It requires the Main brief, records successful host writes and
  refreshes workspace metadata after creation; opening the page never recreates
  a deleted file.
- **System-wide key** `main-agent`, default `⌘⌥⇧H` (`shortcuts.rs` GLOBALS,
  `shortcuts.ts` COMMANDS; remappable in Settings → Keyboard shortcuts). Three
  modifiers because a global key takes the chord from every app.
- **Tray** "COO". Key and tray call `lib.rs` `show_main_with(app,
  "main-agent")`, which brings the window forward and emits the `main-agent`
  app-menu action (Import a source's key and tray item share it since S5); that
  action only opens (already open: focuses the composer), never toggles away.
- Agents see it in `list_workspaces` as `kind: main` (filter `kind: "main"`;
  it is not a `vault` workspace).

## One conversation (2026-11-07)

From the person's side the COO has one conversation, whatever
happens underneath (Alex, 11-07: "Why are there two COO chats?").
Its **current** conversation is its newest unarchived one
(`chat_chain::main_current`, Rust `src-tauri/src/chat_chain.rs`); every other
COO conversation, and any chat with a handover successor, has
**continued** in its chain's current one (`chat_chain::current_of`).

- **No message lands in a chat that has moved on.** A chat that continued
  shows "This conversation continued in …" with Open in place of its
  composer (`components/ChatChainBar.tsx`, lazy). A send that still reaches
  it (a race at handover, the phone, `send_to_session`, a held send) goes
  to the current one: `chat_queue::accept_turn_with_effort` routes every
  sender (`chat_chain::route_send`), and `commands::chat_send` routes the
  person's first so it is held behind a running turn rather than refused.
  It runs with the current chat's own agent, model and native session, and
  its mode, never stronger than the send's (`chat_queue::apply_route`); a
  Stop and send there becomes Queue for after. The agent sees a
  `<hollow-forwarded>` block worded for who sent it (the person, an agent,
  Hollow); both chats show the note (`<support>/Forwards/<vault>.json`,
  `conversation_chain`; none for Hollow's own turns). A successor or new
  thread whose first turn still waits in the queue counts, from its queued
  row. A new COO conversation started at depth 0 while the current
  one works joins it, unless it continues others (Continue from summary); a
  handover's successor (depth 1 or more) starts its own. Admission checks
  again under the chat-row gate, so a send that loses a race with a
  handover is refused as busy and its retry goes on. Steering a chat that
  continued answers `STEER_ENDING`, so the words go as the next message,
  and its banner keeps Stop while its last turn ends. `start_session` and
  moves refuse the COO's workspace (`app_sessions::MAIN_START_REFUSED`,
  `MAIN_MOVE_REFUSED`); `send_to_session` to an earlier chat goes to the
  current one (`forwardedFrom` in its answer).
- **Wakes and check-ins** go to the current conversation and wait behind
  its running turn whatever day it started (`today_thread`), so a wake never
  opens a second thread beside one running past midnight (11-04 12:04 AM).
  The COO's own handover is no "chat it started": its successor's end
  no longer wakes it (the once-per-handover self-wake is gone).
- **Cards follow the chain.** The current chat shows the open approval and
  question cards of the chats before it (`list_pending_interactions` with
  `chain`, `chainCardKeysFor`); an approval answers the run that asked by
  its id, a question card's answer goes as the current chat's next message.
- **One Main in the lists.** The COO button opens the current
  conversation (below); Library → Conversations and the phone's
  `/v1/sessions` list only the current one (`oneMainConversation`,
  `chat_chain::list_filter`). The earlier ones are under "Earlier in this
  conversation" at the top of the current chat, folded, and open read-only.
  Typing in Library → Conversations' search lists them all again, and search
  results anywhere still find text in any of them.

## The daily thread

`todaysMainSession` (`src/mainAgent.ts`): opening the COO picks the
current conversation, its newest, when it started today or still works;
one from an earlier day and idle is the daily soft reset: it starts today's,
which continues from it. Never an older conversation than the newest. A thread started today
counts before the session list refreshes (`lastSeen` in `store/mainAgent.ts`), but only a key in the COO's
folder that it can open (`reopenableMainSessions`: listed, or held in the background there). Each day is a fresh native
session; the notepad and plan carry what matters.

**Across days (OB19, 2026-10-17).** When Hollow's own turn (a wake, a
check-in, the weekly update, the morning review, a swarm synthesis) finds no
thread today and starts one, it continues the newest earlier COO
thread: `queue_wake_as` sends `continued_from` = that key
(`main_agent::continued_for`: the newest unarchived conversation in the Main
folder, only when started before today's local day). The new thread's first
turn merges that thread's task state (items tagged with its key) and opens the
`<task-state>` block with a line naming the key and `read_session`
(`context_compiler::continues_note`), with **no** conversation bundle: a
host-tagged turn is compiled like a handover's successor
(`compile_turn_within`'s `host`). Before, overnight wakes that crossed
midnight started a thread with no task state and no link (every new Main
agent manifest of 10-16/17: `taskState.merged` 0). The person's own new
thread, opened from the composer, is unchanged.

**Who asked a turn (OB20).** Hollow's own turns carry their tag in the queued
row (`ChatSendRequest::host_asker`) and `execute_turn` records it as
`turns.asked_by`: `hollow:wake`, `hollow:checkin`, `hollow:weekly`,
`hollow:wake_me`, `hollow:morning`, `hollow:swarm` (`main_agent::ASKED_BY_*`).
`agent` stays another agent's turn and NULL the person's. Readers test the
`hollow:` prefix (`session_store::person_asks`); the task-state fold credits
only `agent`, so a host turn's items carry no `by` (as before, except a
wake-up in a chat an agent started, recorded `agent` until now).

On the page, wakes use a one-line Hollow disclosure and quiet check-ins say
"Checked in · nothing needs you" (`mainTurnKind` in `src/mainAgent.ts`).

## Mission control (M2)

`main_agent.rs`.

- **The subagents it started** (`children`, command and read action
  `main_agent_children`): durable child tasks queued by any conversation whose
  cwd is the COO's folder in the last 7 days
  (`session_store::delegations_for_cwd`), each with its run-queue state,
  workspace, cost (`descendants::rollup`) and whether the COO was told
  (`child_deliveries`). The store reads them on every run-queue change for its
  presence (working while one runs) and the orb's count; the person sees them
  in the conversation's Subagents chip and tab (`session_subagents`, COO
  rebuild L3), where Stop is `chat_cancel` on the subagent's conversation.
  **Scope (2026-11-11):** Active spans the whole chain (a running one from
  40 days ago still shows); Done and the chip's count are the current
  conversation's own plus handover predecessors last active within 24 hours
  (a mid-task handover doesn't empty the tab); the chain's other finished
  subagents sit behind a visible **Show older (N)** button at the end of Done,
  25 per click, newest first, until none are left (`session_subagents(parent,
  older)`, `subagents_paged`; `done_older`, `older_total`). Earlier chats are
  unchanged: they're other chats, never subagents.
- **Waking it** (`after_chat_finished`, called from `run_pump::emit_row` for
  every finished chat row; `decide`): when a started chat ran and ended
  (`done`, `failed`, `interrupted`; not one you stopped) in the last 6 h,
  unreported, Hollow queues one turn in **today's** COO thread (a new
  one when today has none) with `WAKE_MESSAGE` ("⟲ Hollow: a chat you started
  has finished… Its answer, files and pull requests are below; call
  read_session only when you need more…") plus one line per ended chat, and
  records those chats as reported (`deliver_child` via `wake`). Each line
  (title, place, agent, key, status, end time) carries the child summary
  `descendants::child_summary` (OB17) reads from the turn that ended the
  chain and its receipt: files changed (count, up to 5 paths), PRs (number
  and action), failure count, `verificationState`, then the end of its
  answer, redacted (`claude_runner::redact`), cut to a line start, indented,
  `<` escaped. The answers share `descendants::answer_budget`:
  `clamp(3600 / children, 300, 1200)` characters each, so one child gets
  1,200 and eight 450. Lines run newest ended first (overnight 9 Spare 4;
  no readable end time last), and only the first 8 (`MAX_NOTES`) get a
  summary, so the summaries go to the chats that ended most recently;
  later ones keep the bare line under one "No summary … read_session" hint,
  so a wake stays bounded. The title is flattened to one line, and receipt
  values (paths, PR actions, the state) are kept to one line (control
  characters and U+2028/U+2029 as spaces), clipped, at most 5 PRs. The
  same summary follows each
  `<hollow-child-finished>` line. Not while that thread has a turn
  queued or running, nor in the 60 s after its turn ends (a re-check follows).
  Each child turn wakes once, recorded only once queued
  (`{support}/MainAgent/{vault}.json`), at most 24 wakes a local day, at the
  thread's own last mode (`main_agent::wake_mode`; capped at `acceptEdits`
  until 2026-11-12). A chat that handed over
  (`sessions.handover`) wakes it when the last successor of its chain ends
  (`chain_end`; reported to it as the chain root, `root_deliveries`; a
  successor someone stopped ends the chain and is not news), and
  its Subagents row shows the chain's state and opens the last successor (overnight 10-11 A1; L3). The day's cap holds back only wakes for chats
  Hollow's own turns started; a chat the person asked for is reported past it.
  A held-back wake (`Decision::Skipped`: the cap, or 24 wakes) is kept as
  `WakeState.skipped` and shown under the page header's status with its reason
  (`main_agent_status.wakeSkipped`). The minute timer re-runs the wake check,
  so a wake missed by a restart, a refused send or a busy thread comes later. The open conversation adopts the
  queued turn like any other (`planQueuedChatRows`); the webview shows the
  message as a note from Hollow (`isWakeMessage`).
- **Schedule honesty**: `SCHEDULE_NOTE` in the prompt says exactly what runs
  it (messages, wakes, swarm ends, check-ins per Settings), so the standing
  orders' wording never reads as a promise of its own.

## Check-ins (M3)

`MainAgentSettings` (`{support}/MainAgent/settings.json`; Settings →
Background work → COO; the setter `set_main_agent_settings` is user-only,
never an agent action). Defaults are the 2026-10-07 decisions: every 30
minutes, 8:00 to 18:00 local, working days, $100/day hard stop, act on its own,
swarms of up to 6 parts on Claude Haiku 5.5, six at once.

`start_timer` (lib.rs, queue owner only, candidate builds only with
`HOLLOW_CANDIDATE_BACKGROUND=1`) runs `decide_pulse` every minute. It is
free: it reads `signals` (things waiting on you except the COO's own,
the current nudges, runs that failed since the last check-in) and compares
them with the ones already reported. A check-in turn is queued in today's
thread only when something is new, or for the day's first check-in (a brief
of the day); never while that thread is busy or within 60 s of its last
answer, and never past the cap (`budgets::main_agent_spent`, `main_agent_split`'s
first half: what it ran **on its own** today, i.e. its wake and check-in turns
(message starts with `WAKE_PREFIX`) with their consultations and helpers, the
chats those turns started, swarm parts and pressure tests). The person's own
turns and the chats they asked for are the second half, `askedTodayUsd`: shown
apart in the header's day line and never capped (overnight 10-11 A1: on 10-10 two lanes
the person asked for spent past $100 and every later wake was dropped silently). A check-in that
finds nothing to say answers `PULSE_OK`, which the webview shows as
"Checked in · nothing needs you". Suggestions come as question cards, one
single-choice question per suggestion (Do it / Not now / Never), docked over
the message box (2026-10-14; until then one ask-canvas, whose answers had to
be copied by hand when no window had watched the check-in).
`main_agent_status` gives the panel its line (spend, next check-in,
suggestions taken). The next check-in is exact (`next_check_in_at`): the interval after the last one, or the top
of the first allowed hour; the old 15-minute step read up to 14 minutes late (ENG-97).


**Meetings in the next hour** (overnight 10-11 A4): when a check-in may run (on, in hours, interval passed),
`check_pulse` first reads the next hour's meetings (`meeting_prep::for_this_mac(vault, 1)`, at most every 10 minutes:
a check-in that finds nothing new leaves the window open every minute) and `meeting_signals` adds a `meeting-prep:<event id>` signal for each without a prep
pack. The line asks it to run `agenda.prepare_meeting` and say where the pack is. That action is on the unattended
safe list: it writes one new note in a workspace's `outputs/prep/` and sends nothing; `regenerate` is refused
unattended. See
[PREP_PACKS.md](PREP_PACKS.md).
**How check-ins run** (plan completion L2): the page header's More links to Automations → "How background work
runs", which explains check-ins beside scheduled automations and the silent heartbeat (when, what each sees,
what it may do on its own, what it leaves; `src/backgroundWork.ts`, [AUTOMATION_RECOVERY.md](AUTOMATION_RECOVERY.md)).

## Acting on its own (M4)

`main_agent::unattended`. Wake and check-in turns are registered by queue row;
`app_broker::execute` recognises their runs (`run_queue::id_for_run`) and
allows only reads plus `sessions.start` (≤ acceptEdits, named or not) / `send` / `stop`,
task create/update/status/priority, notes under `workspaces/main/` (never
`STANDING_ORDERS.md`), `sessions.suggestion_outcome` and
`learning.submit_learning_proposal`. Everything else is refused with a pointer
to suggest it; allowed actions show no card. A chat such a turn starts
inherits the limits; it may not target the vault root or the COO, nor
`send` to a COO conversation or a chat in a build workspace (any other chat
may be messaged, in that chat's own mode). Every COO turn's own file tools
are denied `STANDING_ORDERS.md` (`STANDING_ORDERS_DENY`). Detection also fails
closed on any COO turn whose message starts with "⟲ Hollow:". With "Act on its own" off, only
reads. Each act is logged (`main_agent_activity`, the panel's "Done on its
own"). The turn itself runs in its thread's own mode (`main_agent::wake_mode`;
capped at acceptEdits until 2026-11-12), so its file tools and shell follow
that mode; its limits are the cards and this allow list.

**Installing Hollow always asks** (P0-27, decision D12, 2026-10-22):
`build.install_built_app` is on `app_actions::ALWAYS_ASK`, so even an
attended Bypass chat (the COO's included) shows its install card and
installs nothing until Alex allows it; unattended turns keep refusing it.
Allowed, the build waits in the top bar's Update button as a Local build and
installs once the turn has ended and nothing runs (row U10).
Confirm cards time out in about two minutes, so the COO should call it
only when Alex has just said "Install" (guidance; no code enforces it).

## Other parents woken when their children end (overnight OB10b)

Only the COO is woken by `decide`; a project lead gets its batch report
without asking ([Project leads](#project-leads-l8)). Any other chat that
started child tasks asks for it: `run_action sessions.wake_me {onSessionsEnd: {keys,
mode}}` (`wake_me::schedule_on_end`), then ends its turn instead of looping
on `wait_for_session` (122 of 133 waits came back "running"; one chat made 98
wait-only calls over 11.6 h). `mode` is `all` (default: every key ended) or
`any` (the first); each key must be a conversation the caller started
(`descendants::my_turns`, the `list_sessions` `mine` set, successors
included), and a lane that hands over counts as ended when its last
successor ends (`descendants::chain_end`). `after_chat_finished` marks it due
once (`wake_me::arm_ended`) and sends it at once (`run_due`); the minute timer
runs `arm_ended` too, for a child that ends without a `finished` event
(cancelled while queued, interrupted at startup), and sends a fired wake once
the parent is idle. A fired wake whose parent stays busy for 30 minutes
(`LATE_MINUTES`) is dropped as missed; the children still reach the parent
through its next turn's note. The message opens with the reason, then
one line per ended child with its OB17 summary (`descendants::ended_line`),
and names the ones still running; the children it carries are recorded as
reported (`deliver_child` / `deliver_to_root` via `wake`) just before it is
sent, so the woken turn's own note and later ones don't repeat them. A send
that fails, or never settles (a crash, found by `recover`), forgets them again
(`session_store::undeliver`, only what that send recorded): the retry records
them anew, and a wake that then never runs (cancelled, missed) leaves them to
the parent's next note. It waits at most 24 h; it is one of the
conversation's wakes (one pending, 24 a day), and a stopped asking turn or
the person's next message cancels it. **Never in a COO thread**
(refused: it is woken by its own mechanism, so it would be woken twice; its
waits' "still running" advice keeps OB10a's wording).

**The orchestrator limit.** A woken parent runs exactly like any `wake_me`
turn: unattended, in the conversation's current mode and never above the
asking turn's (a bypass parent stays bypass since 2026-11-12, a plan one stays plan),
with only the safe actions of `main_agent::unattended::SAFE` (no teammates,
computer control, services or connectors), and it can't ask for another wake.
The child answers its message carries (OB17 summaries, written by the
children's agents) reach that unattended turn at the same trust level as the
COO's own wake message. So a parent that must merge, push or act past
those limits writes down where things stand and waits for the person.
Widening that for orchestrators is **S19 (Needs Owner)**; nothing here
widens it.

## Swarms (M5)

**On Claude Haiku 5.5 (2026-10-07).** Parts run on `swarm_model`
(default `claude-haiku-5-5`: $0.10 / $0.50 per million tokens for prompts up
to 100K, $0.50 / $2.50 above; about 40× below Opus 5.5) unless the swarm or a
part names another model; a `claude-` model forces the Claude agent. Default
size 6 (Settings, up to 100; 25 before 2026-11-15, saved values keep theirs). At most `swarm_concurrency` parts (default 6,
up to 10) run at once: the run pump admits a part while fewer than that many
parts run (`swarm_part_limit`, registry `swarm_parts`). Since 2026-10-20 a
part also counts toward Settings → Chat → Chats running at once, like
every other agent turn (before, parts ran in slots of their own beside the
person's three), so a big swarm can fill the default 10 and the person's chats
wait behind it (a swarm tops out at 10 parts, so a higher limit leaves room). Build-workspace parts keep the usual rule and the two toolchain
slots. Claude Code 2.1.286 runs Haiku
5.5 but can't price it (`costBasis: "unknown"`, its estimate about 40× too
high), so `usage::UNPRICED` prices such entries from their tokens at one
rate per model (Haiku 5.5's up-to-100K rate: one rate keeps resumed turns'
differences right; turns with longer prompts are under-counted until Claude
Code prices it); caps and roll-ups use that. A part on a non-Claude agent
keeps its own default model.


`sessions.swarm` (card in every mode but bypass; asks in auto), only from the
COO's own conversation at depth 0 and only when the person asks or
agrees. Parts (≤ `swarm_max`) are durable child tasks recorded against the
starting run, so they appear in its Subagents tab and
roll into its cost. `advance` (after every chat end) launches at most 3 at
once, at most 2 in build workspaces (the machine's two toolchain slots),
skips parts once the day's cap or the swarm's budget is spent, and, when every
part has ended, queues one wake asking for `outputs/swarms/<id>.md`. Parts
never wake the COO one by one. Known limit: the swarm's budget counts
only finished parts, so up to three live parts can overshoot it. State: `{support}/MainAgent/{vault}-swarms.json`.

## Learning from suggestions (M6)

When the person answers a suggestion the agent records it
(`sessions.suggestion_outcome`: done / not_now / never;
`{support}/MainAgent/{vault}-suggestions.json`). The last 30 days give the
"taken" share in the status line; the newest ten declined-for-good
suggestions are listed in every check-in message so they aren't offered
again; a "never" also becomes a learning proposal, which only the person
accepts.

## Role: delegate by default

Its system prompt (`chat.rs` `main_agent_prompt`, within
`MAIN_AGENT_PROMPT_BUDGET`) makes it the person's first stop for prompts,
conversations and questions. It answers directly what a few reads answer
(status, where things are, what a chat found, Hollow settings) and runs any
Hollow action itself. Building, research, planning or writing a deliverable
goes to the project's lead (`send_to_session` to `read_workspace`'s
`projectAgentKey`, its current conversation since L8) or to a subagent (`start_session`; with no project, the
Chats folder `quick-chats`); then it ends its turn and reports when the work
returns. That is a default, not a limit (Alex, 11-10: "I want the COO to
be able to do anything I can do in Hollow"): its turns add no tool rule
beyond the standing-orders one below.

Delegation splits by real dependencies (Alex, 11-10: "All of hollow
should think like this"): every part that needs nothing from another starts
at once, only a part that needs another's output waits for it, and past the
chat cap, build slots or budgets the rest queue. The rule is one text,
`delegation::DEPENDENCY_RULE`, in the "Delegating" block every chat that can
start sessions gets (`delegation::guidance` with child tasks), the COO's and
every lead's turns included; their roles only point at it in a few words.

Overnight work follows the same shape: a project's night is started by its
lead, a multi-project night by the COO. The orchestrator is a subagent of
that parent and the lanes are the orchestrator's subagents, so the Subagents
tab is the board; boards and lane ledgers stay Markdown in `outputs/`. The
prompts say so in one line each (the COO's role, `project_agents::context`).

## Project leads (L8)

Each project has one conversation you talk to: its **lead**, the fixed
`s-project-<hash>` conversation (`project_agents::key`). Since COO rebuild L8
(2026-11-10):

- **One row.** The sidebar's project row opens the lead (`openProjectAgent`);
  a project lists no conversations, folds nothing and has no ⋯ or +. Its
  right-click (Control-click, VoiceOver Show Menu) is the project's menu. The
  collapsed sidebar's pinned icons open the lead too, and ⌘⇧] / ⌘⇧[ step
  between leads.
- **Lead chain.** The lead's handovers (long-context rollovers included)
  continue behind the same row, as the COO's thread does:
  `chat_chain::lead_root` treats a successor as the lead (prompt, tool
  policy, depth 0, worker batches), `lead_current` is the last successor,
  and `read_workspace`'s and `workspace_sessions`' `projectAgentKey` name it.
  A send to the fixed or an earlier lead conversation goes to the current
  one (`route_send`), and its "Earlier in this conversation" lists the ones
  before it.
- **Moved projects keep their lead (2026-11-14).** A lead's key is a hash of
  the id its project had when the lead started; Move Hollow home rewrites
  workspace ids (`workspace_aliases`), not session keys. So a project's lead
  is any of `project_agents::keys_for` (its current id's key and each older
  id's), the one active last (`lead_key`, `LeadKeys` for the rail in one
  read); `validate`, `lead_project`, worker batches and Earlier chats find
  the project through `project_of`. Before this, Main's send to a moved
  project's lead was refused as "another project" and its page showed no lead.
- **Project work from the COO is the lead's.** The COO's role sends a
  project's building, research, planning and deliverables to its lead
  (`send_to_session` to `projectAgentKey`), which runs and reports it; a
  subagent of the COO's own is for work with no project (`quick-chats`).
- **Earlier chats.** The project's other chats are under the lead's
  Subagents chip → Earlier chats (`subagents::earlier_folder`, up to 100,
  newest first; Library → Conversations has every one), readable and
  messageable; archived chats stay archived. The chip shows for earlier
  chats alone (`chipShown`).
- **Every chat in the project (2026-11-14).** The tab also lists subagents
  that other conversations started in the project folder (the COO's, a
  worker's, an earlier chat's finished ones, one moved in): running or queued
  under Active, finished under Earlier chats. Each row says who started it
  (`SubagentRow.startedBy`: `lead`, `main`, `chat` or `you`, shown as "by the
  lead", "by the COO", "by another chat", "by you" after the title and in
  its tooltip; `subagents::starter_kind`).
- **Role** (`project_agents::context`, within `LEAD_PROMPT_BUDGET`): available
  for prompts, conversations and questions about its project; building,
  research, planning and deliverables go to subagents (`start_session` in
  its project), split by real dependencies with independent parts started at
  once (`delegation::DEPENDENCY_RULE`, above), then it ends its turn. It is the one writer of WORKSPACE.md
  Decisions and AGENTS.md (and PLAN.md, SOURCES.md): when a subagent ends it
  records decisions and outcomes there in one "Updated my notes" line, and it
  searches its subagents' chats instead of keeping task history.
  Its overnight orchestrator is its subagent and the lanes are that
  orchestrator's subagents.
- **Tool policy** (`project_agents::lead_denied_tools`, added in
  `chat.rs` beside `coo_denied_tools`). On an agent that holds deny rules
  (Claude) a lead turn denies `Bash`, `Write` (no new or rewritten files) and
  `NotebookEdit`, and `Edit` of: the turn folder when it isn't the project
  folder (a build worktree), the project folder's subfolders and its other
  top-level files, every other workspace folder, and the vault's other
  top-level entries, each as given and resolved (`/var` and `/private/var`).
  Claude's rules can't say "edits except these four files" (a deny beats any
  allow), so `Edit` of files outside the vault and the turn folder, and
  files created after the turn started, stay open. A path the rule syntax
  can't hold (a space, comma, parenthesis or glob character anywhere it must
  name, say a vault entry "My Notes"), or an unreadable project brief, fails
  closed and logs it: `Edit` is denied too, so that Claude lead can't edit
  its notes itself and delegates them (acceptable: it delegates). Codex, Antigravity and Grok hold none of these rules,
  so they get none and their lead turns are admitted; the policy is in the
  prompt. The COO's turns are unchanged.
- **Reports on its own.** Each subagent a lead starts joins its batch report
  (`wake_me::join_batch`): one `onSessionsEnd` wake with `batch: true` and
  `onFailure: true`, keyed by the lead's fixed conversation so one batch
  spans its handovers, armed once every subagent in it has ended (or one
  failed) and the lead is idle, sent once as an unattended report turn to
  the chain's current conversation. A start while one waits joins it; the
  person's message, the lead's own `wake_me` and a handover don't cancel it.
  A crash after the report is queued is recovered from any conversation of
  the chain, and a subagent whose registration was lost (a crash just after
  its start was queued) is adopted from the live queue on the next arm. A
  report turn that delegates again opens a new batch: batch reports have
  their own cap (`BATCH_MAX_PER_DAY`, 96 a day per lead), apart from its 24
  own wakes; past it, or past 30 subagents in one batch, the report is
  dropped with a log line. A hard-stop budget that would hold the turn
  (`budgets::preflight_readonly`) drops it as missed, logged; the children
  still reach the lead's next note. The
  COO keeps its own wake (`decide`), so it has no batch.
- **No data moves.** Nothing on disk is moved, renamed or deleted; a fixture
  vault hashes identical before and after (`chat_chain` tests).

## Chats with no project (L8)

A new prompt with no project goes to the COO (Q1): sidebar New chat, ⌘N with
no project open, Home's Ask "New chat" scope and files dropped on Home open
the COO (`store/workspaces.ts` `openQuickChat`), and with text send it as its
next message; Ask's Add to project sends to that project's lead. The
sidebar's Chats section is gone. Old quick chats stay in `quick-chats`,
listed under the COO's Subagents chip → Earlier chats, and COO subagents with
no project still run there, hidden from the sidebar.

## Provider and model

Any installed agent may run the COO (L6, 11-10; the Claude-only rule P0-20
and its notices are gone). The model menu, `/agent` and `/model` offer every
provider as in any chat, a fresh conversation keeps the composer's agent, and
the phone's routes take the asked agent. The default stays the app default.
The one tool rule its turns add is `main_agent::STANDING_ORDERS_DENY` (its own
file tools never edit `STANDING_ORDERS.md`), applied by `chat::coo_denied_tools`
only on an agent that holds deny rules (`agent::holds_deny`, Claude today);
on the others the same rule is in the prompt and the standing orders, so the
generic tool-restriction validation never refuses a COO turn.
The COO menu caps its height to the window, with a scrolling model list,
so its search remains reachable at Largest text.

## The runner COO (M3b, 2026-11-15)

With a runner, its headless Hollow (`hollow serve`) can run a COO beside the
project leads: the `main` workspace's COO in the runner's own vault, turned on with
`hollow serve runner-coo on` (`runner_coo.rs`,
[HEADLESS_SERVE](HEADLESS_SERVE.md#the-runner-coo)). It is woken when a chat it
started ends and at its own `wake_me` follow-ups, never checks in (its prompt's
schedule line says so), and is reached at its chat key. A serve without the role
still wakes a COO already working there but never checks in either. Each Hollow
fires only its own wakes; a follow-up records the Mac it was asked on. The app's COO
becomes the dispatcher once a paired Mac advertises it (below).

## The dispatcher COO (M3a, 2026-11-15)

With a runner COO paired, this Mac's COO is the **dispatcher** (`dispatcher_coo.rs`):
it answers quick questions itself and routes long work at once to the runner, so its
own conversation is never busy with it. It is on while a paired Mac's record says it
runs a runner COO (`MachineRecord.coo` and `cooSeenAt`, written from every `GET
/v1/machine` and at pairing; the mini's `coo` capability needs `hollow serve runner-coo
on` there), said within the last 24 hours (a runner that stopped answering ends it; a
record from before `cooSeenAt` counts its `lastSeenAt` only within the hour, until the
first check stamps it), and this is the app (`hollow serve` never dispatches). The app
asks such a Mac again hourly (`dispatcher_coo::start_refresh`, behind the check-ins'
gate: queue owner, background agents allowed; a look every 10 minutes for Macs that run
a runner COO or never answered, all at once, each for at most 20 s, and a Mac asked on
the timer is not asked again within the hour), so the dispatcher stays on while the
runner answers. Nothing paired that way: the prompt is
the single COO's byte for byte.

- **Prompt:** the worker-batch bullet (`WORKER_GUIDANCE`) is swapped for
  `dispatcher_coo::note`, no longer than it, so the prompt stays within budget.
- **`sessions.route_to_runner {request, workspace?}`** (Write, COO conversations
  only): asks the runner for its COO's key now, then sends the request as text to the
  runner COO's chat, or with `workspace` to that project's lead there. It returns when
  the runner took it, with where it went and its state, never that chat's words (S26):
  the answer stays there, read through the peer link's remote chat view or the phone.
  Offline, busy (never held) or no lead there say nothing was sent or queued. It has a
  card (where it goes, that it runs with that Mac's accounts, the request's first 600
  characters) that asks by the chat's mode exactly as `sessions.start` does (never in
  bypass): the request can carry text from notes or web pages, and the runner holds the
  person's GitHub, Google Cloud, Linear and Claude logins. Not in the unattended `SAFE`
  list, serve or Ask Hollow. A timeout asks the person before a retry (each route is a
  new message). The routed text says only that the dispatcher sent it, not the person,
  and that its instructions are untrusted; the runner COO's schedule line says to do
  nothing risky from such a message unless the person confirms in their own chat, and a
  message to a lead (whose prompt has no such line) carries that sentence itself.
- **Never long work here:** local work from a COO conversation, or a chat under one by
  its parent chain (a COO subagent), asks the person on a card in every mode
  (`dispatcher_coo::guard` in `app_broker::execute`): `sessions.start`, `start_workers`,
  `swarm`, `send` and `steer_session` into another local chat (not a COO conversation), `resume`,
  `resume_all`, `resume_at`, `handover` (but the COO's own, which continues the
  dispatcher), `automations.run_workflow` and `extract.run`. The Allow is their yes; the
  action's own card (a stronger mode) may follow it, and the guard card says so. The
  parent walk follows `parent_key` to the top (up to 1,024 links, stopping at a cycle).
  `automations.save` is not guarded: it is Confirm and asks on its own card.
  Automations and a closed window get the refusal. **Unattended COO turns (check-ins,
  wakes) can't start or steer local work while a runner COO is paired** (their
  `sessions.start`/`send` in `SAFE` meet the guard's refusal), and can't route (not in
  `SAFE`): they read, write notes and tasks, and suggest.
- **Check-ins** stay on this Mac (they ask the person); the runner COO never checks in.
- **The setting (PR2):** `agents.cooDispatcher` (`AgentSettings.cooDispatcher`), `auto`
  (absent, the default) or `off`; anything else saves as `auto`. `off` makes
  `dispatcher_coo::active` return none whatever is paired, so the prompt is the single
  COO's byte for byte, nothing is guarded, and a route is refused with the setting's
  words. Agents change it through `change_setting` (Confirm, section `devices`), which
  asks on a card in every mode, bypass included (`app_actions::ALWAYS_ASK_SETTINGS`,
  `always_asks_call`; a `set_agent_settings` that changes it too), and no automation or
  check-in can change it, since `off` lifts the guard on the COO itself; the
  person in Settings → Devices → COO on this Mac, under Macs (`src/settings/CooDispatcherRow.tsx`,
  saved at once), a row shown once a paired Mac runs or ran a runner COO, or while it
  is saved off. The next COO turn reads it.
- **Status (PR2):** `main_agent_status` and `sessions.main_agent_status` carry
  `dispatcher` (`dispatcher_coo::Dispatcher`): `dispatching`, `setting`, `machineId` and
  `machine` (the runner COO's Mac, also while off), `since` (`MachineRecord.cooSince`,
  stamped when that Mac's `coo` goes from none to a key, pairing included, kept while it
  has one, cleared when it has none; absent on older records), `off` (`not-the-app`,
  `setting`, `no-runner`, `pairing-needed`, `runner-quiet`, `runner-not-started`) and
  `whyOff` in plain words.
- **The routed turn, here (follow-up):** the route's row in the transcript gets a chip
  (`src/components/RouteChip.tsx`, "Sent to Mac mini's COO") that opens that chat on that
  Mac, read live. The hub watches the routed chat through that Mac's list stream
  (`Hub::follow_routed`, started for it when the window doesn't follow the list, stopped
  when none waits, forgotten after 12 hours) and posts one Hollow notification when it is
  neither queued nor running again ("Routed work ended on Mac mini", Open:
  `peer-chat:<machineId>/<key>`); no words of it. **Kept across a restart (Lane M,
  2026-11-18):** the waiting turns are in `<support>/Peers/routed-turns.json`
  (`routed_store.rs`, `0600`, written whole and atomically, at most 64 entries). The
  app's boot (`dispatcher_coo::start_refresh`) reads it and watches again
  (`Hub::resume_routed`, idempotent), so a turn that ended while Hollow was closed posts
  its notification at the first list. A turn is `pending`, `delivered` (claimed before the
  notification is posted: a crash between loses one, never sends two; a failed post gives
  it back) or `expired` (past 12 hours, no notification); done entries stay a day as
  tombstones, then go. A chat with a turn waiting takes no second entry.
- **A runner COO with no conversation yet (follow-up):** the runner lists `coo-start`
  instead of `coo`, so the status says `runner-not-started` and names that Mac, and
  Settings → Devices → COO on this Mac offers **Start its COO** (`peer_start_coo`,
  user-only): `POST /v1/machine/coo` starts its first conversation there with a fixed
  greeting, the record names it and this Mac dispatches at once. The hourly check keeps
  asking such a Mac. Plan: vault `outputs/hollow-build-run-2026-11-14/m3a-plan.md`.

## COO presence

`MainAgentIndicator` in `src/components/SessionIndicator.tsx` is the same SVG at the top bar (22 px)
and COO space (40 px). `src/styles/main-orb.css` gives resting/watching a breathing aura, working opposing
arcs, waiting a pulse with a stable conversation count, failed a broken ring/cross, and night a crescent and slower
orbit. Paused by the cost cap retains a dotted ring. Static shapes and words remain under either reduced-motion gate.
Per-instance IntersectionObserver and document visibility pause every animation; window-away uses `--ambient-play`.
Unmount disconnects/listeners; without IntersectionObserver it stays static. No frame loop or polling was added.

`mainAgentNow` shares the state and count between both surfaces. Waiting and unseen failures come only from the
COO's own conversation keys via `sessionState`, with the page's open conversation included. Waiting/failure
remain above work, but words retain concurrent live work. `mainAgentAttention` deduplicates keys. Night requires both
a running night item from the existing fetch and its matching claimed/running run-queue row (`runningNightCount`).
The night table settles on the scheduler's minute tick, so its status alone is insufficient. Queued work, unknown or
finished queue rows, and clock time do not qualify. Manually launched overnight lanes (and, before they were cut,
Runbooks) without night-queue evidence do not get a moon; their ordinary working evidence still applies. Existing refresh cadence
and click/keyboard entry are retained. Native VoiceOver and installed appearance remain separate checks.

## Mission Control activity tracks

`MissionControl.tsx` stays lazy. Its default Tracks view groups queue/feed turns by workspace identity (not a guessed
lane name), with equal-width chronological segments, explicit states and the existing selected-turn actions.
Orbit/List preferences persist. `missionTracks.ts` is the pure grouping, phase and cost projection;
`components/MissionTracks.tsx` renders SVG segments with native buttons. Finished details freeze elapsed time at the
recorded finish. j/k and arrows select, Tab reaches segments, V cycles all three views.

Building/Gating describe a matching active direct shell tool, never success. No authoritative merge receipt exists
in this feed, so Finished never becomes Merged from assistant text. Automations still lack streamed lines (A7).
The feed retains 24 conversations and three lines; the queue returns recent finished turns filtered to today.
The recorded-cost meter covers only these returned turns and separates unknown finished costs from pending live turns.
The separate header uses the existing day ledger, refreshed once a minute; neither is a full overnight total.
No new poll. Sparks use transform only, with per-segment intersection and visibility cleanup; both motion gates and
the existing window-away setting stop them. No observer support means static. Native VoiceOver remains a hand check.

V4 raises only the lazy MissionControl chunk budget from 20 to 22 KiB (measured 20.3 KiB with Tracks). The app and
launch-set ceilings are unchanged; no chart dependency or eager import was added.
