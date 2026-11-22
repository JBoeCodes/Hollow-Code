# Question-card and owned-MCP stream fixtures (Paperclip PR 13)

Parser, recorder and store tests read these files (`agent_tests.rs`:
`parse_codex_normalizes_recorded_mcp_tool_calls` and
`parse_claude_replays_recorded_mcp_captures` for the four live captures;
`interaction_schema.rs` and `interactions.rs` for the synthetic ones). Like `../usage/`, they are
**stdout** recordings (or stdout-shaped files), not transcripts.

## Recorded (user-consented, 2026-09-25)

Captured in a throwaway temp folder against a three-note temp vault (never the
live vault), with the installed `hollow vault-mcp` as the Hollow MCP stand-in,
attached the way `agent.rs` attaches owned servers. Ids were replaced with
stable placeholders (`id-NNN`, `toolu-NNN`, `msg-NNN`), temp paths with
`<TMP>`, hook output blanked, and init inventories emptied except the MCP
tools and servers. CLIs: Claude Code 2.1.283, codex-cli 0.153.4. The probe
record is in the Paperclip state folder (`FINDINGS.md`, "Milestone 3/4 probes").

| File | What it shows |
|---|---|
| `codex-mcp-tool-call.jsonl` | `codex exec` with `default_tools_approval_mode="approve"` on the owned server: an `mcp_tool_call` item at `item.started` (`status: in_progress`, `result: null`) and at `item.completed` (`status: completed`, `result: {content, structured_content}`, `error: null`). |
| `codex-mcp-tool-call-unapproved.jsonl` | The argv Hollow built before PR 13 (no approval mode): the same item completes as `status: failed` with `error.message` "MCP tool call requires approval, but approval policy is never". This is the bug PR 13 fixes. |
| `claude-mcp-preapproved.jsonl` | Claude with `--allowed-tools mcp__hollow_vault` (the whole-server form) in `acceptEdits`: the model first calls the built-in `ToolSearch` (MCP tools are deferred in 2.1.283), then `mcp__hollow_vault__search_vault`. Top-level `parent_tool_use_id` is `null` on main-agent lines. |
| `claude-mcp-unapproved.jsonl` | The same call without the preapproval (Hollow's argv before PR 13): a `system` `permission_denied` event, then a `tool_result` with `is_error: true` ("…you haven't granted it yet"), and the run still ends `subtype: success`. This is the Claude half of the bug PR 13 fixes. |

What they establish:

- The Codex item fields are `id`, `type: "mcp_tool_call"`, `server`, `tool`,
  `arguments` (an object), `result`, `error` and `status`. There is **no**
  `isError` field: failure is `status: "failed"` plus `error.message`.
  `parse_codex` normalizes the pair to `ToolUse{name: "mcp__<server>__<tool>",
  input: arguments}` at `item.started` and `ToolResult{is_error}` at
  `item.completed`.
- Codex fails (never prompts for) an owned MCP call unless the server's
  inline override carries `default_tools_approval_mode="approve"`.
- Claude denies (never prompts for) an un-preapproved MCP call in headless
  `acceptEdits`, and the run still ends `success`. `--allowed-tools
  mcp__<server>` preapproves the whole server.

## Synthetic (no capture of `hollow_questions` exists)

The `hollow_questions` server did not exist when the probes ran, so these are
hand-written in the recorded shapes above. Field names match the captures; the
question payloads and the `Posted …` result text match `question_mcp.rs`.

| File | Models |
|---|---|
| `claude-ask.jsonl` | A main-agent question call: `ToolSearch`, then the `tool_use` assistant event (repeated once, as Claude repeats it per content block), then a `tool_result` starting `Posted 2 questions`. Native session `id-101`. |
| `claude-ask-error.jsonl` | The MCP rejected the request: the `tool_result` has `is_error: true`. |
| `claude-ask-nested.jsonl` | A `Task` subagent asks: both the `tool_use` and its `tool_result` carry top-level `parent_tool_use_id: "toolu-task"`. Must never become a card. |
| `codex-ask.jsonl` | The Codex pair for `hollow_questions`/`ask_user_questions`, thread `id-201`, item `item_1`. |
| `codex-ask-turn2.jsonl` | A resumed turn of the same thread that reuses `item_1`: Codex item ids restart on every exec, so ids are keyed per run. |
| `codex-ask-failed.jsonl` | `status: failed` with `error.message`. |
| `codex-ask-rejected.jsonl` | `status: completed` but the result text is the MCP's own rejection, not `Posted …` (how an MCP-level `isError` would surface if Codex reports it as completed; not exercised live). |

Replace the synthetic files with a consented capture of a real
`ask_user_questions` call if one is ever made.
