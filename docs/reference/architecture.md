# Architecture

Part of the agent manual. [AGENTS.md](../../AGENTS.md) is the entry point and
links every reference doc; this one holds the subsections of CLAUDE.md's
"Architecture" and its "Microsoft 365" section, moved there word for word on
2026-10-13 (only relative links were re-based). Read it before adding a lock,
a command, a window or a tray item, or when a change touches startup, IPC or
performance. Keep it current with the code in the same change, as AGENTS.md
asks.

These subsections sat under CLAUDE.md's `## Architecture`, whose two-process
overview moved here on 2026-11-21 and comes first, under
[Two-process overview](#two-process-overview).
The backend module map, which sat between "Adding a new command" and
"AppState", is in [its own doc](backend-module-map.md).

## Two-process overview

Two-process model under one binary:

```
┌──────────────────────────── Hollow.app ─────────────────────────────┐
│                                                                     │
│   WebView (Solid+TS)                Rust shell (Tauri 2)            │
│   ─────────────────                ───────────────────              │
│   src/store/* ←──── invoke() ───→  src-tauri/src/commands*          │
│   src/api.ts                       AppState (Arc<Config>)           │
│   src/pages/*.tsx                                                   │
│   src/components/*.tsx                                              │
│                       events       ┌── Vault FS (tokio::fs)         │
│                ←─── chat-event ──  │── `claude` subprocess (stream) │
│                                    │── `claude -p` one-shot         │
│                                    └── on-disk JSON caches          │
└─────────────────────────────────────────────────────────────────────┘
```

- **Frontend** talks to the shell **only** through `invoke()` — there is no
  HTTP server, no WebSocket. [`src/api.ts`](../../src/api.ts) owns mock routing and IPC
  tallying. Components use its typed `api` or a lazy typed API module (for example
  `canvas/itemsApi.ts`), never call `invoke()` directly.
- **Backend** owns all I/O: filesystem, the tracker API,
  and the agent CLIs. The renderer is pure UI.
- **Streaming** (long-running chat) goes the other direction: Rust emits
  `chat-event` Tauri events (typed in `chat_event.rs`, built by `chat_event::build`); [`store/chatRuntime.ts`](../../src/store/chatRuntime.ts)
  (re-exported by `store/runtime.ts`) has one listener (module init) feeding the
  `chatTurns` store (survives nav-away); subagent events add
  `parentToolUseId` ([AG7](frontend-state.md)); panels use the one
  `panel` event (AG6: `{sessionKey, panel, snapshot}`, replace-only).

Lock order, performance profile, the IPC contract, command registration,
AppState, windows, tray and Microsoft 365 are in
[Architecture](architecture.md); every backend module is in
the [backend module map](backend-module-map.md), every frontend
module in [docs/FRONTEND_MODULES.md](../FRONTEND_MODULES.md).

### Lock order (merged plan slice 4)

The full list, with what each lock guards, is the module doc of
[`src-tauri/src/lock_order.rs`](../../src-tauri/src/lock_order.rs); a PR that adds
a lock, or nests two that were never nested, updates both. While holding a
lock, take only locks from a later tier or sub-tier (4a before 4b, 5a before
5b before 5c), never the same or an earlier one; two locks in one tier or
sub-tier are never held together. The std mutexes are not reentrant. **Tier
1 is the exception:** its locks are never waited on without a bound (the
lease is a try-lock, the run queue owner flock is tried once, a permit polls
within a bounded wait and then refuses), so they may be held together (the
run queue owner flock is held for the life of the process, alongside every
other lock) and a tier 2 sync holder may take the background write lease.
Never make one of them an unbounded wait.

1. Admission, cross-process, refused rather than waited on: the full-vault
   run lease (lease file + `Leases/{vault}.lock` flock), the run queue owner
   flock (`run_pump`), the run permits (`run_permit` flock slots).
2. Sync: `tracker_sync::root_lock(root)` (one per vault root) → the
   `.tracker-sync.lock` flock → the background write lease (tier 1, a
   try-lock) → vault writers (Notion two-way sync and its locks were
   removed on 2026-11-10). Also before
   `HISTORY_LOCK`, never under it: `directory_connections::UPDATE` (the COO's
   `workspaces.open_project_workspace`, then its `with_live_run` write) and
   `companion_inbox::PASS` (an inbox pass's Apply, `companion.revoke_device`;
   `DEVICES` is a leaf inside).
3. `automated_history::HISTORY_LOCK` (`with_live_run` closures may take
   `CREATE_LOCK` and tiers 5-7, never the write guard). Since SE5
   (2026-11-17) it is a writer-preferring shared/exclusive lock: a scoped
   run's admission (`begin_scoped`: register, walk, write the record) holds
   it shared, so chats starting together walk their folders at once;
   everything else holds it exclusively and so still sees each admission
   whole. A shared holder must never ask for it exclusively (`host_write`).
   The stat cache's load and save take `STAT_CACHE_IO` (its save counter
   `STAT_CACHE_SAVES` nested inside, a leaf), and a save merges into the
   file as it is now. Right under it,
   History's object store flock (`Leases/{vault}.objects.lock`, Q-R9
   2026-11-13): shared by a write from before its snapshot until its record
   names the objects (a bounded wait, then the write refuses; it never goes
   ahead without it), exclusive for object cleanup (a 250 ms try, else the
   round is skipped; retention deletes only in the first 20 s of its hold).
4. 4a `workflow_permissions::write_guard`; 4b `vault_goals::GOALS_LOCK` or
   `vault_tasks::CREATE_LOCK`, never both (`CREATE_LOCK` never under the
   mutate lock).
5. 5a the vault mutate lock, `vault::lock_mutate()`: `MUTATE_LOCK` plus a
   cross-process flock on `Leases/vault-mutate.lock`; 5b `BUDGETS_LOCK` or
   the stamp `STORE_LOCK`; 5c the review `QUEUE_LOCK`. The mutate lock is
   released before 5b or 5c is taken; budgets nests `BUDGETS_LOCK` →
   `QUEUE_LOCK`.
6. Derived-index leaves: link `INDEX`, walk cache, search and embed indexes.
7. `run_guard::TRACKED` / `OWNED_QUIET`, innermost.

Separate chain: `CHAT_ROW_ACTIVE` → `session_store::WRITE_LOCK` and
`run_queue::WAITERS` → `WRITE_LOCK`; `WRITE_LOCK` is a leaf and is never held
while taking the lease, `HISTORY_LOCK`, the write guard or the queue lock.
Every other module mutex is a leaf.
The update client's "Update when chats finish" ticket is a leaf: Cancel and
install race through one atomic transition; a successful Cancel can never start
installation (the old scheduled-install slot went with row U10).
The quit-request mutex is a leaf too. Queue/registry reads run off the event-loop
thread, and window operations happen after releasing the request lock.

**Cross-process limit.** Only the flocks, the O_EXCL lease file and SQLite
reach other processes (the headless `hollow notion-sync` CLI that shared
them went with Notion sync on 2026-11-10). The mutate flock waits at
most `MUTATE_PROCESS_WAIT` (2 s); past that Hollow logs and continues under
the in-process lock alone, so a hung process can't freeze the UI or
deadlock. Obsidian and an agent's own file tools take no Hollow lock; only
compare-and-swap and the no-replace renames protect against them.

### Performance profile

The 0.5.0 startup now warms only provider discovery, and primary pages skip
entrance animation. Library and workspace text reads are bounded to 1 MB;
workspace core prompt files total at most 48 KB, plus bounded source text (24 KB of query-centred chunks; each turn reads at most 1 MB per attached file and 16 MB in all to choose them; see [context limits](../SOURCE_CONTEXT.md)); past a limit the context is trimmed and says so, never an error that blocks the turn. `read_workspace` / `WorkspaceDetail` carries `contextHealth: [{name, bytes, included, limit}]` (durability D3: from file sizes, same `context_shares` arithmetic, no reads; empty for Chats), and the agent-facing data remains available after the old composer explanation panel's removal. The ring shows recorded message shares ([Usage](../USAGE_INSTRUMENTS.md)). These changes have functional
coverage; native latency and memory targets have not been measured.

Normal file updates use a debounced OS watcher and incremental caches. The
heavier paths are semantic backfill and queries (the model loads on demand
and is evicted after five minutes unused), keyword search (it scores every
FTS5 prefilter candidate), per-request retrieval (each chat or workflow scans
the vault once inside the run lease: the index's BM25 candidates are read, and
up to 32 MB of matching notes are scored, see `retrieval_broker.rs`) and agent subprocesses.
Measure the whole process group, not the main process alone.

**Launch weight (overnight P3, 2026-10-12).** The App chunk is capped at 340 KB
(330 until consolidation D/E raised it, 2026-10-13; 338.6 → 320.8 KB in P3): the confirm dialog mounts at the first confirm (`OnceOpened`
on `confirmDialog()`, preloaded when idle), the activity dock loads when it first
shows (lazy in TopBar), and the launch path imports `streakAction.ts` and
`taskLocks.ts` instead of `workflowHealth.ts` and `taskEdit.ts`. Keep it so:
put a launch-path helper in a small module, not in a page's model file.
The chat runtime loads the runner's lines (`remoteRunNotice.ts`, about 7 KB) on
first use (overnight 11-07: the chunk was at 339.6 KB); a `remoteRun` line is
pushed in stream order and gets its text when they land.
Work imports `workspaceShell.ts` for its result filter and brief prompt, keeping
the other `workspaceRecap.ts` calculations in lazy section chunks. The original
module re-exports both helpers; their behavior and request timing stay unchanged.
ST1 (plan completion night, 2026-10-14) took the App chunk from 330.4 to
302.6 KB and the launch set from 745.6 to 709.2 KB without raising a budget: the
side panel was its own chunk, fetched when `pages/Work.tsx` mounted (since run
15 P1 the Preview pane, `components/PreviewPane.tsx`, is a lazy chunk fetched
on the first open); Home's Due and Last night cards load lazily when
Home mounts (no Suspense boundary); the top bar
imports `runQueueLive.ts`, not `runQueue.ts`; and the batch-import runner
moved from the rail's import group (at launch) to the Imports page (both
removed with From other apps in run 15 P4). Launch
IPC is budgeted by `tests/smoke/launch-ipc.spec.ts` (32 calls on the plain
mock, 43 on `demo=large`; raise it there on purpose, with the reason).

**Profiling night 11-20.** Measured on `61f56db9c` (browser mock, Chromium; Rust test profile): cold Home 127 ms and
shell 95 ms, 45 launch IPC calls, Tasks switch 283 ms (every one of 5,000 rows in the DOM), a 1,000-turn thread loaded back to
turn 0 at 21,132 DOM nodes and +59 MB heap, `read_agent_conversation` 338 KB and `tasks` 295 KB over IPC (no Rust path over
22 ms). `perf_hot_paths_report` now prints serialized payload bytes beside time; `scripts/perf-transcript.mjs` holds the
transcript budget. Full tables: the `profile.md` of that night's workspace outputs.

**Measuring and guards (five asks B4, 2026-10-10).** `scripts/perf-report.mjs`
(bundle + launch set), `scripts/perf-mock-budgets.mjs` (browser mock),
`scripts/perf-switch.mjs` (ST2: cold start to an interactive Home and the
Home / Today / Tasks / Review / 1,000-turn-chat switches as p50, p90 and n,
each click timed inside the page), `scripts/perf-style-trace.mjs` (overnight 10-15/16 A13: a
style-invalidation trace of Home → Tasks, ranked invalidations and every recalc's element count; it
showed about half that switch is the page transition's capture, `MEASURES.md` "Recheck 2026-10-16") and
`src-tauri/src/perf_hot_paths.rs` (IPC hot paths over a synthetic vault at the
owner's scale: `TMPDIR=/tmp cargo test --lib perf_hot_paths_report -- --ignored
--nocapture` prints them, wall and thread CPU time; `hot_paths_stay_under_their_ceilings` fails the gate
at roughly 5–20× the measured test-profile times, judged on the calling thread's CPU time (overnight 10-16/17
OF10: wall time over with CPU under is load, printed as a note; a path that hands work to other threads would be
listed in `WALL_JUDGED` and judged on wall time), re-timing only the paths over their ceiling up to
three tries (best median kept) so one slow sample on a loaded machine doesn't fail it; the receipts
parse-count test is the exact guard). `src-tauri/src/turn_prep_probe.rs` (run 15 S0/S1) times a chat
turn's retrieval over an owner-shaped vault and Projects folder (42,200 listed files, the newest ~32 MB
read per request): `cargo test --lib turn_prep_probe -- --ignored --nocapture`, before (line-by-line
scorer) and after side by side, failing if the evidence differs; add `--config profile.test.opt-level=2`
for release-like numbers. What keeps them fast:
- Retrieval scores lines with one `vault_search::LineScorer` per request (run 15 S1): each term's
  `memchr` finder is built once and each file's body is lowercased and searched once per term, instead of
  every line lowercased and searched once per term; the answer is exactly the line-by-line reference's
  (`line_scorer_matches_the_line_by_line_reference`).
- Two per-row helpers no longer scan or format per call (ST2, 2026-10-14): `workspaceOrigin.ts`
  `distinguishing` reads a title index built once per workspace list (a
  `WeakMap` keyed by the array, so `store/workspaces.ts` `projectWorkspaces`
  returns the same array until the list changes), and `studio.ts` `studioDate`
  keeps one `Intl.DateTimeFormat` (since a later 10-14 change shared, through
  `monthDay.ts`, with `dueLabel`, the sidebar's and Home's "when"; it
  keeps the time zone of its first use until a reload). On the scale fixture (Chromium, mock) that
  took cold start to an interactive Home from 262.8 to 164.5 ms p50 and Chat →
  Home from 80.5 to 40.4 ms, and removed the long task on every Home and chat switch.
- `session_store::open` applies its schema once per database file per process
  (`SCHEMA_READY`, keyed by path, device and inode; a recreated file gets it
  again). Add new DDL to `apply_schema`, never after the early return.
- `session_store::search_sessions` (the palette's conversation search; since
  2026-11-04 by title or the person's own messages: not a request another
  agent sent, one Hollow wrote for them, or a "⟲ Hollow" wake, check-in,
  night-queue or runbook message) filters in SQL: one condition per query
  word for the first 8 of at most 32 (the title, else an `EXISTS` over the
  conversation's `turns.asked`), reading unarchived conversations newest
  first from `idx_sessions_recent` (`archived, last_activity_at DESC, key`),
  so a page stops at its last row, and seeking past the cursor's keyset.
  Rust checks each returned row again, every word, folding titles fully, and
  gives a hit by text its snippet (`MessageText::Redacted` for the agent
  action: a word only a secret holds is no match, a placeholder holds none).
  `lower()` folds ASCII letters only, so each word is also tried as typed
  and, when it opens with a letter outside ASCII, capitalized: "GRÖSSE" in a
  message is found by "GRÖSSE", not by "grösse". A query nothing holds still
  reads every message of the person's (`search_sessions(plan)` in
  `perf_hot_paths.rs`, over about 2.2 MB of messages).
  Since 2026-11-08 the ranked search (`search_sessions_ranked`) reads each
  priority with its own query (its folders, the cursor, `LIMIT`) and stops
  at a page plus one; see [chat search](../CHAT_SEARCH.md).
- `evidence.rs` keeps the parsed `.evidence-receipts.json` per vault
  (`STORE_CACHE`, keyed by length, mtime and inode); readers scan it through
  `with_receipts` without copying, writers read the file fresh.
- `workspaces::list` reads the build bindings once, not per workspace, and
  each brief's frontmatter comes from `BRIEF_CACHE` (keyed by path; reused
  for up to 60 s while length, mtime, ctime and inode hold, so even an
  in-place write that puts the mtime back is seen; overnight Y7).
- `workspaces::list_with_dirs` resolves `workspaces/` once: a `read_dir`
  directory entry is never a symlink, so its canonical folder is that folder
  plus its name, and only a symlinked or missing brief goes through
  `vault::safe_join` (two `realpath`s a call). The rail's conversation groups
  and `SessionHomes` (exact-key opens, palette search) use it, and match a
  session cwd by its literal path before canonicalizing it. Held by
  `listing_resolves_paths_a_fixed_number_of_times` (a `SAFE_JOINS` count, not a
  timing; overnight 10-11/12 P5).
- `vault_tasks::list_parsed` reuses each task file's parse from `TASK_CACHE`
  (`file_stamp::StampCache`: the brief cache's stamp rule and 60 s age, for
  any per-file parse that depends only on path and bytes; P5).
- Sync `#[tauri::command]`s run on the **main thread** and freeze the window
  while they work. Store and vault *reads* are `#[tauri::command(async)]`
  (the session reads in `commands/sessions_api.rs`, `list_sessions`,
  `read_session`, `vault_note`, `vault_backlinks`, `list_history_runs`); an
  async command that borrows `State` must return `Result`. That runs the body
  on an async worker, which chat streaming shares, so the whole-vault walks
  (`vault_tree`, `vault_items`, `vault_notes_list`) are `async fn`s that hand
  the walk to `spawn_blocking`; the broker calls their `*_now` twins. Writes
  stay sync (serialized). Window, menu and tray commands must stay on the
  main thread.
- The schema-once cache only remembers a file once `journal_mode` reports
  `wal`, so a first open that lost the WAL race tries again next time. The
  receipts cache treats a missing file as empty and any other stat error as
  an error. A turn's end and the build panel's commit, push and PR ask for git
  state unthrottled. The backend caches each worktree's rail state for 120 s
  only while its `git_stamp` holds (the worktree's `HEAD`, its reflog, the
  checked-out branch's ref, `packed-refs`, `reftable/tables.list` and the
  base's loose ref, read without spawning git), and
  `build_changes::commit`/`push`/`open_pr` drop the entry (`forget_rail_state`,
  which bumps a per-workspace generation so a read begun earlier isn't kept);
  `build_rail_states`' optional `fresh` ids (the Changes panel's workspace
  after commit/push/PR, the workspace whose foreground or background turn just ended) are read again
  regardless (overnight Y7/B7). Background completion uses the retained conversation cwd, never the page on screen; returning a running chat to the foreground does not count as completion. Fresh invalidations bypass the frontend throttle and survive overlapping/failed reads until the latest successful response applies them. The rail itself still asks every minute.
- `src/launchIpc.ts` tallies every `api.ts` call for the first 15 s of a
  window; a call repeated with the same arguments more than twice is over
  budget (Settings → Advanced → Launch calls, one console warning). Credential-bearing keys and whole env/header maps are masked before tally storage; credential-only differences intentionally share a bucket. The total
  has no budget yet: set one from a native reading.
- `refreshWorkspaces` (`src/store/workspaces.ts`) runs one chain at a time:
  mounts and the minute timer pass `{ join: true }`; any other call gets one
  refresh that starts after the running one. `refreshRailGitStates(maxAgeMs)`
  skips an ask for the same build workspaces within the window (20 s from the
  chain, 50 s from the rail's timer): each ask spawns git and `gh`.

### IPC contract

Rust types in [`src-tauri/src/types.rs`](../../src-tauri/src/types.rs) use
`#[serde(rename_all = "camelCase")]`. A drifted shape deserializes as
`undefined`/missing fields in the WebView and you'll spend an hour debugging
the wrong thing, so the main responses are now generated rather than copied
(merged plan slice 6 item 7):

- **Generated (`src/generated/ipc.ts`, never edit).** Each page's main
  response derives `ts_rs::TS` under `#[cfg_attr(test, …)]` (ts-rs is a
  dev-dependency; nothing ships): Work `list_workspaces` (`WorkWorkspace` =
  Rust `workspaces::Workspace`, `BuildInfo`), a workspace's panel
  `read_workspace` (`WorkspaceDetail`, `WorkFile`, `SourceContext`,
  `ContextDocHealth`, `WorkspaceReentry`, `ReentryLastRun` = Rust
  `workspace_reentry::LastRun`, `BriefStale`; fixture `workspace.json`), Your
  work `workspace_digests` (`WorkspaceDigest`, `LatestResult`; fixture
  `digests.json`), conversations (`AgentSessionSummary`, `HarnessStint`; its
  `resumable` is the TS-only `SessionResumability`), the rail tree
  `workspace_sessions` (`WorkspaceSessions`, `SessionChanges`, `StartedBy`;
  fixture `workspace-sessions.json`), one conversation `read_agent_session`
  (`AgentSessionDetail`, `SessionRecordTurn`; fixture `session-detail.json`),
  a chat send `chat_send` (`SendAccepted`, its `phase` the TS-only
  `SessionPhase` = Rust `agent_sessions::SessionPhase`), the run queue
  `list_run_queue` (`RunQueueItem`, its `source`/`status` the TS-only
  `RunQueueSource`/`RunQueueStatus`; fixture `run-queue.json`) and its
  `run-queue-event` payload (`RunQueueEvent`, its `kind` the TS-only
  `RunQueueEventKind` = `run_queue::EVENT_KINDS`), History & recovery
  `list_history_runs` / `get_history_run` / the restores (`HistoryRun`,
  `HistoryChange`, `HistoryCoverage` = Rust `automated_history::Coverage`,
  `HistoryUncapturedReason` = Rust `UncapturedReason`, `RunFlag`, its `kind`
  the TS-only `RunFlagKind` = `run_guard::FLAG_*`; fixture `history.json`) and
  `history_storage_status` (`HistoryStorageStatus`), Tasks `tasks` (`TasksResponse`,
  `Task`, `SubTask`, `TrackerLock`, `BlockerWarning`), Library
  `list_all_results` (`LibraryResult`), Results `workspace_results` /
  `set_result_review` (`WorkResult`, `ReviewEvent`, `ResultAcceptance` = Rust
  `Acceptance`, `ResultOrigin`, `ResultChecks`, `ResultCanvas`), Automations `list_workflows`
  (`Workflow`), Settings `get_agent_settings` (`AgentSettings`; its `canvases` is the
  TS-only `CanvasFrequency`), the
  needs-you query `list_needs_you` (`NeedsYouPage`, `NeedsYouItem`) and the
  canvas store (`CanvasRecorded`, `Canvas`, `CanvasRevision`, `CanvasSummary`,
  `CanvasStateSaved`, `CanvasRevisionHtml`; fixture `canvas.json`), a run's context manifest `run_context_manifest`
  (`RunContextView`, `RunManifest`, the `Manifest*` parts and the connector
  `ConnectorSelection`/`ConnectorDecision`/`ConnectorBasis`) and the
  Plan row toggle `toggle_plan_row` (`PlanRowToggled`) and the Activity
  timeline `workspace_activity` (`WorkspaceActivity`, its `ActivityEvent`s
  and `ActivityGaps`; fixture `activity.json`).
  `workspaceTypes.ts` and `types.ts` re-export them under the old names.
  `src-tauri/src/ts_bindings.rs` lists the roots; every type they reach is
  emitted, sorted, one field per line (a union, such as an internally tagged
  enum, stays on one line), with field docs left on the Rust side.
  ts-rs follows the serde attributes; two things it can't infer need a
  `#[cfg_attr(test, ts(…))]` on the field: `ts(optional)` on an `Option`
  with `skip_serializing_if` (else it types it `field: T | null`, or
  `field?: T | null` when it also has `default`, instead of `field?: T`) and
  `ts(as = "Option<…>", optional)` on a `Vec` or `bool` with
  `skip_serializing_if` and no `default` (with `default` ts-rs already writes `field?: T`; plain `ts(optional)` doesn't compile on a
  non-`Option`), and `ts(as = "crate::ts_bindings::…")` or
  `ts(type = "…")` for a `String` holding a fixed vocabulary the frontend
  narrows to a union (the TS-only enums in `ts_bindings.rs`; Rust still
  writes the strings, so keep the enum and the producing code in step).
- **Drift check.** `ts_bindings::generated_types_are_current` (in
  `cargo test --lib`, so in the merge gate) regenerates the file and fails on
  any difference. After changing one of these types, from `src-tauri/` run
  `HOLLOW_REGENERATE_IPC=1 cargo test --lib ts_bindings` and commit
  `src/generated/ipc.ts` with it.
- **Round-trip fixtures.** The same module serializes a representative value
  per page to `tests/fixtures/ipc/<page>.json` (checked and regenerated the
  same way), and `src/ipcFixtures.test.ts` parses each with the generated type
  and asserts serde's keys are exactly the type's keys (a compile-time list
  check plus a runtime one), so ts-rs and serde can't disagree unnoticed.
- **Still hand-written** (`src/types.ts`, `workspaceTypes.ts` and the other
  `*Types.ts`): every other response, every command argument/input type, and
  event payloads other than `run-queue-event`. Update both sides by hand for those, or better, add the
  struct as a root in `ts_bindings.rs` and switch its import. Next candidates:
  the receipt types and the
  agenda/recordings types.

### Command registration is centralized

Every `#[tauri::command]` lives in [`commands.rs`](../../src-tauri/src/commands.rs)
or a focused adapter under `src-tauri/src/commands/` and MUST also appear by
its defining module path in the `generate_handler!`
macro in [`lib.rs`](../../src-tauri/src/lib.rs)
(the block in `tauri::Builder::default().invoke_handler(...)`). A command that
compiles but isn't registered will throw **"command not found"** at runtime —
the most common foot-gun in this codebase.

### Adding a new command (walkthrough)

1. Define the request/response types in `types.rs` (Rust side, camelCase serde).
   For a response, prefer adding it as a root in `ts_bindings.rs` and
   regenerating `src/generated/ipc.ts` (see IPC contract); otherwise mirror it
   in `src/types.ts`.
2. Write the `#[tauri::command] pub async fn …` in the nearest focused
   `commands/` adapter and delegate real I/O to its domain module.
3. Add the function to the `generate_handler!` list in `lib.rs`.
4. Add a typed wrapper to `src/api.ts`:
   `myCommand: (arg: Arg) => invoke<Result>('my_command', { arg })`.
5. In the calling page/component, wrap the call in `trackTask()` from
   `store.ts` so the ActivityDock surfaces it.
6. `cargo check` (Rust) and `tsc --noEmit` (frontend) before running.

### AppState

[`commands.rs`](../../src-tauri/src/commands.rs) defines `AppState`:

- `Arc<Config>` — parsed config, immutable for the app lifetime.
  Vault file writes need no lock — the OS provides atomic-enough
  semantics, and `vault_tasks::upsert` + `vault_projects::set_status`
  are collision-safe by design.
- `config_path` (the file actually loaded), `configured` (false in setup mode)
  and `config_error` (why loading failed, shown by the wizard).

The chat send task owns its child through `process_group::drive`; `chat.rs`
keeps only the `CURRENT_STOP` stop slots, and ingest its own `INGEST_STOP`.

### Core host

Core services take a host, not the GUI's `AppHandle`, so they can run with
no window server (M1, `hollow serve`, from PR 2). [`core_host.rs`](../../src-tauri/src/core_host.rs)
has the trait `CoreHost` (emit to all, emit to one window, the managed
`AppState`, the main window's focus and visibility, a native banner, and
`gui()` for a callee not moved yet) and `Core`, an `Arc<dyn CoreHost>`. The
GUI is one host: `AppHandle` implements it with Tauri's own calls, and emits
are byte-identical (`emit_str` with the payload `serde_json::to_string`
makes, as `emit` does). The rules: a long-lived service holds a `Core`, a
small helper takes `&impl crate::core_host::CoreHost`, an owned entry point
takes `impl Into<Core>` so setup passes `app.handle().clone()` unchanged; never
import `CoreHost` at module level (it would make Tauri's `emit` ambiguous). On
it since PR 1: the run queue pump, chat turns and titles, held and phone
sends (`commands::send_chat_on`), workflows, the heartbeat, the COO with
wake-ups, pressure tests and the disk check, notifications (the registry
holds a `Core`; `registered_app()` is its GUI handle), the phone API's
service and the self-hosted jobs' helper. Since PR 2 also the agent tool
sockets (approvals, the app tools, task and services brokers), the companion
inbox and bridge, directory connections, tracker sync, the vault watcher,
nudges and "Keep learning". Still on `AppHandle`: the schedule window jobs
(`schedule_windows::JOBS`), learning capture after a turn (`chat.rs`, marked
`M1 PR2: migrate`) and the four `registered_app()` callers, which skip their
emit in serve.

### Core startup and `hollow serve`

[`core_boot.rs`](../../src-tauri/src/core_boot.rs) holds the core's startup in
three phases, each taking a `Core`: `process_services` (crash reaper, login
shell PATH, org context pull, effort catalogs, the approvals socket),
`account_services` (quota and accounts registration, CLI updates, install
results, the scratch sweep) and `vault_services` (everything on the vault, in
the order setup always had: notifications, usage ledger root, the brokers, the
queue owner lock, the reconciles, the pump, the scheduler, tracker sync, the
watcher, nudges, the phone API, the heartbeat, self-hosted jobs, the COO). The
GUI's `.setup` runs the boot sweep and its quit-signal watcher, then the three
phases with its own steps between them (global shortcuts allowed, the login
item, menu, tray, icons, shortcuts, the show thread); a source test
(`core_boot_tests.rs`) pins both hosts' order. The schedule window timer starts
only where `gui()` is the GUI's handle (`SKIPPED_IN_SERVE`).

[`serve.rs`](../../src-tauri/src/serve.rs) is `hollow serve [--data-dir <dir>]
[--config <file>] [--phone-port <port>] [--log-events]`: the same phases on a plain Tokio
runtime (`tauri::async_runtime::set`), with no `tauri::Builder`, no event loop
and no window, tray, menu, shortcut, autostart, window state or dialog plugin.
Its `ServeHost` owns an `AppState` built by `core_boot::load_state` (the app's
config lookup: `--config`/`HOLLOW_CONFIG`, then the usual places); events are
dropped (`--log-events` or `HOLLOW_SERVE_LOG_EVENTS=1` logs each by name and
size; there is no serve config file: the flags go in the LaunchDaemon's
arguments) and a banner
is a stderr line. There is no setup mode: a `--config`/`HOLLOW_CONFIG` file that isn't there
(never the usual lookup, which would fall back to the working folder), no usable config.json, or a vault
that is not a folder, exits 78. Startup:

- **One Hollow per data folder.** Serve takes the app's own `gui.lock`
  (`launch.rs`) without waiting and writes `serve.json` (its pid) beside it,
  removed at exit. Held lock: exit 75. An app launched while serve runs finds
  the lock held and the marker's pid alive, prints that `hollow serve` runs the
  core and exits 2 instead of `open -b` (which would only launch it again). An
  app that does take the lock removes any `serve.json` (a crashed or killed
  serve's), so a reused pid can't block a later hand-off. A vault whose run
  queue lock another process holds also exits 75 before anything starts, and
  so does a core that ends up not owning its queue after the vault phase.
- **`--data-dir` isolates.** It sets `HOLLOW_APP_DATA_DIR` and
  `instance::isolate_under`, which moves `support_dir()` to `<dir>/Support`
  and the cache roots to `<dir>/Caches`, so the Sessions DB, `phone.json`,
  locks and agent sockets are serve's own (the app never calls it, so it is
  unchanged). Because its queue lock is then in its own folder too, serve also
  holds the vault's queue lock in the app's support folder for its life
  (`hold_app_queue_lock`): the app on the same vault finds its queue owned
  elsewhere, and serve refuses a vault the app runs (75).
- **Lifecycle.** SIGTERM or SIGINT settles every live run through the app's
  `stop_children_once` (`orphans::settle_on_signal`), shuts the runtime down,
  only then lets go of the queue owner lock (so the pump claims nothing after
  the settle), and exits 0; a second signal while settling exits 128 + signal.
  Runtime start failures exit 1, bad arguments 2. Everything goes to stderr
  (launchd captures it); `[serve] ready: ...` marks the end of startup. A
  notification logs its title only (`--log-events` adds the body).
- **What fails closed.** The approvals socket answers every request at once
  with a refusal (`approvals::HEADLESS_REFUSAL`), so a confirm-gated Claude
  Code tool fails with a reason instead of a two-minute wait. TODO(M2): route
  cards to a phone or peer.
- **Hollow app tools (M1 PR 5, `serve_actions.rs`).** Handlers read their
  state through `app_actions::Host::app_state`, a `tauri::State`, which only a
  Tauri `Manager` hands out. So `ServeHost` keeps its `AppState` in a
  `StateShell`: a mock-runtime app (tauri's `test` feature, in the release
  build since this PR; it adds the `tauri::test` module and no dependencies)
  that is only a container for the state, with no window, webview or event
  loop, built only in `serve_actions.rs` (pinned). `CoreHost::action_host`
  (None by default) gives the broker a `ServeActionHost` (no GUI handle,
  `Host::core` = serve's `Core`); `handle_conn` takes the GUI path when
  `gui()` is Some, serve's when `action_host()` is, and a bare host still gets
  `HEADLESS_APP_TOOLS`. Serve's `dispatch` runs the broker's own checks with
  `SERVE_POLICY` (`screen`, `reword`) and a `ServeAsker`:
  - **Runs:** Reads of the areas the COO works in (sessions, tasks,
    workspaces, vault, automations, notifications, history, usage, goals,
    commitments, interactions, review, learning, agenda, tracker) plus
    `settings.get` (UI-owned keys come back `unavailable`),
    `settings.get_main_agent_settings` and `build.overview`; the writes in
    `WRITE_IDS`, which hold every write the unattended COO may make
    (`main_agent::unattended::SAFE`, pinned) and a few more. Writes keep the
    live-run lease and the unattended gate. 124 reads and 31 writes on
    2026-11-15.
  - **Fails closed:** every Confirm and External action, in every mode:
    `ServeAsker::standing_approvals` is false, so bypass, auto, an automation's
    stamped tools and an unattended COO's safe class never stand in for the
    card, and its `FailClosed` `CardRoute` shows none. M2 adds a live route
    (the person's Mac, a Needs-you item); the broker and the actions don't
    change. Also refused: the `tracker.*` write actions (and the other
    `TRACKER_WRITES`) until cards reach the person. That covers only those
    actions: with Settings' "Track agent work" on, a `sessions.start` or
    `sessions.send` that serve runs still queues the chat's Linear issue and
    its Started and Outcome comments in the work outbox (`tracker::work`,
    `tracker::work_outbox`), and serve runs `tracker_sync`, which sends them.
    Also refused: the `SERVE_REFUSED` ids (window, webview, Dock, Finder, browser
    sign-in, login Keychain, quit or replace Hollow); any other action ("isn't
    available on this runner yet"). A handler's "needs the running app" reads
    "needs Hollow's window" in serve (`reword`).
  - Every outcome is audited in `.app-actions.jsonl` as in the app (a refusal:
    `ok: false`, `card: null`). A drift test in `serve_actions_tests.rs`
    fails when an action serve runs calls `cx.app(..)` or a UI setting.
    `sessions.stop` and `notifications.create_notification` take
    `Cx::core` (the same core functions their commands call), so they run
    in serve.
  - **Limits.** A handler runs on its connection's thread with no timeout,
    as in the app. There is no per-handler panic isolation, and the release
    profile is `panic = "abort"`, so a panic in any of the actions serve runs
    aborts the whole daemon: launchd restarts it (`KeepAlive`
    `SuccessfulExit` false) and the boot sweep and reaper settle the runs it
    left.
- **Phone API.** Starts as in the app: when `<support>/Companion/phone.json`
  says `enabled`. `--phone-port` moves its loopback port (default 47680) for a
  serve beside this Mac's own Hollow; the app never sets it. With no window
  (`gui()` is `None`) its `WindowModes` is `without_window()`: settled from the
  start, so nothing ever emits `phone-access-modes-wanted` or waits
  `WINDOW_WAIT`; a phone's new chat takes the record's `newChatMode`/
  `newChatAgent` read at each send, else Plan (`HEADLESS_DEFAULT_MODE`); a
  chat's own mode is the record's `chatModes` as in the app, and a chat with
  neither a choice nor a recorded mode takes the stored default or Plan
  (`routes::chosen_mode`; a choice the window doesn't show counts as none),
  never the window's Edits-only fallback. The GUI keeps its window round.
  Turns serve starts itself with no mode to go on (a wake, a check-in, a
  routed send with none recorded, a workflow with no `permission_mode`, the
  last resort in `chat.rs`) take `app_mcp::unrecorded_mode`: `acceptEdits` in
  the app, in serve the stored default capped at `acceptEdits`, else Plan.
  `--data-dir`'s folder is set to 0700.
- **Secrets (M1 PR 3).** Serve installs `secret_store::SecretFile` before
  anything reads a secret: the same one-bundle JSON as the Keychain item, in
  `<app data>/secrets.json`, 0600, written whole through a temp file and a
  rename, every read and change re-read from disk under a `flock` on
  `secrets.lock` (so `secret-set` and a running serve never lose each other's
  keys), the never-replace-an-unreadable-bundle rule, and no retry wait: a
  file prompts nobody, so a failed read is tried again at the next use (a
  `chmod` or `secret-set` fixes it at once). A file
  not owned by the user, or readable by group or others, is refused. No legacy
  import, and `read_item` finds nothing. `claude_quota`'s `security
  find-generic-password` read is skipped (`serve::serving()`), so the usage
  card stays hidden. Threat model: the serving user and root (so any admin
  with sudo) can read it, as they can that user's whole home; it
  is never in the vault, a log, an event or argv. The app keeps the Keychain.
- **Commands** ([`serve_control.rs`](../../src-tauri/src/serve_control.rs),
  each taking `--data-dir`): `secret-set <key>` (value from stdin, a no-echo
  prompt at a terminal that refuses when echo can't be turned off and puts
  echo back on Ctrl-C; a value on the command line is refused; a
  `claude:<id>` token is shape-checked as Settings does), `secret-list` (key
  names only), `secret-delete <key>`, `pair [--machine] [--json]` (opens a
  pairing window in the running serve; the code goes to stdout once, serve
  logs only that a window opened; `--machine` opens the separate machine
  window for another Mac, listening first if the iPhone app is off, and
  `--json` prints one line with the code and, for a machine, this Mac's
  `machineId`, slug and name: [machines.md](../phone-api/machines.md)), `enable-phone`, `disable-phone`, `set-default-mode <mode>
  [--agent <agent>]` (without `--agent` the mode is read the most
  restrictive way for every agent; it prints what each agent's new chat
  gets). The phone ones first try the folder's `gui.lock`: free means no
  serve runs, whatever a stale `serve.json` says, so they change `phone.json`
  on disk holding it; held with a live serve's marker, they go through its
  **control socket** `<app data>/serve.sock` (bound in a fresh 0700 folder,
  set to 0600 there and renamed into place, so never wider; each peer's uid
  must be serve's own, `peer_cred`, so not even root drives it; one JSON line
  each way, the line within 5 s and 4 KiB); held otherwise, 75 (the app
  caches the record; its Settings is the place). `pair` needs a running
  serve.
- **Not yet:** no move-home rollback guard (the app's shows an alert); adding
  a Claude account headless (its token goes in with `secret-set`, but the
  account list is the app's).
- **As a LaunchDaemon (M1 PR 4):** the plist template
  [`docs/launchd/com.hollow.app.serve.plist`](../launchd/com.hollow.app.serve.plist)
  (runs as `UserName`, `KeepAlive` on a non-zero exit only, `ExitTimeOut` 60
  for the settle) and `write-serve-daemon.sh` beside it, which fills it in and
  prints the sudo steps without running them. Runbook:
  [HEADLESS_SERVE.md](../HEADLESS_SERVE.md).

### Windows

Three are defined in [`tauri.conf.json`](../../src-tauri/tauri.conf.json):
Main's native title is set at setup from `BuildIdentity::short_text()` (embedded
version, short commit and dirty qualifier). The custom title bar stays hidden;
the title identifies the binary in the Window menu and Mission Control.
- `main` — the dashboard: `transparent: true` (Liquid Glass vibrancy), an
  overlay title bar (`trafficLightPosition {x:16, y:24}`, which centres the
  lights at about `y − 2.5` pt, level with the 44 px top bar), drag regions on
  every non-interactive layer. **Links never navigate it away:**
  `linkGuard.ts` sends unclaimed web links to `openWebLink` and mail links to
  the default app, and `main_nav.rs` blocks every navigation of `main` except
  the app and the frames it embeds. Links in an artifact frame reach the host
  only through the `sandbox-doc.js` bridge and `openFrameLink`, gated on that
  frame having focus and user activation (3 a minute). Interactive artifact
  links additionally require a host confirmation of the full destination; Cancel
  opens nothing, and changing or removing the frame invalidates a pending approval.
  The host sandbox runtime removes standard and WebKit-prefixed WebRTC peer
  constructors before artifact scripts: CSP alone does not block STUN/TURN.
- **A second custom scheme, `hollow-media`, plays local video** (inline video,
  2026-10-17; `media.rs`). The player asks `media_grant` for one file and gets
  an opaque 32-hex token (OS CSPRNG); `hollow-media://localhost/<token>` is the
  only URL, so no path is ever in one. The handler is asynchronous: it returns
  at once on the main thread and reads on the blocking pool, at most 4 reads
  app-wide (extras queue; never a 5xx, which WebKit's media loader treats as
  fatal). Every answer is a `206` of at most 2 MiB (wry can't stream a body),
  and WebKit asks again for the rest; a GET without `Range` gets the first
  chunk only. Grants (32, least recently used evicted) pin the canonical path,
  device, inode, length and mtime, and every request re-checks them (410 for a
  swapped file, 409 for a changed one). Only the `main` webview is served
  (403 otherwise). That label is per webview, **not per frame**: an artifact
  or canvas iframe inside `main` passes it, and is held off instead by its CSP
  (no artifact mode's `media-src` matches `hollow-media:` or `http:`), its
  opaque-origin sandbox (it can't read the tokens in the main DOM), `main_nav`
  (navigation to the scheme is denied) and token secrecy. The main CSP's
  `media-src` is exactly `'self' hollow-media: http://hollow-media.localhost`.
  Posters (`media_poster`, PR 3) take another route: the same path rules, no
  grant, `qlmanage -t` (a PNG) then `sips` to JPEG, else ffmpeg
  (`-protocol_whitelist file`, a `file:` input), each step bounded to 5 s in
  its own process group, at most two at once, cached under
  `{app cache}/posters/<dev>-<ino>-<len>-<mtime>.jpg` (never the vault;
  failures marked `.none`), at most 150 KB, and delivered as `data:`, which
  `img-src` already allows.
  Artifact frames also forbid popups: a fresh blank popup could restore those
  constructors. Links go through the host bridge; nested frames remain CSP-blocked.
  Canvas mode (scripts, no network) is scripted, so its links are confirmed too.
  **Every frame message is canvas bridge v1** (`src/canvasBridge.ts`):
  `{__hc:1, kind, …}`, accepted only from that iframe's own window with the
  opaque `null` origin, at most 64 KB as UTF-8 JSON, exact fields per kind.
  Child → host: `size`, `error`, `navigate`, `open`, `state`. Host → child:
  `theme` (`watchFrameTheme` in `htmlSandbox.ts` observes `<html>`'s
  `data-theme`, `data-glass` and inline tokens and posts the live `--app-*`
  set and `color-scheme`; the runtime re-checks names/values, applies them,
  and fires `hollow-theme`) and `init` (Canvas C3 wave 2: once per frame
  load, `{canvasId, kit, locale, reducedMotion, state}`; the runtime exposes
  it as the read-only `hollowCanvas` global, `init` / `get` / `set`, and fires
  `hollow-init`). `state` is a plain JSON object ≤16 KB for the frame's own
  canvas only (`acceptFrameMessage(e, frame, canvasId)`; any other id, or a
  frame that isn't a saved canvas, is refused), taken only while that frame
  has focus, and saved debounced with the store's CAS from the canvas's
  current revision (a conflict re-reads and keeps the newer value). The kit's
  `hc-tabs`, `hc-table` sort and `hc-details` keep their view through it.
  **`draft`** (C4 A2; question cards, 2026-10-14): `{canvasId, answers,
  summary, nonce}` for the frame's own canvas only, answers ≤16 KB in the ask
  kit's value shapes, summary ≤500. Hollow draws the ask-canvas's Send button
  under the frame (`HtmlArtifact`, words from `<hc-ask send>`); its trusted
  click posts host → child `collect {nonce}` (32 hex, `newCollectNonce`), and
  the kit answers with `draft` or `ask-invalid {nonce, count}` (fields need
  attention; the frame marks them). `draftRefusal` then needs: an ask-canvas,
  not locked, its current revision, a conversation to send to, and
  `requested` (the nonce of that click's `collect`, within
  `COLLECT_TIMEOUT_MS`, 3 s). A draft the frame posts on its own is refused
  as `unrequested`, so no canvas script can send. A taken draft is SENT as
  the person's next message (`ChatBody.sendFromCanvas`; while the agent is
  answering, held for after the turn as "Queue answers", P1-33); a send that
  fails keeps the answers in the canvas with Send now and Take them back
  (`canvas/askHold.ts`, shared by every copy); the status shows under the
  frame. A pointer click on Send within 500 ms of the
  frame's height changing is refused (`questionDock.tooSoon`).
  An answered canvas locks with `answered {summary}`. The turn's end saves
  every closed canvas fence (`workspace_canvases::record_reply`), so a
  canvas from a turn no window watched (an unattended check-in) is current
  and can be answered; before 2026-10-14 it was never saved and every Send
  was refused `not-current` ("Hollow didn't take the answers").
  **Refused messages are logged** (Canvas C6 P1, `src/canvas/bridgeRefusals.ts`
  → `record_canvas_bridge_refusal`, `canvas_bridge_log.rs`): each refusal of
  a frame's own message (not another window's `source`/`origin`, not the
  person's `confirm`/`cancelled`, not Hollow's own failed send
  `not-sent`/`send-failed`; a `state` while the frame isn't focused is
  `focus`) is one JSON line of kind (a child kind or `other`), size, reason,
  canvas id, conversation key and a `suppressed` count, never the payload or
  any canvas text. At most 30 a minute in the webview and again in Rust.
  `{app data}/canvas-bridge-refusals.jsonl`, never the vault; retention: at
  64 KB it replaces `canvas-bridge-refusals.1.jsonl`, so two files and
  128 KB at most. The frame hears nothing new and there is no UI count.
  The runtime captures `window.parent` before artifact scripts can shadow it.
  The theme set is `THEME_TOKENS` (44 of the bridge's 48: the C1 nine plus
  every colour, with `--app-success|warning|danger|info` and `-soft`).
  **Every artifact document inlines the Hollow Canvas Kit** (Canvas C2,
  docs/history/CANVAS_C2.md): `sandboxHead` adds the kit
  stylesheet and, after the runtime, the kit script, loaded lazily in its own
  chunk (`loadCanvasKit`). Offline, Images and HTTPS allow the runtime and
  the kit only by the document's own nonce (`script-src 'nonce-…'`, never a
  hash: a hash also admits an external `<script integrity>` carrying it,
  SRI repair), so `hc-*`
  components render while artifact scripts stay blocked; Canvas and
  Interactive run it under `'unsafe-inline'` and list no hash or nonce.
  Components are light DOM, render text only, and stay
  readable when the script doesn't run. Data components (`hc-bars`,
  `hc-chart` on a DPR-sized `<canvas>` with a hidden data table, `hc-table`)
  read text rows or a `<table>`; `hc-file` renders an `a[data-vault]`, so the
  only message it can cause is bridge `navigate`, which chat and Result frames
  open in the Preview pane (clicked, inside the folder or vault;
  `openFrameFile`, CANVAS.md). The frozen kit@1 API is in
  CANVAS_C2.md. Charts never use ResizeObserver: they redraw on `hollow-theme`,
  window `resize` and the kit's `hc-reflow` (fired by `hc-tabs`/`hc-details`).
  **Ask-canvases (`<hc-canvas ask>`, Canvas C4 A1) also inline the ask kit**
  (`src/canvas/kit/ask/` → `generated/kit-ask.min.{js,css}`, its own lazy
  chunk via `loadCanvasKitAsk`; `HtmlArtifact`'s `ask`, set by `ChatCanvas`
  from the fence's root): `hc-ask` (the form and its Send) and the fields
  `hc-choice`, `hc-multi`, `hc-range`, `hc-rank`, `hc-text`, `hc-date`,
  `hc-matrix`, `hc-pick`, native labelled controls with validation inside
  the frame. Its script follows the kit's, with a nonce slot of its own in
  the script-free modes (`artifacts.rs` takes two to four slots), and its
  hash is the app CSP's third. Send builds the bridge `draft`
  (`src/canvas/kit/ask/model.ts`: answers ≤16 KB UTF-8 JSON, summary ≤500)
  and posts it; the chat puts it in the composer (below); on `answered` the
  form's fields go `inert`, Send goes and the sent lines show. Budget: 24 KB
  of its own (`KIT_ASK_BUDGET_BYTES`); the display kit has its own 56 KB.
  CANVAS_C4.md.
- The `quick-capture` and `quick-ingest` borderless overlays were removed in
  S5 (2026-11-03): `Cmd+Alt+Shift+T` and `Cmd+Alt+Shift+I` open the main
  window's Add sheet set to a task or to a Library source instead.
- `md-preview-{n}` — runtime-created viewers for Markdown files the OS opens
  (`bundle.fileAssociations`, `md_open.rs`; only in an installed `.app`), with
  **Save to vault** into `vault/imported/`.

All windows load the same Vite bundle; `main.tsx` branches on `?window=`.
**Each window needs a capability file** in `src-tauri/capabilities/`
granting the `core:window` permissions it uses, or calls such as
`getCurrentWindow().hide()` are silently denied. Close on `main` hides the
window (the tray keeps Hollow resident); preview windows close for real.

### Tray and global shortcuts

Use `get_window("main")` for native main-window operations. Tauri 2.11.1's
`get_webview_window("main")` returns None after a browser tab or canvas export
attaches a child webview, even while the native window is visible. A hidden
child still counts. `lifecycle::bring_main_forward` is the shared show,
unminimize and focus path for startup, Dock/open-app activation, menus, tray
actions, link handlers and the quit dialog. Dock activation brings hidden or
minimized main back even when a separate document preview is visible; it
preserves the selected window when main is already visible. Native focus/visibility
reads and UI-setting nudges also use the native lookup. Mock-runtime tests
attach a child through Tauri's manager and exercise availability and reopen;
the pure state table covers native states that the mock does not simulate.

Navigation and inline approval cards use `lifecycle::main_window_state` and
the pure `window_state.rs` policy. Positive native focus is evidence that main
is open even when visibility is false or unreadable. Focus, or visible plus
unminimized, clears a stale tray flag. Missing windows refuse both routes;
genuinely closed-to-tray windows refuse cards. Minimized, Cmd-H-hidden and
unknown native states retain the existing card wait, while navigation still
requires positive visible/focused evidence. Hide/show handlers update the tray
flag only after the platform accepts the request; focus events also clear it.
The flag carries a version. Observation repairs use compare-and-swap against
the version sampled before native reads, so a later close or hide/show/hide
cycle cannot be overwritten by an older open-window observation.
Reopening from the tray, Dock, menu or COO restores a minimized window.
The policy regression is tested with contradictory native observations; the
original installed AppKit sequence still needs a post-install hand check.

Registered by [`shortcuts.rs`](../../src-tauri/src/shortcuts.rs) from the saved
shortcut settings, after the menu and tray exist (lib.rs's handler asks
`shortcuts::global_for` which command a key is). Settings → Keyboard
shortcuts can change or remove them, and the off switch unregisters both. A
key another app already holds is a warning on that page, no longer a boot
failure. Not registered in setup mode, with `HOLLOW_APP_DATA_DIR` set, or in a
candidate build (`instance::is_review_launch`; a review build would take the
installed app's keys). The defaults:

`Cmd+Alt+Shift+T` quick capture, `Cmd+Alt+Shift+I` Import a source,
`Cmd+Alt+Shift+H` COO (command ids `quick-capture`, `quick-ingest`,
`main-agent`, kept so a saved binding still works). All three call
`show_main_with`: the main window comes forward and the `app-menu` event runs
the shared action (`quick-capture`: the Add sheet set to a task;
`quick-ingest`: the Add sheet set to a Library source, both since S5;
`main-agent`: the COO). For the two Add keys it first records where
Hollow was (`lifecycle::came_from`), so `after_summoned_capture` can put it
back after a save, as the removed quick windows did (or bring it forward when
an import fails). The tray menu starts with a disabled build
identity and Copy Build Identity, followed by Open Hollow, COO, New
Chat, Quick Capture Task… and Import a source… (P1-04; was Quick
Ingest; both the Add sheet since S5, like their keys) and Quit. The menu-bar
icon is a template image (`icons/tray-template.png`, regenerate with
`icons/gen_tray_template.py`).
The ordinary tooltip uses the same short build identity; legacy review-copy
tooltips keep their label. Tray copy uses `tray-copy-build` and forwards once
to the existing `copy-build` action: app and tray callbacks are both global,
so sharing their menu ID would copy twice. Full provenance stays in System
and Help's copy action; the palette exposes that same action by build keywords.

---

## Microsoft 365 (Outlook removed 2026-09-26; built-in connector 2026-10-01)

### Calendar adapter constraints

Agenda uses the source in app-data `agenda-calendar.json`: `source` is `mac`,
`microsoft` (missing-file default), or `off`; `calendarIds` is null for all,
an array for a fixed selection, and [] for none. Damaged settings fail closed
and remain untouched until an explicit source save. Keep the
macOS 11 minimum in `src-tauri/tauri.conf.json`: the SDK marks
`requestFullAccessToEventsWithCompletion:` as macOS 14+, so a Mac-calendar
adapter checks the selector and uses the older event-access request below
that version. Only choosing Mac or clicking Allow calendar access requests access
(`set_calendar_source` takes an explicit `requestAccess: true`). Status,
Agenda refresh and background meeting checks never request it. The Settings
card lists calendar/account names, keeps missing selections visible, and offers
System Settings plus Refresh after denial; unanswered/write-only access has an
explicit Allow calendar access retry. `calendar_source.rs` owns settings,
the reader seam and view mapping; `calendar_source/native.rs` owns EventKit.
The test binary cannot dispatch to the native reader or permission requester.
Agents can read `agenda.calendar_source_status` without arguments; it returns
the same passive status as Settings and never persists a choice. Source changes
and opening Calendar privacy settings are user-only in the command map and
excluded from the app-action registry.

EventKit participants expose `isCurrentUser`; do not filter their attendees by
the Microsoft account. Its attendee array is nullable, and event identifiers
can change after calendar moves or sync, so preserve unknown attendee coverage
and do not promise immutable native event identity. The local SDK headers are
the source for these constraints. No native access was requested to inspect them.

Mac events bypass both the Microsoft event cache and its day/event-ID attendee
cache. Every source/selection save invalidates a separate source generation;
Microsoft sign-in changes do not invalidate Mac reads. An in-flight
read returns a refresh notice if that generation or its settings changed.
Native IDs hash calendar ID, event ID and original occurrence date under a
`mac:` prefix; Microsoft IDs stay unchanged. All three meeting-prep paths use
occurrence-carried native attendees, not `eventWithIdentifier:` (which returns
the first occurrence). Nil attendees stay unknown, an empty array stays known.
All-day dates use local civil boundaries: midnight ends stay exclusive; a
non-midnight final-day end rounds to the next civil day. Timed events retain
absolute instants across DST. Native semantics still need the attended hand check.
Both sources share local-date/RFC3339 window parsing and a 93-day cap. Malformed
native occurrences are skipped with a counted partial notice; valid events remain
visible and sort by their actual start instant. Meeting prep accepts native titles
as well as Microsoft's subject field.

Hollow has no Outlook feature of its own (no calendar page, meeting
workspace or mail; deleted 2026-09-26). The Tasks Agenda dispatches read-only
through `calendar_source.rs` or `m365_mcp::agenda_events`. Agents get the built-in Microsoft 365 connector
(calendar and OneDrive/SharePoint files, read-only, on Hollow's own Microsoft
sign-in in `m365_auth.rs`). While it is signed in it replaces the claude.ai
Microsoft 365 connector in runs (since 2026-10-10; that one also has mail,
which Hollow doesn't do). Its tokens are the `m365_connector` Keychain key, written only by
the GUI under one auth generation lock.

Archive: architecture, windows and tray detail
(the September 7 idle sample, the full `main` window notes).

---
