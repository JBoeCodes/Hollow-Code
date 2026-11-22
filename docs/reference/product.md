# What Hollow is: the current shape

Conversation menus offer optional automatic settlement after seven idle days
or a specifically verified PR merge. Policies start off, apply once to current
activity, and keep unfinished or attention-needed work visible. The open window
checks once a minute. Settled retains the reason, Wake and Undo.

## Home overview

Conversation menus offer Pin, Snooze until (quick times: in 1 hour, in 3 hours,
tomorrow morning, this weekend, next week, or a date and time picker), Wake and
Settle. Every one of Settle, Snooze, Pin and Wake shows a 5 second Undo notice;
Edit → Undo Chat Action (no chord, so text fields keep ⌘Z) runs the newest. Settle is the former Archive action, with a reason and Undo;
Library Conversations still holds archived chats. The sidebar has counted
Pinned chats, Snoozed and Settled folds. A current chat remains in its normal
home; attention and new activity override snooze. Expired old snoozes stay
reachable, with Ready chats for conversations outside project-lead and Quick
chats rows. Wake opens that exact conversation. Nothing is deleted.

Local running and queued sidebar rows have
one counted Working fold, initially closed. Attention rows lead Recent, and the
current row stays visible. Project leads and Subagents remain their existing
homes. Paused rows bypass the recent cap so their Resume route stays reachable.
Each Working fold remembers its open state and retains the row's action menu.
Quick-chat and organized chat rows also carry status marks (`rowStatus.ts`,
`store/rowStatus.ts`): Approval or Input while waiting (from pending approvals
and the Needs you list), Plan when a proposed plan arrived in a chat not on
screen (this window only, cleared when the chat opens), an unread dot (turns
since `chatVisitMark`'s last visit; never for a chat never opened), and the
linked pull request (a build workspace folder's PR from the sidebar's git
states, or the PR an automatic settlement recorded).
The Quick chats heading has Select chats (also Cmd-click, Shift-click for a stretch: a bar
with Settle, Snooze…, Move…, All and Done; each chat is written with its own revision so a
changed one is left and counted; one 5 s Undo wakes the settled or snoozed ones) and View
(`railView.ts`): sort (recent, title, newest started), show (all, needs you, working,
unread, with a pull request), group (none, agent, day) and up to 12 saved views, all kept in
localStorage. A chosen view lists every match up to 50 in the order asked, with no Working
split; the default keeps attention first, the cap of 8 and the Working shelf.
Subagent chats (`forkKind` subagent with a listed parent) nest under their
lead in Quick chats and in a project's row (`railChildren.ts`, `RowChildren.tsx`):
a count button folds them; a row whose child works or waits opens by itself and
shows the child's state; the fold choice lasts until the window reloads. Every
quick-chat, organized-chat, child and project row renames in place (double-click
or Rename… in its menu; Enter or leaving the field saves, Escape cancels).
A running row shows its elapsed time (`45s`, `12m`, `1h 05m`) from the moment
this window saw it running (`store/sessionStates.ts` `runningSince`, one shared
second clock). The Working, Snoozed and Settled folds are shelves (`rail-shelf`):
the sidebar's flex column anchors them at its bottom, under every other group.

Dragging a chat to Pinned, Active or Settled, the row details card and file drops on a row are in
[Sidebar interactions](sidebar-interactions.md); the Preview mini player and agent cursor in
[Preview player](../PREVIEW_PLAYER.md).

Sidebar project and quick-chat rows have a visible actions button, including
remote chats. It opens the same menu as right-click, so Rename, Move, Archive
and remote restrictions keep one owner. Opening a menu never opens its chat.

Work's populated Home leads with a 56px folded-threshold crop, written
status links and compact resume rows. Running owns the detail that used to
sit on Review, Today and Last night (cut overnight 11-08 S3 and S4). Home
no longer repeats waiting-item or night lists.
Every workspace keeps its header, filters, Find and Show all N
workspaces visible but mounts rows only after Show all, Find text or the rail's
All workspaces request (which focuses Find). Hide workspaces folds all rows;
pins use pin/pinOff.
All work adds Recently deleted (P1-41): workspaces in `.trash/`, newest first,
each with Restore (`restore_workspace` moves the newest copy back; refused while
the id is taken). Delete…'s snackbar offers the same restore as Undo.
Connections & imported work mounts its mirrors only while open; closing it
also stops both timers.
Return-card ID/order and session changes share a two-second trailing refresh.
Initial data, Retry and the minute refresh bypass that delay. Last-good cards
remain during errors: stale refreshes use status, first-read failures use alert.

## Quick entry

The Quick Capture and Quick Ingest windows are gone (S5, 2026-11-03).
`Cmd+Alt+Shift+T` and the tray's Quick Capture Task… bring Hollow forward on
the Add sheet below set to Task (`openAdd('', 'task')`): its why line keeps
the window's syntax hint, and its preview shows every field the window's
chips did (due and time, duration, repeat, priority, project, labels).
Import a source, from Library's ⋯, the palette, `Cmd+Alt+Shift+I` or the
tray, opens it set to a Library source. Its textarea keeps pasted
paragraphs. Enter saves where the windows did (S5 review): a Task the sheet
was set to or that was picked, and a single web address as a Library source;
Shift-Enter adds a line there. A note, a guessed task and other text keep
Enter for new lines, and Command-Enter always adds or imports; the hint by
the buttons says which. Drafts, failures and late results follow Add's rules
below; the visible New task and New → Add routes remain, and no shortcut
changed.

Both keys and their tray items reach an open Add sheet (`appMenu.ts`
`ADD_KEYS`): it follows the new kind unless one was picked in it, keeps the
draft and takes focus back. Behind any other dialog they name it ("Close
“Command palette” first, then press the key again"); nothing is queued. A
sheet they opened never moves Hollow after a save (a note or workspace file
says so with an Open action instead of opening it), and steps back 0.7 s
after the save is done, an import's once it has succeeded: lib.rs
`show_main_with` records where from (`lifecycle::came_from`: the tray,
minimized, or another app) and `after_summoned_capture` hides the window or
minimizes it and hides Hollow, so macOS shows the app that was in front. Any
click, key or focus loss in Hollow, or a page change, in between cancels it,
and from inside Hollow it does nothing. A summoned import that fails brings
Hollow forward with the draft back in Add.

Top bar → New → Add keeps unsent text at sessionStorage key
`hollow.add-sheet.draft` (plain string), including across Escape and reload.
Discard draft or successful text submission clears it. File selection does
not erase the separate text draft. A saved draft takes precedence over new
seed text; Replace with selected text is an explicit choice. Picker rejection
shows its reason, cancellation is quiet. Async completions cannot close a
reopened sheet or clear different text typed while saving. A save failure after
closing shows a global failure receipt; a late note success reports its Library
destination without navigating away.

Add's kinds are Task, Note, Library source and Recording link for text, and
Workspace file or Library source for a picked file (S5). A single web address
is guessed a Library source (the old Web source kind). Import a source opens
the sheet with `openAdd('', 'source')`: the kind holds while typing, and a
kind clicked in any Add sheet holds the same way (a picked Library source
also survives choosing a file). Domain (optional) / Purpose (optional) go to
`ingest_run`. An import runs through `trackTask` in the Activity dock and the
sheet closes at once; success says "Imported N source notes into Library.",
and a failure says "Import failed" and puts the text back as the draft when
nothing new was typed since, with its kind, Domain and Purpose (in memory:
the session draft key holds only text).

Expanded canvases (the Preview pane) use a compact permission
indicator and Canvas actions menu. Title/version/saved metadata is in the
menu and frame label; expanded Versions and pin… reveals the existing controls.
The transcript's canvas header is unchanged. See [Canvas hosts](../CANVAS.md).

Part of the agent manual. [AGENTS.md](../../AGENTS.md) is the entry point and
links every reference doc; this one holds the **Current shape** list from
CLAUDE.md's "What this app is", moved there word for word on 2026-10-13 (only
relative links were re-based). Read it before changing a surface it describes:
the rail, workspaces, Results, the Preview pane and project files, build workspaces, the Main
agent, extraction, as-of history, records, the scheduler, runbooks, Chats
and the rest. Keep it current with the code in the same change, as
AGENTS.md asks.

The section's opening paragraph, its last five bullets (canvases, agents,
automations, context, local files) and the settled owner decisions moved to
[What this app is](#what-this-app-is), at the end of this doc, on 2026-11-21.
[AGENTS.md](../../AGENTS.md#settled-owner-decisions) keeps the binding summary of the decisions.

**Current shape (version 0.5.15, 2026-10-13).**

Consolidation is installed at `8e1962fd`; verification and open requirements distinguish implemented routes from full-plan completion.

- **Rail (the sidebar; overnight 11-04 S1; L8, 11-10; L1, 11-12):** Projects
  (⌘1; called Work until L1), Tasks (⌘2), Library (⌘3), Automations (⌘4), and
  Settings in the bottom bar. Search chats (S2) is the top bar's search icon
  beside the sidebar's toggle (⌘⇧K, since L1): it opens in the sidebar's
  place, showing a hidden sidebar first, projects by name and chats by title
  or by what you asked in them, with that message's snippet; its last row,
  Search everything, opens the command palette, ⌘K. New chat (a prompt with
  no project) is the COO's page, the orb; the sidebar's row went in L1.
  Below the pages, **Recent** (its heading folds it, remembered): pinned
  workspaces first in pin order, each with its pin (2026-10-21, see
  [Pinned workspaces](../WORKSPACES.md#pinned-workspaces)), then
  the others that have a conversation (the ones used here, then the rest by
  modified time; the newest 8, the open project and one with a live chat
  always listed). The rest are on the Projects page (All projects, with
  search and archived ones); the sidebar's Show N more and All projects rows
  went in L1. **One row per project** (L8, see
  [Project leads](../COO.md#project-leads-l8)): a click opens the project's
  lead, its one conversation; no conversation list, nothing to fold, no fold
  arrow, + or ⋯. Its badge carries the project's live state, with a git state
  and a pinned project's pin, a mark. Right-click, Control-click or
  VoiceOver's Show Menu (VO-Shift-M) open its menu: Pin / Unpin, New
  conversation, Open lead and the rest, the items of the project's ⋯ on Work
  and in the top bar and on Home's rows. A project's other chats are under
  its lead's Subagents chip → Earlier chats; chats with no project list in
  the **Quick chats** section under the projects (2026-11-14,
  `components/RailQuickChats.tsx`: newest first, 8 shown, right-click is the
  conversation menu, its + starts one); Library → Conversations lists every
  one. New chat, ⌘N with no project open and Home's Ask (New chat) start a
  quick chat, and so do files dropped on Home; the orb still opens the COO. Drag the sidebar's right edge to size it (200 to 420 px, remembered;
  double-click resets).
  Home and Running are the views of Work (Assignments went with the company
  in L5 and L11, 11-10).
  Today, the weekly review, Map
  and Incoming were cut (overnight 11-08 S3 and S4). Retired
  Vault, Calendar, People, Projects, Graph, Ingest and Approvals destinations
  keep compatibility redirects. Some
  backend commands that served them remain registered without a UI.
- **A workspace** is a folder under `vault/workspaces/<id>/` holding a small
  Markdown brief (`WORKSPACE.md`), `PLAN.md`, attached sources, `outputs/` and
  session records. An open workspace shows its conversation; the top bar's
  **Project files** opens its notes and Results in the Preview pane (below).
  Contract: [Workspaces and Library](../WORKSPACES.md).
- **Results.** Outputs are called Results everywhere a person or agent reads
  (decision 7). The folder stays `outputs/`, and view ids, IPC names
  (`list_workflow_outputs`) and the `output_glob` key keep their names.
  Results joins `outputs/` with origin receipts, recorded checks and the
  person's review (draft / ready / accepted / changed since accepted) in
  `{workspace}/.hollow/results.json`. Only a person accepts or reopens a
  Result; agents and automations (`hollow_app`) may only mark one ready. A
  saved chat canvas is one Result of kind `canvas` (its `.canvas.html`,
  titled from the sidecar, with its revision); see the saved-canvases gotcha.
  See Results store.
- **Waiting on you** is the one surface for everything waiting on the person
  (chats a restart interrupted, approvals, questions and unanswered
  ask-canvases, Results to review, review items and proposals, failures, sync
  conflicts, conflict copies, unsaved drafts), reached from the bell's Needs you
  tab. See all opens the full waiting view (also ⌘5). See
  [Waiting on you](surfaces.md#waiting-on-you-merged-plan-wave-3-decision-7).
- **The Preview pane** (run 15 P1, 2026-10-27; `components/PreviewPane.tsx`,
  `store/preview.ts`) replaced the side panel. It is the one right-hand pane,
  beside any page, hidden until the person clicks a file or a link (or an
  agent navigates to a web page), never opened by a finished turn. It shows a
  file by type (Markdown, HTML in the sandboxed frame, images, code, CSV/TSV
  as a table, JSON, video; PDF, audio and anything else as details with Open
  and Reveal), a
  web page (the browser's web mode) or a canvas's Expand, with back/forward,
  Open, Reveal, Copy path, a Files menu, × / Esc / ⌘\ and a
  remembered width. Since 2026-11-06 all of that sits in one header row with
  the tabs; a tab's tooltip and the file's ••• menu give its kind, size, time
  and full path. Since 2026-11-17 a text or Markdown file also has **Review
  notes**: leave a note on any line, resolve or delete it, and send every open
  note to the chat as one draft message for one revision turn (the notes follow
  their lines when the file is edited).
  See [Preview pane](frontend-state.md#preview-pane-and-its-web-mode).
- **Side panel summary card Recall** (RS1 part 1, 2026-11-15): after Sources,
  a Recall section appears when the active chat has recalled learnings. It
  shows a counted header, collapsed by default like Outputs, Subagents and Sources.
  Section buttons open independently and save their choices per machine in
  `hollow.sideCard.sections.v1`. Recall items start with title buttons; opening
  one reveals its statement, path, optional lifecycle and feedback. Long content
  scrolls inside the bounded card. Not relevant
  and Outdated keep the card open and append a correction prompt to the draft,
  asking for a review proposal before any trusted context change.
  RS1 part 2 removes the composer Recall pill and ChatBody's receipt feedback
  callback; the side panel owns those correction prompts.
  RS1 part 3: a chat reply's run record no longer lists Recall (`hideRecall`
  on `ExecutionReceipt`); an automation's run record still does. The card
  keeps the newest reply's recall after the turn ends (it reads the latest
  assistant turn's receipt when no turn is running).
- **Side panel card header and git rows** (t3code parity 5.1 and 5.2,
  2026-11-20): above the sections, a header shows this Mac's name, an Open in
  menu (Finder and the editors installed here: `open_in.rs`) and, in a project,
  Add project script (drafts a prompt until parity 4.5's editor). Then, for a
  chat in any git folder (`git_card.rs`, `SidePanelCardGit.tsx`), the branch
  chip (switch, New branch, New worktree, which points the chat at the new
  folder when no turn runs), Changes +N -N and Commit and push with a message
  generated locally from the file names that the person edits first. Git
  changes refuse while an agent run holds the vault, an unfinished merge or
  conflicted files exist, or a turn runs in the chat.
  Parity 5.3: Changes opens a Diff tab (`DiffPanel.tsx`, `diffPanelModel.ts`,
  Preview target `diff` keyed by folder): uncommitted changes against HEAD
  with untracked files, or one turn's changes (a chat in its own worktree: the
  CP1 checkpoint commits), a file tree, Find (file names and diff text) and a
  Viewed box per file kept per folder and scope until the file changes again.
  Parity 5.4: More git actions, Pull request opens a read-only PR tab
  (`PrPanel.tsx`, `pr_card.rs`, target `pr`): the branch's PR, checks and
  comments through `gh pr list --head`; links open only over https.
  Parity 7.3: Projects → Pull requests (`GithubPages.tsx`, `githubModel.ts`,
  `github_pages.rs`): a folder's PRs or issues through `gh`, filters, detail with
  checks, reviews and comments, triage (comment, label, assign, close, reopen)
  behind a confirm; agents: `github.list`/`github.view`, `github.triage` (card).
  Linear links only, no second task system, no merge or review.

  Parity 7.2: the card header's Terminal button (and a New tab's Terminal
  tool) opens a Terminal tab (`TerminalPanel.tsx`, target `terminal` keyed by
  chat; shells in `store/terminals.ts`, backend `terminal.rs`): up to 4 shells
  per chat on a pty in the chat's folder, a screen drawn by Hollow's own small
  VT emulator (`terminalScreen.ts`, no dependency), keys from `terminalKeys.ts`.
  Sandbox: see [agent-tools](agent-tools.md#the-terminal-drawer-parity-72).

  Parity 5.5: Servers (this folder's running services, `ServicesPanel` in
  compact mode) and Automations (workflows bound to the chat by `in_chat`,
  `CardAutomations.tsx`) are counted card sections that read their counts
  while collapsed.
- **Side card responsive audit** (U5, overnight 2026-11-17): no defect
  reproduced. `tests/smoke/side-card-responsive.spec.ts` opens every section
  (the tallest case) at 720x480, 720x700, 900x480, 1100x600 and 1440x900, at
  100% and Largest (125%) text, light and dark. It asserts the card stays inside
  the chat pane, never scrolls sideways, shows every header title whole, scrolls
  to every header, leaves the transcript full width and the composer on screen,
  and that Preview's close route brings the card back. At 720x480 with Largest
  text the card is 180px wide and scrolls inside itself, and each section body
  (75px high) scrolls too, so a long status line wraps in about 62px. That is
  by design; widen the body only if a control is ever lost.
- **Project files in the Preview pane** (COO rebuild L4, 2026-11-10; the
  workspace panel and its sections Overview, Brief, Plan, Results, Sources,
  History, Changes and Notion project are gone). The top bar's **Project
  files** opens the project's WORKSPACE.md in the Preview pane; the pane's
  **Files** picker pins WORKSPACE.md, PLAN.md, SOURCES.md and AGENTS.md, then
  a build workspace's changed files (and its Changes tab), then the
  conversation's files (`src/projectFiles.ts`). PLAN.md's checkboxes tick in
  the Markdown view (`toggle_plan_row`, refused when the row or file moved
  on); a task's From: plan opens PLAN.md at its row. A file in a workspace's
  `outputs/` opens with its **review bar** (`components/ResultReviewBar.tsx`:
  state, checks, origin, Mark ready, Accept, Compare with accepted, Ask for a
  revision, More); Needs you, Library, Home, chat chips and run records open
  a Result there, its workspace first. A build workspace's branch in the top
  bar opens its **Changes** tab. Decisions are in Library → Notes → Library folder → Records
  (`components/RecordDecisions.tsx`: Revisit, Settle again). Overview's job is the
  project lead's; the curated `section="..."` canvas view is gone (canvases
  stay in chat and open in Files); earlier versions of a note are in History
  & recovery.
- **The conversation stays mounted** behind Tasks, Library, Automations,
  Settings and Needs you (overnight 11-04 U4, `App.tsx` `.page-retained`):
  its scroll place, draft and running turn are as left when its sidebar row
  brings it back. Hidden, the Work page puts nothing in the top bar, takes no
  focus or Escape, takes no New project request until it shows, and runs no minute refresh. Home still
  mounts only while it shows.
  Library Conversations takes sole ownership
  of the shared chat while open; returning restores the workspace composer
  with the shared draft rather than retaining a second hidden composer.
- **Build workspaces** (`build_workspaces.rs`) make a git worktree and branch
  under `~/Hollow/Worktrees/<repo>/<name>` off origin's default branch; the
  workspace's conversations run there as scoped runs that hold no vault lease.
  Their Changes tab diffs the worktree, commits, pushes, opens PRs, lists
  services and, for Hollow's own repo, builds and installs Hollow
  (`self_install.rs`). A new worktree's `node_modules` is an APFS clone of a
  sibling's install of the same `package-lock.json` when one exists (OC7; `create` only). See
  [Build workspaces](../WORKSPACES.md#build-workspaces).
- **COO** (the Main agent until COO rebuild L6, 2026-11-10; M1 of the
  [main agent](../COO.md) programme, 2026-10-07) is a second built-in
  workspace (id `main`, `kind: main`): the always-on agent behind the top
  bar's orb (`MainAgentOrb.tsx`, one click; ⌘⇧M and View → COO; system-wide
  `⌘⌥⇧H`, tray "COO"). It delegates by default and may run on any agent. It has
  one surface, its page (run 15 P3 removed the drawer beside other pages). It
  keeps a notepad (its brief), a plan and your
  `STANDING_ORDERS.md` (in its context), reads across the vault, delegates into
  workspaces with `hollow_app` session tools, and opens one thread per day
  (`todaysMainSession`). Hidden from project lists like Chats
  (`isBuiltinWorkspace`); can't be renamed, archived or deleted. M2
  (`main_agent.rs`): the subagents it started (`main_agent_children`, its presence;
  the person sees them in its Subagents chip and tab since COO rebuild L3), and a wake turn ("⟲ Hollow: …", ≤24/day, in the thread's own mode) when a
  subagent it started ends while it is idle (a chat that hands over: when its last
  successor ends; a held-back wake shows its reason under its page header's status; the
  minute timer re-checks; overnight 10-11 A1). M3–M6: check-ins (a free gate every
  minute; a turn only when something is new; $100/day hard stop on what it runs
  on its own, never on chats the person asked for (`budgets::main_agent_split`). The cap counts recorded child/hand-over ledger spend while successors run (`descendants::recorded_chain_cost`, bounded32 levels/512 runs per root, each run once across roots; chain rows use their local-day start and Main-folder descendants keep root attribution); complete receipt/list totals remain nullable and unreported provider costs are not estimated (overnight B6). Settings →
  Background work → COO), unattended turns limited to safe actions in
  `app_broker::execute` (`main_agent::unattended`), swarms (`sessions.swarm`,
  parts on Claude Haiku 5.5 by default, 6 at once in their own run-pump slots,
  2 in build workspaces; `usage::UNPRICED` prices Haiku 5.5 until Claude Code
  knows it) and suggestion outcomes
  (`sessions.suggestion_outcome`). Its page has **no header band**
  (removed 2026-11-11: it took screen space and repeated the Subagents chip, Usage and Settings): the chat starts at
  the top like any project's. Its actions are in the top bar's Actions for COO
  menu (`cooMenu.ts`: Actions it took on its own, Change hard stop, the
  check-in setting, How check-ins run, Open failed conversation, Edit or Create
  default standing orders); its day (check-ins, spend against `dailyCapUsd`,
  Acts on its own / Suggests only, suggestions taken, a held-back wake) is the
  Today row in Settings → Background work → COO; the chats it started are the
  Subagents chip and tab. Palette **Open COO** (`act:main-agent`).
  Status reads refresh independently of the child list. Only an
  explicit click creates default standing orders, without replacing any
  existing file.
- **Extraction** (2026-10-09, master plan F3; [docs/EXTRACTION.md](../EXTRACTION.md)):
  `extract.rs` turns a kept recording transcript or a conversation range into
  proposals on the review queue only: tasks → `meeting_task` (with `anchor`),
  commitments → `commitment` proposals, decisions and person facts → `record`
  (F4 accepts). Each cites a segment and quotes it; unknown owners and dates
  stay unknown; `.hollow/extracted.json` keeps source hashes so nothing is
  extracted twice; near duplicates that change a value are flagged, not
  dropped. Keeping a transcript extracts it in the background. Its model is
  Settings → Advanced → **Extraction model** (overnight A11), per
  agent (`extraction-settings.json` in app data; `settings.extraction.*`, the
  set is confirm-tier with an old → new card): none chosen is the newest
  Haiku on Claude (`claude-haiku-5-5` until Claude Code says which,
  `agent_models::haiku_fallback_id!`) and the agent's Settings → Models model
  elsewhere; the agent is resolved once per call (`run_oneshot_selected`).
  **Commitment tracker** (#2): an unclear direction arrives as `unknown`
  (accept refuses until the person picks), a promise between two others isn't
  queued, Tasks → Commitments shows I owe / Owed to me, `list_commitments`
  filters by direction and person, and "N promises found" is notified.
- **As-of history** (2026-10-09, master plan F5; [docs/AS_OF.md](../AS_OF.md)):
  `asof.rs`. A nightly window job snapshots briefs, plans, tasks and decisions
  into History's object store (`.hollow-history/snapshots/<day>.json`; an
  unchanged day writes nothing; 90 days, newest kept) and writes per-workspace
  metrics. History's GC counts snapshot references. `history.read_as_of`
  answers from the newer of snapshot and agent-run version, else
  `not-captured` (never empty); `workspaces.metrics` reads the metrics.
  **Vault time machine** (#17): its Compare with a past day… view went with
  the workspace panel (COO rebuild L4, 2026-11-10; earlier versions of a note
  are in History & recovery); a chat asking "what did I know on 9/15
  about X" gets an `<as-of-context>` block (`asof::question_block`, beside the
  recall) with versions, capture dates and decisions settled by then.
- **Records** (2026-10-09, master plan F4; [docs/RECORDS.md](../RECORDS.md)):
  `records.rs`. Accepting a `record` item writes `decisions/<date>-<slug>.md`
  or `records/facts/<date>-<slug>.md` (anchor kept); a fact needs exactly one
  matching `people/` note or the person's pick. People notes get a fenced
  generated block (owes you / you owe / their decisions / facts) and the rest
  stays byte-for-byte. Settled decisions reach agents through trusted recall
  only when relevant. Supersede is confirm and always asks; revisit is
  user-only. **Decision ledger** (#20): agents propose with evidence
  (`records.records_propose_decision`; a conflicting settled decision is
  flagged on the card, never edited), a one-time brief importer on request,
  `asOf` reads, a generated block in `me/Decisions.md` (global settled; never
  created by Hollow), and Library → Notes → Library folder → Records
  (`RecordDecisions.tsx`: Revisit, Settle again; it was the Overview's
  Decisions block until COO rebuild L4).
  **People dossiers** (#3, `people_dossier.rs`): names match only an exact
  title, file name or `aliases:`; an unmatched name from extraction becomes a
  `person_alias` review ("Is 'Ana' Ana Ruiz?" / "Add a page for …?"); the
  generated block adds Last contact and Open threads;
  `records.records_refresh_people` regenerates the blocks.
  **Process capture** (#10 stage 1, overnight B5; `process_capture.rs`):
  `process.capture {workspace, video, transcript?, everySeconds?}` turns a
  recording in the workspace's `sources/` into
  `outputs/process-captures/<name>/capture.md`, locally (no model): ≤60
  frames (1280 px, every 10 s or wider to cover an `ffprobe`d length) by
  Homebrew `ffmpeg` with `-protocol_whitelist file`, taken in a temp folder
  with no lock (`Lease::Own`: only `commit`, the copy-in and note write, runs
  in `app_broker::live_write`); `frames/` is neither Result nor Activity
  (`process_capture::is_frame`). A transcript given or found beside the video
  (`.srt`, `.vtt`, Whisper `.json`, timestamped `.txt`/`.md`, ≤5 MB; Hollow
  doesn't transcribe yet); draft steps split at 1.5 s pauses, cue words or
  400 characters, each with its time and the frame on screen then. Paths go
  through `vault::safe_join`. Recordings go into `sources/` by Finder: Add
  source stops at 20 MB. **Stage 2** (morning N1, `process_skill.rs`; Hollow
  calls no model, the chat's agent fills in): `process.draft_skill` writes
  `<capture>/skill/SKILL.md` from capture.md's steps and Inputs/Outputs
  (`[fill in]` / `_Describe this step._` markers) and copies the listed
  workspace files into `fixtures/run-1/{input,expected}/` (≤20 MB each, 50 MB
  total; links out of the workspace skipped; refused when SKILL.md exists).
  `process.test_skill` (confirm, in auto's ask list) checks SKILL.md, runs
  `skill/scripts/run.sh|run.py|run <in> <out>` per `fixtures/<case>/` on
  copies in a temp dir (cleared env, 120 s, own process group via
  `process_capture::run_bounded_until`, which also ends it when the run is
  no longer live; pipe readers get a 2 s grace), compares `expected/` (text
  ignoring CRLF and trailing spaces), writes `test-results.md` for reading
  and keeps the record Save trusts (SHA-256 of `skill/` + `fixtures/`,
  Finder/git junk ignored, and the counts) in app data
  `process-skill-tests.json`, so editing the note can't unlock Save. No
  script, or no `expected/`, = skipped; script with no case = fail; links
  refused. `process.save_skill` (confirm, in auto's ask list) refuses unless
  that record has no failure and the digest still matches, then stages
  `skill/` through `skill_import` (re-checking the digest) and installs from
  the card's token; fixtures never install. Drafting the automation/runbook
  isn't built.
  **Pressure tests** (#8, `stakeholder_sim.rs`;
  [docs/PRESSURE_TESTS.md](../PRESSURE_TESTS.md); the screens and palette
  actions were cut overnight 11-08 S4; asked in chat, the agent reads
  `personas/<role>.md`): personas
  `personas/<role>.md` for CEO, CFO, Ops VP, IT are drafted only from vault
  evidence (`personas.personas_draft`: matching people roles, settled
  decisions on the role's topics, `me/Decisions.md`, accepted facts; numbered
  `[E<n>]` anchors in a Hollow-written block; none → refused) and used only
  after the person accepts the exact bytes (`personas_accept`, user-only;
  SHA-256 in `<support>/Personas/`, so any edit needs acceptance again;
  each entry HMAC-signed with an owner-only key beside it, an unsigned or
  edited one counting as not accepted, morning P2).
  `simulate.pressure_test {doc, personas}` (a chat the person started) runs one `plan`-mode chat per persona,
  one at a time (vault lane), in an empty scratch folder that runs bare (no
  Hollow tools or shared connectors, `mcp_attach::bare_scratch`, as bake-off
  variants' copies do) (Settings swarm
  model, at most the swarm limit, within the COO's daily cap, which
  counts their cost); a point without a shown evidence id is dropped; Hollow itself writes the report, labelled "simulated critique", to
  the draft's workspace `outputs/pressure-tests/<date>-<slug>.md`. Local only.
- **Scheduler v2** (2026-10-09, master plan F1; [docs/SCHEDULER.md](../SCHEDULER.md)):
  named windows (`tonight`, `weekday-evening`, `friday-pm`, `nightly`) in the
  Mac's zone, edited on the Automations page, in the **Automation windows and the run queue**
  fold (Settings PR 4; the overnight cap is edited in Settings → Usage → Budgets since P1-07) (`pages/workflows/AutomationWindowsPanel.tsx` + pure
  `automationWindows.ts`, lazy; user-only `set_automation_windows`; built-ins
  keep their names, custom windows can be added/removed; overnight S10); an automation can say `schedule: window:friday-pm` (catch-up in
  the same stamped string: `/run-all`, `/skip`; run-once-on-wake by default); host window jobs run once per
  occurrence; the night queue (`night_queue_*`) drains inside `tonight` at
  acceptEdits under the M4 unattended limits and a $40 overnight hard stop
  shared with the `overnight` budget scope (`night_queue::night_budget`); its
  turns carry `unattended` (long-work account routing, R1 failover in-turn),
  and each Claude night turn is ended at a safe point once the night's
  committed spend plus every running turn's estimate reaches the stop
  (`night_queue::NightSpend`, per run id; the ended row keeps the estimate;
  the turn's close goes through `chat::close_night_stop` and
  `chat::reconcile_night_spend`; tests run both after a fake-CLI drive, and
  pin that `execute_turn` calls them, since it can't run in a unit test).
  The overnight shift (#1): up to 3 items at once, at most 2 taking a slot
  (code, or a non-Claude agent); a code item gets its own build workspace on
  local branch `night/<night>-<id>` (no push/PR/install; removed again if its
  start fails); a non-code item can't name a build workspace; runbook items
  run an approved runbook unattended, never in a build workspace (refused when
  added, in the dry run and by `runbooks::open_checked` for any unattended
  run; morning M1); running items hold back their budget
  (at least $5, or what they have cost so far when that is more), Claude
  turns are guarded mid-turn as above, and all are stopped at the cap;
  unattended turns get no shared connectors (`connectors::for_turn`), no
  teammates, computer or services (`mcp_attach::for_unattended`), and can't
  start or steer chats in build workspaces; after the window the morning
  report goes to `workspaces/main/outputs/night/<night>.md` and one isolated
  COO turn presents it (`night_queue_morning_report`). The read
  action `system.overnight_summary` (`overnight_summary.rs`; words in
  `src/overnightCard.ts`) stays. The Home Last night status link that
  opened it was cut overnight 11-08 S4. The command covers the budgets' overnight window,
  18:00 yesterday to 07:00 today local (capped at now), all day until the
  next morning, and past 07:00 what the handover chains that *started* in
  it did until their last row, receipt or PR (never past the next 18:00;
  a chain's successors join whenever they started; a night chat reused for
  morning work still counts until then; nothing else after
  07:00 counts; `to` says where it stopped). The action's optional
  `from`/`to` (RFC 3339, local `YYYY-MM-DD HH:MM`, or a date) set an exact
  window instead. It lists receipts that started then (failed = not
  `success`/`cancelled`, or a check whose last run failed); PRs from the
  night's shell `toolUse` events running `gh pr merge <n>`/`gh pr create` as
  a simple command (or in `bash -c`; a heredoc's body skipped, the commands
  after it read, opened by an unquoted `<<` outside a comment or one inside
  `"$( … )"`, never a `<<<`, and closed only by its exact delimiter line;
  not `--auto` or
  background calls) whose `toolResult` isn't an error, paired by
  conversation, turn and id, with truncated inputs read from their preview
  (agents' merges only, not GitHub's view; a merge with no `-R` takes its
  create's repository, else its chat's receipt's); moved repo heads, one row per
  repository; the live waiting count (`waitingBadge`); and the evening's
  morning report (`night/<evening>.md`). `events(at)` is indexed for it.
  Its reply also groups the night's usage-ledger rows by handover chain
  (`chains`: chats, turns, stints, PRs, known cost, runs with a NULL-cost
  row as `unknownCost` (each run once), running rows as `inProgress`, peak
  context (rows' `usage_json` or receipts' `apiUsage`), $/merged PR)
  with the night's totals (S06; NULL-cost rows with no run id, mostly Jev
  requests, are `unknownCostCalls`); rows whose agent never started are left
  out, and predecessors come from the handover records, else the handover
  registry. Home shows only run/failure counts. The Last night link was cut
  overnight 11-08 S4. The strip owns one summary read; the duplicate
  inline PR detail and dismissal are removed. The old localStorage key
  `hollow.overnightCard.dismissed` remains untouched but unread. Home's status strip
  (Due, Waiting and Running now tiles) and its resume rows were never mounted by
  chat-first Home and were deleted on 11-10 (GlanceStrip, HomeReturnRow and the
  glance model); due tasks are a Tasks filter, waiting items are in Needs you,
  and running work is in Work → Running.
  Every
  workspace shows six pinned-first rows (in pin order), then Show all N workspaces; Find and
  a full-list request expand it. Pins and row menus remain. Connections
  stays below the list.
  A **dry run**
  (overnight A13, `night_queue::simulate`/`dry_run`; `automations.night_queue_dry_run`)
  pretends tonight over the queue as it
  is: the shift's own `decide_drain`/`decide_stop`, the start checks read
  without effect (`start_problem`, as `start_item`/`open_checked`), fake
  agents finishing at each item's budget or $5 reserve; mid-night it starts
  from what runs and what was spent; with no `tonight` window it says nothing
  would start. It returns the morning report that night would make. Nothing
  runs, is claimed, sent, spent or written. The Dry-run tonight screen was
  cut overnight 11-08 S4. The command stays until the Rust removal.
  The Friday weekly update (#4, `digest.rs`) is the first window job: drafted at
  15:00 into `workspaces/main/outputs/weekly/<Friday>.md` with evidence per line,
  then one COO prose turn; nothing is sent. The same job writes the week in
  review's snapshot (`weekly_review::save_snapshot`, `outputs/review/<Friday>.md`) once
  and notifies "Your week in review is ready".
  The gardener (#19, `gardener.rs`) scans nightly and keeps one review card
  (kind `gardener`): clean idle worktrees (never one a pending build or an
  app's provenance holds, `worktree_hygiene::held_for`), idle workspaces, duplicate learnings,
  `[Hollow test]` tasks, conflict copies (action `resolve`: listed, never
  applied or ticked); Apply ticked applies only ticked lines a fresh scan still
  finds; `gardener.apply` is always-ask. Never deletes or touches the tracker.
  Its idle-workspace lines need no real open plan rows (seed rows don't count;
  `workspace_shape.rs`). The Suggest what to archive dialog (11-08 S4) and its
  `workspaces.archive_suggestions` action (11-10 S6) were cut. The COO
  suggests archiving in chat. Archive one workspace stays on its menu.
  A workspace **never used** (`workspace_shape::never_used`: only the
  template files, brief sections other than Outcome still the template text,
  plan only its seed rows, no conversations, no worktree, not tracker-managed or
  built-in) is proposed by the gardener from a day old, whatever the threshold
  (overnight 10-11 Y1). Connecting a Projects folder
  (`directory_connections.rs` `materialize`) makes a `local-<hash>` workspace
  only for a Git repository or a folder with ≥ 5 supported files, and never
  for a folder named like a current `notion-*` workspace (title key:
  lowercase letters and digits; ties go to the lower id); other folders get
  theirs when the person clicks **Open project workspace**
  (`open_project_workspace`, user-only, under the connection lock; the legacy
  Notion twin opens instead when there is no own workspace or only an archived one).
  The twin keeps the folder's scope: `resolve_scope` gives a `notion-*`
  workspace named like a connected project that project (basis
  `notion-twin`), and external sessions in the folder route to it
  (`project_homes`). A connected `local-*` workspace still in the index is
  never "never used", and any conversation ever run in a workspace (archived
  or older than the 1,000 counted) is use (`ever_had_a_conversation`).
  Workspaces made before stay.
  **Conflict copies** (overnight A10, `conflict_copies.rs`): `Note 2.md`,
  `Note (1).md` and "conflicted copy" files beside their original, read by
  `vault.conflict_copies`; one Waiting on you item (kind `copies`) and the
  palette's **Resolve conflict copies…** open `ConflictCopiesDialog.tsx`
  (diff, then Keep the original / Keep the copy after a confirm; the other
  goes to `.trash/`; keep-copy is one atomic replace, `vault::replace_from`;
  refused while a run holds the vault or a chat runs in that workspace, and
  over a protected original), or **Not a copy** (morning M7: a real title such
  as `Chapter 2.md` beside `Chapter.md`; no confirm, Undo in the dialog's note;
  marked by both paths in `<support>/ConflictCopies/<vault>.json`, outside the
  vault, so the scan, the bell's cached scan and the gardener skip it). The
  dialog keeps Tab inside (`holdFocus`). `conflict_copy_pair`,
  `resolve_conflict_copy`, `dismiss_conflict_copy` and
  `undismiss_conflict_copy` are user-only; the resolve binds both files' hashes.
- **Executable runbooks** (2026-10-09, master plan #11, runbooks.rs, removed 11-10 in S6;
  [docs/RUNBOOKS.md](../RUNBOOKS.md)): a note with `runbook: true` whose
  top-level `- [ ]` lines are steps (`action`/`args`, `skill`, `prompt`,
  `check`, `confirm`, `on_fail`). A run is one ordinary chat turn per step in
  one conversation of the runbook's workspace (no second engine); receipts are
  Hollow's own (the turn's queue row, the action's call in its tool calls, a
  check Hollow ran: `exists`/`nonempty`/`contains`; any other check is
  `unchecked`, never ✓). Run copy `workspaces/<id>/outputs/runbooks/<name>/<day>.md`;
  the note is never written. `runbooks.run` cards the first run of each
  version per workspace and needs the dry run's `expectedHash` (approval
  binds that hash; `runbook_approvals`; asks in auto too); never from
  automations or a runbook's own step turn; turns at the caller's mode,
  capped at `auto` (`acceptEdits` unattended). A rerun the same day skips
  settled steps and never re-runs a step whose turn ran or began on its own
  (re-check, else it waits); confirmations are used up per turn. `confirm:`
  steps and external actions wait (`runbooks.confirm_step`, always-ask;
  `runbook_confirm_step` is user-only). Tables `runbook_runs`/`runbook_steps`
  in the session store. The Runbooks, Tonight and Last night screens were
  cut overnight 11-08 S4. Automations is the list (`pages/Workflows.tsx`).
  runbooks.rs and the night-queue commands went in the Rust removal (S6, 11-10).
  `runbooks.confirm_step` stays always-ask. Do this tonight (the Plan row
  button, the night line and the orb suffix) was cut in the same change.
  `night_queue_add` stays until that Rust removal.
- **Quick chats** (called Chats from 2026-11-04 to 2026-11-14) is a built-in workspace (id `quick-chats`) for chats with no
  project. Conversations can be moved between workspaces
  (`move_agent_session`), which starts a fresh native session with the
  conversation bundle. A chat moved into a project leaves its lead a one-line
  note, given once at the start of the lead's next turn as a `<hollow-arrived>`
  block (`chat_chain::record_arrival`, `take_arrivals`; quick chats, 11-14).
  The COO's Earlier chats no longer list the quick chats. A whole workspace can be **merged** into another
  (`workspace_merge.rs`, master plan #18): the rail's **Merge into…**, the
  palette's **Merge this workspace into…** (open workspace), or
  `workspaces.merge_workspace` (confirm; asks in auto) shows every file in
  `sources/`/`outputs/` (a name taken, without case, gets `-merged[-N]`; a
  folder pushed aside keeps all its files, a name taken inside it suffixing
  the file only (a folder clash inside it gets that part a folder of its own); a source's `.provenance/<name>.json` follows its file;
  symlinks and `.DS_Store` never move) and every conversation (archived too),
  refused when the target would then hold 100 or more source files or a
  sources list within 2 KB of 48 KB (it couldn't take another attachment;
  morning P1), bound to the
  plan's hash (`preview_workspace_merge`); then conversations move as chat
  moves do, files move with `vault::move_note` (no replace), the source's
  brief/plan/sources list are appended under `## Merged from “…” (<id>),
  <date>` (headings two levels deeper, renamed files relinked; the target's
  `plan_task_candidates` reads those ids back, so merged plan rows keep the
  tasks they already became, `workspace_merge::merged_from_ids`), and the
  source is archived with `merged_into: <id>` (`Workspace.mergedInto`),
  nothing deleted. The merge writes `.hollow-merge.json` + `MERGED.md` into
  the source (and `merged_into`) before anything moves, kept current as it
  goes; **Undo merge** (its menu, the snackbar, the source's row in Every
  workspace, which reads "Merged into …", or
  `workspaces.undo_workspace_merge`) puts back exactly that, only between the
  two workspaces' `sources/`/`outputs/`, skipping and naming anything changed
  since. A workspace with a merge recorded can't be merged again until
  undone. Not History: History keeps no copy of hidden
  files (provenance) or files over 2 MB, and restores all or nothing. The
  rail holds the workspace edit lease. Refused for built-ins, a
  tracker-managed or archived source (a legacy `notion-*` one merges since 2026-11-10), an archived target, a running or
  queued conversation in either workspace (other than the calling agent's).
  The preview (`workspace_merge::preview`) names that last refusal as
  `blocked` before the confirm, which then shows it with Merge off (P1-38);
  the confirm starts on Cancel, is described by one summary line ("Moves N
  files and M conversations into …"), says moved conversations run in the
  target's folder from their next turn, and records each one's moved notice
  (`recordMove`) as a single move does.
  The **workspace map** was cut overnight 11-08 S4. Work is Home and
  Running.
  Archive one workspace stays on its menu. Its brief helpers live on in
  `brief_read.rs` (11-10 sweep).
  **Connections** (five asks A2, `workspace_connections.rs`, read
  action `workspaces.workspace_connections {workspace?, includeArchived?}`):
  weighted edges with reasons, each with its own weight (project 0.6;
  people 0.2 each ≤0.5 by people-note names/aliases (word n-gram lookups; a one-word name only capitalised; an alias two notes share ignored) or emails (TLD-shaped, not `git@`/file names) in brief+plan;
  link 0.45 by wiki-link or `workspaces/<id>` path; source 0.45 by SOURCES.md
  SHA-256, else 0.3 by file name; task 0.3 for plan-made tasks in a project
  the other names or has tasks in; terms TF-IDF cosine ≥0.2 ×0.8 ≤0.5 over
  title×2 + brief body, df>50% dropped; chat 0.25+0.05 each ≤0.5 from
  `session_starters` across workspaces), edge = 1 − Π(1 − w), floor 0.12, a
  person/email/source signal in >40% of ≥5 workspaces or >25 ignored, a term
  in >60 pairs nothing; pairs from inverted indexes; two cache slots (with and
  without archived) on `vault::walk_generation()` + 60 s TTL; a `workspace`
  arg that is unknown, built in or archived is refused. Nothing in the
  window draws these edges. The map screen that used to was cut overnight
  11-08 S4, its action 11-10 S6. Merge into…, Archive… and Archive older entries… stay on a project's menu.
  `context_compact.rs` (overnight C4; its agent action
  `workspaces.compact_context` was removed in P0-24 and the old per-day
  picker, `archive_preview` / `archive_older_entries`, went with the workspace
  panel, COO rebuild L4, 2026-11-10; **V5a, 2026-11-15, brought archiving back
  as `workspaces.archive_shared_docs`** and the workspace menu's
  **Archive older entries…**). One shared doc at a time: `doc` WORKSPACE.md
  moves older dated Current context entries (Outcome, the rest of the
  context and Decisions stay), `doc` PLAN.md moves sections whose rows are
  all ticked (open work stays). `mode` `dry_run` (default, no card) returns
  sizes before and after, what would move, the archive note's path and the
  doc's `hash`; `mode` `apply` needs that hash as `expectedHash` (a doc that
  changed moves nothing) and the dry run's `today` (so it moves exactly what
  was previewed, even after midnight), asks on a card in every permission mode
  including auto and bypass (Write tier; `asks_in_mode` forces the apply, the
  dry run has no card), refuses while another run is live in the workspace,
  the caller's own run aside,
  refuses a move whose kept plus archived lines are not the original's
  (`is_lossless`), writes the archive note first and the doc by
  compare-and-swap, and a second apply finds nothing older and writes
  nothing. The app's apply is its own History run (the "Archive older
  entries" label); an agent's lands in its own run. A WORKSPACE.md move
  stops at the first heading inside Current context (a `###` sub-heading's
  entries stay). The menu item
  dry-runs both docs, shows one confirm, then applies each. The move itself
  is the C4 engine: it moves a brief's older "Current
  context" entries (per blank-line block: top-level bullets, or a whole
  paragraph; dated only at their start, `YYYY-MM-DD` or `MM-DD` after `- `/`**`;
  undated and "As of" lines never) word for word into
  `outputs/workspace-archive/<date>-current-context.md`, splicing the brief
  by line ranges (each line keeps its ending; byte-for-byte elsewhere) with
  one pointer line at the section's end that replaces every earlier one
  (since overnight 10 A9 only a line of the generated shape, `is_pointer_line`:
  a line of the person's that quotes the phrase stays).
  Plan sections it moved sit in `outputs/workspace-archive/<date>-plan.md`
  with one `Finished sections were moved to …` line at the end of the plan's
  lead, which a cut plan keeps. `outputs/workspace-archive/` (`ARCHIVE_DIR`)
  is not a Result (`workspace_results::is_listed`); History runs carry
  the "Archive older entries" label. **Every chat** (quick, workspace, vault; every
  provider) opens its appended prompt with **"Visual answers in Hollow"**
  (`visual_prompt.rs`, with `canvas_prompt.md` and `canvas_kit_prompt.md`;
  Canvas C3 wave 2 C4, it replaced `HTML_PREFERENCE_PROMPT`; the board guide
  stopped being sent on 2026-10-07, diagrams are `hc-flow` canvases, and
  Canvases Off now means Markdown only): plain text / a canvas / a diagram canvas / an ask-canvas
  by shape, one "how often" line from the Canvases choice, "state the answer
  in prose too", the hollow-canvas fence and id rule, the Canvas sandbox as
  served, a 2 KB kit@1 reference and (C4 A3) an **Ask-canvases** section
  (`canvas_ask_prompt.md`: an example, the kit-ask reference, how the answer
  arrives) whose "when" line follows the chat's question channel
  (`visual_prompt::Asking`: question cards for simple choices where
  `hollow_questions` is attached; an ask-canvas instead of a question card for
  Grok and Antigravity, which keep the fenced `hollow-questions` block for a
  yes or no). With Canvases Off none of the canvas parts are sent. Its claims
  must match `artifacts.rs`, both kit manifests, the A2 answer format and
  the store (`visual_prompt::tests` pins policy, elements, attributes, the
  answer block, limits and size).
  **Since agent effectiveness G1 (F6, 2026-10-07)** only Most replies keeps
  the formats (canvas guide, kit, ask-canvases: the *visual guide*, about
  8 KB) in the appended prompt. At When it helps the prompt keeps the
  decision rule, the how-often line, the prose rule, the ask "when" line and
  where the guide is (about 2 KB); at Off only the decision rule, Markdown
  only (about 0.7 KB), with no guide at all (2026-10-07). Every chat turn
  at When it helps writes the guide to `$HOLLOW_SCRATCH_DIR/hollow-visual-guide.md`, and a
  turn whose request looks visual (`visual_prompt::visual_cue`: Hollow's own
  cue words such as chart, compare, plan, timeline, board, four or more
  numbers, a named `.csv`/`.xlsx`/`.json` file), or whose guide file couldn't
  be written, carries it as a `<hollow-visual-guide>` block ahead of the
  caller's text, once per native session (`derived_state` kind
  `visualGuide`; the manifest's `compiler.visualGuide`). That block is not
  fingerprinted. After the visual block every chat's prompt has the
  **capability index** ("Hollow capabilities in this run",
  `chat::capability_index`, about 0.7–1.5 KB): one line per capability the
  run has (the attached owned servers by name, answer forms and where their
  formats are, question cards, library connectors where an agent can have
  them, skills: where the turn's `<hollow-skills>` block is and how this run
  reads a skill whole, scratch), then the agent's line of the per-provider
  capability contract (`capability_contract::guidance_line`, F9: what it
  doesn't have here and the fallback); it replaced the workspace and
  quick-chat briefs' `computer=…` list. A recorded `visualGuide` is written
  only once the agent spawned and printed, never while composing. A chat
  changing its Canvases choice restarts its native session once (the resume
  fingerprint includes the choice); editing the text doesn't, unless the
  change bumps `chat_resume::PROMPT_CONTRACT` (fingerprint v3, OB04).
  Quick and vault chats also keep the preview-pane note
  (`PREVIEW_PANE_PROMPT`). After editing it, regenerate the billed probe's
  prompts: `HOLLOW_REGENERATE_PROMPTS=1 cargo test --lib probe_prompts`
  (`tests/fixtures/prompts/canvas-real-reply.json`).

## Context cards and memory

Moved from CLAUDE.md's Context bullet (build run 11-17, 2026-11-19) to keep the entry point under its cap.

A project's agent-written architecture card (`outputs/PROJECT_CARD.md`, keyed by commit and prompt version) rides the envelope first ([`project_card.rs`](backend-module-map.md)).
  A card may link a source-derived design brief with `design: DESIGN.md` (SD1: at most 2 KB, five sections, its own prompt version; checked only when linked).
  A new build workspace gets `outputs/PROJECT_CARD_DRAFT.md`, read-only facts and a "Needs attention" list, with a fingerprint proving the repository unchanged (BI7, `first_connect.rs`).
  A skill with a `references/` folder reads as its kernel plus one named reference (`read_skill` `reference`, OC8, `skill_index.rs`); one without reads as before.

Memory candidates carry a version a card's Save sends back (stale edit refused); each capture run appends a change report to `.hollow/memory-reports/` (ME1, `memory_report.rs`, [vault](vault.md)). A learning that repeats a `me/` line gets a retire card, and its Save turns the note into a link (PX5, `offer_profile_copies`).

Canvases in short: agents answer with `hollow-canvas` fences; an ask-canvas's answers go when the person presses Hollow's own Send under it (a canvas script can't send); canvases open in Files. The full reference (fence, modes and CSP, bridge, kit, store, ask-canvases, prompt, receipts, code map, limits, risks) is [docs/CANVAS.md](../CANVAS.md); the slice reports archived docs to `CANVAS_C6.md` are history.

## What this app is

**Hollow** is a native desktop workspace for substantial work with agents.
The sidebar's pages are **Projects (was Work) / Tasks / Library / Automations**,
with **Settings** at the bottom; **Needs you** has no sidebar row and opens
from the bell's See all or ⌘5 (`src/navigation.ts`). Each workspace owns a small Markdown brief,
plan, attached sources, outputs and session records. Local files remain
canonical; Obsidian and other editors can read them without owning the product
model. Tasks and projects live in Linear ([sync](../TRACKER_SYNC.md), `sources.rs`; Settings, Sources and each task's label say where, H4b);
Teams: Engineering plus optional Personal (HOL), no other ([two teams](../TRACKER_SYNC.md#two-teams-default-and-personal-2026-11-12)). Notion is a read-only archive (two-way sync removed 2026-11-10). Needs you is Waiting only;
there is no Inbox overlay or Approvals page. Retired page links open Projects or Library; the palette and
global shortcuts expose current surfaces only. Conversation history is
Library → Conversations. See [Sources and workspace context](../SOURCE_CONTEXT.md)
for the complete surface map, context limits, source connections and compatibility.
Settings → Sources → Context's Set it up for me (CX18) is a card only: the person creates the repo.

The full **Current shape** list (rail, workspaces, Results, Waiting on you,
the Preview pane, build workspaces, the COO and every surface since) is
in [What Hollow is](product.md). Its last bullets, kept here:

- **Preview pane**: one app-wide pane (`components/PreviewPane.tsx`,
  `store/preview.ts`: `openPreview(target)`) is mounted by the shell beside
  every page, hidden until the person clicks a file or a link (or the agent
  navigates to a web page), and never opens by itself when a turn ends. It
  shows a file by type, a web page, a canvas's Expand, Subagents, Changes, Diff, a PR, a New tab's Tools (+), with
  back/forward, × / Esc / ⌘\ and remembered width
  (`sidePaneWidth`). The persistent card (`components/SidePanelCard.tsx`: header, git rows, counted sections (Servers, Automations); [product](product.md)) shrinks below 900px, hides only for Preview. Top bar
  Project files opens a project's notes there, the Files picker pins them,
  a Result opens with its review bar and PLAN.md's boxes tick
  ([product](product.md)). New code calls `openPreview`.

- **Canvases** (C0 to C6): `hollow-canvas` fences render inline in an
  opaque-origin frame (scripts on, no network), saved with revisions in
  `outputs/canvases/`. **[docs/CANVAS.md](../CANVAS.md) is the one current
  reference**; read it before changing anything canvas-related.
- **Pinned workspaces** lead the sidebar's Projects; Home orders by activity: one state, `pinned: <rank>` in each brief, written
  compare-and-swap by `workspace_pins.rs` (menus, drag, Move
  up/down, and `update_workspace`'s `pinned`/`pinPosition` for agents); never
  archived or built-in
  ([Pinned workspaces](../WORKSPACES.md#pinned-workspaces)).
- **Question cards** (`ask_user_questions`) dock where the message box is,
  one question at a time, and the last answer sends straight to the agent;
  Esc hides them behind a "questions waiting" chip
  ([agent-tools → Question cards](agent-tools.md)). The COO's
  check-in suggestions use them (Do it / Not now / Never).
- **Agents:** Claude Code, Codex, Antigravity and Grok, through one app-wide
  agent/model preference and one event vocabulary
  ([Chat](agents.md#chat-how-the-headless-agent-integration-works)). Each
  agent's menu uses its own mode names; a chat keeps its mode on every turn Hollow
  runs for it (Codex Auto, default and acceptEdits run under a `hollow_auto` profile with a nested-`.git` deny; [Per-agent modes](../PER_AGENT_PERMISSIONS.md),
  [argv](agent-tools.md#permission-modes-across-agents-2026-10-02);
  prompts and the appended system prompt stay off argv (`secret_fds.rs`; token still env, Antigravity's prompt is a stream-json stdin line since 11-17: [off argv](agent-tools.md#broker-proofs-off-argv-and-peer-checks-merged-plan-slice-6)). Claude runs fix
  background tasks off and refusal fallback off ([why](../AGENT_CONTEXT.md)).
  Grok chat turns can run over ACP (`agents.acp.grok`, off; bypass only, else headless: [Grok over ACP](agents.md#grok-over-acp-oc1-2026-11-18)).
  Secrets: one Keychain item; [1Password](agents.md#1password-secrets-parity-75-2026-11-20) (`op://`, off) is an optional second path.
- **Automations** are `vault/workflows/*.md` files; every run goes through one
  durable run queue. A silent heartbeat proposes tasks.
- **Welcome**: one lazy page, reopen in About ([details](../WELCOME.md)).
- **Context:** the composer's usage ring shows recorded message shares, and
  its Folder and connectors menu shows two resource lines. Details:
  [Usage](../USAGE_INSTRUMENTS.md).
  Profile and Memory apply across chats; Jev optionally ranks evidence.
  A stale memory or home file is labelled and listed last, one recheck card each
  (`context_stale.rs`, ME2; [module map](backend-module-map.md)).
- Outside text: untrusted.rs

  Project cards, design briefs, first-connect drafts, skill references and memory
  reports: [Context cards and memory](product.md#context-cards-and-memory).
- **Local files stay canonical.** Linked tasks and projects mirror Linear
  ([Tracker sync](../TRACKER_SYNC.md)); legacy `notion_id` keys are history
  (history). Hollow has no
  built-in email, Slack or Teams; an agent may use a messaging service only
  through a connector the user shared. The built-in Microsoft 365 connector is
  read-only calendar and files.

**Settled owner decisions** (do not re-open; each is enforced in code and
described in the section named):

- **Bypass asks nothing:** no Hollow approval or confirm card ever appears in
  `bypassPermissions` (question cards and task suggestions are the agent
  asking, not approvals, and still show). **Exceptions:**
  `app_actions::ALWAYS_ASK` asks on a card in every mode and never runs from an
  automation or a COO check-in: editing a line of the COO's standing orders
  (`workspaces.edit_standing_orders`, G1: card with before and after; raw
  Edit/Write of the file stay denied), the
  gardener's tidy-up (decision 5), settling a decision record (only the person
  does), `friction.draft_fix` (#13), `build.install_built_app` (P0-27, D12),
  `build.publish_main` (U8: on Alex's word),
  `settings.cfo_approve_extra_usage` (11-10) and `personas.accept` (C7). One more, per call: a destructive or access-granting
  `tracker.graphql` mutation asks in every mode, one with an unreadable by-id team is refused (`asks_in_mode`, not `ALWAYS_ASK`; [Linear GraphQL](../TRACKER_SYNC.md#linear-graphql)). Its team guard covers queries and refuses workspace-wide creates (R3); adapter writes pin the org id.
- **Linear writes in AI and HOL ask no card** (Alex, 11-12 5:19 PM; T1, 2026-11-15): `app_actions::TRACKER_WRITES` are Write tier, automations included; other teams stay refused by the R3 guard and `linear::team_for` (tracked work AI, Hollow-only HOL). [Team routing](../TRACKER_SYNC.md#team-routing-and-labels-t1-2026-11-15).
  Keep `mode_contract`'s bypass wording and its fixture in step.
- **Real work gets an issue** (T3a, 2026-11-15): `tracker/chat_work.rs` classes a person's chat each turn. `tracker.trackAgentWork` is `off`, `dryRun` (default: audit only) or `on` (one issue per chat via X1: AI in a tracked project, else HOL with `agent`). [Details](../TRACKER_SYNC.md#chats-that-did-real-work-t3a-2026-11-15). T3g: [build-log comments](../TRACKER_SYNC.md#build-log-comments-t3g-2026-11-15); DR1b: [RECORDS](../RECORDS.md).
- **Waiting on you and commitment copies** (T3b, T3e, 2026-11-15): `tracker/waiting_work.rs`, `commitment_work.rs` copy items waiting 10+ min and his open commitments to Linear issues, his, `needs-review` or `commitment` (AI in a tracked project, generic title; else HOL), closed when done. Details in [TRACKER_SYNC](../TRACKER_SYNC.md) (T3b, T3e, L2).
  **Auto asks only** before leaving this Mac, deleting, touching secrets or
  accounts, or giving agents more power (Permission modes across agents).
- **Decision 1:** services are bounded in every mode but bypass.
- **Delegates are bounded by their parent:** allowed tools intersect, denied
  tools accumulate, a deny wins; never widened (Permission modes → Teammates).
- **Decision 3:** only a person accepts a Result or resolves review items,
  commitment and learning proposals and the profile builder. Agents may only
  propose or mark ready.
- **Decision 5:** History keeps at most 200 runs, 90 days and 512 MiB, and
  captures files up to 2 MiB; a file it can't capture shows as not
  recoverable, and a run with one never claims to be fully recoverable.
  After the first snapshot in a process, a full-vault snapshot rereads only
  paths file events named; a dropped event walks the vault.
- **Decision 6:** at a hard budget ceiling work may continue on another
  account or agent with room; concurrent runs share one reservation; the
  switch shows in the chat and on the receipt; a ceiling never stops work
  silently.
- **Decision 7:** Jev, Grok and Antigravity stay (Antigravity's own `auto`);
  one Waiting on you surface; outputs are Results; a hidden page goes only
  once confirmed unused (the 2026-10-03 audit found Conversation history and
  From other apps both live; run 15 P4 removed From other apps once 0 of
  1,104 discovered conversations had ever been added).

**Conversation history** is Library → Conversations since S4
([contract](surfaces.md#library--conversations-s4-2026-11-03));
page id `chat` opens it. Run 15 P4 removed From other apps (its page, rail
group, Settings section, Add's conversation-export kind and the background
scan of other apps' folders) and Conversation history's Claude history tab;
an old `imports` id opens Projects. `session-imports.json` and the session database's `external_*`
tables stay on disk, unread. Goals, commitments and people are vault data
without a page of their own; learnings are Library → Learnings.

Historical programmes are kept in 3.1
and 5.0, with the
program ledger (which also carries the
Paperclip `PCLIP-*` rows). They are not the current implementation contract:
do not restore their dashboards, rooms, rituals, graph or CRM to satisfy a
historical checklist. The approved product reset
and [current source contract](../SOURCE_CONTEXT.md) take precedence.

macOS-first (tray icon, global hotkeys, dock). The same Rust + WebView builds
for Linux. README.md has the prose tour aimed at humans; this file is the
operator's manual aimed at you.

Archive: status blocks, programmes and surface notes.
