# Claude quota recovery

Hollow's automatic failover moves a turn between eligible **added Claude
accounts**. It does not change the selected inference provider/model, switch
to a paid API, create a credential or export a ChatGPT/Claude app connection.
Rotation is always on. Usage shows how it works, the per-account In rotation
checkbox, the routing ceiling and Check usage. Single-account, pinned-account
and failover-off settings migrate away on load; compatibility setters cannot
restore them. Organization boundaries remain authoritative.

The agent settings catalog exposes only `accounts.claudeCeiling` for the
early-move threshold. The retired routing and failover keys/actions are gone;
old window IPC calls remain compatible but cannot disable automatic rotation.

## Selection and rejection state

Initial admission and every quota retry refresh stale readings before choosing.
Refreshes are serialized, with account probes concurrent and individually bounded
to 12 seconds. Readings or attempts less than 120 seconds old are reused.
The score is the fuller live 5-hour or weekly window. Lowest usage wins;
recent picks break exact ties only, including peer picks. Unknown readings rank
after measured room. Conversation affinity never outranks headroom. Accounts
at the configured ceiling or 98% are excluded. Extra usage being disabled plus
a near-full plan window also blocks an account. No eligible added account,
or an unreadable account store, produces a waiting/repair message, never a login
fallback. Explicit login-bound connectors retain their existing boundary.

A provider quota rejection is saved independently of meters in
`claude-accounts.json` as `exhaustedAccounts` keyed by local account id, with
`resetAt` and `meterMismatch`. A later allowed/0% reading cannot erase it.
Provider reset text takes precedence, including the local-clock form
`resets 11:30pm`. Otherwise a blocking window's reset is used, or the next known
window boundary to recheck capacity. If no reset is known, the record remains
blocked until a probe supplies a future reset. The next boundary permits one
fresh attempt, not an assertion that a monthly spending cap has been lifted.
Rejections survive restart and concurrent settings saves; removed accounts lose
their rejection state. A stale reading that claimed room is logged as a mismatch.

Receipt `accountChoices` records each actual account, binding-window headroom
and selection reason, plus rejection/meter mismatches. It appears in the existing
receipt details. Tentative CFO account assignments are cleared when actual
admission evidence is recorded. The CFO's budget and reserve holds remain.

## Two different events

- A running turn can pass the Spread routing ceiling. Hollow moves only at
  its existing safe checkpoint, with a resumable native session and no open
  tool calls, question card or background work started in that attempt.
  Background work is preserved; a quiet chat row now explains that hold.
- A failed attempt can return a direct subscription/billing quota rejection.
  That rejection now counts as evidence even if the last meter said room or
  the usage probe is unavailable. Generic rate errors still need exhausted
  stream/probe evidence. Authentication failures and service-capacity/network
  outages do not walk accounts.

Terminal quota recovery visits each eligible account at most once per turn.
Proactive ceiling moves remain limited to one, before any other switch.
Monthly spend, spending-limit and usage-limit rejections count as quota errors.
Selection excludes
tried accounts, checks configured organization policy and eligibility, and
keeps the native conversation on terminal failover. It sends the existing
continue prompt rather than replaying the user's request. No eligible account
produces an Accounts/reset recovery message; it does not claim that every
stored account is exhausted. A second failure continues to the next untried
eligible account. Automation restarts retain their separate one-switch bound.
A session id carried from a prior turn is insufficient for a continue prompt:
this attempt must announce its own session before terminal failover can resume.
A direct quota rejection before init with no model reply, tool or question card
retries the exact current prompt and original resume identity on the next
account. It never sends a continue prompt into an unaccepted previous turn.
Claude-login-bound connectors
still require that login and cannot move to an added account.

Settings → Usage explains this scope and observed availability. Its summary
uses the loaded view, makes no new usage/token probe and does not promise that
an unknown account has room. Per-account exclusions, ceiling and organization
preferences remain; rotation and quota retry cannot be disabled.

## Investigation limits

This port of Sam's #1047 is grounded in source and mocked regressions.
No live credential, remaining-room probe or account state was inspected for
the port. A 100% meter alone is not a verified terminal provider rejection.
No live conversation was stopped, rerouted or modified to investigate it.

## Provider boundaries

Anthropic documents shared session/weekly limits and distinguishes them from
model-family limits. Changing Claude models therefore does not reset the
shared subscription window. The usage endpoint itself can also be rate
limited without stopping model requests. Newer Claude Code releases may wait
for a reset automatically, depending on policy; that behavior was not verified
for the live incident. [Claude Code usage documentation](https://code.claude.com/docs/en/costs)

Subscription sign-in and API-key billing are different access paths. Model
availability remains account-dependent. [Claude account and model guidance](https://support.claude.com/en/articles/14552983-models-usage-and-limits-in-claude-code)

API request-rate and monthly spending limits are separate; a generic HTTP 429
is insufficient to infer exhausted subscription quota. [Anthropic API limits](https://platform.claude.com/docs/en/api/rate-limits)

## Verification and remaining checks

Frontend policy and real chat-event tests cover off/unavailable/limited/unknown
views, quiet background notices, preserved busy state/draft/conversation and
foreign-conversation isolation. Rust tests cover direct rejection with stale
or failed usage probes, generic 429, auth, capacity, no eligible account, no
native session, tried-account bounds and independent job facts. Stub CLI
regressions preserve native resume and avoid resending the user request;
shared workflow classification retains its stop/spending guards.

Each landed port requires an independent review and a full exact-tree local
gate. Native provider switching remains a separate hand check after an
authorized install. No intentional quota exhaustion, paid inference, account
change or live-job interruption is needed for the gate.
