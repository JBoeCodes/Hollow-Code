# Ask your vault (overnight 2026-10-10/11, A6, to 2026-11-03)

Ask your vault was a modal view: a question got a cited answer from the person's own notes, in a thread, with the
exact passages beside it. **One Ask (2026-11-03) removed the view**: every question is a chat. This file is kept
because links point here. (The Settings switch that gives chats Hollow's read-only vault reader, `vaultMcpEnabled`,
keeps its name; see [agent tools](reference/agent-tools.md#ask-your-vault-read-only-search-and-readers).)

## Where asking lives now

- **Your work → Ask anything**, the one Ask: **New chat** or **Across all my work** (the Main agent). A chat answers
  from the notes with the same retrieval broker the view used (a chat's automatic retrieval, and the vault reader
  while it is on). See [ASK_HOLLOW.md](ASK_HOLLOW.md#where-asking-lives-now) for the command bar's Ask row.
- `g v` and the palette's **Ask anything…** keep this view's ids (`ask-vault`, `act:ask-vault`, the agent's
  `navigate` page `ask-vault`) and open Your work with the field focused.
- A saved **Answer from my notes** choice (`hollow.ask.lastScope` = `notes`) now starts on New chat.

## Save as note, in a chat

A chat reply's **More actions → Save as note** (`components/TurnFooter.tsx`, `store/replyNote.ts`; where and how it
is named: `turnFooter.ts` `replyNotePath`) saves that reply's Markdown as a new note through `vault_create_note`:

- in a workspace's conversation, into that workspace's `outputs/answers/`, so it is one of its Results (where this
  view's Save as note put a thread);
- in a chat in Chats, the Main agent's conversation, or one outside any workspace, into the Library's `library/`.

The file is `<date>-<slug>.md` from the question (else the reply's first line), and never overwrites anything (`-2`,
`-3`…). The note is the title as a heading, a "Saved from a chat reply" line, then the reply. A toast names the path;
its **Open** shows the note in the Preview pane.

## Citations in a chat

A reply's file references are already links: an inline-code path (`src/app.ts:42`, `notes/plan.md`) or a Markdown
link opens that file in the Preview pane (`components/ReplyMarkdown.tsx`, `chatFileLinks.ts` `fileHrefFromCode` and
`fileLinkFromHref`). Since S1b (2026-11-03) a cited line opens at that line: `path:12`, `path:12:3` (the column is
dropped), `path:12-20`, `path#L12` and `path#L12-L20` pass `line` and `end` to `openPreview`, and the pane shows the
file's source with that range marked and scrolled to the middle (`PreviewSourceLines.tsx`, as for a Run record's
Cites). An inline-code path keeps its lines in its `data-file-ref` as a `#L12-20` fragment (the text shown is
unchanged); a link with no line opens the file at its top, as before. Tool chips and reply-footer chips carry no lines.

## The backend (unchanged)

`src-tauri/src/vault_answer.rs` and its user-only commands `vault_answer_ask` / `vault_answer_save` stay registered;
no screen calls them any more. Retrieval was `retrieval_broker::retrieve_by`, unscoped, the first 10 ranked passages
(the matching line ±3 lines), answered by the default agent tool-free (150 s, metered `vault_answer`) with checked
`[n]` cites; a saved thread went to a workspace's `outputs/answers/`.
