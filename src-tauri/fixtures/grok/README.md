# Grok fixtures

Recorded 2026-09-26 from grok CLI 1.0.41 (`grok -p … --output-format
streaming-messages-json --permission-mode plan`), signed in with the user's
SuperGrok OAuth login (`apiKeySource: "oauth"`). The format is Claude Code's
stream-json: `system/init`, `assistant`, `user` (tool results), `result` with
`usage`, `modelUsage` and `session_id`.

- `oneshot.jsonl`: "Reply with the single word OK."
- `tool-call.jsonl`: one `read_file` tool call and its result, with thinking blocks.
- `partial.jsonl`: `--include-partial-messages` (chat streaming): Anthropic
  `stream_event` deltas (thinking and text), then the whole message.
- `resume.jsonl`: `--resume <session id>` of the one-shot session; same id.

Edited only to remove the user's MCP servers, MCP tool names, skills and
slash commands from `system/init`,
and the temp folder path.
- `resume-missing.stderr`: `--resume` of an unknown session id: exit 1, no
  stdout, this stderr (the wording `result_kind` matches).
- `resume-sandbox-mismatch.stderr` (1.0.46, 2026-10-02): `--resume` of a
  session started without a sandbox, with `--sandbox workspace`: exit 1, no
  stdout, this stderr. A session's profile is fixed when it starts;
  `--fork-session` is refused the same way.
- `resume-sandbox-to-off.stderr` (1.0.46, 2026-10-02): the other direction,
  `--resume` of a session started with `--sandbox read-only`, with
  `--sandbox off` (what Hollow passes for auto and bypass): exit 1, no stdout,
  this stderr. Recorded free (a made-up `--model`; the profile check comes
  first). As the message says, omitting `--sandbox` would resume it as `read-only`.
