Sanitized, hand-written fixtures for `src-tauri/src/session_import/` (H5a).
No real chat history: every id, path and message here is made up.

- `claude_code_session.jsonl`: a Claude Code session with a `summary` title,
  a tool call and its result, two sidechain lines, one malformed line, an
  image and a last line cut off mid-write.
- `codex_rollout.jsonl`: a Codex rollout with `session_meta`, an injected
  environment context, a function call and its output, an `event_msg`
  duplicate, one malformed line and an image.
- `unknown_layout.jsonl`: JSON lines in no layout a reader knows.

H5b (ChatGPT export):

- `chatgpt_conversations.json`: a `conversations.json` array, hand-written.
  `conv-sample-0001` has a hidden system node, Custom Instructions context, an
  image and a named attachment, a null part, a code call with its tool
  output, and an edited-prompt branch (one other branch). `conv-sample-0002`
  has no `current_node` and no title, a null part and an unknown content
  type. Three elements that aren't conversations (a number, an object with
  no id, a string) follow. Tests build the export zips from it at run time.

H5b (claude.ai export):

- `claudeai_conversations.json`: a `conversations.json` array, hand-written.
  `...0001` has a named attachment (with a body that must never be imported),
  a file, a `thinking` block, a tool call and result, an unknown block type,
  an empty `content` list and a non-object message. `...0002` has no title,
  no block lists and a bad time. A conversation with no `chat_messages`, one
  with no `uuid`, a number and a string follow. Tests build the export zips
  (with a `projects.json` and `users.json` that must never be read) at run time.
