# Chat, composer and conversations

Part of the agent manual; [AGENTS.md](../../AGENTS.md) is the entry point. State model: [frontend state](../reference/frontend-state.md).

## Composer and drafts

- Composer drafts belong to conversation keys; each project has a separate New chat slot. The first accepted send transfers unsent follow-ups and clears New chat atomically. Every write requires the exact workspace/cwd/key owner; empty composers clear their slot.
- Retention is 100 drafts or one million characters; oversized text stays whole in memory with an unsaved notice and is excluded from the persisted budget, so it cannot evict other drafts. Attachment bytes stay window-only.
- Workspace draft actions await conversation ownership and recheck identity before appending ([Workspace draft ownership](../reference/frontend-state.md#workspace-draft-ownership)).
- Both editors (plain and the notes Markdown editor) expose source offsets through `composerField.ts`. Completion preserves text on both sides of the caret; Escape only dismisses suggestions. File references stay Markdown in the draft store.
- The composer bar has one control per job: + (attach, add to Sources, commands), Dictate, the model menu (model, effort, Permissions, Canvases; red in Bypass), Context (connectors included), usage and Send. Put a new setting in one of these menus, not a new button. The bar is the `composer-bar` size container; controls clip their own text.
- A file dropped anywhere on the window attaches to the chat (`windowFileDrop.ts`): window listeners leave a drop an inner target took; detect file drags by `dataTransfer.types` ("Files").

## Transcript and replies

- Proposed-plan cards need an explicit provider proposal event and a finished Plan-mode result; never infer them from prose. Accept/Refine only prepare drafts.
- Thinking disclosures show only completed, non-redacted provider text, stored separately from the answer and excluded from context reconstruction.
- Reply math and alerts use `replyRichText.ts`: bounded, untrusted KaTeX converted to native MathML with a narrow tag allowlist. Keep MarkdownView sanitization and remote-image consent in front of the DOM. Conversation chips open only `hollow://conversation/<exact-key>`.
- Reply file and image decoration lives in lazy `ReplyMarkdown.tsx`: remote loading is explicit, local reads are capped, async results belong to their conversation, and code folding never truncates copied source. See [Chat rendering](../CANVAS.md#chat-rendering).
- HEIC attachments are converted on this Mac using byte signatures, bounded pixels and the shared 10 MB preview-output cap.
- Streaming stays cheap per delta: extend an open reply by store path (`setTurns(i, 'text', …)`), never a whole-array `setTurns(produce(...))`; the reading anchor (`chatReadingAnchor.ts`) measures nothing while following the bottom. Measure with `tests/perf`.
- Scrolling, the working SVG and the follow-the-end rule: [Chat motion](../CHAT_MOTION.md).
- `ThinkingRow`'s disclosure is classified in `src/inventoryMap.ts`.

## Conversation state and permissions

- Conversation permissions restore from that chat's saved choice or recorded turn mode. Native resume requires a known matching mode; history loads fail closed. Quota failures save a Plan fallback so omitted chats cannot resurrect broader modes. New chats use the one saved default.
- Pin, Snooze, Wake and Settle share revision-checked state in the session store; Settle uses the archive lifecycle and Undo. The COO has parity through `sessions.list_conversation_organization` and `sessions.organize_conversation`, both requiring revision and activity, with no exception.
- Automatic settlement is off by default, one-shot per conversation revision (`sessions.configure_conversation_settlement`), and rechecks every guard inside its write transaction. New activity or manual organization disables it; live, queued, paused, pinned and unanswered work is never settled.
- Chat state across navigation: [Chat state](../reference/frontend-state.md#chat-global-survives-navigation); queue ownership: [Workspace conversation and page context](../reference/frontend-state.md#workspace-conversation-and-page-context).

## Chat search

- Search indexes scrubbed Hollow transcripts through `session_search.rs` (session store child module); sidebar, palette, agent and phone searches share it. Production calls `search_sessions_ranked`.
- The index is derived: source changes mark the changed turn (or deleted chat) transactionally, and a search reindexes at most 128 marked documents before reading, leaving the rest to a background drain.
- Never index tool inputs or write a search fixture into the live store. Replay checks open only their own snapshot write-capable; source databases stay read-only.
- The phone shows a chat only when its workspace is on `companion_docs::shown_on_devices` (an allow-list; `linear-*` never). See [chat search](../CHAT_SEARCH.md).

## File links and the Preview pane

- Every file a chat names (chips, local links, images, Result chips, run-detail rows) opens in the Preview pane (`openPreview`).
- Open a named file through `store/fileOpen.ts` `openNamedFile`: it reads the link against every folder the chat works in (`fileLocate.ts`) and asks `chat_locate_file` which holds it; never resolve against `chatCwd()` alone. A relative path no folder holds is searched under every project and the vault (`chat_find_file`): one match opens, several offer a picker, none gives a toast. An absolute path never opens another file of its name.
- Reply file links render as chips (`fileChip.ts` `fileKind`, `ReplyMarkdown` `chipify`). Agents are told to write vault-relative or absolute paths for files outside their folder (`visual_prompt.rs` `FILE_LINKS`).
