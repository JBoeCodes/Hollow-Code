# Tracker sync

Hollow's projects and tasks live in a task tracker (Linear, workspace Example
Org, team Engineering only, decided 2026-11-07); the vault mirrors them
and holds everything else. This doc is the contract for the tracker code in
`src-tauri/src/tracker/`, its sync engine and the IPC the Settings panel and
the Tasks page read. Notion two-way sync was removed on 2026-11-10; Notion is a
read-only archive.

**Where it stands.** Overhaul A2 (overnight 2026-11-08) landed the
provider-neutral types, the adapter trait, the config block, the key storage,
the IPC shapes and command stubs, and the test stub server. A3 (the same
night) landed the Linear adapter over GraphQL (below). A5 adds the sync
engine, mirror renderer, cached reads, durable user edits and issue create
journal. A6 adds the task/project/phone seams, scoped actions and tracker
guidance. Workspace identity (A7) follows.
The engine uses the immutable running config; a saved change applies after a
restart.

## Two teams: Engineering and Personal (2026-11-12)

Hollow touches exactly two Linear teams: the **shared team** Engineering (key
`ENG`, id `11111111-…`, real project work, config `teamId`) and Alex's **personal
team** Personal (key `PER`, id `22222222-2222-4222-8222-222222222222`, items only he
and Hollow use, opt-in config `personalTeamId` / `personalTeamKey` in the `linear`
block; Settings → Task tracker, "Your own team", Add Personal). LP, PRO and every
other team stay refused, by key and by id, in every guard:

- **Adapter** (`linear.rs`): `team_allowed`/`allowed_team` are the one allow-list.
  `LinearAdapter` is still scoped to one team; `for_team` returns a scoped copy for
  the other. A create naming a team that isn't the adapter's own is `OutOfScope`
  before any request. `settings::ProviderInfo::allows` guards config and ledger.
- **Sync** (`tracker_sync.rs::passes`): one pass per team, shared first, each through
  its own adapter, with the ledger focused on that team (`Ledger::swap_to`: `scope`,
  cursors and metadata for the focused team in the primary slots, the other's in
  `others`). Records, outbox and journals are shared and carry their issue's team;
  `Ledger::owns` keeps a pass to its own team. A ledger without `others` is today's.
- **Mirror:** task and project files carry `tracker_team: ENG|PER`.
- **Writes:** `tracker.create_task`/`create_project` and `TrackerNewTask`/
  `TrackerNewProject` take `teamKey` (ENG by default; `resolve_team`, routed by
  `team_for` since T1, below). Edits,
  comments, archive and history pick the adapter by the item's team. Agent work
  ops keep the issue's team and each team's pass sends its own.
- **`tracker.graphql`:** a mutation may name ENG or PER (key or id), nothing else; by-id
  targets must be in either team, and one whose team can't be read is refused, card or
  not.
- **IPC:** `tracker_status` has `personalTeamId/Key`; `tracker_view` takes `teamKey`
  and returns `teamKeys`, rows and projects carry `teamKey`. The Tasks screen shows
  a team filter and key chips only when two teams exist.
- Which agent writes go to PER: [Team routing and labels](#team-routing-and-labels-t1-2026-11-15).

## Team routing and labels (T1, 2026-11-15)

Alex, 2026-11-12 5:19 PM: agents may do anything in Linear without asking
(create, edit, comment, status, assign, close, archive), in teams AI and HOL only.

- **Routing** (`linear::team_for(work, named)`, pure): tracked work goes to AI,
  Hollow-only work (only Alex and Hollow: Hollow's own build items, agent
  bookkeeping, his commitments) to HOL. A named team wins when it is AI or HOL (key,
  any case, or id); LP, PRO and any other team are refused ("Hollow works only with
  ...", nothing sent). `sync_api::resolve_routed` applies it on Linear; a task create
  with no `teamKey` and a HOL-only label (`commitment`, `agent`, `hollow-lesson`,
  `Work::of_labels`) goes to HOL, and is refused, not filed in AI, when HOL isn't set up.
- **Tiers:** `app_actions::TRACKER_WRITES` (`tracker.create_task`, `create_project`,
  `comment_issue`, `attach_canvas`, `archive_task`, `graphql`, `ensure_labels`) are
  Write with no card in every mode, and automations may run them (D6, 11-12 8:57 PM).
  `tracker.archive_task` left `ALWAYS_ASK`. `tracker.graphql` keeps its card only for
  a destructive or access-granting mutation (item 6 below; an automation can't run
  those). Edits, status and assignee changes go through `tasks.update_task` (the
  mirror's outbox) or `tracker.graphql`. Unchanged: the R3 team guard, the org pin,
  the adapters' team reads, COO check-ins (`tracker.*` writes are not on their safe
  list) and Ask Hollow, which never offers them (`app_actions::leaves_this_mac`).
- **Labels** (`tracker.ensure_labels`, `sync_posts::ensure_labels`): per configured
  team, `linear::labels_for` (D8): AI `needs-review`, `commitment` (T3e: a
  team commitment lands in AI); HOL `needs-review`, `commitment`, `agent`,
  `hollow-lesson`. A create naming no team still goes to HOL when it carries
  `commitment`, `agent` or `hollow-lesson` (`HOLLOW_ONLY_LABELS`). Reads the team's labels (`labels()`, a
  workspace label of the same name counts) and creates only the missing ones with
  `issueLabelCreate` and the team's id (`TrackerAdapter::create_label`), under the
  sync root lock; a second run creates nothing. One `tracker_write` audit row
  (action `label`) per create. `overnight` and `lane:<id>` are not built (S36).

## Company activity (removed 11-10)

The company's activity journal (`companyActivity` in the ledger) went with the
company code (COO rebuild L11). A ledger that holds one still loads: the field
is kept verbatim (`Ledger::company_activity`, an opaque JSON value), never
read, sent or counted as pending, and it no longer blocks a tracker rekey.

The adapter's complete scoped `issue_comments` reads and
`create_issue_comment` with a final host authorization callback stay; the agent
work outbox below uses them. Stub tests do
not establish live API acceptance; that verification remains separate.

## Tracking agent work

COO rebuild X1, 2026-11-10; the decisions are the X0 note in the Hollow vault
workspace (`outputs/coo-rebuild-2026-11-10/linear-tracking-audit.md`). Code:
`tracker/work.rs` (links, resolution, facts, comment bodies, reads) and
`tracker/work_outbox.rs` (the ledger outbox and its pass), both
provider-neutral and lifted from the company activity journal without its
roles.

**Which issue.** At a subagent's start (`start_session`, a worker batch;
`app_sessions::queue_turn` records it once the session key exists) the host
resolves, in order: `start_session`'s `issue` argument (a key or id; it must
be in team ENG's mirror and the workspace tracked or `quick-chats`, else the
start is refused and nothing is sent), the parent's link (`inherited`; a
helper at depth 2 stops here), a packet source task carrying `tracker_id`,
the single best open issue of the workspace's tracker project by token-set
Jaccard of at least 0.6 (a tie links nothing; closed issues never match),
else a create in that project. A workspace is tracked when its PROJECT.md or
WORKSPACE.md names `tracker_project` for the configured provider. Untracked
workspaces and the vault root never link; automations never create. Reads
come from the ledger's snapshots, never the network.

**Stores.** Sessions DB `work_links(session_key PK, issue_id, issue_key,
workspace_id, linked_by, created_issue, at, depth, tracked, outcome_at,
title)` (`title` since T3g, the run's title at its start),
`linked_by` one of `packet`, `inherited`, `task`, `matched`, `created`,
`person` (reserved for lane 3); `issue_id` stays null while a create is
pending; `tracked` says the switch was on at the start; a handover's
successor gets a copy of its predecessor's row. `work_events` holds the host
facts the tracker gets (`create`, `started`, `progress`, `outcome`), queued
only while the running config has `tracker.trackAgentWork` on
(`work::register_running` at setup); taken facts are kept 30 days. The
ledger's `work` journal (omitted while empty; an older binary refuses a
nonempty one, as with `companyActivity`) holds the writes.

**Writes** (only with `trackAgentWork` on; a dry run, the default since T3a, and off send nothing; applied after a restart;
team ENG by id only; never Done, except T3b's and T3e's close of a copy, below). The pass, after the person's own edits,
takes up to 32 ready facts (a create, or a fact whose run's issue is known;
progress notes are read apart, so nothing waiting holds the window) and makes
up to 4 sends. It never fails the pass: problems go in the report, and a
rate, auth or network failure only stops tracking for that pass. While the
switch is off it sends nothing, and `work_outbox::cancel_if_off`, run by
`sync_with` before any remote call and saved at once, settles waiting writes
as Skipped and takes every untaken fact, so an offline pass cancels too. Start: a move to the team's first
`started` state when the issue is backlog or unstarted (once per issue while
in flight), then a Started comment (title, who asked, agent, model, effort,
session key). Outcome: a comment, a build log since T3g (below) (status, Done/Partial/Blocked, PR numbers
without `#`, `outputs/` result paths and checks from the receipt, working
time, tokens, cost summed with inherited helpers); on success a move to the
next `started` state (In Review) only when Hollow's move is the issue's last
change (state matches and the mirror's `updated_at` is no later than
Hollow's last confirmed write, 5 s grace). Every queued move is rechecked
against the freshly pulled mirror right before it is sent, and Skipped when
a person moved the issue (a move to started also when the state differs from
the one seen at queueing, so Backlog to Todo counts). Failed or stopped runs comment only, with their
host-known reason. Only a run's first turn end posts the Outcome; later
turns become progress notes. `tracker.report_progress` (one line, optional
link) posts at most one progress comment per issue every 15 minutes; notes
in between fold into it, and a run's notes within a minute fold into one
fact. Helpers (depth 2) never post; a depth-1 run that inherited its
parent's issue (an orchestrator's lane) does. Creates reuse the ledger
`creates` journal under a key from workspace, project, parent and title, so
a retried start makes one issue (lost replies resolve through the pass's
create recovery, by the create's own issue id); issues just created count
for later title matches. The
project is checked live, mirrored and in team ENG before every create; a
refused create, or one failed for 10 minutes, unties its run and takes its
facts. The Outcome's cost reads "at least $x" when some usage rows had no
price.

**Safety.** Every comment is fixed host copy, run through
`claude_runner::redact`, at most 2000 bytes, and embeds `Hollow record
work-<32 hex>`. The attempt is saved before the call; a lost reply or a crash
after the save is confirmed by finding exactly one comment with that id and
body, and is never resent. An uncertain move is confirmed by reading the issue
back and skipped when it moved elsewhere. A write proven not sent settles as
Skipped (with its reason) when the issue left the team or is gone, and waits
5 minutes for a passing cause. An uncertain write is rechecked at most 3
times, then Skipped; a full journal (2000) evicts settled entries, and
settled bodies keep only their record id after a day. Caps: 30 tracking comments an hour
across Hollow, 4 an issue (the rest wait); a low rate budget pauses tracking
writes first.

**Reads.** `tracker_issue_work(issueId)` (agent action
`tracker.work_for_issue`) returns `{runs: WorkRun[], audit: AuditEntry[]}` with
`WorkRun = {sessionKey, title, agent, model, effort, status, startedAt,
endedAt, workingMs, tokens, costUsd, linkedBy}` (totals as the Subagents
tab's, `subagents::row_of`). `session_subagents` rows carry `issue: {id, key,
title, stateName, stateKind} | null` and `trackerIssue` (its key).
`tracker_status.trackAgentWork` and `set_tracker_settings` `{trackAgentWork}`
are the Settings switch.

### Chats that did real work (T3a, 2026-11-15)

Decision c5 (`workspaces/main/outputs/hollow-next-build-decisions-2026-11-14.md`):
real work gets an issue, quick questions and checks don't. Code:
`tracker/chat_work.rs`, called from `chat.rs` after each turn's receipt and
X1's own `work::after_turn`.

- **Which chats.** The person's own chat only (top level, attended, not a bare
  persona or bake-off run: `person_chat` on the turn), never the COO's or a
  project lead's conversation (their subagents are X1's), never a stopped turn.
- **Classifier** (`classify`, pure, host facts only, never the request's words):
  build when the turn itself opened or merged a pull request (the receipt's
  `pullRequests`, from its own `gh pr` calls), pushed (`git push` or a GitHub
  pull request in its outside effects, done or uncertain), or changed files or
  made commits in a repository (`repo_changes`); a PR number in the reply is
  never evidence, so "is PR 482 merged?" files nothing. Deliverable when the
  turn itself wrote a file in its own workspace's `outputs/` (`workspaces/<id>/outputs/`,
  not a nested `outputs/` or another workspace's; none for a chat in no
  workspace); research when the turn ran at least 10
  minutes and answered in at least 1500 characters; anything else is a quick
  question or a check and makes nothing. A chat is decided at the first turn
  that qualifies. A turn that ended in an error is filed only when its own
  facts show real work (a PR it opened before failing).
- **The setting** `trackAgentWork` (config.json `tracker` block; settings key
  `tracker.trackAgentWork`, section Sources, Confirm): `off`; `dryRun` (the
  default when unset): one `tracker_write` audit row, action `create`, outcome
  `skipped`, detail "Dry run: Hollow would file this <kind> chat as an issue in
  team <AI|HOL>; nothing was sent.", nothing queued; `on`: the X1 path below.
  The older `true` and `false` read as on and off; anything else is off with a
  problem line. Saved through the boolean (`set_tracker_settings`,
  `tracker.set_settings`, `tracker.configure`), `true` turns it on and
  `false` turns on into off but leaves a dry run alone, so a form that saves
  the status's `trackAgentWork: false` unchanged keeps the dry run; only the
  settings key sets all three. `track_agent_work` (X1's switch) is true only when on. Needs the
  tracker set up for team ENG; applies after a restart (`chat_work::register_running`).
- **Where** (T1's `linear::team_for`): a workspace whose PROJECT.md or brief
  names its `tracker_project` is tracked work, filed in team ENG in that
  project; any other chat (quick-chats, an untracked workspace, the vault root)
  is Hollow-only work, filed in team PER with no project and its `agent` label
  (`work_outbox::hol_input`, from HOL's label metadata; filed without it when
  the label is missing, `tracker.ensure_labels` makes it). With HOL not set up
  the chat gets no issue and one `tracker_write_failed` row says why; it is
  never filed in AI instead. The R3 guard and the adapters still refuse LP,
  PRO and every other team.
- **Once per chat.** The create key is `work-` + 32 hex of SHA-256 of the
  key of the chat's first conversation (`chat_work::create_key`; a handover's
  successor has a new session key, so the chain is walked with
  `chat_chain::predecessors`): the audit row's `request_key` and the create
  journal's key. `work_links` (keyed by the chat) gets `linked_by` `chat`; a
  chat is never decided again once it, or any conversation it was handed over
  from, has a link or an audit row under that key, and a successor with no
  link of its own takes its predecessor's (`work::continue_link`), so its
  turns report on the same issue. `work::begin_as` writes the link and its
  facts in one transaction (a link that already exists queues nothing); a
  chat link with no issue and no fact ever queued is dropped and decided
  again. A create the outbox gives up on (`Created::Failed`) leaves a
  `tracker_write_failed` row under the create key before the link is
  dropped, so later turns don't retry it.
- **On.** `work::begin_as` records the link and queues create (with
  `teamId` HOL's id for a Hollow-only chat; none means team ENG, as before),
  Started, then the turn's Outcome. The outbox sends them under X1's rules:
  each team's pass creates its own (`WorkEvent::Create.team_id`); Started moves
  a backlog or unstarted issue to started (In Progress); the Outcome comment,
  then In Review only when Hollow made the last move; never Done. Later turns
  of the chat are progress notes. The issue never carries the person's
  request: its title is the chat's own title (redacted, at most 200
  characters) and its description host facts only ("A build chat in
  workspace proj (Hollow chat session s-...); its first real-work turn ended
  with status success.", then Hollow's notes).
- **Main's chats table.** The COO prompt now says "Don't list started chats in
  PLAN.md: list_sessions mine has them." and the wake message no longer says
  "log it in PLAN.md" (`V5B_WAKE_MESSAGE` keeps stored wakes readable). Old
  rows in Main's PLAN.md stay and are still read.
- **Rollback.** Set it back with `settings.change` `tracker.trackAgentWork`
  `dryRun` (records, sends nothing) or `off` (records nothing), or write
  `"trackAgentWork": "off"` in config.json's `tracker` block, then restart
  Hollow. Issues already filed stay in Linear and keep their links; the next
  pass settles every waiting write as Skipped and takes every untaken fact
  (`work_outbox::cancel_if_off`), so nothing is back-posted when it is turned
  on again. Dry-run rows stay in the audit.

### Waiting on you copies (T3b, 2026-11-15)

Decision c5: Waiting on you items become Linear issues assigned to Alex,
closed on their own; Hollow stays where he reads and answers, the issue is a
copy. Code: `tracker/waiting_work.rs`, run once per sync (`sync_with`, after
`cancel_if_off`, local only), and the X1 outbox.

- **Which items.** From `needs_you::waiting_now`: questions, ask-canvases,
  review items, results to review and sync conflicts (`MIRRORED_KINDS`) that
  appeared after the current mode began and have waited 10 minutes, at most 8
  new a pass. The mode's start is kept in the sessions store
  (`derived_state`, key `s-hollow-waiting-copies`, kind `t3b-since`) and
  resets whenever the setting changes (off to dry run, dry run to on, on to
  off and back), so a backlog is never filed, nor what waited while it was
  off. A pass that reads off only because the tracker's provider or team is
  missing for a moment leaves it alone. When the store can't be read the last
  start this process knew is used; with none, nothing is filed and one
  `tracker_write` row (`waiting-mode-start-unknown`) says so; a start that
  can't be saved is kept in memory, so the clock never restarts every pass.
  A changed result's own time is its file's modified time, so it gets its
  first-seen time instead (`derived_state` kind `t3b-results-seen`; changed
  results already waiting when that store is first kept count as backlog).
  An item with no time can't show it appeared after, so it is never copied.
  Approval cards (on screen for minutes), failure notifications, the restart
  offer, conflict copies and lesson memory offers (L2, below) stay in Hollow.
- **Where** (T1's `team_for`, through `chat_work::destination`): an item whose
  workspace names its `tracker_project` involves the team: team ENG, in that
  project. Any other (a learning, a person alias, a question in quick chats or
  an untracked workspace) is HOL, no project.
- **The issue.** Team AI is shared, so its title is generic ("A question
  waiting on you", "A sync conflict waiting on you") and nothing of the item's
  words reaches it. HOL, his own team, gets the item's title, redacted, at most
  200 characters. Description: host facts only (kind, workspace, chat session
  key). Label `needs-review` (`WorkEvent::Create.labels`; when the team's
  metadata lacks it, the pass makes it with T1's `create_label`, one `label`
  audit row). Assignee: the signed-in user, the ledger metadata's `viewer`
  (the adapter's `whoami`), never a fixed id; while unknown the issue is
  created unassigned and one `tracker_write` row (`<create key>-unassigned`)
  says so. No Started move or comment: a copy is not a run.
- **Once per item.** Link `work_links` key `waiting:<item id>`, `linked_by`
  `waiting`, with its create fact in one transaction; create key `work-` + 32
  hex of SHA-256 of the item id. An item is decided by its link, a dry run's
  row under its create key, or a failed create there within the last hour, so
  a retry, a restart or a re-render never files a second one. An item that
  comes back after its copy closed gets none.
- **Failures are retried.** A refusal (HOL not set up) is audited once under
  `<create key>-refused` and decides nothing: the item is filed once HOL is.
  A create the pass's recovery read back absent (never reached Linear) is sent
  again with its same issue id (`work_outbox::create`, for `waiting:` keys
  only), so Linear can't make two, at most 3 more times
  (`WAITING_RESENDS`, counted in the create journal's `resends`); then it is
  given up for good, with one `tracker_write_failed` row under
  `<create key>-gave-up`, and the item is never filed again. A create the
  outbox refused (project gone) is tried again after an hour.
- **Closing.** A linked item whose issue exists and that is no longer in the
  list, on a pass where every source was read (`unavailable` empty, app data
  found). A question or review item closes on its store's own status (a
  snoozed or still-applying review item waits); a result, conflict or canvas,
  or a question or review item whose row is gone, only when absent on two
  complete passes at least 60 seconds apart with nothing between (kept in
  memory, so a restart starts the count again; an incomplete pass, a pass
  that isn't on and a mode change also start it again). When the issue's team has no completed state in the ledger, the
  close is not queued: one `tracker_write` row (`<create key>-no-completed-state`)
  says so and the next pass tries again; the outbox also leaves a close fact
  untaken, audited once, while its team has none. `work::finish_link` sets
  `outcome_at` with one `WorkEvent::Resolved`. The answer is host copy,
  redacted: "Answered in Hollow." (HOL may add the chosen option's own label,
  yes or no, a count of verdicts; words the person typed are never quoted,
  only "a written answer (kept in Hollow)"; team ENG never gets option labels),
  "Dismissed in Hollow.", "Accepted in Hollow.", "Rejected in Hollow.",
  "Reviewed in Hollow.", "Resolved in Hollow." The outbox posts that comment,
  then moves the issue to the team's first completed state (`OpKind::MoveDone`),
  only when the link has `linked_by` `waiting` and `created_issue` (never an
  issue Hollow didn't create for the item) and only while the issue is open:
  a person's close or cancel wins (rechecked against a fresh read right before
  the move).
- **The setting.** `tracker.trackAgentWork`, as T3a: `off` nothing; `dryRun`
  one `tracker_write` row per item, "Dry run: Hollow would file this waiting
  <kind> as an issue in team <AI|HOL>, assigned to you with needs-review;
  nothing was sent.", nothing queued and nothing closed; `on` the above.
- **Rollback.** `settings.change` `tracker.trackAgentWork` `dryRun` or `off`,
  then restart: queued creates and closes settle as Skipped and nothing more is
  filed or closed. Issues already filed stay open in Linear; close them there
  (filter by label `needs-review`).
- **Going back to an older build.** From this build on, the `work` journal
  keeps a write kind it doesn't know as `Unknown` (loaded, never sent) and a
  fact kind it doesn't know is taken unsent (`#[serde(other)]` on `OpKind` and
  `WorkEvent`), so a later build's writes never stop this one. Those writes
  are lost, though: an unknown write is saved back as `unknown` and an
  unknown fact is marked taken, so going back to the newer build afterwards
  never sends its writes that were still pending. A build before
  T3b has neither: it refuses the whole ledger when the journal holds a
  `moveDone` write (settled ones stay in the journal), files a pending copy
  create without its label and assignee, and leaves `resolved` facts unsent,
  where they hold its 32-fact intake window. Before installing such a build,
  with Hollow quit: drop the `moveDone` entries from the vault's
  `.tracker-sync.json`
  (`jq '.work.entries |= with_entries(select(.value.kind != "moveDone"))'`,
  written to a copy, then moved over the original), and mark the copies' facts
  taken in the sessions store (`session_store::db_path_for`):
  `UPDATE work_events SET journaled = 1 WHERE session_key LIKE 'waiting:%';`.

### Lesson memory offers (L2, 2026-11-15)

A Hollow lesson (team PER, label `hollow-lesson`, Agent notes naming
"Workaround memory: `<file>.md`") looks fixed in the installed build when the
mirrored issue is completed and its `completedAt` is at least 15 minutes
before the installed app's `generatedAt` (`/Applications/Hollow.app`
provenance, else the running app's), a margin for the merge after closing.
An issue marked Done before its fix merged can still be offered early, so the
offer says "the fix looks installed" and Delete is reversible. Code:
`lesson_memories.rs`. Issues are read only from the local ledger
(`tracker_sync::mirrored_issues`), so no Linear call and no path around R3;
with no HOL pass mirrored, nothing is offered.

- **Which file.** Issue text alone never picks one. The named file must be
  the issue's pair in the L1 lesson map (16 pairs, `KNOWN`), or its body or
  its `MEMORY.md` line must name the issue key (whole key: PER-1 is not
  PER-13). It must be a plain file in a workspace's Claude memory folder
  (`<config>/projects/<slug>/memory/`; the folder an issue names is ignored,
  the vault moved), never `MEMORY.md`, never through a link.
- **The offer.** Waiting on you kind `lesson_memory`, "Delete the memory
  <name>?", `category` the issue key. PER-1, PER-13, PER-12, PER-9 first,
  then by number; four at most at once. Hollow-local: not in
  `MIRRORED_KINDS`, so T3b never copies it, and Main's check-in skips it.
- **Answers** (`answer_lesson_memory`, `restore_lesson_memory`, user-only, no
  agent action). Delete, in order: a copy to the vault's `.trash/` (hash
  recorded), the answer saved with everything Undo needs, the one
  `MEMORY.md` line out (index links `](name)`, `](./name)`, `](<name>)`,
  with or without a title; two lines naming it refuse), then the file, then
  a History row "Lesson memory deleted". The index is re-read right before
  its write: changed since read means nothing changes. A failing step undoes
  the earlier ones (answer and copy). Undo (the snackbar) restores the file
  (never over one there now) and its line at its old place while the index
  is as the delete left it, else at the end; same re-read rule, and a failed
  save takes both back out. Keep records the choice. Either way the memory
  is not offered again. Answers live in app support,
  `LessonMemories/<vault stem>.json`.

### Commitment copies (T3e, 2026-11-15)

Decision c5, on T3b's machinery: a commitment he owes becomes a Linear issue
assigned to him with its due date, closed when it is done. Code:
`tracker/commitment_work.rs`, run once per sync right after T3b's reconcile;
filing, the close gate and the outbox are T3b's (`waiting_work::file_copy`,
`settle`, `clock`), so everything above holds unless said here.

- **Which.** A commitment in `commitments/*.md` that is open, `owed_by_me`,
  not snoozed today, and whose `created` is after the current mode began (T3b's
  clock, one for both), at most 8 new a pass. Owed to him, closed and older
  ones never are; a snoozed one is filed once it wakes.
- **Where.** Its `source_path` under `workspaces/<id>/` names the workspace;
  one whose workspace names its `tracker_project` is team ENG in that project,
  titled "A commitment you made"; any other is HOL, its own title redacted.
  Description: host facts only (no counterparty, context or evidence). Label
  `commitment`, assignee the viewer (unassigned plus one audit row while
  unknown), `dueDate` from the commitment's `due` (`WorkEvent::Create.due`,
  `NewIssue.due`), none when it has none or it isn't a date. A due date edited
  later is not carried over.
- **Once.** Link `work_links` key `commitment:<id>`, `linked_by`
  `commitment`, id 32 hex of SHA-256 of the note path (stable: status edits
  rewrite in place and every path has a random suffix); create key `work-` +
  32 hex. Decided, refused, retried and resent exactly as T3b (the resend
  covers `commitment:` keys, `waiting_work::is_copy_key`). Reopened after its
  copy closed: no second issue.
- **Closing.** The stored status first: `fulfilled` ("Marked done in
  Hollow.") or `cancelled` ("Dropped in Hollow.") closes on that pass. A note
  gone from a whole read of the folder ("Removed from Hollow's commitments.")
  only on two complete passes at least 60 s apart; an unreadable folder starts
  the count again, and a note that no longer parses is present, never gone.
  A missing `commitments/` folder is an empty ledger only while the vault root
  exists and no commitment copy is open; otherwise the read is incomplete, so
  a vanished folder never closes every copy. A commitment edited away from
  `owed_by_me` (or its workspace changed) after filing keeps its issue open
  until it is marked done, dropped or deleted; the issue is not moved or
  closed for it.
  The comment is headed "Hollow: commitment closed"; the outbox's
  `MoveDone` takes `linked_by` `waiting` or `commitment` (`linked_by::is_copy`)
  with `created_issue`, only while the issue is open.
- **The setting.** `tracker.trackAgentWork` as T3a and T3b: dry run writes
  "Dry run: Hollow would file this commitment as an issue in team <AI|HOL>,
  assigned to you with commitment, due <date>; nothing was sent."
- **Rollback.** `settings.change` `tracker.trackAgentWork` `dryRun` or `off`,
  then restart; queued creates and closes settle as Skipped. Filed issues stay
  in Linear (filter by label `commitment`). A build before T3e loads a pending
  create's `due` and drops it, and closes no commitment copy (its `MoveDone`
  check reads `waiting` only), so with Hollow quit mark those facts taken:
  `UPDATE work_events SET journaled = 1 WHERE session_key LIKE 'commitment:%';`.

### Build log comments (T3g, 2026-11-15)

Every agent and subagent working on an issue comments on it as it goes:
what it did, why, how it checked. Code: `work::render_outcome`,
`work::render_progress`, `work::LogHead` and `work::after_turn` in
`tracker/work.rs`; the outbox's intake (`tracker/work_outbox.rs`) fills the
head.

- **Issue only, never a project.** Linear's MCP `save_comment` takes a
  `projectId` parent, but a project comment is the same surface as a project
  update's discussion, and decision c5 rules out project updates. So the
  comment goes on the work's issue, and no code writes to a project.
- **Which comments.** The ones that already existed, restyled: a linked run's
  Outcome (its first turn end) and the folded progress comment (later turn
  ends and `tracker.report_progress` notes). A worker chat (T3a link) and each
  of its depth-1 subagents have their own link, so each leaves its own
  comment on the issue; helpers (depth 2) still never post. Started comments
  and T3b/T3e close comments are unchanged.
- **Format.** Line one `Build log, <local date and time>, <run>`: the fact's
  time in the Mac's zone (`2026-11-15 14:03 (UTC-05:00)`), and the run's
  title as captured when it started (`work_links.title`: the start title's
  first line, redacted, at most 80 characters; never the live session title,
  so a rename or a later summary never shows), with `(chat)`, `(subagent)` or
  `(agent)`; a link from before T3g has no title and reads `A subagent`. A
  progress comment folding notes from several runs names up to three. The
  body is rendered once at intake and stored in the outbox entry, so a retry
  sends it byte for byte. Then `What:` (Finished, Stopped or Failed,
  the reply's Done/Partial/Blocked, a failure's host reason, PR numbers,
  files changed in the turn's repositories, `outputs/` results), `Why:` (`work
  on <key>, <issue title>`, the issue's own title, so a team ENG copy's generic
  title stays generic) and `Checked:` (the receipt's checks as `<kind>
  <outcome> (<runner>)`, at most 8, or `no check recorded`). The runner comes
  from a fixed list only (`work::runner_of`): a known tool and its step, such
  as `cargo test`, `npm run gate`, `vitest`, `pytest`, `tsc`; wrappers
  (`hollow-slot`, `npx`, `env`, assignments) are skipped. Arguments, paths,
  file names and URLs never appear; a command with no known runner reads
  `<kind> <outcome>` alone. The
  totals line and the session and record lines follow. A progress comment's
  `What:` lists its notes, newest kept when the 2000-byte cap bites, so
  `Why:` and `Checked:` are never cut.
- **Privacy and secrets.** Host facts only: no request text or reply text
  beyond the start title T3a and Started already use and the reply's
  last-line word and PR numbers. The title is redacted when captured and the
  whole body again before the cap. The stub-Linear test
  (`a_chat_and_its_subagent_each_leave_a_dated_build_log_comment`) masks a
  planted `ghp_` token in a subagent's title, keeps a title renamed after the
  start out, and keeps `pytest tests/test_acme_payroll_dispute.py` and
  `cargo test --manifest-path ~/Hollow/Projects/Client-Foo/Cargo.toml` out
  (they read `pytest` and `cargo test`).
- **No spam.** Still one Outcome per run, later turns as progress notes, a
  run's notes within a minute folded, and at most one progress comment per
  issue every 15 minutes, with the 4-an-issue and 30-an-hour caps. Progress
  notes whose issue isn't in the mirror at intake are never sent and stay
  pending (untaken) for a later pass.
- **Facts.** `WorkEvent::Outcome.filesChanged` and `WorkEvent::Progress.checks`
  are new and default when absent, so facts queued by an older build still
  render (with `no check recorded`).
- **The setting.** `tracker.trackAgentWork` as before, no new setting: `on`
  posts; `off` nothing; `dryRun` queues nothing and writes `tracker_write`
  audit rows (action `comment`, outcome `skipped`) folded as the real path
  folds: the run's first turn end "Dry run: Hollow would post this run's
  build log comment (its outcome) on AI-n; nothing was sent.", later ones "...
  a progress build log comment ..." at most once per issue every 15 minutes.
  Request key `build-log-<receipt id>`, or `build-log-<session>-turn-<turn
  count>` without a receipt, so a repeat adds no row
  (`work::dry_run_build_log`).
- **Decision comments (DR1b, 2026-11-17).** A decision record that names an
  issue (`issue: ENG-12`) leaves a build log comment when recorded or when its
  status changes (`tracker/decision_log.rs`, `WorkEvent::Decision`,
  `work::render_decision`): head `Build log, <time>, Decision <change>`, then
  `What:`, `Why:`, `Rejected:` (omitted when empty) and `Verified:`. It rides
  the newest linked run of that issue (`work::link_for_issue_key`; none: nothing
  is made), so dry run, caps, redaction and the outbox are the ones above:
  `dryRun` writes a `tracker_write` audit row (request key `decision-log-<hash>`)
  and queues nothing. Fields: [RECORDS](RECORDS.md). A build before DR1b drops
  the `decision` fact as unknown.
- **Rollback.** `settings.change` `tracker.trackAgentWork` `dryRun` or `off`,
  then restart; queued comments settle as Skipped. Posted comments stay in
  Linear. A build before T3g reads the new fields' facts and ignores them
  (serde skips unknown fields) and never names `work_links.title` (its
  queries list their columns), so reverting the commit needs no data step.

## Status and canvas copies

Two things that live in Hollow can also go to the tracker as a copy (overhaul
A18 and A19, design section 9: Hollow is where Alex reads; a tracker copy is
optional and never the only copy). Both are team ENG only, journaled in the
sync ledger before any request, take the comment box's locks and leave one
audit row (`status_update`, `attach`).

**Friday status** (`tracker/status.rs`). For each tracked workspace (its
`PROJECT.md` names a `tracker_project` of this provider) Hollow writes the
week's status from the mirror (the ledger's issue snapshots), never the
network and never a model:

- *Done this week*: completed, changed in the last seven days (a completed
  issue's `updatedAt` stands in: the mirror reads `completedAt` since G4, but
  issues mirrored before have none until their next sync). Hours saved use
  `completedAt` only: a run counts as accepted when its issue is Done and was
  completed after the run started (runs.md, AI value ledger).
- *In progress*: started.
- *Blocked or overdue*: open, with a due date before today, an open blocker
  (`blocked by ENG-3`) or a `Blocked` label. Listed once, here.
- *Next up*: unstarted, soonest due first, then priority; five at most.
- *Build log this week*: `## YYYY-MM-DD ...` headings of the workspace's
  `BUILD_LOG.md` (or `docs/BUILD_LOG.md`) dated in the last seven days.
- Health: nothing blocked or overdue is on track; three or more overdue, or
  blocked work with nothing done or started, is off track; else at risk.

Where it goes, in order:

1. `workspaces/<id>/outputs/status/<YYYY-MM-DD>-friday-status.md` per
   project (frontmatter `title`, `summary`, `kind: friday-status`, `week`,
   `project`, `tracker_project`).
2. A whole run also writes the COO's digest,
   `workspaces/main/outputs/status/<YYYY-MM-DD>-friday-status.md`, one line
   and a link per project. Both are Results, so the COO's page lists them.
3. **The Herald's line** is the digest's `title` ("Friday status 2026-11-13:
   4 projects, 9 done this week, 5 in progress, 2 blocked or overdue."): the
   morning paper reads Results by title (`workspaces.list_all_results`), so
   it needs no hook of its own.
4. One notification, "Friday status is ready", opening the digest (a
   one-project run opens its note and writes no digest).
5. With `tracker.postStatusUpdates` on (Settings, Task tracker, Status copy;
   off by default) each project's note body goes to the tracker as a project
   status update with its health (`TrackerAdapter::post_project_update`;
   Linear's `projectUpdateCreate`). Journal `statusPosts`, key
   `status-<project>-<ISO week>`: once a project a week; an unconfirmed one is
   never resent. Refused before any request outside team ENG, for a project
   the mirror doesn't hold, and (in the adapter) for a project shared with
   another team. A failed copy leaves the note and is recorded in the
   journal, the audit record and the run's `copies`.

When: the `friday-status` window job on `friday-pm` (Friday 15:00 local,
run once on wake) while `tracker.fridayStatus` is on (Settings, Task tracker,
Friday status; off by default; both switches apply after a restart, like the
rest of the block); the project detail's **Write status now** (Tasks, a
project; `write_friday_status {workspaceId}`); the agent action
`tracker.friday_status {workspace?}` (Write tier).

**Canvas copies** (`tracker/sync_posts.rs`). A saved canvas's ⋯ → **Attach
to issue…** (Library, Results, Canvases) finds an issue in the mirror
(`tracker_view` search), confirms on Hollow's card, and calls
`attach_canvas_to_issue {workspaceId, canvasId, revision, issueId}` (agent
action `tracker.attach_canvas {canvas: <vault path>, issue}`, Write, no card
since T1). Hollow uploads the revision's fence body (`<id>.canvas.html`, as
`<title>.canvas.html`, `text/html`; the sidecar stays home: it names
conversations) with `TrackerAdapter::upload_file` (Linear: `fileUpload` for a
signed address, then a PUT of the bytes with the address's own headers and
never the key), then `attach_upload` (Linear: `attachmentCreate` on the issue,
title `Canvas: <title> (vN)`). Journal `canvasAttachments`, key from the
workspace, canvas id, revision and issue id: a repeat says the copy is
already there; the upload is kept once it lands, and Linear's
`attachmentCreate` updates the attachment with the same URL rather than
adding one, so an uncertain attach may simply be tried again. Refused before
any request outside team ENG or for an issue the mirror doesn't hold; the
adapter reads the issue's team before it attaches. The canvas stays in
Hollow. It replaced "Add to Notion page", removed with Notion sync (2026-11-10).

## The audit record

COO rebuild X2, 2026-11-10 (the X0 note's section 5, as above), code in
`src-tauri/src/audit.rs`. Hollow's own record of agent runs and
tracker writes, read in Hollow first; Linear's copy is only the Started and
Outcome comments.

**Store.** `audit_log` in the sessions database: `id` (`a` + 12 hex digits of
milliseconds, strictly increasing in a process, + 16 random), `at`, `kind`
(`run_started`, `run_ended`, `tracker_write`, `tracker_write_failed`,
`issue_linked`), `session_key`, `parent_key`, `root_key`, `asked_by` (`person`,
a session key, `automation:<id>`, or `<surface>:<id>` for another run such as
`ask`), `agent`, `model`, `effort`, `account` (`usage_ledger.account_ref`,
never a credential), `workspace_id`, `issue_id`, `issue_key`, `action`
(`create`, `update`, `comment`, `archive`, `link`), `fields` (JSON: `before`,
`after`; a run end's `turns`, `costPartial` and `startRow`), `outcome` (`ok`,
`failed`, `stopped`, `skipped`), `detail`, `cost_usd`, `tokens_in`,
`tokens_out`, `working_ms`, `request_key`, `prev_hash`, `hash`. Indexes on
`(session_key, kind)`, `issue_id`, `issue_key COLLATE NOCASE` and
`(request_key, kind, action, outcome)`. `BEFORE UPDATE` and `BEFORE DELETE`
triggers abort. `hash` is SHA-256 of `prev_hash` plus the row's canonical
JSON (an array of every other column in that order, `fields` as stored).
`audit::append(conn, entry)` first sanitizes the row: free text (`detail`,
every string and key in `fields`) goes through `claude_runner::redact`; text
columns are clipped to 300 characters, `fields` strings to 200, keys to 64,
`request_key` to 200, collections to 32 items, nesting to 4 levels, and the
whole canonical row to 8 KB (`fields` gives way first); non-finite numbers
are stored as none and `-0.0` as `0`. It then reads the last hash, computes,
inserts and moves the one-row `audit_head` (newest id, hash, row count) in the
caller's transaction. A write with a `request_key` is recorded once per kind,
action and outcome (an indexed lookup), so a retry that changes nothing adds
no row. `audit::verify` walks the chain by rowid and names the first `edited`
row (a hash mismatch, or a non-finite number no append stores), `gap`, or
`tail` (the chain ends short of `audit_head`: the newest rows are gone). An
audit write never fails or holds up what it records: `audit::record` logs,
counts (`audit::errors`, reported by `tracker.audit` and `verify`) and
returns.

**Who writes what.** The host only. `work::begin`: `run_started` for every
subagent start and worker (a worker whose link is refused still starts
untied), and `issue_linked` when its issue is known (a create's link when the
outbox confirms it). A child task sent to an existing conversation
(`sessions.send`, also one held behind a busy turn) is a `run_started` asked
by its sender (`work::record_sent`; nothing goes to the tracker).
`work::after_turn`: `run_ended` at every turn end of a subagent
(`sessions.fork_kind = 'subagent'`) or linked run, with its totals so far,
read as the Subagents tab reads them (`subagents::stats`: usage joined through
the turns' run ids, the newest chat turn's model and account; working time
and turn count from `turns`). Its `asked_by` is the oldest start no earlier
turn end took (`startRow`), else the person: a turn typed into a subagent's
chat is the person's. Tracker writes, one row each: the IPC writes and the
agent actions (`sync_api`'s `create_with`, `create_project_with` keyed by the
create journal's own key, `comment_with`, `archive_with`, and a refusal
before them in `create_named`, `create_project`, `comment`, `archive` or
`edit`), a queued edit when the pass sends it (`reconcile` and
`reconcile_project`, fields before and after, keyed by the issue and a hash
of base and pending values, so an edit the tracker keeps refusing is one
failed row; a send answered for an issue that has since left the team is
`ok`, "now outside the team"), and the work outbox's creates, moves and
comments. The edit's asker rides in `Outbox.askedBy`: the newest staged
change's asker wins, and a change captured from the file is the person's. A
refusal before any request (an issue or project outside the team, unknown, a
refused edit through the IPC or the task broker, a start naming an issue
outside team ENG) is `tracker_write_failed` with its reason; an unconfirmed
write is `tracker_write_failed` "Unconfirmed" and a later confirmation adds
its `ok` row. Who asked: an agent action runs its write inside
`audit::asking(caller)`, the task broker inside `audit::asking_sync`;
anything else is the person. Agent-work writes are asked by the run's
session.

**Mirror.** The Outcome comment's working time, tokens and cost are the
newest `run_ended` row of the run and of each helper that inherited its link,
summed (`audit::run_sums`), so Linear's copy equals Hollow's record; the live
totals stand in only when the audit row couldn't be written. Nothing else is
mirrored; the chain stays local.

**Reads.** Agent action `tracker.audit` (read tier): filters `issue` (id or
key), `session`, `workspace`, `since`, `kind`, `limit` (default 50, at most
500), newest first, `verify` to check the chain; `{entries, count,
writeErrors, verify?}`. `tracker_issue_work(issueId).audit` lists the issue's
rows (by id and by key), newest first, at most 200; TS `AuditEntry`
(`src/trackerApi.ts` re-exports it), mock `workAudit`.

## Names

Workspace identity (A7): a new project gets a provider/slug/id8 folder with
WORKSPACE.md, PROJECT.md, generated TRACKER.md and the ordinary context records.
Both project and brief carry `tracker_project`; the brief also stores
`tracker_project_path`. Stored/discovered mirror paths win over new naming.
The ledger saves a project record in `state: initializing` before publishing
workspace files, then marks it active after the mirror and context land. Only
an initializing record may complete a missing first mirror. A missing active
named mirror is preserved as missing, never rebuilt from a template.

Generated context requires `<!-- hollow-tracker-context: 1 -->`; authored
TRACKER.md is refused. Existing unbound migration briefs stay byte-exact, while
PROJECT.md still protects their identity. Their explicit brief binding belongs
to the importer. Guards and project-path readers use frontmatter, not a name
prefix. Legacy Notion folders keep strict UUID recognition and retained
NOTION.md stays in the context envelope beside TRACKER.md.

`tracker` in names, types, IPC, config, vault keys and copy. A provider's name
appears only inside its adapter file (`tracker/linear.rs`) and in its display
name ("Open in Linear"). A second provider needs a new adapter file and one
row in `tracker::settings::PROVIDERS`, nothing else.

## Types and the trait (`tracker/mod.rs`)

- `TrackerIssue`: id, key (`ENG-37`), url, title, description, state
  `{id, name, kind}`, assignee `{id, name, email, displayName?}` (members
  carry the display name; `answers_to` matches a person by name, display
  name, email or the email's local part, which assignee names in a mirror
  edit also resolve by after an exact name), priority 0 to 4 (none,
  urgent, high, medium, low), due (a date), project id, labels, parent id,
  team id, updated, archived, trashed, and the detail fields: estimate, cycle
  `{id, number, name, starts, ends}`, milestone `{id, name, target date}`,
  creator, created, children (keys), relations (`blocks`, `blockedBy`,
  `related`, `duplicateOf` with the other key), links, attachments and
  comments `{id, author, body, created, edited, parent id}`.
- `TrackerProject`: id, key (`P-ENG-6`), url, name, description, status
  `{name, kind}`, lead, start and target dates, team ids, updated, progress,
  links.
- `StateKind`: backlog, unstarted, started, completed, canceled (Linear's
  types; `triage` reads as backlog). Hollow's done and open logic reads the
  kind, never a state's name.
- `Field`: the writable issue fields (title, description, status, priority,
  assignee, labels, project, due, parent, estimate, cycle, milestone) and
  their task-file keys. `READ_ONLY_KEYS` are the tracker-owned keys Hollow
  never pushes (creator, created and updated times, sub-issues, relations,
  links, attachments, comment count): a local edit to one is restored by the
  next pass.
- `TrackerError`: NotConfigured, Auth, OutOfScope, RateLimited `{reset_at}`,
  Conflict `{remote}`, NotFound, Network, Provider. Messages never carry the
  key or issue text.
- `TrackerAdapter`: `provider`, `display_name`, `writable_fields`, `rate`,
  `whoami`, `scope`, `states`, `labels`, `members`, `cycles`, `milestones`,
  `issues_changed(since, after)`, `comments_changed(since, after)` (its own
  cursor, so a comment-only change shows up), `attachments(issue ids)`,
  `projects_changed(since)`, `issue`, `create_issue`, `update_issue`,
  `archive_issue`, `create_project`, `update_project`, `add_link` (append
  only). Calls return boxed futures (`TrackerFuture`).

**Scope guard, in every adapter.** Every query names the configured team.
Any node whose team isn't it is dropped and logged (`tracker: dropped
out-of-scope node <id>`, `keep_in_scope`), never written. A mutation reads its
target's team first and fails with `OutOfScope` unless it matches. With the
team-restricted key, Linear refuses other teams too. Every scope test proves
an out-of-team node is dropped.

**Rate limits.** The adapter reads `X-RateLimit-Requests-Remaining` and
`X-RateLimit-Complexity-Remaining` into `RateBudget`; under 10% of either
left (`RateBudget::is_low`), the next pass waits for the reset. A
`RATELIMITED` error or HTTP 429 backs off 30 s, doubling to 15 minutes.
Every query stays under 10,000 complexity points (nested connections paged
at 50).

## Config (`tracker/settings.rs`)

```json
"tracker": {
  "provider": "linear",
  "pollSeconds": 120,
  "linear": { "teamId": "<team uuid>", "teamKey": "AI", "transport": "graphql" }
}
```

- No block, or no `provider`: the tracker is off.
- The provider's own block names the one team Hollow may touch, and the
  transport (`graphql`, or `mcp`, the fallback over Hollow's `linear` sign-in).
  **There is no default team**: without one the tracker stays off, so nothing
  can read a team nobody chose. **Only team Engineering**
  (`11111111-1111-4111-8111-111111111111`, the lane's decision 2026-11-10, for
  people and agents alike): the provider's `ProviderInfo.team_pin`
  (`linear::TEAM_ID`) makes `set_tracker_settings` refuse another team, the
  status's `missing` say "Hollow works only with team Engineering.", and
  `LinearAdapter::from_config` refuse to build.
- `pollSeconds` is clamped to 60 to 3600 (the setting's 1 to 60 minutes).
- `trackAgentWork`: `off`, `dryRun` (the default when unset) or `on`; the
  older `true`/`false` read as on/off ([Chats that did real work](#chats-that-did-real-work-t3a-2026-11-15)).
- `fridayStatus` and `postStatusUpdates` (A18, both false unless set): the
  Friday 15:00 status job and its tracker copy ([Status and canvas
  copies](#status-and-canvas-copies)). `tracker_status` reports them as
  saved.
- Loading is tolerant: a malformed value is ignored with a line in
  `problems` (shown in Settings), never a config error. Old `notion.*` keys
  still load and are ignored; only the importer reads them.
- Settings writes it with `set_tracker_settings`, which validates first and
  keeps keys it doesn't model; the running app applies it after a restart
  (`TrackerStatus.restartNeeded`).

## Sources of truth (H4a)

Where tasks and projects, knowledge and personal context live (design
`sources-and-import-2026-11-13` section 3 and phases 1 and 1b, in the Hollow
workspace). Code: `sources.rs` (rules), `sources/migration.rs` (owner
switches), `commands/sources_api.rs`, `src/sourcesApi.ts`. The screen is H4b
(below).

```json
"sources": {
  "version": 1,
  "tasks": { "owner": "tracker" },
  "knowledge": { "owner": "vault", "references": [ { "connector": "notion-official", "label": "Notion archive" } ] }
}
```

- **Config.** `tasks.owner` is `vault` or `tracker`; the provider and teams come
  only from the `tracker` block. A missing block (or `tasks`) reads as
  `tracker` when `tracker.provider` is set, else `vault`, so today's config means
  what it meant. Knowledge is always the vault's (N27): `references` are
  connector-library ids agents may read, read-only, never a key (other keys in
  a reference are ignored without echoing them). Personal context has no
  setting. Loading is tolerant: a bad value is ignored with a line in
  `problems`.
- **Workspace override.** `tasks_source: vault|tracker` in `PROJECT.md` (else
  `WORKSPACE.md`); no key inherits. It can never widen scope: a `tracker`
  naming another provider, or a `tracker_team` that isn't one of the block's
  teams (key or id) or that `ProviderInfo::allows` refuses, is a problem line
  and the workspace inherits Settings. With no tracker block, a tracker
  override inherits too.
- **One rule.** `sources::resolve` gives a `Binding` (role, owner, inherited,
  chosen, provider and team, reachability). `owner_of(binding, kind, key)` is
  the tracker for the provider's `writable_fields`, `READ_ONLY_KEYS`,
  `IDENTITY_KEYS` and the keys the mirror writes, the vault for every other
  key; `sends` is the writable part. Tests prove it agrees with the engine's
  `owned_field`/`local_field` and that a pass only ever changes tracker-owned
  keys.
- **Labels** (`sources::label`, sent as data): "Tracked in Linear" (with Open
  in Linear), "Stored in Hollow" (plus "Was tracked in Linear" and its link
  after a switch), "Read-only from Notion", and "Linear unreachable since
  9:40 AM" when the key isn't saved or the last pass failed (the time is the
  last successful read). Lost access keeps the mirror and the tracker's
  ownership; a mirror under a removed tracker block reads as unreachable,
  never "Stored in Hollow".
- **Generations.** The ledger has `generation` (an owner switch writes a new
  one) and each outbox entry the generation it was queued under; both are
  omitted while 0, so older ledgers load and save byte for byte. Before
  capture, `hold_stale` moves entries from another generation to `held`:
  capture and both reconcile steps skip held records (their error says the
  edit is held, the pass reports it) and nothing of theirs is sent. Rolling
  the switch back releases it: the pass returns a held entry of the current
  generation to the outbox. `storedInHollow` (a switch of every
  workspace to Hollow) stops the pass right after the ledger loads: nothing is
  captured, read or sent, agent work's waiting writes settle as Skipped (as
  with tracking off), and the editors refuse tracker edits.
- **Switches** (`sources::migration`, the `tracker-import` pattern). `plan` is
  the dry run (queued, held and conflict counts, mirrored tasks and projects
  kept, files with why, config change, generation, or why refused). `apply`
  takes the sync mutex and flock, journals vault files through
  `tracker_import::files` (backup, `journal.jsonl`, `manifest.json`) under the
  mirror lease, then writes the ledger (new generation, stored flag) and
  config.json's block, recording hashes in `record.json`. Records and the
  ledger and config backups (0600) live in `sources-migrations/<id>/` beside
  config.json, never in the vault. Refused while any queued or held edit,
  unconfirmed create or conflict is open (a rollback ignores held edits, since
  it is what releases them). An interrupted apply resumes with
  the same switch; another waits. `rollback` restores config.json, the ledger
  and every file byte for byte (newest run first; a file changed since is
  left and named; config.json changed since gets only its `sources` block
  back). No switch calls the tracker: nothing there is archived, deleted or
  edited.
  - Every workspace to Hollow: mirrors kept, each tracked `PROJECT.md` gets
    `Was tracked in Linear: [P-ENG-6](url) <!-- hollow:was-tracked -->`,
    `tasks_source: tracker` overrides are removed, `storedInHollow` set (a new
    ledger is written when none exists). Back to the tracker removes the lines
    and clears the flag; its first pass changes no mirror bytes.
  - One workspace: a workspace with no tracker project switches either way
    (`tasks_source: vault`, or the key removed to inherit). One that mirrors a
    tracker project can't move to Hollow alone yet (the pass would need to skip
    one project: follow-up), and one workspace can't be tracked while Settings
    keeps tasks in Hollow; both are refused with the reason.

| Command | Tier (action) | Answer |
|---|---|---|
| `sources_status` | read (`sources.status`) | `SourcesStatus`: rows Tasks, Knowledge, Personal (binding, label, last read, queued and held edits, conflicts, problems), workspaces with an override, binding or marker, references (in the library or not), generation, `storedInHollow`, an unfinished switch, and the task and project keys the tracker owns and the ones sent (`trackerTaskKeys`, `sentTaskKeys`, `trackerProjectKeys`, `sentProjectKeys`, from `owner_of`) |
| `sources_set` | confirm (`sources.set`) | `SourcesStatus`; saves references; an owner change is refused (a switch, not a Save) |
| `sources_migration_plan` | read (`sources.migration_plan`) | `SourcesMigrationPlan` for `{workspace?, to}` |
| `sources_migration_apply` | confirm (`sources.migrate`, Lease::Outside) | `SourcesMigrationOutcome`: the plan, the record, resumed |
| `sources_migration_rollback` | confirm (`sources.rollback_migration`, Lease::Outside) | `SourcesMigrationRecord` |
| `sources_migration_status` | read (`sources.migration_status`) | `SourcesMigrationStatus`: switches newest first, the unfinished one |

### Settings, Sources and the labels (H4b, T5)

- **Settings, Sources** (`settings/SourcesPanel.tsx`, lazy; words in
  `sourcesSettings.ts`): under "Where your work lives", three role rows.
  **Tasks and projects** offers T5's two choices, Vault only or Vault plus
  Linear (T4, Notion as a task source, is gone). Picking the other one runs
  `sources_migration_plan` and shows its counts; the switch button stays off
  while edits wait, are held or conflict, or while a switch stopped part way
  (Finish the switch repeats only the steps left), and applying needs a
  confirm. The last switch has Roll back. **Knowledge** is the vault plus a
  list of read-only connector references (add from the connector library,
  Remove; Save is `sources_set`). **Personal context** says "Always in your
  Hollow folder" with no control and opens Context. **Workspace task
  sources** lists overrides (`Inherits: Linear`, `Override: vault only`) with
  a switch each, and a workspace picker for a new one.
- **Labels on every task:** `Task.sourceLabel` is filled by
  `sources_api::label_tasks` for the Tasks list (`enrich_tasks`) and the
  phone's `GET /v1/tasks` (`sourceLabel` through `Host::label_tasks`,
  docs/phone-api/tracker.md), and `TrackerViewRow.sourceLabel` by
  `label_rows` in `tracker_view`. A task with a tracker link uses the tasks
  role's binding; a task the tracker doesn't hold reads "Stored in Hollow"
  whatever the role says. Every row carries its label (title and screen
  reader text); a chip shows when it differs from the group (unreachable,
  was tracked), the toolbar shows the role's, and the issue and local task
  details show their own with Open in Linear. Unreadable settings leave
  items unlabeled, never mislabeled.
- **The tracker row can't move tasks:** while tasks follow the tracker
  block (no `sources` choice saved), `set_tracker_settings` refuses turning
  the tracker off (`TRACKER_OFF_IS_A_SWITCH`): that would move tasks to the
  vault with no dry run. Switch to Vault only first.

## The API key

Hollow's one Keychain bundle (`secret_store`, key `tracker:<provider>`).
Settings saves a pasted key (`set_tracker_key`; one line, no spaces), removes
it (`clear_tracker_key`), or copies the provider's own Keychain item into the
bundle once (`use_keychain_tracker_key`; Linear's item is `hollow-linear` /
`default`, created 2026-11-07). The copy means later builds never prompt
for a second item (`secret_store`'s module doc says why). The key never
reaches a log, an error, argv or the frontend; `TrackerStatus.key` says only
`missing`, `saved` or `unreadable`.

## IPC (`tracker/ipc.rs`, `commands/tracker_api.rs`, `src/trackerApi.ts`)

| Command | Tier | Answer |
|---|---|---|
| `tracker_status` | read (`tracker.status`) | `TrackerStatus`: state (`off`, `needsSetup`, `blocked`, `ready`, `syncing`, `error`), provider, team, transport, interval, key state, what is missing, what blocks it (always null since Notion sync's removal; `blocked` stays in the vocabulary so the IPC shape doesn't change), last and next sync, last error, requests left, outbox, problems, restart needed, whether the engine is built |
| `tracker_view` | read (`tracker.view`) | `TrackerView`: the team's states, issue rows (status kind, priority, key, title, labels, project, due, assignee, estimate, cycle, milestone, lock, pending, conflict) and project rows (status, lead, target date, progress, counts), filtered by `TrackerViewQuery` |
| `tracker_issue` | read (`tracker.issue`) | `TrackerIssueDetail`: the issue with every field, its project, path, lock, writable fields, conflicts, pending edits and the file's vault-only notes |
| `tracker_issue_history` | read (`tracker.issue_history`) | `TrackerHistory` (`events`, oldest first, and `more`): one mirrored issue's change history read from the tracker now (`TrackerAdapter::issue_history`, Linear's `issue.history(first: 50)` through the team filter: its newest 50, and `more` is that connection's `hasNextPage`), one event per field changed (state with both kinds, assignee, priority 0 to 4 as text, labels added and removed, project, title, description). Read-only and never stored in the mirror; an unmirrored issue is refused before any read, and errors carry the code and path only. Adapters without it answer "unsupported" (the trait's default) |
| `tracker_link_setup` | read (`tracker.link_setup`) | `{value, steps}`: the Linear Custom link value (`hollow://issue?prompt={{prompt}}`) and where to paste it |
| `take_linear_links` | plumbing | the queued `hollow://` links, each `{seq, source, identifier, tracked, workspaceId, draft, note}`; drains the queue ([Linear link](#linear-link)) |
| `tracker_team_meta` | read (`tracker.team_meta`) | `TrackerTeamMeta`: states, labels, members, cycles, milestones, projects, viewer (whom the key belongs to, or null) |
| `tracker_sync` | write (`tracker.sync_now`) | `TrackerSyncOutcome` (Sync now) |
| `edit_tracker_item` | user-only | `TrackerWriteOutcome`; `TrackerEdit` carries new values and the values read (compare-and-swap) by `TrackerField` name |
| `create_tracker_task`, `create_tracker_project` | Write, no card (`tracker.create_task`, `tracker.create_project`; T1) | `TrackerWriteOutcome`; stable caller `idempotencyKey`; AI or HOL by `team_for` |
| `archive_tracker_task` | Write, no card (`tracker.archive_task`; left ALWAYS_ASK 2026-11-15) | `TrackerWriteOutcome`; AI or HOL issues only |
| (verb only) | Write; a card in every mode only for a destructive or access-granting mutation (`tracker.graphql`) | one Linear GraphQL query or mutation with the saved key; see [Linear GraphQL](#linear-graphql) |
| (verb only) | Write, no card (`tracker.ensure_labels`, T1) | `LabelsEnsured` per team (`team`, `created`, `present`); see [Team routing and labels](#team-routing-and-labels-t1-2026-11-15) |
| `write_friday_status` | Write (`tracker.friday_status`) | `TrackerFridayStatus`: the notes, the digest, the Herald line and any tracker copies |
| `attach_canvas_to_issue` | Write, no card (`tracker.attach_canvas`) | `TrackerWriteOutcome`; a copy of a saved canvas on a team ENG issue |
| `set_tracker_key`, `clear_tracker_key`, `use_keychain_tracker_key`, `set_tracker_settings` | user-only | `TrackerStatus` |

Lock kinds (`TrackerLock`): `linked`, `remoteMissing`, `moved`, `unreadable`,
`staleRelations`, `outOfScope`.

The engine is built (`commands::tracker_api::ENGINE` is true). Reads use
the local ledger, not a network request. Sync now runs a complete pass;
issue edits queue durably, and issue create and archive use the scoped
adapter. Project creates and archives persist attempted intent before calling
the provider. A lost reply never authorizes another attempt; unresolved
project creation requires checking the tracker before a new request. Project
edits use staged recovery; unsupported status names become conflicts. The
running broker policy is registered from AppState before the broker starts,
never inferred from a saved config file. Its provider/team must match the
validated ledger. The ledger's
`writableFields` comes from the adapter. A missing legacy list uses that
provider's declared static capabilities without a network or key read;
an explicit empty list and unknown providers stay read-only. Creates remain scoped to the configured team and anything meant
for the person also appears in Hollow. The browser mock
(`src/mockTracker.ts`) plays the finished tracker, so the screens build first.

## Linear link

Linear's Custom link coding tool opens a URL in which `{{prompt}}` is the issue's
prompt. Settings, Sources, Task tracker shows the value to paste, with a
Copy button, and the COO's `tracker.link_setup` returns the same text:
`hollow://issue?prompt={{prompt}}` (`tracker_link_setup`, a constant). The
scheme is registered in `src-tauri/Info.plist` (`CFBundleURLTypes`, what the
deep-link plugin writes on macOS, without the crate), so macOS hands the URL to
the running app, or launches it, as `RunEvent::Opened` beside markdown files.
It works in an installed build only; `tauri dev` doesn't register the scheme.

`linear_link.rs` handles it. Accepted: `hollow://issue?prompt=<urlencoded>` and
`hollow://issue?id=ENG-123`. The URL is untrusted input:

- Over 200,000 bytes it is refused; the decoded prompt is cut at 16,000
  characters (the draft says so).
- Control characters are stripped (newlines and tabs stay in the prompt), and
  so are zero-width, bidirectional and tag characters.
- Only `prompt` and `id` are read (the first of each); anything else, a port,
  credentials or a fragment is ignored. Another host or scheme opens nothing.
- Nothing reaches a shell, a path or a setting. The one file read is the
  frontmatter of a `tasks/*.md` mirror file (symlinks skipped), and the only
  writes are the History row and an in-memory queue of five.

The identifier comes from `id`, else from the prompt: Linear's
`identifier="ENG-307"` attribute wins, else the first identifier-shaped word,
preferring team ENG. It is looked up as `tracker_key` in the mirror with
`tracker_state: active`. Team AI only: another team's identifier, an unmirrored
one or none opens the **COO** with the text and the note "Hollow doesn't track
LP-12 (Hollow tracks team ENG only)". A mirrored issue opens its **project lead
chat** (the workspace named by the file's `project_url`, which must exist and
not be the COO's), or the COO when it has no project. The composer gets
`Linear issue ENG-307: <title>`, a link to the task file and the tracker URL,
then the prompt under the line "Prompt from the Linear link:" (text from
outside never reads as the issue's own words). **It is a draft: Hollow never sends it, starts a run, writes a
file or changes a setting.** A chat that won't open falls back to the COO.

Each open adds a History row (`automated_history::record_event`: a finished run
with no changes and no vault lease, so a live run doesn't block it) with source
"Linear link" and the identifier, never the prompt; a refused link adds none,
and the same URL twice within 3 seconds is one open. Event rows have their own
retention budget (100), so a burst of links can't push real runs out of the 200.
The mirror scan, the History write and the queueing run on a spawned thread,
never on the OS callback. The backend queues the result and nudges the window
with `linear-link-pending` every half second (20 seconds at most) until
`take_linear_links` drains it, so a cold launch needs no boot call (the launch
IPC budget); the window gaining focus nudges again while a link still waits.
The window waits for the workspace list on a cold launch, and retries a chat
that won't open (3 tries) before saying which identifier didn't open.

A Hollow Candidate installed beside the everyday app shares the `hollow://`
scheme (both bundles merge the same `Info.plist`); macOS picks one. Not yet
verified on a built `.app`: that the key survives the bundler's plist merge
(`plutil -p` on the app), and that Linear percent-encodes `{{prompt}}` (one
real click; an unencoded `&` or `#` in a prompt would cut it short). The window half is `src/linearLink.ts`,
loaded lazily. Tests: `linear_link.rs` (parsing, the identifier match, unknown
and other-team identifiers, draft-only, the cap), `automated_history_tests.rs`,
`src/linearLink.test.ts`.

## Hollow's screens for the tracker

**Settings, Sources, Task tracker** (overhaul A14,
`src/settings/TaskTrackerPanel.tsx`, words in `src/trackerSettings.ts`): the
first block in Connections. Provider (Off or Linear); the team, which is only
ever the Engineering team (`ALLOWED_TEAMS`: choosing Linear saves that team,
and a config naming any other team shows "Use Engineering (AI)", never a
picker); the key's state with Save key, Use the key saved in Keychain and
Remove key (asks first); last and next sync, requests left this hour, edits
waiting to send, the last error; the interval (1, 2, 5, 10, 15, 30 or 60
minutes) and Sync now (enabled only when a pass can run). A `tracker-sync-updated`
event refreshes it. Service health lists the tracker first, named for its
provider, with agent access matched on the provider's id; the capability doctor
has a Task tracker check (off is declined, a pass within two intervals is live).
Settings search finds it under Task tracker.

**Tasks** (overhaul B1, `src/pages/Tasks.tsx`, `pages/tasks/IssueList.tsx`,
`pages/tasks/IssueDetail.tsx`, words and edits in `src/trackerTasks.ts`): the
team's issues from `tracker_view` grouped by status (in progress, to do,
backlog, done, canceled; within a kind the team's state order; done and
canceled start folded), each row with priority, key, status, title, labels,
project, due and assignee; the vault's untracked tasks in a Local tasks group
that opens the local editor. An issue opens beside the list (`tracker_issue`)
with the title and Markdown description (both edit), conflicts (Use mine or
Keep the tracker's), vault notes, sub-issues and relations (rows from the
loaded list: status, key, title and assignee, a click opens it; a key the list
doesn't hold shows as the key), links and attachments, an activity feed and an
Agent activity section that reads `TrackerIssueDetail.agentRuns` once lane 4
adds it; the properties column edits status, priority, assignee, labels,
project, milestone, cycle, estimate and due through `edit_tracker_item`, one
field at a time, with the value read as `expected` (`fieldEdit`), and says on
hover why a field is locked (`fieldLock`). New issue uses
`create_tracker_task`; Archive asks first and uses `archive_tracker_task`;
Open in Linear opens the issue's URL. Page keys only, never with a modifier:
j/k or the arrows, Enter, Escape, c, /.

The activity feed (`pages/tasks/issueActivity.ts`) reads
`tracker_issue_history` when the issue opens, after each edit and after a sync
pass (a signal, never a resource), and merges it with the comments in time
order after "created": "Alex Rivera moved from Todo to In Progress, 2h ago".
When `more` says older changes were left out, one quiet "Earlier changes
aren't shown" line sits just before the oldest change shown (older comments,
which are complete, keep their place above it).
When the history can't be read (tracker off, offline, refused) it falls back to
the mirror's synthetic feed: created, comments, Last updated.

**Full page** (row T1, 2026-11-12): the pane's Open full page shows the issue
as the whole Tasks page (the list, view bar, toolbar and filters hidden; the
list stays mounted and its scroll and selection come back). It is always two
columns: the content on the left, a sticky properties column on the right
(Status, Priority, Assignee, Labels, Project, Milestone, Cycle, Estimate, Due
date, then Created by, Created, Updated and Open in Linear); under 45em (720px)
of detail width the properties stack above the content. Back to list, or
Escape when focus isn't in a field, returns; on a narrow window, where the pane
replaces the list, it closes the pane. The mode is kept per window
(sessionStorage `hollow.tasks.issuePage`).

**Views and filters** (overhaul B2, `pages/tasks/ViewBar.tsx`, model in
`src/trackerViews.ts`): view tabs My issues, All, Active (started and not
started), Backlog, By project and By assignee (both open issues only); a filter
bar of chips for status, priority, assignee, label, project, cycle and due date
(each with a none value where one exists), added from Filter and removed with
its × or Clear filters; Display's Group (status, assignee, project, priority)
and Order (priority, due date, last updated). Choosing a view resets the
grouping to the view's own. Local tasks show in My issues, All and Active under
the label and due filters, and hide under a filter only an issue has. The choice
is saved per window (sessionStorage), and a new window starts from the last
choice anywhere (localStorage, `hollow.tasks.view`). My issues keeps the issues
assigned to `TrackerTeamMeta.viewer`, whom the key belongs to: the pass reads it
with `whoami` alongside the hourly team metadata (or on the next pass for a
ledger saved before it was kept); a failed read leaves it unknown without
failing the pass, and the view then says Sync now reads it instead of showing
nothing as yours.

**Conflicts in the page** (row T2, 2026-11-12, design section 7): while any
row has `conflict`, the view bar ends in "Conflicts (n)" (`trackerViews.ts`
view `conflicts`, not a tab: `TAB_VIEWS` leaves it out, `showConflicts` keeps
it while n > 0 or while it is chosen; saved like the others). It lists the
issues with an open conflict, closed and archived ones included, grouped by
status, under the filters; local tasks don't show. With none left it says No
conflicts and offers Show all issues. A row's Conflict flag says on hover
that the edit was not applied and how to settle it. Settling stays in the
issue detail: Use mine is the design's Re-apply (it sends your value again)
and Keep Linear's takes theirs; saving that field acknowledges the conflict.
The browser mock seeds conflicts with `mockTracker.conflicts`
(`{ [issueId]: TrackerConflict[] }`), and a save of the field clears it.

**A project** (overhaul B3, `pages/tasks/ProjectDetail.tsx`, words in
`src/projectPortfolio.ts`, backend `src-tauri/src/project_portfolio.rs`): one
component with two entry points, an issue's Open project on the Tasks page
(the pane) and a project record in Library, Records (`workspaces/<id>/PROJECT.md`
opens as the project; Show the file opens the editor). A tracked project shows
the tracker's status, lead, progress (completed over all its issues, from the
mirror's rows), open and total issues, target date, description and Open in
Linear; on the Tasks page its issues list inside the detail (Library shows
the counts). Every
project shows the vault's portfolio fields from PROJECT.md (department, type,
hours saved and value a year, owners, GitHub and live site; requested by,
stakeholders, domains, meetings, captures, War Room and docs as lists, a vault
path opening in Library), edited with Edit and saved through
`set_project_portfolio` (compare-and-swap on the revision; `PORTFOLIO_CHANGED`
keeps the draft and offers Reload). The tracker's keys are refused there and a
pass never writes the portfolio keys. A vault-only project offers Track in
Linear: it names what it creates, then `create_tracker_project` with
`workspaceId` makes the team project and writes the tracker's fields and
identity into that PROJECT.md (`project_portfolio::link_tracked`, the same
renderer a pass uses), so the next pass adopts the file and has nothing to
send back; a tracked one is refused before any request. The project chat
button opens the project's workspace. Agents have `workspaces.project_portfolio`
and `workspaces.set_project_portfolio`; `tracker.create_project` takes
`workspaceId`.

**Comments** (overhaul B5, `CommentBox` in `pages/tasks/IssueDetail.tsx`,
backend `comment_tracker_issue`, `tracker_sync::comment`): the issue detail's
Activity ends in a comment box. A comment (at most 2,000 UTF-8 bytes, the
adapter's limit; the box counts bytes) is journaled in the ledger's `comments` under the box's
request key before any request, so a retry never posts twice; the adapter
reads the issue's team before it may send (`create_issue_comment`'s
authorize seam), a failure before that removes the entry (nothing was sent),
and a lost reply after it is unconfirmed and never resent. A confirmed
comment joins the mirrored thread at once; the next pass reads it again by id.
Drafts are kept per issue with their key until they post. Archived, moved or
missing issues take no comment. Agents have `tracker.comment_issue`
(Write, no card since T1; the issue's team is read before sending).

**Projects** (overhaul B4, `pages/tasks/ProjectList.tsx`): a Projects tab
after the issue views (`trackerViews.ts` view `projects`, saved like the
others). One dense row per team project from `tracker_view`'s `projects`:
name and key, status, lead, target date, progress (completed over all the
project's issues, from the mirror's rows) and open of total issues; started,
planned, backlog and paused first, completed and canceled last, then by name.
Find a task narrows them by name, j and k move, Enter opens. A row opens the
project detail beside the list with its issues (the same list, filtered to the
project, grouped by status with done and canceled folded); an issue opens in
the pane, and its Open project comes back. The issue filters and Display don't
apply on this tab.

**Library, Records**: `library/records/` (meetings, domains, War Room,
captures; never `library/records/people/`, which agents keep) and every
`workspaces/*/PROJECT.md`. A tracker mirror reads "Tracked in Linear: its
fields come from there" in Library (`sourceAuthority` `tracked`).

## The Linear adapter (`tracker/linear.rs`)

`LinearAdapter` implements `TrackerAdapter` over `api.linear.app/graphql`,
the key in the `Authorization` header as is (marked sensitive; never in a
log, an error or `Debug`; the key only ever goes to Linear's own endpoint,
since the stub endpoint is for tests only). `LinearAdapter::from_config`
needs provider `linear`, transport `graphql`, a team set by its id (a key
like `ENG` is refused, since it would match nothing) and a saved key, else
`NotConfigured` says which is missing.

- **Scope guard.** Every issue, comment, attachment, project, milestone and
  label query names the team in its filter (`issues(filter: {team: ...})`,
  `comments(filter: {issue: {team: ...}})`, `projects(filter:
  {accessibleTeams: {some: ...}})`, labels of the team or of no team). A node
  that still carries another team is dropped with one log line (ids only),
  and so are nested ones: a parent, sub-issue or related issue in another
  team is left off the issue, and a comment whose issue is in another team
  (or on no issue) never arrives. `issue(id)` reads through the team filter
  too, so another team's issue is `NotFound`, never read.
- **Writes.** `create_issue` always sends the team's id; `update_issue`,
  `archive_issue` and `add_link` first read the target (and a new parent)
  through the team filter and refuse with `OutOfScope` when nothing comes
  back, before anything is sent; the mutation's reply is checked for the
  team again. A project is written only when the team is its only team (a
  project shared with another team is read, never written). An update sends
  only the fields its patch touches (`Some(None)` clears with null) and never
  a team. Issue links use `attachmentLinkURL`, project links
  `entityExternalLinkCreate`. The other ids a write carries (state, cycle,
  milestone, project, labels, assignee) come from the adapter's own
  team-scoped reads, and Linear refuses a state, cycle or label of another
  team on an issue of this one; the restricted key can't see other teams'
  ids at all.
- **Projects.** A project's `key` is Linear's `identifier`; its `team_ids`
  keep the scoped team only, so another team's id never leaves the adapter
  (a shared project is still read, and still never written).
- **Pages and points.** Linear refuses a query over 10,000 complexity
  points. Issues come 40 a page with labels, sub-issues and relations 20 an
  issue (about 8,700 points counted generously); an issue whose nested list
  comes back full is read again with the detail query (lists of 50,
  comments and attachments included, about 780 points; a list longer than
  50 in detail is cut there). Comments 100 a page, attachments for 50
  issues a query, milestones for 20 projects a query, projects 50 a page,
  states, labels, members and cycles 100 a page. Linear's own counts in the
  live smoke (2026-11-08, `X-Complexity`) were lower than the estimate for
  every query but one: issues page 116, detail 265, comments 540, projects
  1,911, labels 391, states 292; milestones for 50 projects cost 5,555
  against an estimate of 3,356, hence 20 a query. Cycles are the ones ending in the
  last 30 days or later. List issues carry no comments or attachments: those
  come from `comments_changed` (its own cursor, so a comment-only change
  arrives even when the issue's `updatedAt` doesn't move) and
  `attachments`. Each page is sorted oldest change first; the engine moves a
  cursor only after the last page. Attachments whose `sourceType` is absent
  or `url` are links; the rest (GitHub, uploads) are attachments.
- **Rate limits.** Each reply's `X-RateLimit-Requests-*` and
  `X-RateLimit-Complexity-*` headers become `rate()` (the engine waits for
  the reset under 10% left; `RateBudget::is_low`), and `X-Complexity` the
  last query's points. A 429 or a `RATELIMITED` error backs off for 30 s,
  doubling to 15 minutes; a call inside the window fails with
  `RateLimited {reset_at}` without a request, and a good reply clears it.
- **Errors.** `AUTHENTICATION_ERROR` or 401 is `Auth`, `FORBIDDEN` is
  `OutOfScope`, a not-found error `NotFound`, anything else `Provider` with
  a known Linear code (`KNOWN_CODES`, else "an unrecognized error") and the
  GraphQL path's fields that are words of the query Hollow sent
  (`INVALID_INPUT at issueUpdate`). Nothing else the server wrote is kept,
  since a message, code or path can echo what was sent (an issue's title,
  the key); an error that still names the key loses its text.

## Tests

`tracker/stub.rs` is a canned GraphQL server for tests: it records every
request's headers and GraphQL body, replies with headers of its own
(`X-RateLimit-*`, 429), and `unscoped` refuses any issue or comment query
that doesn't name the team, so an adapter test fails if a query forgets the
filter. `issue_node` builds an issue in Linear's GraphQL shape.

`tracker/linear_tests.rs` runs the adapter against it: another team's node
in a reply is dropped; the stub refuses an unscoped request and every adapter
read passes it, and each of the 13 reads is checked for the team at its exact
filter path; a 429 or `RATELIMITED` backs off and a good reply clears it;
a mutation on another team's issue (or under another team's parent, or on a
shared project) is refused before anything is sent; a comment on another
team's issue is dropped and a comment-only change arrives through the
comments cursor; the detail query stays under the point cap with 50
comments and 50 attachments, and `every_query_stays_under_the_point_cap`
counts each query with Linear's published rules (a field 0.1, an object 1,
a connection multiplying everything under it by its page size). The ignored
`live_read_only_smoke` runs only by hand on this Mac with the key in the
environment for one command; it calls reads and a schema check of the write
inputs, and prints counts and points, never issue text.

The audit record's tests (X2): `audit_tests.rs` (the chain, the triggers,
an edited row and a gap found by `verify`, the filters, one row per request
key and outcome, a counted DB error), `tracker/audit_sync_tests.rs` (each IPC
and agent write path leaves one row and the row count equals the stub's
mutations; another team's issue through each path is refused with zero
mutations and a failed row; a dropped audit table never fails a comment) and
`tracker/work_audit_tests.rs` (start and end rows, the Outcome's numbers equal
the `run_ended` row's, the outbox's writes and refusals).

## How Hollow reaches the tracker

| | GraphQL API with an API key | Linear's MCP server with Hollow's `linear` sign-in |
|---|---|---|
| Team scope | The key can be limited to the team, so Linear enforces "never touch another team" | Sees every team the user sees; scope is enforced only in Hollow's code |
| Endpoint | `api.linear.app/graphql`; typed schema, `updatedAt` filters, `includeArchived` | `mcp.linear.app/mcp` tools; output shaped for models |
| Limits | 2,500 requests and 3,000,000 complexity points an hour; 10,000 points a query | 5,000 requests and 2,000,000 points an hour |
| Use | **Primary** | **Fallback** (built only without a key) |

Polling, no webhooks (a webhook needs a public HTTPS address; routing it
through the homelab would put work data there). One incremental issues query
and one comments query every 120 s (plus attachments for changed issues),
projects every 10 minutes, states, labels, members, cycles and milestones
hourly: about 75 requests an hour, about 3% of the key's budget. A full pass
at 3 AM catches what an incremental pass can miss.

## The engine (A5)

`tracker_sync.rs` owns a mutex, a non-waiting `.tracker-sync.lock` flock,
then a `tracker-sync-` History lease. The lease covers root task files and
the project folders it actually patches. Since SE4 (2026-11-16), known
in-process syncs and scoped chats run beside each other in either admission
order, including in the same folder. Each mirror commit uses atomic CAS or
exclusive creation inside `automated_history::host_write`; History serializes
only that commit and its attribution with admission and finish, never a network
wait. A stale baseline refuses the write and preserves the newer edit. Unknown
or foreign sync owners retain conservative scoped exclusion. Every
root broker write must acquire the same exclusive full-vault lease before
writing, including writes from an unrelated scoped chat. Folder coverage
does not grant permission to bypass the root writer lease.
A held lease is `LeaseHolder::TrackerSync`, shown as
"task tracker sync". The watched
ledger is written through `run_guard::write_host_state`, note writes use
the vault's compare-and-swap writer, and destination symlinks are refused.

The version-1 `.tracker-sync.json` uses camelCase keys: `provider`, `scope`,
`cursorIssues`, `cursorProjects`, `cursorComments`, `lastSync`, `lastError`,
`records` (id to `Record`), `outbox` (id to editable `values`, `base`, and
optional `staged` before/after field maps for an interrupted editor write),
`creates` (caller key to input, timestamp, attempted flag, issue id and receipt), the
team metadata cache and its time, the project refresh time, full-pass date,
failure count, retry time and rate reading. Each record has id, key, kind,
path, workspace id, editable baseline, conflicts, error, remote-missing
flag, state, and its typed issue or project snapshot. Conflict fields are
vault key strings, covering both issue and project keys. A corrupt, unsafe or
differently scoped ledger refuses sync. An existing tracked file with no
baseline refuses too: restore or migrate the ledger, never guess a base.
An existing mirror must bind its `tracker` and `tracker_id` or
`tracker_project` to the record before any sync/edit workflow uses its path.
Migrated paths keep working while bound; a missing noncanonical path is
never recreated. Record payload kinds and rediscovered path uniqueness
are validated too.
Baseline, pending and staged field maps admit only tracker-owned keys;
the renderer also filters supplied values before patching, so an unknown
map key cannot overwrite vault-only frontmatter.

The initial pass waits 20 seconds for first paint, unless a local edit wakes
it sooner. Pulls run every configured interval (120 seconds by default,
clamped to 1 to 60 minutes) and on Sync now. Issues and comments have
independent paged cursors; cursors advance after complete page fetches and
durable retry snapshots for any failed items. Failed items retry from those
snapshots without holding unrelated cursors back. An incomplete fetch never
advances its cursor. A repeated cursor or 1000-page limit fails closed with
an error rather than looping or silently skipping data.
Projects refresh every ten minutes; states, labels, members,
cycles and milestones hourly. The first pass and one pass during local
3 AM fetch the full set. A missed daily pass remains owed after 3 AM until
completed, including when the Mac wakes later. HTTP failures and rate limits back off from 30
seconds to 15 minutes; the retry time survives restart. A low provider rate
budget holds the next pass until its reset. Health records contain counts
or generic reasons, never issue text or the key.

Before fetching, the engine saves local edits to its outbox. A changed
issue is merged against the last successful baseline, with a fresh scoped
detail read before a local update. A disjoint local change is sent; when
both sides changed the same field, the tracker wins and the local value is
kept in the ledger and a `tracker:conflicts` body block. The read-only
detail and comments are restored, never included in an update. User edits
pin every changed field to the value the editor read. They write the local
mirror and durable outbox, retaining all outstanding fields across
successive edits and leaving unrelated conflict notes visible. Reverting
all queued values to the baseline clears the outbox and pending flag after
the file write succeeds. A complete revert keeps its own staged journal
until then, so an interrupted revert cannot resurrect the previous edit.
An editor journal written before its mirror includes the before/after
snapshot. Capture finishes that interrupted file write before recomputing
pending fields. A newer conflicting file edit preserves both file and
journal and refuses recovery. A missing mirror with queued changes stays
missing and keeps those changes until restored.
The editor also restores previous staged writes before accepting another
request, so an edit arriving before the next capture cannot replace an
unwritten intent. Conflict acknowledgements are saved in the same journal
transaction as the changed values. Create-recovery diagnostics are saved
at the failed or ambiguous read, without waiting for a later item write.
Edits then wake the timer with a 250 ms debounce;
network availability and a current sync can extend delivery. Task-broker,
phone and agent write-through hooks arrive with A6.

Project name, description, lead and start/target dates use the same outbox,
fresh scoped re-read, three-way merge and CAS write. Project status snapshots
currently contain no status ids, so a local status edit is recorded as a
conflict and the provider's status is restored; no guessed status id is
sent. Its exact project file is available from Needs you. A pending project
edit forces a fresh project pull before the ten-minute interval elapses.

An issue create saves its caller key, input, timestamp and an issue id of
its own (a UUID v4, `issueId` in the journal) before marking the attempt,
and sends that id as `IssueCreateInput.id`; Linear refuses a second create
with it. A lost reply or a crash mid-send is checked at the next pass by
reading that id in any team (`TrackerAdapter::create_landing`, Linear's
`IssueLanding`: no team filter on purpose, only the id and team id
selected). In the team: adopted. In another team (moved there): the error
stays and nothing is sent. No node at all, from a well-formed answer: the
create never landed. An agent run's create (key `work-...`) then has
`attempted` cleared and the work outbox resends it with the same id, under
its project check and 10-minute give-up. A person's create is resent by the
pass itself with the same id, at most three times (`resends`, saved before
each send); after that `attempted` is cleared and the error asks for it to
be created again from the task, which reuses the saved id. A read or parse
error or an odd answer keeps `attempted` and the error; "not found" text
never counts as absence here. Hollow never adopts an issue by matching its
fields: a person's issue with the same title, body and project is not this
create (finding Q-R22, 2026-11-13). A journal an older build wrote has no
id and stays "no unique confirmed outcome"; an unattempted one gets its id
before its first send.
The current user IPC reuses a hash of an identical create payload as its
journal key, so resubmitting it after a lost reply is also idempotent.
Agent callers in A6 supply their own stable request keys.
Archived or trashed issues get `tracker_state: archived`. A previously
mirrored id that is no longer visible in the configured team gets
`tracker_state: moved`, `remoteMissing` and a note; its files are retained
and its outbox stops. That state can mean moved or otherwise unavailable.
Any unsent local values are retained as conflict notes when an issue is
archived or becomes unavailable.
Complete project snapshots apply the same unavailable state to a missing
project, preserving unsent values as conflicts. Its mirror stays readable
with `tracker_state: moved`; a returned project becomes active again.
The state and parked conflicts are still written if its cached project
snapshot is absent. An unavailable task whose file is missing remains
missing, rather than creating a template that could erase its local notes.
Independently paged links and attachments survive issue detail re-reads;
a full comment refresh drops comments no longer returned.

`tracker/mirror.rs` is the one pure renderer for sync and the importer.
It patches only owned frontmatter and `tracker:description`,
`tracker:comments`, `tracker:conflicts` blocks. Unknown keys, lists and
vault-only prose keep their bytes. Typed lists use JSON flow syntax, valid
YAML; comments cannot inject a closing block marker. Work mirrors carry
`scope: work`, and new project mirrors use
`workspaces/<provider>-<id>/PROJECT.md`, so the work-data guard applies.
Migration retains existing paths by seeding `Ledger` and rendering the
full snapshot with this module. Its first sync changes no mirror bytes or
mtimes; ordinary ledger sync timestamps still update. The task schema is
design section 5 of the 11-07 Linear plan in the Hollow workspace.

Every timer or manual pass emits `tracker-sync-updated {ok, changed}`.
The local A6 bridge in `tracker/seams.rs` recognizes tracker mirrors before
the legacy task parser. It takes the sync mutex and process lock before the
task broker's History lease, then calls the same staged editor without a
second background lease. Pinned file hashes are mandatory. Task metadata
includes provider identity, state kind, exact priority and ownership locks;
tracker ownership takes precedence over retained Notion archive metadata.
Description, comment and conflict blocks are excluded from local checklist
and activity parsing. Local activity writes never edit those blocks.
Old generic task writers refuse tracker edits; the user task commands and
broker route supported fields to the tracker editor instead. Priority
migration skips tracker mirrors. Project edits share staged recovery;
create/archive actions journal attempts before sending. A project create
outside scope reports an error, retains its diagnostic and never resends.
Rich task detail requires the same ledger path and id as the requested file.

Completing a recurring tracker issue uses the local completion-date recurrence
calculation and saves the next due and unique unstarted state in one durable
edit. Several unstarted choices refuse the edit rather than guessing. Broker
and phone results retain `rolledForwardTo`; a stale hash cannot roll twice.
The shared task/project editor trims only outer hash whitespace before CAS.

Tracker claims keep `claimed_by` and `claimed_at` local. Claim and release
take running tracker exclusion before History, validate the bound active
record and patch only those two keys. Provider status and body blocks never
pass through the generic renderer. Any nonclosed active tracker state can
carry local ownership; other owners and stale hashes still refuse. Closed
or unavailable projections clear ownership, so reopening cannot revive it.
Staged task replay uses the same projected edited state as the normal editor.
Each cached field resolves independently, so an ambiguous unchanged member or
project name cannot hide a completion. A changed state must resolve before
the durable journal can be rendered. Tracker issue keys and ids join the
shared blocker index only when unique; unresolved duplicates remain warnings.
Phone extensions: [contract](phone-api/tracker.md).
The engine is never run against the real vault during the overnight build.
Acceptance tests use `tracker::stub` and temporary M4-style vaults; set
`HOLLOW_TRACKER_FIXTURE_DIR` to put them in lane scratch on a test runner.

## The importer (`hollow tracker-import`, `tracker_import.rs`)

The one-time Notion-to-vault migration and the tracker rekey (design
`linear-source-of-truth-2026-11-07` sections 5 and 8, board rows M1 and M2),
a hidden subcommand run from a lane build with explicit paths:

```text
hollow tracker-import --config <config.json> --vault <vault>
    (--preflight --staging <dir> | --dry-run --staging <dir> [--mapping <md>]...
    | --apply --backup-dir <dir> | --rekey --backup-dir <dir> [--mapping <md>]...
    | --rollback <manifest.json>) [--member <name>]
    [--bind-project <project key or id>=<workspace dir>]...
```

- **Read only outside the vault.** Notion goes through the importer's own
  client (`tracker_import/notion.rs`): a closed `Request` enum of three GETs
  and the data-source query POST, 350 ms apart; a test checks the variants
  and the file's text. API redirects are off; Working Files are plain GETs
  on a separate client with no Authorization, capped at 25 MB while reading. The tracker goes through
  `tracker::adapter_with_key` (the team scope guard included) and only its
  read methods. Nothing is created, edited or archived in either service.
- **Secrets.** The Notion token is `notion.token`, or the legacy
  `notionToken`. The tracker key is Hollow's bundle (`tracker:<provider>`),
  else the provider's own Keychain item, read and never copied. Neither is
  printed or put in argv, and every message and report line is scrubbed.
- **Config.** `tracker` as above; when it is absent the team is assumed to be
  team ENG (`DEFAULT_TEAM_ID`, said in the preflight); any other team, in the
  config or behind the key, is refused. Notion data-source ids
  come from `trackerImport.notionSources` (`projects`, `tasks`, `meetings`,
  `peopleMap`, `peopleMembers`, `domains`, `warRoom`, `captures`, `docs`,
  `aiSubscriptions`); projects, tasks and meetings default to the design's
  ids. A missing id is an error line in the report, and that source is skipped.
- **Modes.** `--preflight` prints two-way sync, the token and key presence,
  `whoami`, the team and whether `--member` (default Sam Taylor) is in it
  (`TrackerUser::answers_to`: name, display name or email, case-insensitive,
  or the email's local part with dots as spaces; Sam's tracker name is his
  email address, which a name-only match missed in dry run 1),
  and copies `tasks/`, every `workspaces/notion-*/{WORKSPACE,PROJECT,NOTION}.md`
  and `.notion-sync.json` into `<staging>/vault-snapshot-<YYYYMMDD-HHMMSS>/ (a new folder each run)`.
  `--dry-run` plans the import and the rekey together on an overlay, writes
  every planned file into the staging folder at its vault path, plus
  `report.json` and `report.md`, and touches nothing in the vault. `--apply`
  writes the import; `--rekey` writes the tracker identity and the ledger;
  `--rollback` undoes one run. `--apply` and `--rekey` refuse while the
  config still has `notion.twoWaySync: true`: Hollow no longer runs that
  sync, but a build from before its removal would write the same files.
- **What apply writes.** Projects: the portfolio fields of design section 5
  into `workspaces/notion-<id>/PROJECT.md` (lists as block lists; a tracked
  project keeps the tracker's title, status and target date), and the Notion
  description as a body section. Tasks: `notion_task_id`, `notion_status`,
  `due_time`, `meeting`, `assignee_note`; `notion_project_ids` removed; done
  tasks get `tracker: "none"` and `archived_from: "notion"`. Records under
  `library/records/` (meetings, people merged by name across both sources,
  domains, War Room, captures, a README), Recruiter docs under that
  workspace's `outputs/docs/`, and the AI Subscription Responses as one table
  (work-only). Record keys are the Notion property names in snake case;
  relations name vault paths, or `notion:<id>`. Working Files go to the
  project's `sources/notion-files/` once (an existing file is never fetched
  again). `.notion-sync*.json`, `.notion-sync-backups/`, `.notion-imports.*`
  and every `NOTION.md` move to `.archive/<date>/` with their paths.
- **What rekey writes.** Each tracker project is bound to one workspace, in
  this order: a `PROJECT.md` that already names it; `--bind-project
  <key or id>=<dir>` (repeatable, wins over every rule below; an unknown
  project, a folder that isn't a workspace, or one already bound is an error
  line and not applied, and it never moves a project an existing `PROJECT.md`
  already binds elsewhere); a workspace its description cites (a Notion
  project first); the one workspace with its title; then the workspace its
  issues cite most. That last count reads the 5th column ("Source") of every
  `--mapping` row (a row with a `vault:tasks/` 6th column; other tables in
  the file count for nothing) whose issue is in the project (paths like
  `workspaces/<dir>/PLAN.md`), or, when none of its rows cites a workspace,
  its issues' descriptions; each issue counts once per folder. The winner
  needs at least 2 votes (1 when the project has at most 2 issues), twice
  the runner-up's count, and a workspace no earlier rule took. Every
  project's winner is found before any is bound: a workspace several
  projects claim goes to the one with twice the votes of every other
  claimant, else to none, and each claimant is reported with the others.
  A `PROJECT.md` naming a project the team doesn't have is an error line and
  is left alone. Anything else is listed under "Tracker
  projects without a workspace" with the reason and the `--bind-project` to
  use, since the first sync pass would create a workspace for it. Every
  binding is listed with its reason. The bound workspace gets `tracker` and
  `tracker_project` in `PROJECT.md` through `mirror::project` (the brief is
  left alone, so the engine never rewrites it); a workspace with no
  `PROJECT.md` (a `local-*` one) gets a minimal one through
  `frontmatter::patch`, in the same manifest. The ledger records the project
  at that path, so the engine's first pass adopts it and creates nothing
  (a test syncs after binding). Task bindings, one file per issue: an existing `tracker_id`; then the
  `--mapping` tables (6th column `vault:tasks/<file>.md`), each row checked by
  key and title, a mismatch reported and not applied; a file named by
  several rows takes the lowest-numbered issue and lists all of them, the
  others getting `tasks/<provider>-<id>.md` with a "Split from" line; then,
  for open Notion files with no row, the issue whose description cites the
  file path, its `TASK-n`, or the same title. The rest are "needs an issue".
  Every other team issue gets its own file. Files render through
  `mirror::task`, and `.tracker-sync.json` is the engine's own `Ledger` with
  `base` set to the tracker's values, so the first sync pass pulls and pushes
  nothing (a test runs one). A ledger with unsent edits or creates, or one
  sync has already run on (`lastSync` set), is refused.
- **`tracker.configure`.** The agent action for the cutover (Confirm,
  `Lease::Outside`, no Tauri command of its own): writes the tracker block
  (provider, default Linear; team; poll interval; `trackAgentWork`) through
  `save_tracker_settings`, Settings' own save, and with `copyKeychainKey`
  copies the provider's Keychain item as `tracker.use_keychain_key` does.
  Any team but Engineering (`11111111-1111-4111-8111-111111111111`, key
  `ENG`) is refused before anything is written. It returns `tracker.status`;
  the key is never an argument, a result or an error. The person's path is
  Settings, Task tracker.
- **Manifest and rollback.** Each apply or rekey that changes anything makes
  `<backup-dir>/<stamp>-<mode>/` with `files/` (the original bytes of every
  changed file), `journal.jsonl` (each entry written before its change),
  `manifest.json` (every path created, changed or moved and each folder
  created, with before and after SHA-256) and the run's report. `--rollback`
  takes `manifest.json` (or the journal of an interrupted run, whose first
  line names the vault), refuses it whole if any path leaves the vault or a
  backup leaves `files/`, and works backwards: restores the bytes, removes created files and empty created
  folders, moves archived paths back, and verifies every hash. A path changed
  since the run is left alone and named, and the run stays rollbackable
  (`rolled-back.json` is written only after a clean rollback). Roll back a rekey before its apply.
  A second apply or rekey on the same data changes nothing, and writes no
  manifest.

## Linear GraphQL

`tracker.graphql {query, variables}` (`tracker/graphql.rs`, added 2026-11-11 on
Alex's go) sends one GraphQL query or mutation to Linear with the tracker
engine's saved key (`secret_store`, never argv, never in the result or an audit
row), so an agent can do what an admin can: project statuses, labels, workflow
states, anything the other tracker tools and the official connector lack.
Write tier since T1 (2026-11-15): a call the guards let through asks no card, from a
chat or an automation; a destructive or access-granting one still asks in every mode (6),
so an automation can't run it, and a COO check-in runs none. Guards, in order, all before anything is sent:

1. One operation per call; no subscription; a mutation names its root fields
   directly (no fragment spread at its root) and has no variable defaults (a
   default could carry a team the guard never sees), so the checks read what runs.
2. No secret-named field selected anywhere in the document, fragments included,
   whatever its alias (`{ webhooks { nodes { s: secret } } }` is refused, since the
   alias would rename the key the result masking reads). Secret-named
   (`secret_name`, 2026-11-12): containing secret, token, password, passphrase,
   credential, authorization or apiKey; a camelCase or snake_case word starting
   `pass` (not bypass); or ending in Key or Keys (privateKey, signingKey), except a
   bare `key`, `urlKey` and `teamKey(s)`, which are ids.
3. Team guard (mutations): no inline literal and no variable (any key named
   `team`, `teams`, `teamKey(s)` or ending `TeamId(s)`) may name a team other than
   Engineering (its id or the key `ENG`); `teamCreate` is refused;
   `teamUpdate`, `teamDelete`, `teamUnarchive` and `teamCyclesDelete` need
   `id` = the AI team. Since 2026-11-13 a team update's `parentId` (inline or at any
   depth of a variable) must be the id of AI or HOL too (null, a key or any other
   team is refused), and the other ids in its input are read or refused like any
   field's (4), so `teamUpdate(id: AI, input: {parentId: LP})` can't make AI a
   sub-team of LP. `issueLabelCreate` with no `teamId` of AI or HOL (missing, null)
   is refused: it would create a workspace label, shared across teams. Other
   workspace-wide creates are refused too (R3, 2026-11-14), by allowlist: a
   mutation's root field ending `Create` must be in `TEAM_CREATES` (issue, issue
   batch, issue label, issue relation, comment, attachment, external link, project,
   project milestone, project update, cycle, workflow state, template, custom view);
   any other (initiative, customer, project status or label, emoji, roadmap, a
   document, ...) is refused. Access-granting creates (webhook, apiKey, oauth, invite,
   membership) keep asking on a card (6). `templateCreate` and `customViewCreate`
   also need their own top-level `input.teamId` of AI or HOL (a `teamId` nested in
   `templateData` or `filters` does not count). A `projectUpdate` whose `teamIds` is
   empty or null (inline or in a variable) is refused: it would take the project out
   of every team. **Reads are team-guarded the same way** (a query, and a
   mutation's selection set): any argument key containing `team`, case-insensitive
   (`team`, `teamId`, `accessibleTeams`), at any depth, `team(id:)` and a
   `teams(filter:)` or `administrableTeams(filter:)` (only `id` and `key`, inline or by variable) may hold only AI or
   HOL (by id or key, literal or variable) and only the operators `id`, `key`, `eq`,
   `in` and `some`; a variable filter is held to the same keys (`neq`, `nin`, `or`,
   `name` beside them are refused). A query variable whose default is a string,
   object or list must be passed explicitly (a default could stand in for a missing
   variable). A boolean in a team-named position (`includeSubTeams: true`) names no team and passes; a mutation's variables are walked for team-named keys too. LP, PRO and any other team are refused. Left open on purpose: a read
   that names no team (an unfiltered `issues`) or reads one entity by id still
   returns what the key sees; the organization check is its only limit.
4. By-id targets (mutations, 2026-11-12): an issue named by the `id` of
   `issueUpdate`, `issueArchive`, `issueUnarchive`, `issueDelete`, `issueAddLabel`,
   `issueRemoveLabel`, `issue(Un)Subscribe`, `issueReminder` and a few more (the
   `ids` of `issueBatchUpdate`), by an `issueId`, `issueIds` or `relatedIssueId`
   anywhere in the arguments or variables, or by an issue mutation's `parentId`; a
   comment (`commentUpdate`, `commentDelete`, `comment(Un)Resolve`, a `commentId`)
   or attachment (`attachmentUpdate`, `attachmentDelete`, an `attachmentId`), read
   through its issue; a workflow state (`workflowStateUpdate`,
   `workflowStateArchive`). After the organization check one `HollowTeamGuard`
   query reads every target's team (at most 50); one outside the AI team (by id),
   or not found, refuses the call before it is sent. Since 2026-11-13 the same read
   covers projects (`projectUpdate`, `projectArchive`, `projectUnarchive`,
   `projectDelete`, a `projectId`; every team the project is in must be AI or HOL,
   so one spanning LP is refused), project milestones (`projectMilestoneUpdate`,
   `projectMilestoneDelete`, a `projectMilestoneId`, read through the project), labels
   (`issueLabelUpdate`, `issueLabelDelete`, `issueLabelRetire`, `issueLabelRestore`, a
   `labelId(s)`, `added`/`removedLabelIds`, a label's `parentId`; a workspace label
   has no team and is shared across teams, so it is refused) and cycles
   (`cycleUpdate`, `cycleArchive`, a `cycleId`). Fail closed: any other `id` or `ids`
   argument, and any other key ending `Id` or `Ids` inline or in variables at any
   depth, that the guard can't read the team of (a document, a customer,
   `reactionCreate`'s `projectUpdateId`) is **refused before any card**, "Nothing was
   sent.": a person's approval can't override the team rule (until 2026-11-13 these
   asked on a card and an approved one was sent with only the readable targets
   checked). Exempt: the attribute ids in `ISSUE_ATTRIBUTE_IDS` (assignee, delegate,
   labels, cycle, subscribers, template; since 2026-11-14 not `stateId` or
   `sourceCommentId`, which have their team read: a state or comment of LP or PRO is
   refused) on an issue whose
   team is known (its `id` read by the team guard, or `issueCreate`'s `teamId` set to
   the AI team as a concrete string; null or missing establishes no team, so a
   project, label or cycle id is read and any other is refused); since 2026-11-13 an
   issue's `projectId` and `projectMilestoneId` are not exempt and have their project
   read like any target, because Linear may add the issue's team to that project.
   `PROJECT_ATTRIBUTE_IDS` (`statusId`, `labelIds`) are exempt on `projectUpdate`
   once its `id` is read and on `projectCreate` once its `teamIds` name AI or HOL as
   concrete strings: a project status and project labels are workspace settings that
   only change that project. Also exempt: people (`PERSON_IDS`: assignee, delegate, lead, members,
   subscribers), who belong to no team. A bare `id` nested inside an input (not the
   field's own `id`) is refused too, and so is a nested `ids`. If the
   `HollowTeamGuard` read itself fails, the call stops there, "Nothing was sent.". A
   reply's `parentId` (`commentCreate`) is read
   as a comment.
5. `viewer { organization { id urlKey name } }` must name Example Org's
   organization id (`00000000-0000-4000-8000-000000000001`, `graphql::ORG_ID`), asked
   before every call. Only the id is pinned (#1301, 2026-11-13): the name is
   anyone's to choose and a urlKey can be changed and then taken by another
   organization, so a key whose organization is named Example Org (or has
   urlKey `example-org`) under another id is refused. The refusal shows the
   name, id and urlKey it got. The tracker adapter's own writes (create, edit,
   comment, upload: any `mutation` through `LinearAdapter::post`) pin the same id
   once per adapter (R3, Q-R25; a new key is a new adapter), nothing written otherwise.
   `create_landing` reads an empty answer as `Absent` only when the key also sees a
   team beyond AI and HOL (`TeamsVisible`); a key limited to those two can't tell a
   lost create from one moved into a team it can't see, so the read-back errors and the
   journal stays `attempted`. This narrows the gap, it does not close it: a key that
   sees some other team, but not the one the issue was moved to, still reads the
   move as `Absent`.
6. Asks on a card in every mode, bypass included (`app_actions::asks_in_mode`;
   not in `ALWAYS_ASK`, which is per action): a mutation whose operation name or
   root field contains Delete, Archive, Remove, Trash, Retire, Suspend, Demote,
   Merge, Rotate, Revoke or Disconnect, or that sets `trashed` or `archivedAt`
   (inline or in variables); one that grants access or reaches outside the
   organization, a root field starting with organization, user, webhook, apiKey or
   oauth, or containing Invite, Integration or Membership (`organizationInviteCreate`,
   `userPromoteAdmin`, `webhookCreate` with `allPublicTeams`, `teamMembershipCreate`).
   An approved card never skips item 4: every by-id target is still read after it,
   and an unreadable one never reaches a card (so `userPromoteAdmin(id)`,
   `userUpdate(id)` or `webhookUpdate(id)`, which name no team, are refused). The card shows the query and the
   variables (800 characters each), not only the operation name, and says which of
   these it is.
7. Linear's rate limit: a 429 or a reply with no requests left closes a window
   (until Linear's reset, else 60 s) per endpoint; calls inside it are refused
   with no request.

Every call, sent or refused, is one `tracker_write` audit row (action `graphql`,
`after` = the query text and the root fields, then for a call that passed the local
checks the kind, operation name and the variables). The query text is rebuilt from
the parsed tokens (comments and commas dropped), so it keeps inline arguments (an
issue id, a team) while every value under a secret-named argument, input key or
variable default (string, list or object) is masked, then the key, `lin_*` tokens
and the redactor's token shapes are removed, at most 2,000 characters. A document
that doesn't parse keeps no text. The variables have secret-named fields masked. `tracker.audit`
reads it. The reply is `{data, errors, rateLimit}`: `data` has secret-named fields
masked, an error's message is kept (scrubbed, 600 characters) so a bad field can
be fixed.

Example, a project status named Paused (type `paused`, a workspace setting):

```graphql
mutation CreatePausedStatus($input: ProjectStatusCreateInput!) {
  projectStatusCreate(input: $input) { success status { id name type } }
}
```
variables `{"input": {"name": "Paused", "type": "paused", "color": "#95a2b3", "position": 3}}`.
