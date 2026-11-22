# Usage instruments

Settings → Usage opens on the period controls and visual overview. Claude plan, Budgets,
Value and Routing remain below the existing totals and breakdowns. All editing,
resume and enforcement behavior is unchanged. The overview is lazy and does not poll.

`usage_summary` adds `daily: UsageDay[]`, `undatedRuns` and `nightAttributionError`.
Each day has `date` (local YYYY-MM-DD), `total`, `byAgent` and `nightByAgent` buckets.
The grouping uses **recording/checkpoint `at`**, exactly like the summary's period filter,
not start time. A running row can move days as its checkpoint changes. No schema migration
or new ledger writer. Invalid dates remain in totals and get an explicit chart omission count.
Absent days mean no rows recorded; they are not fabricated zero days.

Reported and estimated costs remain separate nullable fields and separate chart selections.
The stacked step area shows known positive amounts only, never a bill or complete coverage.
Incomplete/unknown/unpriced/running/not-started counts stay visible. Native buttons inspect
recording days; all daily and agent values have a keyboard-reachable table equivalent.
The agent amounts include all summary rows, including any undated rows disclosed below the chart.

**Partial runs carry a priced floor (OB07, 2026-10-17).** A run that streamed tokens but has no cost (killed,
interrupted after a crash, an unverified resume, an `ask_agent` timeout) keeps `cost_usd` NULL and `partial`, and
now records `usage_ledger.cost_floor_usd`: the price table (`usage_prices::floor_usd`) applied to the tokens it
streamed, by model, at the input, cache-read and output rates (input at the input rate, never the 2× cache-write
rate, so it errs low). Messages without a model id are priced at the row's model. Checkpoints write it too, so a
crashed run's `interrupted` row keeps it. A model with no price row adds nothing: with the built-in rows Codex has
no floor (a row someone adds in `usage-prices.json` prices a Codex model only when its id is that model's exact id, or
the id plus a date), and its
tokens are counted as before. The floor is **never** added to a known total (`reported`/`estimated` buckets) or a
budget (`budgets::LedgerRow` doesn't read it); `usage_summary` reports it apart as `partialFloorUsd` (null, never 0,
when no run has one) and `partialRuns`, for "≥ $X more in N incomplete runs". Finished and `interrupted` rows
count; a live `running` checkpoint is in progress, not incomplete, so it doesn't. A row whose cost is known (a
turn guard's estimate included) has none. Rows from before the column read NULL.

**A resume after a cancelled turn is priced as a span (OB24, 2026-10-17).** On `--resume` Claude reports the
session's cumulative cost and tokens, so a resumed turn subtracts the newest earlier row of the same native session
that stored a terminal (`session_store::previous_terminal`; a cancelled or killed turn stores `terminal: null`, an
interrupted one no `usage_json`, and both are skipped now, where before they were picked and left no baseline).
`session_delta` is still the exact path: accepted only when the token delta equals this turn's streamed sums. When
it isn't, but the baseline is an earlier state of the same report (no model missing, no field lower) and the cost
rose, the row is basis `session_delta_span`: `cost_usd` is the cost rise, `cost_status` `reported`/`estimated`
like any priced row, tokens are this turn's main-model usage (as `resume_unverified` records them), and
`usage_json` adds `spanRuns` (the run keys of the skipped rows, oldest first, then this row's: "a span over N
runs"; at most 64, the newest, with `spanRunsOmitted` counting the rest) and `unstreamedTokens` (tokens the
session reported but this turn did not stream: the delta minus this turn's streamed sums, never below 0; the
skipped turns' tokens are in it, and also on their own rows, and so are side-model tokens and start-of-message
snapshot gaps). The skipped rows stay `partial` with a NULL cost; `usage_summary` adds the span to known cost once
and leaves the skipped rows' OB07 floors out of `partialFloorUsd`/`partialRuns` (`spanned_runs`), so no dollar
counts twice in known cost; rows past the 64 keep their floors beside the span, so `partialFloorUsd` can read high
there. A skipped row with a positive `cost_usd` (a night-guard estimate) blocks the span; a `0` estimate doesn't. Over the 4 KB `usage_json` cap the fallback
keeps `basis`, `truncated`, the span's fields and `terminal`, so the next resume still subtracts this row rather
than passing over it and spanning its cost again (still too large: `spanRuns` goes, all counted as omitted, then
the terminal). No baseline, a cost that didn't rise, a baseline with a lower field, or a skipped row that already
has a `cost_usd` (a turn guard's estimate, or a priced row that lost its terminal to the cap: the rise would count
it again) keeps `resume_unverified` (no cost, a floor). A terminal that fails to deserialize is no baseline; no
older row is tried. The chat chip and receipt show the span's cost on the resumed turn.

A span **moves the spend onto the resuming row**: the cancelled turns' cost is charged to that row's `run_id`,
day and surface, so run rollups (`descendants::rollup`), the daily bars and the day/night windows charge it to the
later run, and a span that crosses midnight lands on the second day. A budget still counts a spanned row among its
`uncosted_partial_runs` (it has no cost of its own), so `UsageTotals::missing` reports a run with unknown cost
that the span already covers: the safe direction. One overlap remains, on the high side: the efficiency scorecard
(`scripts/agent-eval/scorecard.mjs`) still sums every non-known row's floor into `partialFloorUsd`, spanned rows
included. The scorecard doesn't enumerate bases. Non-chat resumed runs, which stream only start-of-message
snapshots and so rarely pass the exact check, now mostly become spans with no skipped rows: far more rows leave
`resume_unverified` than the cancelled-turn case alone.

The night band is deliberately **Night queue · direct runs**. A bounded query joins selected
usage rows to `run_queue.history_run_id` and exact `night_queue.queue_id` or
`runbook_steps.queue_id` links. UNION deduplicates references; only `surface=chat` counts,
since delegates can carry the parent's run id. Counts are ledger invocations, including
separate failover attempts, rather than distinct turns. Queue retention limits this attribution.
No inference from time, prompt, label, workspace or provider. The band excludes descendants
and manually started lanes and explicitly is not the overnight budget total. A failed join
is unavailable, never zero or an assertion that no overnight work occurred.

`src/usageInstruments.ts` supplies pure stacked geometry and reuses V1's `gaugeValue`.
`src/components/UsageInstruments.tsx` and `BudgetGauge.tsx` render native SVG, not a canvas
iframe or the kit runtime. Unknown/error gauges have no value arc; percentages above the
limit stay in text while the arc clamps. Budget words and controls remain authoritative. Paused/over chips use ink text with an error
border, fixing their former 4.16:1 dark-theme text contrast.
`src/styles/usage-instruments.css` uses theme tokens and transform-only entrances, the existing
shared visibility observer, both reduced-motion gates and static fallback without an observer.
No frame loop, background poll or new dependency. Native WKWebView/VoiceOver are separate
from the mock browser checks.

**Calls and context per run (OB05, 2026-10-17).** Cost scales with calls × context, so each ledger row's
`usage_json` now says both (`usage::CallStats`, the same keys on the chat and workflow receipts as `apiUsage`):
`apiCalls` (distinct main-thread assistant message ids), `peakContextTokens` (the largest fresh + cache-write +
cache-read prompt one main-thread message sent), `cacheWriteTokens` (cache creation over every streamed message,
subagents' too; the row's `input_tokens` still folds it in; absent, not 0, when no message wrote the cache),
`numTurns` (the results' `num_turns`, summed when a row holds more than one result: it is per invocation, also on
`--resume`) and `apiMs` (their `duration_api_ms`). `apiMs` is absent on a resumed Claude turn, because
`duration_api_ms` is session-cumulative there like `total_cost_usd` (the recorded resume reports 6,990 ms against a
2,345 ms wall clock); a Claude row that started its session fresh keeps its last result's, which already covers an
earlier attempt on the row. Grok's result goes through the same parser, but its `duration_api_ms` is per invocation
on `--resume` (`fixtures/grok/resume.jsonl`: 2,164 ms after the one-shot's 3,526), so a Grok row keeps it, summed
over its results. After a limit failover or a workflow restart, the receipt's `apiUsage` covers the last attempt
only (each attempt has its own usage row; a ceiling move keeps one row, so its message figures cover both attempts;
its first attempt ends before any result, so `numTurns` and `apiMs` are the second's).
A Claude message carrying `parent_tool_use_id`
is a subagent's: it goes to `subagents { calls, tokens }` (distinct ids; input + cache writes + cache reads + output)
and never counts toward `apiCalls`, the peak or the chip's `contextTokens`, though the row's totals still include it
(they must match `modelUsage`). Only per-message streams (Claude, Grok) give the message figures, and only a
Claude-format `result` (Claude's, and Grok's, which goes through the same parser) the other two; a key nobody supplied is absent, never 0, so Codex and Antigravity rows carry none. Side-model
calls the CLI doesn't stream (its background haiku) are not counted.
The scorecard reports `peakContextTokens`, `apiCalls` and `subagents` per group (every row of its own) and over the
window's finished top-level runs (no parent surface, no `oneshot`/`ingest`/`delegate` helper call, not `running`; OF28,
its `ledger` blocks), null rather than 0 where no row carries them; the summary prints them "over top-level runs",
"not installed" when no run carries them. OB07's `cost_floor_usd` prints "none in this window (0 of N runs)" when
the store has the column but no row in the window carries a floor, "not installed" only without the column. A
group's `toolsBeforeText` (OB08a), like the latency fields, leaves out long runs.

**Workflow-tool agents (`workflowAgents`, overnight 9 S31, 2026-10-19).** Agents the Workflow tool launches write
their own transcripts (`<config>/projects/<slug of the run's cwd>/<session>/subagents/workflows/<wf>/*.jsonl`), and
their usage is often missing from the result's `modelUsage`, so a row's tokens can't hold them. When a Claude run
finishes, `usage_ledger::workflow_agent_usage` sums the usage blocks of those files' messages stamped inside the run's
window (its `started_at` to the finish; one figure per message id, snapshots max-merged) into `workflowAgents
{ count, calls, tokens, inputTokens, cachedInputTokens, outputTokens, cacheWriteTokens, estimateUsd, unmeteredTokens,
basis: "estimated", truncated? }`, in `usage_json` (kept through the size cut) and on the receipt's `apiUsage`. The
config folder is the command's `CLAUDE_CONFIG_DIR`, else Hollow's, else `~/.claude`. `estimateUsd` is the price table's
figure by model (`usage_prices::list_usd`, with cache writes at the write rate rather than the input rate) and
`unmeteredTokens` is the workflow messages the run's stream didn't also carry (by message id), plus what it streamed,
less the row's tokens, when positive. Neither ever reaches the row's tokens, cost or floor. Only the timestamp and
the message's id, model and usage are deserialized; content is skipped unread. Files last written before the run
began are skipped; the scan stops at 400 files or 256 MiB (`truncated`: a floor) and runs under `block_in_place` on a
multi-thread runtime, since `finish` runs on an async worker. Absent for other agents, a run with no session, and a
run with no workflow message in its window. Not followed: Claude Code's shortening of a project folder name over 200
characters.

**The usage ring's Context section (2026-11-12).** One Last turn heading groups
input, output and cost. Cache reads moved under Cache, where the explanation
distinguishes repeated model-call reads from window occupancy. What's using it
shows one split of the latest manifest's recorded text: Conversation (request,
written continuity, task state and attachments), Project notes (workspace,
retrieval and recall), Your profile (profile and updates), Instructions (other
recorded layers). Shares use UTF-8 sizes with largest-remainder rounding.
This is labelled Latest message, not the retained context window: provider
history, hidden system prompts, tool schemas and post-compaction retention
cannot be reconstructed from the available data. Missing manifests show
Breakdown unavailable. No file names, byte totals or routine yellow states
appear in the split.

The ring reads on opening and refreshes on a usage event; old conversation
responses cannot populate a new one. Context details opens Settings, Advanced,
Search & indexes. Conversation context details there retains tool-result
estimates and largest results from the session, and the raw recorded manifest.
The agent-facing context APIs and all prompt data remain unchanged. The former
folder-and-plug panel, explanation helpers and styles are removed. Folder and
connectors is now a two-line resource menu with passive connector status and
a Settings link. Permissions remain in the access chip; project notes are in
the side panel's Files picker; profile, skills and memory keep their primary homes.

Continue in a new conversation uses the unchanged handoff rule: 75% of a known
window or 350K on a 1M window, and an available branch action.

**The agent's own gauge (OB11, 2026-10-17).** `sessions.context` (read tier, no Settings control or UI) tells a
running chat turn how full its own window is, so a handover can follow a number instead of a feeling (lanes handed
over at a median 615K). `chat::drive_turn_attempts` registers the turn by history run id in `context_gauge.rs`'s
registry and publishes a reading from its ledger's accumulator after every `MessageUsage`; the guard
removes it on every return. The action reads the caller's `run_id` (the broker's verified caller), so an agent only
ever sees its own turn: `contextTokens` (`UsageAccumulator::context_tokens`, main thread only), `contextWindow`
(inferred like the composer's ring: Claude 1M for a `[1m]` model or once past 200K, else 200K; other agents null),
`advisoryTokens` 350000 on a 1M window only (else null), `pastAdvisory` and `calls` (OB05's `CallStats` so far).
`calls` is the current attempt's: a fresh retry or a limit failover starts a new ledger row, so its counts start
over (a ceiling move keeps the row); `numTurns` and `apiMs` come from a result, so they appear only once one has
arrived (a result followed by more messages), and a resumed Claude attempt's `apiMs` is dropped, as on the ledger row. The figure is per run, not per thread: a Claude Code subagent that calls it shares the turn's
`hollow_app` server and run id, so it sees its parent's main-thread context, not its own.
A caller with no live turn (an automation, a finished turn) gets `live: false` and a note, not an error. Ask
Hollow doesn't offer it (`ask_hollow::CHAT_ONLY`); `sessions.handover`'s description names it.

**Efficiency scorecard (evening 2026-10-16).** `scripts/agent-eval/scorecard.mjs` reads the same ledger and follows its
buckets (`usage_ledger.rs`): only `reported` and `estimated` rows count as known cost, and a window with any other
row has no total, only a floor. A `partial` row's cost (the budget guard's estimate at a kill) is reported apart as `partialCostUsd`.
Jev requests (agent NULL, scope and model `jev`) are their own `oneshot/jev` group and count as `unpricedByDesign`, not
`unknownRuns`; their group has no total either (unpriced is not $0). TypeSafe reports no tokens, so since S28
(2026-10-17) each Jev row's `usage_json` carries the request's volume instead (`jev::request_volume` and
`jev::response_volume`): `use` (`connectors`, `passages`, `proposals` or `other`), `items` (questions asked),
`requestBytes`, `responseBytes` and `latencyMs`; numbers and that label only, never request text. The request side is
set before the request is sent, so an `abandoned` row (a call the preparation deadline dropped) still has `use`,
`items` and `requestBytes`, with no response side. The scorecard doesn't read them yet.
It adds what the instruments don't show: latency percentiles, resets per 100 turns and their causes, injected bytes
per layer and a transcript reconciliation, and (OF02) a tool pass: ToolSearch per 100 turns, always-loaded tool use,
first-call cache by new/reset/resume, waits that bought nothing, Bash refusals by permission mode and handover numbers;
(OF19) the context anatomy: cache writes by cause, what each main-thread call carries by kind, poll-only calls,
Python edit scripts, result sizes and Workflow-tool agents (an estimate in $, since the ledger doesn't meter them);
and (OF20) tool errors per 100 calls by agent and tool, expected non-zero exits and masked failures. Its weights
count a 1-hour cache write at 2 and a 5-minute one at 1.25, which the morning report shares (its `subagentShare` moved too).
See [Build & ship](reference/build-and-ship.md).
