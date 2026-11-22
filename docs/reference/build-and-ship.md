# Build & ship

## Changes freshness

`BuildStatus.blocked` and `AppBuildStatus.blocked` are nullable sentences from
the existing backend git and install guards. Changes disables actions and shows
that sentence before a confirmation; each mutation rereads status before opening
its dialog. Backend guards remain the final authority. Install also binds the
finished receipt and drops stale or out-of-order reads; job events trigger an
authoritative reread rather than clearing a busy refusal from their partial data.

`workspaceWorking(cwd)` includes the open conversation, background conversations
and known queued/running chat rows. The backend covers unmapped automations.
The lazy `components/buildFreshness.ts` watcher rereads on queue transitions
(even if another lane remains busy), domain-level `build-changed` events, window
visibility restoration, and every 15 seconds while the panel is visible.
Queue events have no folder, so they reread the current panel's authoritative
guard. Hidden panels skip periodic reads; unmount cancels listeners and responses.
Changes shows the last successful status read time beside Refresh. External git
changes appear on the next visible poll; timestamps never imply an external fetch.

## Quit and installs

The app menu's custom Quit item and standard ⌘Q binding, tray Quit and closing with keep-running off
reach `RunEvent::ExitRequested`. When live or queued work exists the window
offers Quit and stop N runs, Keep running in the menu bar, or Cancel. A failed
work read asks rather than treating the count as zero. Install/setup exits and
an explicitly accepted quit bypass the dialog. Close prevents window destruction
while the quit decision is pending. Only the user can resolve that decision.
The queue/registry read runs off the event-loop thread. A pending request has
an id and underlying run identities; webview reloads read it again, repeated
quits re-emit it, and confirming rechecks for newly started work before exit.
Cancel and Keep running invalidate an in-flight redundant check. Expected exit
is a one-request flag, consumed by ExitRequested.
Do not use Cocoa's predefined Quit item: it invokes `terminate:` directly and
bypasses this guard in tao 0.35.2. macOS Dock Quit, logout/shutdown and Force Quit
are outside this app-controlled path; do not claim that those show the dialog.

**Install flow now: the Update button** (row U10, 2026-11-15). Every install,
a channel update or a branch build, goes through the top bar's Update button:
Restart to update, or Update when chats finish (no expiry, Cancel on the
button, in memory only: quitting discards it), then `hollow install-update`
("In-app installer" below). A finished build from Changes → Hollow app is
offered there as "Local build of <branch>"; an agent-approved install
(`build.install_built_app`) offers the approved receipt's build and starts
its wait. The scheduled-install ticket, its banner, `scheduled-install-event`,
`pending_self_install`, `cancel_scheduled_install` and the 30-minute timeout
are gone. Details: [docs/UPDATES.md](../UPDATES.md), "Local builds (U10)".

A confirmed restart: `exit_running_work` returns `{key, name}` rows of running
work, and `setup_restart(confirmedKeys)` checks them again off the event-loop
thread before marking exit expected; `RESTART_WORK_CHANGED:` reports work started
since the confirmation. A fallback busy label is only needed for short non-run
leases: scoped/full-vault history runs expose their real identities alongside
queued/live chats, even when they overlap. Work without a stable run identity
refuses restart until it finishes. Native mismatch retries stop after three
attempts. Its one Settings caller, the Notion connection editor's Save now,
restart later dialog, went with Notion sync on 2026-11-10. Wizard and explicitly
confirmed agent restarts omit `confirmedKeys` and keep their existing behavior.
The interrupted-chat offer queries recent
recovered rows across launches, preserving the 24-hour, latest-turn, attended,
native-session and existing-folder checks.

Part of the agent manual. [AGENTS.md](../../AGENTS.md) is the entry point and
links every reference doc; this one holds "Build & ship" (a subsection of
CLAUDE.md's "Running the app") and "Self-install CI compatibility", moved
there word for word on 2026-10-13 (only relative links were re-based). Read it
before building, installing or recording an install (the **Current install**
paragraph lives here now), and for the evaluation benches, versioning,
signing, config lookup, the disk guard, diagnostics and settings
export/import. Keep it current with the code in the same change, as AGENTS.md
asks.

### Build & ship (how a change reaches the installed app)

**What a finished build brings (R15a):** Changes → Hollow app loads a comparison
once per finished receipt. The details list PRs, direct commits, missing
build-log entries and notes about checks not done by hand; the Update button's
Local build notes repeat the list as plain text (U10). The comparison uses the
receipt's `base_commit..commit`, not a moving branch tip, and reads BUILD_LOG at
that target. The base is the installed commit when the build finished, not a
fresh install-status assertion. Unavailable history stays an explanatory note;
dirty receipts warn that uncommitted changes are outside the commit list.
`build.install_brings {id}` shares the UI's `app_build_brings` implementation
on the blocking pool. Omitting id retains the existing origin/main report and
ancestor requirement. No fetch, build or install happens while listing.
Without an id, it and `build.install_ready` read the checkout `hollow_repo.rs`
picks: the build workspace repository named by the last install receipt (`last-result.json`) when that install succeeded, else
the build workspaces' Hollow checkouts (live before removed, newest first, one
holding the installed commit preferred), never path order. Both name it (`repo`,
`repoSource`); when its `origin/main` lacks the installed commit, the note (or a
reason, with `ready` false) says so. An unknown installed commit is said the
same way, but doesn't hold `ready` false.

**HTTPS fetch fallback:** when updating `refs/remotes/origin/main` from an explicit GitHub URL, use
`git fetch --no-prune <url> main:refs/remotes/origin/main`. This checkout's pruning configuration can delete that
tracking ref before trying to update it if `--no-prune` is omitted. A plain URL fetch to `main` updates only
`FETCH_HEAD`; compare/rebase using the fetched SHA rather than assuming `origin/main` moved.

**No test bed (Alex, 2026-10-09).** A change ships on a green
`npm run gate` (plus CI whenever GitHub billing lets it run), then an install
with Alex's OK. The candidate test bed was dropped for good and its driver,
scripts and bed rows were removed on 2026-10-12; its record is
docs/history/TEST_BED.md. `tauri.candidate.conf.json`
and the candidate isolation in `instance.rs` stay for side-by-side review
builds. **Paid evaluation rounds** have no runner since the bed's pilot went;
the suite (`scripts/agent-eval/pilot-tasks.json`, its seeded vault in
`scripts/test-bed-vault.mjs`) and the scorer stay. A round is
scored from its folders, on each task's named artifact, by
`node scripts/agent-eval/score.mjs score|compare` (Wilson intervals per
slice, paired comparison with drop and p95 flags; reads `run.json`, the
artifacts and the run's F11 `context-manifest.json`, reporting an absent one
as absent, never as zeros). The Jev study (four conditions, local against
Jev on 22–26 judged items each in `scripts/agent-eval/jev-fixtures/`) is run
by the binary's hidden `hollow jev-study` (production code paths; a loopback
`--endpoint` or an explicit `--typesafe`, else it refuses) and scored by
`node scripts/agent-eval/jev-study.mjs score <dir>` (P/R/F1, explicit-name
and cardinality failures, latency, requests, paired bootstrap intervals).
docs/history/AGENT_EFFECTIVENESS.md.
The **vault-wide bench** (overnight A14, `vault_bench.rs`): from `src-tauri/`,
`cargo test --lib vault_bench -- --ignored --nocapture` builds 500 workspaces
and 30,000 notes and times the vault-wide calls (`wide_calls`), the link
index's phases (`index_phases`) and each Waiting on you source
(`needs_you_kinds`). `build_index` reads and parses notes on up to 4 threads
(vault order kept) and `parse_wikilinks` is a hand scan pinned to
`WIKILINK_RE` by a randomized comparison test; the bell's conflict-copy scan
is cached by `vault::walk_generation()` (its own uncached walk, 60 s safety
TTL: the watcher skips `_` folders). Gate tests: a 3,000-note cold index
build takes the threaded path and stays under 3 s, and the hand scan stays
faster than the regex side by side.
The **preparation bench** (F8, wave 3) is free and deterministic: from
`src-tauri/`, `HOLLOW_PREP_BENCH_OUT=<dir> cargo test --lib prep_bench --
--ignored` (`prep_bench.rs`: a generated 612-note vault, a chat turn's
preparation steps, retrieval and connector selection through the production
stage (O2), against the loopback fake Jev in `off`, `score`, `timeout` and
`malformed` modes (a fresh ranking guard per sample), `score-shared` and
`timeout-shared` (one guard: reuse, the breaker), `semantic-slow` (a 10 s
semantic stand-in), and (O3) `codex` and `codex-cached` (a Codex command whose
native inventory a stand-in CLI, `codex_inventory::fake`, lists every sample,
or once through one kept cache), 20 samples each by default), then
`node scripts/agent-eval/prep-bench.mjs <dir>` for p50/p95 per step, the
`stage` (retrieval and connectors together) and in total, the deadline's
cuts and how Codex's inventory was served (it reads run manifests the same
way).

The **efficiency scorecard** (evening 2026-10-16, HF01) is free and reads only
what Hollow already records: `node scripts/agent-eval/scorecard.mjs summary
--vault <v> [--since <iso>] [--until <iso>] [--json <file>]` (since defaults to
7 days back) joins the sessions database's usage ledger, run queue, turns,
stints, steps, delegations and friction tables, the vault's
`.evidence-receipts.json` and `.evidence-manifests/`, and Claude transcripts
under `~/.claude/projects` (`--transcripts <dir>`, `--no-transcripts`) by
native session. Codex runs (overnight 9 A14) join their thread's rollout
under `$CODEX_HOME` (else `~/.codex`)`/sessions` (`--codex-home <dir>`;
`--no-transcripts` skips it too) by the same native session: only
`token_count` lines (one Claude-shaped call each: uncached input, cached as
cache read, no cache writes; a repeat of the same totals dropped),
`compacted` lines (counted) and tool-call items (their name only) are parsed.
They fill the `chat/codex` group's `transcripts` block, a per-group `codex`
block (compactions, compacted runs, tool calls by name) and a top-level
`codex` block (turns; first call by session and resume gap, its cache-write
figures null since Codex writes no cache; `coldResume`, the over-60-minute
resumes' first-call cache read and cold share; handover context over Codex
predecessors). The Claude tool pass leaves Codex runs, and handovers from
them, out. A Bash failure head skips a leading `set -euo pipefail` ending in a
newline, `;` or `&&`. Runs count by when they started; `--overlap` counts every run
whose `[started_at, at]` overlaps the window (a `running` row's `at` is its
last checkpoint, so its span is open-ended; an `interrupted` row's `at` is the
reconciler's sweep, so it counts only when it started inside). Either way a
`straddling` block lists the runs that started before `--since` and ended
inside it or are still running (with cost; never an interrupted one),
`counts.stillRunning` the running rows in the groups or straddling, and
`longRuns` the top-level runs over 30 minutes (a lane's 2-hour turn), which
count in their group's runs and cost but stay out of its latency percentiles
(interrupted rows never qualify: their `at` is the reconciler's). Per surface
and agent: runs and outcome rates (over finished runs, rendered with their
count and n, `50.0% (3/6)`); p50/p95 of queue wait, preparation, retrieval,
spawn, first text, turn, the history snapshot/diff and the ledger's own
start → end (`wallMs`); cost p50/p95 over runs whose status is
`reported` or `estimated`, with every other run counted apart (a `partial`
row's cost too) and the known sum a floor, never a total. Jev requests (agent
NULL, scope and model `jev`) are the `oneshot/jev` group, counted
`unpricedByDesign` rather than `unknownRuns`, with their latency and outcomes
on a line of their own; cache-read share;
tool errors per top-level turn; retrieval notes and bytes read and the
prefilter's pass rate; from transcripts, calls per turn (subagents included),
main-thread context per call, cache writes, the cost share of calls at 400K
context or more (weighted by Anthropic's relative token rates) and a ledger
reconciliation (interrupted rows are skipped: their end is the reconciler's).
Also native-session resets per 100 turns by cause, counted when they happened
(a stint's `reason` is written when it closes), each turn under the agent of
the stint that covers it (not the session's current agent), with mode-flip resets (config-changed resets whose
turn ran in another permission mode than the one before), `resetsByPart` (each config-changed reset under every
OB03 part whose hash differs between the closed stint's `config_parts` and the session's next stint's; `unknown`
when either side has none or nothing differs; usually what the notice named, not for a pending reset or a fresh
attempt that never recorded a native id, whose exact parts are only in the manifest's `compiler.resetParts`) and a
Main-agent row
(sessions in `workspaces/main`); injected bytes per layer overall, by the
compiler's session state and entry, and how often the budget cut the
workspace brief; tool errors by tool and the friction families seen (with the store's 30-day count); ask_agent
timeouts and the delegation records; history timings per workspace; and a
`queue` block (S14's lease waits): waits, rows with a waiting episode, and the
highest waits per second of any row with at least 2 waits (`wait_count /
(claimed_at − waiting_since)`, to the last wait when never claimed; the first
wait sets `waiting_since`, so one wait times nothing) with that row's waits
beside it (`maxRateWaits`), overall and by waiting holder.
A **tool pass** (overnight 2026-10-17, OF02) over the same transcripts fills a
top-level `transcripts` block, per Claude chat turn whose transcript joined
(main thread only; `turns` is the n): `toolSearchPer100Turns` and
`toolSearch` (select/keyword calls, turns with one, `selectedByTool` from
`select:` queries, an entry that isn't a tool name counted as `other`);
`alwaysLoad.usedShare` for app_mcp.rs's six ALWAYS_LOAD
tools and `any`; `firstCall.bySession.{new,reset,resume}` (each turn's first
call, in a slice that starts after the previous turn's `ended_at`: cache
write and read p50, `coldShare` = no cache read, with n; new = the session
has no earlier turn; reset = a stint of the session closed with `config
changed`, `resume failed`, `moved` or `invalid session id` at or after the
turn's start and at or before its end (`ended_at`, else its first call),
since announce_reset closes the stint inside the turn; that is the turn the
continuity flip count charges the reset to; `agent changed` is not pinned to
a turn) and `firstCall.resumeByGap` (`lt5m`,
`m5to60`, `gt60m` minutes since the previous turn's `ended_at`, `unknown`).
`coldShare` rarely moves: the system prefix stays cached across sessions, so
even a new or reset session's first call usually reads some cache;
`cacheWriteP50` is the useful signal. `waitOnlyCalls`
(wait_for_session or run_action `sessions.wait` calls, and those whose answer
was `finished: false`, split by the answer's status in
`stillRunningByStatus`: `queued`, `running` (`claimed` included) or
`other`); `bashRefusalRate` per `turns.permission_mode` (failed
Bash results in friction.rs's `permission` class over Bash calls; a result
that starts with `Exit code N` came from a command that ran, so its own
"Permission denied" is not a refusal);
`handoverContext` (the predecessor's last call's context at the handover,
with `unjoined: {noPredTurnInWindow, noTranscript, noSuccessorJoined}`
counting the handovers it and the overlap leave out) and
`handoverRereadOverlap` (successor first-turn Read files the predecessor had
touched with Read, Edit or Write, and `predecessorTouched`, the predecessor
sets' sizes summed; counts only. Main thread only: subagents' reads and
files read through Bash (`cat`, `sed`) are not seen, so it undercounts);
plus `delegation.handover.childFirstTurnOk` (successors
whose first turn ended `success`, over those that ended). It reads only tool
names, ToolSearch `select:` names, Read/Edit/Write paths (in memory),
`is_error`, a Bash command (classified in memory as a poll, an inline Python
edit script or a check run, and dropped), and result text in three cases: a
failed Bash result's first 200 characters, classified and dropped, a clean
Bash result whose command runs a check (matched against
`runner-summaries.json`, below), and a wait answer's `finished` and
`status`. The test parses every fixture line through a guard that fails on
any other read. Additive, so still schema 2; compare prints ToolSearch per
100 turns and first-call cold shares, with — for a side that predates the
block.
The **context anatomy** (overnight 2026-10-17, OF19) adds to that block, over
the same joined turns' main thread. Main-thread transcript files are read
whole for it: the byte size of every prompt, text and thinking part, tool
input (as the JSON the model wrote) and tool result, and of each attachment
that has a `rendered` form; a `compact_boundary` system line empties what
later calls carry. Sizes only reach the report. `cacheWrite` (`tokens`,
`oneHourTokens`, `byCause`: `sessionFirst` = a new or reset turn's first call,
`coldResume` = a resume's first call over 60 minutes after the previous turn
ended, `warmResume`, `resumeUnknownGap`, `incremental` = every other call,
each with its share); `coldResume` (turns, first-call write p50 against the
warm resumes' p50); `carried` (per kind, bytes/4 × the later main-thread
calls that carry it, since the last compaction, with its share of the calls'
context: `toolResults` with `toolResultsByTool`, `bashCommands` with the
`pythonEditScripts` part, `toolInputs`, `firstPrompts` (the thread's first
non-meta prompt), `prompts`, `compactSummary`, `instructions`,
`skillListing`, `otherAttachments`, `assistantText`, `thinking`, and
`measuredShare`, the rest being the system prompt and tool definitions);
`pollOnlyCalls` (every tool use a wait: wait_for_session, `sessions.wait`, a
Bash command opening with `sleep`, a `setTimeout` or a `wait*.sh` once leading
`cd …&&`, `set …;`, `export …;`, `source …;` and `NAME=…;` segments are off;
a `for`/`until` loop that sleeps, or a wait after real work, is not one) and `pythonEditScripts`
(inline `python3 -c`/`- <<EOF` that writes a file, per 100 Bash calls, beside
Edit calls), each overall and per `turns.permission_mode`;
`bashResultBytes` and `toolResultBytes.byTool` (n, p50, p95, p99, total);
`costShareAtLarge` (main thread only; the group key includes subagents); and
`workflowAgents` (Workflow-tool agents, `subagents/workflows/*/*.jsonl`, by
native session over the window, since they run on after their turn ends:
agents, calls, tokens, share of weighted tokens and `usdAtJoinedRate`, an
estimate at the joined runs' known $ per weighted token; kept out of the
runs' calls and the reconciliation, which the ledger can't meter yet). The
Python-edit and poll regexes live in scorecard.mjs; morning-report.mjs
imports them. **Weights changed:** a cache write's 1-hour part
(`usage.cache_creation.ephemeral_1h_input_tokens`) now weighs 2, the rest
1.25 (before, all 1.25), which moved the evening baseline's chat/claude
`costShareAtLarge` 0.3469 → 0.3411 and nothing else in the scorecard. The
morning report shares `weight`, so its per-lane `subagentShare` (and
`costShareAtLarge`) moves with it, as its poll-only count moves with the
shared poll rule: compare a night across the change with that in mind.
**Tool errors per call** (OF20): each group's `tools` adds
`errorsPer100Calls` (beside `errorsPerTurn`: a 130-call single-turn lane
chat looks ten times worse per turn), `expectedNonzero` (a failed step whose
result starts `Exit code 1 (no match)`, or whose last command segment is
rg, grep, test, `[`, diff or cmp with no output), `failures` and
`failuresPer100Calls`, `checkFailed` (clean exits OF16's step spine marked
as a failed check) and `maskedFailures` (clean Bash results whose command
runs a check, and those whose output holds a failure line from
`scripts/agent-eval/runner-summaries.json`, by runner). A command runs a
check (`isCheck`) when one of its segments' heads is a runner: vitest, jest,
tsc, eslint, pytest; `cargo test|build|check|clippy|nextest`, `go test`,
`playwright test`, `vite`/`tauri build`, `node --test`, `python -m pytest`;
`npm test`, `npm run`/`pnpm`/`yarn`/`bun` with a check script (`test*`,
`gate`, `lint`, `typecheck`, `check`, `build`, `e2e`, `smoke`…); `npx`/`bunx`
of any of those. A runner's name elsewhere in the command doesn't count, so
`grep -n vitest BUILD_LOG.md`, `cat tsconfig.json` and `npm run dev` are not
checks. The list is an allow-list: a check run any other way (`make test`,
`npm --prefix app test`, `if cargo test; then`) drops out of both
`maskedFailures` and `cleanCheckResults`. A subshell's or group's brackets
are read past, so `(cd app && npm test)` counts. A top-level
`toolErrors.byAgent.{agent}.{tool}` (`all` first; the window's turns by the
agent of their stint) has the same counts; a shell tool's entry adds its
failures' pipe and redirect counts and top command heads. A failed step is
read as its command head (friction.rs's rule, a `zsh -lc '…'` unwrapped; a
first word that isn't name-shaped, `[\w.+-]{1,32}` after basename and
lowercase, is `other`, so a quoted JSON blob piped to `jq` is not kept;
`[` and `[[` stay heads, so they still count as predicates)
and booleans only (`STEP_ERROR_FIELDS`; the result's text never leaves SQL).
The summary table gains a `Tool errors/100 calls` column; compare flags a
per-100-call change outside its Newcombe interval either way and prints the
per-agent rates.
**Turn outcomes, the COO's own turns and native memory** (overnight
2026-10-17, SC4). A top-level `outcomes` block (S08) over the window's turns,
every agent, each rate with its `turns`, `n` and `share`: `cancelled`,
`errored` and `timedOut` over `finished` turns (a status other than `running`;
chat.rs writes a timeout as `error`, so an error turn whose top-level chat
ledger row says `timeout` is timedOut; that row says `timeout` for a
no-output watchdog stall too, `Ended::Stalled` in usage_ledger.rs
`outcome_for`, so timedOut counts stalls as well as time limits), `byStatus`;
`stoppedBeforeText` (of
the stopped turns whose phase timings are known, receipt else queue row, those
with no `firstTextMs`: ended before any text, a stop before spawn included;
`unknownTiming` the rest); `reaskWithin120s` (of the stopped turns with an
`ended_at`, those whose next turn in the conversation, asked by the person
(`asked_by` NULL), started 0–120 s after it ended, both bounds inclusive); and
`toolFailuresPerTurn` (`toolErrors`' failures summed over agents, per tool,
over the window's turns). **These are proxies: never auto-tune on them.** A
cancel can be the person changing their mind, a re-ask a follow-up. An
`interrupted` turn counts as finished but never as stopped: the restart
reconciler (session_store.rs `interrupt_chat_run_in`) writes its `ended_at` at
the sweep, not when it stopped, and no person or agent stopped it. `asked_by`
NULL means "the person" only where the column is filled: it exists since
2026-10-07 (older rows are all NULL), and Hollow's own wakes and check-ins stay
untagged until OB20 merges, so until then a wake within 120 s of a stopped
turn reads as a re-ask (`turns.asked` is never read to tell them apart). A
top-level `mainAgent` block (OF23) keys on `turns.asked_by` `hollow:<kind>`
(OB20's tags: `wake`, `checkin`, `weekly`, `wake_me`), `all` and `byKind`:
turns, `cost` (p50 and sum over turns whose top-level chat ledger rows all
carry a known cost, a floor while `unknownTurns` > 0, printed `≥ $x`, and
null, printed `unknown`, when no turn's cost is known, never $0; delegate rows
are not the turn's), `toolCalls`/`toolsPerTurn` (steps), `readSessionCalls`,
`readSessionPerTurn` and `turnsWithReadSession` (a step whose name ends
`read_session`), `refusals`/`turnsWithRefusal` (failed steps whose result is
main_agent.rs's unattended-turn limit, `REFUSAL_LIKE`; the test checks every
`unattended::allows` refusal matches) and `pulseOk` (answers exactly `PULSE_OK`
after trimming, over the turns with a stored answer: `PULSE_OK_SQL`, one
boolean worked out in SQL over `transcripts.payload`'s `$.text`, the only read
of that column; the morning report's SQL guard lets exactly that expression
through and pins it with its own literal copy), plus `suggestions` (the COO's suggestion log,
`<support>/MainAgent/<stem>-suggestions.json` beside the database: `done`,
`not_now`, `never` and `takenShare` over the 30 days before the window's end;
each entry's `at` and `outcome` only; null without a log or for a `--db`
outside a `Sessions` folder). No tagged turn reads `status: "untagged"` with
`all` zeroed, never a missing block (the live store before OB20 merges). A
top-level `memory` block (S13): `nativeWrites` (the joined runs' transcript
Write/Edit/MultiEdit uses into an auto-memory folder, `<transcript
root>/<slug>/memory/…`, any slug since Claude Code keys a worktree's memory by
its repo root, absolute and without `..`, whose result came back without an
error; null without transcripts), `nativeWriteErrors`, `nativeWriteRuns`,
`nativeWritesByWorkspace` (the ledger's workspace id); `receiptMemoryWrites`
(the window's receipts' `memoryWrites`, PR #736, summed over the receipts that
carry it; null when none does, not 0); `learningsAccepted` (the vault's
`learnings/**.md` with `type: learned-context`, `trust: approved` and a
`learned_at` in the window; frontmatter only, the first 4 KB, a `restricted/`
folder skipped at any depth; an undone accept's note is gone, so it counts the ones that
stand; `learnings` adds `approved` and `undated`) and `nativeWritesPerLearning`.
The tool pass now reads MultiEdit's `file_path` too (both guard tests'
whitelists); paths are classified in memory and only counts leave. Additive,
so still schema 2. The summary prints one line per block; compare prints the
re-ask and stopped-before-text shares with their n (and the stopped turns
without timings), the wakes'
read_session per turn and native writes against learnings accepted (— on a
side that predates them; never flagged).
**Ledger call figures, partial-run floor and first model event** (2026-10-17,
OF28). A `ledger` block per group (over every row of its own) and one at the top
level over the window's finished top-level runs only (`isFinishedTopLevel`: no
`parent_surface`, not a `oneshot`/`ingest`/`delegate` helper call, not
`running`), so a child's or a running checkpoint's peak never sets the
window-wide percentiles:
`peakContext` and `apiCalls` (p50/p95/n over the rows that carry OB05's
`peakContextTokens` and `apiCalls` in `usage_json`), `subagents` (`runs` with a
subagents object, their `calls` and `tokens`) and `runs`, the n they are out of.
`usage_json` is read only through `USAGE_STATS_SQL`, a JSON object of those keys
built in SQL (its `raw` block and `note` never leave it; text that isn't JSON
gives the row none of them); the morning report's SQL guard lets exactly that
expression through and pins it with its own literal copy. Each group's `cost`
adds `partialFloorUsd` (OB07's `usage_ledger.cost_floor_usd`, summed over the
runs without a known cost that carry one, never added to `floorUsd` or
`totalUsd`) with `partialFloorRuns` as its n. `latency.firstModelMs` (OB08a's
phase timing, ms to the first model event) joins the latency percentiles, and a
group's `toolsBeforeText` gives the tool calls before the first text
(p50/p95/n; like the latency fields it leaves out long runs). A key no row in
the window carries is null, never 0: Codex and
Antigravity rows carry no call figures, and a store or receipts from before
OB05, OB07 or OB08a carry none, which the summary prints as
`not installed (0 of N runs carry it)`. The scorecard's `sources`
(`usageJson`, `costFloorUsd`) says which ledger columns the store has: a store
with `cost_floor_usd` but no floor in the window prints the floor as
`none in this window (0 of N runs)` instead. The call figures and the first
model event can't be told apart that way (`usage_json` predates OB05, and
receipts don't say), so with none they stay "not installed". Additive, so
still schema 2. The summary prints one line each (the peak context line says
"over top-level runs"); compare prints peak context and API calls p50/p95 with their n, the
partial-run floor and the first model event p50 per group (— on a side that
predates them; only `firstModelMs` can raise a p95 flag, as a latency).
**Pass claims** (2026-10-17, S27). `verification.claims` per group (its
top-level runs' receipts, once per run id, so a failed-over turn's two
ledger rows count its one receipt once) and at the top level (every receipt in the window):
`n` (receipts carrying S27's `verification.claim`), `claims` (final replies
that claimed a pass), `claimWithoutCheck`, `claimAfterFail` (see the receipt's
`verification` in [vault.md](vault.md)) and `per100Runs` for each of the three,
out of n. Null when no receipt carries a claim (receipts from before S27, or
surfaces with no reply), which the summary prints as `not installed`. Additive,
so still schema 2. compare gives each group's and the window's n and per-100
figures (null on a side that predates them) and prints one line; never flagged.
**Definition lookups** (2026-10-18, repo features R8). The tool pass also reads
Read's numbers `offset` and `limit` and Grep's `pattern` (classified in memory,
then dropped), and classifies a Bash command's grep/rg, `cat` and `sed -n` segments
from the command it already reads. `transcripts.definitionLookups` gives, for
`all` joined turns (the chat/claude group) and for `longRuns` (turns of 30 minutes
or more, from `started_at` to `ended_at`): `defSearches` (a Grep, grep or rg whose
pattern, flags and their values left out, matches `fn|function|class|struct|enum|
trait|interface|type|impl|def|const` then a name) per 100 turns;
`wholeLargeSourceReads` (a Read with neither offset nor limit, or a `cat` with no
pipe, of an rs/ts/tsx/js/mjs/cjs/py/go/swift/kt/java file whose result is at least
40,000 B): count, bytes and `carriedTokens` (Σ bytes/4 × the turn's later
main-thread calls before a compaction); `huntAfterSearch` (such a read within 3
uses after a definition search in the same turn) and `windowedAfterSearch` (a
windowed source read there: a Read with offset or limit, or `sed -n 'A,Bp'`),
each with per 100 turns; `tooLargeRetries` (a Read error followed within 2 uses
by a windowed Read of the same file) and `shareOfContext` (carriedTokens over the
turns' context tokens). Counts only: no path or pattern reaches the report.
`defSearches` over-counts: the regex runs on a grep/rg's words, path arguments
included, so it is not a real rate. `carriedTokens` counts only within the turn: a
read carried into the session's later turns is missed, so "upper bound" describes
intent (a whole read may serve more than one lookup), not carry. Claude only: Codex
lanes are not measured. `tooLargeRetries` counts any Read error followed by a
windowed Read of the same file, not only a too-large one. Claude Code caps a Bash result at 30,000 characters by
default, so a `cat` rarely counts as large. Additive, so still schema 2; the summary prints
one line, compare doesn't read it. The 10-18 measurement and the no-build decision
for a `find_definition` tool are in the build log.
`compare <A.json> <B.json>` flags a p95 rise over 10%, and a success drop, an
error, cancel or timeout rise, a reset rise or fall and an ask_agent timeout
rise only outside their Newcombe interval; a pair with under 20 finished runs
on either side is marked low n, and so are its flags (a p95 flag also when
either side has under 20 samples). Scorecards are schema 2
(overnight 2026-10-17, OF12); compare still reads a schema-1 file, reading its
`oneshot/unknown` group (all Jev) as `oneshot/jev`. It opens the database read-only
(falling back to `immutable=1`, which the report notes) and writes aggregates
only: no message text, prompt, friction message, delegation packet or
transcript path. The database is
`~/Library/Application Support/Hollow/Sessions/<vault stem>.db` unless `--db`
names one. A change to the harness records its before/after from it.

**A chat's own context** (overnight 2026-10-16/17, OF27): `node
scripts/agent-eval/my-context.mjs [--session <key>] [--json]` prints how many
tokens the running chat's main thread carries now. It takes the run id from
the scratch marker `$HOLLOW_SCRATCH_DIR/.hollow-scratch` (a chat's Bash has no
run id; `--session` uses that conversation's running turn instead), the
native session from the chat's own usage-ledger row (not the `delegate` or
`oneshot` rows that share its run id; every Sessions database is searched
unless `--vault`/`--db` names one), and the last main-thread call of the
Claude transcript (`--transcripts <dir>`, default `$CLAUDE_CONFIG_DIR` or
`~/.claude`, then `projects`): input + cache read + cache write. Subagent
calls don't count. It also reports this run's calls, how many were at 400K or
more, the peak and the last call's age, and says to hand over at 350K
(overnight board rule: after every merge and before every claim). Read-only,
usage blocks only; anything missing exits 3 with the reason, a bad
argument exits 2. `--marker <file>` reads another marker.

**The golden envelope's delta** (overnight 2026-10-17, OB15): `node
scripts/agent-eval/envelope-diff.mjs [<base.json> <head.json>] [--ref <ref>]
[--fixture <name>] [--all] [--json <file>]` prints, as a Markdown table for
the PR body, each scenario layer whose bytes changed between two versions of
`tests/fixtures/prompts/envelope.json`, then each changed scenario's total.
With no files it compares `git show <ref>:` that file (default
`origin/main`) with the working tree's, and heads the output with the ref's
short SHA. The ref must name a commit here (a typo'd or unfetched one exits 3;
one starting with `-` is refused); only a commit without the file reads as
empty, every layer new. `--fixture turn-budget.json` reads that fixture's
`after` sizes as layers: systemPrompt, prompt, configured and workspace
overlap (configured and workspace are inside the first two), so don't add
them up. Regenerate the fixture first (`HOLLOW_REGENERATE_PROMPTS=1 cargo test
--lib envelope_fixture`); every prompt-touching PR pastes its table. `--json`
writes the counts (scenarios, layers changed, bytes delta, per-layer sums)
and the rows. A missing file exits 3, a bad argument 2.

**The morning report** (overnight 2026-10-17, OF21): `node
scripts/agent-eval/morning-report.mjs --since <launch> --hw <HW> --vault <v>
--repo <checkout> [--lane <name>=<session key>]... [--installed-at <iso>]
[--out <md>] [--json <file>]` puts tonight's lanes against the evening
lanes on one sheet (tonight, 2026-10-17: `--since 2026-10-17T02:25Z --lane
ob=s-20261017-6b4c62eb`, since lane-ob.md names no chat key). Every time
needs `Z` or an offset (a bare time would be local; exit 2), and `--since`
must be before `--until` (or now), checked before anything is read. Lanes
come from the `lane-*.md` reports in `<HW>/outputs/overnight-<date>` and
`evening-<date>`, the date of `--since` / `--baseline-since` in UTC, the
day after, or New York; when two of those folders exist, the one whose
lane reports name the most chats with a turn in the window is read, with a
note (`--tonight`/`--baseline` name others). The baseline window defaults
to the evening lanes' 2026-10-16T21:00Z → 23:10Z, and over that default
window HB and HF are seeded with their own chats (`s-20261016-0e4cbde5`,
`s-20261016-39fdcf85`) unless `--lane hb=`/`hf=` names them, so the
evening needs no flag. A lane's chats are the session keys its report
names plus `--lane`, extended both ways through `handover` delegation
records (`childSession`); a report that names no chat says so ("lane OB: no
chat named in lane-ob.md; pass --lane ob=<key>") and is not guessed. Its
runs are those chats' whole turns overlapping the window (a lane's first
turn starts before `--since`). Per lane: main and subagent calls,
main-thread context per call p50/p95, the share of weighted tokens on calls
at ≥ 400K, handovers with the context at each, Python edit scripts per 100
Bash calls (inline `python3 - <<EOF` or `-c` whose code writes a file) and
Edit/MultiEdit calls, main thread and subagents apart (the table shows Edit;
Write calls are in the JSON), poll-only calls (every tool use a wait:
`sleep`, `node -e "setTimeout…"`, `wait_for_session`), the subagent share of
weighted tokens, chats, stints, ledger cost (known, unknown by status, and
any cost a partial row carries), the PRs the report claims (`#N → sha`, or a
merged heading item's PR link) that `git log --first-parent --merges
origin/main` (`--ref`, `--git-log <file>`) shows merged in the window, and
known $ per merged PR (`≥` while any run has no cost). A lane with no chat,
a named chat missing from the database, or a chat its report calls a
successor with no handover record to a predecessor has an unknown cost
(never $0), and so do the totals. A chat two lanes reach counts once in the
totals (ledger rows by `run_key`), with a note. Notes also flag a lane whose
transcripts carry under 98% of its ledger tokens, runs with no transcript
(once per lane), and an older database without `stints` or `delegations`.
Then placeholders for later items' JSONs in the
night folder (`gate-stats*.json`, `envelope-diff*.json`, `replay-probes*.json`,
`replay-bench*.json`, `retrieval-eval*.json`, `retrieval-skips*.json`; number
and boolean fields only), and a scorecard compare against
`scorecard-baseline(-v2).json`, labelled "not installed": tonight's merges
move product numbers only after install. Read-only; tool uses are classified
in memory from the tool name and a Bash command (scorecard.mjs `readCalls`
`toolUse`), and no text, input or result reaches the report.

The **retrieval label report** (HF06) checks the broker's fusion against
what Jev kept on real runs: `node scripts/agent-eval/retrieval-labels.mjs
<vault>/.evidence-manifests [--json <file>]`. Over runs whose retrieval Jev
labelled, it gives the AUC of the broker rank, `localScore` and
`semanticScore` against `kept` over the candidates Jev decided (the rest of a
mixed pool is counted as `unlabeled`, and a Jev-decided one without a boolean
`kept` or a rank as `malformed`, both not scored and both counted in a skipped
Jev run too): pooled, and the mean of
per-run AUCs (runs
where everything or nothing was kept are counted, not averaged), the keep
rate by score band, and a sweep of the RRF weights (0–2 by 0.25 each) and
semantic floor (0.50–0.80) that re-fuses each recorded 24-passage pool the
way `retrieval_broker.rs` `blend` does. The rebuild is approximate (it says
how often it lands on the recorded rank; compare settings with its own
current-setting row), floors under 0.6 can't be told apart (no lower hit is
recorded), and Jev's keeps are a proxy, not ground truth. Its agent label
set (S26) reads what the agent did, on any engine's runs: the citation rate
(kept passages the final reply named, `cited`, over runs with
`citedChecked`, Wilson 95%), each score's AUC against `cited` among the
kept, and need-recall (the share of `agentReads.notesOpened` retrieval had
among its candidates, and kept, over runs where retrieval ran: candidates
and no `skipped-…` `queryBasis`; notes opened in the others are counted
apart as `noRetrieval`). Manifests written before 2026-10-17 carry neither,
so on an older folder that section says so. The offline baseline (citation
4.4% over 446 runs) was measured against the whole assistant text; the
manifests read only the final reply, so the two rates aren't directly
comparable. A folder with agent labels and no Jev
labels still reports (exit 1 only with neither). Only numbers and labels
reach its output: need-recall compares paths and prints counts; no path,
passage text or reason.

The **real-vault retrieval eval** (OF01a) measures known-item recall on a
real vault: `HOLLOW_RETRIEVAL_EVAL_VAULT=<vault copy> TMPDIR=<fresh mktemp -d>
cargo test --lib retrieval_eval -- --ignored --nocapture` (from `src-tauri/`;
the full command is below; unset, the test prints a skip line and passes). Each note under `library/`, `tasks/` and
`learnings/` (never a `restricted` folder) with a title of at least one
retrieval term, and no other note sharing them, becomes a chat-length query
(`before tomorrow's meeting could you pull together everything we have on
<title> including notes, tasks, decisions and open questions so I can review
it`); at most 300, evenly spaced over every eligible note, so a folder counts
in proportion to its size. It calls the broker's public
`retrieve_lexical` (chat's ranking, semantic half off, no Jev rerank and no
connected projects) after engaging the FTS prefilter as the app does (the
test build's index root is a temp folder, so the live index is never
opened, and the run deletes that folder, which holds note bodies, when it
ends) and writes recall@8/@24, MRR, nDCG@8, the share cited in the
8-passage evidence budget, broker ms p50/p95 and how often the scan stopped
at a budget, in total and per folder, as JSON
to `$TMPDIR/retrieval-eval-<unix>-<pid>.json` (or `HOLLOW_RETRIEVAL_EVAL_OUT`;
`HOLLOW_RETRIEVAL_EVAL_COMMIT` is copied in). A rank counts distinct files
in the 24-passage pool. Aggregates only. OB12 (BM25 candidates) is gated on
its lexical recall@8 reaching 35%. Baseline at `fb9828fd` (2026-10-16, the
vault without `restricted/`, n = 300): recall@8 26.7%, MRR 0.178 (library
6.8%, tasks 80.5%). Run it on a copy without `restricted/` folders, since the
broker's own scan reads every note.

Since OF01b the same test adds a **semantic half** and a **real-query
replay**, and names the commit its numbers come from
(`HOLLOW_RETRIEVAL_EVAL_COMMIT`, else `git rev-parse HEAD` of the checkout,
`-dirty` when tracked files differ). The report's schema is
`hollow.retrieval-eval/2`: the lexical fields as before, plus `semantic`,
`blended` and `replay` blocks, each `{skipped: <reason>}` when it can't run.
Each lexical and blended pass (known-item and replay) runs twice over the
same queries; `determinism` records per pass whether the rank vectors
agreed (ms aside; `null` for a pass that didn't run) and `deterministic`
whether all did. A mismatch is reported, never a panic.

- **Semantic half.** Needs the bge model (`HOLLOW_RETRIEVAL_EVAL_MODEL_DIR`,
  else the checkout's models folder (bge-small-en-v1.5, absent in a fresh worktree), else
  `/Applications/Hollow.app/Contents/Resources/models/bge-small-en-v1.5`;
  none: `semantic: {skipped: "no model"}`) and the live EmbedIndex DB:
  `HOLLOW_RETRIEVAL_EVAL_EMBED_DB`, or `HOLLOW_RETRIEVAL_EVAL_LIVE_VAULT=<the
  live vault folder>` to derive
  `~/Library/Application Support/Hollow/EmbedIndex/<vault folder>-<16 hex>.db`
  (this Mac: `hollow-vault-1d1d4c39cd10e040.db`). The vault is never
  re-embedded and the live DB is never written: the test snapshots it into a
  fresh `$TMPDIR/hollow-test-retrieval-eval-<pid>-<random>/` (mode 0700) with
  `replay_probes::snapshot_db` (`VACUUM INTO` from an `immutable=1&mode=ro`
  connection, the same read as `sqlite3 -readonly 'file:<db>?immutable=1'
  '.backup <copy>'`), reads `vectors`, `chunks` and `meta` raw (schema v2
  or v3, recorded as `semantic.index.schema_version`; never through
  `EmbedIndex::open`, whose `ensure_schema` drops an index of another
  version), drops every row under a `restricted` folder (counted), and
  deletes the snapshot before embedding anything (also on a panic). A
  snapshot whose `PRAGMA quick_check` isn't `ok` (a torn copy) is not read:
  `semantic: {skipped: "snapshot quick_check failed"}`. Only the
  queries are embedded. WAL frames the app hasn't checkpointed are not in
  the snapshot. `semantic.snapshot.live_wal_file_bytes` is the size of the
  live `-wal` file, not the bytes left out: an ordinary checkpoint leaves
  that file's length alone (the app caps it at 16 MiB with
  `journal_size_limit` and empties it after each launch sweep with
  `wal_checkpoint(TRUNCATE)`, R7a), so only 0 proves the copy complete. Quit Hollow first
  for a complete copy. The stderr summary prints that size and the
  `quick_check` verdict (no path). `semantic` has
  semantic-only recall@8/@24, MRR and nDCG@8 (distinct eligible notes by best
  chunk, no floor) and `reaches_blend` (the share whose target the broker
  would get at or above `SEMANTIC_MIN`); `blended` runs the broker's own
  `retrieve_with` with those hits, so its floor, RRF weights (recorded in
  `semantic.fusion`) and diversity apply unchanged.
- **Replay.** Each Jev-labelled run manifest in
  `HOLLOW_RETRIEVAL_EVAL_MANIFESTS` (default
  `~/Library/Caches/overnight-20261017/manifests-base`) gives its kept paths;
  the request is that run's `turns.asked`, read from a snapshot of the
  Sessions DB (`HOLLOW_RETRIEVAL_EVAL_SESSIONS_DB`, or derived from
  `HOLLOW_RETRIEVAL_EVAL_LIVE_VAULT`) taken and deleted the same way before
  the broker runs, and held in memory only (same `quick_check` gate;
  `replay.sessions_snapshot` has its WAL file size, since turns still only
  in the WAL show up as `runs.request_missing`). Only runs whose manifest
  `retrieval.queryBasis` is `request`, or absent (manifests before
  2026-10-17), are replayed: chat queried the others with a wake's subject
  or a follow-up plus its prior topic, not `asked`; they are counted by
  value in `replay.runs.skipped_basis`. Kept paths under `restricted`,
  outside the vault (project files), no longer eligible for retrieval
  (`me/`, `sessions/`, `workflows/`, a workspace's non-output files:
  `ineligible`), or missing from the vault copy (drift) are counted in
  `replay.kept` and skipped; runs without a request are counted too, and
  Jev candidates without a path, rank or boolean `kept` are counted as
  `replay.manifests.malformed_candidates`. Each run goes through the broker
  inside `catch_unwind` under a quiet panic hook, so a panic can't print the
  request; `panicked` counts them (the hook is process-global, so run the
  test alone, as below). `replay.lexical` and `replay.blended` lead with
  per-run numbers (`run_mrr_first`, the best target's reciprocal rank;
  `run_any_at_8`/`_24`), then per-target recall@8/@24 and `target_mrr`,
  whose reachable maximum is `target_mrr_ceiling` (a run with N kept
  targets can at best rank them 1..N), not 1. A
  label file (`HOLLOW_RETRIEVAL_EVAL_LABELS`, else
  `$TMPDIR/retrieval-labels-<unix>-<pid>.json`) keeps run id, sha256 of each
  path, kept, rank, localScore, semanticScore and jevScore: no path, no text.
  The hashes are **unsalted**, so a listing of the vault maps them back to
  paths: keep the label file as private as the vault's file names.

Everything written is aggregates only: no request, note text, title or path
reaches the report, the label file or the test's output. From `src-tauri/`,
on a vault copy without `restricted/`:

```sh
T=$(mktemp -d /tmp/hollow-retrieval-eval.XXXXXX); trap 'rm -rf "$T"' EXIT
rsync -a --exclude restricted <vault>/ "$T/vault/"
TMPDIR="$T" HOLLOW_RETRIEVAL_EVAL_VAULT="$T/vault" \
  HOLLOW_RETRIEVAL_EVAL_LIVE_VAULT=<vault> \
  HOLLOW_RETRIEVAL_EVAL_OUT=/tmp/retrieval-eval.json \
  HOLLOW_RETRIEVAL_EVAL_LABELS=/tmp/retrieval-labels.json \
  cargo test --lib retrieval_eval -- --ignored --nocapture
```

The snapshots refuse a `TMPDIR` inside the checkout, a `Worktrees` folder
or `~/Hollow` (`replay_probes::refuse_temp_dir`).

The **real-vault replay probes** (OF17, `src-tauri/src/replay_probes.rs`) run
the merged pure functions over a vault copy and a snapshot of the Sessions DB
at $0 and write aggregates JSON (counts, shares, byte sizes; never a request,
note text, path or session key): each workspace's envelope through
`context_for_within` (briefs and plans cut; of the cut briefs, Decisions kept,
the newest dated entry kept, the left-out marker, the archive pointer lost:
OF24's number), `task_state::fold` over every conversation (steps, the calls
it drops as read-only, outcomes, host goals overall and for Main), each handover (successor
started while the handing-over turn ran; merged on its first turn under
OB14's peek and under the finished-turns rule before it),
`prepare_chunks` notes with no chunk by kind, and over the newest 500
requests the `history_cue` share and the oversized-note notice share (and
what it was before HF05). The Sessions DB is
`~/Library/Application Support/Hollow/Sessions/<vault folder>-<16 hex>.db`
(`session_store::vault_file_stem`; this Mac's vault:
`hollow-vault-1d1d4c39cd10e040.db`). From `src-tauri/`:

```sh
(
  T=$(mktemp -d /tmp/hollow-replay-tmp.XXXXXX)
  V=$(mktemp -d /tmp/hollow-replay-vault.XXXXXX)
  trap 'rm -rf "$V" "$T"' EXIT
  rsync -a --exclude 'restricted' ~/Documents/projects/hollow-vault/ "$V/"
  TMPDIR="$T" HOLLOW_REPLAY_VAULT="$V" \
    HOLLOW_REPLAY_DB="$HOME/Library/Application Support/Hollow/Sessions/hollow-vault-1d1d4c39cd10e040.db" \
    HOLLOW_REPLAY_OUT=<workspace>/outputs/replay-probes-<date>.json \
    cargo test --lib -- --ignored replay_probes
)
```

The run gets its own `TMPDIR` (`mktemp -d`, so 0700) and the trap deletes
that folder and the vault copy whole: no glob (zsh aborts a trap on a glob
that matches nothing) and no `find /tmp` (on macOS `/tmp` is a link, and
`find` doesn't follow a start-path link, so it never descended), so a Ctrl-C
or kill that skips the guard's drop still takes the snapshot with it. The
subshell keeps the `EXIT` trap from outliving the run when the block is
pasted into an interactive shell.

Without `HOLLOW_REPLAY_OUT` the JSON goes to stdout (add `--nocapture`);
with either input unset the test prints a skip line and passes, and it
refuses a vault with a top-level `restricted/`. The report at
`HOLLOW_REPLAY_OUT` holds aggregates only and is the only thing written
outside the two temp folders. Setup copies each workspace's context
documents, its `PROJECT.md` and its `sources/` (with `.provenance/`), and
the vault's `projects/` and `goals/` (what the envelope's attached sources
and project alignment read; never a `restricted` path or a link) to
`$TMPDIR/hollow-test-replay-<pid>-<random>` (0700) and snapshots the DB with
`VACUUM INTO` from an `?immutable=1&mode=ro` connection (no lock, no
`-wal`/`-shm` beside the live file; frames Hollow hasn't checkpointed yet are
missed) into a file created empty at 0600 first, where the test build's
session store looks for the copy (`$TMPDIR/hollow-test-sessions-<pid>/`,
0700 when setup makes it); the report's `snapshotQuickCheck` is the
snapshot's `PRAGMA quick_check`. The snapshot holds every message body: the
`Replay` guard deletes it and the copied docs on drop, the report's
`snapshotDeleted` says so, the `trap` is the second guard, and setup refuses
a `TMPDIR` inside the checkout, under a `Worktrees` folder or inside
`~/Hollow` (or one it can't resolve), so it never lives under a worktree.
Nothing writes the vault or the support dir. HB03 reads `fold`'s own
read-only rule over the raw tool calls (`readOnlyDropped`, `stepsKept`,
`readOnlyShareBefore`; after is 0 by construction). The handover probe cuts
a predecessor turn still in flight at the successor's start back to its
events by then; event retention (200 conversations, 90 days) drops older
ones, and `approximate` says when any such turn was read from its final
transcript. S07, OB18 and OB25 reuse
`Replay::setup`/`setup_with` (extra vault folders such as `learnings`);
OB18's host-subject and OB25's follow-up basis counts are a TODO in
`requests` until R1 merges.

The **replay bench** (S07, `src-tauri/src/replay_bench.rs`) replays the
recorded turn sequences through HEAD's `config_fingerprint` and
`plan_resume_with`, conversation by conversation, and counts config-changed
resets recorded → replayed: per 100 turns for Claude conversations, the
COO's and all, mode flips (resets with `mode` among their differing
parts; `modeChangedResets` counts any reset at a turn whose mode moved),
Main-agent wakes (`mainWakes`: every Hollow wake, `is_wake_message`), and
resets by the fingerprint part that changed, in OB03's vocabulary
(`chat_resume::PARTS`, `unknown` for none, the same names as the reset
notice and the scorecard's `continuity.resetsByPart`). A replayed reset is
named as the chat path names it: each replayed stint is stamped with
`config_parts` and the reset goes through `chat_resume::reset_parts`.
Recorded stints from before OB03 hold no parts, so a recorded reset is
named by diffing the rebuilt inputs against the turn before (the mode
itself compared, as v1 hashed it), `unknown` when none differs: v2's prompt
bytes, a contract bump, or an input not rebuilt. `byPart` names a reset by
its first differing part in fingerprint order, so `mode` hides `mcp`;
`recordedByPartAll` and `replayedByPartAll` count every part. Each turn's
input is rebuilt from `turns.permission_mode`, the chat usage row's
requested model and agent, `sessions.cwd`, and the run's context manifest
where the vault still keeps one (the newest 500 runs:
`capabilities.ownedServers` and the Canvases choice its visual-guide
delivery implies). A field a turn lacks comes from the nearest turn of its
conversation that has it (servers from one of the same plan, unattended and
wake kind first, cut to what the turn's kind attaches: an unattended
`wake_me` wake-up, morning review or night item has no teammates, computer
control or services), else a fixed default; the report's `assumed` counts
those turns by field, each side's `assumedPartResets` the resets whose
differing part was filled in, and `manifests` says how many counted turns
had one. Recorded closes the bench can't count (`unmatched`: before any
turn, at a turn before the window, at a turn skipped for want of an agent)
and `skippedTurns` are reported too. The window defaults to the recorded
baseline's (2026-10-09T00:00Z to 2026-10-16T21:16Z); `HOLLOW_REPLAY_FROM`
and `HOLLOW_REPLAY_TO` (RFC 3339) move it. It uses the probes' setup (same
snapshot, same guards) but reads nothing from the copy except the context
manifests, so only `.evidence-manifests/` is copied (setup copies no
missing folder and needs none of them). From `src-tauri/`:

```sh
(
  T=$(mktemp -d /tmp/hollow-replay-tmp.XXXXXX)
  V=$(mktemp -d /tmp/hollow-replay-vault.XXXXXX)
  trap 'rm -rf "$V" "$T"' EXIT
  trap 'exit 130' INT TERM HUP
  rsync -a --exclude 'restricted' ~/Documents/projects/hollow-vault/.evidence-manifests/ "$V/.evidence-manifests/"
  TMPDIR="$T" HOLLOW_REPLAY_VAULT="$V" \
    HOLLOW_REPLAY_DB="$HOME/Library/Application Support/Hollow/Sessions/hollow-vault-1d1d4c39cd10e040.db" \
    HOLLOW_REPLAY_BENCH_OUT=<workspace>/outputs/replay-bench-<date>.json \
    cargo test --lib -- --ignored replay_bench
)
```

The second `trap` turns Ctrl-C, a kill or a closed terminal into an `exit`,
which runs the `EXIT` trap, so a signal can't end the shell past it.
Listing the signals on the `rm` trap instead would delete the copy and then
let the block carry on to the `cargo` line without it.

Then, from the repository root,
`node scripts/agent-eval/replay-bench.mjs <bench.json> [<other bench.json>]`.
The JSON is numbers only (counts, per-100 rates, the window, the snapshot's
`quick_check` and `snapshotDeleted`): never a session key, a path, a model
name or a request; a request is read only to tell a wake and an unattended
turn. The script prints recorded → replayed per bucket with its n, resets
by first part and by every part, `skippedTurns` and `unmatched`; given a
second bench file it also compares the recorded and the replayed rates,
this file → the other (before and after a change), and its `summaryLine`
is the morning report's one line. The unit tests (no DB but
one temp store the session store writes) run with
`cargo test --lib -- replay_bench::tests`.

**Canvas accessibility:** the app-level axe and keyboard pass
over canvas surfaces is `tests/smoke/canvas-a11y.spec.ts` (glass over a
painted stand-in). `ARTIFACT_STYLES` paints `--app-canvas` on `body`, never
`html`: a root background stops an artifact's own `body{background}` filling
the frame (`artifact-background.spec.ts`).

**App builds on the remote runner (smart routing 7, 2026-11-03; off by
default).** Settings → Advanced → App builds (This Mac, the default /
Remote runner; `agentSettings.appBuildsOn`, `agents.appBuildsOn`). With Remote
runner and a runner set up, Build app (`self_install::run_build`) first runs
`hollow-slot --on <runner> --fetch <REMOTE_APP_OUT>` (an ignored folder
under the Rust target folder, so the tree stays clean) with a
script that refuses to run anywhere but a macOS runner (`HOLLOW_REMOTE_HEAD`
is set only there, so `hollow-slot`'s own fallback to this Mac becomes a
failure and the ordinary build here runs; `uname -s` must be Darwin), then
`env CI=true APPLE_SIGNING_IDENTITY=- HOLLOW_PROVENANCE_CHECKOUT=<this checkout>
npm run tauri:bundle` (ad hoc: the runner has no signing key) and zips the
bundle with `ditto`. Back here the zip is unpacked beside the install
target's `release/bundle/macos/Hollow.app`, checked to hold exactly one
`Hollow.app`, moved into place, signed again with the worktree's
`bundle.macOS.signingIdentity` and `entitlements` (`--options runtime`, as
Tauri signs one built here) and verified (`codesign --verify --deep
--strict`); the fetched folder goes however it ends. The runner gets 30
minutes; any failure there (the runner, the fetch, the unpack, the signing)
logs why and builds here with a whole deadline of its own; a stop stops it.
`scripts/source-provenance.mjs` records this Mac's HEAD and dirty flag from
`HOLLOW_REMOTE_HEAD`/`HOLLOW_REMOTE_DIRTY` and this checkout from
`HOLLOW_PROVENANCE_CHECKOUT`, so a build of a tree with uncommitted changes
reads as such on the install card and in the title. What comes back is
signed with this Mac's identity and gets Hollow's macOS permissions, so this
is for a runner its person controls (the Settings row says so). The spike
(2026-11-03, commit `0e3a0efa`, against the installed app built here from
the same commit): same source fingerprint, same files, byte-identical
Info.plist, icon and model, same entitlements, the same designated
requirement after re-signing (so macOS's permission grants carry over), a
16 KB larger binary linking the runner's macOS 26.2 SDK where this Mac's
used 27.2. That SDK difference is why it is off until tried once by hand.
The spike's call took 749 s, a wait for another chat's Rust build on the
runner included; a new worktree there also runs `npm ci` and fetches the
pinned model first.

**An install takes the approved build, byte for byte.** A build from Hollow
records a `BuildReceipt` (a unique `buildId`, a sha256 over every entry of the
built `Hollow.app`, the embedded `sourceFingerprint`, `commit`, `dirty`,
`baseCommit`); the agent's `install_built_app` card shows what changed and
binds the receipt; the approval re-checks it (`check_approved`), and the
update client copies the bundle to `updates/local/<buildId>/` when it offers
the Local build and hashes that copy then and again before
`hollow install-update` starts (the installer gets the copy), refusing a bundle that differs or a
build a newer one replaced. Bypass shows no card but still binds.

**Tauri versions move together.** `tauri build` refuses to bundle when the
`tauri` crate and npm `@tauri-apps/api` differ in major/minor, and PR CI
doesn't run `tauri build`, so after adding or bumping a Tauri plugin crate
compare `grep -A1 'name = "tauri"' src-tauri/Cargo.lock` with
`@tauri-apps/api` in `package-lock.json` (or run the pre-release lane).
`tauri-plugin-window-state` is pinned to `~2.4` for that reason.

**One version source (2026-10-02, merged plan slice 6).** `package.json`'s
`version` is the release version. `src-tauri/tauri.conf.json` doesn't repeat
it: its `"version"` is `"../package.json"`, which Tauri 2 resolves relative to
the config file at build time (so the bundle's `CFBundleShortVersionString`
follows package.json). Cargo can't read package.json, so `Cargo.toml`, the
`hollow` entry in `Cargo.lock` (behind `env!("CARGO_PKG_VERSION")`) and both
version fields in `package-lock.json` are copies.
`node scripts/version.mjs check` (`npm run version:check`; also run inside
`npm run plan:check`, so CI fails on drift) checks them, and
`node scripts/version.mjs bump <x.y.z>` (`npm run version:bump -- <x.y.z>`)
rewrites every copy, checking all five files before writing any, so a Cargo
file it can't update leaves nothing half-bumped.

**Built versions are stamped `0.6.<n>` (2026-11-15, updates row U1).** n is
`git rev-list --count --first-parent <ref>` (default HEAD; `node
scripts/version.mjs stamp [ref]`; a shallow clone is refused). Release builds
are made from main's tip, where n always rises, is the same on any machine and
means one commit; a branch build counts its own first-parent history, so two
lanes' branch builds can share an n. It replaces "each install record PR bumps the
version". The stamp exists only in a build made by `npm run release:build`
(below): Tauri gets `--config '{"version":"0.6.<n>"}'` (Info.plist and
`package_info()`) and `build.rs` gets `HOLLOW_BUILD_VERSION`, which
`source-provenance.mjs` writes as `build-provenance.json`'s `version`, the
running app's version (`app_menu::build_identity().version`, the window
title). Nothing is written to the tree. `env!("CARGO_PKG_VERSION")` (user
agents, MCP `serverInfo`, settings exports) stays the committed baseline. The
committed copies stay on the 0.5.x baseline: `version.mjs check` fails on a
0.6.x package.json and `bump` refuses one, so a committed version never
collides with a stamp (0.7 and later only get a note: the rule doesn't cover
them). Plain `npm run tauri:bundle` (Hollow's **Build
app**) still carries package.json's version. After the provenance check passes, the
installer accepts package.json's version, the stamp of HEAD, or the stamp of
the commit the bundle's `build-provenance.json` records (so a docs-only commit
after the build still installs), and refuses any other version
(`acceptedVersions`).
Never bump at install time: the installer refuses a dirty tree.

**Release build: `npm run release:build`** (`scripts/release/build-app.mjs`).
Runs `tauri build --ci --bundles app` (`--dmg` adds the dmg; no
`createUpdaterArtifacts`: the release builder signs the archive) with the stamp and a signing identity from the
environment, then verifies; any failure exits 1 with the reason. A dirty
tree is refused unless `--allow-dirty` (local branch builds pass it; the
nightly builder never does), and the printed version says "dirty". Unknown
options and stray arguments are refused. The verify tools, git and the
self-test run with the `APPLE_API_*` and Apple ID variables removed; only
`tauri build` sees the API key. Variables
(names only are printed, never values):

| Variable | Meaning |
|---|---|
| `HOLLOW_SIGNING_IDENTITY`, else `APPLE_SIGNING_IDENTITY` | Signing identity. Default the local self-signed "Hollow Dev Signing" until the Developer ID cert exists; `-` signs ad hoc. |
| `HOLLOW_TEAM_ID` | Team the signature must carry. Default `TEAM000000` for a named identity other than the local self-signed one; none for ad hoc. |
| `APPLE_API_KEY`, `APPLE_API_ISSUER`, `APPLE_API_KEY_PATH` | All three: Tauri's bundler notarizes and staples. None: no notarization. Some: refused. Refused with an ad hoc or self-signed identity. `APPLE_ID`/`APPLE_PASSWORD`/`APPLE_TEAM_ID` are removed from the build's environment. |

Verify (design section 3, and what `prerelease-build.yml` does):
`codesign --verify --deep --strict`; `codesign -dv --verbose=4` for the
signer (`Authority`, `TeamIdentifier`, or `Signature=adhoc`); `spctl --assess
--type execute` and `stapler validate` only when it notarized; Info.plist
and `build-provenance.json` carry the stamp; `source-provenance.mjs --check`;
`embed-selftest` loading the bundled model. These check the `.app` only:
a `--dmg` image is not verified. `--ref <r>` stamps from `<r>`, which must be
the checked-out commit; `--verify-only` re-checks the last build, taking
whether to expect notarization (spctl, stapler) from the same `APPLE_API_*`
variables.

**Signing identity switch (when the Developer ID cert lands, row U0).**
Local branch builds sign with the same Developer ID as release builds (one
designated requirement for every Hollow on Alex's Macs; Hollow's **Build
app** reads `bundle.macOS.signingIdentity`, which `APPLE_SIGNING_IDENTITY`
overrides). The first build signed by the team changes the designated
requirement from the self-signed cert's, so macOS asks once more for
Hollow's permissions (calendars, microphone, any Full Disk Access) and
Keychain once more for **Always Allow**; after that the per-rebuild password
prompt goes away.

**Current install** (installs from the Update button since row U10, recorded in
`last-result.json` after the restart; the latest recorded here, 2026-10-07 16:58:00 EDT): clean `a1c80d02688e` (whiteboards removed,
`hc-flow` diagrams, Jev dedup guard, #405–#409), receipt `20261007-165755-56d860aa`, `ok=true`,
`matchesRunning=true`, version 0.5.14. Bound build `20261007-155847-5f88b4c3` and installed
provenance match the bed-tested fingerprint `5000f4493e03…` (64 pass / 13 skipped / 0 fail).
Journal: prepared → promoted → verified → healthy, no corrective relaunch. Signature
verification passed. `68c7e286` is kept for rollback. This install-record change advances
source version to 0.5.15 for the next build; the running app remains 0.5.14.
Evidence and limits.

**Reading install evidence.** `self_install_result` returns a result only
until the user dismisses the notice, then `null` (by design). Use
`self_install_receipt` / `build.self_install_receipt`: the persistent
`last-result.json` receipt with `acknowledged`, `matchesRunning`, `state`
(`installed`, `failed` or `unknown`) and a one-line `summary`. `spctl -a`
rejects the app because "Hollow Dev Signing" is self-signed; that is expected.

**A finished build survives a restart** (2026-10-06). When an app build
finishes, `self_install.rs` saves it (job fields + receipt) to
`<app data>/self-install/finished-build.json` (0600); a new build or a failed
one removes it. At launch `restore_finished_build` puts it back as `built`,
unless the installed app already is that build (commit and source
fingerprint) or its bundle is gone, and the update client offers it in the
Update button again unless it was hidden with Not now (U10). The install
still binds the receipt and re-hashes the bundle, so a restored build that
changed on disk is refused.
Before this, a Hollow restart between build and install meant a rebuild.

Hollow is used as an installed `/Applications/Hollow.app`, not run from
`tauri dev`. The shared-release sequence is: **commit → PR → merge to
`main` → rebuild → replace the installed `.app` → remove the build bundle**
(verify first with
`npm run plan:check` + `cargo test --lib` + `npm test` + `npm run typecheck` +
`npm run build` + `npm run bundle:budget` + an isolated-port `npm run test:smoke`).

A direct user-requested local build/install can use the current working tree;
installing locally does not mean the change was committed or published. Record
each install's provenance and checks in the build log, replace the Current
install paragraph above, and move the old one to the
build-and-ship archive.

- **Normal rebuild/install:**
  `CARGO_TARGET_DIR="$HOME/Library/Caches/hollow-release-target" npm run tauri:rebuild`
  (`tauri:bundle`, the underlying `tauri build --ci --bundles app`, then
  `tauri:install`). The target must be absolute with no `~` segment:
  `check-cargo-target.mjs` (first in `tauri:bundle` and `gate:rust`) and the
  installer refuse a relative or unexpanded-`~` one, which once
  left 17.9 GB in literal `src-tauri/~/` folders. For raw `cargo` commands
  `build.rs` (`src-tauri/src/cargo_target_rule.rs`) refuses such a
  `CARGO_TARGET_DIR`, and any target under a `~` folder however it was chosen;
  a relative `--target-dir` or cargo-config target is not caught. **A dev/test
  target grows without bound on macOS:** debug info stays "unpacked" in
  `debug/deps/*.rcgu.o` and old objects are never collected (an overnight
  lane's target reached 142 GB, 136 GB of them, ~5 GB per gate). For repeated
  gate runs set `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0` (one
  rebuild, then flat: rustc keeps objects only for unpacked debug info). It
  doesn't remove objects already there (delete the target for that; it only
  costs a cold build), any run without the variables flips back and growth
  resumes, and test backtraces lose line numbers. With four-worker Vitest caps,
  also set `VITEST_MIN_THREADS=1 VITEST_MIN_FORKS=1`: on a many-core Mac
  the default minimum can exceed the explicit maximum and fail before tests start.
  The service-lifetime test also checks the whole captured environment for
  `hollow-run`; a test cwd beneath a turn scratch path gives a false failure
  via shell PWD. For the full gate, use a fresh short neutral directory outside
  the checkout, e.g. `TMPDIR=$(mktemp -d /tmp/v.XXXXXX)`, and keep logs in turn
  scratch. Since overnight 9 A12, `npm run gate` does this itself:
  `scripts/gate-run.mjs` replaces a TMPDIR longer than 16 characters, inside the
  checkout or under a `hollow-run` path with a fresh `/tmp/hg.XXXXXX` for the
  chain, says so on stderr, records it as the run record's `tmpdir` and removes
  it at exit. Running single test commands by hand still needs a good TMPDIR.
  Test modules that share one `$TMPDIR/hollow-…-<pid>` folder per test process
  take it from `test_temp::process_dir` (overnight 10 A1): its first call
  sweeps dead test processes' folders, and an exit hook removes the process's
  own, so a filtered `cargo test --lib <module>` leaves no folder behind; a
  new one must use it and start with a `test_temp::PREFIXES` entry. A worktree-contained temp folder is incompatible with replay tests
  (which deliberately refuse it), Git-root fixtures and prompt-budget path
  lengths. A symlink into a `hollow-run` path also fails the service assertion
  once canonicalized. The short neutral path keeps approval sockets below
  macOS SUN_LEN. Do not use the installed app's data or another lane's temp.
  The disk
  guard (`disk_guard.rs`, below) warns when free space gets low. Never stop after the raw bundle: macOS indexes it as a
  second Hollow. Keep the target dir out of iCloud-synced folders (iCloud
  xattrs fail `codesign --verify`), and `xattr -cr src-tauri/models` if model
  weights were copied with `ditto`. The first build of a fresh checkout
  downloads and verifies the ~133 MB semantic-search model
  (`npm run fetch:model`).
- **Install and cleanup:** `npm run tauri:install` refuses to replace a
  running Hollow, verifies the build, stages it, verifies again, and swaps it
  into `/Applications`, keeping the last two verified bundles (the installed
  app and `/Applications/.Hollow.app.previous`; `--rollback` swaps them,
  `--status` shows both). It then removes the temporary build bundle. Check
  that Hollow is idle with
  `pgrep -a -f '^/Applications/Hollow\.app/Contents/MacOS/hollow( |$)'`
  (**keep the `-a`**: macOS `pgrep` leaves out its own ancestors, and from a
  Hollow turn Hollow is one); a normal `TERM`, never `KILL`, is the fallback.
  Each install, rollback and reopen check takes an installer lock and writes a
  journal that recovery replays.
- **Install build folders** (`~/Library/Caches/hollow-install-target-<workspace>`,
  one cargo target per build workspace) are never removed automatically.
  `build.install_targets` lists them with why each stays (built the installed
  app or the rollback copy, found through the `source` of installer journals
  whose install went in;
  a running, failed or finished-not-installed build's; the newest `keep`,
  default 2). The confirm-tier `build.prune_install_targets` deletes the rest,
  its card listing every folder and bound to that list
  (`src-tauri/src/install_targets.rs`). The person's own way in is Settings →
  Advanced → Disk space → **Delete old install builds…** (overnight H20): it
  previews with `install_targets`, lists the same folders in a confirm dialog
  (sizes from the last measure) and calls `prune_install_targets` with that
  list's hash.
- **Worktree hygiene** (overnight A9, `src-tauri/src/worktree_hygiene.rs`):
  `build.worktree_hygiene` reads, for every repository this vault's build
  workspaces came from, its linked worktrees (merged into the remote default,
  clean, commits on no remote, bound workspace), local-only branches (listed,
  never deleted) and literal `~` folders with sizes (git reads use
  `--no-optional-locks`; du can take seconds). The confirm-tier
  `build.clean_worktrees` removes only merged, clean, pushed, unlocked
  worktrees nothing runs in (`lsof` cwds), holding no ignored file beyond
  rebuildable output (`gardener::rebuildable`, the gardener's own rule: a
  `.env` keeps it), not exactly at the default tip, and not at the commit
  the installed app or its rollback copy was built from (their
  `build-provenance.json` in /Applications, `install_targets::app_commits`;
  reason "built the installed app" / "built the rollback copy"; an app
  whose provenance can't be read keeps every worktree; overnight
  H20, after the release checkout that built the installed app showed up as
  removable), not the folder either app was built in (the provenance's
  `checkout`, `install_targets::app_checkouts`, same reasons, wherever its
  HEAD has moved since), and not the worktree of a build running, waiting to
  install or installing (`self_install::held_worktrees`, from the build job
  and the saved `finished-build.json`; reason "a finished build waiting to
  install" etc.; the Local build is installed from its copy under `updates/local`;
  folders match canonically, links and `/tmp` for `/private/tmp`; overnight
  S01a, after a release checkout moved past both apps' commits was listed
  while its build waited, and the old Install and restart then failed "The worktree
  is gone"; the gardener's `worktree:` line skips the same holds, overnight
  S01e; both read the holds again right before each removal,
  `worktree_hygiene::held_now`, overnight R2, and for a bound one once more
  after its services stopped and its unsaved-work check, right before `git
  worktree remove`, `build_workspaces::remove_for_unless`, overnight R4, so a
  worktree a Build or Install has needed since the list was shown stays with
  its binding, reported as "Kept: <reason>, since the list was shown", and
  the rest of the list goes)
  (bound ones through `build_workspaces::remove_for`; ignored build output goes with them and the
  card names it) and the `~` folders of idle worktrees,
  its card bound to the exact list and removal method; optional `repo`.
  The person's way in is Settings → Advanced → Disk space → **Remove finished
  worktrees…** (overnight H20): `worktree_hygiene`, a confirm dialog listing
  the same paths, then `clean_worktrees` with the report's hash. Both paths
  share one decision (`report_for`), so every guard applies to both. A list
  that changed since the preview is refused with the agent's words ("…
  changed since the card was shown…"); the panel rewords that to "The list
  changed since you saw it. Nothing was deleted/removed; press the button
  again to see the new list." (`diskGuard.ts` `cleanupErrorText`).
  **Reclaiming generated output (OC6, 2026-11-16):** the same report also
  lists, for each worktree that stays (for whatever reason), the
  ignored build folders (`node_modules`, `target`, `dist`, and `target` under `src-tauri`)
  it could shed, as `reclaim` on the worktree, with `reclaimBytes` and a
  `reclaimHash`. Only an idle worktree is listed: no process in it, not
  locked, no bound build workspace, not what an app or build came from, HEAD
  and the git index untouched for 30 days, every changed path untouched for
  7 days, and each folder ignored, holding no tracked file, a real folder
  inside the worktree and untouched for 30 days (newest change in its top two
  levels); anything that can't be dated (an unreadable folder or entry, more than the walk's entry cap) counts as touched, an untracked folder is walked eight levels deep, and a folder other than `node_modules` holding a `.env*` file or a local database (`gardener::rebuildable`, the rule `clean` uses) is never offered. `clean` itself skips the reads (`Opts.reclaim`). This is a dry
  report. `worktree_hygiene::reclaim_output` (bound to the hash, rechecking
  each worktree right before removing, deleting only the listed folders and
  never a worktree or branch) is written and tested but not wired to an
  action: that needs a confirm action with registry, command-map and COO-parity
  entries, the next step.
- **Builds are signed with the stable local identity "Hollow Dev Signing"**
  until the Developer ID cert exists (release builds take theirs from the
  environment: see "Release build" above)
  (`bundle.macOS.signingIdentity` in `tauri.conf.json`, a self-signed
  code-signing cert in the login keychain). With no Team ID, **every rebuild
  asks once for the login password** for the one shared Keychain item
  (`secret_store.rs`); click **Always Allow**. If the cert is missing,
  `tauri build` fails at signing: recreate it (self-signed with the
  codeSigning EKU, `security import`, `security add-trusted-cert -p codeSign`).

Config lookup order (first hit wins): `$HOLLOW_CONFIG`,
`~/.config/Hollow/config.json`,
`~/Library/Application Support/Hollow/config.json`, `./config.json` (a
candidate build reads only `$HOLLOW_CONFIG` and its own Application Support
folder). `vaultPath` is required; Projects and Vault are editable in Settings
→ Hollow folder (restart when the vault changes). Other keys: `projectsPath`,
`hollowHome`, optional `claudeProjectsPath` and `gitReposPath`, the `tracker` block
([Tracker sync](../TRACKER_SYNC.md)); `notion.*`, `notionSyncIntervalMin`, `workspaceSource`
and `sourceOfTruth` still load and are ignored (only the importer reads the Notion keys);
legacy keys stay readable and are preserved. Examples: [README.md](../../README.md),
`config.example.json`. Config writes refuse malformed existing JSON, preserve
unknown keys and use a private temp file plus atomic rename (0600).
`HOLLOW_DEBUG_LOG=1` writes redacted per-run debug logs to
`~/.cache/hollow/*.log`. **Disk guard** (overnight D1, `disk_guard.rs`): the
minute timer reads free space (`statvfs`, the home volume and, when the vault
is on another volume (`st_dev` differs), the vault's: `Report.vault`) and,
under the threshold (Settings → Advanced → Disk space → **Warn under**,
`disk_guard::threshold_gb`, app data `disk-guard.json`, 1–10,000 GB, default
50; `set_disk_threshold`, a confirm action auto runs; morning N6), warns in the bell
once a local day (job `disk-guard`, link `settings:system`) with Hollow's
largest folders (`~/Library/Caches/hollow*`, each build worktree under
`build_workspaces::worktrees_root(hollowHome)`, the debug log folder; `du`
within 20 s, unsized rather than guessed; an in-memory guard keeps a disk too
full to write `jobs.json` from re-running it each tick). The warning is
`kind: warning` (Recent activity, a Hollow loop in its filter), not a Waiting-on-you item; Settings → Advanced
→ **Disk space** (`settings/DiskSpacePanel.tsx`, on demand) and the read
action/command `disk_guard` give the same report. Measuring is read-only; the
panel's **Delete old install builds…** and **Remove finished worktrees…**
(overnight H20, `diskGuard.ts` words) run A5's and A9's cleanups after a
confirm dialog bound to the previewed list's hash, then measure again; the third, **Remove old indexes…** (repo
features R7b, `hollow_databases.rs`), removes other vaults' search indexes the same way. **Hollow's databases**
(Settings → Advanced, `settings/DatabasesPanel.tsx`, lazy; `hollow_databases {check}`) lists the SQLite files under
`~/Library/Application Support/<Hollow>/`: `Sessions/<vault>-<16 hex>.db` (conversations; never removed),
`SearchIndex/<vault>-<16 hex>.db` and `EmbedIndex/<vault>-<16 hex>.db` (rebuildable), and `Companion/applied.db`;
since repo features R14b also `SearchIndex/<vault stem>.projects-<16 hex>.db` (kind `projectIndex`: a vault's
content index of its Projects folder, `project_text_index.rs`; whose it is comes from the vault stem before
`.projects-`, so this vault's is never offered and another vault's unused one is, like its search index; this
vault's goes through Settings → Advanced → Search & indexes' switch).
The stem is `session_store::vault_file_stem` for all three per-vault stores (a test keeps the two indexes'
`db_path_for` equal to it), so "this vault's" is told by name (an unresolvable vault folder plans nothing). An
index of another vault is offered (`stale_indexes`) once its `.db` and any non-empty `-wal` are an hour old and no
other process holds a lock on its `-shm`'s byte 128 (SQLite's dead-man switch, which every WAL connection holds:
an idle second Hollow leaves an old `.db` and, after R7a's TRUNCATE checkpoint, an empty `-wal`);
`remove_stale_indexes {expectedHash}` renames the `.db` to `<db>.removing`, removes `-shm` then `-wal` (so a stuck
`-wal` keeps its frames for the put-back), then the tombstone, only when the plan's hash (sorted path, length, mtime) still matches. Just before each rename it probes
that file again (`shm_in_use`, `recently_used` at the current time): one opened since the list was made is kept and
reported failed, "opened since the list was made; kept" (overnight 10 A5). If a sidecar stays, the tombstone goes
back by `hard_link` then unlink, never a rename, so a database made at the path meanwhile is never replaced: the
old one stays `<db>.removing` and the conflict is reported. The window between the probe and the rename remains
(closing it needs a write lock on the `-shm`'s byte 128, not taken).
The files this process's `engage` opened (`vault_search_index`/`vault_embed_index::engaged_db_path`, recorded
before opening) are never offered under any stem, so a symlinked vault whose target changed since launch can't
make its own index look like another vault's. A crash between the rename and the last delete leaves
`<db>.removing` (and possibly the database's `-wal`/`-shm`): the plan offers it with `why: "left by an interrupted
removal"` once its status-change time is an hour old (a rename keeps the mtime), and the same flow removes it.
**Save diagnostics…** (Settings → Advanced, palette;
`save_diagnostics {dir}`, `diagnostics_bundle.rs`, overnight C6) writes
`hollow-diagnostics-<stamp>.zip` into a folder outside the vault the person
picks, never overwriting: `versions.json`, `health.json`, `receipts.json`
(newest 200, metadata only: no commands, excerpts, paths or session keys),
the last 256 KB of each `~/Library/Logs/<Hollow>/self-install/` log (never
`services/`), and `debug-logs.json` (debug logs by name, size and header only:
their bodies are model replies that quote notes). In every string the vault
path and the names under it become `<vault>/…` and home `~` (configured and
canonical, whole paths only), then `claude_runner::redact` runs. Map tier
`user-only` (in `USER_ONLY`): no agent action. Never sent.
A missing or unparseable config boots *setup mode*
(the onboarding window) instead of failing.

**Export settings…** / **Import settings…** (Settings → Advanced, the
"Settings file" row, and the palette's two entries;
`settings_transfer.rs`, overnight F7) works through the same key registry as
`get_settings`/`settings.change`, each key read and written by its own owner.
- **Export** writes every key except `Type::Secret` ones to a `.json` the person
  picks: `{hollowSettings: 1, exportedAt, appVersion, settings}`.
- **Import** previews first (`settings_import_preview`). It lists each change
  old → new, the count already matching, and what is left alone and why:
  unknown keys, secrets, personal history (`paletteRecents`, `railCollapsed`),
  values of the wrong type, defaults (a file can't reset a key), and keys whose
  current value can't be read. Confirm-tier changes come first, with what each
  one is. `shortcuts` travels as `{enabled, keys}` only.
- **Apply** (`settings_import_apply {path, expectedHash}`) runs only if the
  file and current values still hash the same as the preview.

All three commands are user-only.

Archive: install records and build/ship detail
(every install record, the full installer journal and reopen-check notes, the
full command table and config notes).

---

## In-app installer (hollow install-update)

Updates design 2026-11-14 sections 6 and 9, lane U row U4 (2026-11-15). The
installer of `scripts/install-macos-app.mjs`, ported into the binary
(`src-tauri/src/update_install/`), so an update installs without a checkout or
Node. The updater client (`update_client`, U5) starts it for channel updates
and, since U10, for Local builds from Changes → Hollow app (the old Install
and restart path and its helper script are gone; the .mjs stays for
`npm run tauri:install` by hand). U6 adds Go back.

**How it is run:** from the staged new bundle's own binary, as a detached
process: `update_install::detach::spawn_detached(<staged>/Contents/MacOS/hollow,
args, log)` starts `hollow install-update <args>` in its own session and
process group (`setsid`), stdin closed, output appended to the log, and the
environment cut to `PATH`, `HOME`, `TMPDIR` and `LANG` (so no `HOLLOW_*`); the
log is created 0600. It never opens a window and never
takes the app's `gui.lock`: `main.rs` dispatches it before the launch gate.

**The app verifies first:** `update_client` checks the staged bundle itself
(the same `verify::check_bundle`: signature, expected team, version) **before**
spawning the helper, because the helper runs the staged bundle's own code and
so can't vouch for itself; it passes `--expected-team` explicitly and
`--allow-downgrade` only after a channel switch.

```
hollow install-update --staged <new Hollow.app> --expected-version <v> [options]
hollow install-update --go-back [--restore-snapshot <dir>] [--app <path>] [--journal-dir <dir>] [--no-relaunch]
hollow install-update --status  [--app <path>] [--journal-dir <dir>]
```

| Flag | Default | Meaning |
| --- | --- | --- |
| `--staged <path>` | required | the new `Hollow.app` (a real folder, not a symlink) |
| `--expected-version <v>` | required | its `CFBundleShortVersionString` must equal this |
| `--expected-team <id>` | `TEAM000000`, or `HOLLOW_UPDATE_TEAM` | its TeamIdentifier must equal this; `-` (or `adhoc`) expects an ad hoc signature with no team, and a team id never accepts ad hoc |
| `--app <path>` | `/Applications/Hollow.app` | the installed app (absolute, ends in `.app`) |
| `--journal-dir <dir>` | `~/Library/Application Support/com.hollow.app/installer` | lock, journals, snapshots, failure file |
| `--snapshot-source <path>` | `~/Library/Application Support/Hollow/Sessions` | a Sessions `.db`, a folder of them, or a folder holding `Sessions/`; a missing default is "no chats yet", a missing explicit one fails |
| `--no-snapshot` | off | skip the snapshot (recorded as skipped) |
| `--relaunch` / `--no-relaunch` | relaunch | reopen and check the new app; without it the journal settles `unchecked` |
| `--allow-downgrade` | off | install a lower version (the app passes it for a channel switch or Go back) |
| `--install-id <id>` | `YYYYMMDD-HHMMSS-<8 hex>` (UTC) | names the journal and the snapshot folder |
| `--restore-snapshot <dir>` | off | `--go-back` only, absolute: first put the Sessions databases back from this snapshot (row U6, below) |
| `--quit-timeout-secs`, `--start-timeout-secs`, `--settle-secs` | 60, 60, 20 | the waits below (at most 3600 each) |

An install or go back prints one JSON line on stdout,
`{"ok","exit","code","message","journal"}`, and a sentence on stderr; a usage
error (exit 2) writes only stderr. `--status` prints the installed and previous app (path, version,
bundle id, team), the lock holder, unsettled journals and the last failure,
and writes nothing.

| Exit | Code | Meaning |
| --- | --- | --- |
| 0 | `ok` | installed and started healthy (or `unchecked`), went back, or `--status` |
| 1 | `failed` | failed before anything moved, or every move was undone |
| 2 | `usage` | bad command line |
| 3 | `busy` | another installer (this one or the .mjs) holds the lock |
| 4 | `refused` | the staged bundle: signature, team, version, bundle id, symlink, location or downgrade |
| 5 | `not-writable` | this account can't write the install folder (needs an admin account); nothing changed |
| 6 | `snapshot-failed` | the Sessions snapshot failed; nothing swapped |
| 7 | `app-running` | Hollow did not quit within the quit timeout, or was opened during the install |
| 8 | `went-back` | the new version failed to start twice; the previous one is back and reopened |
| 9 | `needs-hand` | putting things back failed, leftovers no journal explains, an unfinished .mjs install, or an unreadable journal |
| 10 | `nothing-to-go-back` | `--go-back` with no `.Hollow.app.previous` |
| 11 | `start-failed` | `--go-back` swapped but the app did not start (not reversed) |
| 12 | `restore-failed` | `--go-back --restore-snapshot`: the snapshot could not be restored; every move undone, data untouched, apps not swapped |

A `--go-back --relaunch` that ends in 1, 4, 10 or 12 after Hollow quit
changed nothing, so it opens the installed app again (the message ends "Hollow
was opened again"); never after 9.

**Order of one install:** writable check (a create-and-remove probe file; exit
5 before any lock or journal) → installer lock → wait for Hollow to quit →
recover earlier journals → journal `prepared` → staged checks → copy to
`.Hollow.app.update-staging` (`ditto --rsrc --extattr --acl --noqtn --clone`,
like the .mjs), then `xattr -d -r -s com.apple.quarantine` on the copy (`-s`: a symlink
itself, never followed out of the bundle), best effort
(`--noqtn` was seen keeping the flag on macOS 27; dropping it is safe because
the signature and team were checked first), and check the copy again, with
the executable compared byte for byte (`staged-verified`) → Sessions snapshot
(`snapshot-written`) → refuse if Hollow was opened meanwhile → `codesign
--verify` of the copy once more → the moves
(`promoted`) → reopen check (`started` per try) → `healthy`, or go back.

**Staged checks:** a real folder ending `.app`; not inside (or containing) the
installed app, `.previous` or the two in-flight names; no symlink whose target
resolves outside the bundle (absolute, or `..` past it; `Versions/Current`
style links are fine); bundle id `com.hollow.app`; the expected version;
`codesign --verify --deep --strict`; the expected team: exactly one
`TeamIdentifier=` line in `codesign -dv`, equal to it, and for a real team id
(ten capitals or digits, checked before use) `codesign --verify --deep --strict
-R '=anchor apple generic and certificate leaf[subject.OU] = "<team>" and
certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate
leaf[field.1.2.840.113635.100.6.1.13] exists'`, so a self-signed certificate
naming the team fails (`effects::team_requirement`; ad hoc and today's
"Hollow Dev Signing" have no team and skip it); no lower version than the installed one without
`--allow-downgrade` (dotted numbers; an unparseable different version counts
as lower). The installed app is kept as `.previous` only if it verifies now
(signature and bundle id, no team: builds before Developer ID have none); one
that doesn't goes to the discard name, so it never replaces a verified previous.

**Lock:** `<journal dir>/installer.lock`, the .mjs's file and protocol (text
written to a temp file and hard-linked into place; takeover of a dead holder
through `installer.lock.takeover-<inode>`), so the two installers refuse each
other. Ours also records `pidStartedAt` (`proc_info::process_start_micros`): a
holder is dead when its pid is gone or now has another start (pid reuse); an
unknown answer counts as live. A damaged lock is dead. Released only while it
still holds our text.

**Moves (no moment without an app):** over an existing app, (1)
`renamex_np(RENAME_SWAP)` of the staging copy and the installed app, (2)
`.previous` to `.Hollow.app.update-discard`, (3) the staging name (now the old
app) to `.previous` (or to the discard name when it didn't verify). A fresh
Mac is one rename. Go back is one swap of `.previous` and the app. The discard
(the older previous) is deleted only once the new app is healthy (or
`unchecked`). The in-flight names differ from the .mjs's (`.installing`,
`.discard`, `.rollback`), so its folder-based recovery never touches ours; if
one of its names exists, this installer stops (exit 9) rather than guess.

**Journal:** `<journal dir>/update-journal/<id>.json` (the .mjs keeps
`journal/`), written whole (temp, fsync, rename, folder fsync) before and after
each step. Fields: `schema` (1), `kind` (`install` | `go-back`), `id`, `pid`,
`startedAt`, `state`, `app`, `staged`, `expectedVersion`, `expectedTeam`,
`fromVersion`, `toVersion`, `keepCurrent`, `newIno`, `plan` (`[{op: rename |
swap, from, to, ino}]`, written before the first move), `movesDone`,
`snapshot` (`{skipped, folder, files: [{source, copy, userVersion, bytes}]}`),
`attempts` (`[{attempt, launchedAt, outcome, pid, detail}]`), `steps`
(`[{step, at, ...detail}]`) and, for a go back that restores data, `restore`
(`{folder, fromInstall, databases, aside: [[from, to]]}`). States: `prepared`, `staged-verified`,
`snapshot-written`, `data-restored` (go back with a restore), `promoted`, `started`, then settled `healthy`,
`rolled-back`, `failed`, `unchecked` or `start-failed`; `rolling-back` is in
flight. The newest 20 settled journals are kept.

**Recovery (start of every install and go back):** each unsettled journal is
finished from what it recorded. A move that happened but was never recorded is
found by inode (a folder keeps its inode through a rename; each move records
the inode of the folder it carries to `to`). Nothing moved: remove the staging
copy, `failed`. Some moves done: undo them newest first, `rolled-back`. Every
move done and the app in place verifies: it stands, `healthy` with `confirmed:
false` (the helper died after the swap). Stopped after any failed start (between
tries, or the exit 9 below) or while `rolling-back`: finish going back and
write the failure file, judged by the last attempt: a journal whose last try was
healthy keeps its app (the crash came after it started). A journal
that can't be read, has another `schema`, or lacks `plan` or `movesDone`
(always written, `plan` possibly `null`; only genuinely optional fields
default) is renamed
`<id>.json.bad` and not obeyed. If its text names a state from which apps may
have moved (`snapshot-written`, `data-restored`, `promoted`, `started`, `rolling-back`) and it is
newer than the newest settled journal, every run stops (exit 9) until a person
checks the apps and deletes it; the message gives the by-hand way back (swap
`.Hollow.app.previous` and the app, or `node scripts/install-macos-app.mjs
--rollback`), since `--go-back` is blocked too. The newest 5 `.bad` files are kept. Then a
leftover staging copy (any settled newest journal) or discard (newest
`healthy`, `rolled-back` or `unchecked`) whose earlier removal failed is
removed again, unless a `.bad` newer than the newest settled journal exists;
one that can't be removed, or that no journal explains, stops the run (exit
9). The lock file is released by `Drop`, which a release build's
`panic = "abort"` skips: that lock's pid is then dead and the next run takes
it over.

**Reopen check (decided in U4):** `open -n <app>`; healthy when a process of
the installed executable (`ps` args equal to it or it plus `-flag`; not a
subcommand child, not the installer) appears within the start timeout and is
the same live pid when the settle time has passed. No marker file: apps in the
field write none, and a check needing the new app's help would roll back
updates from apps that predate it. A hung but living app counts as started. A
watched process that exits is replaced by any other process of the executable
started since the launch (a single-instance handoff), looked for for up to 1 s
after the exit, and the settle time starts over for it. One try watches at most
the start timeout plus three settle times (120 s by default): an app that keeps
exiting and handing off is a failed try (`never-settled`), so it can't hold the
installer and its lock forever. After a failed try only
the processes this launch started get TERM (never KILL), and only while each
pid keeps its start stamp; another process of the executable is reported and
left alone (one that started after this launch is named as a late start of it,
not as another Hollow). If one won't quit, or another is running, exit 9
without going back. Two failed tries: undo the moves, delete
the failed copy, write the failure file, reopen the previous app once, exit 8.

**`last-install-failure.json`** (`<journal dir>/`, written atomically before
the previous app reopens; U5/U6 read it at start and say why): `schema` (1),
`installId`, `at`, `attemptedVersion`, `restoredVersion` (null unless it went
back), `wentBack`, `reason`, `attempts` (as in the journal), `journal` (path).

**Sessions snapshot:** `<journal dir>/snapshots/<install id>-from-<old
version>/<name>.db`, folders 0700, files 0600. Not beside `.previous` in
`/Applications`: conversations are private, `/Applications` is readable by
every account and may not be writable by this one; the app data folder is per
user and on the databases' volume, and the journal pairs the snapshot with the
kept app. Each database is copied with `VACUUM INTO` (one read transaction: a
consistent copy even with a writer open; rusqlite's `backup` feature is off
and isn't needed), then opened read-only and must pass `PRAGMA quick_check`
with the source's `user_version`, which the journal records. The source is
opened read-only. Any failure stops the install and removes the partial
folder. A `failed` install, or one `rolled-back` before any start, removes its
snapshot (the live data is unchanged). After a go back from failed starts the
snapshot stays: the new version ran and may have migrated the stores, and U6
restores from it; the keep-3 prune removes it after three later installs. Each settled install prunes to the newest 3 snapshot
folders by modification time, never its own, touching only real folders named
`*-from-*` (no symlinks, no stray folders).

**Go back from the app (row U6):** `update_go_back` runs the helper from a
clone of the installed app (`<journal dir>/go-back-helper/<id>/Hollow.app`,
`ditto --clone`, checked first: signature, bundle id, the running version and
the same executable bytes), because `--go-back` swaps both bundles a helper
could run from (a running binary keeps working after its bundle is renamed,
but anything it loaded lazily from the bundle would not). The clone is removed
at the next launch, or before the next go back, while no installer holds the
lock. Flow, plan and skip rule: `docs/UPDATES.md`, "Go back (U6)".

**`--restore-snapshot <dir>`** (`update_install/restore.rs`): the folder must
be a direct child of `<journal dir>/snapshots/` and named by a journal's
`snapshot` record, which says which live database each copy belongs to.
Before anything moves: each copy passes `quick_check` at its recorded
`user_version`; it is copied to `<live>.go-back-restore` and checked again,
the check's `-wal`/`-shm` removed, and the copy and its folder fsynced; no
other process has the live database open (the `-shm` lock byte,
`hollow_databases::shm_in_use`). Any failure there: exit 12, nothing moved.
The renames join the go back's plan before the app swap: the live `.db`, and
any `-wal`, `-shm`, `-journal`, to `<file>.before-go-back` (all of a
database's files with `-<install id>` when any of their plain names is taken),
then the restore copy to the live name. The plan is journaled whole first; the restore moves
run, the restored databases are checked in place (`data-restored`), then the
swap. A failure at any point undoes every move (restore included: the live
files come back, the copies are removed); a crash is undone by the next run's
recovery (it never keeps a restore without its swap), which also removes
leftover restore copies with their `-wal`/`-shm`, and, once every journal is
settled, any `*.go-back-restore` beside a database a journal names (a crash
before the plan was saved). The `.before-go-back` files stay: they hold the
chats from after the update.

**Not covered yet:** the candidate profile and
`HOLLOW_TEST_BED` (the .mjs keeps those), and the provenance check and embed
self-test the .mjs runs (the signature and team stand in for them).

### macOS privacy grants after an update

Guidance, not behaviour. The installer (`scripts/install-macos-app.mjs` and `hollow install-update`) does not touch,
check or read privacy grants, and nothing in Hollow tests them. The idea comes from how the Orca project documents the
same trap (OC11); it has not been reproduced on this Mac, so treat each line as something to check, not a measured
result. The same note for the headless daemon is in
[HEADLESS_SERVE.md](../HEADLESS_SERVE.md#macos-privacy-grants-after-an-update).

- **A child is charged to its original path.** macOS attributes a helper process (a CLI, a computer-use helper, the
  `hollow serve` binary) to the app it was started from, at the path that app had then, not to the helper's own file.
  Permission prompts and grants (Accessibility, Screen Recording, Files and Folders) hang on that original app path.
- **An update can strand a stale denial.** The installed path never changes, but the bundle that held it moves to
  `.Hollow.app.previous` (pinned by the test "an update keeps the installed path and moves the old bytes to the hidden
  previous path" in `scripts/install-macos-app.test.mjs`). A helper started before the update keeps running from the
  old bundle and keeps being charged to the old path. The pane can show Hollow as allowed while the helper is still
  refused: a stale permission denial.
- **Re-grant after an update.** Accessibility and Screen Recording granted to the old path need a re-grant (turn the
  entry off and on again, or remove it and add it again, in Privacy & Security), then restart the helper. Don't expect
  the new app to inherit the grant.
- **For a helper that must keep a grant, run it from a signed copy started by launchd.** Copy the binary to a fixed
  path outside the app bundle, sign it, and start it as a launchd job, so its attribution doesn't depend on which app
  version launched it. The headless daemon already has this shape.
- **Find what still holds the old path** with `lsof +D <old folder>` (here `.Hollow.app.previous` beside the installed
  app, or the folder the helper was started from): it lists processes with files open under that directory. Quit or
  restart those, then check the grant again.

## Updates Worker and builder (U3, U2, 2026-11-15)

The update host for channel builds (updates design section 4) is a Cloudflare
Worker in `infra/updates-worker/`, plain ESM with no npm dependencies, over the
R2 bucket `hollow-updates`. It serves `/channels/<main|nightly>.json` and
`/builds/<0.6.n>/<file>` to a per-person bearer token (only SHA-256 hashes in
`people.json`, compared in constant time, one identical 401 for every token
failure, 403 for a channel not allowed), and `/install/<token>`, the teammate
install page. A manifest is served only when its archive, `.sig` and
`notes.json` are in the bucket; `infra/updates-worker/src/publish-order.js` is
the upload order the release builder and promotion follow (manifest last).
Tests: `scripts/updates-worker.test.mjs` (in `npm run test:scripts`). Not
deployed yet (U11). Contract and formats: [Updates](../UPDATES.md) and
[the Worker README](../../infra/updates-worker/README.md).

**Release builder (U2):** `scripts/release/nightly.mjs`, run every 300 s on the
Mac mini by the LaunchDaemon file `scripts/release/com.hollow.app-nightly.plist`
(written, not loaded; loading is Alex's one `sudo` line in
[Updates](../UPDATES.md#the-release-builder-u2)). Each run fetches
`origin/main` and, for a tip it hasn't published or failed on, builds it in its
own detached worktree (never a dev checkout) and runs the named checks in
order: gate record for the tip's fingerprint (else `npm run gate`), the build
(`build-app.mjs --no-verify`), verify (`--verify-only --no-selftest`), the
embed self-test (`--selftest-only`), the minisign `.sig` of
`Hollow.app.tar.gz` (`tauri signer sign`), `spctl` and `stapler` on the
unpacked archive, notes (`scripts/release/notes.mjs`,
a port of `install_brings.rs` plus new `docs/WHATS_NEW.md` entries) and the
manifest (checked with the Worker's own `parseManifest`; never lower than the
live one). It refuses an ad hoc, self-signed or un-notarized build up front.
Only then does it
upload in `publishOrder` (manifest last) through `scripts/release/r2-uploader.mjs`
(SigV4 over node `crypto`/`fetch`). A failed check uploads nothing, logs it
and writes `failure-<commit>.json` into its outbox. Tests:
`scripts/release-nightly.test.mjs`, `release-notes.test.mjs`,
`release-r2-uploader.test.mjs` (fakes and temp repos only). `build-app.mjs`
gained `--no-verify`, `--no-selftest` and `--selftest-only` for these steps.

**Updater client (U5):** `src-tauri/src/update_client/` checks the channel 45 s
after launch and every 30 minutes through `tauri-plugin-updater` (`~2.12`,
check and signed download only), unpacks into app data, verifies with
`update_install::verify` and only then reports `ready`; Restart to update and
the no-expiry Update when chats finish start `hollow install-update` from the
staged bundle. Inert ("Updates are not set up on this build.") until
`plugins.updater.pubkey` is filled (U11). Commands, events, settings, token:
[Updates](../UPDATES.md#the-updater-client-u5).

**Publish to Main (U8):** every Nightly now also uploads
`builds/<v>/manifest.json` (never served), so any kept Nightly can become
Main. `scripts/release/promote.mjs` (`status`, `list-nightlies`, `promote`,
`withdraw`) runs on the builder: it verifies the build's files and `.sig`,
writes `channels/main.json` last (no rebuild) and appends
`main-history.json` (the builder's prune takes the same `promote.lock`).
Hollow reaches it through the runner in one ssh call
(`update_client/publish.rs`, which `git archive`s `scripts/release` from the
builder clone's `origin/main`), only on a Mac whose `UpdateSettings.publisher`
is on. The agent action `build.publish_main` is `ALWAYS_ASK`. Rules, nudge,
commands: [Updates](../UPDATES.md#publish-to-main-u8).

**People (U9):** the Worker's `GET /me` gives a token's `{name, channels}`;
the client asks it on every check, limits the channel picker to it, moves off
a channel no longer allowed, and words a 401 as "did not accept this token (it
may have been revoked)". On the publisher's Mac (`UpdateSettings.publisher`),
`update_client/people_admin.rs` makes each person's token, sends only its hash
on ssh stdin to `scripts/release/people.mjs` on the runner (the builder's env
file holds the R2 key, strict host keys), checks each answer matches the
change, and returns the install link once. The script writes with a
conditional PUT (`If-Match`) and creates a missing file only on `init`; a
revoke takes effect within the Worker's 60 s cache. Details:
[Updates](../UPDATES.md#people-u9).

---

## Self-install CI compatibility (2026-09-29)

Tauri parses CI as a boolean (`true`/`false`), so the app build runner sets
`CI=true`. The `tauri:bundle` script explicitly passes `--ci` to override the
legacy installed runner’s `CI=1`; without it, upgrading through Build app
fails before compilation with an invalid value for `--ci`.

Archive: quirks, common tasks, glossary and other reference notes.

## Trace assertions (NV5)

`node scripts/agent-eval/trace-assert.mjs <stream.jsonl | folder>... [--json <f>]
[--skill <name>]... [--exempt <action id>]...`, or the same flags after
`scorecard.mjs trace-assert`. It scores the path an agent took, where the
scorecard scores the cost and the artifact. A stream is a Claude transcript or
a `claude -p --output-format stream-json` capture: tool uses in assistant
lines, results in later user lines (read as `readCalls` reads, but no `usage`
block is needed). Four checks, each `pass`, `fail` or `na`:

- `skill-never-read`: a skill the run named (a `Skill` call, a file read inside
  `skills/<name>/`, or `--skill`, the skill an eval row expects) whose SKILL.md
  was never loaded, or only after its other files were used. A `Skill` call
  that errored loaded nothing.
- `run-action-without-describe`: a `run_action` id with no earlier successful
  `describe_action` of that id (the MCP path is `list_actions`,
  `describe_action`, `run_action`). `--exempt sessions.wait` skips ids an agent
  may know. A describe in an earlier session of a resumed chat is not seen.
- `errors-recovered`: each tool error is followed within three calls by a call
  that succeeded.
- `wasteful-calls`: the share of eligible calls (not Bash, `run_action`, polls
  or session starters) that repeat an earlier identical call with nothing that
  changes state in between; fails above 20%, `na` under 5 eligible calls.

A summary counts `na` apart from `pass` and `fail`; `failShare` is fail over
pass plus fail, `null` when no run applied, so a check that did not apply is
never a zero. Only tool names, skill names, action ids and each error's class
are read; no prompt, text or result reaches the report. Fixtures:
`scripts/agent-eval/fixtures/trace-assert/`; test:
`scripts/agent-eval-trace-assert.test.mjs`.

## Skill lift (NV4)

`node scripts/agent-eval/skill-lift.mjs score <suite.json> <answers dir> [--k
<n>] [--json <f>]`, or `run <suite.json> --out <dir> [--repeats 3] [--library
<dir>] --live` to record the answers first. Does a skill help at all? A suite
names one skill and its cases: a prompt, the score.mjs rules its answer must
meet (the one grader both arms share), and `negative: true` for a case the
skill does not apply to. The first suite is
`scripts/agent-eval/fixtures/skill-lift/humanizer.json` (9 rewrites of
synthetic text full of AI writing tells, 3 negatives).

Answers are `<dir>/with|without/<case>.<attempt>.txt`. An attempt with a
`.fail` note (the agent ended in an error, its turn limit or the timeout) is a
fail; one with a `.error` note (spawn error, no result) is left out and makes
the report provisional. `<answer>.skills.json` lists the skills the agent was
offered (its stream's init event), so each arm's skill set is checked: the
without arm must never be offered the skill, the with arm always. Per arm: pass rate
(mean of each case's share of passing attempts), attempts with a Wilson
interval and pass@k (cases solved within their first k attempts; k defaults
to the most attempts recorded) with a Wilson interval. Lift is with minus
without over the cases both arms answered, with a seeded paired percentile
bootstrap over cases (2,000 resamples), shown overall, where the skill
applies, and on negative cases (the skill must not hurt them). Under 10
paired cases, with no negative case, with a run left out, or with an arm
offered the wrong skills, the report is labelled provisional.

`run` is paid: Claude Code, 2 arms x cases x attempts (72 for the humanizer
suite), only with `--live`, after clearing earlier answers. Both arms run the
same command (`-p --output-format stream-json --max-turns 4 --setting-sources
project --strict-mcp-config --permission-mode default`, no edits or shell),
taking turns case by case, each in an empty scratch folder; only the with
arm's folder links the library's skill as a project skill. Project settings
only means neither arm loads the person's user or plugin skills (a user
copy of the same skill would otherwise reach both). Never in the gate; the tests write answers from
`fixtures/skill-lift/humanizer-answers.json` and use a fake spawn
(`scripts/agent-eval-skill-lift.test.mjs`).

## Judge qualification (NV6)

`node scripts/agent-eval/judge-qualify.mjs run --judge rules|claude|codex --out
<dir> [--items <file>]... [--repeats 3] [--live] [--json <f>]`, or `score
<verdicts.jsonl>` to re-score a run. It measures the judge, not the agent:
before a judged score is trusted, does the judge agree with itself and with
the labels?

Items are the pilot suite's judged blocks (`pilot-tasks.json`): each task's
reference answer labelled pass and each of its `fails` labelled fail (26
today), with the task prompt and fixture evidence; `--items` adds
hand-labelled ones (`{schema: 1, items: [{id, prompt, evidence, answer,
label}]}`). The judge's prompt holds the task, evidence and answer, never the
label, the reason or the reference answer. Each item is graded three times.
A hash of each item's task (a hand-labelled item: its id) splits the items
about half and half, so one task's reference and failing answers stay on one
side: the dev set is where a judge prompt may be tuned, and only the held-out
set (4 tasks, 12 items today) decides. Per set: self-agreement (items whose
repeats are unanimous, pairwise agreement, the ids that flipped) and label
agreement (majority against the label, per label, Cohen's kappa; a tie or no
verdict is undecided and counts as a miss). A judge is qualified when the
held-out set has both labels, self-agreement is at least 0.95, label
agreement at least 0.9 overall and for each label, kappa at least 0.8, and
every item has all its verdicts (no unknown or duplicate records);
provisional when that holds on fewer than 10 items; otherwise not qualified
with the reasons. A judge that always says fail is refused by the pass
label and kappa, however many fail items there are.

`rules` is score.mjs's deterministic rule scorer, free: the baseline, which
qualifies (26 of 26 unanimous and on the label, kappa 1). `claude` (`-p
--output-format json --max-turns 1 --tools "" --strict-mcp-config`) and
`codex` (`exec --ignore-user-config --sandbox read-only --ephemeral`) are paid,
one call per item and repeat (78 today), run only with `--live`, each in an
empty temporary folder of its own (so it can't read the labels or the
repository's instructions from disk) and settled after 120 s. Never in the gate; the tests use the rule judge and a fake spawn
(`scripts/agent-eval-judge-qualify.test.mjs`).

## Skill routing eval (BI1)

`node scripts/agent-eval/skill-routing.mjs score <cases.json> <streams dir>
[--agent claude|codex]... [--library <dir>] [--json <f>]` scores whether the
right skill fired; `run <cases.json> --out <dir> [--agent ...] [--limit <n>]
--live` records the streams first. A case is a prompt, the one skill that should
fire (`expect`, null when none should) and its `confusable` near misses;
`scripts/agent-eval/fixtures/skill-routing/cases.json` holds 20 synthetic cases
over the shared library `~/.agents/skills` (4 expect no skill).

The choice is the first skill the agent reaches for: Claude a `Skill` call, a
`Read` or shell read of `skills/<skill>/SKILL.md`, `hollow_vault read_skill` or
`run_action skills.read_skill`; Codex (`codex exec --json`) the first shell
command naming a `skills/<skill>/SKILL.md`, or the same MCP reads. "No
skill" needs a normal end (Claude's `result`, `error_max_turns` included;
Codex's `turn.completed`); an empty stream, an error end, no end, or a
live run's `<stream>.error` note (spawn error, timeout, non-zero exit) is a
failed run, left out of the accuracy like a case with no stream (not run),
never a miss. Each agent is scored apart: right, confused (a listed near miss
chosen), wrong, missed, fired with none expected, with a Wilson interval;
every failed confusable pair is named, `(none) -> x` when a none-expected
case fired its near miss. Claude also sees its own user and plugin skills, so
its "wrong" can name a skill outside the library. Folder names map to frontmatter
names and a `plugin:` prefix is dropped.

`run` is paid (one call per case and agent, about 40 for the shipped set).
Without `--live` it prints the plan and spawns nothing; it refuses a case whose
skill the library lacks. Each run is in a scratch folder that links every library skill
as Claude's project skills (and as the folder's own agents skills for a library
other than the shared one), Claude with `--disallowedTools Edit Write MultiEdit NotebookEdit Bash`,
`--max-turns 3`, `--strict-mcp-config` (no MCP server) and `--permission-mode
default`, Codex with `--sandbox read-only --ephemeral --ignore-user-config`
(the sandbox does not cover MCP servers; without the user config none runs,
and no memory is written), and is stopped as soon as its choice is seen or
after 180 s with none. Reports hold case ids, skill names and
counts only. It never runs in the gate; the tests use recorded streams
(`fixtures/skill-routing/streams/`) and a fake spawn
(`scripts/agent-eval-skill-routing.test.mjs`).

## Lane tooling (OR2, LF3, BI3, BI6, DOC1)

Deterministic helpers for lanes and overnight runs. None calls a model or the
network, none is in the gate yet, and each is run by hand or by a planner.

- `node scripts/lane-preflight.mjs --claims <dir> --paths a,b --now <iso>`
  (OR2): a claim lists the `paths:` it expects to touch, one JSON or markdown
  file per claim. The same file in another lane's live claim blocks (exit 1); a
  neighbour in the same folder, a claimed folder holding the file, or a module
  and its folder warns (exit 0); a claim older than 72 hours is listed STALE and
  holds nothing. The clock is `--now`, so the verdict reads no wall time.
- `node scripts/agent-eval/run-facts.mjs <transcript.jsonl> [--json] [--role-prompt f]`
  (LF3): identical tool calls run 12 or more times, repeated error text,
  ping-pong pairs, attempts and reworks per edited file, wall against active
  time, and the largest tool result's share of tokens. It suggests patches from
  a closed set (`change-retry-limit`, `add-gate`, `remove-gate`,
  `append-role-prompt-line`); a person applies one, the tool never writes.
  Fixture: `scripts/agent-eval/fixtures/run-facts/`.
- `node scripts/lane-triage.mjs <ledger.md>` (BI3): every row has a triage
  outcome (work, close, block, drop); a block needs `Unblock:` (a checkable
  condition) and `Owner:`, a close `Evidence:`, a drop `Why:`. A row is never
  deleted on suspicion; it is blocked with the question.
- `node scripts/lane-finish.mjs` (LF2), `lane-extract.mjs` (LF4) and
  `lane-outcome.mjs` (BI2): a fresh-artifact finish check against a stage-start
  baseline, a section-extracted brief that refuses on a missing or contradicting
  source, and a git check that a claimed PR or branch is on `origin/main`,
  pushed and clean, plus board facts as JSON; none runs from `land.sh`.
- `node scripts/anti-goals.mjs --rules <file> "<candidate item>"` (BI6): rejects
  an item that matches a dated rule in the rules file, naming the rule and its
  date. Its default rules file was removed (ENG-674), so pass `--rules`.
