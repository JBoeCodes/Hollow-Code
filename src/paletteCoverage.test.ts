import { describe, expect, it } from 'vitest';
import runSource from './actionRun.ts?raw';
import { ACTION_DEFS, actionLabel } from './actionDefs';
// Settings' jump lives in its navigation controller (L4, X18).
import settingsSource from './settings/controllers/settingsNavigation.ts?raw';
import whatsNew from '../docs/WHATS_NEW.md?raw';
import { SETTINGS_SECTION_GROUPS } from './settingsSections';
import { paletteDefaultOrder } from './paletteDefault';

// Overnight G3: every feature in What's new that a person can trigger is one
// palette search away, by a palette action (`act:…`) or a Settings jump
// (`section:Control`, the palette's Settings rows). Anything else says where
// it lives instead: a card that appears on its own, agents only, or nothing to
// trigger. A new What's new heading fails here until it is placed.
type Reach = { palette?: string[]; settings?: string[]; elsewhere?: string };

const COVERAGE: Record<string, Reach> = {
  'Pull requests and issues for a project': { palette: ['act:pull-requests'] },
  'Give a chat a scratch folder and turn it into a project': { elsewhere: 'A quick chat\u2019s menu (sidebar row, Library \u2192 Conversations, the \u22ef beside its title) \u2192 Scratch folder, New project from this chat; the palette\u2019s New project from this chat' },
  'See projects by their letters and color': { elsewhere: 'Sidebar \u2192 a project\u2019s menu \u2192 Change color, Reset color' },
  'Clone a repository from an address': { elsewhere: 'Projects \u2192 New project \u2192 Templates and location \u2192 Repository worktree \u2192 Clone from an address' },

  'Run named scripts for a project': { elsewhere: 'The card beside a chat \u2192 Scripts \u2192 Add script, Run, Edit; the script\u2019s \u21e7\u2318 key from anywhere in the project' },

  'Ask a quick question in a side chat': { palette: ['act:toggle-side-chat'] },
  'Reopen a closed tab in the Preview pane': { palette: ['act:reopen-closed-view'] },
  'Swipe to go back and forward': { elsewhere: 'Two fingers swiping sideways on the trackpad; the Back and Forward buttons and ⌘[ ⌘] stay; Settings → Keyboard shortcuts → Swipe to go back and forward' },
  'See where each shortcut works': { elsewhere: 'Settings → Keyboard shortcuts: a context chip on a row, and the In the palette list' },

  'Import a VS Code theme and choose your fonts': { elsewhere: 'Settings → General → Appearance → Theme → Edit themes… → Import VS Code theme…; Text font and Code font beside Text size' },
  "Regenerate a chat's title": { elsewhere: 'A conversation menu (sidebar row actions, or the dots beside its title) → Regenerate title' },
  'Read secrets from 1Password instead of the Keychain': { settings: ['accounts:1Password'] },
  'Update every agent at once': { elsewhere: 'Settings → Accounts → Agent updates → Update all' },
  'Settings for diffs, links, worktrees and archived chats': { elsewhere: 'Settings → Diffs, Browser, Worktrees and Archived' },



  'Run actions and pick rows by number in the command palette': { elsewhere: 'Command palette (⌘K) → > or the Actions chip; ⌘1 to ⌘9 while it is open; Back button or Backspace' },
  'Antigravity can use Hollow\'s tools and your shared connectors': { elsewhere: 'Settings, Connectors: the Antigravity toggle on a connector; the chat connector menu and tool cards in an Antigravity chat' },
  'Antigravity asks before it acts, and Auto works': { elsewhere: 'Composer permission menu (Antigravity: Accept edits, Auto) and the chat approval card for an Antigravity command' },
  'Antigravity shows usage, context and estimated spend': { elsewhere: 'Composer usage ring (Antigravity), Settings, Usage and Budgets' },
  'One effort control for Antigravity': { elsewhere: 'Composer model menu, Antigravity: one row per model, and the Effort row lists only the levels that model takes' },

  'Motion on running tools and the preview loader': { settings: ['general:Panel animation'] },
  'Import Pi history and pair your phone from welcome': { settings: ['about:Welcome to Hollow'] },
  'One welcome page for Hollow': { settings: ['about:Welcome to Hollow'] },
  'Drag conversations between sidebar sections': { elsewhere: 'Sidebar: drag a local chat row onto Pinned, Active or Settled; a conversation menu (right-click or the actions button on its row) \u2192 Pin, Unpin, Settle do the same by click' },
  'See row details and drop files onto a chat': { elsewhere: 'Sidebar: hover or focus a local chat row, or its menu \u2192 Details (Close details dismisses it); drop files on the row to attach them to that chat\u2019s composer' },
  'Shrink the Preview pane to a mini player': { elsewhere: 'Preview pane header \u2192 Mini player, Expand, Close; the agent cursor follows an authorized inspection of a shared tab' },
  'Accept or refine a reported plan': { elsewhere: 'Conversation: explicit provider proposal card in a completed Plan-mode reply' },
  'Expand images and attach HEIC photos': { elsewhere: 'Conversation: local/attached image click, zoom controls and Open in Preview; composer and phone attachment upload' },
  'Read math, alerts and linked conversations in replies': { elsewhere: 'Conversation: rendered reply math, alert blocks and conversation link chips' },
  'See context handoff preparation': { elsewhere: 'Conversation: context handoff divider details during a fresh follow-up or retry' },
  'Read reported thinking when available': { elsewhere: 'Conversation: Thinking disclosure, including expanded finished run steps' },

  'Stash a draft or send in background': { elsewhere: 'Message box → + → Stash this draft / Restore draft / Send in background; project empty hero → or start without a project' },
  'Write messages with live Markdown': { elsewhere: 'Message box → + → Use live Markdown / Use plain text' },
  'Live work time and quieter warnings': { elsewhere: 'Conversation: live elapsed header and clickable warning disclosures' },
  'Mention files and skills while writing': { elsewhere: 'Message box → + → Mention a file or Use a skill' },
  'Calmer loading and measured disclosures': { elsewhere: 'Settings help and search targets, Library All, sidebar Search chats and existing drag destinations; the panel-duration setting disables motion' },
  'Computer control: a settings sheet, chat controls and a Doctor': { settings: ['connectors:Computer control'] },
  'Agents can block a site for themselves, and only you can unblock it': { settings: ['connectors:Computer control'] },
  "Undo an agent's turn in a build chat": { elsewhere: 'A reply in a chat with its own worktree → More (the dots) → Restore files to before this turn' },
  'Decision cards show a scenario and what each option does': { elsewhere: 'The question card over the message box when an agent asks you to decide' },
  'See how well each shared skill is written': { elsewhere: 'Library → Skills → the Lint score on a shared skill; open it for its Skill check' },
  'Select chats, sort, filter, group and saved views': { elsewhere: 'Sidebar → Quick chats heading → Select chats or View (sort, show, group, saved views)' },
  'Subagent chats under their lead, and rename in place': { elsewhere: 'Sidebar → a chat or project row: the count button shows its subagent chats; double-click the row or its ⋯ → Rename…' },
  'Status marks on chat rows in the sidebar': { elsewhere: 'Sidebar → a chat row: Approval, Input, Plan, an unread dot and its pull request' },
  'Live timers and bottom shelves in the sidebar': { elsewhere: 'Sidebar → Working, Snoozed and Settled shelves; a running row shows its time' },
  'Snooze with quick times, and undo Pin and Wake': { elsewhere: 'Conversation actions → Snooze until… (quick times and a picker); Edit → Undo Chat Action, or the notice’s Undo' },
  'Pin, snooze and settle conversations': { elsewhere: 'Conversation actions → Pin, Snooze until, Wake or Settle; sidebar organized folds' },
  'Fold background work in the sidebar': { elsewhere: 'Sidebar → Working under projects or Quick chats' },
  "Leave notes on a file's lines and send them as one message": { elsewhere: 'A text or Markdown file in the Preview pane → Review notes → Add a note on a line → Send N notes to chat' },
  'Open sidebar actions with a click': { elsewhere: 'Sidebar → project or quick-chat row → actions button' },
  'Keep working on a moved chat while its Mac is away': { elsewhere: 'A moved chat’s notice above the composer → Continue here offline…' },
  'Move a chat to your runner Mac': { settings: ['devices:Move running chats to the runner when this Mac sleeps'] },
  "Choose whether this Mac's COO sends long work to your runner": { settings: ['devices:COO on this Mac'] },
  'Open a chat on your Mac mini': { elsewhere: 'Settings → Devices → Macs → Pair a Mac; the sidebar’s machine strip → a Mac → a chat' },
  "Choose between two Macs' versions of a file": { elsewhere: 'Waiting on you → Decisions → Two Macs changed <file>; Settings → Sources → Context → N clashes' },
  'Quick chats are back': { elsewhere: 'Sidebar → Quick chats (its +), New chat or ⌘N with no project open' },
  'Check a repo for secrets before you push': { settings: ['sources:Context check'] },
  'See and connect your context layers': { settings: ['sources:Context'] },
  'Move Hollow home into one real folder': { settings: ['sources:Move Hollow home'] },
  'Branch, worktree and Commit and push from the chat card': { elsewhere: 'A chat in a git folder → summary card → the branch row (switch, New branch, New worktree) and Commit and push' },
  'Review changes by file or by turn in a Diff tab': { elsewhere: 'A chat in a git folder → summary card → Changes row opens the Diff tab; the Compare menu picks a turn' },
  'Run a terminal beside a chat': { elsewhere: 'A chat → summary card → Terminal, or the Preview pane\'s + (New tab) → Terminal' },
  'See a branch\'s pull request, checks and comments in the side pane': { elsewhere: 'A chat in a git folder → summary card → More git actions → Pull request' },
  'Servers and automations in the chat card': { elsewhere: 'A chat → summary card → Servers and Automations' },
  'Open a chat\'s folder in your editor from the card': { elsewhere: 'A chat → summary card → Open in (Finder and installed editors); Add project script in a project chat' },
  'See what subagents are doing': { elsewhere: 'A chat → summary card → Subagents' },
  'Subagents per chat is a setting, and waits name their limit': { settings: ['chat:Subagents running at once from one chat'] },
  "Each agent's access modes are its own": { elsewhere: 'A chat\u2019s access chip in the message box; Settings \u2192 Chat, new-chat mode per agent' },
  'One compact usage ring in the message box': { elsewhere: 'The usage ring and Folder and connectors in a chat\u2019s message box; Context details opens Settings \u2192 Advanced \u2192 Search & indexes' },
  'A second Linear team, Personal': { elsewhere: 'Settings \u2192 Task tracker; Tasks \u2192 team filter' },
  'Library is one list': { palette: ['act:library-notes', 'act:library-results', 'act:library-recordings'] },
  'Side panel files by name, and every reply says when it finished': { elsewhere: 'A chat → summary card → Outputs and Sources; the row under each reply; Subagents tab → a subagent → its message box' },

  'Small follow-ups start a fresh subagent with a brief': { elsewhere: 'automatic for agents: a chat that started subagents → the Subagents chip → the Subagents tab, a row that says Fresh chat with a brief' },

  'The message box shows its access mode': { elsewhere: 'Any chat’s message box: the access chip beside the + (the agent’s own mode names); the model menu’s Permissions row' },

  'Issues open full page, like Linear': { elsewhere: 'Rail → Tasks → an issue → Open full page; Back to list or Escape returns' },
  'Settings fits on a screen': { settings: ['general:Appearance', 'connectors:Your connectors', 'shortcuts:Reset all to defaults'] },
  'Settings: one line each, and fifteen clutter rows gone': { settings: ['shortcuts:Show all keys', 'advanced:Runtime', 'advanced:Capability doctor', 'advanced:Diagnostics', 'sources:Hollow folder', 'chat:Permission mode for new chats'] },
  'Where everything in Settings went': { elsewhere: 'Settings page header: Where things went, shown until 2026-11-26' },
  'Settings has a new list': { settings: ['general:Run at login', 'chat:Chats running at once', 'accounts:Agent updates', 'updates:Channel', 'updates:Check now', 'sources:Task tracker', 'devices:Pair a device', 'advanced:This Mac'] },

  'Each setting in one place': { settings: ['models:Effort', 'usage:COO daily hard stop', 'usage:Overnight cap', 'personalization:How many learnings to suggest', 'connectors:Recordings'] },
  'Skills, History and profile suggestions moved to Library; windows and the run queue to Automations': { palette: ['act:library-skills', 'act:library-history'], settings: ['connectors:Skills are in Library', 'advanced:History & recovery is in Library', 'background:Automations, windows and the run queue are on the Automations page', 'personalization:Waiting for you'] },

  'Work is now Projects; the sidebar is shorter': { elsewhere: 'Sidebar → Projects (⌘1) and Recent; top bar → Search chats (⌘⇧K); Projects → All projects; the COO orb for a prompt with no project' },
  'Library has Skills and History tabs': { palette: ['act:library-skills', 'act:library-history'] },
  'Library opens on All, one searchable list': { palette: ['act:library-all'] },

  'A reply ends with its files, how long it worked, and a quiet row': { elsewhere: 'Any finished reply: the file chips and the Worked for line above it, and its row of Copy, Useful, Not useful, Read aloud, Retry, More actions and the time under it' },

  'Usage shows every agent\'s limits and detail': { settings: ['usage:Automatic account rotation', 'usage:Move on at', 'usage:Usage by day'] },
  'File links show the file\'s name and type': { elsewhere: 'Any chat reply: a file chip opens the file in the Preview pane; hover shows its path' },
  'One chat per project: its lead': { palette: ['act:new-quick-chat'] },
  'A project\'s files open beside the chat, Results with their review': { palette: ['act:open-project-files'] },
  'The Main agent is now the COO': { palette: ['act:main-agent'] },
  'Budgets for subagents and projects, and spend by where': { settings: ['usage:Budget for each subagent', 'usage:Project budgets'] },
  'See what a chat started, in a Subagents tab': { elsewhere: 'A chat that started subagents: the Subagents chip above its thread (or a Started a subagent line) opens the Subagents tab in the Preview pane; the COO page’s More holds its band’s links' },
  'The chat outline is a strip of dashes, as in ChatGPT': { elsewhere: 'A chat with two or more prompts → its left margin (the dashes), or Conversation tools → Outline' },
  'Settings search finds settings on other pages, and Library links to Search by meaning': { elsewhere: 'Rail → Settings → Find a setting; Library → Find a file → Turn on search by meaning; Rail → Settings → Advanced → Capability doctor' },

  'A banner and dock badge when a chat finishes or needs you': { settings: ['general:Chat announcements'] },

  'Panel animation: choose how long panels take to move': { settings: ['general:Panel animation'] },
  'A side panel card, and tabs like a browser': { elsewhere: 'An open chat → summary card; the Preview pane → + → New tab → Files / Browser / Subagents' },

  'Run an automation inside a chat': { elsewhere: 'A conversation’s ⋯ → Schedule in this chat…; Automations → Edit → Inputs & scope → Runs in' },
  'Notion sync is removed; Notion stays readable as an archive': { elsewhere: 'Rail → Tasks → Sync now; Library → Results → Canvases → a canvas → Canvas actions → Attach to issue…; Settings → Connectors → Your connectors → Notion (official) → Sign in' },
  'Automations that start on an event, and leads that wake for new issues': { elsewhere: 'Automations → New automation or Edit → When: Event; right-click a tracked project in the sidebar → Wake the lead for new issues' },
  'Hours saved count finished issues, and wait for one': { settings: ['usage:Value this month'] },
  'A Friday status for each tracked project': { elsewhere: 'Rail → Tasks → a project → Write status now; Settings → Sources → Task tracker → Friday status and Status copy' },
  'Attach a canvas to an issue': { elsewhere: 'Library → Results → Canvases → a canvas → Canvas actions → Attach to issue…' },
  'Comment on an issue from Hollow': { elsewhere: 'Rail → Tasks → an issue → Comment, under Activity' },
  'Upload your own icon, or have an agent draw one': { elsewhere: 'Settings → App and COO icons → Upload an icon or Generate a logo; COO → top bar Actions for COO → Change icon…' },
  'Conflicts in Tasks, in one list': { elsewhere: 'Rail → Tasks → Conflicts (n) (shown while an issue has a conflict) → an issue → Use mine or Keep Linear’s' },
  'A Projects tab on Tasks': { elsewhere: 'Rail → Tasks → Projects → a project' },
  'One project page, and Records in Library': { elsewhere: 'Rail → Tasks → an issue → Open project; Rail → Library → Library folder → Records → a project' },
  'Tasks has views and filters like your tracker': { elsewhere: 'Rail → Tasks: the view tabs (My issues, All, Active, Backlog, By project, By assignee), Filter, and Group and Order' },
  'Tasks looks like your tracker': { settings: ['sources:Task tracker'] },
  'Scheduled reports keep their setup and results': { elsewhere: 'Automations → Automation options → Scheduled report setup; saved reports and exact run selection remain in automation review' },
  'Read-only company reports keep their evidence': { elsewhere: 'Nothing to click: company work is paused and its screens were removed (11-10)' },
  'Company reports leave room for conversation': { elsewhere: 'Nothing to click: company work is paused and its screens were removed (11-10); Settings → Usage → Budgets still controls usage limits' },
  'Set up Hollow in five short steps': { settings: ['sources:Run setup again'] },
  'Review the instructions and memory found on this Mac': { elsewhere: 'Library Learnings: the Found context card at the top (Scan this Mac, then Accept, Change layer or Skip on one item)' },
  'Found Claude Code memories get a suggested layer and a reason': { elsewhere: 'Library Learnings: the Found context card, "Why this layer" on each Claude Code memory entry after Scan this Mac' },
  'Reusable lessons can become skill proposals': { elsewhere: 'Library Learnings and the source reply: Save or Reject a pending profile or skill lesson' },
  'Grounded Know-how saves with Undo': { elsewhere: 'Library Learnings: per-item Undo and explicitly opened project-scoped private memories; proposals wait for Save' },
  'Memory stays searchable as it ages': { elsewhere: 'Library Learnings Saved: usage, pin, restore and expiry; profile byte size and consolidation proposal above the list' },
  'Learnings live beside their source reply and in Library': { elsewhere: 'Library Learnings and the source reply line; Settings Memory links to the same home' },
  'Hollow looks for lessons while a chat rests': { elsewhere: 'Automatic after quiet time, long chats or handover; review in Needs you or Settings Memory' },

  'Search what agents said in past chats': { elsewhere: 'Sidebar Search chats, command palette conversation results and the agent conversation-search action' },

  'Tracked projects get a local workspace': { elsewhere: 'The tracked project workspace in the sidebar; generated TRACKER.md beside its brief and project note' },
  'A chat shows how long this kind of job usually takes': { elsewhere: 'On a reply: the Worked for line, or the time under the reply, and beside the live clock while it runs. Quiet until three finished chats of that kind' },
  'Tracker edits keep their provider details': { elsewhere: 'Existing task edit controls, paired phone task routes and tracker actions requested in chat; no new screen or shortcut' },
  'Votes, Routing and About these settings are gone': { elsewhere: 'A reply has Copy and More; Useful and Not useful came back to its row on 11-12 (C2), with no Votes list. Settings → Usage keeps Budgets. Settings → Advanced holds diagnostics, the build, search, databases and Prices. Each Settings section shows how it saves' },
  'History snapshots reread only what changed': { elsewhere: 'Nothing to click: once Hollow has snapshotted the vault since it opened, an agent turn stores History by rereading only the files that changed' },
  'Map, Runbooks, Tonight, Present and What\'s new are gone': { elsewhere: 'Rail → Work and Rail → Automations. Ask in chat' },
  'The tracker sync foundation': { elsewhere: 'Background foundation, off until configured. Task tracker settings and the rebuilt Tasks page follow; no new screen in this change' },
  'Not verified only when a reply claims it': { elsewhere: 'Under an agent reply: the line "Not verified" when the reply said done, fixed, or that its checks pass, and nothing confirmed it. The Run record still opens from that line' },
  'The Main agent\'s notes stay one quiet line': { elsewhere: 'The COO conversation, under a reply that edited a brief or a plan: the line "Updated my notes". The brief and the plan are where you read them' },
  'Bookmarks are gone from a conversation': { elsewhere: 'Any conversation: the Outline button above the thread lists your prompts. A reply has no bookmark' },
  'Question cards keep the question in view': { elsewhere: "A question card docked over a chat's message box: the line under each question, the selected row, and the answers in the transcript" },
  'An open question card stays open': { elsewhere: "An open question card docked over a chat's message box: it stays while a wake, check-in or scheduled turn runs" },
  'A new chat opens in the middle of the pane': { elsewhere: 'A new project conversation or a new chat in Chats: the greeting, the message box and the starters in the middle of the pane' },
  'Code blocks wrap, and tables copy': { elsewhere: 'Any code block in a reply: wrap beside copy in its header; any table in a reply: Copy table above it' },
  'A name above every chat': { elsewhere: 'Any project conversation or chat: the name above its thread, with its ⋯ and a double-click to rename; the sidebar row’s ⋯ and right-click Rename… do the same' },
  'Save a whole conversation as a note': { elsewhere: 'Actions for this conversation → Save conversation as a note, beside Copy conversation' },
  'Replies stream without redrawing what\'s done': { elsewhere: 'Any reply while it streams: select words in a finished paragraph, or use a table, as the rest arrives' },
  'Retry with another model': { elsewhere: 'The latest finished reply’s More actions → Retry with <model>; under a message that failed, the Retry with another model button' },
  'Quote selected text': { elsewhere: 'Any conversation or chat: select words in a reply or in your own message, then press Quote above the selection' },
  'Read a reply aloud': { elsewhere: 'Any finished reply: the Read aloud button in its row, which then says Stop reading' },
  'Consistent page headings': { elsewhere: 'Page headings throughout Hollow; text size in Settings → General' },
  'Quieter Results and empty views': { elsewhere: 'Library → Results, shared empty/loading views and buttons' },
  'Steadier toolbar and subtask controls': { elsewhere: 'The top bar and task editor subtask list' },
  'A quieter empty Tasks list': { elsewhere: 'Tasks, List view, with an empty or filtered list' },
  'Steadier chat controls': { elsewhere: 'Approval cards in chats and conversation history row actions' },
  'One header row in the Preview pane': { elsewhere: 'Click a file or a link in a chat: the Preview pane beside the page' },
  'Steadier filters and actions': { elsewhere: 'List filters, toast action buttons and close buttons in task and Add dialogs' },
  'Tidier project rows in the sidebar': { elsewhere: "The sidebar's Projects: a project row's icon and its + (the fold arrow and ⋯ it gained went on 11-07: click folds the open project, right-click is the menu)" },
  "Memory is checked before it's saved, and every change is logged": { elsewhere: 'The Learning suggestions and About you cards: the message when a save is refused; the change log has no screen until the Learnings tab' },
  'A saved learning goes to the one place it belongs': { elsewhere: 'The Learning suggestions cards: Save and Undo are the same buttons; the saved line appears in the About you note or the project brief it belongs to' },
  'Friction is no longer a page, and routine exit codes stop counting': { elsewhere: 'Nothing to click: the Friction section and its jump link left Settings → Usage; friction.list and friction.draft_fix stay as agent actions, the draft behind a card' },
  'One Main agent conversation': { elsewhere: "The COO button opens its one conversation; earlier chats are under Earlier in this conversation at the top of it, and a chat that continued shows Open" },
  'Today, Weekly review, Agenda, Incoming and Commitments are gone': { elsewhere: 'Rail → Work (Home and Running), Rail → Tasks (List or Board), the bell → Needs you, Settings → Connectors → Agenda calendar. The old screens have nothing to click' },
  'Agents do the work those screens used to': { elsewhere: 'Nothing to click: in a chat that can write, the agent adds a Plan row or a task, or asks with an approval card. People notes, the commitment ledger and what an install brings stay where they are' },
  'Work records stay on this Mac': { elsewhere: 'Nothing to trigger: Library records and a Linear project workspace are never copied to the phone backend. Settings → Devices → Companion devices → Backend still chooses personal or work data for everything else' },
  'Project rows: click to open and fold, right-click for the menu': { elsewhere: "The sidebar's Projects: click a project to open it, click the open one to hide or show its chats, right-click for its menu (the project's ⋯ on Work and in the top bar has the same items)" },
  "Choose your chat's send settings on iPhone": { settings: ['devices:iPhone app'] },
  'Tasks on your iPhone': { settings: ['devices:iPhone app'] },
  'Quick notes from your iPhone': { settings: ['devices:iPhone app'] },
  'Canvases on your iPhone': { settings: ['devices:iPhone app'] },
  'Browse your workspaces on iPhone': { settings: ['devices:iPhone app'] },
  'Search your chats and workspaces on iPhone': { settings: ['devices:iPhone app'] },
  'Keep Hollow running with the lid closed': { settings: ['devices:iPhone app'] },
  'Notifications and question cards on your iPhone': { settings: ['devices:iPhone app'] },
  'Your chats on your iPhone': { settings: ['devices:iPhone app'] },
  'Photos and files from your iPhone': { settings: ['devices:iPhone app'] },
  "Buttons on your iPhone's notifications": { settings: ['devices:iPhone app'] },
  "A running turn on your iPhone's lock screen": { settings: ['devices:iPhone app'] },
  'Files a chat names open in the side pane': { elsewhere: 'A file named in a reply (in backticks, a link or the prose), a tool row, a reply’s changed-file chip or its run details' },
  'Drop files anywhere in Hollow': { elsewhere: 'Drag a file from Finder over the window; the chat’s + → Attach files… is the click path' },
  'Question cards always send': { elsewhere: "The question cards docked over any chat's message box" },
  'Quick chats is now called Chats': { palette: ['act:new-quick-chat'] },
  'Long chats stay light, and show what is new': { elsewhere: 'Any long chat: Show earlier messages, Show later messages, Jump to latest; the new-messages line with Dismiss' },
  'Large task lists open fast': { elsewhere: 'Tasks → the end of a group with more than 60 rows → Show more' },
  'Cancel keeps your workspace result selection': { elsewhere: 'Result links in Library, Needs you, chat replies and run records; the existing draft prompt' },
  'Coming back to a chat lands at its end again': { elsewhere: 'Any workspace chat, coming back from its sidebar row or Work' },
  'Escape closes a menu first in a narrow window': { elsewhere: "The sidebar in a narrow window: a chat's ⋯, or a project's or chat's right-click menu" },
  'Long messages of yours fold behind Show more': { elsewhere: 'Any chat: Show more under a long message of yours' },
  'Notification history lives in the bell': { elsewhere: 'Bell → Notifications: filters, Details, Mark read, Delete and App messages; Settings → General → Clear history' },
  'Cancel keeps Library where you left it': { elsewhere: 'Canvases and Compare versions links into Library → Results; the existing unsaved-edit prompt' },
  'Attach any file to a message': { elsewhere: 'Chat composer → + → Attach files…, a drop or a paste; + → Add to Sources… in a workspace chat' },
  'Older chats load in the sidebar': { elsewhere: "The sidebar: Show older under a project or in Chats; a project's right-click menu or ⋯ on Work → Open in Conversation history" },
  'A quieter Map and empty Running view': { elsewhere: 'Work → Home → Projects → Map; Work → Running' },
  'Your work is now Work': { elsewhere: 'Sidebar → Work; native Go menu → Work; existing go-work action and shortcuts' },
  'Results lead the Library': { elsewhere: 'Library → All → Outputs → Grid (the Results preview grid); Notes and Recordings open from All too' },
  'Quieter Tasks and Automations': { elsewhere: 'Tasks and Automations: primary create actions, support disclosures and empty lists' },
  'A quieter top bar': { elsewhere: 'The COO orb (New chat), top bar → Search chats → Search everything, Recent +; Library → Library actions → Add…; Work → Running → Background tasks' },
  'Workspace documents beside your chat': { elsewhere: 'Gone with the workspace panel (COO rebuild L4): Project files in the top bar opens a project’s notes in the Preview pane' },
  'Your messages: a cleaner bubble, Edit and Retry': { elsewhere: "Any chat: hover over your message (time, Copy, Edit on the last one); the latest finished reply's footer (Retry) and its More menu" },
  'A long Projects list stays short': { elsewhere: "The sidebar → Projects: a project's arrow" },
  'Search your chats from the sidebar': { elsewhere: 'Top bar → Search chats (⌘⇧K), shown in the sidebar; its last row, Search everything, opens the palette' },
  'Every chat in the sidebar has a ⋯ menu': { elsewhere: "The sidebar: a chat row's ⋯ on hover (always on the open chat) or right-click; a project's menu is its right-click" },
  'Calmer Settings': { elsewhere: 'Settings: each section says how it saves; Interface: App and COO icons' },
  'The mic sits beside Send': { elsewhere: 'Every chat: the message box bottom row; the cost in the usage ring hover and its Usage card' },
  'The message box says where it goes': { elsewhere: 'Project, quick and COO chats: the message box placeholder and the Context (folder) button on the composer row' },
  "The sidebar works like ChatGPT's and Claude's": { elsewhere: 'The sidebar: New chat and Search on top, Tasks among the pages, Projects and Chats headings, its right edge' },
  'A quieter Work page': { elsewhere: 'Work: Home, Today, Running; Home projects: Map' },
  'Notifications have a bell': { elsewhere: 'Top bar → Notifications; Needs you: see all opens the full review view' },
  'Needs you is in the bell': { elsewhere: 'Top bar → Notifications → Needs you → See all needs you' },
  'Find marks the words it matched': { elsewhere: 'A conversation: ⌘F or the magnifier over the transcript' },
  'A self-hosted model can join a bake-off': { elsewhere: 'The bake-off runner was removed (11-10 S6); self-hosted models stay under Settings → Defaults' },
  'Self-hosted models on Codex': { settings: ['models:Self-hosted models'], palette: ['act:switch-model'] },
  'Background jobs on a self-hosted model': { settings: ['advanced:Background jobs'] },
  "A self-hosted model's usage reads billed by GPU time": { elsewhere: "A chat on a self-hosted model: the turn's usage and its hover note; Usage totals" },
  "A self-hosted model's chat runs on this Mac": { elsewhere: "Chats on a self-hosted model whose Runs on is the runner; automatic, the turn's first line says why" },
  'A self-hosted chat shows when its model is starting': { elsewhere: 'The line above the message box in a chat on a self-hosted model, with its Start now' },
  'App builds can compile on the remote runner': { settings: ['advanced:App builds'] },
  'Database tests can run on the Mac mini': { elsewhere: 'agents only: hollow-slot --postgres <command>' },
  'An automation runs on one Mac': { elsewhere: 'Automations → Edit → Inputs & scope → Runs on; agents: save_automation runsOn' },
  'Chat with a self-hosted model': { palette: ['act:switch-model'] },
  'Builds and tests pick the Mac mini or this Mac': { settings: ['devices:Builds and tests'] },
  'Self-hosted models in Settings': { settings: ['models:Self-hosted models'] },
  'A video named with % or # plays': { elsewhere: "The videos under a finished chat reply; nothing to trigger" },
  'Import a source opens the Add sheet': { palette: ['act:quick-ingest'] },
  'Conversation history is in Library': { elsewhere: 'Library → Conversations, and the sidebar’s Search chats; Settings’ own button was cut when History & recovery moved to Library, History (Settings PR 4)' },
  'One Ask: every question is a chat': { palette: ['act:ask-vault'] },
  "A reply's cited lines open at those lines": { elsewhere: "A chat reply's file links" },
  "Next 7 days is now Agenda's week": { elsewhere: 'The week view was removed with Agenda (overnight 11-08). Tasks is List or Board, and Due today is a filter' },
  'Weekly review moves into Today': { elsewhere: 'Today and Weekly review were removed (overnight 11-08). Ask in chat; Work is Home and Running' },
  'Chat uses the whole pane again': { elsewhere: 'Every project and quick chat; layout only' },
  'A quieter message box': { elsewhere: 'Chat composer → + / model button → Permissions / Context → Connectors for the next turn' },
  'A Mac mini turn goes on when Hollow quits': { elsewhere: "A conversation's Continue, or Waiting on you → Resume interrupted chats" },
  'Set up a runner from scratch': { elsewhere: 'Settings → Devices → Remote runner → Set up (Check, Apply per step, Create for Google Cloud)' },
  'A remote runner is yours alone': { elsewhere: 'Settings → Devices → Remote runner (Set up, Record me as owner, Turn off)' },
  'Mac mini turns can open pull requests and reach Google Cloud': { elsewhere: 'Runs on → On the runner (Changes, a new conversation) and Settings → Devices → Runner GitHub access' },

  'Files your .gitignore names go to the Mac mini': { elsewhere: "Workspace turns on the Mac mini; nothing to trigger (the turn's first line names what stayed)" },
  'Commits made on the Mac mini are yours': { elsewhere: "Turns on the Mac mini; nothing to trigger (the worktree's git config decides)" },
  "The morning report says what's still on the Mac mini": { elsewhere: 'The Last night screen was cut (overnight 11-08 S4). The written morning report still names a Mac mini item whose work did not come back' },
  "Cut-off Mac mini work that can't come back is set aside, not cleared": { elsewhere: 'Turns on the Mac mini; the next turn there names where it went' },
  'Unvouched Mac mini changes come back as copies': { elsewhere: 'Workspace turns on the Mac mini; the next turn there brings them as copies' },
  'Work from a cut-off Mac mini turn comes back': { elsewhere: 'Turns on the Mac mini; the next turn there brings it back' },
  'Overnight and worker chats run on the Mac mini': { settings: ['devices:Overnight and worker chats'] },
  'Video renders and transcriptions can run on the Mac mini': { elsewhere: 'agents only: hollow-slot --remote --send <files> --fetch <output> <command>' },
  'The Mac mini keeps room for its work': { elsewhere: 'Turns and gates on the Mac mini; it prunes its own folders' },
  'See whether the Mac mini is ready': { settings: ['devices:Runs on'] },
  'Mac mini turns survive a dropped connection': { elsewhere: 'Any turn whose Runs on is Mac mini; reconnects on its own' },
  'Workspace chats can run on the Mac mini': { elsewhere: 'A new conversation in a workspace → Runs on (This Mac / Mac mini)' },
  'Build workspaces can run on the Mac mini': { settings: ['devices:Runs on'] },
  'Builds wait while memory is low': { elsewhere: 'agents only: hollow-slot holds builds and dev servers under 30% free memory' },
  'Tests and gates can run on another Mac': { elsewhere: 'agents only: hollow-slot --remote <command>; the runner is named in ~/.config/hollow/slot-remote' },
  'Workers when you request them': { elsewhere: 'A project lead or the COO → request a task batch' },
  'Local projects without Notion setup': { elsewhere: 'COO project tools; Projects → plus' },
  'A main agent for every project': { elsewhere: 'Project sidebar → Lead; project menu → Open lead' },
  'Cleaner project creation': { palette: ['act:new-workspace'] },
  'Smoother long conversations': { elsewhere: 'Every conversation; automatic targeted transcript updates' },
  'Fold for Hollow and Main agent': { settings: ['general:App icon'] },
  'Wait or continue when an agent runs out': { settings: ['models:When an agent runs out'] },
  'Let a chat read a tab you share': { elsewhere: 'Preview → a web tab → Share with chat → In use by strip → Stop' },
  'See where each started chat stands': { elsewhere: 'A chat’s Subagents chip → the Subagents tab → a subagent: its answer ends with its Done, Partial or Blocked line (Chats I started went with COO rebuild L3)' },
  'Import a link with any agent': { elsewhere: 'Library → Library actions (⋯) → Import a source → Link; New → Add… → Link' },
  'Measure text on the picture': { elsewhere: "a reply canvas's findings row → Measure on the picture" },
  'Services in one place': { settings: ['connectors:Services'] },
  'Warnings for agent versions known to break Hollow': { elsewhere: 'Settings → Accounts → Agent updates; the agents.update_cli card' },
  'Drafts stay with their chat': { elsewhere: 'Project and Quick chat composers; automatic conversation draft restore' },
  'Projects and Quick chat weeks': { elsewhere: 'Sidebar → New project / Chats; Sidebar → Tasks; chat → Move to project' },
  'Charts that fit the data': { elsewhere: 'a reply canvas (agents follow the design guide); Settings → Chat → Canvases' },
  'Know where a skill came from': { elsewhere: 'Library → Skills → a shared skill → Source → Check for update… / Local note' },
  'Start with a conversation': { elsewhere: 'Work → Home → Ask / All projects; Rail → Tasks' },
  'Documents and pages together': { elsewhere: 'File and web links → Preview tabs; PDF Results → page controls' },
  "Share a page's text from Annotate": { elsewhere: 'Preview pane → a web page → Annotate → Include page text → Add to chat' },
  'Crimson accent': { elsewhere: 'Stock themes throughout the app; Settings → General → Appearance' },
  'Each chat keeps its permissions': { elsewhere: 'Chat composer → model button → Permissions; Settings → Chat → Permission mode for new chats' },
  'A design check for canvases': { elsewhere: "a settled reply canvas's findings row → Ask to fix; the reply's receipt → Canvases" },
  'Clearer Claude quota recovery': { elsewhere: 'Settings → Accounts; quiet routing notices in the conversation' },
  'Clearer daily facts': { elsewhere: 'Settings → Usage / History; a build workspace’s branch → Changes. Today, the day ring and Commitments were cut (overnight 11-08 S3)' },
  'Ruled Automations and Map': { elsewhere: 'Automations; Work → Map' },
  'A simpler Settings sheet': { elsewhere: 'Settings → every section' },
  'Find Main actions': { elsewhere: 'COO → Actions → Automations → COO actions' },

  'Choose the amount of texture': { settings: ['general:Texture', 'general:Ambient motion'] },

  'One waiting list': { elsewhere: 'Needs you → Waiting or Incoming; Work → Running' },
  'Fewer, clearer places': { palette: ['act:main-agent'] },

  'Files and canvases together': { palette: ['act:canvas-gallery'] },

  'Still, architectural empty views': { elsewhere: 'Eligible empty views. The What’s new page was cut (overnight 11-08 S4)' },
  'Long chats keep up while a reply streams': { elsewhere: 'Every chat: a streaming reply (no control)' },
  'Quieter choices and overlays': { elsewhere: 'Search, confirmations, context menus and questions in chat (shared styling)' },

  'Room for workspace documents': { palette: ['act:open-project-files'] },
  'A focused Main agent space': { elsewhere: 'Top-bar orb → COO; Actions → Automations → COO actions' },

  'A denser task workbench': { elsewhere: 'Rail → Tasks: the issue list and its views; New issue or New local task' },
  'Claude answers start sooner': { elsewhere: 'Every Claude chat: its start-up beside preparation (no control)' },
  'Resume from a quieter Home': { elsewhere: 'Work → Workspaces' },
  'Easier Library scanning': { elsewhere: 'Library → Notes / Results / Recordings' },
  'Previews open only when you click': { palette: ['act:toggle-side-pane', 'act:open-project-files'] },
  'Web pages open, or say why': { elsewhere: 'Preview pane → a web link from a reply, or its New tab address field' },
  'One continuous conversation': { elsewhere: 'Every conversation; reply file actions; Settings → General' },
  'A clearer workspace list': { elsewhere: 'Sidebar; running-work popover → Manage → Work → Running' },
  'A quieter canvas workbench': { elsewhere: 'Chat → canvas replies and ask-canvases; Library → Results → saved canvases' },

  'Textured instruments, exact values': { elsewhere: 'Chat → request an instrument canvas; instrument → Data table or Table' },

  'Chats get ready sooner': { elsewhere: 'Every chat: the search before the agent starts (no control)' },
  'Patterned chart evidence': { elsewhere: 'Chat → request a chart canvas with dot fills; chart → Table' },
  'Canvas texture choices': { elsewhere: 'Chat → request a canvas; saved canvases in Library → Results' },
  'A quieter workbench': { elsewhere: 'Shared app surfaces; Work and empty chats; Settings → General → Motion / Theme / Density' },
  'Read source freshness': { elsewhere: 'Library → source file → source details' },
  'Retry an interrupted view': { elsewhere: 'Needs you; Retry on a failed view. Runbooks was cut (overnight 11-08 S4)' },
  'Read Companion workspace names': { elsewhere: 'Settings → Devices → Companion devices → What the backend copies' },
  'A quieter Main day': { elsewhere: 'The COO page header: its day in one line, and More (the side band went with COO rebuild L3)' },
  'A clearer Main agent header': { elsewhere: 'The COO page header and the top bar; top bar Actions for COO → Change icon…' },
  'Settings buttons sit where they belong': { elsewhere: 'Settings → Devices → Remote runner (Set up again, Turn off); Settings → Devices → Companion devices (the key and backend forms)' },
  'Steadier sidebar and Connections': { elsewhere: 'The sidebar Projects list (fold or unfold a project from its row); Settings → Connectors (Recordings, Your connectors)' },
  'Quieter Settings': { elsewhere: "Settings: a section's or sub-section's introduction and the runner rows' help (More), Interface → Appearance, Devices → Remote runner → Set up" },

  'Clearer saved canvas surfaces': { elsewhere: 'Work → pinned canvas; Canvas actions → Versions and pin, Export canvas' },
  'Plan progress comes first': { elsewhere: 'Workspace → Plan → Summary → Plan notes' },

  'An automation can end': { elsewhere: 'Automations → an automation → Edit → Schedule → Ends at / Or after this many runs' },
  'Compact automation covers': { elsewhere: 'Automations → an automation' },
  'Recover a result search': { elsewhere: 'Library → Results → Files → Clear search / Reset filters' },
  'Read plan progress by section': { elsewhere: 'Gone with the workspace panel (COO rebuild L4): PLAN.md opens in the Files tab (top bar Project files)' },
  'Compare pictures before source': { elsewhere: 'Library → Results → Canvases → Compare versions → View source changes' },
  'Results take shape while loading': { elsewhere: 'Library → Results → Files, first loading state' },
  'See transcript matches': { elsewhere: 'Library → Recordings → Find in transcript / Clear search' },

  'Clearer utility pages': { elsewhere: 'A build workspace’s branch → Changes → Build; Settings → Usage / History & recovery. Runbooks and Commitments were cut (overnight 11-08)' },
  'A more welcoming start': { elsewhere: 'Empty views across Hollow. The What’s new page was cut (overnight 11-08 S4)' },

  'Canvases feel at home': { elsewhere: 'Existing canvases in chats, Library → Results and the Preview pane' },
  'Calmer notices and questions': { elsewhere: 'Notices, menus, dialogs and agent questions; Settings → General → Motion' },

  'Clearer automation instruments': { elsewhere: 'Automations; Work → Home and Running' },
  'Preview a palette result': { elsewhere: 'Search anything; Settings → General → Ambient motion' },
  'Easier Library reading': { elsewhere: 'Library → Notes / Recordings' },

  'A satisfying task check': { settings: ['general:Ambient motion'] },
  'Experimental light and depth': { settings: ['general:Ambient motion'] },
  'A calmer conversation': { elsewhere: 'Workspace and COO conversations; existing controls stay in place' },
  'Main agent at a glance': { elsewhere: 'Top-bar COO and its page; Settings → General → Ambient motion' },

  'Clearer settings and spending caps': { elsewhere: 'Settings → General / Usage' },
  'A more tangible shell': { settings: ['general:Motion'] },

  'More readable task cards': { elsewhere: 'Tasks → Board; Tasks → Agenda → Week → Not this week' },
  'A shared material language': { elsewhere: 'Shared controls throughout Hollow; Settings → General retains appearance choices' },
  'Failures first in Review': { elsewhere: 'Needs you; a workspace → Activity; Running' },
  'A clearer day and week': { elsewhere: 'Those screens were removed (overnight 11-08). Tasks → Due today is the day filter that remains' },

  'Edit your own skills': { elsewhere: 'Library → Skills (a skill’s Edit skill and Use in <workspace>; the row menu’s Edit)' },
  'Room to read your work': { elsewhere: 'Work → Workspaces' },
  'An open window stays reachable': { elsewhere: 'Automatic native-window recognition for navigation and approval cards' },
  'Clearer workspace summaries': { elsewhere: 'Gone with the workspace panel (COO rebuild L4): a project’s notes open in the Files tab; Library → Results' },
  'A lighter workspace shell': { elsewhere: 'Work loads on demand. Release notes stay in docs/WHATS_NEW.md; the page was cut (overnight 11-08 S4)' },
  "Dictation stops when it can't help": { elsewhere: 'The composer’s Dictate button; a refusal notice’s Open System Settings' },
  'Canvas items become Plan rows or tasks': { elsewhere: 'Removed (overnight 11-08). Ask in chat to add a Plan row or a task' },
  'Find what failed in Activity and Logs': { elsewhere: 'A workspace’s Activity (Filter history by status); each reply’s Worked for ledger (the Logs view was removed in run 15 P1)' },
  'Results and Plan say checks in words': { elsewhere: 'A Result’s review bar in the Files tab (its checks in words); Library → Results cards' },
  'Know which build is running': { palette: ['act:copy-build'] },
  'Shortcuts follow macOS': { elsewhere: 'Settings → Keyboard shortcuts (Change, Remove, Reset; Go to rows Remove); the menu bar' },
  'Annotate with the keyboard': { elsewhere: 'Side panel → Browser → Annotate → capture stage, Comment on the whole page and numbered marks' },
  'The command palette finds what you mean': { palette: ['act:jump'] },
  'The browser panel and Annotate keep their keys': { elsewhere: 'Side panel → Browser → Back, Forward, Reload; its Annotate bar → Undo, Redo, Discard' },
  'A build workspace\'s Changes tell one story': { elsewhere: 'A build workspace’s branch in the top bar → Changes in the Preview pane (delivery record Retry, Update from <base>…, Build app); a queued message’s Open it' },
  'Reduced motion is consistent': { settings: ['general:Motion'] },
  'More room for larger text': { settings: ['general:Text size'] },
  'Appearance follows you': { elsewhere: 'Settings → General → Appearance → Match macOS; Text size; New theme… → contrast measurement' },
  'Back and Forward step through workspaces': { palette: ['act:nav-back', 'act:nav-forward'] },
  'Clearer agent failures and command help': { elsewhere: 'Conversation failure/warning steps; composer → Commands → /help; Settings → Accounts → Agent updates' },
  'One menu for every conversation, with Undo': { elsewhere: 'A conversation’s right-click or ⋯ menu in the sidebar or Conversation history, and the ⋯ beside the open conversation’s title' },
  'Archive, Hide, status and Merge each do one thing': { elsewhere: 'A workspace’s menu → Archive…, Unarchive, Mark as done, Merge into…; Work → All work → a merged row’s Undo merge…' },
  'One run record everywhere': { elsewhere: 'Reply More actions → Show run record; workspace Activity → run; Automations → automation → Run record' },
  'Undo a workspace delete, and Recently deleted': { elsewhere: 'A workspace’s menu → Delete… → Undo; Work → All work → Recently deleted → Restore' },
  'Pick which interrupted chats to resume': { elsewhere: 'Needs you → Interrupted by a restart, a card that appears only after Hollow quit mid-turn; the sidebar\'s conversation rows' },
  'Canvas answers stay put, queue while the agent works, and say when they didn\'t go': { elsewhere: 'A conversation → the Send button under a canvas that asks you questions, and its expanded copy in the Preview pane' },
  'Clear keyboard focus': { elsewhere: 'Automatic keyboard affordances on existing controls' },
  'Steering says what happened, in words': { elsewhere: 'A conversation while a turn runs → the hint under the box, Send options, a steer row that wasn’t taken' },
  'Retry a failed message, copy a conversation, and Esc that stays in its popover': { elsewhere: 'A conversation → under a failed reply (Retry); Conversation tools → Copy conversation (Markdown); the composer → Connectors' },
  'Icon edits keep your draft': { elsewhere: 'Settings → General → Appearance; COO → top bar Actions for COO → Change icon…' },
  'Settings in seven sections': { settings: ['shortcuts:Use keyboard shortcuts', 'accounts:Claude', 'devices:Pair a device', 'advanced:Search & indexes', 'sources:Hollow folder', 'advanced:History & recovery is in Library'] },
  'Settings has 15 sections': { settings: ['chat:Retitle new chats', 'usage:Budgets', 'advanced:Context prioritizer', 'personalization:Memory', 'devices:Pair a device'] },
  'A compact Home with one workspace list': { elsewhere: 'Work → Workspaces → Show all N workspaces / Find; sidebar → All workspaces; Connections & imported work' },
  'Settings asks before dropping unsaved changes': { settings: ['usage:In rotation', 'background:Automations, windows and the run queue are on the Automations page'] },
  "Plaud's two sign-ins are named apart": { settings: ['connectors:Recordings in Hollow', 'connectors:Recordings'] },
  "Library's tabs keep their state honest": { elsewhere: 'Library → Notes / Results / Recordings / Connected projects; Library actions → Show file history' },
  'Quick entry keeps what you typed': { elsewhere: 'Quick Capture / Quick Ingest; Library → Library actions → Add a task, note, link, file or recording… → Discard draft' },
  'Live state stays in the collapsed sidebar': { elsewhere: 'Sidebar → pinned workspace, Quick chats or Work; top bar → Show sidebar' },
  'Rename, move and trash notes in Library': { elsewhere: 'Library → a note’s ⋯, or File actions in the reader → Rename… / Move to folder… / Move to trash' },
  'Create a workspace and start from its goal': { elsewhere: 'Sidebar → Projects + → Create project / Create and start' },
  "What's new remembers where you left off": { elsewhere: 'The What’s new page and the welcome tour were cut (overnight 11-08 S4). Ask in chat' },
  'Canvas changes say what is kept': { elsewhere: 'Canvas head → Try again / pin menu; Results or Library → Canvases → Canvas actions → Open source chat / Export canvas' },
  'Scheduled work names its configured window': { elsewhere: 'Automations → the background clock’s Tonight window (a configured hour band, not a queue)' },
  'Usage names its provider and keeps partial Codex tokens': { elsewhere: 'Conversation composer → usage ring; Settings → Usage → Codex plan' },
  'Reply images, code and media are easier to use': { elsewhere: 'Conversation reply → thumbnail, Show all lines, Copy, Load images; expanded Bash row → Copy; failed video → Try again' },
  'Spending caps in one list, and the Main agent’s day in one line': { settings: ['usage:COO daily hard stop', 'usage:Overnight cap'] },
  'Opening a failure in Review leaves it waiting': { elsewhere: 'Bell → Needs you → See all → Failures' },
  'Tasks filters have one home, and the layouts clear names': { elsewhere: 'Rail → Tasks → the view tabs, Filter, and Group and Order' },
  'Notion-linked tasks move by drag, menu and bulk edits too': { elsewhere: 'Rail → Tasks → an issue → its properties column' },
  'Task rows have a ⋯, and their keys work once you Tab to a row': { elsewhere: 'Rail → Tasks → a row’s ⋯; Rail → Tasks → Commitments → Drop' },
  'Review has one entry, an honest count and ⌘5': { elsewhere: 'Bell → Needs you → See all (⌘5); Go → Needs You; Needs you → Waiting → All → Unsaved drafts → Discard' },
  'Tonight includes scheduled work': { elsewhere: 'The Tonight queue, Runbooks and Dry-run tonight were cut (overnight 11-08 S4). Automations stays' },
  'Choose automation windows and triggers by clicking': { elsewhere: 'Automations → Automation options → Edit → Schedule; Automations → Automation windows and the run queue (moved from Settings in PR 4)' },
  'Notices wait for you and keep a history': { elsewhere: 'Bell → Notifications → App messages; notice stack' },
  'Map controls and a text list': { elsewhere: 'Work → Map → List and map header controls' },
  'Review puts what unblocks running work first': { elsewhere: 'Bell → Needs you → See all → Waiting → All, Decisions, Failures' },
  'Today says when its facts changed, and its rows open what they name': { elsewhere: 'The Today screen was removed (overnight 11-08). Ask in chat' },
  'The Main agent shows its day clearly': { palette: ['act:main-agent'] },
  'Running now stays visible all week': { elsewhere: 'Work → Running tab; the Today tab’s weekly count' },
  'A quieter composer while the agent works': { elsewhere: 'Conversation composer → model menu → Effort → Ultracode; the permissions button' },
  'Ask says what Enter does': { elsewhere: 'Rail → Work → Ask anything → Where to ask' },
  'Workspace cards show their state in words': { elsewhere: 'Rail → Work → Workspaces → Pick up where you left off' },
  'Choose the calendar Agenda reads': { elsewhere: 'Settings → Connectors → Agenda calendar' },
  'One word, one meaning: Running, History, Summary, Blocked': { palette: ['act:mission-control'] },
  'Plainer words: automations, imports, schedules and toasts': { palette: ['act:quick-ingest'] },
  'Section toolbar: views switch, actions ask, Markdown opens the file': { elsewhere: 'Gone with the workspace panel (COO rebuild L4): a project’s notes open as files in the Preview pane' },
  'Your work opens on Workspaces every time': { elsewhere: 'Rail → Work (⌘1) → Workspaces; a canvas pin menu → Pin to Work' },
  'Panel requests open the right workspace conversation': { elsewhere: 'A Result’s review bar → Ask for a revision (it opens the Result’s own workspace first)' },
  "Weekly review's next week labels events that run over several days": { elsewhere: 'The Weekly review screen was removed (overnight 11-08). Ask in chat' },
  'The side panel remembers where you were in each workspace': { elsewhere: 'A workspace page: Chat · Workspace and its open section' },
  'Task drafts survive linked edits and can resolve conflicts': { elsewhere: 'Tasks → Edit task → Review; task sheet' },
  'Wikilinks open in Library; the palette keeps your pick; Prepare works during a meeting': { elsewhere: 'Library → a note → its [[link]]; the command palette; Today → Prepare' },
  'The Hollow app switch says what agents can do': { settings: ['connectors:Built into Hollow'] },
  'The Main agent keeps answering beside your page': { elsewhere: 'Removed with the drawer in run 15 P3: the COO page (top-bar orb) has the same conversation, permission mode and Stop' },
  "A reply can't look verified when its checks are stale or missing": { elsewhere: 'A reply’s footer → its check chip → the Run record' },
  'Settings sets the permission mode for new chats; a chat\'s own menu changes that chat': { settings: ['chat:Permission mode for new chats', 'models:Effort'] },
  'One-click deletes and runs ask first': { elsewhere: 'Settings (Keyboard shortcuts, Accounts); a subagent’s Stop in a chat’s Subagents tab; a workspace’s menu; Automations' },
  'Main agent model choices explain what can run': { elsewhere: 'Removed in COO rebuild L6 (11-10): any installed agent may run the COO, so its model menu lists every agent like any chat' },
  "Review's keys act only on what you are looking at": { elsewhere: 'Bell → Needs you → Waiting. Incoming was removed (overnight 11-08)' },
  'A failed view has a way back': { elsewhere: 'The affected page or Settings section → Retry / Reload page; Work → Running' },

  'No paid read starts just because you opened a page': { elsewhere: 'Settings → Personalization (Keep learning, Learn about me / Look again)' },
  'Every Review view lists all of its items': { elsewhere: 'Bell → Needs you → All, Decisions, Results, Failures. Incoming was removed (overnight 11-08)' },
  'A file dropped on the message box is attached or refused in words': { elsewhere: 'A chat’s message box: drop a file on it; the notice’s Add to Sources… button' },
  "Tidying Recent notifications never clears a failure you haven't seen": { elsewhere: 'Bell → Notifications: Mark listed read and Clear listed' },
  '"calendar" finds the Agenda': { elsewhere: 'Settings → Connectors → Agenda calendar. The Agenda task layout was removed (overnight 11-08)' },
  'What\'s new names the build you are running': { elsewhere: 'The What’s new page was cut (overnight 11-08 S4). The running build stays on the install notice and in Settings → Advanced' },
  '⌘K then Enter never stops or starts agent work': { palette: ['act:stop-run', 'act:clear-debug-logs'] },
  'Automation edits stay yours': { elsewhere: 'Automations → New automation; Automation options → Edit' },
  'Replacing your work asks first': { elsewhere: 'Library → Skills → New skill; Automations → Edit → Save as skill; Prep/Recap → Write it again; Settings → General → Appearance' },
  'Stop a running automation and review every night item': { elsewhere: 'Automations → an automation → Stop run. Last night was cut (overnight 11-08 S4)' },

  'New running work gets a fresh restart choice': { elsewhere: 'Any setting that needs a restart (Settings → Sources → Task tracker) offers Restart, which waits for running work' },
  'Installing Hollow always asks you first': { elsewhere: 'agents only: the install card in the chat that asked (build.install_built_app); the allowed build then waits in the top bar’s Update button' },
  'Update Hollow from one button': { elsewhere: 'The top bar’s Update button (shown only when an update or a Local build is ready); Changes → Hollow app → Show in Update button' },
  'Merge, install and deploy suggestions need a second Enter': { elsewhere: 'A chat’s message box after a reply: the suggestion, its Place suggestion button and the hint under it' },
  'Changes stays current while other conversations work': { elsewhere: 'Build workspace → Changes' },
  "A source Hollow couldn't read says so": { elsewhere: 'The source row says when it could not be read. The Today screen was removed (overnight 11-08)' },
  'Quit without losing track of running work': { elsewhere: 'App/menu-bar Quit and the top bar’s Update button (Restart to update, Update when chats finish)' },
  'Today keeps the right day': { elsewhere: 'The Today screen was removed (overnight 11-08). Tasks → Due today is a filter on the task list' },
  'Steer a chat while it works': { elsewhere: 'A chat’s message box while a turn runs: its send button, Send options and Stop' },
  'Your own app and Main agent icons': { settings: ['general:App icon', 'general:COO icon'] },
  'Four Hollow app icons': { settings: ['general:App icon'] },
  'Ask ⌄ leaves the top bar': { palette: ['act:main-agent'] },
  'Pin workspaces to the top of the sidebar': { elsewhere: 'a workspace\'s right-click or ⋯ menu → Pin, Unpin, Move up / Move down; the sidebar\'s Pinned group' },
  'Ten chats at once': { settings: ['chat:Chats running at once', 'chat:Chats at once in one workspace'] },
  'Up to 25 chats at once': { settings: ['chat:Chats running at once', 'chat:Chats at once in one workspace'] },
  'Each chat gets credit for its own files': { elsewhere: 'Shown on its own: a chat\'s run record, Results and History & recovery' },
  "Chats in one workspace keep each other's brief edits": { elsewhere: 'automatic: a notification, Library → History → the run, and Waiting on you for a conflict copy' },
  'Background windows on a clock': { elsewhere: 'Automations → background clock → a window opens Automations' },
  'A canvas view that fails keeps your Summary choice': { elsewhere: 'Gone with the workspace panel (COO rebuild L4): the curated canvas views were removed; canvases stay in chat' },
  'A calmer task list': { elsewhere: 'Tasks → Tasks scope chips; Tasks → Commitments; Settings → Keyboard shortcuts → Lists' },
  'One automation run strip': { elsewhere: 'Automations → an automation → Run now or Automation options' },
  'Settings at a glance': { elsewhere: 'Settings → any section shows how it saves' },
  'Clearer view controls': { elsewhere: 'Tasks, Library → Results, workspace Results and Work → Running' },
  'A quiet place to start': { elsewhere: 'Tasks and Automations, when their complete lists are empty' },
  'Library in pictures': { elsewhere: 'Library → Results → Files or Canvases' },
  'Archive older entries': { elsewhere: 'A workspace’s menu → Archive older entries… (V5a): a preview of WORKSPACE.md and PLAN.md, then one confirm; agents use workspaces.archive_shared_docs; History & recovery undoes it' },
  'Your work at a glance': { elsewhere: 'Rail → Work → Home and Running. The Today tab and its weekly count were cut (overnight 11-08 S3)' },
  'One title per screen': { elsewhere: 'Destination tabs. Runbooks was cut (overnight 11-08 S4)' },
  'Create in Notion': { elsewhere: 'Removed with Notion sync (2026-11-10); New task saves a local task, and a tracker-linked project creates issues in the tracker' },
  'New project in Notion': { elsewhere: 'Removed with Notion sync (2026-11-10); projects are made in Hollow or the tracker' },
  'Install questions read the right checkout': { elsewhere: 'agents only: build.install_ready and build.install_brings (no id)' },
  'A failed install comparison says so': { elsewhere: 'build workspace → Changes → Hollow app, after a finished build' },
  "Integrity check counts files it couldn't open": { settings: ["advanced:Hollow's databases"] },
  'Files and Visuals on a shelf': { elsewhere: 'Right panel → Files or Visuals → thumbnail shelf' },
  'Sources in a row': { elsewhere: 'Gone with the workspace panel (COO rebuild L4): SOURCES.md opens in the Files tab; the composer’s + → Add to Sources…' },
  'Brief, headline first': { elsewhere: 'Gone with the workspace panel (COO rebuild L4): WORKSPACE.md opens in the Files tab (top bar Project files)' },
  'Quieter Plan rows': { elsewhere: 'Gone with the workspace panel (COO rebuild L4): PLAN.md opens in the Files tab and its boxes tick there' },
  'History by day': { elsewhere: 'Gone with the workspace panel (COO rebuild L4): the Subagents tab, Work → Running and History & recovery show what ran' },
  'Compact workspace notices': { elsewhere: 'Gone with the workspace panel (COO rebuild L4): the project’s notes open as files' },
  'Make the missing canvas views together': { elsewhere: 'Gone with the workspace panel (COO rebuild L4): the curated canvas views were removed; canvases stay in chat' },
  'Results in pictures': { elsewhere: 'Library → Results → Files' },
  'Overview at a glance': { elsewhere: 'Gone with the workspace panel (COO rebuild L4): ask the project lead; notes open in the Files tab' },
  'Panel summaries at a glance': { elsewhere: 'Gone with the workspace panel (COO rebuild L4): Library → Results and the Files tab' },

  'More room for panel canvases': { elsewhere: 'Canvases in chat and the Preview pane: Visuals → Canvas actions → Versions and pin…' },
  'Canvas, visual and text together': { elsewhere: 'Gone with the workspace panel (COO rebuild L4): files show Preview / Source in the Preview pane' },
  'One clear workspace panel header': { elsewhere: 'Gone with the workspace panel (COO rebuild L4): the Preview pane’s one header row (tabs, the file’s controls, close)' },
  'The tool-folder list no longer waits on a stalled network folder': { settings: ['advanced:Folders agents can run tools from'] },
  "A broken PDF in Sources can't keep stopping Hollow": { elsewhere: 'nothing to trigger: it happens when a source file is read' },
  'Skill descriptions without a stray "> "': { elsewhere: 'Library → Skills' },
  'A busy Codex retries once instead of stopping': { elsewhere: 'nothing to trigger: it happens when a Codex turn hits an outage; the reply shows its row' },
  'Codex chats show how full their context is': { elsewhere: 'the usage ring beside the composer, after a Codex reply' },
  'Codex spend, estimated at the price you enter': { settings: ['advanced:Prices'] },
  'Canvas views open as you left them everywhere': { elsewhere: 'Library → Canvases' },
  'The Main agent starts fresh after an hour away': { settings: ['background:Start fresh after an hour away'] },
  '"Tests pass" with nothing to back it is flagged': { elsewhere: "nothing to trigger: a reply's Run record" },
  'Agents find project files by what they say': { settings: ['advanced:Search inside project files'] },
  'Hollow reads the PDFs and Office files in your projects': { elsewhere: 'Library → Connected projects → a PDF or Office file' },
  'Recap a meeting after it ends': { elsewhere: "Today's past meetings, the Agenda's event detail, Library → Recordings" },
  "See Hollow's databases, and remove other vaults' old indexes": { settings: ["advanced:Hollow's databases", 'advanced:Disk space'] },
  'Export with a workspace or custom canvas look': { palette: ['act:save-canvas-image'] },
  'Agents hear how full their context is when a message starts': { elsewhere: 'agents only; recorded layers are in usage ring → Context details' },
  'Present a Result or release as slides': { elsewhere: 'Present and Play as slides were cut (overnight 11-08 S4). Results still exports Markdown' },
  'The Run record checks the lines a reply cites': { elsewhere: "a reply's Run record → Cites" },
  'See what a finished build brings before installing': { elsewhere: 'build workspace Changes → Hollow app' },
  'Agents search a conversation for what was said': { elsewhere: 'agents only' },
  'Give one workspace its own canvas look': { elsewhere: 'workspace ⋯ → Canvas look…' },
  'The usage ring says what fills the context, and offers a fresh start': { elsewhere: "a conversation's usage ring" },
  'Check a flow layout and draft a fix': { elsewhere: "a reply canvas's layout line → Ask to fix" },
  'Save a flow diagram as SVG': { palette: ['act:save-canvas-image'] },
  'Agents are told how full their context is as a long turn grows': { elsewhere: "agents only: a long chat's next Hollow app tool result" },
  'Mermaid flowcharts draw in replies and files': { elsewhere: "a reply's Mermaid fence; a .mmd or .mermaid file in the Preview pane" },
  'Agents ask a long saved log for what they need': { elsewhere: 'agents only' },
  'Save a canvas as image or PDF': { palette: ['act:save-canvas-image'] },
  'Agents are reminded of your rules after they compact': { elsewhere: "agents only: the receipt's Compacted row; What your last message used" },
  'Search indexes no longer keep a big leftover file': { elsewhere: 'nothing to trigger: it happens at launch' },
  'Videos play in the side panel and in Results': { elsewhere: "a video's name in a reply or footer chip (it plays in the Preview pane), or Library → Results" },
  'Videos play right under the reply that made them': { elsewhere: 'appears on its own under a finished reply that made or names a video' },
  'Video Results show a frame of the video': { elsewhere: "appears on its own on a video's Results card and under a reply" },
  "An agent no longer waits two minutes on a card you can't see": { elsewhere: "automatic: the agent's reply in the chat says it, while Hollow's window is closed to the tray" },
  'Context in one place': { elsewhere: 'composer usage ring and its Context details link' },
  'A compaction window for Claude and Codex (off; no switch yet)': { elsewhere: 'no control yet: an agent setting with no Settings item; the count is in the run record' },
  'Build slots in every chat that can run commands': { elsewhere: 'agents only' },
  'A chat that started other chats can be woken when they finish': { elsewhere: "agents only: the chat's agent asks for it; the turn's receipt shows it" },
  'Agent updates wait for a tool you started in Terminal': { elsewhere: 'automatic: Settings → Accounts → Agent updates waits while the tool is in use' },
  'Make a canvas view of a workspace section': { elsewhere: 'Gone with the workspace panel (COO rebuild L4): the curated canvas views were removed; ask for a canvas in chat' },
  "Automations say when a schedule can't be read": { elsewhere: "appears on its own: the automation's row and details in Automations when its schedule can't be read" },
  "Companion backend: says when it can't be reached": { settings: ['devices:Backend for the web app'] },
  'Undo last batch stays after the snackbar': { elsewhere: 'Library → Learnings, above the list after a bulk Save or Reject of learnings' },
  'See the folders agents can run tools from': { settings: ['advanced:Folders agents can run tools from'] },
  'Remove finished worktrees keeps a build waiting to install': { settings: ['advanced:Disk space'] },
  'Changed files: clearer names, Results from run details': { elsewhere: "a reply's changed files and run details in a workspace conversation; Show its history in Results when a Result is gone" },
  'Companion fixes: Discard always answers, removing a backend forgets it': { elsewhere: 'Discard it on an Old change from a device card in Needs you; Settings → Devices → Companion devices' },
  'Agent updates wait for bunx and bun run too': { elsewhere: 'automatic: Settings → Accounts → Agent updates waits while the tool is in use' },
  'Question cards and canvas Send: no answers by accident': { elsewhere: 'automatic: the question cards over the message box, and Send under an ask-canvas' },
  'A changed file in outputs/ opens in Results': { elsewhere: "a reply's changed files in a workspace conversation: Open in Results on their right-click menu and the reply's ⋯ (their click opens the Preview pane since 2026-11-04)" },
  "Open never launches an app hiding behind a document's name": { elsewhere: "a workspace's Sources cards (Open original), the chat's file preview (Open) and Results (Open in its app)" },
  "Settings show an agent's change right away": { settings: ['personalization:How many learnings to suggest', 'advanced:Extraction model'] },
  'A signed-out calendar says how to fix it': { elsewhere: 'appears on its own: Settings → Connectors → Microsoft 365 → Sign in again, when the calendar sign-in has lapsed' },
  'Old changes from your devices ask first': { elsewhere: 'a card that appears on its own in Needs you when a device change is over 14 days old' },
  'Free disk space from Settings': { settings: ['advanced:Disk space'] },
  'Test the companion backend connection': { settings: ['devices:Backend for the web app'] },
  'Plainer words: "Run record" instead of "receipt"': { elsewhere: "nothing to trigger: a reply's Run record, Activity, an automation's Logs and run details" },
  'What\'s new opens on the newest releases': { elsewhere: 'The What’s new page was cut (overnight 11-08 S4). docs/WHATS_NEW.md keeps the notes' },
  "Questions one at a time over the message box, and an ask-canvas's Send sends": { elsewhere: "automatic: an agent's question cards over the message box (chat and the COO page), and Send under an ask-canvas" },
};

/** `### Title [#123](…), [#124](…)` → `Title`. */
const headings = whatsNew.split('\n')
  .filter(l => l.startsWith('### '))
  .map(l => l.slice(4).replace(/\s*\[#\d+\]\([^)]*\)(,\s*\[#\d+\]\([^)]*\))*\s*$/, '').trim());

/** The palette's fixed actions (actionDefs.ts) by palette id, in its words. */
const paletteLabels = new Map(ACTION_DEFS.filter(d => d.palette).map(d => [d.palette!, actionLabel(d)] as const));

const settingsControls = new Set(SETTINGS_SECTION_GROUPS.flatMap(g => g.items.flatMap(s => (s.controls ?? []).map(c => `${s.id}:${c.label}`))));

describe("palette coverage of What's new (overnight G3)", () => {
  it('reads every heading', () => {
    expect(headings.length).toBeGreaterThan(30);
    expect(headings.every(h => h && !h.includes('[#'))).toBe(true);
  });

  it("places every What's new feature, and only those", () => {
    expect(headings.filter(h => !(h in COVERAGE)), 'a heading with no entry in COVERAGE').toEqual([]);
    expect(Object.keys(COVERAGE).filter(k => !headings.includes(k)), 'an entry whose heading is gone').toEqual([]);
  });

  it('gives each feature a way in, or says where it lives instead', () => {
    for (const [title, reach] of Object.entries(COVERAGE)) {
      const ways = (reach.palette?.length ?? 0) + (reach.settings?.length ?? 0);
      expect(ways > 0 || !!reach.elsewhere?.trim(), title).toBe(true);
      expect(ways > 0 && !!reach.elsewhere, `${title}: either a way in or elsewhere`).toBe(false);
    }
  });

  it('names a palette action that exists, with a clear label', () => {
    for (const [title, reach] of Object.entries(COVERAGE)) for (const id of reach.palette ?? []) {
      const label = paletteLabels.get(id);
      expect(label, `${title}: ${id} is not a palette action`).toBeTruthy();
      // A clear name: a capital first, short, no sentence full stop.
      expect(label!, id).toMatch(/^[A-Z]/);
      expect(label!.length, id).toBeLessThanOrEqual(40);
      expect(label!.endsWith('.'), id).toBe(false);
    }
  });

  it('names a Settings control the palette lists', () => {
    for (const [title, reach] of Object.entries(COVERAGE)) for (const id of reach.settings ?? []) {
      expect(settingsControls.has(id), `${title}: ${id} is not a Settings control`).toBe(true);
    }
  });

  it('offers Merge into… for the open workspace with the sidebar menu’s guard', () => {
    // Its condition (actionDefs.ts `can-merge`), evaluated in actionRun.ts.
    expect(ACTION_DEFS.find(d => d.palette === 'act:merge-workspace')?.when).toBe('can-merge');
    const at = runSource.indexOf("'can-merge': () =>");
    const guard = runSource.slice(at, runSource.indexOf('},', at));
    for (const check of ['isQuickChats(', 'isMainAgent(', 'canMergeFrom(', 'canLeaveConversation()']) expect(guard, check).toContain(check);
  });

  it('keeps Merge into… out of the empty-query list (a heavy action)', () => {
    // The order lives in paletteDefault.ts since P0-02.
    const rows = [{ id: 'act:merge-workspace', group: 'Conversation' }, { id: 'act:open-files', group: 'Conversation' }, { id: 'nav:tasks', group: 'Navigation' }];
    expect(paletteDefaultOrder(rows, [], null).map(r => r.id)).toEqual(['act:open-files', 'nav:tasks']);
  });

  it('jumps to a Usage panel, not the whole Usage section', () => {
    // Value this month is a div.usage-block heading; without the class the
    // nearest match is the Usage <section> and the jump centres all of it.
    // Since P1-21 the Settings section itself is never the match at all.
    const closest = settingsSource.match(/target\??\.closest<HTMLElement>\('([^']+)'\)/)?.[1] ?? '';
    const order = closest.split(',').map(s => s.trim());
    expect(order.indexOf('.usage-block')).toBeGreaterThan(-1);
    expect(order.indexOf('.usage-block')).toBeLessThan(order.indexOf('section:not(.settings-section)'));
    expect(order).not.toContain('section');
  });
});
