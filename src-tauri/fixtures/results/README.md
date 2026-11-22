# Result-classification fixtures (Paperclip PR 14a)

`src/result_kind.rs`, `src/agent_tests.rs` and `src/workflows_tests.rs` tests read these
files. They are **stdout** (or, for `*.stderr.txt`, stderr) of the agent CLIs,
in the shape Hollow's parsers consume. No live provider turn was sent for this
PR, so every file but one is **SYNTHETIC**. If a real shape differs, the run
degrades to the kind `error` (today's behaviour), never to a wrong success.

## Recorded (user-consented, 2026-09-25)

| File | Source |
|---|---|
| `antigravity-denied-write.jsonl` | `agy -p … --output-format stream-json --sandbox --mode accept-edits` (Antigravity 1.2.10), copied from the Milestone 3/4 probe capture. A headless write is auto-denied: exit 0, `result.status: "SUCCESS"`, `response: ""`, `denied_actions: [{action: "write_file"}]`. |

## Synthetic

| File | What it stands for | Where the shape comes from |
|---|---|---|
| `claude-refusal.jsonl` | a refusal at exit 0 | Claude Code 2.1.283's result schema (`stop_reason`), and its bundle's `stop_reason==="refusal"` checks |
| `claude-max-turns.jsonl` | `--max-turns` reached | the 2.1.283 bundle: subtype `error_max_turns`, `errors: ["Reached maximum number of turns (N)"]`, terminal reason `max_turns` |
| `claude-quota.jsonl` | a plan usage limit | the "Claude AI usage limit reached\|<epoch>" text PR 9b's table already uses |
| `claude-overloaded.jsonl` | HTTP 529 overload, with the synthetic API-error assistant message | the 2.1.283 schema's `@internal` `is_api_error_message` / `api_error_status` fields |
| `claude-auth.jsonl` | HTTP 401 | the "Failed to authenticate. API Error: 401 …" text in CLAUDE.md Quirks |
| `claude-unknown-session.jsonl` | a `--resume` of a session Claude no longer has, Paperclip's shape: a new id in `system/init`, then an `errors[]` result | Paperclip's claude-local tests |
| `claude-unknown-session.stderr.txt` | the same failure with no stdout at all (exit 1) | the exact string in the 2.1.283 binary, `No conversation found with session ID:` |
| `claude-poisoned-message-id.jsonl` | a resume rejected over a poisoned message id (result only) | Paperclip's poisoned-message-id retry case |
| `claude-poisoned-api-error.jsonl` | the same, wrapped as an API-error assistant message before an `is_error` result with no text | the 2.1.283 schema's `is_api_error_message` |
| `codex-quota.jsonl` | a ChatGPT-plan usage limit as `turn.failed` | codex-cli's "You've hit your usage limit" wording |
| `codex-high-demand.jsonl` | a bare top-level `{type:"error", message}` event | codex-cli 0.153.4's `error` event (before PR 14a Hollow read it as "codex error") |
| `codex-at-capacity.jsonl` | an OpenAI overload as `turn.failed` (overnight 9 A3) | the error object verbatim (`message`, `codex_error_info: server_overloaded`) from the codex-cli 0.163.0 rollout of `s-20261019-186e1e2e`, 2026-10-19T00:59:33Z, in exec's `turn.failed` envelope |
| `codex-unknown-thread.stderr.txt` | `exec resume` of a thread with no rollout | codex-cli's "no rollout found for thread id" wording |

The tests also reuse the recorded `../usage/` files: `claude-oneshot.jsonl`
(success), `claude-max-budget.jsonl` (a budget stop: subtype
`error_max_budget_usd`, terminal reason `budget_exhausted`, exit 1),
`codex-exec.jsonl` (success) and the synthetic `codex-failed.jsonl`.

Paperclip PR 14b's fresh-retry tests in `src/chat_tests.rs` drive a stub CLI that prints
`claude-unknown-session.jsonl`, `claude-unknown-session.stderr.txt` (to stderr, no
stdout) or `claude-poisoned-api-error.jsonl` on the resume attempt and the recorded
`../usage/claude-oneshot.jsonl` on the fresh one; a few one-line streams (a tool use
before the failure, a resumed legacy session) are written inline in those tests.
