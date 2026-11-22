# Usage stream fixtures (Paperclip PR 6)

Parser, accumulator and ledger tests read these files. They are **stdout**
recordings of the installed CLIs, not transcript files.

## Recorded (user-consented, 2026-09-25)

Captured in a throwaway temp folder (never the vault) with the argv Hollow
builds in `agent.rs` (`build_claude` / `build_codex`). Ids were replaced with
stable placeholders (`msg-001`, `toolu-001`, `id-001`, ... — consistent within a
file set, so message-id deduplication is testable), signatures and hook output
were blanked, init inventories emptied and personal paths replaced. **Usage and
cost numbers are untouched.** CLIs: Claude Code 2.1.280 on a subscription login
(model `claude-opus-5-5[1m]`), codex-cli 0.153.4.

| File | Invocation | Mirrors |
|---|---|---|
| `claude-oneshot.jsonl` | `-p … --output-format stream-json --verbose --permission-mode acceptEdits --no-session-persistence` | workflows, one-shots |
| `claude-chat-tool.jsonl` | `… --include-partial-messages`, one Bash tool call, two assistant messages | a fresh chat turn |
| `claude-chat-resume.jsonl` | the same session with `--resume <id>` | a resumed chat turn |
| `claude-max-budget.jsonl` | non-interactive with `--max-budget-usd 0.01` | a budget stop |
| `codex-exec.jsonl` | `codex --disable apps --disable plugins --disable remote_plugin exec --json --skip-git-repo-check -s workspace-write` | a Codex turn |
| `codex-resume.jsonl` | `codex … exec resume <thread> --json --skip-git-repo-check` | a resumed Codex turn |

What they establish (these facts drive `src/usage.rs`):

- `result.modelUsage` includes side models (a `claude-haiku-4-5-*` entry on
  every run); top-level `usage` covers the main model only and is all zeros on
  `error_max_budget_usd`. Always sum `modelUsage`.
- On `--resume`, `modelUsage` and `total_cost_usd` are **session-cumulative**;
  top-level `usage` is per-invocation.
- `assistant` events carry a start-of-message usage snapshot. The final
  per-message usage arrives only in `stream_event` → `message_delta`, which
  needs `--include-partial-messages` (chat, and since Paperclip PR 8b any
  non-interactive Claude run a token budget watches, parsed in usage-only mode).
- `--max-budget-usd` is enforced only after a message completes (the capture
  ended at $0.173 against $0.01), exits 1 with subtype `error_max_budget_usd`
  and no result text (PR 8b's backstop: a budget stop, recorded from
  `modelUsage`).

- Codex `turn.completed.usage` is per turn, including on resume, and
  `cached_input_tokens` is a subset of `input_tokens`. No cost field.

PR 8b's budget tests also cut `claude-chat-tool.jsonl` after its first
`message_delta` (line 19) to simulate a run killed mid-stream.

## Synthetic (no capture exists)

- `codex-zero-usage.jsonl`: a `turn.completed` whose counters are all zero
  (Codex's default usage), which must read as "not reported", never as zeros.
- `codex-failed.jsonl`: a `turn.failed` with no usage.
- `claude-subagent.jsonl` (OB05): a partial-messages Claude turn whose main
  thread (`msg-sa-1`, `msg-sa-2`) spawns a `Task` subagent with two messages
  (`msg-sub-1`, `msg-sub-2`, `parent_tool_use_id: "toolu-task-1"`), and a
  `result` whose `modelUsage` is the sum of all four messages' finals.
- `claude-compacted.jsonl` (OB22): a partial-messages Claude turn whose main
  thread sends a 161,002-token prompt (`msg-cp-1`), then a `compact_boundary`
  system event, then a 9,003-token prompt (`msg-cp-2`). No `result`. The
  context after it is `msg-cp-2`'s, never read as a reset.
