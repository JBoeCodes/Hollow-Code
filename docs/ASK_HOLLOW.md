# Ask Hollow

Overnight 10-11 A2 to 2026-11-03. Ask Hollow was a dialog: a sentence typed in ⌘K became a plan of Hollow actions the
person previewed and ran, or a short answer with its sources. **One Ask (2026-11-03) removed the dialog**: every
question is a chat, asked from Your work's Ask field. This file is kept because links point here.

## Where asking lives now

- **Your work → Ask anything** (`components/StudioHome.tsx`, scopes in `src/askScopes.ts`, sending in
  `src/store/ask.ts`). Where to ask offers **Across all my work** (the Main agent, on its page; ⌥-Enter only puts the
  text in its box) and **New chat** (a new conversation in Chats; **Add to project** starts it in a project). A request that
  Ask Hollow would have planned goes to the Main agent, which runs Hollow's actions as tools in its own turn, with
  their cards and permission mode.
- **The command bar.** A query that reads as a sentence (three words or more, or a question of two;
  `looksLikeSentence`) gets one **Ask: “…”** row. It leads, unless the best row is a strong label match
  (`askRowIndex`), and Enter on it asks the text where Ask anything is set to send (`store/ask.ts` `askHere`). Its hint
  names that place as Home shows it (`askPlaceLabel`: "New chat", "New chat in <project>" when Add to project
  picked one, "Across all my work"), so it never sends into a project unseen.
- **Ask anything…** (the palette action, id `act:ask-vault`) and `g v` open Your work with the field focused
  (`openHomeAsk`, only once the navigation is accepted). From the palette, a typed sentence goes into the field when
  it is empty.
- A saved **Plan it in Hollow** choice (`hollow.ask.lastScope` = `hollow`) now starts on Across all my work.

## Past runs

Plans Ask Hollow ran are History runs with source `ask` (shown as **Ask Hollow**, `src/historyRun.ts`): Library →
History & recovery lists them, and **Undo** (`restore_history_run`) still restores what they changed. The backend is
unchanged and still registered (`src-tauri/src/ask_hollow.rs`, `ask_hollow_plan` / `ask_hollow_run`), so nothing a run
recorded is lost; no screen calls the two commands any more.

## How the backend works (unchanged)

1. **Catalog slice** (`pick_actions`): the core set (tasks, workspaces, sessions.start, search, people, agenda), then
   the registry's actions sharing the most words with the request, at most 48. Never an action that leaves this Mac (External, or a card-less tracker write in
   `app_actions::TRACKER_WRITES`), one on `app_actions::ALWAYS_ASK`, or a chat-only one (`CHAT_ONLY`).
2. **Planner** (`Planner`; `AgentPlanner` in the app): the person's default agent and saved model, tool-free, 120 s,
   metered `ask_hollow`. The reply is one JSON object: `read`, `plan` (≤ 12 steps), `answer` or `none`.
3. **Reads** run at once through the action handlers (read tier only), cut to 12,000 characters each; two read rounds
   at most.
4. **Checking** (`check_steps`): known ids, offerable, arguments valid, ≤ 12 steps; the preview line comes from the
   action's card.
5. **Kept plan**: stored in memory under `ask-<16 hex>`; a plan runs once.
6. **Run** (`run`): one full-vault History run (`try_begin_owned(vault, "ask", id)`), steps in order through
   `app_broker::execute` as caller `{surface: ask, perm: acceptEdits, depth 0}`. Run steps append digest-only
   `.app-actions.jsonl` audit rows with their History run id.
