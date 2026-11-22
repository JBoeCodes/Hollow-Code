# Side chat

A second chat beside the one on screen, for a quick question about it that must not disturb its run
(t3code parity 7.6, 2026-11-20). Code: `src-tauri/src/side_chat.rs`, `src/store/sideChat.ts`,
`src/components/SideChatPane.tsx`, mounted by `src/pages/Work.tsx`.

## What the person sees

- The Side chat button beside a chat's name, Actions for this conversation → Open side chat, or ⌘K → Toggle side
  chat opens a pane in the desk's second column. Close side chat (the pane's ×, the same button, or Esc) puts it
  away. There is no chord. A narrow window gives the pane the whole desk. The panel slides for the Panel animation
  duration (Settings, General) and is static under reduced motion.
- It is for a project chat (a project's lead included) or a chat in Chats. The COO keeps its own page, a chat on
  another Mac or an offline copy has none, and a side chat has no side chat.
- A reply has Add to main chat: it puts the reply in the main chat's message box as a draft. Nothing is sent, and
  no row is added to the main transcript, until the person presses Send there.
- New side chat starts another; Earlier side chats lists the main chat's own. Side chats are not rows of the
  sidebar, Conversation history, search or Jump.
- Same access as the main chat is off by default.

## Rules the code holds

1. **A chat of its own.** `sessions.fork_kind = 'side'`, `parent_key` the main chat. Its turns, run ledger rows,
   budgets and usage are its own, so it counts like any chat. It is not a subagent: none of the subagent rules
   apply.
2. **Seeded, not shared.** The first turn sends `continued_from = [parent]` (the Continue path), so the context
   compiler bundles the parent's recorded turns and task state into the side chat's fresh native session. Later
   turns resume the side chat's own native session. The parent is only read, and its native session is never
   resumed.
3. **Never stronger than the parent.** The mode is at most the parent's last recorded mode and `plan` (read only)
   unless Same access is on. `side_chat::clamp_admitted` applies this at queue admission to every send into a side
   chat, whoever sent it (`hollow_app`, the phone, a held send). The agent is the parent's own.
4. **Isolated in the window.** `store/sideChat.ts` reads only `chat-event`s tagged with its own conversation key;
   `chatRouting.routeEvent` files those as `orphan` for the main runtime. Approval cards bound to the side chat
   show in the pane.

## Commands

`side_chat_open(parentKey, fresh)` returns the main chat's newest side chat (a new one when none, or when `fresh`
and the newest has a turn), `side_chat_list(parentKey)` its earlier ones, `side_chat_send(key, prompt,
clientTurnId, sameAccess)` queues a question like any chat turn. Open and send are the person's click.

## Limits

The seed is the parent as of the side chat's first turn. No refresh-context button yet. A question card the side
chat asks is answered from Waiting on you. Not offered from Conversation history.
