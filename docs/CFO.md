# CFO

The CFO (COO rebuild row L7, 2026-11-10; design: vault
`outputs/coo-leads-subagents-2026-11-10/design.md`, section 4) is not a chat. It
is a Rust module, `src-tauri/src/cfo/`, that every turn passes through. For each
prompt and each subagent it decides **agent, model, effort and account**, and
records the decision with a one-line reason. It makes no model call, so it adds
no time to a turn.

Two rules come first: the person's explicit choice always wins, and an agent's
explicit choice wins when the allowlist permits it.

## Modules

| File | Owns |
|---|---|
| `cfo/mod.rs` | The public API: `Decision`, `decide_turn`, `run_effort`, `place_subagent`, `unattended_hold`, `view`, `check_models`, and the commands `cfo_settings`, `cfo_save_settings`, `cfo_approve_extra_usage`, `cfo_view` (each with a COO action). |
| `cfo/settings.rs` | `CfoSettings` in the `cfo_settings` table (one JSON row, key `policy`) and the one-time migration from `company_settings`. |
| `cfo/job.rs` | `JobKind` and `classify(prompt, origin, build)`. |
| `cfo/effort.rs` | The level per job kind (K2) and `EffortSource`. |
| `cfo/accounts.rs` | Readings, `evaluate`, `hold_for`, `choose_account` (moved from the company code's `cfo.rs`). |
| `cfo/choose.rs` | `candidates`, `allows`, `choose` (moved from the company code's `routing.rs`). |
| `cfo/budgets.rs` | Budgets per project and per subagent (L9): `claim` (the pump), `Watch` (a running turn), `record_pause`, `pause_note`, `set_subagent_budget`. |

The company code these pieces came from was removed 11-10 (L11); the copies
here are the only ones. The CFO still reads the `company_settings` table once
(below), with plain SQL, tolerating a vault that never had it.

## Policy (`cfo_settings`)

`CfoSettings`, camelCase on the wire:

| Field | Default | Meaning |
|---|---|---|
| `allow` | `[]` | `[{agent, model?}]`: the models the CFO may use. Empty means every installed agent on its saved model. An entry with no `model` allows any model of that agent. Saved models must be in the agent's real model list (`agent_models` ids, plus the pinned ids its aliases resolve to); `cfo_save_settings` refuses anything else. |
| `effortMode` | `automatic` | `automatic`: the CFO picks per job kind. `fixed`: each agent's saved level in `agent_settings.efforts`, as before the CFO. |
| `reservePercent` | `20` | Room in every account's 5-hour and weekly window kept for the person's own prompts, 0 to 90. The pool ceiling for unattended work is `100 - reserve`. 20 matches the old 80% long-work line, so behaviour did not jump. |
| `extraUsageUntil` | none | RFC 3339: unattended work may run past an account's included usage until then. Set by `cfo_approve_extra_usage` (1 to 72 hours; 0 withdraws). |
| `projectBudgets` | `[]` | `[{workspaceId, fiveHourTokens?, weekTokens?}]` (L9): tokens a project may use over the last 5 hours and the last 7 days, rolling. At most 64; a budget with neither figure is dropped. |
| `subagentBudget` | `{}` | `{tokens?, minutes?}` (L9): every subagent's cap on its whole chain. Empty: no cap. Minutes at most 10,080. |

**Migration.** A vault with no `policy` row reads `company_settings` once:
`routing`'s `allow` list, and `cfo`'s reserve plus buffer as the reserve (capped
at 90) with its extra-usage approval. It writes the `policy` row and never reads
the company rows again. A damaged company row is ignored. The company rows are
left as they are, and no company table is altered.

The COO reaches all four through agent actions (COO parity L13):
`settings.cfo_settings` and `settings.cfo_view` read; `settings.cfo_save_settings`
and `settings.cfo_approve_extra_usage` are confirm actions that ask in `auto`
(the approval asks in Bypass too and never runs unattended: it is on
`app_actions::ALWAYS_ASK` since the 11-10 sweep), whose card shows old to new (or the hours approved), with the same checks as
the commands (real models only; 0 to 72 hours). Commands and actions share
the inner functions `settings_in`, `view_in`, `save_settings_in` and
`approve_extra_usage_in`.

## How it decides

**Job kind** (`job.rs`): deterministic word matching over the first 2 KB of the
prompt, on word edges, first match wins: review words (review, check this diff)
lean Review; code words (PR, merge, fix, implement, refactor, build, test,
`hollow-slot`, ...) lean Code; plan, design, roadmap lean Plan; research, find
out, compare lean Research; report, summary, digest, status update lean Report.
A build workspace leans Code. A question under about 200 characters that ends in
`?` is a Question. Otherwise an automation is a Report and anything else is a
Conversation. Check-ins and wakes are always Conversation. The reason the footer
shows comes from the same match (`code change`, `review`, `short question`, ...).

**Origin** (`Origin`): `Person` (the person sent it: attended), `Subagent`
(another agent started or messaged it, or it has a parent conversation),
`Automation` (night items, unattended runbook steps, workflows) and `CheckIn`
(a turn Hollow queued itself, the heartbeat).

**Effort** (`effort.rs`, K2): Conversation and Question low, Report and Review
medium, Research, Plan and Code high. A requested effort wins (source `you` for
the person's own turn, `agent` for an agent's `start_session` or
`send_to_session` effort). Then Automatic gives the kind's level (`cfo`) and
Fixed the saved level (`fixed`). Ultracode stays the chat's switch. The level is
a request: `effort::resolve` maps it per model downstream, as before.

Precedence for a person's chat turn (settings plan PR 12): the chat's own effort
wins, then a fixed level, then Automatic. The composer's Effort (`chatEffort`) goes
with each `chat_send` as `effort`, is checked against `effort::LEVELS`, rides the
queued row's payload (`ChatTurnPayload::effort`) and reaches `turn_cfo` as
`requested_effort`; the decision records the level used and its source in the run
manifest and on the receipt. Test:
`chat::cfo_turn_tests::a_chats_own_effort_wins_then_a_fixed_level_then_automatic`.
Resume after a restart (Waiting on you, Resume ticked) sends each chat's own
stored effort the same way (`ResumeRequest::effort`, checked by the same
`chat_effort`; an unknown level refuses that chat's resume; overnight C14).

**The chat's own effort lives in Rust** (2026-11-12): the session store's
`chat_efforts` table (`session_store::{chat_effort, set_chat_effort,
import_chat_efforts, inherit_chat_effort}`), keyed by session key, with no
`sessions` row needed. The window reads and writes it through `chat_efforts_list`,
`chat_effort_set` and `chat_efforts_import` (`src/store/chatEffortStore.ts` is
its cache; Default clears the row; New chat starts a fresh conversation and the
old one keeps its level). The old localStorage `hollow.effort.<key>` values move
over once at start (the import never overwrites a level Rust has) and are then
removed. Every turn resolves its requested effort when it runs
(`chat::resolve_requested_effort`): the send's named effort, else the chat's
own, else none (Fixed, then Automatic). So an automation in the chat, a
wake-up (`wake_me`), `continue_after`, a COO-triggered resume
(`sessions_api::resume_requests`), a phone send and a handover successor's first
turn all run at the chat's level, and a follow-up (`app_sessions::follow_up_for`)
names only the caller's level, so the run reads the chat's own when it starts
(a level changed or cleared while it waits applies); a fresh-brief follow-up
names the chat's own, since its new conversation has none. A turn's recorded
level is history, never reused, so Default holds and a one-turn override stays
one turn (2026-11-13). A send typed in a conversation that continued elsewhere,
whose level equals the old conversation's own, has that level dropped when
admission forwards it (`chat_queue::accept_subagent_turn`), so the successor's
own level runs; a level that differs (a phone pick) is kept. The window chains each chat's saves
(`putChatEffort`), so they reach Rust in the order picked. `record_handover`
copies the predecessor's level to the successor when the successor has none. Tests:
`chat::tests::runs_the_backend_starts_use_the_chats_own_effort`,
`app_sessions::tests::a_handover_successor_inherits_the_chats_effort_and_a_follow_up_uses_it`,
`app_sessions::tests::a_follow_up_never_reuses_a_recorded_or_queued_effort`,
`run_pump::tests::a_forwarded_send_runs_at_the_successors_effort_not_the_old_composers`,
`sessions_api::tests::a_coo_resume_runs_at_the_chats_own_effort`,
`phone_access::tests::controls_tests::a_phone_send_without_an_effort_runs_at_the_chats_own_level`
(the real phone route; added 2026-11-13),
`session_store::tests::a_chats_own_effort_is_kept_cleared_imported_and_inherited`,
`src/store/chatEffortStore.test.ts` (migration, save order).

**Agent and model** (`choose.rs`): a named agent or model must be installed
and, with a non-empty allowlist, inside it; otherwise the start is refused with
the allowed list and nothing starts. A model named with no agent goes to an
agent whose catalog (or a non-empty allowlist entry) has that model, else the
app's default agent. With nothing named (or `auto`) and an EMPTY allowlist, a
subagent runs on the app's default agent as before; a review still goes to
another agent than the calling chat's. With a non-empty allowlist the CFO picks
from it: a review on another agent than the producer's, then the app's default
agent if it is among the least busy (chat turns queued or running per agent),
then the least busy allowed agent. Model ids match through the catalog's alias
map: Claude's family aliases (`sonnet`) resolve to their pinned ids
(`agent_models::newest_claude`); the other agents' catalogs expose no alias map,
so their ids compare as written. The same allowlist applies when an AGENT names
a model or agent on `send_to_session`, `handover` and `ask_agent`
(`cfo::check_named`); the person is never restricted. A `cfo_settings` row that
can't be read counts as the defaults here, as for turns. The placement's reason
("independent of Claude", "least busy allowed agent") rides a `cfo-<hex>` client
turn id to the child's first turn (`remember_placement`, `take_placement`, in
memory) and joins the job's reason in its decision ("review, independent of
Claude").

**Account and the reserve** (`accounts.rs`): readings are the in-memory ones
Hollow already keeps (Claude accounts' stream and refresh readings, Codex's
passive rate-limit events); nothing here asks a provider. Only enabled Claude
accounts count; with none enabled, Claude Code's login (where the router then
runs) is read instead. A window whose reset time has passed, or with no reset
time and a reading older than 30 minutes, counts as unmeasured (room), never as
full. An agent with no readings has room. Approved extra usage (not expired)
lifts both the reserve line and the 100% line.

The hold and the pick use ONE set of accounts (`accounts::pool`,
`claude_accounts::long_work_eligible`): the account the router would really
start the run on, plus, only while failover is on, every other usable account
in its organisation (any organisation when crossing is allowed) that is not out
of allowance. Unattended work is held when none of them has room; an account in
another organisation never lifts it. Among those with room the CFO picks the one
whose window resets soonest (its unused allowance would be lost first); resets
within 30 minutes of each other count as a tie, broken by more headroom. Reset
times are compared as parsed times. The person's prompts are never held and
never moved; `claude_accounts`' spread decides for them.

## Budgets (L9)

Budgets count real usage, tokens (input plus output, as `usage_ledger` records
them) and a subagent's working time; dollars stay a side estimate in the spend
view and never limit anything here. Both default to no cap, so an overnight lane
never stops on a cap the person didn't set. Only unattended work is bound (the
same `run_pump::cfo_unattended` set as the reserve); the person's own prompts,
even typed into a subagent, always run.

- **Project:** the tokens of every ledger row whose `workspace_id` is the project
  (written by `run_attribution`) at or after now minus 5 hours, and now minus 7
  days. Past either, the project's unattended chat turns wait at claim with a
  `CFO budget: ` note (deferred, rechecked every minute, clearing as usage rolls
  out), its running subagent turns end at their next safe point, and the person
  gets one notification per project per 5 hours.
- **Subagent:** its handover chain's tokens and working time (`subagents::
  chain_usage`, the same sums as the Subagents tab). The cap is the policy's
  `subagentBudget`, each figure overridable for one subagent in
  `cfo_subagent_budgets` (`session_key`, `tokens`, `minutes`, `parent_key`,
  `paused_at`, `paused_reason`, `set_at`). Past it, a running turn ends at its
  next safe point (`Yield::Budget` in `chat.rs`, the night guard's path; the turn
  ends with the pause note) and a queued turn fails at claim with the same note
  (a hold would wait forever, silently). The ledger is read at most every 15 s.
- **Telling the parent:** the pause is recorded on the subagent and
  `descendants::child_summary` adds the note to its line, so the COO's wake, an
  `onSessionsEnd` wake and the parent's next-turn note all say why it stopped. The
  person gets one notification ("Subagent paused: ...").
- **Raising it:** `sessions.subagent_budget {session, tokens?, minutes?}` (agent
  action, write tier) sets one subagent's figures and clears its pause; a figure
  left out keeps the default. Only a conversation up the subagent's own line (its
  parent, the parent's parent) or the COO may call it: never the subagent
  itself, a sibling, one of its own subagents or an automation. A pause clears
  when the subagent next runs (a raised budget, or the person's own prompt). The
  person sets the defaults and project budgets in Settings, Usage.

## Spend view (L9)

Settings, Usage gains four tables after By agent, from `usage_ledger::summary`
(`UsageSummary.byProject`, `byLead`, `bySubagent`, `byAccount`):
**By project** (`workspace_id`, "No project"), **By lead** (each top-level
conversation with its subagents' and consultations' usage rolled up, found by
following `sessions.parent_key` while `fork_kind = 'subagent'`), **By subagent**
(each subagent, 50 at most by tokens, then "Other subagents") and **By account**
(`account_ref`, labelled by hashing the added Claude accounts' ids; rows with no
account under their agent). A handover successor counts with the conversation
its chain began as. By project and By account each sum to the total.

## Overhead (L10)

The CFO's first goal is Hollow's per-turn overhead. A subagent gets the **lean
profile** (`chat::lean_subagent`, docs/AGENT_CONTEXT.md, the child row): no
visual guide, a short capability index, no question-card or next-step guidance;
the COO and the leads keep the full set. For a Claude subagent's first turn the
appended prompt falls from 22,765 to 18,473 bytes. `chat_g2_budget_tests::
a_subagent_turn_carries_no_visual_guide_question_policy_or_next_step` holds it
for every provider.

The 11-10 ablation (vault `outputs/coo-rebuild-2026-11-10/l10-ablation.md`: 2
bench tasks, 7 ways, Sonnet 5.5, one run each, $4.28) found the static layers
add 6 to 8K new-context tokens to a first turn (9 to 21%) and 8 to 14% cost; the
lean subagent is within 15% of the terminal on total tokens on both tasks and
+7 to 18% on new-context tokens. No single layer's removal beat the run-to-run
noise, and the bench's 1.4 to 3.3 times the terminal's tokens is not explained
by static layers, which points at Hollow's MCP tool schemas and the context
repeated per tool call. Next: measure with the tools attached, three runs per
cell, before trimming attended layers. To dump the real layer texts, run
`envelope_fixture_is_current` with `HOLLOW_DUMP_LAYERS=<dir>`.

## Where it hooks in

- **Chat turns** (`chat.rs`, the block that reads `agent_settings::effort`):
  `turn_origin` and `turn_cfo` make the decision, and `EvidenceCollector::set_cfo`
  records it in `RunManifest::cfo` and `EvidenceReceipt::cfo`. Every turn that
  isn't the person's (a subagent whatever its `unattended` flag, a night item,
  a check-in or wake) is long work, so a conversation's account affinity never
  keeps it on an account past the line; an unattended Claude turn starts on the
  CFO's pick when it is in the eligible set (`claude_accounts::long_work_may_use`).
- **Automations, the heartbeat and teammates** (`workflows.rs`, `heartbeat.rs`,
  `team_mcp.rs`): `run_effort` in Automatic mode. An automation is a Report
  unless its prompt says otherwise and starts on the CFO's pick
  (`unattended_account`); the heartbeat is a Conversation (and still capped at
  high); team-mcp reads the policy without writing.
- **Subagents** (`app_sessions::start`): `place_subagent`. `effort: auto` (L1's
  default) leaves the effort to the chat turn's CFO decision.
- **Holds** (`run_pump::claim_verdict`, `budget_verdict`): unattended rows go
  back to queued with the hold reason (`ClaimVerdict::Defer { held }`, its text
  starting `CFO hold: `) while `unattended_hold` says no account they may start
  on has room. Unattended means a chat row with `unattended`, `parent`,
  `appDepth` or a non-empty `hostAsker` (Hollow's own check-ins, wakes and
  child-completion reports for the COO or a lead), a cron or trigger
  automation, or a retry of one (the retried row's own source decides). Never
  held: the person's sends, Run now, a retry of a Run now, the heartbeat. An
  automation is judged on the agent it will really run on, a budget fallback's
  when the budget preflight moved it. A held cron occurrence past its misfire
  grace is skipped with a `not run: every account was past the reserve for your
  own prompts (Settings, Usage)` run-log row and ONE notification (`NoticeKind::
  Reserve`) naming the automation and the reserve; an occurrence that catches up
  keeps waiting instead.
- **Long-work line** (`claude_accounts.rs`): `LONG_WORK_CEILING` became
  `long_work_ceiling()`, `100 - reserve`, published by the CFO at app start
  (`publish_at_startup`) and whenever it reads or saves the policy
  (`set_long_work_reserve`; never in tests).
- **Saves** (`cfo_save_settings`): a patch of only the fields one panel owns
  (Agents: `allow`, `effortMode`; Usage: `reservePercent`), read, changed and
  written in one transaction (`settings::update`), as is an extra-usage
  approval, so the two panels never overwrite each other.

## Where it shows

- **Turn footer** (`TurnFooter.tsx`, `src/cfo.ts` `cfoFooterLine`): "Sonnet 5.5 ·
  Medium · review", with "(you)" after the effort when the person chose it.
- **Settings → Models** (`settings/CfoSettings.tsx`, inside `AgentDefaults`):
  **Effort** (Automatic or Fixed; Fixed shows the per-agent levels, under the
  Reasoning effort heading) and **Models
  the CFO may use** (each installed agent's real models as checkboxes, plus "Any
  model of this agent"; none checked says "Any installed agent, on its saved
  model"). No free-text box.
- **Settings → Usage** (`UsagePanel.tsx`, under the provider limit cards):
  **Reserve for your own prompts** (0 to 50%), one line per account ("Room for
  unattended work" or "Past the reserve until 3:00 PM"), **Extra usage for
  unattended work** (Approve for 4 h or 24 h, Withdraw), and (L9) **Budget for
  each subagent** (tokens, minutes) and **Project budgets** (per project: 5-hour
  and 7-day tokens, with its usage now and whether it is reached).

Settings search finds all six rows (`settingsSections.ts`). The preview mock
keeps the policy in `mockCfoSettings` and seeds accounts from `mockCfoAccounts`;
`tests/smoke/cfo-settings.spec.ts` covers the rows.
