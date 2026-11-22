# Interrupts: one record for question cards and approvals

AG3 (2026-11-17). Hollow's own design, after AG-UI's interrupt and resume
vocabulary. Code: `src-tauri/src/interrupt.rs` (the shared type and the two
mappings). Fixtures: `src-tauri/fixtures/interactions/interrupt-*.json`, read by
the module's tests, so the page and the code cannot drift.

Question cards (`interaction_schema.rs`, `interactions.rs`, `question_mcp.rs`)
and the approvals broker (`approvals.rs`) keep their own stores and wire events
(`approval-request`, `approval-resolved`, `ChatInteraction`). `interrupt.rs` is a
pure mapping from each to one record; it stores, shows and answers nothing.

## The interrupt

```json
{ "id": "apr-19a7c2f1b3e-1f4a-3", "reason": "hollow:external",
  "message": "Publish the build to the beta channel", "toolCallId": "toolu-007",
  "responseSchema": { "...": "JSON Schema of the answer payload" },
  "expiresAt": "2026-11-17T21:02:00+00:00", "metadata": {} }
```

| Field | Meaning |
|---|---|
| `id` | Unique across kinds and launches. A question card keeps its store id; an approval is `apr-<launch>-<n>`, where `<launch>` is the process start time in milliseconds plus the pid, in hex (the old `apr-<n>` restarted at 1 each launch). |
| `reason` | `hollow:question`, `hollow:approval` (a change inside Hollow, a computer verb), `hollow:external` (the action leaves this Mac: kind `app-external`). |
| `message` | What the person reads: an approval's action text, a question card's title (else its description). |
| `toolCallId` | The agent tool call it belongs to, when bound (never guessed); `null` otherwise. |
| `responseSchema` | Approval: `{approved: boolean, reason?: string (max 500)}`. Question: the card's normalized request, whose answer is `interaction_schema::Answer`. |
| `expiresAt` | An approval: shown time plus the broker's 120 s client timeout. A question card does not expire: `null`. |
| `metadata` | Kind-specific extras (`kind`, `title`, `sessionKey`, `shownAt`; for a question `mode`, `status`, `supersedeOnUserMessage`). |

## The answer

```json
{ "interruptId": "apr-19a7c2f1b3e-1f4a-3", "status": "resolved",
  "payload": { "approved": false, "reason": "Use the staging channel" },
  "metadata": null }
```

`status` is `resolved` or `cancelled` (timed out, turn stopped). **Deny by
default** (`InterruptAnswer::approved`): only `resolved` with `payload.approved`
the boolean `true` approves; a cancelled answer, a missing field, `"true"` or `1`
all refuse. `payload.reason` is the Decline text (SA1, below); it is dropped on an
Allow. The answer's `metadata` is reserved for a phone signature (the phone
approvals design); nothing reads it yet.

## Where Hollow differs from AG-UI

AG-UI: pending interrupts block new input. Hollow: **a reply supersedes an open
question card** (`supersedeOnUserMessage`, default on; `interactions.rs`
`supersede_for_message`), so typing in the composer closes a card the person no
longer wants to answer. An approval card is never closed by a reply: it ends by
Allow, Deny, its 120 s timeout or its turn ending. The record carries no
"blocks input" flag.

## Not built here

- `GET /v1/waiting` lists question cards and, since B4 PR 1 (2026-11-19), `kind: "approval"`
  items read from `approvals::pending_cards()` through `phone_access::approvals_api::open_cards`
  (not `interrupt::waiting`). `GET /v1/approvals` lists the cards with a digest; answering from the
  phone is B4 PR 2 (`docs/phone-api/approvals.md`).
- `approvals::respond_interrupt` answers a card from an interrupt answer; no route
  calls it yet. The card UI still answers through `approval_respond`, which stays
  user-only (an agent must never answer its own card). A phone answers through
  `approvals::respond_from` (B4 PR 2), which takes the same one-shot sender under the
  same lock, so the first answer wins and records who answered (`AnsweredBy`).

## Claude tool defer (AG4 probe)

Claude Code's PreToolUse hook can answer `permissionDecision: "defer"`. Probed
on 2026-11-18 with Claude Code 2.1.326 (Haiku) by `scripts/probe-claude-defer.sh`
(manual, about a cent; never in a gate; it checks each point below except the
last and exits 1 when one fails):

- **Defer:** a `-p` turn ends at once with `stop_reason` and `terminal_reason`
  `tool_deferred`, `is_error: false`, an empty `result`, and
  `deferred_tool_use: {id, name, input}`. The tool did not run.
- **Resume:** `claude -p --resume <session>` (no new prompt) fires the hook again
  for the same call, same `session_id`. `allow` runs the tool and the turn goes
  on; `deny` refuses it, lists it in `permission_denials` and the model reports
  the refusal. The hook input carries `tool_use_id`, so the answer can be matched
  to the deferred call.
- **Parallel calls are fragile** (a hand run; the script does not repeat it):
  two Bash calls in one message both hit the hook
  and both were held, but `deferred_tool_use` named only one of them, and
  neither ran. A build must treat a deferred turn as one card for the named call
  and expect the model to retry any other.

So the round trip works on the installed version. Built as AG4b
(2026-11-19), Claude only (Codex has no such path; its approvals stay declined):

- **Hook.** An interactive local chat turn in ask-first mode (`default`) on
  Claude's own models carries one PreToolUse hook in its single `--settings`
  (`claude_defer::settings_with_hook`), matching `Bash`, `Edit`, `MultiEdit`,
  `NotebookEdit` and `Write`. Its command is the hidden `hollow claude-defer-hook`
  (`claude_defer::run_hook`), with the answers file path in
  `HOLLOW_DEFER_ANSWERS`. Other tools follow Claude's own flow (the hook prints
  nothing). Not on a runner run (the hook would run on the mini) or a
  self-hosted model.
- **Defer.** A turn ending `tool_deferred` is held, not shown: the empty
  `result` envelope waits like an error's (`TurnStream::defer_tools`), and the
  attempt loop (`chat.rs` `Next::Deferred`) shows the approval card (title
  `Claude: <tool>`, kind `app`, scoped to the turn's session and run, 120 s,
  `approvals::ask_deferred_call`), writes the answer for that exact call id in
  the answers file (`claude_defer::record_answer`, 0600, in the turn's scratch
  folder) and resumes the same native session with a go-on prompt. The hook
  fires again for that id: an Allow prints `allow`, a Deny or anything else but
  the boolean `true` prints `deny` with the person's words, an id with no
  answer defers again, so a second call gets its own card. A resume with a
  prompt through stream-json stdin was probed on 2.1.326: the hook re-fires for
  the same id and allow runs the call.
- **Declines.** A timeout, a Stop during the card, a sink with no app, or an
  answers file that cannot be written never allows: the first three decline the
  call with a fixed sentence the agent relays, the last ends the turn. At most
  24 cards per turn.
- **What the card shows.** The whole call: a command (line breaks kept, plus
  sandbox-off and background flags), or a path with the old and new text of an
  edit, each change of a MultiEdit, a notebook cell's new source, a write's
  content. Declined without a card, never shown cut: over 2000 characters, over
  40 lines, a run of 60 spaces, or a character that changes how text reads
  (bidi and zero-width marks, line and paragraph separators, the tag block). A
  call with no id is denied by the hook. Claude's hook precedence for other
  hooks (deny over defer over allow) is from its docs, not probed. A 120 s
  timeout is a recorded Deny the model sees; a Stop that lands with the click
  records a Deny.
- **Limits.** The card is not bound to the tool row (the broker's binding notes
  only Hollow's own tools), each deferred segment closes its own usage row, and
  the hook defers every gated call, including
  ones Claude's allow rules would have run. The phone cannot answer these cards
  yet (see "Not built here").

## Decline with a typed reason, subagent cards (SA1)

A Deny can carry the words the person typed (`approval_respond(id, approved,
reason?)`, cleaned by `approvals::clean_reason`: control characters to spaces,
500 characters). They reach the agent through the same refusal it already got:

- Socket gate (computer, M365): the verdict line is `{"approved":false,"reason":..}`;
  the MCP tool's error becomes `the user denied this action: <reason>`.
- In-process gate (Hollow app cards): `app_broker::declined` appends
  `Their reason: <reason>` to "The user declined <action>; nothing changed."

A card raised in a subagent's conversation (`sessions.fork_kind = subagent`)
carries `subagent: {key, parentKey, title}` in `approval-request`, and the chat that
started it shows the card too, with "From subagent: <title>".

## Cards raised to the COO (2026-11-20)

Alex: a card must not appear only in a subordinate chat. A card from a project
lead's conversation (`chat_chain::is_lead_conversation`), or from a subagent whose
parents lead (up to 12 steps) to a lead or to a COO conversation, is **raised to the
COO**: `approvals::coo_route_in` puts `routedTo: {askedKey, askedTitle, askedKind
(lead|subagent), cooKey}` in `approval-request` (both the in-process and the socket
payload; `ApprovalSink::coo_route`), and `PendingCard.routed_to_coo` says so. There is
still one card id, never a copy. The chat view (`chatApprovalOrigin.approvalPlacement`)
draws the Allow/Deny card only in the COO chat (the Main agent workspace's own
conversation, whichever thread is open), and in the asking chat and its parent only a
note, "Waiting for approval in the COO chat", with an "Open the COO chat" link
(`openMainAgent`); "the COO chat" is any open conversation in the Main agent's folder (`cooChatShown`), from the Main agent page or Library. When a routed card arrives and nothing on screen shows it, the listener opens the COO chat (`chatApprovals.ts`); inside the asking chain it only says so in the footer. The COO's rail row gets the approval mark (`rowStatus`). The answer is
by id (`approval_respond`), from the COO chat, the bell or a phone, so one answer resolves
both views and the waiting call continues as for any card. Not routed: the COO's own chat,
an ordinary chat, and a subagent of an ordinary chat (SA1 still shows that in its parent).
Hard-coded always-ask actions are unchanged; they simply ask in the COO chat now.

## Exact-action approvals (ND6, 2026-11-20)

An Allow is for one exact call. `approval_terms.rs` fingerprints the tool (or
action id) and its canonical arguments (SHA-256; key order and `null` against
`{}` do not matter, any other change does). `app_broker::execute` calls
`approvals::expect_call` before it asks, so the card binds to the arguments the
executor will run; the in-process ask puts `argsHash` and `notCovered` (what the
yes does not cover: only this call, one use, nothing else in the chat, and for
an outward card that an interrupted call may have happened) in the
`approval-request` payload and `argsHash` in `approval-resolved`, and leaves a
`Grant` for the asking thread. After the wait, the run-live check and every
other guard, `execute` verifies each grant against the arguments about to run:
a mismatch voids the approval, nothing runs and the agent is told to ask again.
Not covered here: the socket path (an MCP subprocess runs its own call after
the reply) and the deferred Claude tool call (Claude's hook is already exact
per call, `claude_defer`); the phone digest still signs title and action text
only.

## Uncertain is a state (ND9, 2026-11-20)

`approval_uses.rs` journals every External-tier action (it leaves this Mac) in
app data (`ApprovalUses/<fingerprint>.json`, outside the vault, pruned after a
week). `app_broker::execute` calls `check` before it builds the card and `begin`
inside `run`, just before the handler; `Open::finish` closes it as `done`,
`failed` or `uncertain`. Fail closed: only an error whose words say nothing was
sent ("nothing changed", "nothing ran", "not approved") is `failed`; any other
error, and a panic (on drop), is `uncertain`, and a clean failure of a retry over
an uncertain record keeps it uncertain. A process that dies between
`begin` and `finish` leaves `started`, which the next launch reads as uncertain.

| Found before a call | What happens |
|---|---|
| `done` in this run | Refused, no card: one approval is one use. |
| `started` in this launch | Refused: it is running now. |
| `uncertain`, or `started` from an earlier launch | Never retried on its own. The card is forced even in bypass or any standing approval, carries "An earlier attempt of this exact call (from ...) was interrupted and may already have happened. Check the destination first", and a new Allow replaces the record. An automation or an unattended check-in is refused. |
| `failed`, or `done` in another run | Clear: a new approval is a new use. |

`begin` is the gate: under a process-wide lock it re-reads the journal and
refuses a call that is done in this run, running now (this launch, or another
Hollow process whose pid is alive, such as `hollow serve`), or uncertain without
the card's time (`acknowledged`). An unreadable record reads as uncertain, never
clear; only `done` and `failed` records are pruned (after a week). A record that
cannot be written stops the action (nothing runs). Not covered
yet: Confirm-tier actions (they land in the run's History diff), the socket
path, and the Cua outward click, which goes through `computer_broker` and needs
its own journal call.
