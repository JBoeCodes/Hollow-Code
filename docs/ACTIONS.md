# Shared actions

Generated from `src/actionDefs.ts` by `node scripts/route-ledger.mjs`; `src/routeLedger.test.ts` fails while this file is
stale. One definition per action a person can run from more than one place. The palette, the `g` go-to keys, the ⌘
shortcuts, sidebar creation and the macOS menu bar all run it through one dispatcher (`src/actionRun.ts`), so its
behaviour, its condition and its words are set once. Its visible click path and proof are its row in the
[route ledger](ROUTE_LEDGER.md).

## Effects

| Effect | Means |
|---|---|
| `navigate` | Goes somewhere; changes nothing |
| `view` | Shows or hides something; changes nothing |
| `draft` | Opens a form or message box; nothing is saved or sent until you do |
| `write` | Changes files or settings on this Mac |
| `run` | Starts or stops agent work, which may cost money |
| `external` | Sends something outside this Mac |

“Confirms” marks an action whose effect waits for a preview or a confirmation you answer.

## Conditions

| Condition | Offered | Otherwise the person is told |
|---|---|---|
| `running-turn` | While a conversation is running a turn | Nothing is running in this conversation. |
| `chat-action-undo` | For 5 seconds after a chat is settled, snoozed, pinned or woken | Nothing to undo: Undo shows for 5 seconds after Settle, Snooze, Pin or Wake. |
| `workspace-open` | While a workspace is open | Open a project first: Projects → a project, or the sidebar. |
| `side-pane` | While the Preview pane shows something, has an item to bring back, or a workspace is open (its side panel card) | Open a workspace, a file or a link first: it shows in the Preview pane. |
| `can-leave` | Unless a message is still being sent | Wait a moment: the current message is still being sent. |
| `build-workspace` | In a build workspace (one with its own git worktree) | Changes are shown only in a build workspace, one with its own git worktree. |
| `conversation-agent` | With an agent installed, in a conversation or workspace | Open a conversation first; models belong to the agent that answers it. No agent installed? Settings → Models. |
| `can-merge` | In a workspace you made, with nothing running | Open a workspace you made first. Chats and the COO can’t be merged, and nothing may be running. |
| `can-go-back` | After you have moved between pages | Nothing to go back to yet. |
| `can-go-forward` | After going back | Nothing to go forward to. |
| `recent-saved-canvas` | After using a workspace canvas | Open a workspace canvas first. |
| `closed-view` | After a Preview pane tab was closed | No closed tab to bring back yet. |
| `side-chat` | In a project chat or Chats (never the COO), or while a side chat is open | Open an ordinary chat first: a side chat sits beside a project chat or a chat in Chats. The COO keeps its own page. |

## Actions (60)

| Action | Effect | Condition | Ways in | Own words per surface | Ledger row (visible path) |
|---|---|---|---|---|---|
| New conversation (`new-conversation`) | draft | Always | palette `act:new-conversation`, key `new-conversation`, menu `new-conversation` | — | `create.conversation`: Projects → All projects → a project’s ⋯ → New conversation |
| New chat (`new-quick-chat`) | draft | Always | palette `act:new-quick-chat`, go `quick-chat`, key `new-quick-chat`, menu `new-quick-chat` | — | `create.quick-chat`: Sidebar → Quick chats → New quick chat (+) |
| New project (`new-workspace`) | draft | `can-leave` | palette `act:new-workspace`, key `new-workspace`, menu `new-workspace` | — | `create.workspace`: Sidebar → Projects + |
| Jump to a workspace or chat (`jump`) | view | Always | palette `act:jump`, go `jump`, key `jump` | — | `find.search`: Top bar → Search chats → Search everything |
| COO (`main-drawer`) | navigate | Always | key `main-drawer`, menu `main-drawer` | — | `ask.main-agent`: Top bar → COO (the orb) |
| Open the COO (`main-agent`) | navigate | Always | palette `act:main-agent`, go `main-agent`, key `main-agent` | palette: “Open the COO” | `ask.main-agent`: Top bar → COO (the orb) |
| Dictate into the message box (`dictate`) | draft | Always | palette `act:dictate`, key `dictate`, menu `dictate` | key: “Dictate (tap, or hold and release)”; menu: “Dictate” | `ask.dictate`: A conversation’s composer → Dictate (microphone) |
| Ask anything… (`ask-vault`) | navigate | Always | palette `act:ask-vault`, go `ask-vault` | — | `ask.scoped`: Rail → Projects → Ask anything → Where to ask (across all my work: the COO, on its page; or New chat, a prompt that starts a quick chat, which Add to project sends to that project’s lead) |
| Open Running (Projects) (`activity`) | navigate | Always | palette `act:mission-control`, go `mission-control` | palette: “Open Running (Projects)” | `work.activity`: Rail → Projects → Running |
| Open Pull requests (Projects) (`pull-requests`) | navigate | Always | palette `act:pull-requests` | palette: “Open Pull requests (Projects)” | `work.pulls`: Rail → Projects → Pull requests |
| Open Canvases (Library) (`canvases`) | navigate | Always | palette `act:canvas-gallery`, go `canvas-gallery` | palette: “Open Canvases (Library)” | `library.canvases`: Rail → Library → All → Outputs → Grid → Canvases |
| Library: All (`library-all`) | navigate | Always | palette `act:library-all` | — | `library.all`: Rail → Library → All |
| Library: Notes (`library-notes`) | navigate | Always | palette `act:library-notes` | — | `library.notes`: Rail → Library → All → Browse folders |
| Library: Results (`library-results`) | navigate | Always | palette `act:library-results` | — | `library.results`: Rail → Library → All → Outputs → Grid |
| Library: Recordings (`library-recordings`) | navigate | Always | palette `act:library-recordings` | — | `library.recordings`: Rail → Library → All → Meetings → Recordings |
| Library: Skills (`library-skills`) | navigate | Always | palette `act:library-skills` | — | `library.skills`: Rail → Library → Skills |
| Library: History (`library-history`) | navigate | Always | palette `act:library-history` | — | `library.history`: Rail → Library → History |
| Resolve conflict copies… (`conflict-copies`) | write, confirms | Always | palette `act:conflict-copies` | — | `review.conflict-copies`: Bell → See all → Waiting → Decisions → the conflict-copies item |
| Export canvas… (`save-canvas-image`) | draft | `recent-saved-canvas` | palette `act:save-canvas-image` | — | `canvas.save-image`: A workspace canvas’s ⋯ → Export canvas… |
| New note (`new-note`) | draft | Always | palette `act:new-note`, key `new-note`, menu `new-note` | — | `library.new-note`: Rail → Library → All → New note |
| Add a task, note, link, file or recording… (`add`) | draft | Always | palette `act:add` | — | `tasks.add`: Library → Library actions → Add a task, note, link, file or recording… |
| New task (`new-task`) | draft | Always | palette `act:new-task`, key `new-task`, menu `new-task` | menu: “New Task…” | `tasks.new`: Projects → Tasks → New local task |
| Quick capture task (`quick-capture`) | draft | Always | key `quick-capture` | — | `tasks.add`: Library → Library actions → Add a task, note, link, file or recording… |
| Import a source (`quick-ingest`) | draft | Always | palette `act:quick-ingest`, key `quick-ingest` | key: “Import a source” | `library.import`: Rail → Library → Library actions (⋯) → Import a source |
| Manage connectors (`connectors`) | navigate | Always | palette `act:connectors` | — | `settings.connectors`: Rail → Settings → Connectors |
| Theme and appearance (`appearance`) | navigate | Always | palette `act:open-appearance` | — | `settings.appearance`: Rail → Settings → General |
| Toggle sidebar (`toggle-sidebar`) | view | Always | palette `act:toggle-sidebar`, key `toggle-sidebar`, menu `toggle-sidebar` | — | `nav.sidebar`: Top bar → sidebar button |
| Keyboard shortcuts (`shortcuts`) | view | Always | palette `act:shortcuts`, key `shortcuts`, menu `shortcuts` | key: “Keyboard shortcuts card” | `settings.keys`: Rail → Settings → Keyboard shortcuts → Show all keys |
| Save diagnostics… (`save-diagnostics`) | write, confirms | Always | palette `act:save-diagnostics` | — | `settings.diagnostics`: Rail → Settings → Advanced → Diagnostics → Save diagnostics… / Clear debug logs |
| Export settings… (`export-settings`) | write, confirms | Always | palette `act:export-settings` | — | `settings.transfer`: Rail → Settings → Advanced → Settings file → Export settings… / Import settings… |
| Import settings… (`import-settings`) | write, confirms | Always | palette `act:import-settings` | — | `settings.transfer`: Rail → Settings → Advanced → Settings file → Export settings… / Import settings… |
| Clear debug logs (`clear-debug-logs`) | write | Always | palette `act:clear-debug-logs` | — | `settings.diagnostics`: Rail → Settings → Advanced → Diagnostics → Save diagnostics… / Clear debug logs |
| Stop the running turn (`stop-run`) | run | `running-turn` | palette `act:stop-run`, key `stop-run`, menu `stop-run` | menu: “Stop Running Turn” | `conversation.stop`: A running conversation’s composer → Stop |
| Undo the last chat action (`undo-chat-action`) | write | `chat-action-undo` | palette `act:undo-chat-action`, menu `undo-chat-action` | menu: “Undo Chat Action” | `conversation.menu`: An open conversation → the ⋯ beside its name above it |
| Switch model… (`switch-model`) | view | `conversation-agent` | palette `act:switch-model` | — | `conversation.model`: A conversation’s composer → the model button |
| Open Changes (`open-changes`) | view | `build-workspace` | palette `act:open-changes` | — | `workspace.changes`: Open a build workspace → its branch in the top bar → the Changes tab in the Preview pane |
| Open project files (`open-project-files`) | view | `workspace-open` | palette `act:open-project-files` | — | `workspace.files`: Open a workspace → top bar Project files → the Preview pane’s Files tab (its Files picker pins the notes) |
| Toggle preview pane (`toggle-side-pane`) | view | `side-pane` | palette `act:toggle-side-pane`, key `toggle-side-pane`, menu `toggle-side-pane` | — | `preview.pane`: Click a file or a link (a reply’s changed-file chips and links, a tool’s file, a citation) → the Preview pane opens on the right; × hides it |
| Reopen closed view (`reopen-closed-view`) | view | `closed-view` | palette `act:reopen-closed-view`, menu `reopen-closed-view` | — | `preview.pane`: Click a file or a link (a reply’s changed-file chips and links, a tool’s file, a citation) → the Preview pane opens on the right; × hides it |
| Toggle side chat (`toggle-side-chat`) | view | `side-chat` | palette `act:toggle-side-chat` | — | `conversation.side-chat`: A conversation → the Side chat button beside its name (Close side chat, or Esc, closes it) |
| Merge this workspace into… (`merge-workspace`) | write, confirms | `can-merge` | palette `act:merge-workspace` | — | `workspace.menu`: The workspace’s ⋯ (Home row or workspace header) |
| Previous conversation (`prev-conversation`) | navigate | Always | palette `act:prev-conversation`, key `prev-conversation`, menu `prev-conversation` | — | `conversation.switch`: Sidebar → a project row (its lead) → the chat summary’s Subagents section → its Subagents chip → Earlier chats → a row (read it, or message it from its composer) |
| Next conversation (`next-conversation`) | navigate | Always | palette `act:next-conversation`, key `next-conversation`, menu `next-conversation` | — | `conversation.switch`: Sidebar → a project row (its lead) → the chat summary’s Subagents section → its Subagents chip → Earlier chats → a row (read it, or message it from its composer) |
| New project from this chat (`new-workspace-from-chat`) | draft | Always | palette `act:new-workspace-from-chat` | — | `conversation.menu`: An open conversation → the ⋯ beside its name above it |
| Back (`go-back`) | navigate | `can-go-back` | palette `act:nav-back`, key `go-back`, menu `go-back` | — | `nav.back-forward`: Top bar → ← / → |
| Forward (`go-forward`) | navigate | `can-go-forward` | palette `act:nav-forward`, key `go-forward`, menu `go-forward` | — | `nav.back-forward`: Top bar → ← / → |
| Projects (`go-work`) | navigate | Always | go `home`, key `go-work`, menu `go-work` | — | `work.resume`: Rail → Projects → Home → Recent projects, or All projects → Find → a row |
| Tasks (`go-tasks`) | navigate | Always | go `tasks`, key `go-tasks`, menu `go-tasks` | — | `tasks.list`: Rail → Tasks |
| Library (`go-library`) | navigate | Always | go `library`, key `go-library`, menu `go-library` | — | `library.all`: Rail → Library → All |
| Automations (`go-automations`) | navigate | Always | go `automations`, key `go-automations`, menu `go-automations` | — | `automations.list`: Rail → Automations |
| Needs you (`review`) | navigate | Always | go `waiting`, key `go-review`, menu `go-review` | — | `review.waiting`: Bell → Needs you → See all → All |
| Conversation history (`history`) | navigate | Always | go `history` | — | `library.conversations`: Rail → Library → Conversations |
| Usage (`usage`) | navigate | Always | go `usage` | — | `settings.usage`: Rail → Settings → Usage (opens on Limits) |
| Settings (`settings`) | navigate | Always | go `settings`, key `settings`, menu `settings` | menu: “Settings…” | `settings.page`: Rail → Settings |
| Workspace conversation (`workspace-conversation`) | navigate | Always | key `workspace-conversation` | — | `workspace.back-to-conversation`: Sidebar → the project row (it opens the project’s lead, L8) |
| Command palette (`search`) | view | Always | key `search`, menu `search` | menu: “Search…” | `find.search`: Top bar → Search chats → Search everything |
| Command palette (second key) (`search-alt`) | view | Always | key `search-alt` | — | `find.search`: Top bar → Search chats → Search everything |
| Search chats (`search-chats`) | view | Always | key `search-chats` | — | `find.search`: Top bar → Search chats → Search everything |
| Diagnostics (`diagnostics`) | navigate | Always | menu `diagnostics` | — | `settings.diagnostics`: Rail → Settings → Advanced → Diagnostics → Save diagnostics… / Clear debug logs |
| Copy build identity (version, commit) (`copy-build`) | view | Always | palette `act:copy-build`, menu `copy-build` | palette: “Copy build identity (version, commit)” | `settings.build-identity`: Rail → Settings → Advanced → Diagnostics → Copy build identity |
