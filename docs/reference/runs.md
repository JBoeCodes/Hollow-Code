# Runs: usage, budgets, results and resume

Part of the agent manual. [AGENTS.md](../../AGENTS.md) is the entry point and
links every reference doc; this one holds CLAUDE.md's "Runs" section, moved
there word for word on 2026-10-13 (only relative links were re-based). Read it
before changing the usage or run ledger, budgets, mid-run enforcement, the
friction loop, the flight recorder, the step spine, bake-offs, the value
ledger, result kinds, resume, scratch space, the watchdog or phase timings.
Keep it current with the code in the same change, as AGENTS.md asks.

## Runs: usage, budgets, results and resume

Company work was removed 11-10 (L11). A queued row it left (`restrictedResume`,
an `s-company-` chat turn) ends once as failed, "company work was removed
(11-10)" (`run_pump::removed_company_row`). Record:
company assignments.

What happens around every agent run. The full per-PR contracts (Paperclip
PRs 6-17 and the merged-plan changes to them) are in the
runs archive; each heading
below has the same name there.

### Usage ledger (Paperclip PR 6)

P1-31: cost-chip estimates name their source: Claude Code, Hollow's saved
model price for Codex, or the other agent. The usage popover clears account
and plan data on open/provider change and discards replies from earlier reads.
The agent descriptor's reportsContext capability controls the empty context
copy; Antigravity is the only current provider that does not report context.
Quota opt-in fetches independently of accounts and preserves open context/cache
details. Plan percentage labels share precision across chat and Settings:
whole percentages from 10 upward, one decimal below 10 when fractional;
overage remains visible and meter values retain the reported precision.

Self-hosted models (Q8, 2026-11-03): a Claude run whose credential is a
Hollow run key (`self_hosted::is_run_key` on `ANTHROPIC_AUTH_TOKEN`) is
`Billing::SelfHosted` (`"self-hosted"`), from `usage::classify`. Its row keeps
the tokens under the served model id (Qwen: `qwen3.8-27b`) and never a price:
`cost_status` is `unpriced` and `cost_usd` null, so Claude Code's own estimate
for an id it doesn't know (`costBasis: unknown`, $0.84 for Q0's warm turn) is
dropped, as is the live estimate, and the transcript's `result` event
stores `costUsd` null. It has no Claude account (`account_ref` none).
`UsageRun::set_self_hosted(name, served id)`, called beside `set_billing` on
the first attempt and on a fresh retry's new row, puts the endpoint's name
into `usage_json` (`endpoint`) and the turn's usage event and records the
served id as `requested_model`, whatever the person picked. The cost chip
shows tokens with "Billed by GPU time on <endpoint>"; the usage popover
shows an Endpoint row in place of the Claude account, plan row and Claude
usage section (`chatUsage.popoverEndpoint`: the last turn's, kept while the
next turn runs and its usage is cleared, or a `self-hosted:` chat model id),
and Billing "Billed by GPU time" (`billingLabel`). `set_self_hosted` does
nothing unless `set_billing` said self-hosted, so a build failure's stand-in
row gets no endpoint. GPU time is no Anthropic spend, never unknown and
never shown as $0: the night guard's estimate is 0
(`UsageRun::night_estimate`); `descendants::rollup` counts such rows in
`gpuTime`, not `unknown`, with no known spend, and keeps every sum with one
null, so `night_queue::spent_tonight` neither charges nor fails closed on
them and the morning report says "billed by GPU time"
(`MorningItem::gpu_time`; its header adds "(plus N billed by GPU time)",
`MorningReport::gpu_time`, as does the Overnight cap row);
`budgets::unattended_spend` skips them. Budgets count them under Global only
(`LedgerRow::gpu_time`, `InFlightRun::gpu_time`): `Scope::Agent(Claude)`
leaves out their tokens and runs; a Global spending budget counts them in
`gpu_time_runs`, never `missing`, so it is never a lower bound for them, and
its card lists them as "billed by GPU time", not as runs with no usage
figures (`UsageTotals::shown_unknown`). Settings → Usage lists
them in their own By agent row, `self-hosted:<served id>` ("Self-hosted ·
qwen3.8-27b"), counted in `gpuTime` instead of `unpriced`, with the cost
"Billed by GPU time" (`bucketCost`, and `agentAmount` in the agent split).
The run outcome and the flight recorder share one wording
(`runOutcome.gpuCostText`, `runCostSuffix`), as does a night item's cost
in Runbooks (`usageFormat.nightItemCost`); a partial self-hosted turn's chip
names GPU time too.

Codex app-server token notifications emit nonterminal turn.usage snapshots.
UsageAccumulator keeps the latest per-turn delta outside message/call counts;
a final report supersedes it. Stop, failure or EOF without a final report
therefore retains partial tokens in the existing ledger row. Resume baseline
subtraction stays in the bridge, and no token notification stays unknown.

Optional account/rateLimits/updated notifications produce passive Codex plan
windows, held only in memory by provider_accounts/limits.rs. The launch's
auth-file metadata owns a snapshot; another login, a passed reset, or a reading
older than 15 minutes hides it. No quota request is started. Chat's usage card
and Settings → Usage show the last reported time, percent used and used-fill,
or “Codex didn't report limits.” Native snapshot availability remains a hand
check; local protocol fixtures do not establish that the installed CLI emits it.
An auth-file token refresh also changes the stamp, conservatively hiding the
old reading until a newly launched run supplies another one.

Settings now opens on lazy daily cost graphics, per-agent amounts and a direct night-queue band; budget gauges
reuse V1 geometry. Recording-day fields, coverage, attribution limits and code map: [Usage instruments](../USAGE_INSTRUMENTS.md).


Exactly **one usage-ledger row per agent invocation**, from every launch
point (chat, workflows, heartbeat, one-shots, ingest, teammate delegations),
inserted `running`, checkpointed at most every 2 s and finished exactly once
(`success` / `error` / `cancelled` / `timeout` / `spawn_failed` /
`budget_stopped`; `interrupted` after a crash). Unknown values are NULL or
`partial`/`unknown`, never zeros; a resumed Claude turn records only its own
share, except that when its token delta can't be verified (a cancelled turn in
between stored no terminal) but the session's cost rose, it records that rise
once as a span over the runs since the last terminal (`session_delta_span`,
`usage_json.spanRuns`; OB24). A `partial` row with tokens but no cost carries `cost_floor_usd`, its
streamed tokens at the price table (OB07): a lower bound reported apart,
never in a known total or a budget, and left out once a span covers its run. Code: `usage.rs` (capture), `usage_ledger.rs`, the `usage_ledger` table
in the Sessions DB. Shown in Settings → Usage and the chat cost chip. A
teammate's row carries its parent's run id; since F10 D2 so does a turn's
learning check (`Meter::oneshot_for_run`), and `descendants::rollup` sums a
run with its descendants (Agent teamwork).

### Run ledger v2 (master plan F2, 2026-10-08)

Each row also says **who paid and what it was for**, written at launch by
`UsageRun::launched` from the command's cwd and account env
(`run_attribution.rs`): `account_ref` (`login`, or `acct-<12 hex>` hashed
from the Claude account id; never the token or name; other agents NULL),
`workspace_id`, `department` (a brief's `department:`, one of Operations,
Finance, HR, IT, Revenue, AI) and `work_kind` (a brief's `work_kind:`, else
`code` for a build workspace, `admin` for the Main agent, else NULL).
`outcome_label` (accepted / revised / abandoned / unknown) is derived from
Results acceptance (`acceptance.runId`), the run's tracker issue (G4: Done,
completed after the run started; `tracker::work::accepted_by_issue`), turn
feedback (`notUseful`) and how the run ended, and refreshed before every
read (`run_ledger.rs`, `acceptance_labels`). Older rows
read as `unknown`. A failover's attempts are separate rows, each with its
own account. Every Jev request is a `oneshot`/`jev` row with unknown cost.
Prices: `usage_prices.rs`, one dated, sourced row per model (built-ins plus
`usage-prices.json` in app data), used only when the agent reports it could
not price a model (`costBasis: unknown`), and for Codex (overnight 9 A13):
a complete Codex row whose model (requested, else the rollout's
`turn_context` model, `codex_rollout::turn_model` → `UsageRun::set_turn_model`)
has a row is `estimated` at `usage_prices::list_usd` (uncached input at the
input rate, cached at cache read, output), with `priceSource` in its
`usage_json`; a row prices its own model id, or that id plus a release
date (`-YYYYMMDD` or `-YYYY-MM-DD`), never a longer id (`gpt-5` is not
`gpt-5-mini`'s price; `usage_prices::row_matches`, RF-817-F1); with no row it stays unpriced (unknown, never $0). No Codex row
is built in. Settings → Advanced → **Prices** (`settings/PricesPanel.tsx`) lists
`price_table` (rows carry `saved`) and saves through `set_price` after an
inline confirm; Remove only on saved rows. Actions: `usage.ledger_query` and `usage.price_table` (read),
`usage.set_price` (confirm), `workspaces.set_department` (write; refused for
Chats, the Main agent and tracker-managed workspaces; a legacy `notion-*` one is plain since 2026-11-10). Named scopes in
`budgets.rs`: `main-agent` (the Main agent's daily stop, number still in its
settings) and `overnight` (unattended runs started 18:00–07:00 on weeknights,
$40 a night, `budgets::overnight_spend`; the F1 night queue adds its own
turns' cost and stops at the lower of its setting and $40, mid-turn too, from
running estimates: [docs/SCHEDULER.md](../SCHEDULER.md)).

A chat shows that kind's usual length beside the time the reply took
(`usage.job_timing`, `run_ledger::typical_duration_ms`). It is the median of
the last 200 successful `chat` rows of the same `work_kind` whose agent
launched (not a delegate, not a workflow, not a run still going, not a
row that never started, and not a run longer than a day).
Fewer than 3, or an unknown kind, shows nothing. The hours a person says
that kind saves (Value this month) are a different number.

### Friction → fix loop (master plan #13, 2026-10-09)

`friction.rs`: patterns of the last 30 days from failed chat steps (the F6
`steps` table; also a shell step that exited cleanly but whose full output
shows a check's failure, e.g. `npx vitest run | tail` printing
`Tests  2 failed`: `step_spine` marks its checkpoint `checkFailed` from the
`checkScopes` (runner ids or kinds) it noted at start, and it is signed `check-failed` /
`failed check from <head>`; OF16), failed or refused Hollow app actions (`.app-actions.jsonl` +
`.1.jsonl`; never a `denied` card, which is the person's decision; a
`timed_out` card counts) and the person's explicit `notUseful` votes
(`turn_feedback`; corrections in chat count only through the vote). The
reply row's Not useful writes one again since overnight 11-12 (C2).
`without_noise` drops what isn't Hollow's wall:
- a shell step (`Bash` for Claude and Codex, Grok's `run_terminal_command`)
  that failed with no output (`Exit code N` alone; Codex's `Exit code N (no
  output)` since OF18, or `failed with no output` on its older steps, whose
  results carried no exit code). A failure with output is signed by its exit
  code and command head (below); when the step's checkpoint doesn't name the
  command it loses its `Exit code N` line, so it is grouped by what the
  command said.
- a failed `hollow_app` tool step that the action log already records for the
  same run and message, or that only carried back the person's Deny
  (`denied_keys`).

`routine_exits_dropped` (overnight 11-08, G3) then drops a shell exit code
alone: a signal signed class `exit-code` stays only when its command held a
check (`Signal::check_run`, read from the step's `checkScopes`: a test run, a
typecheck, a lint, so the exit is that check failing) or the same command and
code recur in 3 or more chats (`EXIT_MIN_CHATS`) on 2 or more days
(`EXIT_MIN_DAYS`, by the first 10 characters of `at`). A dropped pattern loses
every hit and returns whole once it crosses the line. `check-failed`, errors
with their own words, failed or refused actions and votes are never filtered.
`list_view` (`friction.list`) orders new patterns (first seen in the last 7
days) before growing ones (the last 7 days hold more hits per day than the 23
before) before steady ones, then by this week's count, and each pattern carries
`trend` (`new`, `growing`, `steady`).

A mistyped action id gets up to three nearest ids the run may use
(`app_mcp::nearest_actions`; a first-class tool name names its action). The
signature is SHA-256 (16 hex) of tool, error class (a leading UPPER_SNAKE
code, else a word family, else `error`; Claude Code's approval refusals,
"requires approval", "was blocked", `simple_expansion`, "changes directory
before running", "haven't granted", are `permission`) and the first line
normalised (`claude_runner::redact`, then paths, ids, long hex and numbers
replaced, lowercased; quoted values kept). A shell step whose result starts
`Exit code N` is signed instead by class `exit-code` and `exit code N from
<head>` (`friction::signed_as`; the head is the command's first word past
`NAME=value` and a leading `cd <dir> &&`, plus a launcher's subcommand:
`npx vitest`, `cargo test`; since SC1 it first unwraps a `sh`/`bash`/`zsh
-c`/`-lc '…'` wrapper and skips leading `set …;`, `source …;` and `. …;`
prefixes, so `bash -c 'set -euo pipefail; npm test'` is `npm test` and an
existing `exit code N from zsh`/`from bash` pattern re-signs under the new
head, the old one dropping to count 0), so a runner's banner or a NO_COLOR
warning no longer makes its own pattern (HF02, 2026-10-16; no migration: a row signed
the old way stays under its old signature at count 0 after the next recount,
its status not carried over, and the new signature counts the whole window at
once, since every recount re-signs the raw signals). Stored in the session
store's `friction` table (counts, chats, week count, ≤5 redacted examples, sources, status
`open`/`dismissed`/`drafted`), recounted by the `friction` nightly window job
and on every `friction.list`; statuses survive a recount. **Drafts only from
the person:** `friction.draft_fix` is in `ALWAYS_ASK` (a card in every mode,
refused from automations and check-ins; its card refuses an unknown,
dismissed or drafted pattern or a full week). Its one visible home is the
weekly review's Do it card. G2 (overnight 11-08) cut the Settings → Usage →
Friction page, its jump link and the three Tauri commands behind it
(`friction_list`, `friction_set_status`, `friction_draft_fix`); both
`friction.list` and `friction.draft_fix` are now registry verbs with no
command of their own, and `set_status` (Dismiss/Reopen) stays for the review's
Never answer. A draft creates a build workspace (`workspaces_api::create_build_workspace_with`) on
local branch `fix/friction-<sig8>` and sends one chat there (default agent;
`auto` for the person, never above the asking agent's mode) with the packet
(`friction::packet`: pattern, examples, red-first, the gate one step at a
time, never push/PR/bed/candidate/install). At most 3 drafts in 7 days
(serialised). The bypass wording agents read names it among the always-ask
actions (`mode_contract::app_cards`; fixture regenerated).

### Shared run record (P1-29)

Chat's reply menu → Show run record, workspace Activity → a run, and
Automations → an automation share ExecutionReceipt's lazy What happened head
and its Run record disclosure. Changed, Checks, Outside its files and Not
visible to Hollow use the same normalized facts. Automations keeps cost and
guarded retry from its own model, with paths listed once in the disclosure;
its result preview remains separate. The head and disclosure share the newest
run's matching wire; a newest run without a receipt never borrows an older
one. Live recall attaches only when its run ID matches that wire, and clears
when the automation changes. Automation file rows keep a separate History
action. Only the run-history list is compact.
Activity includes failure/check excerpts and Timeline. Disclosure heading IDs
are per instance. Chat's latest retryable turn owns the existing P1-27 Retry
and provider menu; historical records receive no retry authority.
Chat's check chip opens the shared wrapper's disclosure and focuses its
summary, including when reopening a record the person collapsed.
Timeline retries use the shared themed button styles in every host; keep their
existing retry authority and busy disabling.

Copy command preserves the exact sanitized stored words, including a stored
ellipsis, with honest shortened-record wording. It cannot restore wrappers,
environment assignments or bytes omitted by the recorder. Ask to rerun exists
only in Chat, appends through addToDraft and never sends. Repository paths open
against their recorded root. ReceiptCite.target is optional: newly resolved
problem cites keep their canonical absolute file after the restricted-path
filter; legacy/missing/unresolved items remain text. Each target is independent
of the current workspace. No capture limit or raw-shell recording is added.

The Preview pane carries line/end separately from the path. Loaded path
and request nonce must match before source lines scroll/highlight; a newer
file wins late loads. Out-of-range lines name the current file/preview limit.
RunOutcome, ReceiptCheckActions, ReceiptCites and PreviewSourceLines stay lazy.

### Flight recorder (master plan #15, 2026-10-09)

A receipt's **Timeline** (`ExecutionReceipt.tsx` → `FlightRecorder.tsx`,
logic in `flightRecorder.ts`; open by default on a failed receipt, loaded
only when opened) shows `run_steps` as a tree: steps in sequence, a step
nested one level per open delegate (`parentSeq`; a missing or later parent
shows at the top level). The first failed step (`firstError`: error,
interrupted or stopped) and what fed it (`step_spine::fed_by`: the model
step before it at the same nesting, its delegate, earlier write steps whose
file its input names) are marked. Each step has **Retry from this step**,
plus Retry from the start (`retry_run_from_step`, the person's path: it may
put files back), then opens the new conversation. The same panel shows the
run's context manifest (`run_context_manifest`: layers, fallbacks), its
cost (`costRollup.totalUsd`, else "unknown") and the run's hashed account
labels (`accounts`); per-step cost is not recorded and the panel says so.
A non-chat run (an automation's receipt) has no steps; the panel says the
timeline is unavailable.

### Step spine (master plan F6, 2026-10-09)

`step_spine.rs`: a `steps` table (session store; `SCHEMA` run with the
others) with one row per step of a chat turn, written as the stream arrives:
`chat.rs` calls `turn_started` (first `model` step), `tool_started` /
`tool_finished` on each tool event and `finish_turn` after
`session_store::finish_turn`. Kinds `model` (the model's work before each
tool and after the last), `tool`, `delegate` (`Task`/`Agent`, `ask_agent`,
`start_session`, `send_to_session`; later steps nest under the newest open
one, a heuristic: parallel tool calls can mis-nest) and `write` (`Write`,
`Edit`, `MultiEdit`, `NotebookEdit`, Codex `file_change`). A write step's
named files are stored in History's object store via
`automated_history::store_step_object`, pinned in
`.hollow-history/step-pins/<run>.txt` (retention counts pins of retained runs
as references and sweeps the rest; only a live run pins). A `running` step of
a turn no longer live reads `interrupted`. Checkpoint per step: tool, input,
result (≤300 chars), status, and `resultBytes`, the full result's UTF-8 size
before the cut (repo features R6; older steps have none). `context_use.rs`
sums the main-thread sizes (`parent_seq` NULL) for the usage ring's "What's
using it" and `usage.conversation_context_use`. **Retry from step n** (`runs.retry_from_step`,
confirm: a card outside bypass, asks in auto, chats only, depth-checked; Tauri `retry_run_from_step` for the person) is exact or refused:
`wanted_versions` puts each file of the turn's History run back to the newest
version a write step ≤ n recorded, else its before-version, through
`automated_history::restore_to_versions` (History's own CAS restore plan, one
undoable `historyRestore` run; a file already at the wanted version must
still match the after-version). It refuses, changing nothing, after an
unrecorded writer (shell, teammate, any non-read-only tool) at or before n
when the run changed files, a write outside the vault, a file the run's
record doesn't list, or a History conflict. From an agent it refuses
whenever files must be put back (its own turn holds the vault, as with
History restores). The retry is a new conversation in the same folder, same
agent and model (the run's ledger row), at the original mode or weaker and
never above the asking agent's, seeded with the task-state record before
the turn plus checkpoints 1..n, and says it is not a replay. Per-step cost
and account aren't recorded yet (the run's are on its receipt and ledger).

Review fixes (2026-10-09): a write step's paths are taken from its full
input at `tool_started` (kept in the checkpoint's `paths`; the stored event
is cut at 8,000 characters); a write step at or before n with no recorded
files (its result never arrived) refuses; a before-version History couldn't
copy refuses rather than reading as absent; a run pins at most 64 MiB of step
versions (then `uncaptured`); an agent's retry refuses only when files
actually differ from how the run left them; a failed start after a restore
says which History run put files back.

A shell step that holds a check also keeps `checkOutcome` (`passed`,
`failed` or `unknown`: the receipt's outcome, read from the full result by
the evidence collector and handed to `tool_finished`), which the task-state
record reads by tool-use id (`session_store::task_turns`) to call the step
`failed`, `done` or `unconfirmed` (OF16).

### Model bake-off (master plan #14, 2026-10-09)

bakeoff.rs (removed 11-10, S6): `bakeoff.start` (confirm; asks in auto; chats only, never an
automation or an unattended Main agent turn) runs one task on 2–4
agent/model pairs, any installed agent alike, **one at a time**, each as an
ordinary `plan`-mode chat (no writes) whose cwd is a text-only scratch copy of the workspace in
`<support>/Bakeoffs/<vault>/<id>/<opaque>/` (no `sessions/`, dotfiles or
symlinks; ≤2 MB a file, 50 MB in all; removed when the variant ends). That
folder runs bare (`mcp_attach::bare_scratch`, overnight S7): no Hollow tools
or connectors, no retrieval, recall, operator profile or skill index, a
vault-free brief, no learning.
Estimates: 60k in + 8k out at list price from `usage_prices` (errs high);
an unpriced pair reserves cap ÷ n and counts as that after it runs (never
$0); start refuses when estimates pass the cap (≤$50); a pair is skipped once
spent plus its reserve would pass it; a running turn is not stopped mid-way.
All bake-offs started in a UTC month together reserve at most $50 (real costs count once known); these cap estimates and launches, not spend: a variant that uses far more than its estimate can pass the cap, after which every launch and start refuses. While blind, costs, cost-known flags and skip/error reasons are hidden, and scratch folders have opaque names. A bake-off advances only where background agents are allowed (not in a candidate build unless asked). Variants
advance from `main_agent::after_chat_finished` and its minute timer
(`bakeoff::advance`). Grades: `bakeoff.grade` (any agent; always stored as
`ai`) and the **user-only** `bakeoff_grade_person`; 0–10 per rubric
criterion (default correct, complete, clear); blind (shuffled letters, pairs
and costs hidden) until the person graded every answer. Routing suggestions
(`routing.suggestions`) need 3 finished, graded bake-offs of a work kind;
"cheaper" only when both costs are known. **User-only**
`routing_accept_suggestion` writes `routing.json` (app data); v1 records the
route and does not yet change which model a new chat uses. The results page
was removed (overnight 11-08 S5). The runner stays until the next cut.
Per-variant effort is not offered:
effort is a per-agent Setting, not per chat.

Self-hosted models (Q11, 2026-11-03): a pair may name a switched-on
self-hosted entry's model id, `self-hosted:<entry>`, on the agent its API
style rides (`self_hosted::harness`, Q9's one helper that says which: Claude
for `anthropic`, Codex for `openai`). `bakeoff::self_hosted_entry` checks it at the start (`plan`,
so the card and `bakeoff.start` both refuse with a plain reason and "Nothing
started.": no such entry, switched off, an API style Hollow can't run, a
mismatched agent ("Qwen3.8 27B runs on Claude, not Codex: pair it with
claude"), an entry that fails `self_hosted_registry::normalize`, or a list
that can't be read) and again when its turn comes (`step`, through
`Launcher::self_hosted`). The start also pins what the entry runs on
(`Variant::self_hosted_pin`, a `SelfHostedPin`: the upstream's normalized
address and sign-in, the comparison `self_hosted::turn_route_with` makes, plus
the served model; kept in the bake-off's file, never in its report). Since
the Q11 follow-up the file holds only a SHA-256 of those three (`hash`), so a
sign-in command's arguments and the address are never copied into it; a file
with Q11's clear pin reads by hashing it (`PinIn`) and the next save drops the
clear fields; a load writes such a file again at once (`rewrite_clear_pin`:
atomically, only while no bake-off step holds the module's lock and the file
is as it was read), since a finished bake-off is never saved again. A
variant with no pin (a bake-off started before Q11) is
skipped with its own reason: the start recorded nothing to compare, so start
a new bake-off. When its
turn comes, an entry switched off, removed or changed to another style, or
pointed at another address, sign-in or served model than the pin, is skipped
with that reason ("Qwen3.8 27B changed in Settings → Models → Self-hosted
models after the bake-off started (its address, sign-in or served model), so
it was skipped rather than run as another model under this name") and never
runs on another model; a rename only takes the entry's new name. A turn that
then fails before its agent starts (its entry changed while the turn waited
for a chat slot, so the chat route refused it: no ledger row `spawned`,
`Observation::spawned`) is skipped with the route's reason too, not counted as
a run. The registry is read only for such a pair (`PlanEnv::self_hosted`).
The id is kept in one form, and the variant stores the model menu's name
(`Variant::model_label`, "Qwen3.8 27B · self-hosted"), which the card, the
report (`modelLabel`), the Markdown table, routing suggestions
(`Suggestion::model_label`; `model` stays the id, which an accepted route
records) and the Routing panel (`pairLabel`) show. It is billed by GPU time
(`Variant::gpu_time`): no estimate, it reserves nothing against the
bake-off's cap and takes no share of it (an unpriced pair's equal share is
the cap ÷ the pairs billed per token, so adding a self-hosted pair changes no
other pair's reservation), the cap never skips it, it counts nothing once it
has run (its ledger row has no price, Q8), and a bake-off of self-hosted pairs
alone reserves nothing against the $50 monthly cap (the month check applies
only when some pair is billed per token). Such bake-offs are limited by count
instead: at most `MONTHLY_SELF_HOSTED_ONLY` (10) start in a UTC month
(`self_hosted_only_this_month`), and the eleventh is refused with a plain
reason. Its cost reads "billed by GPU time" only once it ran (`gpuTime` on an
ended variant; "not run" when skipped, "not run yet" before its turn;
`gpuTimeRuns` counts ended ones; never $0 and not counted in
`unknownCostRuns`). While blind, `modelLabel`, `gpuTime`, `durationMs`,
`gpuTimeRuns` and (Q11 follow-up) `unknownCostRuns` are hidden like every
cost (a self-hosted run isn't counted in `unknownCostRuns`, so with one
skipped it could say which letter that was), so a cold start's length doesn't
point at a self-hosted answer. A routing suggestion never calls it cheaper on
either side (`PairStats::gpu_time`); it competes on grades like any pair, and
its "best" text says "billed by GPU time". `bakeoff.list` (and
`bakeoff_list`) returns `selfHostedPairs`, each switched-on, valid entry with
its agent, id and menu name, from the registry only, and the Routing panel
names them (`selfHostedOffer`); starting a bake-off is still an agent's
`bakeoff.start`, whose description names these ids. A variant launches
through `send_chat` like any other, so its turn is an ordinary self-hosted
chat turn (credential isolation, effort and window, runs on this Mac, GPU-time
usage) and nothing wakes the model before it: the turn's own first call does.
Timing: the bake-off itself has no per-variant timeout (`advance` only
observes the queue row), and a self-hosted chat turn's no-output watchdog and
deadline are raised to at least `self_hosted::COLD_START_FLOOR` (16 minutes:
`API_TIMEOUT_MS` and a minute more) by `self_hosted::cold_start_limits` in
`execute_turn`, so a watchdog or deadline set shorter in Settings doesn't end
a cold start. Limit: an entry pointed at another address, sign-in or served
model after its variant launched but before the chat route reads it (while the turn
waits for a chat slot) runs as edited; the launch check is the last
comparison with the pin.

### AI value ledger v1 (master plan #6, 2026-10-09)

`value.rs`: a month's cost against estimated hours saved, by department and
work kind, from ledger v2. A run (all rows sharing its run id) saves time
only when its `outcome_label` is `accepted`, and then the person's baseline
minutes for its work kind (`value-baselines.json` in app data) or that run's
own override. **Accepted (G4, 2026-11-10):** a Result the person accepted,
or the tracker issue the run is tied to (`work_links`, X1) is Done (state
kind `completed`) with `completedAt` after the run's turn started; once per
run either way. Done before the run, reopened since, or mirrored before
Hollow read `completedAt` (it fills at the issue's next sync) counts nothing.
Until a month has an accepted run, hours saved are **hidden**: the summary's
`hoursSaved` (`shown`, `acceptedRuns`, `byIssueDone`, `byAcceptedResult`,
`hoursEstimate`, `basis`) says `shown: false`, Settings leaves out the line
and the Saved column, the weekly update leaves out its line and the report
reads "no accepted runs". `tracker_issue_work`'s `WorkRun.accepted` marks a
run counted. No baseline, no saving, and accepted work with none reads
**"baseline not set"**, never 0 h (`ValueTotals.unbaselinedAccepted`; work of
`unknown` kind, which can't have one, reads "work kind unknown",
`unknownKindAccepted`; the panel, its rows and the report; overnight A12). Every saved figure has a basis
and is labelled an estimate; costs tie to the ledger to the cent; unknown costs stay
unknown. `set_baseline` is **user-only** (Settings → Usage → **Value this month**); agents
read with `value.summary` / `value.get_baselines` and draft the report with
`value.monthly_report` (write: `outputs/value-report-YYYY-MM.md` in a named
workspace, a draft Result from the ledger alone, never overwritten, never
sent). Scheduling it for the 1st is an automation (F1), not built in.

### Usage budgets (Paperclip PR 8a)

One global `budgets.json` (scope `global` | `agent:<id>` | `workflow:<slug>`
| `heartbeat`, metric tokens or `costUsd`, window day/week/month). Workflows,
the heartbeat and automation delegates are **stopped** at a hard limit
(`budgets::preflight`). Ordinary chat, ingest and one-shots only warn. A tripped budget opens one pause episode with a
`budget_pause` card in Waiting on you. An unreadable `budgets.json` holds every
enforced run closed.

### Mid-run enforcement (Paperclip PR 8b)

Claude runs watched by a token budget are stopped mid-run by the
`BudgetGuard` (the whole process tree for GUI-owned runs); dollars are
bounded by Claude's `--max-budget-usd` backstop, which can overshoot by one
message. Concurrent runs share **one reservation** (`budgets::reserve`), so a
ceiling is honest across runs claimed together (decision 6). Dollars are held
at the claim: an enforced Claude claim holds its backstop (`SpendGrant`),
which is the headroom other live holds have not taken, with a $0.01 floor.
Ordinary chats, one-shots and ingest take no spending hold, and spending holds
are process-local. This does not control external clients or ordinary chat.
Streamed/checkpoint usage is a lower bound. A local cost cap is an API-equivalent policy, not a bill or proof of
remaining subscription quota. **Ceiling
fallback** (`budgets::preflight_with_fallback`): a workflow blocked by its
own agent's budget runs on another agent with room, recorded on the receipt
and shown in the run log; no candidate leaves the block with a note. A budget
stop is `budget_stopped`, never a failure for the streak.

### Saved turn feedback (Paperclip PR 17)

Useful / Not useful votes are saved per turn by exact `turns.run_id` in the
`turn_feedback` table with a redacted snapshot built from the stores. The
Votes list was removed (overnight 11-08 S5) and stays gone. Since overnight
11-12 (C2) the thumbs in a reply's row write a vote again
(`record_turn_feedback`; Not useful takes an optional one-line reason), so the
friction loop and the run ledger read new votes. No screen lists them.

### Result kinds and effort (Paperclip PR 14a)

Every chat turn and workflow run is classified into a **result kind**
(`result_kind.rs`: refusal, turn limit, usage limit, transient, sign-in,
resume failed, actions denied, stalled, budget stop, cancelled, timed out,
error) from structured envelope fields, never assistant prose. A refusal and
Antigravity's headless write denials end as errors and are never retried.
**Effort** (`effort.rs`) works for every agent and model, mapped to each CLI's
own flag at the nearest level the model takes.

### Resume hardening (Paperclip PR 14b)

A chat turn whose native session is unknown, missing or poisoned (and ran no
content or tool event) is retried **once**, fresh, inside the same lease and
run, with the conversation bundle and the task-state record rebuilt by the
context compiler (`context_compiler.rs`), so finished steps are marked done.
Each
stint stores a configuration fingerprint; a changed configuration starts a
fresh session (`sessionReset`). Fingerprint v3 (OB04) hashes what selects the
appended prompt and `chat_resume::PROMPT_CONTRACT`, not the prompt's bytes, so
an install that only edits guidance resets nothing; bump the contract to reset
on purpose. A stint stamped v1 or v2 resumes and is restamped v3 when it is
this agent's, the last turn that ran on its native session had the same mode
class and requested model, and `sessions.cwd` is the turn's folder (Grok
excepted; a missing value resets once; see [AGENT_CONTEXT](../AGENT_CONTEXT.md)).
Each stint also stores the fingerprint's per-part hashes (`stints.config_parts`,
OB03), so a config-changed reset names the parts that changed (`sessionReset`'s
`changed`, the manifest's `compiler.resetParts`: `contract`, `agent`, `model`,
`mode`, `cwd`, `mcp`, `caps`, `canvases`, `workspace`, `skills`), or `unknown`
for a stint stamped before them. A manual **Continue** follows a stopped or
interrupted turn. **Resume after restart** (`restart_resume.rs`, overnight
2026-10-10 A1): the chat rows the startup recovery ended mid-run become one
Waiting on you item, "Resume N interrupted chats" (kind `interrupted`, group
"Interrupted by a restart"); its checklist (P1-40, `restartResume.ts`
`resumeChecklist`) ticks each with its workspace, agent, model, permission
mode and Ultracode, Cancel focused, and **Resume ticked** sends each ticked
one (unticked ones stay offered; the footer names every skip and its reason)
exactly the Continue text as the composer would: its native session,
agent, the run's requested model, folder and last mode, plus the chat's own
Canvases choice, its own Ultracode switch (each conversation's since
P1-10, kept under its session key) and its own effort (`hollow.effort.<key>`,
checked against `effort::LEVELS` and applied ahead of a fixed level and
Automatic, as a composer send is; an unknown level refuses that chat), which the webview passes per chat
(`list_interrupted_chats` and `resume_interrupted_chats`, both user-only: no
agent action lists or resumes them). Never automatic: a Main agent check-in
skips the item, doesn't report an offered chat as a failed run, and its
`sessions.send` refuses one (S5). Offered across launches from recent recovery rows, the person's turns
(never unattended ones, nor an agent-started chat at depth ≥ 1: the queue row
keeps `{"unattended":true}` or `{"appDepth":n}` through its terminal write for
this, and an automation's chat turn also its `auto-` `clientTurnId`; nor one in Hollow's scratch folders, `mcp_attach::bare_scratch`; S5; the
card lists the agent-started and unattended ones apart with Open only,
`restart_resume::listed` with `agentStarted`, and `resume` refuses one in
words, P1-40), the
conversation's last turn, seen working within
24 h, not archived, with a native session and an existing folder; a turn that
never started goes back to the composer instead. A second restart preserves an
unanswered offer while those eligibility checks still hold. The same loop handles R1 account failover.
Every turn's message carries a `<hollow-clock>` line (`chat::clock_line`,
overnight A2: the local time the turn started, with its UTC offset; after the
workspace envelope and recall, a `clock` budget ask and manifest layer, and
stripped from discovered-session previews); it is in the message, never the
appended prompt, which Claude caches. The appended
prompt's fixed tail is the next-message guidance, `chat::BACKGROUND_GUIDANCE`
(overnight A3: background work ends with the turn; ScheduleWakeup/CronCreate
never resume a Hollow chat, `sessions.wake_me` can, where offered; S5) and the
scratch guidance. A receipt says when a
run ended mid-work (`evidence::ended_mid_work`, chats and workflows): a
`background` failure row when the CLI reported a background task `stopped`
after its `result` and the attempt exited on its own (Claude Code's
`task_notification`; `evidence::CutTasks`), or, for a CLI that reports none,
when it used `run_in_background` and processes (not zombies) were still alive
0.5 s after it exited on its own (`process_group::Driven::left_running`,
`Timings::linger`, agent runs only; Hollow ends them); and a `wakeup` row for
an answered, successful `ScheduleWakeup` / `CronCreate`. A check started with `run_in_background` scores `unknown`,
never `passed`. Failure rows don't change the receipt's headline state. Every turn's
appended prompt also asks for a suggested next message (`next_step.rs`, whose
`strip` also drops an ask-canvas's `hollow-canvas-answer` block from vault
session records, the learning job's prompt and `recent_asks`).

### Per-run scratch space (Paperclip PR 15a)

Each run gets its own scratch folder (`scratch.rs`): the child's
`TMPDIR`/`TEMP`/`TMP` and `$HOLLOW_SCRATCH_DIR`, so downloads and throwaway
files stay out of the vault and the run's diff. Inside a Hollow turn your own
`TMPDIR` is such a folder (`/private/tmp/hollow-run-…`). A chat turn's
folder is named for its conversation; when the turn ends or is stopped it is
set aside as `.kept-<pid>-<nanos>-<folder>` beside it for six hours (at most
three per conversation, newest kept; one holding only its marker goes at
once), so a Stop no longer deletes uncommitted work there (PER-9,
2026-11-14). An automation's folder (named for its run) is still deleted.

### Silent-run indicator and watchdog (Paperclip PR 15b)

Each run has one quiet clock (`run_activity.rs`): the UI shows "quiet Nm"
and a progress phase. Every agent has a **no-output watchdog** (default 30
minutes, 1-1440 per agent in Settings → Models, Off = `null`) and an
optional per-run **deadline** (`agent_settings::run_limits`). A stall ends the
run as `Ended::Stalled` and a deadline as `Ended::TimedOut`, both through the
process-tree settle. A pending Hollow card is not silence.

### Phase timings (Paperclip PR 15c)

Every chat, workflow and heartbeat receipt carries `phaseTimings`
(milliseconds; an unmeasured phase is omitted, never 0): `queueWaitMs`,
`baselineSnapshotMs`, `spawnMs` (preparation; the context manifest's
`preparation.timeline` splits it into every step, below), `firstOutputMs` (any line, stderr
included), `firstStdoutMs` and `firstTextMs` (the first stdout line and the
first assistant text, F11: stderr is not first output), `firstModelMs` (OB08a:
the first model event, i.e. the first thinking, tool call or text the stream
carried; it separates the model's start from the agent work before its first
text), `turnMs`, `repoCaptureMs`, `finalizeDiffMs`, `evidenceMs`, and one
count, `toolsBeforeText` (the tool calls before the first text, a subagent's
included; a run with no text has its count so far, a run with no model event
none). One provider-neutral marker (`RunActivity::note_event`) reads the
parsed events: thinking is `AgentEvent::Thinking` (Claude and Grok `thinking`
blocks, Codex `reasoning` items). Three readings to keep in mind:
- A Claude or Grok tool call is stamped when its `assistant` message arrives,
  after its input has streamed, not when the call starts.
- Text means any text, a subagent's included: a subagent's first text sets
  `firstTextMs` and ends the `toolsBeforeText` count.
- Codex's other items (`todo_list`, `web_search`, `file_change`, …) are shown
  as one-shot tool rows and count as tool calls; its non-fatal `error` item
  does not, and is no model event.

Antigravity's stream has neither thinking nor tool calls, so a first model
event would only repeat its first text: it has no `firstModelMs` and no
`toolsBeforeText`. The heartbeat's tool pass gets the same figures from its
runner's clock (`Collected::activity`), read once `run_collect` has returned,
so its `turnMs` (spawn → then) also covers the runner's usage-ledger write.
Its Codex snapshot pass (the tool-free transform) has no clock, and its
`turnMs` is the whole runner call. They are mirrored into the queue row and
listed in the expanded receipt.
