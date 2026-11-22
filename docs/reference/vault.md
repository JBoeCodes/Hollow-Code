# Vault mirrors, alignment and Tasks

Part of the agent manual. [AGENTS.md](../../AGENTS.md) is the entry point and
links every reference doc; this one holds CLAUDE.md's "Vault mirrors and
source of truth", "Alignment, trusted learning, evidence, and recovery" and
"Tasks & Projects", moved there word for word on 2026-10-13 (only relative
links were re-based). Read it before changing a vault folder schema, sync
direction, the watcher, learnings, evidence, recovery, or task and project
files. Keep it current with the code in the same change, as AGENTS.md asks.

## Vault mirrors and source of truth

The vault is durable storage for authored context and mirrored fields. Ordinary
reads and edits use local files. Linked project/task fields mirror the task
tracker (Linear); its contract is [Tracker sync](../TRACKER_SYNC.md). Bodies,
local plans, sources, outputs and conversations stay local. Notion two-way sync
was removed on 2026-11-10 (history); its keys stay
in vault files as history.
Who owns each field is one rule since H4a (2026-11-18): `sources.rs` resolves
where a workspace's tasks live (config.json `sources`, a workspace's
`tasks_source`) and `owner_of` answers per key; moving tasks between the vault
and the tracker is a journaled, rollbackable switch
([Sources of truth](../TRACKER_SYNC.md#sources-of-truth-h4a)).

`vault_create_note(path, content)` publishes a fully written temporary file
exclusively; an existing destination fails without replacing its bytes. The
temporary file is synced before the hard link; its temporary name is removed
right after the link, then the folder is synced once (best-effort, as
`atomic_write` does after its rename).
`vault_save_note_cas` remains the edit boundary. The editor serializes saves,
retains typing that arrives during a save, and stops note switching, moves, and
native handoff when a save fails or conflicts. The buffer remains available
across page navigation for conflict resolution. In `WorkFileEditor` a save writes
a snapshot through `fileSave.saveSnapshot`; text typed while it is in flight stays
an unsaved draft against the version just written (memory and `fileDrafts`) and
is what the next save writes (review A, slice 4 item 1d). A conflict merge
(`textMerge.merge3`) joins lines with the file's own line break
(`MergeResult.eol`), so resolving a conflict in a CRLF file keeps it CRLF.

### Verified writes (2026-11-14)

Hollow never writes over an edit it didn't read. `vault::replace_if_unchanged`
is the one compare-and-replace: the new bytes go to a synced sibling temp file,
the file is re-read just before the publish, and the publish is an exclusive
hard link (no file expected) or a `RENAME_SWAP` whose displaced bytes must equal
what the caller read (a writer that landed in the window is swapped back; off
macOS a rename, guarded only by the re-read). `vault::write_merging` builds on
it for text: when the file changed, it tries the line three-way merge
(`shared_docs::merge3`) of what was read, the file now and Hollow's text, and
writes the merge (up to four tries). Overlapping edits leave the file as it is,
keep Hollow's text beside it as `<stem> (conflicted copy <stamp> <id>)<ext>`
(offered in Waiting on you) and return `MergeWrite::Conflict`, whose message
the caller shows. Callers: `mutate_note` and `mutate_note_preserve_mtime`
(companion notes), SOURCES.md appends (attach and refresh), the canvas sidecar
and visible copy (compare and replace only; a change is the canvas `CHANGED`
error, which callers reread and retry), and the source refresh's snapshot
write and rollback, which restores the snapshot and sidecar only while they
still hold what the refresh installed and reports anything it kept.
Attachment snapshots publish with `vault::publish_exclusive` (temp file, then
an exclusive link), so a crash never leaves a torn snapshot under its name.
WORKSPACE.md and PLAN.md updates (`shared_docs::update_doc`) already re-run
their transform on the newest copy under a revision check, so they keep that
path.

**A true clash on an agent's write (R7b, 2026-11-15).** `vault_write_note` with
`expectedRevision` (the revision `vault_note` returned) writes nothing when the
file changed. The `WRITE_CONFLICT` error carries both versions (the file now and
the agent's text, each in full up to 24 KB) and the new revision, so the agent
merges them itself and retries; there is no base (a revision isn't the text), so
it is a two-way merge. That raises no item. One item appears only when the agent
can't: `vault_give_up_write` (or the same path refused three times in a row,
`vault_clash::MAX_TRIES`, which gives up for it) keeps its text as one conflicted
copy beside the file. The same text given up again reuses the copy, so a clash is
one item. The give-up needs an original the scan lists (`.md`, `.html`, `.htm`), else it
errors. The refusal counter (per chat and path; only a resend of a revision already seen, not the one just handed back, counts) is in memory,
resets on a write that goes through or after ten minutes. Edits an agent makes with its own file tools (WORKSPACE,
PLAN, SOURCES) still go through `shared_docs`' end-of-run net, which has no
agent left to ask and keeps a copy.

### Per-tab folder schemas

Paths are vault-relative. Full rows (every preserved key, body rules, Notion
mappings and retired folders) are in the
vault archive.

| Record | Vault path | Frontmatter and body |
|---|---|---|
| Tasks | `tasks/{slug}.md` | `title status priority(P1 to P4) due due_time duration_min effort project project_url section labels(comma scalar) recurrence blocked_by created source origin fingerprint`, legacy `notion_id notion_url last_synced_at` (history, never written now), plus preserved unmodelled keys (`previous_due`, `description`, …). Body: prose description, a `- [ ]` checklist read back as steps, and reserved `## Done when` / `## Activity` sections. `fingerprint`/`origin` are written once by an idempotent create. Identity is `vault:{rel-path}`. |
| Projects | `workspaces/notion-<id>/PROJECT.md` (legacy Notion mirrors, plain local workspaces since 2026-11-10: `<id>` is the page id, 32 hex or a dashed UUID; test with `workspaces::is_notion_mirror` / TS `isNotionMirrorId`, never the `notion-` prefix alone, and a titled workspace's id never starts `notion-`/`local-` (`ws-` is put in front); overnight 10-11 Y2) or legacy `projects/{status}/{name}.md` | `title status priority created target_date deployed_at description current_state desired_state non_goals success_evidence goal_urls source`, legacy `notion_id notion_url last_synced_at`. Status uses frontmatter. |
| Project records (B3) | `workspaces/<id>/PROJECT.md` (any workspace; tracked ones also carry `tracker`, `tracker_project`, `tracker_url` and the tracker's `title status status_type target_date start_date lead`, written by the sync pass) | Portfolio keys, vault only and never sent: text `department type hours_saved_per_year value_per_year owners_note github_url live_url`; block lists `requested_by stakeholders domains meetings captures war_room docs`. Read and saved by `project_portfolio.rs` (compare-and-swap on the revision); Track in the tracker writes the tracker keys so the next pass adopts the file. |
| Goals | `goals/{slug}.md` | `type(=goal) title status(active\|paused\|achieved\|dropped) horizon target_date why success_evidence created` |
| Commitments | `commitments/{slug}.md` | `type(=commitment) title direction(owed_by_me\|owed_to_me) counterparty person_path project_url due status(open\|fulfilled\|cancelled) channel source_path evidence snoozed_until created fulfilled_at fingerprint`; `snoozed_until` (`YYYY-MM-DD`, open only) keeps it off the ledger's open list, Today, the Tasks panel and the overdue nudge until that day; `evidence` is the quoted line an accepted proposal rested on |
| Learned context | `learnings/{slug}.md` | `type(=learned-context) title target_kind target_path subject trust(=approved) proposal_id source_run_id learned_at supersedes superseded_by`; only review acceptance creates one (its proposal's payload records it as `learnedPath`; a bulk review also stamps `batchId`, and a bulk Reject an optional `rejectReason`, stored but not yet read); `superseded_by` (or `status: superseded`) means replaced in recall and retrieval alike (`note_lifecycle`) |
| Operator profile | `me/{Identity,Communication,Anti-patterns,Guardrails,Decisions}.md` | optional `title last_updated last_reviewed review_cadence` |
| Workflows | `workflows/{slug}.md` | `title description schedule agent model cwd output_glob routine permission_mode allowed_tools disallowed_tools triggers in_chat runs_on misfire_grace_min skip_if_quiet watch precheck notify ends after_runs runs_counted ended_note created last_started_at last_run_at last_status last_error last_duration_sec next_run_at`, plus any unmodelled key; body is the prompt template, then `## Run log` |
| Workspaces | `workspaces/<id>/` | `WORKSPACE.md` (brief), `PLAN.md`, `outputs/` (Results), `sessions/<key>.md`, `.hollow/results.json` (review state), `.hollow/focus-blocks.json`; see [Workspaces](../WORKSPACES.md) |
| Imported | `imported/{name}.md` | plain copies saved from the Markdown preview window |

**Never re-render an existing task or workflow file.** Every Hollow write to
an existing file is a verified line patch (`frontmatter::patch`), so unknown
keys, lists, comments, quoting, CRLF and the body survive; see the quirks.

**Hidden operational state in the vault** (dot-prefixed, so excluded from
the watcher, Library, search, the semantic index and History's own scan):

- `.hollow-history/objects/{sha256}` + `.hollow-history/runs/{run-id}.json`:
  content-addressed History of automated writes (decision 5: files up to
  2 MiB, newest 200 runs, 90 days, 512 MiB). Each change records
  `beforeCoverage`/`afterCoverage` (`absent`, `captured`, `uncaptured` with a
  reason, `unknown`); only absent and captured sides restore, and
  `HistoryRun::fully_recoverable()` is false otherwise. Retention prunes
  nothing while any run record won't parse. Object cleanup (retention and
  Clear) holds the vault's object store lock (`Leases/{vault}.objects.lock`)
  exclusively, and every write that stores objects before its record names
  them (run start and finish, a live write's before and after, step pins,
  as-of snapshots) holds it shared, so a second Hollow process (the headless
  MCP host) can't lose a run's versions in that gap; a busy lock skips that
  cleanup round (Clear says to try again). A write never stores or reuses an
  object without the lock: a run's admission waits 250 ms, then reports
  History busy and is deferred; a finish, a live write, a step pin or an
  as-of snapshot waits up to 30 s, then refuses and stores nothing (the run
  goes without History's coverage, as for a failed snapshot). Retention
  deletes only in the first 20 s of its hold, so it alone never makes a
  write refuse. Clear removes every run record, step pin and as-of snapshot
  before any object, so a clear that fails midway leaves only unnamed
  versions for a later cleanup. The incremental snapshot stores
  again any object its tip names that is gone, since only the process's own
  tip counts in cleanup (Q-R9, 2026-11-13). `runs/{id}.repo.json` holds a
  run's repository rows with bounded, redacted patches.
  `stat-cache.nosync` (OB09) maps path + size/mtime/ctime/inode/device to
  a hash, so a snapshot skips reading a file whose stat is unchanged and
  more than 2 s older than the snapshot start (racy guard on mtime and
  ctime) while its object's own stat is unchanged too. It is HMAC-sealed
  with a per-process key (an agent-forged entry is ignored; the first
  snapshot after a launch reads everything), disposable (corrupt, foreign
  or another version: rebuilt; a failed save never fails the snapshot;
  never saved over 64 MiB), and outside retention's accounting. A second
  Hollow process on the same vault can't open the first one's cache, so
  each discards and rewrites it: slower, never wrong. Used only on the
  vault root's own device and only on APFS, HFS+, ext2/3/4, XFS or Btrfs;
  on anything else (FAT, exFAT, SMB, NFS, AFP, WebDAV, FUSE, unknown)
  every snapshot reads every file.
  `snapshot-tip.nosync` (K1, 2026-11-08) stores that coverage, sealed with
  the same per-process key. On macOS, after the first snapshot in a process,
  a full-vault snapshot flushes file events and rereads only the paths they
  named. Every other path keeps the tip's hash, and recovery still reads the
  object from the snapshot that stored it. A dropped event, a moved vault
  root, a tip that will not open, or a host that is not macOS walks the
  whole vault. A scoped run still walks its folder. Tests always walk.
  Retention keeps the tip's object hashes. A tip that fails to publish is
  removed, and that generation is not reused, so copying its bytes back
  does not open. An older tip from this process, or one sealed for another
  vault, does not open. A symlink above a changed path, or a path that cannot be
  stat, makes that snapshot walk the vault. A watcher whose setup fails
  releases what that setup made and registers nothing, so the snapshot
  walks and the next one tries again without leaking.
- `.evidence-receipts.json`: newest-first receipts (cap 500) with optional
  `processStop`, `resultKind`, `repoChanges`, `phaseTimings`, `services`,
  `switches`, per-file `additions`/`deletions`, `historyRecoverable`,
  `canvases`, `scheduledWake` and `pullRequests` (overnight F8,
  `evidence::pull_requests_of`: each `gh pr merge <n>` / `gh pr create` call
  whose result wasn't an error, parsed with Home's Last night rules; a
  create's URL read from the full result before the 600-character excerpt
  cap; ≤20 rows; a link only for an `owner/repo` on github.com, opened
  through the app's link guard). A merge with no repository (no `-R`, often
  a later turn than its create; morning N8, `evidence::fill_pr_repos`) takes
  `gh`'s base remote (`gh-resolved`, else a lone GitHub `origin`;
  `build_workspaces::gh_remote_of`, git config read with no process,
  worktrees followed, small regular files only) of the folder the merge's own
  call stood in (`merge_places`: its `cd`/`pushd`s and Codex `workdir`, read
  with `overnight_summary::simple_commands`, `bash -c` scripts as a child
  shell, nothing after a `( … )`/`{ … }` group; an earlier call's `cd` never
  carries over, since a subagent or a fresh Codex shell may have made it),
  else (no call of the turn moved) of the
  run's one watched repository (`EvidenceCollector.repo_roots`, kept when the
  post-run capture takes the baseline). Nothing when `GH_REPO` appears, the
  folder can't be followed, or roots are several. Home's Last night card
  doesn't fill these yet. A receipt also has `toolCalls` (OF18): the
  tool calls the run's stream reported, errors included, for failures per
  call. Absent (unknown, not 0) on receipts written before it and for an
  agent that reports no tool calls (Antigravity). `memoryWrites` (S13): a
  Claude run's successful `Write`/`Edit`/`MultiEdit` calls into Claude Code's
  own auto-memory folder (the manifest's `nativeMemory.autoMemory.dir`);
  absent for other agents. `apiUsage` (OB05, chat and workflow receipts):
  the run's `apiCalls`, `peakContextTokens`, `cacheWriteTokens`, `apiMs`,
  `numTurns` and `subagents`, the keys its usage row's `usage_json` holds
  (see [Usage instruments](../USAGE_INSTRUMENTS.md)); absent when the agent
  supplied none. `verification` (S27, chat receipts): `claim` is `passed`
  when the turn's final reply (its text after the last tool call or tool
  result) matches a short pass-claim list (`evidence::PASS_CLAIMS`: `all
  tests pass`, `gate is green`, `42 passed; 0 failed`, …; a phrase in inline
  code or double quotes, a negating or conditional word in the six words
  before it, a question, or a failure count after it rules a match out),
  else `none`; a chat turn that fails before any reply gets `none`.
  `claimWithoutCheck`: a pass claim with no recognised check after the
  run's last successful write (no write: none in the run); `claimAfterFail`:
  a pass claim whose last recognised check failed. Both omitted when false.
  A "write" here is an `Edit`/`Write`/`MultiEdit`/`NotebookEdit`/
  `apply_patch` tool call or a Codex `file_change` item; shell writes
  (`sed -i`, a redirect) are not counted, a conservative miss. A subagent,
  Workflow run or Hollow consultation/child task that ended after the write
  (or that check) leaves the flag false: its checks are unknown. A
  subagent started in the background (`run_in_background`) never ends in
  the stream, so it leaves both flags false wherever it ran. Antigravity
  reports no tool calls: its receipt records the claim with neither flag.
  Absent on surfaces with no reply and on older receipts. `cites` (repo
  features R5, chat and workflow receipts; `cite_check.rs`): every
  `path:line` (`:N`, `:N-M`, `:N:C`, `#LN`, `#LN-LM`; a path whose extension
  starts with a letter) in the final reply outside fenced blocks and URLs, at
  most 64, checked when the run ended. `checked`, then a count per status,
  each omitted when 0: `seen` (the line was in what the run read: a `Read`
  window, default lines 1–2000; Grok's `read_file`; a `read_source` range; a
  shell `sed -n 'a,bp'`, `head -n N`, or `cat`/`tail`/`nl`/`head -c` as the
  whole file, relative paths under a leading `cd DIR &&` (a bare `cd` is
  `~`, `cd -` the folder before) and Codex's `workdir`; or within 3 lines of
  a `Grep` (`content` mode, `-n` not off)/`grep -n`/`rg -n` hit, at most
  256, a one-file grep's bare `12:` lines included, none from a script whose
  numbered grep reads a pipe), `fileOnly` (the
  file was read, not those lines), `earlier` (one of the conversation's 20
  previous turns read the file, Grok's tools included), `notOpened`, `unknown` (not opened as far as
  the stream shows, but a subagent (Codex `spawn_agent`, Grok
  `spawn_subagent` too), Workflow run, consultation or child task ran, or
  the agent reports no tool calls), `pastEnd`, `missing` and
  `unresolved` (a bare file name the run's reads don't name exactly once).
  A relative path is the run's folder's (`manifest.run_cwd`), else the
  vault's. `items`: the non-`seen` cites, at most 16, `{path, line, end?,
  status}`, the path relative to the folder or vault, else as the reply
  wrote it (`~/…` kept); a cite under a
  `restricted` folder is counted but never listed. Files over 16 MiB,
  binary ones, anything past 64 MiB read in one check and a file whose size
  changed while it was counted get no range check; a cited path that isn't a
  regular file is `missing` (a bare name, `unresolved`).
  A note only: never a run's status or verification state. Absent when the
  reply cited nothing, on other surfaces and on older receipts.
- `.review-queue.json`: the schema-v2 review store for every suggestion kind
  (cap 1,000 terminal items; unresolved items are never dropped).
- `.notifications.jsonl`, `.notion-imports.json` and `.notion-sync.json` (history:
  nothing writes them since 2026-11-10), `.workflow-triggers.json`, `.task-filters.json`, `.app-actions.jsonl`
  (the `hollow_app` audit log).

**Watched dot-state.** The files in `run_guard::WATCHED_STATE_FILES`
(`.review-queue.json`, `.evidence-receipts.json`, `.heartbeat-state.json`
(a tripwire: never written since Paperclip PR 8a), `.workflow-triggers.json`,
`.tracker-sync.json`; the two Notion ledgers left it on 2026-11-10) are hash-watched during automated
runs but never enter History. Every Hollow writer of them goes through
`run_guard::write_host_state`; a new host writer must too, and a new in-vault
host-written state file must join the list.

**Outside the vault** (agent-writable folders must not decide what a run may
do), mostly in `agent_settings::app_data_dir()`
(`~/Library/Application Support/com.hollow.app/`) or
`~/Library/Application Support/Hollow/`:

| State | Where |
|---|---|
| `agent-settings.json`, `agent-tools.json`, `claude-accounts.json`, `budgets.json`, `connectors.json`, `shortcuts.json`, `build-workspaces.json`, `services.json`, `run-ledger.json`, `self-install/`, `canvas-bridge-refusals.jsonl` (+ `.1`, ≤64 KB each) | app data dir |
| Workflow permission stamps, leases, run permits, heartbeat state, connected-project index | `Hollow/WorkflowPermissions/`, `Hollow/Leases/`, `Hollow/RunPermits/`, `Hollow/Heartbeat/`, `Hollow/Directories/` |
| Sessions DB (conversations, usage ledger, run queue, question cards, feedback) | SQLite opened by `session_store.rs` |
| Left by the session importers (removed in run 15 P4), unread, nothing dropped: `session-imports.json` and the sessions DB's `external_*` tables | app data dir; the sessions DB |
| Run MCP configs, broker sockets | `~/Library/Caches/hollow/run-mcp/`, `~/Library/Caches/hollow/*.sock` |
| Per-run scratch | `/private/tmp/hollow-run-<uid>/<16 hex>` on macOS |
| Secrets | one Keychain item (`secret_store.rs`) |

A damaged state file is never read as defaults: `state_file.rs` renames an
unparseable file to `<name>.corrupt-<time>` and returns the error, and the
caller fails closed (see the quirks).

### Sync direction by service

Tasks and projects read local files; tracked ones mirror the task tracker.
Calendar comes from the source connected in Settings → Connectors; Plaud has two
accesses, Recordings in Hollow (Hollow's own sign-in, copies only a transcript you
choose to keep) and Plaud tools for agents (the connector). Email, Teams and
Slack ingestion are retired. Plaud is never synced into the vault (sign-in and
on-demand reads).

### Catching up a stale workspace

The September 6 live catch-up is recorded in the vault's
`notes/Workspace Refresh - 2026-09-06.md`. Current tasks use the `current-work`
label; unresolved historical work uses `needs-review`, with saved filters for
both. `previous_due` preserves a retired planning date (history only, not a new
scheduler field). Obsolete task notes may be recoverably moved outside `tasks/`
to `archive/tasks/{date}/`; never use a completed status to disguise cancellation.
Task/project refresh reads local Markdown and cannot infer current outcomes.
Use actual session timestamps, current source/build records and explicitly
requested mail/recording evidence; preserve unsupported outcomes for review.

### Stable URLs

Every Task and Project has a `url` field used by the frontend as its
**drag-drop / identity key** (not as a click target). It is always a
synthetic `vault:{rel-path}` URI, e.g. `vault:tasks/follow-up-with-alex.md`.
The frontend treats these as opaque IDs; the backend resolves them back
to the vault file via `vault_tasks::find_by_url` and
`vault_projects::find_by_url` (both `safe_join` the path so a crafted
`vault:../…` can't escape the vault).

`TaskModal` and `ProjectModal` are mounted once globally in `App.tsx`, so
their module-scoped `open` signals work from any page; don't mount them per
page (two instances would double-render).

### Vault is live file-watched

One recursive `notify` watcher on the vault root (`vault_watch.rs`, started
in `lib.rs::setup`) debounces bursts (300 ms) and emits `vault-changed`
`{ folders, paths }`; the frontend maps touched folders to the resources to
refetch (coalesced leading-edge), and the watch thread patches the backend
link index and invalidates the walk cache before emitting. Tracker network
access is never triggered by reads or watcher events.

The open note is never reloaded while it has unsaved edits, and paths with a
dot- or underscore-prefixed component (`.git`, `.obsidian`, Hollow's state
files) and `~` backups never fire a refetch. A view of one workspace file
(`WorkFileEditor`) watches `store/vaultChanges.ts`
(the latest event as a signal) and re-reads when its path is named, or when
the 200-path list was cut and its folder changed
(`sectionSource.vaultChangeTouches`); an unsaved draft is never replaced.

### Context layers and the vault

The vault is not one of the four context layers
([CONTEXT_LAYERS.md](../CONTEXT_LAYERS.md)). It holds workspaces, outputs,
the library and conversations, and stays its own private repository: the plan
names `example-user/hollow-vault` (decision D11), but on main the vault root
(`sync_vault.rs`, CX6b) commits only in app data, off until the person turns
it on, and a remote waits for V6. The personal folders that sit in the vault
(`me/`, `memory/`, `learnings/`, `home/`) belong to the personal root
(`sync_engine.rs`) and are never tracked by the vault root.

- **Shared docs are not a layer.** A workspace's `WORKSPACE.md`, `PLAN.md`,
  `SOURCES.md` and `AGENTS.md` are Hollow's record of that work. A fact the
  extractor finds true for everyone on one project goes there (`targetKind`
  `workspace`), never to the org layer; a project repo's own `CLAUDE.md` and
  `AGENTS.md` are the project layer, scanned for machine and person facts.
- **One log per chat** (V5b): each chat in a workspace appends to
  `log/<chat key>.md` (`shared_docs::chat_log_path`), and the shared docs take
  only workspace-wide changes
  ([Shared docs](../WORKSPACES.md#shared-docs-while-chats-run-in-parallel)).
- **Archiving** (V5a): `workspaces.archive_shared_docs` moves a shared doc's
  older entries to an archive note (dry run, then a hash-bound apply,
  lossless check; `context_compact.rs`).

---

## Alignment, trusted learning, evidence, and recovery

Memory maintenance (overhaul J5) uses app-data
`memory-usage/<vault-stem>.json`, not edits to saved facts. The per-note
metadata has `firstSeen`, `lastUsed`, `restoredAt`, `uses`, `pinned` and
`expiresOn` (YYYY-MM-DD, valid through that local calendar day: expiry starts
at the person's local midnight, not UTC midnight). Missing state gives a full
grace period on first observation; corrupt or invalid state is left intact
and excludes its facts from current recall until repaired. A read-only
search or Library visit never records usage. Actual turn recall records
selected uses from the learnings that recall already listed (no second walk)
and writes the sidecar only when a count or a first sighting changed. Know-how is stale after 14 quiet days and archived after 30;
profile rules never age automatically. Pin stops quiet aging, expiry still
applies, and explicit Restore restarts the grace period and clears expiry.
Historical retrieval labels archived/expired facts and never restores them.
They stay searchable in Saved, and no fact is deleted or moved.

Library → Learnings → Saved owns pin, restore and expiry. Each click carries
`expectedHash` over the person-set fields (`pinned`, `expiresOn`,
`restoredAt`) plus the source bytes, so a click after a change to either is
refused, while a recall in between (`uses`, `lastUsed`) is not. A blank
valid-through date clears the expiry. `memory_maintenance` and `edit_memory_maintenance` are user-only
commands; edits pass the scan and append a person-attributed change-log entry.
The view shows all five always-on profile files' total bytes; growth above
16,000 bytes yields a consolidation proposal, not a new truncation or write.

Accepted You and Project learnings (J2's line homes: one line in a `me/`
note, a workspace's WORKSPACE.md Decisions or its AGENTS.md) get the same
controls. Their sidecar key is `line:<accepting proposal id>`; that
proposal's `learnedPath` and `learnedLine` are the line's identity, and a
click names the key (`edit_memory_maintenance { key, ... }`). Profile lines
never age; a Project line ages with its workspace: a turn whose recall is
scoped to workspace `<id>` counts one chat day under `workspace:<id>`
(written at most once per local day). A line still in its home is never
archived, only stale. Opening Learnings (`memory_maintenance`, which runs
`refresh`) records a line's first sighting and, for an intact line that is
expired or stale, files one learning proposal per lapse with
`payload.retires = { proposal, path, line, cycle }` and review source
`memory`. Only Hollow sets `retires`; a chat's or an agent's value is
cleared. Saving it as proposed takes the line out; saving it reworded
replaces the line in place, and the proposal becomes that line's owner. Any
change of words is a rewording, case included ("redis" to "Redis"); only
whitespace is folded. Rejecting keeps the line, and the same lapse is not
proposed again. Pin, Keep this line (restore) or a later valid-through date
supersedes a waiting proposal. A row's Save, edited or not, is a one-item
batch (`resolve_learning_proposals_batch` with `editedStatement`), so its
Undo snackbar and "Undo last batch" put the old words back in place, or
re-add a removed line at the end of its section, like a bulk save's Undo.
The claim is the Save's receipt: it persists the original (`retires.line`)
with `learnedPath` and what the Save writes (`learnedLine`, none for a
removal) before the note is touched. A Save interrupted after the line
changed (a crash, or the queue write failing) recovers to pending with that
receipt; saving it again finishes from what the note shows instead of
calling the missing original a hand edit, logs the change once, and keeps
the original for Undo. Until it is finished, only a Save with the
receipt's words goes through: after an interrupted removal, the words
unchanged; after an interrupted rewording, the same new words ("Finish the
earlier Save first"). The receipt is never overwritten, so a second
interruption can't leave a line that matches neither the original nor the
receipt. To reword further, finish it, Undo, then save new words. A line no longer present exactly as saved shows
as `changed` (also while such an interrupted Save waits): its controls are
refused, and so is its proposal unless the note shows exactly what an
interrupted Save of that proposal wrote. The file is left byte-exact.

Automatic capture (overhaul J6) is owned by `learning_capture.rs`, independent
of the chat provider. Only attended top-level typed turns enter its queue.
The newest finished turn per chat waits until ten idle minutes, four
successful turns, a handover, or 80% of the provider's configured compaction
limit (the context ring's advisory/window when the provider reports no limit).
A new turn never discards it: the turn in flight is kept apart, and the pump
ends it for every turn that is cancelled, fails or cannot start
(`learning_capture::ended`), which restarts the idle wait for the retained
turn. A finished turn whose words pass the explicit gate
(`vault_learning::should_extract`: "remember that", "from now on",
"correction:", a stated preference or decision) is due at once, through the
same capture, lease, scope and proof rules, so it starts right after its
turn instead of waiting in memory for idle. It snapshots before the handoff and
runs detached, so it never delays the turn. A provider that silently
compacts still has its host transcript intact. The retained input holds at
most 8,000 characters of the reply (what the extraction prompt reads), and
a chat with nothing waiting drops its entry.

The extractor reads at most four turns from the source run backwards,
excluding agent task packets and any turn without the source's persisted
capture_scope. The digest is budgeted newest turn first (12,000 characters
of proof, 8,000 of the person's words; 2,000 per message, six tool results
of 800 per turn), so the source turn always keeps its whole share and
older turns fill what is left, still listed in turn order. That field records cloud provider or private model entry only
for admitted turns. CLI agent names cannot distinguish private models from
cloud models on the same harness. Unknown history is excluded and an unmarked
source refuses capture before a model call. The stored scope also picks the
runner: `private:<entry>` runs only on that self-hosted entry, `cloud:<agent>`
on the usual Learning route, and a scope that disagrees with the chat's own
entry refuses. User words and tool results ground the exact quote; assistant
prose is a hint only, and a private capture also gets up to three open
friction patterns as hints. A cloud capture gets none, because friction is
vault-wide and keeps no origin. Profile facts require a quote from the
person's words. Sources are redacted before extraction by
`claude_runner::redact`, which also masks AWS, Google, Slack and Stripe
keys, upper-case `*KEY=`/`*TOKEN=`/`*SECRET=`/`*PASSWORD=` values and a
URL's password, because tool results now reach the extractor for every
provider. A missing quote, transient setup failure or a
negative tool claim is refused; all remaining proposals meet the memory
scan and duplicate screen. One unresolved capture per chat waits for review;
nothing is accepted, moved or rewritten by the capture worker. Off still
stops capture, and a self-hosted chat keeps its extraction on its own entry.
If that entry is asleep, the queued Learning job retains `capture_session`
(optional, backwards compatible), coalesces to the newest input for that
vault/chat and reconstructs grounding when it drains. It never falls back to
an explicit-remember-only check or sends the capture to another provider.

Library → Learnings is the one review home (overhaul J7). The lazy component
also draws a line beneath the reply whose run proposed the lesson. Pending,
Saved, Skipped and Rejected read the existing store without a migration.
Save can edit the statement; Reject requires a one-line reason; Skip uses
the reversible snooze until 9999-12-31 and Review again wakes it.
Bulk Save/Reject and persistent batch Undo remain here. Saved entries keep
profile/project lines, older notes, provenance and replacement controls.
Rows retain ids across refreshes. A reactive draft shared by proposal id
keeps edits and reasons consistent between reply and Library copies, and
across visits. The e/a/Shift+R/z review keys remain on learning action groups;
j/k moves between groups. Navigation commits after the draft guard accepts.
Settings Memory links here. The bell and Needs you still list and count
each pending learning, so an overnight or automation proposal has a signal,
but the row opens this tab and decides nothing; the tab shows the pending
count. After Save, Reject or Skip, focus moves to the next row's actions, else
the previous row's, else the Learning state tabs (in a reply line, to that
line's Open in Learnings link). A failed action shows under its own row only.
A bulk Save or Reject, and a row's Save (edited words or not, sent as a
one-item batch), shows a 10 s Undo snackbar and "Undo last batch (N …)"
until nothing in that batch is left to undo.

The alignment/trust wave and Hollow 3.1 activation share one trust boundary:

- **Suggestions are not records.** Learning, activation, commitment and
  heartbeat generators write only `.review-queue.json`, and only a person
  accepts (decision 3), through the same validated writer the UI uses.
  `hollow_tasks` `propose_tasks` is review-only too. **The one exception:**
  inside a lease-holding chat or workflow run, `hollow_tasks` creates and
  updates tasks directly through the GUI broker, inside that run's History
  (`automated_history::with_live_run`).
- **Recall is approved reference data.** Only accepted learning notes enter
  Chat, Workflow, or Heartbeat prompts, and a superseded one only when the
  request asks about the past (below). The injected block cites
  every `learnings/*.md` source and explicitly says it is reference context,
  not instructions. The cap is five notes / 3,000 characters. Recall matches
  the run's **request** (`turn_request.rs`: a chat's own words plus the names
  of what a reference resolved to; a workflow's prompt after variable
  substitution; the heartbeat's own prompt) with the notes it names as
  subject paths, never the prompt with profile, page, conversation, evidence
  or workspace context around it (agent effectiveness F2, 2026-10-06).
  A global learning (operator or decision target) is always eligible and is
  ranked by the words its subject and statement (not its target path) share
  with the request, compared by a light stem ("deploying" finds "deploy",
  "meetings" finds "meeting"; an inflection match, not a paraphrase one), then
  by recency: a relevant older one beats newer unrelated ones, and slots left
  over fill by recency. A learning on `me/Guardrails.md` ranks ahead of every
  unrelated one (S11, 2026-10-16; before, every global learning scored alike
  and the newest five filled the slots).
- **One lifecycle policy for recall and retrieval** (`note_lifecycle.rs`,
  agent effectiveness F7, 2026-10-06). Only a learning has a lifecycle: a
  `learnings/<name>.md` note with `type: learned-context`, the notes recall
  lists (`note_lifecycle::is_learning_path`). Any other note is untouched
  whatever its frontmatter says, so a project filed under
  `projects/superseded/` with `status: superseded`, or an ADR or Notion page
  with a Superseded status, stays ordinary evidence. The lifecycle is read
  from the frontmatter its reader already parses: approved (`trust:
  approved`), superseded (a `superseded_by:` link, which `supersede_context`
  writes and which may chain, or `status: superseded`), `supersedes:`, its
  date (`learned_at`, else `date`/`updated`) and its exact statement. A link
  is followed only to another learning, and a note read by path to resolve
  a current version is refused when the vault walk would skip it
  (`vault::walk_skips`: a hidden component, `emails/`, `teams/`, `slack/`,
  `inbox/`, `_signature/`). A request is
  current-state unless its own words ask about the past
  (`note_lifecycle::history_cue`: "used to", "history", "historical",
  "previously", "formerly", "originally"; "before" ending a sentence, in
  "before it changed" or pinned by a past date, version or "the switch"-like
  phrase, and "changed since" one (HF04); or an explicit past date; a chat
  reads the request, never the names a reference
  resolved to; the heartbeat is always current-state). Current-state: a
  superseded learning is left out of recall and of retrieval evidence, and in
  retrieval its current version takes its place in the ranking (read from
  disk if it matched nothing itself). Historical: it is kept, labelled
  `historical: superseded by <path> (current since <date>)`, directly behind
  its current version, in both routes. Exact duplicate statements collapse
  to the newest; an unapproved learning is labelled in retrieval and never
  recalled. Learnings are never edited or deleted by this; the audit history
  stays. The manifest's `lifecycle` records every decision.
- **Learning reads the exchange.** A successful chat turn's post-turn
  extraction gate (`vault_learning::should_extract`: "remember that", "we
  decided", "always use", or "decision" after a material change) and its
  extractor see the user's request and the reply, plus the notes the request
  named or resolved (listed apart, as targets only). Profile text, an old
  conversation bundle, or the operator block's own "settled decisions"
  wording never open the gate (until F2 they did: any turn carrying the
  operator block whose run recorded vault changes ran an extraction).
- **Only the person's typed words teach, and rarely (fewer learnings,
  2026-10-11).** A turn learns only when its message came from a composer
  (`ChatSendRequest::typed`, set solely by `commands::chat_send`; queue
  payload `typed`, absent = false) in an attended, top-level, non-bare turn
  (`chat::turn_learns`). Composer sends include the suggested next message
  and question-card answers the person sends. Agent packets (start/
  send_to_session, handover, swarm), wakes and check-ins, resume, retry,
  night and runbook items never do. Settings → Personalization → Learning
  suggestions (`learning-settings.json` in app data; settings key
  `learning.suggestions`; actions `learning.get_learning_settings` /
  `set_learning_suggestions`; an open Settings → Personalization reads it again
  when an agent changes it, through `app-changed` and
  `store/settingsRevision.ts`): `off` (none from the extractor, agents or a
  project scan), `important` (default: the extractor returns `impact`, only
  the first `high` is queued, at most one per exchange;
  `vault_learning::HIGH_IMPACT` defines the bar) or `all` (the old
  up-to-three). A turn that already proposed one (e.g. Teach Hollow) skips
  extraction. An agent's `learning.submit_learning_proposal` goes through
  `vault_learning::submit_agent_proposal`: from the person's own typed turn
  whose words ask to teach (`mark_person_run`/`person_run`/`Asker::of`,
  depth 0; `asks_to_learn`, see below) the
  mode doesn't apply, so Teach Hollow always works; otherwise (their other
  typed turns too) refused when off, and under important from a chat
  another agent started or unless `impact: high`.
  Always one new proposal per run (the caller's run id, overriding
  `sourceRunId`; a rejected or saved duplicate is refused), the
  extractor's duplicate screen, review source `agent`.
  The old `propose_learning_from_text` (any text became source-`chat`
  learnings, uncapped) was removed on 2026-10-12; nothing called it.
- **Rejections teach too (2026-10-11).** Every model or agent proposal
  passes `vault_learning::screen`: first a free local check (`Known`:
  approved current learnings, pending/snoozed proposals and the person's
  rejected ones; `words` keeps numbers like `3.11` and two-letter words,
  reads never/don't/no/avoid and negative contractions as one `not`;
  `paraphrases`: Jaccard ≥ 0.6, or every word of a 3+-word statement found
  in another at most twice its size), then the optional Jev check. A
  paraphrase is dropped (an agent is told whether it repeats a saved,
  pending or rejected one), except when the person asked
  (`Asker::asked`): then it is queued with the near match as
  `possibleDuplicate` on its card (Teach Hollow's correction of a saved
  learning). `asks_to_learn` is narrow ("teach hollow", "remember that",
  "remember:", "from now on") and carries once to the person's next typed
  turn in the same conversation. Exact rejected fingerprints were already
  sticky (`review_queue`).
- **Shared scan rules (context layers CX1, 2026-11-13).** One rules list for
  secrets, one computer's facts, one person's facts, employee data, data files
  and the size limit lives in `example-org/org-context` at
  `tools/ctx-scan/scan-rules.json`, beside the standalone `ctx-scan` (Python
  standard library, runs without Hollow) and a fixture corpus. Hollow vendors
  that whole folder at `vendor/ctx-scan/` (copy with a check):
  `node scripts/vendor-ctx-scan.mjs <org-context checkout>` copies it and
  writes `VENDORED.md` with the source commit and each file's SHA-256, and
  `scan_rules::tests::vendored_copy_matches_its_record` fails when a vendored
  file is edited here. Change the rules by pull request there, then re-vendor.
  `scan_rules.rs` `include_str!`s the JSON and runs it with the `regex` crate:
  `scan_text`/`scan_path`/`scan_file` take a layer (`home`, `personal`, `org`,
  `project`, `vault`) and return rule, line and column, never the text. Its
  tests read the same `fixtures/expected.json` and `fixtures/cases.json` the
  Python tests read, so the two scanners can't drift. `memory_guard`'s stored
  secrets come from the `secret` group (layer `personal`, `context-scan: allow`
  lines ignored, so a memory can't allow its own secret); its rule names are the
  shared ids (`secret.anthropic`, `secret.assignment`, ...). The vault repo's
  first-commit scan (two-Macs V0/V1, layer `vault`) and the per-layer write
  rules and leak checks (CX7, CX10) call the same module instead of keeping a
  list of their own. The context sync (`sync_engine`, CX6a) scans each staged
  blob with `scan_bytes` (`scan_file` for bytes in memory) before it commits. Patterns stay in the syntax Python `re` and Rust `regex`
  share, every repeat bounded.
- **Memory file schema (PX4, 2026-11-16).** `memory_guard::lint_memory(stem, text)`
  checks one `memory/<stem>.md`: a `---` block with `name` equal to the file name
  without `.md`, a non-empty `description`, `type` in `user | feedback | project |
  reference`, and optional `verified_at` and `valid_until` (ISO date or RFC 3339).
  `type` and the two dates may sit at the top level or under `metadata:`; Claude
  Code's `node_type`, `originSessionId` and every other key are optional and left
  alone. Each issue carries the fix and never the file's text. `normalise_memory`
  fixes the drift it can (name set to the stem or added; a top-level `type`,
  `verified_at` or `valid_until` moved under `metadata:`, a duplicate that agrees
  dropped, CRLF kept) and returns `Err` with what only a person can supply
  (description, type, a bad date); normalising twice changes nothing. The write
  guard applies it in `check_layer*` for a Personal write to `memory/<file>.md`
  when the writer passes the file's text as the field `MEMORY_FILE_FIELD`
  (`"memory file"`): a break is refused with the fixes named. `README.md`,
  `MEMORY.md`, `INDEX.md`, deeper paths and writes without that field are not
  linted. No production writer passes the field yet (PX2, PX3 and EV1 are the
  readers of the schema). A test replays the 32 real files' frontmatter
  (`memory_schema_heads.txt`, descriptions masked); two of the 32
  fail the raw lint (a sentence for a name, a dashed name for an underscored file)
  and all pass after normalising. **`pinned: true` (PX3, 2026-11-16)** is a further
  optional key, top level or under `metadata:`, read by `context_pins.rs`: a pinned
  memory is loaded whole in every fresh chat's digest as `personal/pinned.md`, within
  a shared 1,500-byte budget (file-name order; a pin that would pass it is not
  loaded and is named with its size in the section's last line). The lint and the
  normaliser leave the key alone. Three safety memories (`SAFETY_RULES`) are also
  offered once, as Waiting on you cards, for promotion to `me/Guardrails.md`.
- **Per-layer write rules (context layers CX7, 2026-11-14).**
  `memory_guard::check_layer(layer, machine, path, fields)` runs the safety scan
  and then the layer's rules (plan section 4): **Home** only `home/<own slug>.md`
  and `home/<own slug>/<topic>.md` (another machine's file, a path outside
  `home/`, an unset machine slug and a copy of the `<!-- hollow:generated`
  header are refused); **Personal** writes `memory/**` and `learnings/**`, holds
  any other `me/` file for Save (`Verdict::Card(WaitsForSave)`) and always cards
  `me/Guardrails.md`, `me/Decisions.md` and `STANDING_ORDERS.md`
  (`AlwaysConfirm`, names matched case-insensitively), refusing other paths;
  **Org** refuses every direct write and points to `context.propose_org_change`;
  **Project** refuses a line the shared `machine` rules or
  `personal.private_email` match, each field alone and all joined (project
  layer, `context-scan: allow` lines ignored: a text can't allow itself),
  naming `home/<slug>.md` or `me/`/`memory/`. `personal.owner_term` (the
  owner's name and handles) is **not** scanned here: this module has no owner
  terms; `ctx-scan` still checks it at commit time. Home and Personal accept
  ASCII names only (a name like `Guardrailſ.md` could fold onto a guarded
  file), `memory/` and `learnings/` take `.md` notes only, and
  `check_layer_in_root` also refuses a path whose canonical parent leaves the
  layer root through a symlink (`check_layer` alone is lexical: writers open
  no-follow). Refusal text (`Refusal::
  message`) names the layer and never the text. `check` itself is unchanged
  (the eight existing callers); `context.propose_org_change` (CX8, `context_propose.rs`) is the Org
  refusal's way out and does its own scan; no other caller of `check_layer`
  exists until the sync engine (CX6), so the rules are enforced from the first
  write they make. `sync_commit_message` formats the
  commit message with one `Actor:` line per chat and `File:` lines (the repo's
  git history is the change log; CX6 calls it). `run_guard::RunFlag` gained
  `scan`: for an `agentInstructions` flag, the project-layer scan result of the
  file as it stands (`rule (severity, line N)`, never text), flag-only.
- **Every memory write is scanned and logged (overhaul J1, 2026-11-08).**
  `memory_guard.rs`: `scan`/`check` refuse prompt injection (instructions aimed
  at the next model, fence breaks like `</trusted-recall>`), credential
  exfiltration (a stored secret, a command that reads one and sends it out, an
  instruction to put one in the open; a warning such as "never print the API
  key" passes) and invisible Unicode (zero-width, bidi, tag, filler and control
  characters; an emoji's own joiners pass). A finding carries class, rule and
  position, never the text, so a refusal can't echo a secret. The scan runs at
  `vault_learning::submit_proposal_from` (every capture, and early in
  `submit_agent_proposal`, before the duplicate screen sends words to a model),
  at accept (also the person's edit of the words) and at `profile_builder`'s
  `write_fact` (the fact, its label and its section heading). The log is `{app data}/memory-log/{vault stem}.jsonl`, outside
  the vault, so the vault tools an agent works with don't reach it (it is not tamper-proof against anything that can write the file: the chain is unkeyed and `ready` checks only the last line, so it catches damage and edits to old lines on `verify`, not a rebuilt chain): append-only, 0600, each
  entry with actor, chat (run), action (`captured`, `saved`, `edited`,
  `promoted`, `undone`, `refused`), the note path, before and after (cut at
  8,000 characters, hashed whole) and the hash of the entry before it.
  `verify` rechecks the chain; `ready` checks the last whole line, and a log
  whose last line no longer verifies refuses the next memory write (accept,
  undo, About you add, capture) instead of letting it go unrecorded. An append
  that never finished is cut by the next `record` (and a failed append cuts
  itself), so a torn tail never bricks the log. An agent's repeated refusals log
  five per source, the fifth saying so (`record_refusal`). A refusal message
  says the class, the field and the character where the finding starts.
  Negation right before an imperative ("never bypass the permission checks")
  makes it a warning the scan lets through (matches are searched from every start, so a warned imperative can't hide a plain one after it); capture, accept and undo share one `LEARNING_WRITE` lock; undoing a save is refused while its note differs from the logged saved text; `ZWNJ` and a keycap's `FE0F` are
  read as text. Known gaps: no confusable or fullwidth-letter normalisation,
  and a send-out command split across two lines. A refusal's entry has the
  classes and no text. `LearningProposalPayload.edits_path` and `edits_hash`
  make a proposal an edit of a saved learning: Hollow pins the note's SHA-256
  at submit (an agent's value is cleared), accept rewrites that note in place
  through `memory_guard::edit_pinned` (refused, note untouched, proposal back
  to pending, refusal logged, when the note changed meanwhile), and Undo puts
  the logged text back only while the note is still the one the edit wrote.
  Nothing builds these edits yet; J5 (duplicates and conflicts caught at save)
  is the caller. `supersede_context` marks a note and isn't a content write,
  so it isn't logged.
- **Memory candidates are versioned and every capture reports (ME1, 2026-11-18).**
  A learning proposal is a candidate in `review_queue`; its version is
  `review_queue::version_of` (a short SHA-256 over id, status, `updatedAt`,
  snooze and payload, computed on read, never stored). `LearningProposal.version`
  and `list_review_inbox`'s `version` (`VersionedItem`) carry it to the cards.
  Library's Save (`resolve_learning_proposals_batch`, `expectedVersion`) and
  Waiting on you's Save (`resolve_review_item`, `expectedVersion`) send it back;
  `vault_learning::resolve_batch_versioned` / `resolve_proposal_at` refuse an
  accept at a version the item moved past (`review_queue::CHANGED_SINCE_READ`,
  nothing written) and edited words with no version. Every accept also claims
  through `review_queue::claim_at` at the version it read, so a change between
  its read and its claim is not overwritten either. Each extractor run that
  reached the model with at least one candidate (`kind` `capture` or `exchange`) appends one JSON line to
  `.hollow/memory-reports/<UTC day>.jsonl` (`memory_report.rs`, kept 30 days):
  `added`, `updated`, `removed` and `skipped`, each `{id?, targetKind,
  targetPath, text?, status?, reason?, similarTo?, detail?}`; text is capped at
  300 characters and withheld when the memory scan flags it. Reason codes:
  `near_duplicate` (with `similarTo`), `already_in_profile` (PX5, below), `duplicate_check` (Jev),
  `already_proposed`, `ungrounded`, `not_stated_by_person`, `other_machine`,
  `unsafe`, `invalid`, `target_refused`, `one_per_capture`, `write_failed`,
  `sensitive` (ND7: health, religion and the like, not asked) and `tentative` (seen
  once; saved when a second message repeats it, `memory_provenance.rs`).
  **No merge on similarity:** a near-duplicate is skipped and reported, never
  folded into the memory it resembles; only a person's Save changes a saved
  memory.
- **Eval candidates (NV7, 2026-11-18).** `eval_candidates.rs` files a chat turn
  that missed a skill as a review item of kind `eval_candidate` (source `chat`):
  the ask redacted first, the answer never kept, fingerprint SHA-256 of the
  redacted prompt and the workspace fixture's hash. Its accept, through
  `resolve_review_item`, needs `expectedVersion` (claimed with `claim_at`) and a
  reviewer-written `expectedSkill` in the edit (empty is refused, nothing
  claimed), then writes `<Main>/outputs/eval-cases/nv7-<key>.json`. An agent
  files at most 2 new ones per run and none while 20 wait.

- **One home per fact (PX5, 2026-11-18).** `memory_homes::profile_line` finds a
  statement among the `me/` profile notes' list lines by exact words after
  normalising both sides (trimmed, whitespace collapsed, a bullet or `1.`/`1)`
  mark, heading and bold marks dropped, lowercase, a short `Label:` allowed);
  never by similarity. **On write:** a captured candidate that says such a whole
  line is skipped and reported as `already_in_profile` with `similarTo` `<me
  file>: <line>`, and an agent's `submit_learning_proposal` is told "It is
  already a line in me/X.md. Nothing queued." A match only behind a `Label:`
  (the label is part of the fact: `**Slack:** Keep messages short` is not the
  same rule for email), or one the person asked for, is queued with that line
  as `possibleDuplicate` instead. **Retire card:** opening Learnings
  (`memory_maintenance::refresh`) and the launch offer (`context_pins::start`)
  run `vault_learning::offer_profile_copies`: one You learning proposal per
  current `learnings/` note whose statement is a line in an `OPERATOR_TARGETS`
  note, with `payload.retiresNote = {path, sha256}` (the note pinned when the
  card is made), source `memory`, fingerprint including the note, so a card
  rejected or still waiting is not filed again; at most 5 new cards per offer. **Accept retires:** any You
  proposal whose words a current learning note says carries that note in
  `retiresNote` (Hollow sets it at submit; a caller's value is cleared), and its
  card says "Save also retires ...". Save writes the `me/` line (or settles on
  the one there, `alreadyIn`) and then, in the same accept, rewrites the note in
  place through `memory_guard::edit_pinned`: frontmatter kept plus `status:
  superseded` and `superseded_by: <me file>` (out of recall and retrieval; a
  link to a non-learning is never followed), body one line linking to the
  `me/` line; logged as `promoted` with the text before. The note is kept as a
  link rather than deleted so links to it still resolve and Undo can put it
  back from the log (only while it is still the retired text). Other words on
  such a card are refused (they would add a second line), a note changed
  since the card was made is left alone, and a note over the log's 8,000
  characters (`memory_guard::MAX_TEXT_CHARS`, so Undo couldn't restore it) is
  not retired; the Save says why and the card stays pending.
  PX3's promotion of a `memory/` file retires no memory file: Claude Code owns
  `memory/` and its index. Tests: `vault_learning::tests::duplicate_learnings_*`,
  `promoting_a_learning_*`, `a_repeated_me_line_is_flagged_on_write`.
- **Accept saves into one home by scope (overhaul J2, 2026-11-08).**
  `memory_homes.rs` and `vault_learning::accept_one`. A proposal's
  `targetKind` is its scope: `operator`/`decision` is **You**, one list line in
  the `me/` note it names (the About you builder's `insert_fact`, through
  `profile_builder::add_line`; no `learnings/` note any more); `person`,
  `project`, `goal` is **Know-how**, the `learnings/` note accept always wrote;
  the new `workspace` is **Project**, one bullet under the `## Decisions…`
  section of `workspaces/<id>/WORKSPACE.md` (written through
  `shared_docs::update_doc`, the compare-and-swap Hollow's other brief writes
  use) or under `## Learned in chats` in `workspaces/<id>/AGENTS.md`. The
  target must be one of those two files and exist. A chat's task state is the
  fourth place and is derived, never written, so no kind aims at it. A You or
  Project statement is written as one line (whitespace collapsed, no leading
  list mark, at most 600 characters; a longer one stays pending with "Shorten
  it and save again"; the scan reads the one line as written too, since it
  reads line by line). **One fact, one home:** before the write
  `memory_homes::already_in` looks for the fact (words lowercased, list and
  bold marks dropped, also behind a short `Label:`) in every `me/` profile
  note, plus the current `learnings/` notes on the same target for Know-how
  (the same words about another person or project are another fact), or the
  target workspace's two files for Project; a hit
  stores nothing and the accepted proposal says where in `payload.alreadyIn`
  (nothing is logged: nothing changed). A fact already in `learnings/` can
  still move up into `me/`; the gardener lists the copy later. A proposal's own
  earlier save (the queue write failed after the line was written) is
  recognised from the log (`saved_by`) and finished, not repeated. The
  accept's log entry is `saved` at the home note with the line as `after`;
  `payload.learnedPath` is the home note and `payload.learnedLine` the exact
  line, so Undo (`undo_line`) takes out the last list line that says exactly
  it and logs `undone` (a line the person already removed is nothing to
  remove; the same compare-and-swap reopen-first, restore-on-failure order as
  a trashed note). Writes and Undo skip fenced code and frontmatter (a fence
  closes on its own character and length), match the line exactly, refuse a
  home that is a symlink, and keep a bullet that only looks like a template
  placeholder (`- (Always cite sources)`) that About you's own writes would
  drop. The duplicate screen (`Known`, the Jev check) also counts an accepted
  You or Project learning, which is a queue entry with `learnedLine` and no
  `learnings/` note. (CX9 below added `home` and `org` to the extractor's list.) Agents may submit `workspace` through
  `learning.submit_learning_proposal`, and the capture rewrite (J6) will use
  it. Tests: `memory_homes::tests` on a throwaway vault.
- **Home and Org scopes (context layers CX9, 2026-11-15).** `Scope::Home`
  (`targetKind` `home`) and `Scope::Org` (`org`) sit beside You and Know-how
  and have no vault path (`home_path`/`write_line` refuse them,
  `validate_payload` never accepts the kinds, so the accept flow cannot reach
  them). The extractor prompt carries `vault_learning::LAYER_TEST` (the
  plan's four questions in order) and its `targetKind` list ends `home|org`.
  A `home` or `org` item the person's own words state (`stated_by_person`, as for an automatic Know-how save; Home also only for a conversation with no `remoteRun`) is diverted in `propose_exchange` to
  `memory_homes::route_layer_fact`: Home appends a line under `## Learned in
  chats` in this Mac's `home/<slug>.md` in the personal root
  (`save_home_fact`: `check_layer_in_root(Home)` first, no-follow resolve,
  duplicate line is `AlreadyThere`, a `saved` log entry by `host-capture`;
  another machine's file or a scan hit is refused with the layer's sentence);
  Org builds a one-file `context_propose::Request` (`<org folder>/<subject>.md`,
  folder from `targetPath` else `business-rules`), runs the proposal's own
  `validate` and `scan` now, and stores it as a `PendingOrg` in
  `context-org-pending.json` (app data, 50 at most, same fact once; nothing is
  written to the org clone and nothing pushed). `PendingOrg::request(attended)`
  hands it, with the org clone appended to rather than replaced when the file exists, to `context.propose_org_change`, which keeps its card (CX20's review
  card submits). `profile_builder` reads the union of the vault and the personal
  root (vault first) for `me/` and for `learnings/` plus `memory/` (`personal_memory_lines`); its writes still go
  to the vault until the migration. Tests: `memory_homes::tests`,
  `profile_builder::tests::sources_read_the_personal_repo...`.
- **The filesystem is evidence.** Chat, Workflow, Heartbeat and agent-backed
  Ingest take the exclusive full-vault lease (`automated_history::try_begin_run`)
  before launch (workspace turns take a scoped one) and finish the same run on
  every path. A receipt's changed files come from that before/after diff;
  only completed structured tool events confirm attribution and checks, never
  assistant prose. A run whose cwd is a git repository also records that
  repository's changes (`repo_diff.rs`). Restore stays vault-only.
- **Automated runs cannot escalate their own automation.** A workflow whose
  stamped permission fields or prompt changed outside the editor is refused
  until approved. A run's changes to workflow permission fields, `me/**`,
  agent instructions (`CLAUDE.md`, `AGENTS.md`, `CLAUDE.local.md`,
  `AGENTS.override.md`, `.claude/CLAUDE.md`), `.claude/settings*.json`,
  `.codex/config.toml`, watched dot-state or its own History record are
  flagged (`run_guard::RunFlag`) with one notification, computed from an
  in-memory baseline the agent can't touch. Only workflow changes are refused.
- **Restore is conservative.** Normal file/run restore requires the current
  hash to match the recorded after hash and rechecks it immediately before the
  write. Whole-run restore preflights all paths and writes nothing on any
  conflict. Force is a separately confirmed action; both paths snapshot current
  content in a new `historyRestore` run first. The next tracker pass may replace
  a restored tracker mirror.

At startup `reconcile_automated_history` converts abandoned `running`
manifests to `interrupted` diffs and clears orphaned run leases. Retention also
runs during finalization. Do not manually edit `.hollow-history` or treat
`.evidence-receipts.json` as a complete shell audit: checks are deliberately
labeled observed, not exhaustive.

Archive: vault mirrors, schemas, hidden state, trust and tasks.

---

## Tasks & Projects (retained capabilities)

[Tasks in the workspace shell](../TASKS_WORKSPACE.md) defines the current
page. Tasks keep their files in `vault/tasks/` (schema above). Quick-add parses
natural language (`quick_add::parse`); `canonicalTaskStatus()` (`taskEdit.ts`)
groups status synonyms; `components/TaskEditor.tsx` is the one task form;
readiness (`blocks`, `ready`) comes only from `vault_tasks::list`. The full
Todoist-style feature notes are in the
vault archive.

---
### Grounded Know-how and private provider notes (2026-11-08)

Host captures may save a positive Know-how statement that occurs in the
person's own messages in that chat, passes the safety scan and has no in-place
edit. A lesson quoted only from a tool result is proposed and waits for Save,
because a tool result can echo an email, a file or another person's words.
A statement or quote that names an email address, or says anything negative,
waits too: `negative_statement` matches whole words, so any `n't` contraction
(curly apostrophes included), `no`, `never`, `fails`, `error`, `missing` and
the other `NEGATIVE_WORDS` keep it pending, while "notable" and "known" do not.
Grounding is a substring match, so the sentence of the person's message that
holds the quote, and the one that holds the statement, must pass the same
screen (`negated_in_source`): "Don't preserve existing CSV headers." keeps
"preserve existing CSV headers." pending. A sentence ends at `.`, `!` or `?`
and a space, or at a line break, when the next word doesn't start in lower
case, so a hard-wrapped line stays one sentence and a negative in another
sentence doesn't count. A lead-in carries into what it introduces: the
sentence ending a line with `:` is read with the next line and the list items
(`-`, `*`, `+`, `1.`, `2)`) after it, until a line that isn't an item starts a
sentence or follows a blank line, so "Do not:\n- Preserve existing CSV
headers." stays pending. The screen keys text with `statement_key`, the same
function grounding uses, so case folding (a Greek final sigma) can't hide the
quote from it.
The log names host-capture and the proposal retains a single-item Undo batch,
`learning-batch-auto-<id>`. Library and the reply line show "Undo saved
learning" only for that batch (`autoSaveUndoBatch`); a bulk Accept is undone
with "Undo last batch", never from one card. A refused Undo in the reply line
shows its error beside that line's actions (`undoErrorKey`) until the
learning is no longer saved.
You and Project facts, agent submissions and private-memory promotions wait
for Save. No old queue is migrated or automatically accepted on installation.

Library Learnings reads private memories only on an explicit click for the
selected project. Claude's exact encoded project memory directory and matching
Codex MEMORY.md task groups are supported. A Codex group is shown only when
every cwd it names (`cwd=A; cwd=B`, or a rollout's `cwd=A,`) is one of the
project's folders, so a group shared with another folder is shown to neither.
The project's folders are its vault folder plus its cwd and build repository
when those are absolute; an empty or relative cwd is skipped. Files above
2 MB and symlinked paths are refused; at most 80 scrubbed excerpts of 6,000
characters are shown per provider (Claude and Codex each). An exact passage
of 12 bytes to 2,000 characters is pinned against a fresh read before it
becomes a pending proposal. Provider scratch files never become automatic prompt context and
are never changed by promotion. The direct Know-how target kind is `knowhow`
with targetPath `learnings/`; older person/project/goal targets remain valid.
