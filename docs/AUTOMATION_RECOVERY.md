# Automation recovery

What a run did, which retry is safe, and what runs in the background (plan
completion L2, 2026-10-13; closes review V03 and the UI/UX "Delegate safely"
journey). The modules' own header comments are the authority; this page is
the map.

## What a run did beyond its files (`run_effects.rs`)

Every evidence receipt already proves a run's vault changes (History's diff),
repository changes, checks, pull requests and started services. Since L2 it
also carries `outsideEffects`, built from the run's own tool calls (the
`EvidenceCollector` sees them for every surface: automations, chat turns,
runbook steps):

| Part | What goes in it |
|---|---|
| `recorded` | Calls Hollow can name as acting outside the run's files: built-in connector writes (none since Hollow's own Notion connector was removed on 2026-11-10), Hollow app changes (any non-read action), task writes, `git push`, `gh pr / issue / release / repo / workflow` changes, `gh api` with a method or fields, `curl` / `wget` that send data, package publishes, `ssh` / `scp` / remote `rsync`, opening apps or URLs, and other MCP servers' write-verb tools. Each is `done`, `failed` (refused, nothing sent: only for Hollow's own executors, whose errors prove it) or `uncertain` (no reply, a lost reply, a partial write, any errored shell command or other server's call: an exit status doesn't prove nothing happened). |
| `unknown` | Calls whose effects Hollow can't see: shell commands it can't classify as read-only or a check (`evidence::check_kind`: a bounded test / build / typecheck / lint, or `npm run gate`, is local work), other MCP tools it can't classify, helper agents and teammate consultations (their tool calls aren't in the parent's stream), and, for an agent whose stream reports no tool calls (Antigravity), one "unreported agent calls" group. Grouped, with up to three redacted examples. |

Rules: anything the classifier can't place is unknown, never read-only; a
new receipt always has the field (`{}` when the run did nothing outside), and
an older receipt without it is **unknown**, never "none". A call Hollow
executes itself (Hollow app, tasks) carries a `key`: a
SHA-256 of server, tool and arguments (key order doesn't matter; a task
write's agent-chosen `idempotencyKey` is left out, so a retry run's new key
still matches). A timeout reply ("did not answer", "may have completed") is
`uncertain`, so it is guarded too.

## Retries match the record

- **Automatic retry** (`workflow_retry::retry_plan`): a transient failure still
  gets one retry a minute later, but never when the failed run's receipt shows
  a done or uncertain outside effect, has unknown calls, or is missing
  (`RetryInput::outside_effects_unsafe`, computed in `workflows::finalize`).
- **Guarded retry** ("Run again without repeating"): `run_workflow` takes
  `retryOf` (the failed run's history id). `enqueue_run_now` checks that run
  has a receipt and belongs to this automation, then queues the manual row
  with `{retryOf}` in its payload; the claim reads it into
  `RunContext.guarded_retry_of`. Before anything spawns, `prepare_spawn`
  writes the run's guard (`<app data>/RetryGuards/<run>.json`, outside the
  vault, removed when the run finalizes) and adds a note to the prompt saying
  what already happened, what Hollow will block and what it can't.
- **Enforcement** of the guard, at every executor Hollow owns:
  `app_broker::repeat_refusal` (Hollow app tools) and `task_broker::dispatch_at`
  (task writes). An identical
  call is refused with "Not repeated: …" and nothing is sent. Shell commands
  and other servers' tools can't be blocked; the prompt and the run review say
  so.
- A guarded retry would join an already queued Run now row (the queue
  coalesces manual rows) and run unguarded, so `enqueue_run_now` refuses it
  while any row of the automation is queued or running, and the run review
  disables the button meanwhile.
- **Retry of a retry.** The guarded run's own receipt carries the failed
  run's done and uncertain effects (`run_effects::carried`, labelled
  "(earlier run …)"), and a refused repeat ("Not repeated: …") is not
  recorded as a fresh failed call. So a retry of the retry still guards the
  first run's effects (and its unknown calls), and its review still offers
  the guarded retry. Only an errored reply from Hollow's own executors that
  starts "Not repeated:" is treated as a refusal; shell output can't hide an
  effect that way.

## The run review (Automations → an automation)

`src/runOutcome.ts` (pure, tested) turns the newest run's log row, receipt
and cost (`run_cost`: `descendants::rollup`, unknown never $0) into:

- **State**, in the shared operational words (`OPS_STATE`): Never run,
  Preview, Queued, Running, Waiting for you, Blocked, Failed, Stopped,
  Ready for review, Finished. Runbook and Tonight pills use the same
  explanations (`runbookStateHint`, `nightStateHint`).
- **What happened** (`components/RunOutcome.tsx`), right after What to fix:
  Changed, Results (declared result files, else what it wrote), Checks, Cost,
  Outside its files, Not visible to Hollow, and one sentence on how complete
  the record is.
- **Running it again**: `fixFirst` (a hold or a budget stop: fix it first),
  `guarded` ("Run again without repeating"), `checkFirst` (unknown calls, or
  no receipt), `currentFiles` (only files changed: Open in History first, then
  Run now), `safe`.
- **What to fix** (`workflowHealth.failureFix`) keeps its Accounts /
  Permission changes / Budget pauses / Edit links. A sign-in failure no longer
  says the run "stopped before doing any work": only a receipt that proves it
  changed nothing may say so (V03).

Receipts everywhere (chat turns, run history, runbook conversations) show an
**Outside its files** section. Runbook runs and Last night items link to their
conversation, where each step's receipt is.

## Background work, explained together

`src/backgroundWork.ts` + `components/BackgroundWorkExplainer.tsx`: "How
background work runs", a disclosure on every Automations view, also opened
from the Main agent page header's More ("How check-ins run"). Scheduled automations,
Tonight, the silent heartbeat and Main agent check-ins, each with when it runs,
what it sees, what it may do on its own, what it leaves behind, and a button to
where its settings live. Shipped defaults are stated as defaults; the sources
for each claim are listed in the module header.

## Code map

| Concern | Files |
|---|---|
| Effects, guard, retry note | `src-tauri/src/run_effects.rs`; receipt field in `evidence.rs` |
| Automatic retry gate | `workflow_retry.rs` (`outside_effects_unsafe`), `workflows.rs` (`finalize`) |
| Guarded retry plumbing | `commands.rs` `run_workflow(retry_of)`, `workflows.rs` (`enqueue_run_now`, `guarded_retry_effects`, `prepare_spawn`), `workflow_hygiene.rs` (`RunContext.guarded_retry_of`) |
| Enforcement | `app_broker.rs`, `task_broker.rs` |
| Cost read | `commands/usage_api.rs` `run_cost` (action `usage.run_cost`) |
| Run review | `src/pages/Workflows.tsx` (views), `src/pages/workflows/runReview.ts` (controller), `src/runOutcome.ts`, `src/components/RunOutcome.tsx` |
| Editor | `src/pages/workflows/workflowEditor.ts` (controller), `workflowText.ts` (helpers) |
| Explainer | `src/backgroundWork.ts`, `src/components/BackgroundWorkExplainer.tsx`, `src/store/automationsViews.ts` (`openBackgroundWork`) |

## Tests

- Rust: `run_effects::tests` (classification, keys, guard, note),
  `workflow_retry::tests::a_run_with_outside_effects_never_retries_on_its_own`,
  `workflows::tests::a_guarded_retry_carries_its_failed_run_and_refuses_a_foreign_one`,
  `app_broker::tests::a_guarded_retry_refuses_the_exact_app_call_its_failed_run_made`,
  `task_broker::tests::a_guarded_retry_never_repeats_the_same_task_write`.
- Vitest: `runOutcome.test.ts`, `workflowHealth.test.ts` (the sign-in
  wording; fails against the pre-L2 code).
- Browser: `tests/smoke/automation-recovery.journey.spec.ts`, click-only, at
  desktop, the native minimum window and Largest text. Mock switches:
  `localStorage.mockOutsideEffects` (`none` | `notion`, a page another Notion
  server made; unset is an older receipt), `mockRunCostUnknown`.

## Not verified here

Native: the guard against a real Hollow app action in
an installed build, and the explainer and run review under VoiceOver, glass
and trackpad scrolling. Those need a disposable native profile (L5's plan).
