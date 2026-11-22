# Jev in Hollow

Jev (TypeSafe's `jev-1.13.0`, "System One") answers typed questions (scores
and choices) quickly and cheaply. Hollow uses it for **decisions**, never for
writing: the selected agent still plans, writes and codes, plain code still
enforces limits, and Jev only answers "which one" or "how relevant".

**Jev is optional.** Every use has its own switch in Settings → Context
prioritizer, all are off by default, and each switch needs the key (added in
Settings → Accounts → Other providers, kept in Hollow's shared Keychain item, `secret_store.rs` key `jev`).
With no key, a switch off, the Keychain unreadable, a timeout, an HTTP error or
an invalid answer, Hollow does exactly what it does without Jev for that call.
No feature waits on Jev to work.

Concurrent identical ranking bodies share an in-flight request and then use
the exact score cache. Each waiter keeps its own preparation deadline; a
cancelled owner releases its lock. The lookup registry holds at most 64 weak
handles, never request text, credentials or authorization. Under overflow a
forgotten handle may cause a duplicate call. Final callers still read their
current candidates and permissions; the cache supplies scores only.
Settings reset starts a new flight generation. Old callers cannot block new
credentials or put scores or breaker failures back into the reset cache.
Send preparation also supplies a cancellation predicate through flight waits
and transport. Pure cached scores carry an in-memory original decision time;
admission can retain conversation reuse without extending its 30-minute TTL.

## Where Jev decides

| Switch | Module | Question | Without Jev |
|---|---|---|---|
| Jev relevance model | `context_rank.rs` | Score 0-3: is this passage useful evidence, is this connector needed? | Local word-overlap passage order and connector choice |
| Jev duplicate check | `jev_decide.rs` | Choice: does this suggestion repeat one of these existing records? | Exact fingerprint and title-slug dedup only |

(A third use, whiteboard layout, went with whiteboards on 2026-10-07; the
2026-10-07 study measured no difference for it. A saved `whiteboard_enabled`
is ignored when read and dropped on the next save.)

Both share `jev::evaluate_with` (4 s bound, no redirects or retries,
64 KiB response cap, provider bodies never surfaced), the pinned model and
`jev::endpoint()`: TypeSafe's URL, or in a candidate build a loopback
fake (`HOLLOW_CANDIDATE_JEV_ENDPOINT`, plain http to `127.0.0.1` or `[::1]`
only, never a host name such as `localhost`; the everyday app ignores it).

**A candidate with the loopback fake needs no key** (`jev::fake_jev`, 2026-10-07; only the retired test bed set the override): with that
override, Jev counts as configured without a key (the fake ignores auth; a
request carries a placeholder bearer), and every switch and the key live in
the process's memory only, never in the candidate's Keychain item, so they
reset on restart. A stored item would raise a login-password prompt on every
self-signed rebuild and hang the candidate's chat turns. The everyday app and
a candidate without the override still need a saved key, as above
(`jev::tests::a_candidate_on_the_loopback_fake_needs_no_key_and_no_keychain`).
`test_jev_key` and `hollow jev-study --typesafe` still read a real saved key.

**Passage reranking sees the ranked pool** (agent effectiveness F3,
2026-10-06): the retrieval broker (`retrieval_broker.rs`) ranks every
candidate first and hands the reranker its top 24 (`jev::MAX_CANDIDATES`), in
one request; the 8-passage evidence budget is applied after Jev's answer, so a
passage ranked ninth locally can still be kept. Before, Jev only ever saw the 8
already kept. With Jev off or failing, the broker's order stands and the
budget takes its first 8.

## Connector cardinality contract

`context_rank::select_connectors` (agent effectiveness F5, 2026-10-06) gives
exactly one decision per "when relevant" connector candidate, in the
candidates' order, whatever Jev does. Each `ConnectorDecision` has the
connector `id`, `attach`, a `basis`, Jev's `score` when it decided, and a
`reason` when Jev's answer wasn't used. `connectors::attach` returns them with
the attachments (`Attached.selection`) and matches them by id, never by
position.

| `basis` | When | Decided by |
|---|---|---|
| `named` | The request contains the connector's id or name | Attached before Jev; never sent |
| `jev` | Jev returned a valid score for it | Attached at ≥ 2 (`TOOL_ATTACH`) |
| `local` | Jev off (no key, switch off, Keychain unreadable) | Local relevance |
| `local-fallback` | Sent, and the request failed: timeout, HTTP error, oversized or unparseable body, no `answers` object; or (F8, O2) cut by the preparation deadline, not sent because it had passed, or not sent because Jev is paused | Local relevance, reason = the error |
| `unscored` | Jev answered, but not validly for this one (missing, wrong id, not a number, out of 0–3) | Local relevance |
| `beyond-limit` | Not among the 24 one request carries (whether or not that request succeeded) | Local relevance |

Jev scores at most `MAX_CANDIDATES` (24) items per request, and
`jev::score_with` / `score_each_with` return an error for more instead of
scoring the first 24. With more than 24 open candidates, Hollow preselects
locally: the locally relevant ones go first, then library order, and only
those 24 are sent. That keeps the library limit (50) at one bounded request,
so the worst case is one call, cut at the preparation deadline (below). Batching (three calls)
was rejected because it would triple the cost and add network for a case
that local relevance already answers. Answers for ids that weren't asked are
ignored, so an over-long answer can't shift scores between candidates.

## What each run records (agent effectiveness F11)

Each chat turn and workflow run records its relevance decisions on its own
context manifest (`run_manifest.rs`, stored with the run's evidence receipt;
IPC `run_context_manifest`), never in process memory: the connector
decisions above, each retrieved passage's local score, Jev score, `kept`,
basis and reason, and one `jev` entry per use (`passages`, `connectors`) with
its `outcome` (`ok`, `timeout`, `malformed`, `error`, or `off` when nothing
was sent; since F8's O2 also `cached`, `deadline` and `circuit-open`, below),
the items sent, the latency and `costUsd: null` (TypeSafe reports
no per-call cost; unknown is never written as 0). Only hashes and metadata
are kept, never the request or passage text Jev was sent. A reqwest timeout
now reads as "Jev timed out." like the outer 4 s bound, so `timeout` is
deterministic. Settings → Context's "Last request" line reads the newest
run's manifest and names the run.

The relevance model's query is the run's request (`turn_request.rs`, agent
effectiveness F2), first 2,000 characters: a chat's own words or a workflow's
prompt after variable substitution. The names of what "that project" and the
like resolved to inform the local rules only (`context_rank::ConnectorQuery`:
Jev is sent `request`, the local rules read `local`); they are text the person
didn't type this turn, and Jev is never sent new content (wave 1 follow-up). Until 2026-10-06 a workflow's connector query was its
prompt expanded with workspace context, recall and the operator profile, so
Jev saw the profile first and the request could fall past the cut.

## Preparation's two calls: one deadline, reuse and a breaker (agent effectiveness F8)

O1 measured a chat turn waiting for two Jev timeouts one after the other: 4 s
for the passage rerank, then 4 s for connector selection, 8.0 s before the
agent started. Since O2 (2026-10-07):

- **At once, under one deadline** (`prep_deadline.rs`). Retrieval and
  connector selection don't read each other, so a chat turn and a workflow run
  prepare them concurrently, under one `PrepDeadline` (`BUDGET`, 3 s from the
  start of that stage). Each Jev call is given up when it passes
  (`prep_deadline::JEV_MISSED`, outcome `deadline`, every candidate it carried
  `local-fallback` with that reason); a call is not sent at all once it has
  passed (`deadline`, 0 items). The deadline is below the transport's own 4 s
  bound, so a stalled Jev is always cut by it, attributably. A named connector
  is decided before Jev and is never affected. The manifest's
  `preparation.deadline` `{budgetMs, cut}` lists what was cut.
- **Reuse** (`jev::Guard`). A ranking is a pure function of the request Jev is
  sent, so an identical request (key: SHA-256 of the endpoint and the request
  body, i.e. model, query, candidates and question) within 120 s gets the
  earlier scores without a call (`cached`; `items` are the candidates it
  covered). At most 32 are kept, in this process's memory, scores only: never
  the text, a key, a permission or an attach decision, which are recomputed
  from the scores on every run. A failed answer is never kept. A changed
  candidate set, query or question is a new key.
- **A conversation's last connector decision** (S28,
  `context_rank::SessionCache`). Consecutive chat turns decided the same
  connectors in 89% of pairs, so a chat turn reuses its conversation's last Jev
  relevance decision for up to 30 minutes from when Jev made it when the
  inventory is unchanged, the decision covers every candidate, the request
  names no connector and shares no word with one left out (basis `cached`, no
  call). A ranking this guard reused (`cached`) is never remembered, so the 30
  minutes can't restart from a reuse. Keyed by vault path and session key. Ids,
  attach and score only; approvals are re-read every turn. In memory, at most
  256 conversations.
- **A breaker** (the same guard). Three failed calls in a row (timeout,
  deadline, error or malformed) pause Jev for 60 s: ranking uses local
  decisions and records `circuit-open` (nothing sent, 0 items) with the reason
  as a fallback. The first call after the cool-down is sent; an answer closes
  the breaker, a failure opens it again.
- Saving or removing Jev's settings resets the guard (no ranking or failure
  carries over to a new key, endpoint or switch). The guard covers only these
  two calls: `hollow jev-study` sends every judged item (`score_with`,
  `score_each_with`), and the duplicate check is as before.
- **One client was measured and left out.** Building the request's client
  costs 33 µs (debug build) and a loopback call 1–2 ms either way; the only
  other gain, a kept-alive TLS connection to TypeSafe, can't be measured
  without a paid call, so each request still builds its own (wave 3's rule:
  a change that moves no measured number is left out).

Numbers (the `prep_bench.rs` bench, timeout mode): preparation p50/p95
8,294/8,479 ms before, 3,237/3,273 ms after; with one guard across turns, the
third timed-out turn onwards skips Jev (p50 272 ms). See
[AGENT_EFFECTIVENESS.md](history/AGENT_EFFECTIVENESS.md#o2--concurrent-bounded-preparation-with-local-fallback).

## Duplicate check (`jev_decide.rs`)

Runs after the local dedup rules, on suggestions only:

- **Heartbeat task proposals** (`heartbeat::run_beat`, via `jev_filter_tasks`)
  are compared with open vault tasks and pending task suggestions
  (`heartbeat::existing_task_records`; finished tasks are left out, because
  repeating finished work is often real).
- **Learning proposals** (`vault_learning::propose_from_exchange`, after a chat
  turn) are compared with approved learnings and pending learning proposals.

For each proposal (at most 10 per request) Hollow builds a local shortlist of
at most 6 existing records (most shared words, then list order; at most 24
distinct records per request) and asks one Choice question with an option per
shortlisted record plus `none`. The best match's probability decides:

- ≥ 0.85 (`DROP_AT`): the proposal is not queued. Heartbeat logs the count as
  `jevDedupedTasks` in `.heartbeat-log.jsonl`. **Except a changed value**
  (2026-10-07): when the matched record has values (numbers, amounts, dates,
  times, versions) and the proposal brings one it doesn't have, even beside
  the old one ("rose from 48,500 to 55,000"), it is flagged as below instead
  (`changes_a_value`), so an updated value reaches review. A value is had
  when it is the start or the end of one of the record's values ("9 October"
  and `10/09` match `2026-10-09`, `48500` matches `48,500`; "10 October" and
  `2.2` vs `2.4` don't). Leaving a value out, or adding one to a record with
  none, still drops. The Jev study found the case: "The Kestrel budget ceiling
  rose to 55,000 EUR." judged the same fact as "…is 48,500 EUR." Still dropped:
  words-only changes ("Lee" → "Dana"); a new value that equals another of
  the record's values ("lead time is now 2 days" against "type 2 … lead time
  8 days"); a one-part value equal to a version's last part ("firmware 4"
  against `2.4`); digits outside ASCII. Flagged though the same (an extra
  review card): a full date in another order (`10/9/2026` against
  `2026-10-09`), an extra trailing part (`48,500.00`, `2.4.0`), 12-hour
  against 24-hour time, and a reworded duplicate with an extra value. A
  decimal comma reads as a thousands separator (`2,5` → 25).
- ≥ 0.5 (`FLAG_AT`): queued with `possibleDuplicate: "<title>"` in its review
  payload; the card shows "Possibly the same as “…” (Jev duplicate check)."
  This is the human-review branch: the user decides.
- Otherwise queued as before.

Sent: the suggestion titles and short details (≤ 200 / 300 chars) and those of
the shortlisted records. Never paths, note bodies or conversations. A wrong
answer can only hide or annotate a suggestion; it never creates or edits a
record. `possibleDuplicate` is written only by Hollow (anything an agent puts
there is cleared first) and is ignored by validation, fingerprints and
approval. Since agent effectiveness O3 (F11, 2026-10-07) each check names its
`Job` (`heartbeat` or `learning`) and run, and its report (`job`, `runId`, and
its `request`: outcome, items, latency, `costUsd: null`, or `null` when
nothing was compared) is kept per run, the newest 64 in process memory, so a
concurrent check no longer overwrites it. `jev_decisions_report({runId?})`
answers that run's newest or the newest overall, shown under the switch. A
beat's check is also in the beat's run manifest (`dedup`, and its request
among `jev`); a chat turn's learning check runs after its receipt, so it is
in that run's manifest view as `afterRun` (memory only). The view's
`jevCalls` reconciles every Jev call a run made.

Not covered: `hollow_tasks` `propose_tasks`. Its writes run inside
`automated_history::with_live_run` under `HISTORY_LOCK`, and a network call
there would hold the vault lock for up to 4 s.

## Measuring Jev: the study runner (agent effectiveness E3)

Whether each use stays is decided by a study (decision Q3, plan "keep it
optional and earn its place"): every use, local against Jev, on the same
judged items (`scripts/agent-eval/jev-fixtures/`, 22–26 synthetic items per
use). The decisions come from the production code, run by a hidden
subcommand of the normal binary (`jev_study.rs`):

```
hollow jev-study <fixture-dir> <out-dir> (--endpoint <loopback url> | --typesafe) [--max-calls 200] [--items N] [--condition <name>]… [--resume]
node scripts/agent-eval/jev-study.mjs score <out-dir>
```

- It **refuses to run** without `--endpoint` pointing at a literal loopback
  IP (`127.0.0.1` or `[::1]`, plain http; the same rule as the bed
  candidate's fake) or an explicit `--typesafe`. A loopback run sends a
  placeholder key; `--typesafe` reads this instance's saved key (Keychain,
  as the app does) and never prints it. Only the orchestrator runs
  `--typesafe`; it bills the key's TypeSafe account.
- Each use goes through its real path with Jev off and on: `select_connectors_via`,
  `prioritize_passages_via`, the exact dedup rules and `jev_decide::check_traced`
  (three uses since whiteboards went). Nothing about the transport changes:
  one request per item at most, 4 s, 64 KiB, no redirects, no retry.
  `--max-calls` (default 200) stops before a request that would pass it.
- `run.json` (requests made, the cap, where it stopped, `state`) is on disk
  before the first request and rewritten around every one, with the Jev
  arm, so an interrupted billed run keeps its tally. A request is counted
  before it is sent (`calls` includes it and `inFlight` names its item
  until it returns), so a study killed mid-request keeps it counted. An out
  folder that holds any study file is refused unless `--resume`, which
  continues that study: finished arms are kept, a stopped arm goes on from
  its first unanswered item (an item killed in flight is sent again, and
  counted again), the count starts from the larger of `run.json`'s and the
  arm files' own, and `--max-calls` caps the study's total across resumes.
  The earlier run's `error` moves to its `resumes` entry.
- Each result records the decision, latency, the outcome (`off`, `ok`,
  `timeout`, `malformed`, `error`, or `not-sent`), the requests made and the
  cost (0 with none, `null` with one). A failed request keeps production's
  local decision, and the result carries it.
- The scorer gives precision/recall/F1 per use, explicit-name and
  cardinality failures, latency p50/p95 and requests, and Jev minus local
  with paired bootstrap intervals. A use is a keep candidate only with zero
  explicit-name, cardinality and missing-answer failures and a headline
  interval above 0; the cost half of the gate waits for a real per-call cost.

The retired bed row `ae-jev-study` ran the candidate's binary against the bed's
fake in score, timeout and malformed mode ([TEST_BED.md](history/TEST_BED.md#agent-effectiveness-rows));
[AGENT_EFFECTIVENESS.md](history/AGENT_EFFECTIVENESS.md) has the details and limits.

## Adding a new Jev decision

1. Give it its own `Settings` flag in `jev.rs` (serde default `false`, so
   saved settings stay opted out), a `*_key()` accessor, a `Status` field and
   an optional `save_jev_settings` argument that preserves the saved value
   when omitted.
2. Keep the local behaviour as the result whenever the key accessor returns
   None or the call fails. Test that path.
3. Never call Jev while holding `HISTORY_LOCK`, the permission write guard or
   any lock another process waits on.
4. Bound what is sent, say what is sent in the Settings copy, and add a row
   to the table above.
