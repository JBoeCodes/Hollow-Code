# What's new: 2026-11-20

### Pull requests and issues for a project

- **What:** a page lists a project's GitHub pull requests or issues (filter by state, author, label), opens one with its checks, reviews and comments, and lets you comment, label, assign, close or reopen it after a confirm. Ask about this starts a chat about it. Linear links appear when one names a team ENG issue; it stays the task list.
- **Where:** Projects → Pull requests, or the command palette's Open Pull requests (Projects). Needs the GitHub CLI signed in; the page says what is missing.
- **Undo / turn off:** reopen or unlabel by hand on the same page; nothing runs on its own and nothing merges.

### Read secrets from 1Password instead of the Keychain

- **What:** an optional second place for a secret to live. Point the GitHub runner token, the Linear API key or a
  Claude account's token at an `op://Vault/Item/field` reference and Hollow reads it from 1Password when it needs
  it, through the 1Password command line tool. Hollow saves only the reference, never what 1Password returns, and
  hides the value in logs. If 1Password is locked, not signed in or not installed, Hollow uses the Keychain value
  when there is one and says why when there isn't.
- **Where:** Settings → Accounts → 1Password: the switch, Check, and one reference box per secret.
- **Undo / turn off:** it is off until you turn it on. Turn the switch off, or press Use the Keychain on a secret,
  and Hollow goes back to the Keychain. Nothing in the Keychain is changed or removed.


### Run a terminal beside a chat

- **What:** a shell in the side pane, in the chat's own folder. Open up to four per chat, each its own tab, and they
  keep running when you look at another tab or chat. It starts only when the chat's access mode lets its agent run
  commands outside a folder sandbox (Auto, Approve for me or Bypass); in Plan, Default, Accept edits, Read only and
  the like it says so and does not start. Its environment carries no API keys, tokens or Hollow variables, only
  your home, user, language, time zone and a path. If you switch the chat to a mode that forbids it, its terminals
  end. It sends nothing off this Mac and an agent cannot type into it.
- **Where:** a chat's card → Terminal, or the pane's + (New tab) → Terminal. Inside it: New terminal, Copy output,
  Clear and a Close on each terminal. Copy a selection with ⌘C and paste with ⌘V; the terminal binds no ⌘ chord, and
  Esc and Tab go to the shell, so the pane's own close is the × beside its tab and Shift+Tab moves
  focus out of the terminal.
- **Undo / turn off:** close a terminal with its ×, or the Terminal tab (it asks first while a shell is running):
  every shell and what it started is hung up and then killed. Archiving or settling the chat and quitting Hollow end
  them too. Nothing starts a terminal for you.


### Ask a quick question in a side chat

- **What:** a second chat opens beside a project chat or a chat in Chats, for a quick question about it while its
  work carries on. It reads what the chat has said, runs read only by default (you can allow the chat's own
  access, never more) and counts toward budgets and usage like any chat. Nothing goes into the main chat unless you
  press Add to main chat on a reply, which puts it in the main chat's message box as a draft.
- **Where:** the Side chat button beside the chat's name, Actions for this conversation → Open side chat, or ⌘K,
  Toggle side chat. Close side chat or Esc puts it away. It has no key. The COO keeps its own page.
- **Undo / turn off:** close the pane; its chats are not listed in the sidebar and are not sent anywhere.


### Give a chat a scratch folder and turn it into a project

- **What:** every quick chat has its own scratch folder for working files. New project from this chat makes a
  project, moves the chat into it and carries the scratch files into the project's sources folder.
- **Where:** a quick chat's menu → Scratch folder (opens it in Finder), and New project from this chat.
- **Undo / turn off:** move the files back by hand from the project's sources folder; nothing runs on its own.

### See projects by their letters and color

- **What:** a project with no app artwork and no GitHub owner shows its first letters on a tint instead of a
  folder. The tint is chosen for it, and you can change it.
- **Where:** a project's menu → Change color (each click picks the next color) and Reset color.
- **Undo / turn off:** Reset color. The choice is kept on this Mac only.

### Clone a repository from an address

- **What:** the new project form can clone an https or ssh address into your Projects folder and pick it as the
  repository to build in. Plain http, file paths and addresses with a password are refused.
- **Where:** Projects → New project → Templates and location → Repository worktree → Clone from an address.
- **Undo / turn off:** delete the cloned folder from your Projects folder. A failed clone leaves nothing behind.

### Run named scripts for a project

- **What:** a project can keep named commands (test, dev, lint) and run one with a click. Its output opens in
  the Preview pane beside the chat, where you can run it again or stop it. A script can have a shortcut, ⇧⌘ and
  one letter or digit; keys macOS or Hollow already uses are greyed out in the list with the reason.
- **Where:** the card beside a chat → Scripts → Add script, Run and Edit on each row. Add project script in the
  card header opens the same editor.
- **Undo / turn off:** Edit → Delete removes a script. Stop ends a run; no script ever starts on its own.

### Reopen a closed tab in the Preview pane

- **What:** a tab you closed in the Preview pane (a file, Changes or a canvas) can come back, newest first. It has no key, because ⌘⇧T belongs to browsers' own Reopen closed tab.
- **Where:** ⌘K, Reopen closed view; View → Reopen Closed View; or a New tab's Closed section → Reopen closed tab.
- **Undo / turn off:** close the tab again. Hiding the pane is not a close, and a New tab is never kept.

### Swipe to go back and forward

- **What:** swipe sideways with two fingers on the trackpad to go back or forward, like the Back and Forward buttons. It waits for a clearly sideways swipe, once per swipe, and leaves a box that can scroll sideways (a wide table, a code block) its own swipe.
- **Where:** anywhere in the window except editors and dialogs; Settings → Keyboard shortcuts → Swipe to go back and forward turns it off.
- **Undo / turn off:** that switch; the buttons and ⌘[ ⌘] always work.

### See where each shortcut works

- **What:** a shortcut row says where its key works when it is not everywhere ("In a chat"), and the list ends with the keys only the command palette reads (> for actions, ⌘1 to ⌘9 to pick a row, Backspace to go back). These cannot be changed.
- **Where:** Settings → Keyboard shortcuts.
- **Undo / turn off:** nothing to turn off; changing a key works as before.

### Servers and automations in the chat card

The card beside a chat now has a Servers section, with the dev servers and
watchers an agent left running in this chat's folder (open, read the log or stop
them), and an Automations section, with the automations that run in this chat.
Each shows its count while collapsed. Add an automation drafts a request in the
message box.

### See a branch's pull request, checks and comments in the side pane

In a chat in a git folder, More git actions on the card has a Pull request item
that opens a tab beside the chat: the PR for the current branch, each check's
state and the comments and reviews so far. It reads through the GitHub CLI you
already use, changes nothing on GitHub, and Ask about this only drafts a message.

### One effort control for Antigravity

- **What:** the model picker lists Gemini 4 Neon, Argon, Carbon and Hydrogen once each,
  and the Effort row sets the level. It shows only the levels the model has (Neon has
  Extra high, the others stop at High). A saved "Gemini 4 Neon (High)" becomes Neon at High.
- **Where:** the composer's model menu, Antigravity.
- **Undo / turn off:** pick another level; nothing to switch off.

### Review changes by file or by turn in a Diff tab

In a chat in a git folder, the card's Changes row opens a Diff tab in the side
pane. It shows the folder's uncommitted changes with a file tree, or one agent
turn's changes when the chat has its own worktree. Find filters files and
highlights matching lines. Tick Viewed to collapse a file; it unticks itself when
the file changes again. Nothing is staged or sent.

### Branch, worktree and Commit and push from the chat card

For any chat in a git folder, the card shows the current branch with commits
to push. Its menu switches branches, makes a new branch, or opens a new
worktree and points the chat at it. Changes shows how many lines were added
and removed. Commit and push writes a message from the changed file names that
you can edit before anything is committed. Nothing is sent to a model.

### Select chats, sort, filter, group and saved views

The Quick chats heading has two new buttons. Select chats (or Cmd-click a row,
Shift-click for a stretch) ticks rows; a bar then settles, snoozes or moves them
together, with one Undo for the settle or snooze. View sorts by activity, title or
start, shows only chats that need you, are working, have unread messages or a
pull request, groups by agent or day, and saves a combination under a name.

### Subagent chats under their lead, and rename in place

A chat's subagent chats now sit indented under it in the sidebar, folded behind a
count button (they open by themselves while one works or waits). Project rows do
the same for their lead's subagents. Double-click any chat or project row, or
choose Rename… from its menu, to rename it right in the row: Enter saves, Escape
keeps the old name.

### Status marks on chat rows in the sidebar

A chat row now says what it needs: Approval or Input while it waits for you,
Plan when a plan is ready to review, a dot when new messages arrived since you
last had it open, and the pull request it is linked to. The chat you have open
shows only the ask and the pull request.

### Open a chat's folder in your editor from the card

The card beside a chat now starts with the Mac the chat is on and an Open in
menu: Finder, plus Visual Studio Code, Cursor, Zed, Windsurf, Sublime Text or
Xcode when they are installed on this Mac. In a project, Add project script
opens the script editor for a named command you can reuse.

### Antigravity can use Hollow's tools and your shared connectors

- **What:** an Antigravity chat gets the same Hollow tools as Claude and Codex (app, tasks,
  vault, questions) and the connectors you share with it, such as Linear. Each call shows an
  Allow card unless the chat is in Skip permissions. Automations and teammates still run without them.
- **Where:** Settings, Connectors: tick Antigravity on a connector to share it; the chat's connector menu.
- **Undo / turn off:** untick Antigravity on the connector; `HOLLOW_AGY_HEADLESS=1` keeps every turn on the old path.

### Antigravity asks before it acts, and Auto works

- **What:** in Accept edits, an Antigravity command now shows an Allow or Deny card instead of
  being silently refused. Auto is available again (Antigravity's own reviewer: edits in the folder
  and safe commands go ahead, writes outside it are refused). The usage ring shows the real context
  window. One-off runs (automations, teammates) still run without cards.
- **Where:** the composer's permission menu and the chat's approval cards.
- **Undo / turn off:** Deny the card; `HOLLOW_AGY_HEADLESS=1` in Hollow's environment keeps every Antigravity turn on the old path.

### Import a VS Code theme and choose your fonts

- **What:** Edit themes can import a VS Code colour theme file. It opens in the theme editor as a new theme so you can check the colours; nothing is saved until you Save. Text font and Code font each take Hollow's default, a system look or a font name you type (it is used only if installed).
- **Where:** Settings → General → Appearance → Theme → Edit themes… → Import VS Code theme…; Settings → General → Text font and Code font.
- **Undo / turn off:** delete the imported theme in Edit themes; set a font back to Hollow default.

### Regenerate a chat's title

- **What:** asks the chat's own agent for a new title from its first message and replaces the current one, a rename included. Hollow's own messages keep their labels.
- **Where:** a conversation's menu (the row's actions, or the dots beside its title) → Regenerate title.
- **Undo / turn off:** Rename… puts any title back.

### Update every agent at once

- **What:** Update all runs each agent's update one after another when two or more have one. A version Hollow knows breaks things is left out and asks first on its own row.
- **Where:** Settings → Accounts → Agent updates → Update all.
- **Undo / turn off:** Restore on an agent's row goes back where its install method allows.

### Settings for diffs, links, worktrees and archived chats

- **What:** Diffs: wrap long lines and show line numbers. Browser: web links open in the Preview pane or your default browser (hold ⌘ or Shift for the other). Worktrees: what build workspaces left on disk, and a clean-up of the finished ones. Archived: settled chats with Restore.
- **Where:** Settings → Diffs, Browser, Worktrees and Archived.
- **Undo / turn off:** each switch or choice changes back the same way; Restore returns an archived chat to its project.


### Antigravity shows usage, context and estimated spend

- **What:** an Antigravity chat now fills the context ring, lists last-turn tokens and an
  estimated cost at Google's published Gemini rates, and can have a token budget. The prepaid
  credit balance isn't reported by Antigravity: the usage menu links to AI Studio for it.
- **Where:** the usage ring in the composer; Settings, Usage; Budgets.
- **Undo / turn off:** nothing to switch off. Spending budgets stay unavailable for Antigravity.

### One effort control for Antigravity

- **What:** the model picker lists Gemini 4 Neon, Argon, Carbon and Hydrogen once each,
  and the Effort row sets the level. It shows only the levels the model has (Neon has
  Extra high, the others stop at High). A saved "Gemini 4 Neon (High)" becomes Neon at High.
- **Where:** the composer's model menu, Antigravity.
- **Undo / turn off:** pick another level; nothing to switch off.

### Run actions and pick rows by number in the command palette

- **What:** type `>` first in the command palette (or click the new Actions chip)
  to list only actions and pages. Backspace in an empty box steps back, and the
  Back button does the same. With the palette open, ⌘1 to ⌘9 choose the row
  that number labels; closed, ⌘1 to ⌘5 still go to the pages.
- **Where:** ⌘K, then `>` or the Actions chip; the numbers sit at each row's right end.
- **Undo / turn off:** Escape closes the palette; nothing to switch off.

### Live timers and bottom shelves in the sidebar

A chat or project that is working shows how long it has been running, ticking
each second. The Working, Snoozed and Settled groups now sit at the bottom of the
sidebar, below your pinned and recent rows.
### Snooze with quick times, and undo Pin and Wake

Snooze until offers in an hour, in three hours, tomorrow morning, this weekend
and next week, or a date and time picker. Settle, Snooze, Pin and Wake each show
an Undo for 5 seconds; Edit → Undo Chat Action does the same without a shortcut,
so text fields keep their own undo.

### Motion on running tools and the preview loader

- **What:** running tool names shimmer, tool details open and close smoothly, and a preview shows a loading bar while it loads.
- **Where:** the tool rows of a transcript and the Preview pane.
- **Undo / turn off:** Settings, Appearance, panel animation to 0 (or reduced motion on the Mac) holds all of it still.

### Import Pi history and pair your phone from welcome

Import Pi conversations from their local session folder. The latest branch
stays intact and other branches remain in the source. Scan limits and partial
history are visible. Open Set up your iPhone on the welcome page to use the
existing pairing QR and device controls.

### One welcome page for Hollow

See agent readiness and Details together, browse the ACP registry with Add
more agents, or Start without an agent. Optional imports, context and sources
stay within reach. Reopen from Settings, About, Open welcome page.

### Drag conversations between sidebar sections

- **What:** drag a local chat in the sidebar onto Pinned, Active or Settled. Each target names the exact menu
  command it runs, so Undo and the pin rules are the same as the menu's. A pinned chat is not settled until it is unpinned.
- **Where:** the sidebar while you drag a chat row; the row's menu (Pin, Unpin, Settle) does the same by click.
  Rows glide to their new place unless Settings turns panel animation off or macOS asks for reduced motion.
- **Undo / turn off:** Undo on the notice. Nothing moves unless you drop.

### See row details and drop files onto a chat

- **What:** hover or focus a local sidebar chat to see its title, status, turns, last activity, folder and where it
  sits. Drop files on a chat row to attach them to that chat's composer; nothing is sent. Remote chats say they cannot take files.
- **Where:** a sidebar row (hover, focus, or its menu → Details, Close details to dismiss); drop files on the row.
- **Undo / turn off:** Remove the attachment in the composer. Details closes with Escape or Close details.

### Shrink the Preview pane to a mini player

- **What:** the Preview pane can become a small floating player that keeps its page, tabs and media running, and an
  agent that inspects a page you shared marks the match it found with an Agent inspected cursor. The cursor never clicks or types.
- **Where:** the Preview header's Mini player; Expand brings the full pane back and Close or Escape closes it.
- **Undo / turn off:** Expand. The cursor appears only while you share a tab with the chat and clears on the next view.


### Accept or refine a reported plan

When an agent explicitly proposes a plan in Plan mode, the live reply offers
Review the plan, Accept plan and Refine plan. Either action prepares a message
in the composer for you to review and send. Your current draft and permission
mode stay in place. Older proposals stop offering actions after a later message.
### Expand images and attach HEIC photos

Click a local reply image or an attached image to zoom, fit it to the window or
view its actual size. Open in Preview keeps the existing file route. Close or
Escape returns to the conversation. HEIC attachments convert to PNG on this Mac;
invalid or oversized converted images say why they could not attach.
### Read math, alerts and linked conversations in replies

Replies render inline and block math, GitHub note, tip, important, warning and
caution blocks. Conversation links appear as chips; click one to open that
exact conversation. File chips and remote image consent keep their usual routes.
### See context handoff preparation

A fresh follow-up or retry shows Context handoff pending, then ready or
unavailable. Open its details to see how many earlier entries were prepared.
Ready describes preparation, not confirmation that the agent read it.

### Stash a draft or send in background

- **What:** the + menu can stash a draft in its chat, restore it beside newer
  text, or send and open a fresh chat while the accepted turn continues.
  Stash text is saved; attachments stay in this window. Large text pastes
  attach as a file, and chips show reading progress separately from saving.
  Limit notices point to recovery controls. Project heroes offer a quick chat.
- **Where:** message box → + → Stash this draft / Restore draft / Send in background.
- **Undo / turn off:** restore a stash from +; remove a pasted file's chip;
  reopen the background chat from the sidebar and use Stop if needed.

### Write messages with live Markdown

- **What:** the message box can render Markdown while you write, using the same
  editor as notes. Enter follows your send setting; mentions and dictation stay available.
- **Where:** message box → + → Use live Markdown.
- **Undo / turn off:** + → Use plain text. Your draft stays in the box.

### A banner and dock badge when a chat finishes or needs you

- **What:** When a chat finishes, stops with an error or is waiting on you, Hollow shows one macOS banner and puts a number on its dock icon (chats waiting on you, plus finished chats you have not opened). It stays quiet for the chat you are looking at, for subagents, and for a burst of chats (they become one banner).
- **Where:** Settings → General → **Chat announcements**. The banner also needs **macOS system notifications** on.
- **Undo / turn off:** Turn Chat announcements off: the banners stop and the badge clears.

### Panel animation: choose how long panels take to move

- **What:** Panels (the side panel, the sidebar) can slide open and closed over a time you choose, from off to 400 ms. It is off by default. Reduced motion and page changes always skip it.
- **Where:** Settings → General → **Panel animation**: drag the range, or press **Turn off**.
- **Undo / turn off:** Press Turn off (or set it to 0). Your Mac's Reduce Motion setting and Settings → General → Motion also turn it off.


# What's new: 2026-11-19

### Read reported thinking when available

- **What:** When an agent reports completed reasoning text, a Thinking row keeps it separate from the answer.
- **Where:** Click Thinking in the conversation, or inside a finished run's expanded steps.
- **Undo / turn off:** Click again to collapse. Rows start collapsed; agents without reported text show no row.
### Live work time and quieter warnings

- **What:** Running chats say Working for with elapsed time and current activity.
  Host warnings have a short expandable row; identical repeats show a count.
- **Where:** In the conversation. Click a warning row to read the full detail.
- **Undo / turn off:** Click the row again to collapse it. Original messages stay intact.

### Mention files and skills while writing

- **What:** type @ to choose a project or conversation file, or $ to choose a
  Library skill. File chips open Preview; selecting a skill adds its read-first
  request. Escape closes suggestions and keeps your text.
- **Where:** the message box, or + → Mention a file / Use a skill.
- **Undo:** edit the inserted text or remove a file chip. Nothing sends until Send.

### Long chats stay light, and show what is new

- **What:** Reading back through a long chat draws at most 200 turns at a time. Scrolling up loads earlier turns as you reach them, scrolling down loads later ones, and the turns you scrolled past drop out, so a 1,000-message chat read back to its start uses a third of the memory (21 MB over Home, not 59) and typing stays instant (7 slow key presses, not 123). Show earlier and Show later buttons load them too, and Jump to latest returns to the end. A chat with messages that arrived while it was closed shows a line before the first new one, "N new messages since you were here", with Dismiss.
- **Where:** Any long chat: **Show earlier messages** at the top, **Show later messages** at the foot once you have read back, **Jump to latest**. The new-messages line sits in the chat; **Dismiss** is on it.
- **Undo / turn off:** Nothing to turn off. Dismiss hides the line for that visit; the next visit shows one only for messages that arrived after it.


### Calmer loading and measured disclosures

- **What:** Settings help folds resize smoothly, Library loading uses row-shaped placeholders, and sidebar search shows a progress line. Active reads and drag destinations get quiet cues while idle content stays still.
- **Where:** Settings help, Settings search results, Library All and sidebar Search chats. Drag a pinned project or a conversation onto a project to see the destination label; the row menu offers the same move actions.
- **Undo / turn off:** Set panel animation duration to zero in Settings, General, Appearance, or enable Reduced motion. Text, focus and loading status stay visible.


### Agents can block a site for themselves, and only you can unblock it

- **What:** An agent that finds a site it should not read or open (a login page, a tracker) can add it to its own deny list with no approval, because that can only narrow what agents reach. Up to 100 sites; a typed or pasted address reduces to its host. The list is read at every check, so a block holds on the very next call. Only you can remove one.
- **Where:** Settings → Connectors → Computer control → **Set up** → Sites agents blocked for themselves → **Remove**. No keyboard shortcut and no command-palette entry: the list lives in that sheet.
- **Undo / turn off:** Remove puts a site back at once. Hollow's built-in webmail and banking list and your own host list are unchanged.


### Large task lists open fast

- **What:** A group on Tasks draws its first 60 rows and adds 60 more as you scroll or when you press Show more. A list of 5,000 tasks opened in 283 ms with a stall each time; it now opens in 40 ms. The row you have open, or move to with j and k, is always drawn.
- **Where:** Tasks → the end of a group with more than 60 rows → **Show more** (scrolling to it does the same).
- **Undo / turn off:** Nothing to turn off; groups of 60 rows or fewer look as before.


### Set up Hollow in five short steps

- **What:** After you choose the Hollow folder and restart, a setup sheet opens over the app: connect your context (the vault and your other context layers), find what's already here (import your Claude Code and Codex conversations, and scan for instruction and memory files), where your work lives (tasks stay in the vault unless you connect a task tracker), which agents are installed and signed in, and a first workspace or Chats. Every step can be skipped, and Back returns to the one before.
- **Where:** It opens by itself after the first folder setup. Settings → Sources → Where your work lives → Run setup again → **Start setup** opens it again at the first step. People who set Hollow up before this see nothing new.
- **Undo / turn off:** Close the sheet to finish later; it opens at the first step you haven't done or skipped on the next launch. Nothing is imported or copied until you click Import or Accept, and Run setup again undoes nothing you set.

### Review the instructions and memory found on this Mac

- **What:** Hollow can look through your Claude Code, Codex, Cursor, Gemini and Windsurf folders for instruction, rule, command, skill and memory files, and list them on one card together with the org facts learned in chats. Each item shows where it was found, a short excerpt (never for a file that looks like it holds a secret), any scan flags and a suggested layer: Personal copies it into your personal context, Project leaves it in that project's own repo, Organization opens a pull request after you confirm.
- **Where:** Library → Learnings → Found context → **Scan this Mac**. Each item has Accept, Change layer and Skip. There is no accept-all: every item is its own click.
- **Undo / turn off:** Nothing is copied until you accept one item, and a copy never replaces a file already there; delete it from `memory/imported/` in your personal context to take it back. Skip moves an item under Skipped and Unskip brings it back. An organization pull request is closed on GitHub.

### Found Claude Code memories get a suggested layer and a reason

- **What:** On the Found context card, each Claude Code memory entry (the per-project memory folders Claude Code keeps) now has a suggested layer and one line saying why: Home for facts about this Mac or another machine, Personal for how you like to work, Project for facts about one codebase, or Stale for an index file, an empty entry, one marked superseded or an old note about something passing. Home copies the entry into this Mac's own folder in your personal context. Stale suggests Skip. Project writes nothing: it needs that project's lead.
- **Where:** Library → Learnings → Found context → **Scan this Mac**, then the line "Why this layer" on each memory entry. Change layer still offers every layer.
- **Undo / turn off:** Nothing is written until you accept one entry, and a copy never replaces a file. Delete a Home copy from `home/<your machine>/` in your personal context to take it back.

### Computer control: a settings sheet, chat controls and a Doctor

- **What:** Settings, Connectors, Computer control has a **Set up** sheet: the switches for the sandbox Mac and This Mac's screen (off until you turn it on), the apps agents can never use on this Mac (terminals, System Settings, Keychain Access, password managers and Hollow itself) with an Allow and Block for each, **Stop all computer control**, and a Doctor that lists what is set up and what is not, with a fix to copy. In a chat that has used the desktop, a Desktop chip beside the message box and a Computer section in the side panel show what the agent last saw and its recent actions, with Pause, Take over, Hand back and Stop. The menu bar icon has Stop all computer control. A send, a payment, a delete or a password always asks, in every mode.
- **Where:** Settings, Connectors, Built into Hollow, Computer control, Set up; the chat's message box; the side panel's Computer section; the menu bar icon.
- **Undo / turn off:** Turn Computer control off in the same row, or press Stop. Nothing runs on a desktop until an engine is set up; no Cua Driver ships with Hollow, and none has run against this yet.


# What's new: 2026-11-18

### Undo an agent's turn in a build chat

- **What:** In a chat that has its own git worktree (no other chat works in that folder), Hollow keeps a hidden checkpoint before and after each turn. **Restore files to before this turn** puts the worktree back as it was before that turn and undoes it and every later turn. The agent then starts a fresh session without the undone turns; they stay in the transcript, and a note in the chat says what was undone.
- **Where:** The reply's More menu (the dots under a reply), in a chat working in its own worktree. Chats in a shared folder or a repository's main checkout don't show it.
- **Undo / turn off:** Before restoring, Hollow saves the current state in a git ref (`refs/hollow/checkpoints/<chat>/restore-<time>`, named in the chat's note), so `git checkout <ref> -- .` brings it back. Files your `.gitignore` covers are never touched. Nothing to turn off: checkpoints are only taken in a chat's own worktree.

# What's new: 2026-11-17

### Decision cards show a scenario and what each option does

- **What:** When an agent asks you to decide, the question card can show one short example with real numbers, and under each choice a line saying what that choice does in the same example. The recommended choice still carries the one reason for it.
- **Where:** The question card over the message box, when an agent asks you something. Questions without an example look the same as before.
- **Undo / turn off:** Nothing to turn off. Answer or dismiss the card as usual; your answer is sent as before and does not repeat the example. The phone and peer views show the choices' labels only for now.

# What's new: 2026-11-15

### Leave notes on a file's lines and send them as one message

- **What:** Read a plan or a generated file once, leave a note on each line that needs work, and send them all to the chat as a single message for one revision turn. Notes stay with their line when the agent edits the file; a line that was changed or removed is flagged "Line moved or removed" and its note can still be sent with the words it was on. Resolve a note when it is handled; resolved notes are never sent.
- **Where:** Preview pane → a text or Markdown file → **Review notes**. Choose the plus beside a line, write the note (Ctrl or Cmd+Enter saves, Esc cancels), then **Send N notes to chat** at the bottom. The message is added to your draft; nothing is sent until you press Send.
- **Undo / turn off:** Resolve or Delete a note, or choose Preview or Source to leave the notes view. Notes are kept on this Mac per file (the newest 200 notes on each of 40 files). The view numbers the first 1500 lines of a long file.

### See how well each shared skill is written

- **What:** Every skill in your shared library gets a "Lint" score from 0 to 100, with the fixes that would raise it: a missing "Use when" sentence, a too-short or over-long description, a body that is too big, scripts with no prerequisites, hidden characters, or text that looks like a stored secret or an instruction override.
- **Where:** Library → Skills. The score sits on the row; open a skill to read its Skill check.
- **Undo / turn off:** It only reads the skill files and changes nothing. There is nothing to turn off.

### Pin, snooze and settle conversations

- **What:** Pin keeps a chat handy. Snooze until takes a local date and time. Settle archives it with a reason and Undo; active work is protected.
- **Where:** A conversation's actions menu. The sidebar's Pinned chats, Snoozed and Settled sections keep Open and Wake available. Automatic settlement is off by default; choose seven idle days or the verified open PR of the chat's build worktree. The policy applies once and checks while Hollow is open.
- **Undo / turn off:** Wake opens a snoozed or settled conversation. Unpin returns a pinned chat to its ordinary list. Settle and Snooze also offer Undo.

### Fold background work in the sidebar

- **What:** Running and queued local rows move into counted Working folds. Chats needing attention and paused chats stay above recent rows; the current row stays outside Working.
- **Where:** Sidebar → Working under projects or Quick chats. Project pins keep their place.
- **Undo / turn off:** Open either Working fold to show its rows. The choice is remembered.

### Open sidebar actions with a click

- **What:** Project and quick-chat rows have an actions button, including chats on another Mac.
- **Where:** Sidebar → a row's three-dot button. Right-click still opens the same menu.
- **Undo / turn off:** Close the menu with Escape or click outside it. Opening it changes nothing.

### Keep working on a moved chat while its Mac is away

- **What:** A chat that moved to another Mac now offers **Continue here offline**: a copy on this Mac, badged "offline copy", whose new turns sync there once that Mac answers, into the chat or as a chat of their own next to it. The copy says if it stayed here, and why. Long offline stretches sync in parts.
- **Where:** A moved chat's notice above the composer → Continue here offline…
- **Undo / turn off:** Nothing to turn off: the copy syncs only the turns you add to it, and the original keeps going on that Mac.

### Move a chat to your runner Mac

- **What:** Once a paired Mac takes chats (your runner, or the only Mac paired), a chat's menu has **Move to runner**. It asks first, then sends the chat's history once; a running turn stops at its next safe point and keeps going there. The chat then lists once, on that Mac, with a "moved" badge, and its copy here says where it went, with Open. When that Mac doesn't answer, the chat says it is moving and offers Keep it here, or Release here if that Mac is gone (that Mac may keep a copy). Forget this Mac now says which moving chats it releases; chats that already moved there stay there. A new setting moves running chats to the runner when this Mac sleeps. It is off until you turn it on, because the runner's own logins pay for those turns.
- **Where:** A conversation's ⋯ beside its name (or a Quick chats row's right-click) → Move to runner. Settings → Devices → Macs → Move running chats to the runner when this Mac sleeps.
- **Undo / turn off:** Keep it here while a chat is still moving. A chat that moved can't be brought back yet. Turn the setting off.

### Choose whether this Mac's COO sends long work to your runner

- **What:** When a Mac you paired runs a runner COO, the COO on this Mac becomes a dispatcher: it answers quick questions itself and sends long work to that Mac, asking you on a card first. A new setting keeps it the full COO here instead, as with no runner COO paired. The row also says what the COO is doing now: which Mac it sends work to and since when, or why it isn't.
- **Where:** Settings → Devices → COO on this Mac, shown once a paired Mac runs a runner COO: Dispatcher when a runner COO is paired (the default) or Always the full COO.
- **Undo / turn off:** Pick the other choice. The next COO turn uses it.

### Open a chat on your Mac mini

- **What:** Pair this Mac with another Mac that runs Hollow, such as your Mac mini, and open its chats here. A strip above Shared tools in the sidebar shows each Mac, whether it answers and how many chats run there; a Mac opens a list of its newest chats. A chat opens read live: you can send it a message, stop it, and answer its card, all at its own Mac. Its quick chats list under Quick chats with a small badge. Nothing else crosses: no files, tasks, notes or settings, and nothing about its chats is kept on this Mac.
- **Where:** Settings → Devices → Macs → Pair a Mac (an SSH alias that reaches the other Mac). Then the sidebar's machine strip → the Mac → a chat. On the other Mac, Let another Mac connect shows a code, and Revoke unpairs.
- **Undo / turn off:** Forget in Settings → Devices → Macs, or Revoke on the other Mac.

### An automation runs on one Mac

- **What:** An automation can name the one Mac that runs it, so once a second Mac shares the vault each automation runs once, on its own Mac. One with no owner keeps running as before until another Mac is paired with this one; after that it runs nowhere and Hollow sends a notification saying so, once. The automations list says when one runs on another Mac, or on no Mac and how to fix it. Moving an automation to a Mac is a permission change you approve at Save, because each Mac has its own logins. Renaming this Mac keeps its automations.
- **Where:** Automations → an automation → Automation options → Edit → Inputs & scope → Runs on: Any Mac that holds it, This Mac, or a paired Mac. An agent can set it too (save_automation with runsOn "this", another Mac's name, or empty), and so can `runs_on: <machine name>` in the file, approved like any change.
- **Undo / turn off:** Set Runs on back to Any Mac that holds it and save.

### Update Hollow from one button

- **What:** One place to update. When a new Hollow is downloaded and checked, an **Update** button shows in the top bar. Its popover names the version and channel (Main or Nightly) and what's new. **Restart to update** installs it and reopens Hollow when nothing runs; while a chat or automation runs it offers **Update when chats finish**, which waits as long as they take (no 30-minute limit any more) and can be cancelled from the button. **Not now** hides that version. A branch you build for testing (Changes → Hollow app → Build app) waits in the same button as **Local build of <branch>**, with its commit and what it brings, and installs the same way. The build workspace's Install and restart card and its banner are gone. An agent can still propose installing a branch build, but only from a card you allow, and it waits in the same button.
- **Where:** The top bar's Update button. Changes → Hollow app → Show in Update button puts a finished build back after Not now. Settings → Updates for the channel.
- **Undo / turn off:** Not now hides the button until the next version or build; Cancel stops a waiting update. The previous app stays at `/Applications/.Hollow.app.previous`.

### Check a repo for secrets before you push

- **What:** Install context check adds a local check to a repo (never committed) that stops a push carrying a secret, such as an API key. For the first week, a project doc that names your Mac or you is saved and noted for review, not refused; a secret is always refused. A Monthly context audit template checks every context and project repo and reports in Hollow.
- **Where:** Settings → Sources → Context → Context check: Install context check or Remove per repo. Automations → + New automation → Monthly context audit.
- **Undo / turn off:** Remove takes the check out of that repo. The audit runs only once you save it as an automation.

### Choose between two Macs' versions of a file

- **What:** When two Macs change the same lines of a synced file, this Mac keeps its version and the other Mac's sits beside it as `Name (from <machine>).md`. Each such file is now one card in Waiting on you: both machines, when it happened, how many lines differ and the first lines of each version. Keep mine moves the copy to the vault's trash; Keep theirs puts the other Mac's version in place (only while your file is still as the clash left it); Keep both leaves both; Edit opens the file so you can merge by hand, then Done. Every other file keeps syncing while a card waits, and your choice syncs like any edit.
- **Where:** Waiting on you → Decisions → Two Macs changed <file>. Settings → Sources → Context → Personal or Vault sync → N clashes opens the same cards.
- **Undo / turn off:** The replaced version is in the vault's `.trash/`; move it back by hand.

### See and connect your context layers

- **What:** Settings has one row per context layer. Home names this Mac and finds its home file. Personal shows whether your me, memory and learnings folders are synced, when they last synced, and lets you Sync now, Stop syncing, connect your own private repo or keep them Local only. Organization shows the read-only company clone, its last pull and a Check access button. Project folders lists which projects carry a CLAUDE.md or AGENTS.md. Vault sync says Not set up; its first push is held.
- **Where:** Settings → Sources → Context. Create a private repo walks you through making one on GitHub, then Connect takes its URL (never one with a password or token in it).
- **Undo / turn off:** Stop syncing, or Local only, which forgets the remote and keeps every file. Use folder with another path changes a clone folder.

### An automation can end

- **What:** An automation can have an end: a date and time, or a number of runs. When the time passes, or the last run finishes, Hollow sets it to manual, writes a note saying when and why, and it never starts on its own again (you can still run it by hand). The Automations list shows "ends Fri 6 PM" or "ends after 3 runs" beside the schedule, and "ended" once it has stopped. Agents set it with `ends` or `afterRuns` on save_automation.
- **Where:** Automations → an automation → Edit → Schedule → Ends at, or Or after this many runs.
- **Undo / turn off:** Clear both fields and save. To run it again after it ended, pick a schedule and save.

# What's new: 2026-11-14

### Quick chats are back

- **What:** New chat and ⌘N (with no project open) start a quick chat outside any project instead of opening the COO. Quick chats list in the sidebar under Projects, newest first. Move one into a project from its right-click menu (Move to project…): it keeps its history, lists under that project lead's Subagents, and the lead gets a note that it arrived. When a quick chat's reply is clearly one project's work (it names the project's folder, or its name twice), a **Move to <project>** button shows under the reply. The COO keeps its orb.
- **Where:** Sidebar → Quick chats (its + starts one), or Home's Ask with New chat.
- **Undo / turn off:** Fold the section with its heading; Not now hides a Move offer. The COO still opens from the orb.

# What's new: 2026-11-12

### Move Hollow home into one real folder

- **What:** Settings can now move your vault and your Projects folder into one real Hollow folder (`Vault`, `Projects`, `Projects/Personal`, `Repositories`, `Worktrees`, `Archive`), out of Documents or iCloud. Check the move changes nothing: it shows every folder that moves with its size and file count, the links left at the old places, the records updated to the new paths, name clashes and which copy keeps the name (the other goes to Archive), files that are in iCloud only (a folder in iCloud is copied out, not renamed, once its iCloud-only files are downloaded), chats and apps that must stop first, git worktrees to repair, agents' records, login items and scripts, and whether a backup covers the folder. Also move the folders beside the vault adds them to Projects. Quit and move stays disabled, with the reason, while another chat runs, and asks once more before Hollow quits, moves on the same disk and opens again. The panel then shows each step and the checks after the move.
- **Where:** Settings → Sources → Hollow folder → Move Hollow home: Check the move, Quit and move…, Roll back…. Agents can preview it and, with a card you approve, run or roll it back.
- **Undo / turn off:** Roll back… puts every folder and record back while nothing new has been written to the moved vault; if something has, it says what to copy out first.

### See what subagents are doing

- **What:** Running subagents show their current tool and file or command, recent steps, reported plan progress, time, tokens and cost. Swarms and teams stay together. Queued work shows its place; finished work shows its reported outcome and recorded files. Swarm parts use their task names. A click still opens the full conversation. In a subagent's transcript, Copy and Read aloud appear once a reply has finished (no half answers), reading stops when the chat hands over, and a vote counts for the run it sits under.
- **Where:** A chat's summary card → Subagents.
- **Undo / turn off:** Close the tab. Collapse a team to keep it compact. Ambient motion and Reduce Motion follow Settings → General.

### Subagents per chat is a setting, and waits name their limit

- **What:** Settings → Chat → Chats at once has a third control, Subagents per chat (1 to 10, default 6): how many subagents one chat, the COO or a project lead runs at once. It was a fixed six that wasn't shown anywhere, so a COO's starts could queue while far fewer chats than your overall limit were running. A queued subagent now says what it waits for, with the number and the setting: "Waiting for a free subagent slot: this chat is at its limit of 6 subagents at once (Settings → Chat → Chats at once)". The overall and per-workspace waits read the same way. Swarm parts keep their own count under Settings → Automation. The COO can change all three limits when you ask, with a card to approve.
- **Where:** Settings → Chat → Chats at once. A queued subagent's row and header in the Subagents tab, the chat's Queued badge, Mission control and the run queue.
- **Undo / turn off:** Set Subagents per chat back to 6.

### Each agent's access modes are its own

- **What:** A chat's access chip now lists its agent's own modes: Claude Default, Accept edits, Plan, Auto and Bypass permissions; Codex Read only, Ask for approval, Approve for me and Full access; Grok and Antigravity their own. Switching agent maps the mode to the nearest one and says so in one line, and drops an effort level the new agent doesn't have. The access menu keeps its focused row when a turn ends. Settings has a mode for new chats per agent (Auto for Claude and Codex); the single new-chat setting is gone. Codex Auto now lets build chats commit and reach the network, and wakes, check-ins and handovers keep the chat's own mode.
- **Where:** Any chat's access chip. Settings → Chat, new-chat mode per agent.
- **Undo / turn off:** Pick another mode on the chip, or another new-chat mode in Settings.

### One compact usage ring in the message box

- **What:** The old context explanation popover is gone. The usage ring shows one Last turn heading (input, output, cost) with cache reads under Cache, and one compact split labelled "Latest message · recorded text" ("Breakdown unavailable" when there is no record). Folder and connectors is a two-line menu with a Settings link.
- **Where:** The ring and the Folder and connectors button in the message box. Context details opens Settings → Advanced → Search & indexes, which keeps tool estimates, the largest results and the recorded manifest.
- **Undo / turn off:** Nothing to turn off.

### A second Linear team, Personal

- **What:** Hollow can now sync and write a second Linear team beside Engineering: your own Personal (PER), opt-in. Other teams stay refused. Settings shows both teams; Tasks has a team filter and a team chip on each issue; new issues default to Engineering.
- **Where:** Settings → Task tracker; Tasks → team filter.
- **Undo / turn off:** Leave the personal team unset in Settings and only Engineering syncs.

### Library is one list

- **What:** Library's tabs are now All, Conversations, Connected projects, Skills, History and Learnings. Notes, Results and Recordings left the tab strip and open from All, with everything they had: beside the search, Browse folders opens Notes (collections, folders, Records, saved views, the reader and editor) and New note starts a note from a template; with Outputs on, Grid opens Results as a grid (Files or Canvases, by workspace, with review and Compare versions); with Meetings on, Recordings opens your Plaud recordings. Each shows Back to All at the top, which returns to the filter you came from. Links into a note, a canvas or a recording still land where they did. If Library last opened on Notes, Results or Recordings, it now opens on All.
- **Where:** Library: All, the buttons beside the search and beside the filters. The command palette: Library: Notes, Library: Results and Library: Recordings.
- **Undo / turn off:** Nothing to turn off. Back to All, or the All tab, returns to the list.

### Library opens on All, one searchable list

- **What:** Library has a new first tab, All: one list of your notes, meeting notes, people, recordings, result files and canvases, newest first. Type to search titles and folders (By meaning too, when search by meaning is on). Recent shows the last 30 days, with Show older; Meetings, People and Outputs narrow it. A click previews the item in the side panel; Open takes a note to Notes to read and edit it, or a result file to its workspace. A recording opens in Recordings and a canvas in Results. If one source can't be read, it says so with its own Retry and the rest of the list stays. Nothing in your vault moved: All is a view. Library opens on All unless you left it on another tab.
- **Where:** Library: All, the first tab. The command palette: Library: All.
- **Undo / turn off:** Nothing to turn off. Pick another tab and Library reopens on it.

### Side panel files by name, and every reply says when it finished

- **What:** The side panel card lists Outputs and Sources by file name with a small icon for the file's type, as a reply's file chips do; the full path shows on hover and is what a copy takes. Every finished reply's row now ends with when it finished, also a reply that came back without a run record. A subagent's message box is one rounded box like the chat's, with a round Send inside it. In a narrow window the side panel card floats over the chat instead of squeezing it, a narrow message box shows the model's mark without a one-letter name (hover still names the model), and the box's hint stays on one line.
- **Where:** The side panel card (⌘\ on a chat). The row under each reply. The Subagents tab, a subagent's conversation.
- **Undo / turn off:** Nothing to turn off.


### Small follow-ups start a fresh subagent with a brief

- **What:** When the COO or a project lead sends a short follow-up to a subagent whose conversation has grown large (300K tokens of context, or 70% of a smaller window its agent reports), Hollow starts a new subagent instead of continuing the long one. The new one gets the follow-up and a short brief: where the earlier chat left off, what was settled, and which files it changed. It runs on the same agent and model, in the same folder, and is told to make its file edits in one pass per file. Its row and header in the Subagents tab say Fresh chat with a brief, and hovering the row names the chat it came from. A longer message, a project lead's own chat and the COO's chat still continue as before.
- **Where:** A chat's Subagents tab (the Subagents chip above its thread). Agents choose it with send_to_session's fresh option.
- **Undo / turn off:** Nothing to turn off. An agent can pass fresh false to continue the long chat anyway, and the earlier chat stays in the list.

### Where everything in Settings went

- **What:** Settings was reorganised into twelve sections (General, Chat, Models, Accounts, Personalization, Connectors, Sources, Background work, Devices, Usage, Keyboard shortcuts, Advanced). Nothing that did something was lost; each row is in one place. A link beside the Settings title, Where things went, opens this list until 2026-11-26. Old links, the command palette and Settings search still land on the new place. The list, by where each row used to be:
  - Interface
    - Theme cards, custom themes, the themes folder: General, Appearance: one Theme menu, and Edit themes
    - Motion, Texture, Ambient motion, Density, Text size: General, Appearance
    - App and Main agent icons: General: App icon and COO icon (Upload an icon, Generate a logo)
    - Notifications, Proactive nudges, Scan now, Clear history: General, Notifications
    - Keyboard shortcuts: Keyboard shortcuts, in Commands and Go to keys tabs
    - Run at login, Keep running when I close the window: General, Startup
  - Agents
    - Default agent, When an agent runs out, each agent’s model and effort: Models
    - Remote runner, Runs on, Builds and tests, Overnight and worker chats, Runner GitHub access: Devices
    - App builds: Advanced, App builds and web backend
    - No-output watchdog, Run deadline: Advanced, Run limits
    - Extraction model: Advanced, Hollow’s own model calls
    - Agent updates: Accounts, Agent updates
    - Canvases, Enter sends, Retitle new chats, Permission mode for new chats: Chat
    - Claude accounts: add, edit, remove: Accounts, Claude
    - Spread usage, Move on at, Resume on another account, Check usage, In rotation: Usage, Limits
    - Codex, Antigravity and Grok sign-in; the Jev key: Accounts, Other providers and Jev
    - Self-hosted models: Models, Self-hosted models (Background jobs: Advanced, Hollow’s own model calls)
    - Skills: the catalog, New skill, Import skill: Library, Skills
  - Connections
    - Your connectors, Add connectors, GitHub, Microsoft 365: Connectors (one line each; Add a connector opens a sheet)
    - Service health table: Gone: each connector row shows its own state and Repair
    - Agenda calendar source and Calendar access: Connectors, Agenda calendar
    - Built into Hollow, Teammate roles: Connectors, Built into Hollow (Edit roles opens a sheet)
    - Recordings (Plaud): Connectors, Recordings
    - Pair a device, paired devices, the iPhone app, Tailscale: Devices
    - Keep Hollow running with the lid closed: Devices, iPhone app
    - Web app backend, push notifications key: Devices, Backend for the web app
  - Knowledge
    - Share with agents, Keep learning, Your profile: Personalization
    - Profile questions and Ready to add: Library, Learnings (a count stays in Personalization, Waiting for you)
    - Learning suggestions: Personalization, How many learnings to suggest
  - Automation
    - Scheduled automations: Background work (a link to Automations)
    - Automation windows, the run queue: Automations (a fold beside the schedule dial)
    - Chats running at once, Chats at once in one workspace: Chat
    - Silent heartbeat, the COO check-ins, Act on its own, the swarm: Background work
    - Main agent daily hard stop, Overnight cap: Usage, Spending caps (edited there only)
  - Usage
    - Today, This week, This month, the chart, the tables: Usage: All agents and one tab per agent, with Last 30 days
    - Claude and Codex plan usage: Usage, Limits
    - Budgets: Usage, Budgets
    - Minutes saved per accepted run: Usage, Value this month
  - System
    - Capability doctor, Backup evidence, Runtime, Disk space: Advanced, This Mac
    - Developer readouts, Debug logs, Diagnostics, Build identity: Advanced, Diagnostics
    - Hollow folder, Bring existing folders: Sources, Hollow folder
    - History & recovery: Library, History
    - Search inside project files, Search by meaning, the Jev checks: Advanced, Search & indexes
    - Prices, Export and Import settings: Advanced, Prices and Settings file
    - Start locally buttons, Help: All keys, Startup warm-up, System health: Gone: the File menu, the Help menu, the command palette and the Health check cover them
- **Where:** Settings: Where things went, beside the title, or Find a setting.
- **Undo / turn off:** Nothing to turn off. The link goes away on 2026-11-26.


### The message box shows its access mode

- **What:** The message box is one rounded box: what you type on top, then a row with + (attach files, Add to Sources and the commands) and an access chip at left, and the model, the mic and Send at right. The chip names what the agent may do in this chat: Full access (in a warning colour), Auto, Edits only or Plan. Click it to change the mode; Full access still asks first.
- **Where:** Any chat's message box: the chip beside the +. The model menu's Permissions row still has the same choices with each mode's description.
- **Undo / turn off:** Nothing to turn off. Pick another mode on the chip.


### Issues open full page, like Linear

- **What:** An issue can fill the Tasks page: title, description, sub-issues, relations, links, comments and activity on the left, every property in a column on the right. Sub-issues and related issues show their status, title and assignee, and the activity says who changed what in Linear and when ("Alex Rivera moved from Todo to In Progress, 2h ago").
- **Where:** Tasks, an issue: Open full page. Back to list, or Escape, returns to the list where you left it.
- **Undo / turn off:** Back to list. The issue opens beside the list again until you choose Open full page.

### Settings: one line each, and fifteen clutter rows gone

- **What:** Every Settings row now has one line of help, 90 characters at most, and the section hints under each title are gone. A row that can do harm (Hollow app tools, the COO acting on its own, the Jev switches, the App builds runner) keeps its longer help behind its own More. Fifteen things that were never settings left, and each one is still where it belongs. Where it went:
  - Start locally buttons (Create a workspace, Write a note, Add a task, Open Connections): the File menu, the command palette and Library's New note; first-run setup still offers them.
  - Help, All keys: the Help menu, the ? key, and Keyboard shortcuts' Show all keys.
  - Runtime capabilities: where an agent can't do something, its own control says why; Advanced, This Mac, Runtime lists each agent.
  - Danger mode active in this chat and Revert to Edits only: the composer's permission menu and `/permissions`. Settings keeps only the permission mode for new chats.
  - Keys inside a surface: the ? card lists them.
  - Startup warm-up: nothing was lost; Advanced, Diagnostics keeps Launch calls.
  - System health: Advanced, This Mac, Capability doctor, Run all checks.
  - The agent action ids under Disk space: the three cleanup buttons, which agents can still call.
  - The kinds-of-access explainer (Accounts and Connectors), How Hollow adapts to you, and the background-work explainer: each row's own hint; the background-work explainer stays on Automations.
  - Service health table: each connector's state chip and Repair.
  - The Debug logs on/off chip: Advanced, Diagnostics shows the log file count.
  - Local prioritizing, always on: Advanced, Search & indexes, Context prioritizer shows what each request kept.
  - Artwork preview: the app itself; Texture still sets it.
  - The Bake-off monthly cap was already gone with the bake-off runner.
- **Where:** Settings, every section. Old links and searches for a removed row land on the place above.
- **Undo / turn off:** Nothing to turn off. Everything that was a switch or a button elsewhere is where it was.

### Each setting in one place

- **What:** Effort in a chat's model menu now sets that chat only and is remembered with the chat; the default for every chat is in Settings, Models, Effort. A connector's state shows once, on its own row (the Plaud Recordings row no longer repeats the agents' connector state). The COO's daily hard stop and the overnight cap show no copy of the number outside Usage: their rows in Background work and Automations are a link to Usage, where the caps are edited. The heartbeat's on/off chip shows once. The learnings frequency control is now called How many learnings to suggest (the old name still finds it). The two Jev switches say "Needs a Jev key" with a link to the key in Accounts until one is saved.
- **Where:** A chat's model menu, Effort. Settings: Models (Effort), Connectors (Recordings), Usage (caps), Personalization (How many learnings to suggest), Advanced (Search & indexes, the Jev switches).
- **Undo / turn off:** Nothing to turn off. Pick Default in a chat's Effort list to follow the Settings default again.

### Skills, History and profile suggestions moved to Library; windows and the run queue to Automations

- **What:** Four things left Settings for the page they belong to. Skills (the catalog, New skill, Import skill) is in Library, Skills. History & recovery, the restore browser, is in Library, History; its Conversation history button is gone, because Library, Conversations and the sidebar's Search chats already open it. The automation windows editor and the run queue are on Automations, in a fold beside the schedule dial; Add automation from a window still opens the editor. The profile questions and suggestions (Ready to add, the one-at-a-time card, Review all) are in Library, Learnings, among what waits for you; adding one writes the same profile note. Settings keeps a row where each was (Skills are in Library, History & recovery is in Library, Automations, windows and the run queue are on the Automations page) and one line in Personalization: Waiting for you, with the counts. Old links and Settings search still find them.
- **Where:** Library: Skills, History and Learnings. Automations: Automation windows and the run queue. Settings: Connectors, Advanced (This Mac), Background work and Personalization keep the link rows.
- **Undo / turn off:** Nothing to turn off.


### Work is now Projects; the sidebar is shorter

- **What:** The page that was Work is called Projects, in the sidebar, the menu bar's Go menu, the command palette, the g go-to keys and the page's title. Typing "work" in the palette still finds it. The sidebar no longer repeats the Projects page: its project list, now headed Recent, shows your pinned projects and the 8 newest, with no Show more and no All projects row; every project, with search and archived ones, is on the Projects page under All projects. The sidebar's New chat is gone too: a prompt with no project goes to the COO, so the COO's page (the orb, ⌘⇧M) is where you start one. Search chats moved from the sidebar to a small search icon beside the sidebar's toggle in the top bar, with its own shortcut, ⌘⇧K. It opens the same search in the sidebar's place, showing the sidebar first if it is hidden, and Escape puts you back where you were.
- **Where:** The sidebar: Projects (⌘1) and Recent. The top bar: the search icon left of Back. Projects, All projects. The COO's orb, top right.
- **Undo / turn off:** Nothing to turn off. Change or clear ⌘⇧K in Settings, Interface, Keyboard shortcuts.

### The chat outline is a strip of dashes, as in ChatGPT

- **What:** A long chat shows a short dash for each prompt you sent in its left margin, and the dash for the part you are reading is brighter and longer. Point at the strip (or Tab to it) and a card lists your prompts; click one, or a dash, to go back to it. Esc closes the card. The Outline button above the chat opens the same card, and is the way in when the window is narrow.
- **Where:** Any chat with two or more prompts: the left margin of the conversation. The Outline button sits with Find, Copy and Save above it.
- **Undo / turn off:** Nothing to turn off. The strip hides in a narrow chat; the Outline button stays.


### Library has Skills and History tabs

- **What:** Library now has a Skills tab and a History tab beside Learnings. Skills is the skills catalog (it moved out of Settings): browse, find, make, import, edit and use a skill. History is the whole vault's History & recovery: every automated write, what it changed, and Restore. Library reopens on either tab if that is where you left it. The Settings copies are still there for now.
- **Where:** Library: the Skills and History tabs, after Conversations. The command palette: Library: Skills and Library: History.
- **Undo / turn off:** Nothing to turn off. Pick another tab.

### Usage shows every agent's limits and detail

- **What:** Settings, Usage now opens on Limits: each Claude account's 5-hour and weekly meters with the Move on at line, when each resets and its In rotation switch; Claude Code's login; Codex's plan windows; Grok and Antigravity (they don't report plan limits); your daily and weekly budgets as meters with their warn and hard-stop marks, monthly ones below; and the two spending caps, which you change right there. Spread usage moved here from Agents, Accounts, with Move on at, Resume on another account and Check usage. Read live from Claude replaces Show plan usage. Next come All agents and a tab for each agent you used, with totals, a chart by day, and usage by model, by account, by workspace, surface and automation, the ten costliest conversations (each opens its chat) and how runs ended, including runs that moved to another account. Last 30 days joins the periods. Budgets has its own tab.
- **Where:** Settings, Usage: Limits, All agents, Claude, Codex and the other agents you used, Budgets. Agents, Accounts still adds, edits and removes Claude accounts, with a link to Usage.
- **Undo / turn off:** Nothing to turn off. Your Spread usage, Move on at, In rotation and budget settings are unchanged.


### Settings search finds settings on other pages, and Library links to Search by meaning

- **What:** Settings search now lists settings that live on another page, marked "in Library" or "in Automations", and choosing one opens that page. A search hit inside a closed group opens the group and lands on the row. The capability doctor's fix buttons go to the row that fixes the problem (Hollow folder, Accounts, Recordings, Task tracker, Backup evidence), not the section you are in. Library's search shows Turn on search by meaning while that switch is off, and it opens Settings on the switch.
- **Where:** Settings, Find a setting. Library, the search box. Settings, System, Capability doctor.
- **Undo / turn off:** Nothing to turn off. Search by meaning stays off until you switch it on.

### A side panel card, and tabs like a browser

- **Compact sections:** Outputs, Subagents, Sources and Recall start closed with counts. Click a header to open just that section; Hollow remembers your choices. Recall titles open individually to show the full learning and its feedback buttons. Long sections scroll inside the card.
- **What:** On a chat with nothing open beside it, the side panel is a floating card: Outputs (the project's outputs/ files, or Create an output, which asks the chat for one), Subagents (a mark for each of up to four subagents, then "4 done" or "2 running, 5 done"; a click opens the Subagents tab) and Sources (the newest sources, + to attach one, View all to open SOURCES.md). Recall appears after Sources when the chat used recalled learnings, showing their statements and paths. Not relevant and Outdated add a correction request to your draft for review. The pane's tabs work like a browser's: each has its icon, name and ×, and + opens a New tab whose Tools are Files, Browser, Subagents and, in a build workspace, Changes; the one you pick opens in that tab. The Subagents tab lists Active and Done with each one's age, and a row opens its transcript with a back arrow and its model in the header.
- **Where:** The card is visible in each chat while the side pane is closed. It updates as subagents work and outputs arrive. The top bar has one pane toggle. In the pane: + after the last tab, then Browser or another tool.
- **Undo / turn off:** Opening the side pane hides the card; closing the pane brings it back. The card has no separate hide control.

### Upload your own icon, or have an agent draw one

- **What:** The icon maker is gone. For Hollow's Dock icon and the COO's icon you now pick a PNG, JPEG or SVG, see it previewed at every size (and on the COO's status badges), and click Apply icon. Or click Generate a logo: an installed agent (Codex when it has room, else Claude, else another agent) draws four dithered logo options; pick one to preview it, then apply it the same way. Generating uses your plan's usage; closing the dialog stops the drawing. The bundled app icon presets stay.
- **Where:** Settings, App and COO icons: Upload an icon or Generate a logo on each row. The COO: Actions for COO in the top bar, Change icon.
- **Undo / turn off:** Reset to Fold (app icon) or Reset to default (COO icon), on the row or in the icon dialog.

### Conflicts in Tasks, in one list

- **What:** When Linear changed a field you had edited, your edit is not applied and the issue shows Conflict. Tasks now gathers those issues: Conflicts (n) appears at the end of the view tabs while any issue has one and lists just those issues. Open one to re-apply your value (Use mine) or keep Linear's. With none left it says No conflicts and takes you back to all issues.
- **Where:** Tasks: Conflicts (n), beside Projects. Hover a row's Conflict flag for what happened.
- **Undo / turn off:** Nothing to turn off. The control goes away once every conflict is settled.

### A reply ends with its files, how long it worked, and a quiet row

- **What:** A finished reply's tool steps fold into one quiet line, "Worked for 1m 10s ›", from when the reply started to when it finished; click it to see the steps. The words written just before a question card stay on screen, and the reply's own text never folds. The files the reply changed sit as chips above that line, each with its type icon; hover one for its path, click it to open it beside the chat. Under the reply is one row: Copy, Useful and Not useful, Read aloud (click again to stop), Retry on the latest reply, ⋯ for the rest, and when it finished ("Sunday 11:27 PM"). Not useful asks for an optional one-line reason. Your vote helps Hollow find where it keeps falling short; there is no list of votes. Older replies show the row when you hover them. A subagent's transcript in the side panel has the same row (Copy, Useful and Not useful, Read aloud, the time), without Retry or ⋯.
- **Where:** Any finished reply in a conversation or chat.
- **Undo / turn off:** Nothing to turn off. Click the other thumb to change a vote.

### Run an automation inside a chat

- **What:** An automation can run inside one chat instead of starting a new chat each time. Each run is a new turn in that chat, answered by the chat's own agent and model at Edits at most. It waits behind an open question card or a turn that is still running, and never closes a card. If the chat handed over, the run goes to the chat it continued in; if the chat was archived or deleted, the run fails and its run log says why. Its usage counts toward the automation's budget, and an automation that runs in a chat uses the chat's tools, so it can't keep allowed or disallowed tools.
- **Where:** A chat's ⋯ menu: Schedule in this chat… opens a new automation bound to it. Automations, Edit, Inputs & scope: Runs in.
- **Undo / turn off:** Set Runs in to New chat each run, or delete the automation.

### Settings fits on a screen

- **What:** Each Settings section now fits about one screen. Theme is one menu (Match macOS, Light, Dark, Glass light, Glass dark, then your own themes); Edit themes opens a sheet for your themes and the editor. Larger tools open as sheets too: the runner setup, adding a connector (the catalog and the custom form), a self-hosted model, a Claude account, and teammate roles. Closing a sheet puts you back on the row you opened it from. Connector rows are one line each; click a row for its details. Keyboard shortcuts has two tabs, Commands and Go to keys. Heartbeat spacing and daily limit, check-in hours with working days, and Chats at once (overall and per workspace) each share one row.
- **Where:** Settings, General (Theme, Edit themes), Models (Set up again, Add a self-hosted model), Accounts (Add a Claude account), Connectors (Add a connector, Edit roles), Keyboard shortcuts (the two tabs), Background work and Chat (the shared rows).
- **Undo / turn off:** Nothing to turn off. Every setting is still there under its own name.

### Settings has a new list

- **What:** Settings lists twelve sections, each named for what you look for: General, Chat, Models, Accounts, Personalization, Connectors, Sources, Background work, Devices, Usage, Keyboard shortcuts and Advanced. Every setting is still there and works as before; only where it sits changed. Interface is now General (with Run at login), Knowledge is Personalization, Automation is Background work, System is Advanced, and Chat, Accounts and Keyboard shortcuts are sections of their own. Chats at once moved to Chat, Agent updates to Accounts, the task tracker and the Hollow folder to Sources, and the paired devices to Devices. Advanced keeps its rows in closed groups (This Mac, Search & indexes, Hollow’s own model calls, Prices, Settings file, Diagnostics). Old links and searches still land on the right row.
- **Where:** Settings: the list on the left. Find a setting opens the section, and the group inside Advanced, and lands on the row.
- **Undo / turn off:** Nothing to turn off.

# What's new: 2026-11-10

### Notion sync is removed; Notion stays readable as an archive

- **What:** Hollow no longer syncs projects and tasks with Notion, and Hollow's own Notion connector is gone. Tasks and projects are tracked in Linear: the Tasks page lists them, and Sync now there reads Linear. Notion's official connector in the connector library stays and works like any other connector once it is signed in. A canvas goes to an issue with Attach to issue instead of Add to Notion page. Create in Notion, Move to Notion trash, Linked to Notion and the Notion row in Settings are gone. Workspaces that came from Notion are ordinary workspaces now: rename, archive and merge them like any other. Nothing was deleted: your files keep their Notion links as history, and the stored Notion key stays until you revoke it.
- **Where:** Tasks: Sync now. Library, Results, Canvases, a canvas: Canvas actions, then Attach to issue. Settings, Connections, Your connectors: Notion's official connector, Sign in.
- **Undo / turn off:** Nothing to turn off. Revoke Hollow's old Notion key in Notion's integration settings when you no longer need it.

### Automations that start on an event, and leads that wake for new issues

- **What:** An automation can now start when something happens instead of on a schedule: a new issue in team ENG for a project, a merged pull request in a project's connected repository, or a recording or meeting ending. Each trigger is off until you turn it on, never fires for anything from before that, fires once per event even across restarts, and runs at most 6 times an hour unless you change the cap. A tracked project's lead can also wake on its own for each new issue, with a short message quoting it; that turn waits behind any open question card and never closes one.
- **Where:** Automations, New automation or Edit, When: Event. For a lead: right-click a tracked project in the sidebar, Wake the lead for new issues.
- **Undo / turn off:** Turn off beside the trigger's state in the editor, or click Wake the lead for new issues again.

### Hours saved count finished issues, and wait for one

- **What:** A run now counts as accepted when you accept a Result it wrote, or when the issue it worked on in Linear reaches Done after the run started. Each run counts once, and a reopened issue stops counting. Hours saved stay hidden until at least one run this month is accepted; then they say how many runs they come from and that they are an estimate from your baselines.
- **Where:** Settings, Usage, Value this month. The weekly update leaves its hours-saved line out until a run is accepted.
- **Undo / turn off:** Nothing to turn off. Clear a baseline to stop counting that kind of work.

### File links show the file's name and type

- **What:** A file a reply names shows as a small chip: the file's name with an icon for its type (Markdown, spreadsheet, Word, slides, PDF, web page, data, text, picture, code, folder), not its whole path. Two files with the same name show their folder. A click opens it beside the chat, even when the path was written relative to another project; when several files match, you pick one
- **Where:** Any chat's replies. Hover a chip for its full path; copying a reply copies the paths
- **Undo / turn off:** Nothing to turn off.

### A Friday status for each tracked project

- **What:** Once you turn it on, Hollow writes each tracked project's week every Friday at 3 PM: done this week, in progress, blocked or overdue, next up and the week's build log entries, from the issues Hollow mirrors (no model writes the facts). Each project's note goes in its outputs/status folder, the COO gets one digest of every project, the Herald picks up the digest's one-line summary, and one notification opens it. If you also turn on Status copy, the same status goes to Linear as each project's status update, once a week, team Engineering only; the note in Hollow is always written first.
- **Where:** Tasks, a project: Write status now. Settings, Connections, Task tracker: Friday status and Status copy.
- **Undo / turn off:** Both switches are off until you turn them on. A status note is a file like any other; a copy in Linear is removed there.

### Attach a canvas to an issue

- **What:** A saved canvas's menu has Attach to issue: find an issue, confirm, and Hollow attaches a copy of that version's HTML to the issue in Linear (team Engineering only). The canvas stays in Hollow, and attaching the same version to the same issue again adds nothing.
- **Where:** Library, Results, Canvases, a canvas: Canvas actions, then Attach to issue.
- **Undo / turn off:** Remove the attachment in Linear.

### One chat per project: its lead

- **What:** Each project in the sidebar is one row, and clicking it opens the project's lead: the one conversation for that project. There is no list of chats under a project any more, nothing to fold, no + and no ⋯; right-click a project for its menu. The lead answers questions about its project and hands building, research, planning and writing to subagents, then ends its turn; when every subagent it started has finished (or one fails), Hollow wakes it once to report to you and record decisions in the project's notes ("Updated my notes"). When a long lead conversation hands over, the same row opens the new one, with the earlier ones linked at its top. A lead can't use the shell and only edits its project's WORKSPACE.md, PLAN.md, SOURCES.md and AGENTS.md (enforced on Claude; other agents are told). A project's other chats are under the lead's Subagents chip, in Earlier chats, where you can read and message them. Chats with no project now go to the COO: New chat and Home's Ask open it, and your old chats with no project are under the COO's Subagents chip, in Earlier chats. Nothing was moved or deleted. Where Hollow used to call that folder Chats it now says COO: search results, Running, a conversation's About, a canvas's Saved with the COO, and Add to project's Keep with the COO and Move to the COO.
- **Where:** The sidebar's project rows. A lead's or the COO's Subagents chip above its thread, then Earlier chats. New chat at the top of the sidebar. Library → Conversations still lists every chat.
- **Undo / turn off:** Nothing to turn off. Every chat is where it was; open any of them from Earlier chats or Library → Conversations.

### Budgets for subagents and projects, and spend by where

- **What:** Settings, Usage can cap what unattended work uses, in tokens and working time rather than dollars: a budget for each subagent (its whole chain's tokens and minutes) and per-project budgets (tokens over the last 5 hours and the last 7 days, rolling like your accounts' windows). A subagent that reaches its budget pauses, and the chat that started it hears why and can raise it or stop it; a project that reaches its budget holds its unattended work until usage rolls out. Your own prompts always run. Usage also shows spend by project, by lead, by subagent and by account.
- **Where:** Settings, Usage, Unattended work: Budget for each subagent and Project budgets; the By project, By lead, By subagent and By account tables after By agent.
- **Undo / turn off:** Clear the figures (blank is no cap) or Remove a project's budget. Nothing is capped until you set it.

### A project's files open beside the chat, Results with their review

- **What:** The workspace panel and its sections are gone; their files open in the Preview pane instead. Project files in the top bar opens the project's WORKSPACE.md, and the pane's Files picker pins WORKSPACE.md, PLAN.md, SOURCES.md and AGENTS.md, then a build workspace's changed files, then the chat's. PLAN.md's checkboxes tick right there. A Result opens with a review bar: its state and checks, Accept, Compare with accepted and Ask for a revision (only you accept). A build workspace's branch opens its Changes; a Notion project's menu opens its mirror. Decisions are in Library under Records, with Revisit and Settle again. Overview's questions go to the project's lead, and earlier versions of a note are in History & recovery. The curated section canvases, Make a canvas from a file and Archive older entries went with the panel.
- **Where:** An open project: Project files at the top bar's right edge (or Open project files in the command bar). A Result: Library → Results, Needs you → Results, or a reply's changed file. Decisions: Library → Notes → Library folder → Records.
- **Undo / turn off:** Nothing to turn off. Hide the pane with Project files again, its ×, Esc or ⌘\.

### The Main agent is now the COO

- **What:** The always-on agent behind the orb is called the COO everywhere: the top bar, its page, Settings, the palette, the menus, the tray and what agents call it. Each project's own conversation in the sidebar is its Lead. The COO is your first stop for prompts, conversations and questions: it answers what a few reads answer and runs Hollow's actions itself, and hands building, research, planning and writing to the project's lead or to a subagent, then reports when the work comes back. That is how it works by default; it can still do anything you can in Hollow. It also runs on any agent you have installed: its model menu, /agent and /model offer every provider, like any chat.
- **Where:** The orb at the top (⌘⇧M), Settings → Automation → COO, Settings → Usage → Budgets (COO daily hard stop), Settings → Interface → App and COO icons.
- **Undo / turn off:** Nothing to turn off. Its conversations, standing orders and settings are unchanged; pick another model in its model menu to switch agent.

### See what a chat started, in a Subagents tab

- **What:** A chat that started other chats (its subagents) shows a Subagents chip above its thread, with their agents' marks and counts such as "1 running, 3 done". The chip opens a Subagents tab beside the chat: Active (running, then queued with its place) and Done, each row its title with when it ended at the right ("16h ago", or how long it has worked, ticking, while it runs) and a quiet line under it with its model and effort, how long it worked (hover for the whole span) and its tokens (hover for cached). Click a row for its whole session: its model, effort, time and tokens at the top right with what the CFO chose and why under the title, what it was asked, each answer with "Worked for", Copy and the time it finished, Show change in history where it changed files, Stop while it runs (it asks first) and a box to message it; a message to a busy one waits for its turn and says Queued. A "Started a subagent" line in the chat opens the same tab on that one. Subagents no longer show in the sidebar. The Main agent page's Chats I started list and its side band are gone: its day is one line under its status, and More holds Actions it took on its own, Change hard stop, the check-in settings and standing orders.
- **Where:** A chat that started a subagent: the Subagents chip above the thread, or a "Started a subagent" line in it. The Main agent page: More under its status.
- **Undo / turn off:** Nothing to turn off. Close the tab with its ×.

### Comment on an issue from Hollow

- **What:** An issue's Activity ends in a comment box. Your comment posts to Linear (team Engineering only) and joins the thread. A draft waits per issue until you post it, and a retry never posts it twice; if Hollow can't confirm a comment arrived, it says so and never sends it again.
- **Where:** Tasks: an issue, then Comment under its Activity.
- **Undo / turn off:** Delete or edit the comment in Linear.

### A Projects tab on Tasks

- **What:** Tasks has a Projects tab listing the team's projects like Linear does: status, lead, target date, progress (done over all its issues) and open issues. Opening one shows its page with its issues; an issue opens from there.
- **Where:** Tasks: Projects, after the issue views.
- **Undo / turn off:** Choose another view; Tasks remembers the last one per window.

### One project page, and Records in Library

- **What:** A project opens on one page: for a tracked project its status, lead, progress, issues, target date and description from Linear, with Open in Linear and Show its issues; for every project the portfolio fields kept in your vault (department, type, hours saved and value a year, owners, GitHub and live site, people, domains, meetings, captures, War Room and docs), which you edit there. A project that isn't in Linear has Track in Linear, which says what it creates first. Library has a Records collection: meetings, domains, War Room and captures from library/records/, and each project's record. A file mirrored from Linear says Tracked in Linear.
- **Where:** Tasks: an issue, then Open project beside its Project. Library: Library folder, Records, then a project.
- **Undo / turn off:** Edit a portfolio field back; nothing is sent to Linear. Track in Linear makes a real project in the team; archive it in Linear to undo.

### Tasks has views and filters like your tracker

- **What:** Tasks has view tabs (My issues, All, Active, Backlog, By project, By assignee), a filter bar (status, priority, assignee, label, project, cycle, due date) and Display's Group (status, assignee, project, priority) and Order (priority, due date, last updated). Each window keeps its own view; a new window starts from the last one you chose. My issues shows what is assigned to whoever your Linear key belongs to, which Hollow reads on the next sync.
- **Where:** Tasks: the tabs above the list, Filter beside them, Group and Order at the end of the toolbar.
- **Undo / turn off:** Remove a filter with its ×, or Clear filters. All with Group: Status is the default.

### Tasks looks like your tracker

- **What:** Tasks lists the team's issues the way Linear does: grouped by status with counts, every row with its priority, key, status, title, labels, project, due date and assignee, and your local tasks in their own group. An issue opens beside the list with every field, sub-issues, relations, links, the activity feed and an Agent activity section; status, priority, assignee, labels, project, milestone, cycle, estimate, due date, title and description edit in place and send to Linear. Settings has a Task tracker block with the key, sync times, requests left, Sync now and the interval.
- **Where:** Tasks (j and k move, Enter opens, Escape closes, c is New issue, / finds). Settings → Connections → Task tracker.
- **Undo / turn off:** Edit a field back, or change it in Linear: Linear wins a conflict and the issue says what was not applied. Choose Off in Task tracker to keep tasks local. The old board, scope tabs, saved filters and bulk edits are gone.

# What's new: 2026-11-09

### Scheduled reports keep their setup and results

- **What:** Eligible read-only automations can use a selected Claude account and up to eight text files, sharing the same usage-budget safeguards as company reports. New setup offers Automatic with an optional preferred account; existing pinned choices stay pinned. Hollow honors saved routing restrictions and records the actual account and reason before launch. Account choices shows that history without checking accounts or starting work. Original approved instructions remain available when you change the report setup. Saved reports reopen by their exact run, including after a restart; they remain model output until separately reviewed. An incomplete setup or uncertain cleanup stays held.
- **Where:** Automations → Automation options → Scheduled report setup. Saved reports appear with the automation's run history. Preview describes the restricted route; Save as skill uses saved instructions.
- **Undo / turn off:** Disable the automation to prevent future scheduled runs, or Stop run to request cancellation of current work. Changing setup creates a new version and does not silently rewrite already queued work. Saved reports and original instructions remain as evidence.

### Read-only company reports keep their evidence

- **What:** President, COO and CFO responsibilities have stable identities without idle sessions. A report keeps your original request, selected source copies, corrections, worker results and independent review. Report accepted means its delegated checks passed; it does not mean a whole project is ready. An accepted report returns to its original Main conversation. Sharing fixed COO updates to a Linear issue requires your consent.
- **Where:** Work → Assignments. Main can propose a report; accepting a proposal still leaves account and source selection to you.
- **Undo / turn off:** Stop prevents new work and shows Stopping until cleanup is confirmed. Correct report saves a new revision; replacement work is an explicit action. Revoke sharing to prevent new shared updates; already delivered comments remain.

### Company reports leave room for conversation

- **What:** Ready messages from you start before company workers. With at least two chat slots, company work leaves room for conversation; at a one-chat limit, work stays sequential. Report workers and reviewers receive bounded shares of your applicable hard usage budgets. New autonomous workflow grants protect room for supported pending user work, including known future work in the relevant period. Running autonomous scheduled reports can yield when your ready request needs their capacity. Hollow confirms cleanup and final usage before releasing unused room. A paused report can make one fresh continuation under current permission after your request finishes; uncertain records keep it held. Use Automation options → Stop pending work to prevent continuation, including when no process is running. Budget records distinguish that current protection from the decision saved for a prior attempt. Budget records show reserved and recorded usage, including historical attempts, with exact estimated costs. Unknown cleanup or incomplete usage records hold the affected budget for recovery. These controls do not measure remaining subscription tokens. Your workflow Run now requests stay distinct from automatically scheduled work. Workflows traced to your request move ahead of agent-started workflows when eligible, including their retries; retry delays and existing limits still apply.
- **Where:** Work → Assignments → Budget records shows local usage evidence. Assignments and Running show waiting work. Settings → Usage → Budgets controls existing usage limits. Ordinary chat continues to warn about those limits. Automations → Automation options → Budget records shows each configured workflow's real attempts and historical allocation basis.
- **Undo / turn off:** Stop a report from its assignment. Adjusting a budget cannot clear an uncertain attempt; this version has no recovery-clear control.

# What's new: 2026-11-08

### Reusable lessons can become skill proposals
- **What:** A general decision can be proposed for your profile. A task lesson can propose an append to the shared skill the chat used, or a new skill. Both wait for Save; a changed skill refuses a stale edit.
- **Where:** Library → Learnings and the learning line below the source reply.
- **Undo / turn off:** Reject leaves the profile and skill untouched. Undo skill lesson puts a saved skill back as it was, while nobody has edited it since. Learning suggestions Off stops capture. Existing skill instructions stay in place when a lesson is saved.

### Grounded Know-how saves with Undo
- **What:** Exact positive Know-how from a chat's source evidence can save itself. You and Project rules still wait for Save. Private provider memories can be read for the selected project and promoted as pending proposals.
- **Where:** Library → Learnings; Undo saved learning also appears on the source reply. Private memories has an explicit Read button and home choice.
- **Undo / turn off:** Undo saved learning reverses one save. Learning suggestions Off stops automatic captures. Provider files stay untouched.

### Memory stays searchable as it ages
- **What:** Saved Know-how shows how often it was recalled and becomes stale after 14 quiet days, then archived after 30. Archived facts stay searchable and restorable. Pin keeps one active. A time-bound fact can have a valid-through date. Your profile rules never age automatically. Saved profile and project lines get the same pin and valid-through date; when one passes its date, or a project line's workspace has no chat for 14 days, a proposal to remove or update it waits in Pending, and the line changes only when you save that proposal. The always-on profile shows its size and proposes consolidation when it grows.
- **Where:** Library → Learnings → Saved, beside each saved learning.
- **Undo / turn off:** Unpin reverses Pin. Restore (Keep this line for a line) restarts the grace period and clears expiry. Clear the expiry date to keep a fact without an end date. Reject a line proposal to keep the line. Saving one, with or without new words, shows Undo for 10 seconds and Undo last batch, which put the line back as it was. No saved fact is deleted without your save.

### Learnings live beside their source reply and in Library
- **What:** A suggestion sits under the reply that taught it, with Save, Reject with a reason and Skip. Library has one Learnings tab for Pending, Saved, Skipped and Rejected. It keeps source quotes, saved homes, bulk review, Undo last batch and replacement history. Pending items stay pending until you decide.
- **Where:** Library → Learnings, or the learning line in a chat. The Learnings tab shows how many wait. The bell and Needs you still count each pending learning, and its row opens that tab. Settings → Memory links there too.
- **Undo / turn off:** Skipped and rejected items offer Review again. A bulk Save or Reject, or Save on one learning (edited or not), shows Undo for 10 seconds, and Undo last batch stays until nothing in that batch is left to undo.

### Hollow looks for lessons while a chat rests
- **What:** Hollow looks for a durable lesson after ten quiet minutes, every few turns in a long chat and around a handover. It works across models. When you say "remember that", "from now on" or "correction:", it looks right after that turn. Each suggestion needs its source quote; task packets and agent claims cannot become your preferences. Keys, tokens and passwords in what it reads are masked first. One unresolved capture per chat waits for review.
- **Where:** The learning line below its source reply and Library → Learnings. The bell and Needs you count each suggestion and open that tab. Settings → Memory links there.
- **Undo / turn off:** Settings → Memory → Learning suggestions → Off. Capture never accepts a suggestion.

### Search what agents said in past chats

- **What:** Chat search now finds replies, errors and short tool summaries as well as your messages and titles. Secrets are scrubbed before indexing. Your current project and work chats rank first. Agents receive matching saved memories before raw chat evidence, with nearby turns, request, outcome and handover links. The iPhone's Search uses the same index and leaves out work chats unless work data is shared with your devices.
- **Where:** Sidebar Search chats, the command palette and the agent's conversation search.
- **Undo / turn off:** No setting. The index is derived from Hollow's own transcripts.

### Tracked projects get a local workspace

- **What:** A new tracker project has one local workspace with generated project context. Existing project paths stay. Managed project identity survives a folder rename, and local rename/archive controls point back to the tracker.
- **Where:** The project's sidebar workspace, with TRACKER.md beside its brief and project note.
- **Undo / turn off:** Leave the tracker off in its configuration. Existing local notes remain.

### A chat shows how long this kind of job usually takes

- **What:** Beside how long a reply took, a chat also says how long a finished chat of that kind usually takes. The kind is the workspace's work kind: research, document, code, data, admin or meeting. A build workspace with none set counts as code, and the Main agent counts as admin. It stays quiet until three finished chats of that kind are on record. This is not the hours-saved estimate in Settings.
- **Where:** On a finished reply, in the "Worked for" line when the steps are folded, or beside the time under the reply when they are not. While a reply is still running, beside the live clock.
- **Undo / turn off:** No setting.

### Tracker edits keep their provider details
- **What:** Existing task and phone edits use the tracker editor for linked issues, keeping provider comments and local checklists separate. The phone receives exact priority and state types while older status words still work. Agents can read tracker detail and request a create with a stable retry key. Archiving always asks, and an unconfirmed create or archive is never blindly resent.
- **Where:** Task edits, paired phone task routes and the agent's tracker actions. No new screen or shortcut in this change.
- **Undo / turn off:** Keep the task tracker off in its configuration. Existing files remain local.

### Votes, Routing and About these settings are gone

- **What:** Useful and Not useful are gone from a reply. Copy and More stay. Settings no longer has a Votes list or a Routing results page. About these settings is gone: each Settings section shows one line for how it saves. Diagnostics, the build, search, databases and Prices sit in System, under a closed Advanced section. Budgets, Usage by day and Value this month stay.
- **Where:** Any reply. Settings → Usage. Settings → System → Advanced. The save line is on every Settings section.
- **Undo / turn off:** No setting.

### History snapshots reread only what changed

- **What:** Once Hollow has taken one snapshot since it opened, History records a later turn by rereading only the files that changed. A file it did not see change keeps the copy it already stored, so restoring that turn still puts those bytes back. If it is not sure what changed, it reads the whole vault, as it did before.
- **Where:** Nothing to click. It happens when an agent turn starts and when it finishes.
- **Undo / turn off:** No setting.

### Map, Runbooks, Tonight, Present and What's new are gone

- **What:** The Workspace map, Suggest what to archive, Runbooks, the Tonight queue, Dry-run tonight, Present, the What's new page and the welcome tour are gone. Work is Home and Running. Automations stays the list of automations. Ask in chat to archive a workspace, to pressure-test a draft, or to take one-off and overnight work. What an install brings is unchanged.
- **Where:** Rail → Work, Rail → Automations. Nothing opens the old screens.
- **Undo / turn off:** No setting.

### The tracker sync foundation
- **What:** The task tracker can mirror issues, project fields and comments into local files while keeping your own notes. Local issue edits wait in an outbox while offline. If both sides change the same field, the tracker keeps its value and the local edit stays in a conflict note and Needs you. A lost create reply is checked before retrying, so it cannot silently create a second issue. Archived and unavailable issues keep their files. Notion two-way sync blocks this engine until it is turned off.
- **Where:** No new screen in this foundation. Task tracker settings and the rebuilt Tasks page follow; the tracker stays off until configured.
- **Undo / turn off:** Leave the task tracker off in its configuration. Existing Notion behavior is unchanged.

### Not verified only when a reply claims it

- **What:** The line under a reply says "Not verified" only when that reply says the work is done, fixed, or that its checks pass, and nothing confirmed it. A change with no check, and no such claim, no longer says "Not verified". A reply that says its checks pass still names that ("Says it passes"), and checks that ran before the last change still say "Checks out of date".
- **Where:** Under an agent's reply, beside copy and the changed-file chips. The Run record still opens from that line, and from More, and it still names the receipt's own state.
- **Undo / turn off:** No setting.

### Today, Weekly review, Agenda, Incoming and Commitments are gone
- **What:** Those screens are gone, and so are the Graph, the old Calendar, Projects and Imports pages, Library's People collection, and Copy to Plan or Tasks. Work is Home, Map and Running. Tasks is a list or a board, including Due today. Needs you is Waiting: All, Decisions, Results and Failures. People notes stay as files. The calendar connection stays under Settings. The commitment ledger stays for agents. Ask in chat when you want a plan row, a task, or a promise recorded.
- **Where:** Rail → Work, Rail → Tasks, the bell → Needs you, Settings → Connections → Agenda calendar. Nothing opens the old screens.
- **Undo / turn off:** No setting.

### The Main agent's notes stay one quiet line
- **What:** When the Main agent edits its own workspace brief or plan, the reply says "Updated my notes" instead of a file chip for each. Other projects' notes and nested sources keep their file chips. In a project conversation those same files still show as chips.
- **Where:** The Main agent's conversation, under the reply. The brief and the plan are where you read them.
- **Undo / turn off:** No setting.

### Bookmarks are gone from a conversation
- **What:** A reply can no longer be bookmarked, and the outline no longer lists bookmarked replies. The outline of your prompts stays, on the list button above the thread.
- **Where:** Any conversation: the list button above the thread, now named Outline.
- **Undo / turn off:** No setting.

### Question cards keep the question in view
- **What:** The words just before a question card stay on screen when the work folds into "Worked for". After you send, the chat shows each question and then your answer. Your own words no longer start with "Other:". Each question can show one line saying why it was asked. The round choice sits inside the card. A selected row and the Recommended tag use the quiet slate accent.
- **Where:** Any chat that asks with a question card, docked over the message box.
- **Undo / turn off:** No setting.

### Friction is no longer a page, and routine exit codes stop counting
- **What:** Settings → Usage no longer has a Friction section or a Friction jump link. It listed 1,194 patterns and nothing was ever done from it; most were a command that exited with a code, which is routine. Hollow still counts where chats keep hitting the same wall, and an agent can ask for the list (`friction.list`) or draft a fix on a card (`friction.draft_fix`, always asks). A bare shell exit code now counts only when a test, typecheck or lint failed, or when it recurs in 3 or more chats on 2 or more days. The list puts new and growing patterns first.
- **Where:** Nothing to click: the Friction section is gone from Settings → Usage. The weekly review reads the list and offers a fix on a card.
- **Undo / turn off:** No setting. The counts and the nightly recount are unchanged apart from the filter.

### An open question card stays open
- **What:** A question card you are still answering stays on screen when a wake, a check-in, a pulse or a scheduled turn finishes. Those turns wait up to 30 minutes while the card is open, then they post underneath it and leave it open. Only your own message, or Send on the card, closes it. Answers you have already chosen stay in the card.
- **Where:** Any chat with an open question card, docked over the message box.
- **Undo / turn off:** No setting.

### Agents do the work those screens used to
- **What:** In a chat that can write, the agent adds a Plan row or a task when the work clearly calls for it, and asks with an approval card when it is a real choice. A commitment stays in the commitment ledger. A clear one becomes a team ENG issue on the tracker (shown as Linear) only with a card, unless the chat is in bypass. An automation records a local task and does not create that issue. People notes stay in Library, and the agent updates them in the background. Asked to pressure-test a draft, the agent reads each role's notes and critiques the draft as that role. The Main agent suggests archiving a workspace in chat, and takes one-off and overnight work. Automations stay. On a check-in it reads tasks due today or overdue. What an install brings is unchanged.
- **Where:** Nothing to click. The rules are in every chat except a pressure-test or bake-off scratch run.
- **Undo / turn off:** No setting.

# What's new: 2026-11-07

### A saved learning goes to the one place it belongs
- **What:** Saving a learning no longer always makes a note under Learnings. A preference or decision about how you work becomes one line in the matching About you note (Communication, Anti-patterns, Guardrails or Decisions); a note about a person, project or goal stays a Learnings note; a decision for one project becomes a line under Decisions in that project's brief (or its AGENTS.md). If the fact is already written in one of those places, saving it again changes nothing. Undo takes that one line back out.
- **Where:** The Learning suggestions cards (Save and Undo are the same buttons); the saved line shows up in the About you note or the project brief it belongs to. Nothing new to click.
- **Undo / turn off:** Undo on the saved learning removes its line; no setting.

### Memory is checked before it's saved, and every change is logged
- **What:** Before a learning is queued, saved, edited in the review card or added to About you, Hollow checks its words for prompt injection (instructions aimed at the next model), a stored secret or a command that sends one out, and hidden characters. A learning that trips the check isn't saved and says which kind and where, never repeating the text. Every capture, save and undo is written to a change log with who did it, which chat, and the text before and after; the log sits outside your vault, and if its latest entry ever stops adding up Hollow stops saving memory until it does.
- **Where:** In the Learning suggestions and About you cards, as the message you see when a save is refused. The log is read by the Learnings tab that comes next; nothing to open yet.
- **Undo / turn off:** No setting. Reword the learning and save it again.

### Work records stay on this Mac
- **What:** Meeting notes, people notes and the other files under Library records, and a Linear project's workspace, are never copied to the phone backend. That stays true when Settings includes work data. A note that says `scope: work` in its front matter still stays off the personal copy, and is included only when work data is on.
- **Where:** Nothing new to click. Settings → Connections → Companion devices → Backend still chooses what the backend copies.
- **Undo / turn off:** No setting. Those files stay on this Mac.

### One Main agent conversation
- **What:** The Main agent is one conversation again, whatever happens underneath. When it moves to a fresh chat (a handover, or a new day), the old chat says "This conversation continued in …" with an Open button instead of a message box, and anything that still reaches it (a race, your phone) goes to the current chat, with a note in both. Wakes and check-ins wait for the current chat instead of opening a second one. Cards it asked in an earlier chat show in the current one.
- **Where:** The Main agent button. Earlier chats are under "Earlier in this conversation" at the top of the Main agent's chat. Any chat that handed over shows the same banner.
- **Undo / turn off:** No setting.

### Project rows: click to open and fold, right-click for the menu
- **What:** A project in the sidebar has no fold arrow and no ⋯ now. Click it to open it and show its conversations; click the open project again to hide or show them (you stay on it). Right-click it (or Control-click, or VoiceOver's Show Menu) for the menu the ⋯ held, Hide or Show conversations and Pin or Unpin included. The + stays.
- **Where:** The sidebar's Projects. The menu's other items are also in the project's ⋯ on Work and in the top bar, and on Home's project rows.
- **Undo / turn off:** No setting.

### Quick notes from your iPhone
- **What:** The iPhone app can send a short note. Hollow saves it as a new note in your Library, titled by its first line or a title you give, and never replaces a note you have. One iPhone can make 200 a day.
- **Where:** The iPhone app's next build; nothing new on the Mac. An older Hollow makes the app say to update.
- **Undo / turn off:** Delete the note like any other, or turn the iPhone app off in Settings → Connections → Companion devices.

### Replies stream without redrawing what's done
- **What:** While a reply is still coming in, only its newest lines are redrawn. Words you select in a finished paragraph stay selected (so you can Quote them before the reply ends), a table keeps its place, focus and its Copied label, and pictures in the reply don't reload.
- **Where:** Any reply as it streams, in every conversation and chat.
- **Undo / turn off:** No setting.

### Retry with another model
- **What:** The latest reply's More actions now offers Retry with each of the chat's other models (up to four, from the same list as the model button), next to Retry with another agent. Under a message that failed, Retry with another model sits beside Retry with another agent. Choosing one switches the chat to that model, as the model button does, and sends your last message again. The earlier reply stays, and the chat keeps the new model for what you say next.
- **Where:** The ⋯ under the latest reply, and the row under a message that failed.
- **Undo / turn off:** No setting. Pick the earlier model in the model button to go back.

### Quote selected text
- **What:** Select words in a reply, or in one of your own messages, and a Quote button appears just above them. Press it and the selection goes into your message box as a quote (each line starting with ">"), after anything already in the box, with the cursor under it so you can write your question and send. Nothing is sent for you.
- **Where:** Any conversation or chat: select words in the thread.
- **Undo / turn off:** No setting. Delete the quoted lines in the box to drop the quote.

### A new chat opens in the middle of the pane
- **What:** A new conversation opens like Claude and ChatGPT: the greeting sits in the middle of the pane with the message box right under it, and the starters (Find the next step, Make a first draft, Review the work) sit under the box. The box keeps the pane's full width, as in a running chat. Send the first message and the box moves to the bottom, as before. In a very short window the box stays at the bottom, so it is always in view.
- **Where:** A new project conversation, or a new chat in Chats. The Main agent's page keeps its own layout.
- **Undo / turn off:** No setting.

### Code blocks wrap, and tables copy
- **What:** Every code block in a reply has a **wrap** button beside copy: long lines wrap instead of scrolling sideways, in every block at once, and Hollow remembers the choice. Tables in replies now have borders and a header row, scroll inside their own box when they are wider than the chat (they used to push the whole chat sideways), and have a **Copy table** button that copies tab-separated text, which pastes into Numbers or Sheets as cells, and the table itself, which pastes as a table into Pages, Docs or mail. The chat's name above the thread now lines up with the thread's left edge.
- **Where:** The header of any code block in a reply (wrap, copy), and the line above any table in a reply (Copy table).
- **Undo / turn off:** Click wrap again to scroll long lines. Nothing to turn off for tables.

### Steadier toolbar and subtask controls
- **What:** Top-bar buttons and subtask checks keep their size when pressed. Fill and focus feedback remain, and a subtask's check mark appears without growing.
- **Where:** The top bar, and a task editor's subtask list.
- **Undo / turn off:** No separate setting. Both motion preferences still apply.

### A quieter empty Tasks list
- **What:** Empty and filtered task lists use the same heading size and spacing as other empty views, centered in their list area.
- **Where:** Tasks, List view, when there are no tasks to show.
- **Undo / turn off:** No setting. The filters, state messages and View all tasks action work as before.

### Steadier chat controls
- **What:** Allow, Deny and conversation-row actions keep their size when pressed. Conversation actions appear at full size for keyboard focus, and disabled controls stay still.
- **Where:** Approval cards in chats and the conversation history list.
- **Undo / turn off:** Nothing to turn off. Actions and permissions work as before.

### Buttons on your iPhone's notifications
- **What:** A Hollow notification on your iPhone has buttons. One for a finished turn has Reply (type a message into that chat, right from the notification) and Open. One for a question card with a single question and two to four options has a button for each option, so you can answer without opening the app. Replying and answering need the iPhone unlocked (Face ID or your passcode first, when it is locked). The option labels travel through Apple's push service like a notification's title and body do. When Hollow can't take it (the chat is busy, the question was already answered, the Mac is asleep) the app says so in a notification, and a reply that didn't go is kept as that chat's draft.
- **Where:** Your iPhone's lock screen and Notification Center: pull down or press and hold a Hollow notification. It needs the iPhone app's next build and Push notifications set up in Settings → Connections → Companion devices → iPhone app.
- **Undo / turn off:** No setting. Notifications stay off in iOS Settings → Notifications → Hollow; Remove key in Hollow stops them all.

### A running turn on your iPhone's lock screen
- **What:** When you send a message from the iPhone app (or open a chat while its turn runs), the turn shows on the iPhone's lock screen and in the Dynamic Island: the chat's name, its workspace, a timer, and what it is doing now ("Editing files", "Running a command", "Waiting for your answer"). It ends with the turn, saying whether it finished, was stopped or hit an error, and a tap opens the chat. The app keeps it current while it is open, and Hollow refreshes it every few minutes so a long step never reads as out of date. With the app closed, Hollow updates it through Apple's push service, using the same key as notifications, and tells Apple only the step's name: never a file, a command or any of the chat's text.
- **Where:** The iPhone's lock screen and Dynamic Island. Settings in the iPhone app has the switch (Settings → Live Activities). With the app closed it needs the iPhone app's next build and Push notifications set up in Settings → Connections → Companion devices → iPhone app; without the key it still updates while the app is open.
- **Undo / turn off:** The Live Activities switch in the iPhone app's Settings, or iOS Settings → Hollow → Live Activities. Hollow keeps the activity's token in memory only, drops it when the turn ends, and forgets it on Unpair or when you revoke the iPhone.

### Photos and files from your iPhone
- **What:** Hollow takes photos, camera pictures, scanned documents and files from the iPhone app, alone or with a message, and shows the files a chat's messages carried (sent from the iPhone or from the chat box here) to the iPhone as thumbnails and file chips. Each file is saved where Hollow's own chat box keeps its attachments (the chat folder's hidden `.hollow-attachments` folder) and is named to the agent the same way, so the agent opens it with its file tools. A message holds up to 10 files and 50 MB, a file up to 25 MB. Hollow reads the file itself to know what it is, so a name or a label from the phone never makes a file an image.
- **Where:** Nothing new to open here: the files arrive in the chat on this Mac as part of the message. The iPhone app must be on and an iPhone paired (Settings → Connections → Companion devices → iPhone app).
- **Undo / turn off:** No setting of its own: turning the iPhone app off, or revoking the iPhone, stops it. A file stays in its chat's folder until you delete it there.

### Settings buttons sit where they belong
- **What:** Remote runner's Set up again and Turn off are together at the right of its text (Turn off had dropped under it, centred). In Connections → Companion devices, the iPhone app's key form and backend forms put their fields side by side with the button at the end of the row (the button had dropped under the fields).
- **Where:** Settings → Agents → Remote runner (with a runner set up); Settings → Connections → Companion devices.
- **Undo / turn off:** No setting. Nothing was added or removed: the same buttons and fields.

### Steadier sidebar and Connections
- **What:** Adding a fifth project no longer folds the other four at once: the projects you see unfolded stay unfolded, and from then on only projects you haven't opened fold by themselves. In Settings → Connections, the Notion setup row and the Plaud recordings row show their status bar beside the name again (it had pushed the text across the row), and the Your connectors introduction shows its first sentence with More for the rest.
- **Where:** The sidebar's Projects list, and Settings → Connections.
- **Undo / turn off:** No setting. Fold or unfold any project from its row as before; More opens the rest of the introduction.

### A name above every chat
- **What:** A project conversation, or a chat in Chats, shows its name above the thread, as in Claude and ChatGPT. A ⋯ beside it opens the same menu its sidebar row has (Rename…, Pin, Move to project…, Archive and the rest), and a double-click on the name renames it in place: Enter or clicking away saves, Esc cancels. Pressing ↑ in an empty message box now brings back this conversation's own messages first, newest first, and then your earlier messages from other chats.
- **Where:** The row above the thread, at its left (the Find, Outline and Copy buttons stay at its right). Not on the Main agent page, which has its own header, nor in Library → Conversations, which already names the chat.
- **Undo / turn off:** No setting. Renaming from the sidebar row still works as before.

### Tasks on your iPhone
- **What:** Hollow now serves your tasks to the iPhone app: the open ones grouped Overdue, Today, This week, Later and No date, finishing one (a repeating task moves to its next date instead), adding one, and changing its title, date, time or priority. Every change goes through the same writer Hollow's own task tools use, so a task that changed on your Mac after you opened it is never overwritten, one iPhone can make at most 200 tasks a day, and nothing here can delete a task. The Tasks tab that uses this arrives with the iPhone app's next build; until then nothing changes on the phone.
- **Where:** Nothing new on the Mac. The iPhone app's Tasks tab, once installed; pairing stays in Settings → Connections → Companion devices → iPhone app. With a Hollow that has no task routes the tab says to update Hollow.
- **Undo / turn off:** Finishing a task by mistake has an Undo in the app. Turn the iPhone app off in Settings → Connections → Companion devices → iPhone app, or revoke the iPhone.

### Save a whole conversation as a note
- **What:** A conversation can be saved as a note: the whole thread as Markdown, each turn under a heading for who wrote it, the same text Copy conversation puts on the clipboard. It goes where a reply's Save as note goes: a project conversation's Results, the Library from Chats or the Main agent, and never over an existing note. A toast says where it went, and Open shows it in the Preview pane.
- **Where:** The chat title menu, Actions for this conversation → Save conversation as a note. Find, Outline and Copy conversation live in that same menu.
- **Undo / turn off:** No setting. The note is an ordinary file in your vault.

### Read a reply aloud
- **What:** A reply can be read aloud in your Mac's voice. It reads the words and leaves out the symbols: headings and list items pause like sentences, a link reads as its text, a web address as "link", and a code block is skipped, with a spoken "Code block skipped". One reply reads at a time, and the voice stops when you open another conversation or another page. While it reads, the reply's row and the top row of the chat both show a Stop reading button.
- **Where:** A reply's More actions (the ⋯ under the reply) → Read aloud; Stop reading is in the same menu, on the reply's row and at the top of the chat.
- **Undo / turn off:** No setting. Nothing reads until you ask.

### Canvases on your iPhone
- **What:** Hollow can serve your chats' canvases to the Hollow iPhone app, which draws them itself with Hollow's own canvas kit, in light and dark, instead of a row saying to open them on the Mac. A canvas runs its scripts with no internet, as on the Mac. An ask-canvas's Send on the phone sends your answers the way its Send on the Mac does, and the canvas then shows as answered in both places; only its newest version can send. The app also lists a workspace's saved canvases.
- **Where:** In the iPhone app, while Settings → Connections → Companion devices → iPhone app is on.
- **Undo / turn off:** No setting of its own: turning the iPhone app off, or revoking the iPhone, stops it with everything else.

# What's new: 2026-11-06

### Choose your chat's send settings on iPhone
- **What:** Pick an agent, model, effort and permissions before sending. Agent and model carry forward; permissions carry forward unless the Mac has a saved choice for that chat. Effort applies to one send. Bypass all explains and confirms unrestricted access. App-wide defaults stay unchanged.
- **Where:** On iPhone, a chat's Send settings button; in New chat, Model, effort and permissions. Pairing stays in Settings → Connections → Companion devices → iPhone app.
- **Undo / turn off:** Clear pending choices before sending. After acceptance, choose the settings you want for the next send.

### Quieter Results and empty views
- **What:** Library document covers have one frame. Empty and loading views use quieter surfaces, with wrapping action buttons. Shared buttons stay still when pressed and use fill feedback.
- **Where:** Library → Results, empty and loading views, and shared buttons throughout Hollow.
- **Undo / turn off:** No separate setting. Text size and motion preferences still apply.

### Quieter Settings
- **What:** Long section and sub-section introductions in Settings, and the help under the runner rows, show their first sentence, with a visible More for the rest (Less closes it). The themes in Interface sit in a tile grid, three across. In Agents, the rows that route work to a remote runner (Runs on, Builds and tests, App builds, Overnight and worker chats, Runner GitHub access) appear once a runner is set up, instead of explaining a setup you haven't done.
- **Where:** Settings: any section's or sub-section's introduction, the runner rows' help, Interface → Appearance, and Agents → Remote runner → Set up.
- **Undo / turn off:** No setting. The rest of every text is on the page behind More. Settings search still finds Runs on, Builds and tests, App builds and Overnight and worker chats, and takes you to Remote runner until a runner exists.

### Consistent page headings
- **What:** Page headings share one compact title size. Work, Automations, Needs you and Settings now use the compact header spacing already used by Tasks and Library, leaving more room for their content.
- **Where:** Page headings throughout Hollow. Text size still applies in Settings → Interface.
- **Undo / turn off:** No separate setting.

### Steadier filters and actions
- **What:** Filter buttons, toast actions and modal close buttons keep their size while pressed. Selected filters stay marked and no longer bounce when chosen.
- **Where:** List filters, toast action buttons and the close buttons in task and Add dialogs.
- **Undo / turn off:** No separate setting. Motion preferences still apply.

### A clearer Main agent header
- **What:** The Main agent's page header is one status line and at most two links, and each number says what it counts. "Waiting on you in 1 conversation" counts its own conversations, as the orb's badge does (the bell counts everything waiting across Hollow). "6 check-ins today · next when something new comes up" replaces "No upcoming check-in · 6 today": a check-in runs only when something new came up, or as the day's first. Show its day is now Show details. The second row is gone: running chats it started are in the status line, everything waiting is the bell, and the actions it took on its own are a link in Show details → Today. In the top bar, a workspace's origin ("Hollow workspace", "Notion project") no longer shows cut off beside its name (hover the name for it), and on the Main agent page the bar no longer repeats "Waiting on you".
- **Where:** The Main agent page (the orb at the top right) and the top bar. Change icon… moved to the top bar's ⋯ menu (Actions for Main agent) on that page.
- **Undo / turn off:** No setting. Change icon… is also in Settings → Interface → Appearance → App and Main agent icons.

### One header row in the Preview pane
- **What:** A file, web page or canvas in the Preview pane starts right under one row of controls: back and forward, the tabs, New web tab, the file's own buttons (Conversation files, Preview or Source, Refresh, Ask the agent to fix, the ••• menu) and close. The tab names the file, so its name no longer repeats above the content, and the "File", "Web page" or "Canvas" label over the tabs is gone. Hover a tab to see what it holds: the kind, and for the file on show its size and time, then the full path. The file's ••• menu lists the same details under Copy path. The tab on show stays wide enough to read its name, and New web tab sits right after the last tab; in a narrow pane the file's buttons and close move to a second row. A canvas tab is named by its canvas, and a web page keeps its address bar under the tabs.
- **Where:** The Preview pane beside any page: click a file or a link in a chat.
- **Undo / turn off:** No setting.

### Tidier project rows in the sidebar
- **What:** A project in the sidebar is one row again: pointing at it lights the whole row, its controls included, and only two controls trail it, its ⋯ menu and + for a new conversation. The arrow that hides or shows a project's conversations now sits on the project's icon: point at the row, or Tab to it, and the icon turns into the arrow. A pinned project keeps its pin as a quiet mark; Unpin is in its ⋯, beside Pin.
- **Where:** The sidebar's Projects: point at a project, or Tab to it. The open project shows its ⋯ and + all the time.
- **Undo / turn off:** No setting. Hide conversations and Show conversations are also in a project's ⋯ and its right-click menu.

### Up to 25 chats at once
- **What:** Settings now lets you run up to 25 chats at once across Hollow (it stopped at 10) and up to 10 at once in one workspace (it stopped at 5). Nothing changes until you change it: the numbers stay 10 and 5, more chats wait in line, and none starts while your Mac is short of memory.
- **Where:** Settings → Automation → Chats running at once and Chats at once in one workspace.
- **Undo / turn off:** Set the numbers back to 10 and 5, or lower. A lower number holds new chats and never stops one that is running.

### Browse your workspaces on iPhone
- **What:** The Hollow iPhone app's next build has a Workspaces tab: your pinned workspaces first, then the rest, each with its status, its chats and its next step. A workspace opens with its outcome, plan, documents, chats and Results, and a file opens right on the phone: Markdown rendered, CSV as a table, PDFs, images and text, with Share. It only reads: nothing on the phone changes a workspace or accepts a Result. A file over 10 MB that isn't text stays on the Mac.
- **Where:** On iPhone, the Workspaces tab. Pairing stays in Settings → Connections → Companion devices → iPhone app. With a Hollow that has no workspace routes the tab says to update Hollow.
- **Undo / turn off:** Turn the iPhone app off in Settings → Connections → Companion devices → iPhone app, or revoke the iPhone.

### Search your chats and workspaces on iPhone
- **What:** The Hollow iPhone app's next build has a Search tab. Type a few words and it lists the workspaces whose title, status, outcome or next step hold them, and the chats whose title or one of your own messages does, with the part of your message that matched. It searches what you wrote, not the answers, in the same way the search on the Mac does, and only reads.
- **Where:** On iPhone, the Search tab. Pairing stays in Settings → Connections → Companion devices → iPhone app. With a Hollow that has no search route the tab says to update Hollow.
- **Undo / turn off:** Turn the iPhone app off in Settings → Connections → Companion devices → iPhone app, or revoke the iPhone.

# What's new: 2026-11-05

### Keep Hollow running with the lid closed
- **What:** Close your MacBook's lid and Hollow keeps running, so your iPhone keeps reaching your chats. It works while the iPhone app is on and an iPhone is paired, and by default only on power: unplugged, closing the lid sleeps the Mac as usual, and a Mac already closed goes to sleep within 10 seconds of losing power (a check of its own sees to that, and it waits on nothing else the helper does). Tick On battery too to keep it awake on battery as well; a closed Mac that can't sleep can get hot in a bag and runs its battery down. The section says in words whether you can close the lid now. macOS asks for your password when you turn it on, off, or change On battery too: Hollow installs a small helper (two background items) that turns system sleep off while Hollow asks, which it does only while it runs with a phone paired, and turns it back on otherwise. If the helper ever stops, the section says so instead of promising sleep, and a helper from an older Hollow gets Update the helper (one password prompt).
- **Where:** Settings → Connections → Companion devices → iPhone app → Keep Hollow running with the lid closed.
- **Undo / turn off:** Off until you turn it on. Turning it off (with your password) removes the helper and turns system sleep back on. Turning the iPhone app off or revoking the last iPhone lets the Mac sleep again without a prompt, and so does quitting Hollow. If system sleep was turned off some other way, Hollow says so and leaves it alone; if Hollow's helper is gone and left sleep off, Turn sleep back on fixes it. Without Hollow, an administrator removes it in Terminal: `sudo launchctl bootout system/com.hollow.app.lid-closed`, `sudo launchctl bootout system/com.hollow.app.lid-closed-restore` and `sudo rm /Library/LaunchDaemons/com.hollow.app.lid-closed.plist /Library/LaunchDaemons/com.hollow.app.lid-closed-restore.plist`, then `sudo pmset -a disablesleep 0` if `/Library/Application Support/Hollow/lid-closed-sleep-disabled` exists, and last `sudo rm -rf "/Library/Application Support/Hollow"`.

### Notifications and question cards on your iPhone
- **What:** The Hollow iPhone app's next build can tell you when a turn you sent finishes or a question card waits for you, answer a question card on the phone (tap an option, pick several, or type), list every chat waiting for an answer at the top, and start a chat on the Mac mini runner. Notifications go straight from Hollow on your Mac through Apple's push service, signed with a key from your Apple developer account that Hollow keeps in the Keychain. A finished turn is told only for a message you sent, from the Mac or the phone: never for overnight work, the Main agent's check-ins and reports, or a chat an agent started. A question card is told whatever asked it, overnight work included, because it waits for you; your iPhone's Focus settings keep quiet hours quiet. Nothing goes for a chat you have open on the phone. An answer from the phone does what the card's Send does on the Mac. Choosing the Mac mini for a new chat sets that workspace's Runs on, as choosing it in a new chat on the Mac does.
- **Where:** Settings → Connections → Companion devices → iPhone app → Push notifications: Import key, Send test notification and Remove key. The rest is in the iPhone app.
- **Undo / turn off:** Remove key stops every notification; an iPhone turns its own off in iOS Settings. Runs on goes back in the workspace's new chat or its Changes tab.

# What's new: 2026-11-04

### Your chats on your iPhone
- **What:** Hollow can serve your chats to the Hollow iPhone app over your tailnet. The iPhone lists your chats, reads one, follows a running answer as it is written, sends a message, starts a chat in a workspace and stops a turn. You pair it once with an 8-character code or a QR code; Hollow keeps only a fingerprint of its key, never the key, and only your own Tailscale account can use it. A message from the iPhone runs with the permissions that chat shows in Hollow on your Mac, or in Plan, which only reads, when Hollow couldn't save your permission choices. Until Hollow's window has loaded after it starts, a message runs with the permissions Hollow last saved for that chat. While the chat is still answering, the iPhone keeps your message and asks you to send it when the turn ends.
- **Where:** Settings → Connections → Companion devices → iPhone app: the switch, Publish on Tailscale and Pair an iPhone. A paired iPhone shows in the device list above it, with Revoke.
- **Undo / turn off:** Off until you turn it on. Turning the switch off stops every paired iPhone at once, Stop publishing takes it off your tailnet, and Revoke signs one iPhone out. If Tailscale forwards Hollow's port as raw TCP, which doesn't say who is calling, Settings shows it as a problem: Stop publishing removes one Tailscale saved, and for one a tailscale serve still running in a terminal or a Tailscale service holds, Settings says how to stop it. Stop publishing never touches what another app serves on port 8443.

### Files a chat names open in the side pane
- **What:** Click a file an agent made or named and it opens in the Preview pane beside your page. Hollow now looks for it in every folder the chat works in (its own, the project's folder and worktree, the repositories its turns changed, the vault), so a path written relative to another folder, or one from a turn that ran on the Mac mini, opens instead of saying the file may have moved. A path written in the reply's words, not only in backticks, is clickable too. A reply's changed-file chip and its run details open the file there as well; a Result's Accept and Ask for a revision are on the chip's right-click Open in Results and the reply's ⋯. When no folder holds the file, a notice says why: deleted or moved, cleared with the turn's scratch folder, or left on the Mac mini.
- **Where:** Any chat: file names in replies, tool rows, the changed-file chips under a reply and its run details.
- **Undo / turn off:** No setting. Close the pane with its ×.

### Drop files anywhere in Hollow
- **What:** Drag a file from Finder onto any part of Hollow's window, not only the message box, and it attaches to your chat, ready to send to the agent with your message. If the chat is behind another page (Settings, Library), Hollow brings it back first; on Home, a new chat opens holding the file. A dashed outline says where the drop goes while you drag. A folder is still refused in words, and the message box's own limits hold (20 MB a file, 10 files and 50 MB a message).
- **Where:** The whole window while you drag a file over it.
- **Undo / turn off:** No setting. Remove an attached file with its ×; nothing is sent until you send.

### Question cards always send
- **What:** A question card you have filled in always sends. When the chat has moved on to a new session since the agent asked (a Main check-in's card answered after your next message, say), your answers go as an ordinary message with each question in full, and the card says so on its first question. Whatever does hold Send (a turn still running, the chat still loading, a changed file) now shows on the first question, before you pick any answers. Answers name each question in full, so they make sense in any session. And your turn after a check-in that ran in the open chat carries on its session instead of starting a fresh one.
- **Where:** The question cards docked over any chat's message box, such as a Main check-in's.
- **Undo / turn off:** No setting.

### Quick chats is now called Chats
- **What:** The built-in space for chats with no project is called Chats everywhere, as the sidebar section already was: its header, the crumb, its menus, the palette, notices and Settings. Its New quick chat is New chat (as the sidebar's button), Ask's Quick chat choice is New chat (New chat in a project with Add to project), and a chat there has its Context button say No project. An existing vault, whose Chats brief still says Quick chats, shows Chats too; nothing in it is rewritten.
- **Where:** The sidebar's Chats and its New chat, Work's Ask anything, the palette's New conversation…, the menu bar's File → New Chat.
- **Undo / turn off:** No setting. Ids, routes, shortcuts and the folder (`workspaces/quick-chats`) are unchanged.

### Cancel keeps your workspace result selection
- **What:** Cancelling a draft prompt leaves the workspace's selected result and section unchanged. A newer result link also replaces an unanswered older request.
- **Where:** Every link that opens a workspace on one of its results: Open in workspace Results in Library, a workspace's Overview and Activity, Needs you, and the file links under a chat reply or in a run record.
- **Undo / turn off:** No setting. Open the result again when ready to leave the editor.

### Coming back to a chat lands at its end again
- **What:** Going to another page (Tasks, Library, Settings) and back to a workspace chat that you had left at its end brings you back to its end, with its canvases drawn. It used to come back a canvas or so above the end, because the canvases above redrew after you returned. A chat you had left further up keeps your place.
- **Where:** Any workspace chat, coming back from its sidebar row or Work.
- **Undo / turn off:** No setting. Scroll, or use Jump to latest, as before.

### Escape closes a menu first in a narrow window
- **What:** In a narrow window, Escape on an open menu (a chat's or project's ⋯, or any right-click menu) now closes just the menu; the sidebar stays open until the next Escape.
- **Where:** The sidebar in a window narrower than 860 px.
- **Undo / turn off:** No setting.

### Long messages of yours fold behind Show more
- **What:** A long message of yours in a chat (more than 12 lines, or more than 1,500 characters) now shows its first nine lines, faded at the bottom, with Show more under them, as in ChatGPT and Claude. Show more shows the whole message and becomes Show less. A pasted 200-line prompt no longer fills the conversation. Only the words fold: images and a queued message's buttons stay as they were. Copy still copies every word, and while Find in the conversation (⌘F) searches, every long message shows whole, so no match is hidden under a fold; closing Find leaves the message it was on open. Opening or closing one keeps the words you are reading in place, and opening one while a reply streams below stops the view following the reply once the opened words push its end out of view (Jump to latest brings you back).
- **Where:** Any chat: Show more under a long message of yours.
- **Undo / turn off:** No setting. Show less folds it again. An opened message isn't remembered: reopening the conversation folds it again.

### Notification history lives in the bell
- **What:** One history keeps saved filters, repeats, moved-target hints and App messages. Details shows complete text and timestamps. Running and Map leave keys to the bell. Unread failures keep individual actions; bulk actions and Open, including toast Open, leave them unread.
- **Where:** Bell → Notifications. Needs you → See all still holds decisions and Incoming. Existing list keys work in the history.
- **Undo / turn off:** Close the bell or choose All to clear a filter. Clearing listed history cannot be undone; Settings Clear all still deletes every notification, including failures.

### Workspace documents beside your chat
- **What:** Workspace sections open beside the conversation. On a narrow desk the panel covers it; Close returns to the same chat. The conversation stays mounted while you visit other pages; Work Home closes the workspace.
- **Where:** Open a project, then use Workspace panel at the top right for Overview, Brief, Plan, Results, Sources and History. In Preview, Canvas actions → Versions and pin opens a canvas's revision controls.
- **Undo / turn off:** Close the panel with its Close button or the same top-bar toggle. Existing section ids, saved drafts and shortcuts stay unchanged.

### Cancel keeps Library where you left it
- **What:** Cancelling an unsaved-edit prompt leaves Library's view and canvas filters alone. A cancelled Compare request cannot open on a later visit.
- **Where:** Canvases and Compare versions links into Library → Results.
- **Undo / turn off:** No setting. Follow the link again when ready to leave the editor.

### Attach any file to a message
- **What:** Any file now attaches to a chat message, not only an image: choose + → Attach files… (several at once), drop files on the message box, or paste a file you copied. A PNG, JPEG, GIF or WebP image still shows as a thumbnail. Any other file, other image types included, shows as a chip with its name and size, Saving… while Hollow keeps a copy in the chat's folder (a hidden `.hollow-attachments` folder that git ignores, up to 20 MB a file), and × to remove it. When you send, the agent gets each file's original name and the path of its copy, so it can open it, and your message shows the file as a chip that opens in the Preview pane. Pasted text still pastes as text. A message holds up to 10 attachments and 50 MB in all. A folder, an empty file or a file over 20 MB is refused with a note saying why. In a workspace chat the + menu also has Add to Sources…, which adds a file to the workspace's Sources instead of one message, and a dropped folder's note offers it too.
- **Where:** The message box in any chat: + → Attach files…, drag and drop, or paste. Add to Sources… is in a workspace chat's + menu.
- **Undo / turn off:** No setting. Remove a file with its × before you send.

### Older chats load in the sidebar
- **What:** Under a project, or in Chats, Show older now lists that project's older chats right there, 25 at a time, instead of opening Conversation history. To see the project's chats in Conversation history, with its search and archived chats, choose Open in Conversation history from the project's ⋯ (or the Chats heading's ⋯).
- **Where:** The sidebar: Show more, then Show older, under a project or in Chats.
- **Undo / turn off:** Show less folds them away again.

### A quieter Map and empty Running view
- **What:** Map keeps its view controls together and leaves more room for workspace labels. Running uses one empty explanation until there are turns to show; actual turns keep their cost details.
- **Where:** Work → Home → Projects → Map, or Work → Running.
- **Undo / turn off:** Show Legend to keep it open on later visits. Existing saved legend and Running view choices stay.

### Your work is now Work
- **What:** The sidebar, native menu, canvas pins and help now call the page Work. Its Home, Today and Running views keep the same saved state and shortcuts.
- **Where:** Sidebar → Work. Map remains under Home → Projects → Map.
- **Undo / turn off:** No setting. This changes the name; existing routes and saved preferences stay compatible.

### Quieter Tasks and Automations
- **What:** Empty task suggestions no longer take a row, and task lists say whether they are empty, filtered or still uncertain. Automations puts its background-work help on one row and shows one empty-list explanation at every window width. An empty refresh keeps an open automation visible.
- **Where:** Tasks and Automations. New task and New automation remain the primary actions; sync, filters, layouts and background-work controls stay in place.
- **Undo / turn off:** No setting. Close a help disclosure or use View all tasks to clear task filters.

### Results lead the Library
- **What:** Library opens with a grid of files and canvases made in your projects, with larger previews. A selected canvas keeps its detail beside a compact list.
- **Where:** Library → Results. Back to all results returns from a canvas to the grid. Notes, Recordings, Connected projects and Conversations remain one click away. New note stays in the header.
- **Undo / turn off:** Choose Notes or Recordings to keep it as your preferred view. Existing saved choices and Back/Forward still work.

### A quieter top bar
- **What:** Search and New leave the top bar. The sidebar starts chats and projects, Library actions adds notes, links, files and recordings, and Work's Running view holds background operation details beside the run controls.
- **Where:** Sidebar → Search chats → Search everything, New chat or Projects +; Library → Library actions; Work → Running. Click the project's or chat's sidebar row to return to its conversation.
- **Undo / turn off:** No setting. Existing shortcuts and saved data stay unchanged.

### Needs you is in the bell
- **What:** Questions, approvals, Results and learnings share the bell's Needs you tab. Open an item or choose See all for the full list, unsaved drafts and Incoming. The old Review sidebar row is gone; accepting and approving still require your click.
- **Where:** Top bar → Notifications → Needs you. See all opens the full view; ⌘5 still works.
- **Undo / turn off:** Close the bell with Escape or click outside. Opening it never accepts or dismisses an item.

### Your messages: a cleaner bubble, Edit and Retry
- **What:** Your messages in a chat are now a quiet bubble fitted to their words, without the "Prompt" label and the full-width band. Hover over one (or Tab to it) and a row under it shows when you sent it ("10:21 PM", or "Nov 3, 10:21 PM" on an older day, with the full date on hover), Copy, which copies the words you see, and on your last message Edit, which puts it back in the message box, before anything you had already typed, for you to change and send. Hollow can't rewrite a conversation, so the edited message goes as a new message and the earlier answer stays. Edit shows only when nothing is running and nothing is queued. Under the latest reply, once it has finished, a Retry button sends your last message again as a new message, as Retry already did after a failed reply, and the reply's More menu has Retry with each other agent. Retry and Edit send only words, so they aren't offered for a message Hollow wrote for you, such as Continue, a question card's answers or the message that continues on another agent after one ran out of usage, nor for one that went with images, files or a shared page while the conversation stays open; after a failed reply to one of those, the failure row still shows, without Retry. A conversation you reopen keeps only each message's words, so there Retry and Edit are offered for a message that had images, files or a shared page and send its words alone. In a conversation you reopen, each message shows the time its turn started.
- **Where:** Any chat: hover over your message for the time, Copy and Edit; the latest reply's footer for Retry, and its More menu for Retry with another agent.
- **Undo / turn off:** No setting.

### Calmer Settings
- **What:** Seven sections and search remain, with one reading column. Scope details and the icon gallery open when needed, leaving room for everyday controls.
- **Where:** Settings. About these settings explains scope and saving; Interface → App and Main agent icons holds both icon editors. Search opens the matching icon row.
- **Undo / turn off:** Close either disclosure. Your choices, custom icons, themes and saved settings stay unchanged.

### The mic sits beside Send
- **What:** The message box's bottom row is quieter, as in ChatGPT and Claude: +, the model button and the Context button on the left; the usage ring, the mic and Send (or Stop) on the right. The last turn's cost is no longer written on the row: hover the usage ring to read it, or press the ring for the Usage card, which lists it under Last turn. The box now grows to 40% of the chat's height for a long message (it stopped at about eight lines). In a short window the empty box starts one line tall and Library's conversation header fits on one row, so the conversation keeps room to read, and the model menu opens inside the pane it sits in instead of under Library's heading.
- **Where:** Any chat's message box; Library → Conversations for the short-window changes.
- **Undo / turn off:** No setting.

### The message box says where it goes
- **What:** The button under the message box no longer reads "This workspace" in every project chat. The box itself names the project while it is empty ("Message Claude in Q3 regional operations review…"; in the Main agent, "Message the Main agent…"). The Context button beside the model button now says something only when it is news: Quick chat, "Moved to" a project until a moved conversation's next turn runs, or how many sources go with every message. Otherwise it is a small folder icon. Pressing it still opens what goes with your next message, connectors included.
- **Where:** The message box of a project chat, a quick chat or the Main agent, and the folder button on its bottom row. The vault chat in Library keeps its folder label.
- **Undo / turn off:** No setting.

### The sidebar works like ChatGPT's and Claude's
- **What:** The sidebar now starts with New chat and Search chats, then your pages, with Tasks back among them, then Projects and Chats, with Settings at the bottom. New chat starts a chat with no project (a project's own + starts one in it). Search chats finds your chats and projects right in the sidebar; its last row searches everything else. Pinned projects lead Projects, each with its pin, then the projects you used most recently; Show more projects lists the rest in place, and All projects opens the full list. Chats lists your chats with no project under Today, Yesterday, Previous 7 days and Older, instead of week folders. Click Projects or Chats to fold that section, and drag the sidebar's right edge to make it wider or narrower. Folds and the width are remembered.
- **Where:** The sidebar (⌘B shows and hides it).
- **Undo / turn off:** Double-click the sidebar's right edge to go back to the standard width; click a folded section's heading to open it again.

### Every chat in the sidebar has a ⋯ menu
- **What:** Hover a chat in the sidebar and click its ⋯ (the open chat always shows it; right-click opens the same menu). Rename lets you type the new name right in the row: Enter saves, Escape cancels. Pin keeps the chat at the top: at the top of its project, or under Pinned in Chats, however old it is. Move to project moves it into another project. The rest are still there: New project from this chat, Open, New conversation in the project, Copy conversation ID and Archive, with Undo. After Pin, Unpin or Archive from the keyboard, the focus stays in the sidebar: on the chat, or for an archived chat on the one below it (above, for the last; the heading, for an only chat). A project's ⋯ now starts with Rename, Pin and Archive.
- **Where:** The sidebar: hover a chat or a project, then ⋯.
- **Undo / turn off:** Unpin from the same menu; Archive's Undo, or Library → Conversations → Restore.

### Search your chats from the sidebar
- **What:** Search chats at the top of the sidebar opens a search field right there. It finds projects by name and chats by their title or by what you asked in them, newest first, each with its project. Use the arrow keys and Enter, or click; Escape clears the words, then closes the search. The last row, Search everything, opens the search box for everything else with the same words (⌘K still opens it too).
- **Where:** The sidebar → Search chats.
- **Undo / turn off:** Escape or × closes it and the sidebar is as you left it.

### A long Projects list stays short
- **What:** With more than four projects in the sidebar, each project's chats start folded, except the project you have open and any project holding a chat you pinned, so Chats stays in view and your pins show. A project with a pinned chat also stays listed past the newest eight. Unpin a chat from its ⋯ and its project stays open and listed where you are. A folded project still shows when one of its chats is working or needs you. Point at a project to see its arrow and +; click the arrow to keep it open or folded, and Hollow remembers that choice.
- **Where:** The sidebar → Projects.
- **Undo / turn off:** Click a project's arrow to unfold it for good.

### A quieter Work page
- **What:** Home, Today and Running are the three main tabs. Home's shorter Ask leaves more room for recent projects. Today and Week keep their draft previews with quieter buttons.
- **Where:** Work. Map is beside Home's project heading; its existing shortcut and palette action still work.
- **Undo / turn off:** Click Home to return from Map. Your projects, drafts and view settings stay as they were.

### Notifications have a bell
- **What:** The top-bar bell shows newest notifications with Open, Mark read and Clear listed. Routine completions stay there; failures and time-critical notices can still show a short toast. App messages and the full Needs you view are one click away. Clearing listed history preserves unread failures.
- **Where:** Top bar → Notifications. Settings → Interface controls in-app pop-ups.
- **Undo / turn off:** Close with Escape or click outside. Cleared history cannot be restored; marking read leaves the item in history.

### Find marks the words it matched
- **What:** Find in a conversation (⌘F) now marks every match in the text, and the matches in the reply it is on more strongly, instead of only outlining that reply. Labels, tool steps and buttons are never marked. Enter and Shift+Enter step between replies, and while a reply streams in the marks follow it and find no longer jumps back to the first match on every word.
- **Where:** Any conversation with more than two turns: ⌘F, or the magnifier over the transcript.
- **Undo / turn off:** Close the find bar (Esc) and the marks go.

# What's new: 2026-11-03

### A self-hosted model can join a bake-off
- **What:** A model bake-off can now include a self-hosted model you have switched on, such as Qwen3.8 27B, so after a few graded bake-offs the routing suggestions can say which kinds of work it handles well. Ask an agent for a bake-off and name the model; the agent sees which ones are on. It shows as "Qwen3.8 27B · self-hosted" in the bake-off and its suggestions. It is billed by GPU time, so it doesn't count toward the bake-off's cap or the $50 monthly cap and doesn't shrink what the other pairs may spend; once it has run and you have graded, its cost reads "billed by GPU time" (never $0), and a suggestion never calls it cheaper. A bake-off of self-hosted models only has no dollar cap to stop it, so at most 10 of those can start in a month. Its turn starts the model when it comes, which can take 4 to 6 minutes if the model is asleep; nothing wakes it earlier. If the model is switched off or removed, the bake-off doesn't start. If that happens after it started, or the model is pointed at another address or served model, that answer is skipped with the reason instead of running on another model. If it is switched off, removed, changed to another API style or no longer valid after its turn was queued, the turn stops before it starts and the answer is skipped with that reason; pointed at another address, sign-in or served model in that short wait, it runs as edited. A bake-off started before this update that names a self-hosted model skips that answer and says to start a new bake-off. Renaming the model only renames it in the bake-off. A self-hosted chat's turn now also waits at least 16 minutes before the no-output watchdog or a run deadline stops it, so a cold start isn't cut short.
- **Where:** Ask for a bake-off in any chat; grade and accept in Settings → Usage → Routing, which also names the self-hosted models a bake-off can include.
- **Undo / turn off:** Leave the model out of the bake-off, or switch it off in Settings → Agents → Self-hosted models.

### Self-hosted models on Codex
- **What:** A self-hosted model can now run on Codex as well as on Claude. In Settings, each self-hosted model has an API style: Anthropic's Messages API runs it on Claude, as before, and OpenAI's Responses API runs it on Codex, where it shows in the Codex part of a chat's model menu as "<name> · self-hosted". Choose it and that chat's turns run Codex on your own server through Hollow's local forwarder, with Hollow's tools, streaming answers and Stop. Codex runs there with a Codex home of its own that Hollow keeps for that model, never your own Codex settings, so no ChatGPT sign-in, OpenAI key or ChatGPT connector goes near your server, Codex's own usage reporting to OpenAI is off, and its sessions stay in that home so the chat continues where it left off. If that home ever holds a sign-in or settings Hollow didn't put there, or this Mac has organisation-managed Codex settings, the turn stops and names the file instead. As with a self-hosted model on Claude, the chat runs on this Mac whatever Runs on says, and its usage reads billed by GPU time. Effort goes to the nearest level the model takes (for Qwen, High runs as Medium). A model set to one API style isn't offered under the other agent, and a chat that asks for it there stops with a message saying where to choose it. An OpenAI-style model can't use one of OpenAI's own model names (serve it under another name). Removing the model keeps its Codex home, with its Codex conversations, in Hollow's app data; the confirmation names the folder. Background jobs still run on Claude, so they offer only Anthropic-style models, and a job kind sent to a model that later became OpenAI-style is held instead of running elsewhere. Test checks an OpenAI-style model with one Responses call. Codex shows a note on each turn that it has no details for the model's id; that is expected.
- **Where:** Settings → Agents → Self-hosted models → Add a self-hosted model or Edit → API style. Then a chat's model button → Codex → the model's "· self-hosted" row (the palette's Switch model… opens the same menu).
- **Undo / turn off:** Set the API style back to Anthropic's Messages API, choose another model in the menu, or switch the model off in Settings.

### Background jobs on a self-hosted model
- **What:** Hollow's own model calls can run on a self-hosted model, chosen per kind: Recording extraction, Chat and session titles, Learning extraction, Meeting recaps and prep packs, the About-you scan and the Heartbeat. Each stays As now until you choose a model, so the kinds that read private data (HR, payroll, claims, recordings with personal data) can go to Qwen while the rest stay where they are. A kind sent to a model runs only there, through Hollow's local forwarder with a key made for that one job. While the model is asleep its jobs wait, and Hollow wakes it once for all of them when the oldest has waited 30 minutes, when you press Run them now, or right after the model answers a chat; waiting jobs survive a quit. While the model is switched off or removed they are skipped with a note, never sent to Claude instead. Before waking it, Hollow drops jobs that no longer need running, and a job stays queued until it succeeds. One that fails is tried again after 30 minutes, then after an hour, and asking for the same job again doesn't hurry that; after three failures it is listed for you with Try again and Discard. Automatic re-asks, such as the session list's titles, a chat's title and learning suggestions, a check-in's prep pack and the heartbeat, don't restart a job that gave up, so a job the model can't do doesn't keep waking it; your own request, such as Prepare, Recap, Keep transcript or an About-you scan you start, does. An automatic re-ask never undoes yours: a check-in's prep pack joining your waiting Prepare rewrite leaves it a rewrite. The heartbeat's Run now that waits for its model runs when the model wakes, even outside work hours, as it would at once; it doesn't run if a beat ran after you pressed it or a budget holds the heartbeat (the Activity ledger says it was dropped, and a Run now a budget already holds is refused with its reason instead of waiting), and switching the heartbeat off drops a Run now that is still waiting. A job that waits says when Hollow tries it again, with the day when it isn't today ("tomorrow at 00:30"), and Try again names the model the job now waits for. If you set a kind back to As now, even while its model is starting, its waiting jobs are held and listed with Run as now and Discard, so nothing private moves to Claude without your click. With two copies of Hollow open, they never run the same waiting job twice. If the choices file is damaged, every kind is held until it is fixed. A chat on a self-hosted model now gets its title and learning suggestions from that same model when you send those kinds to it, and none of these jobs' results go to Jev's duplicate check.
- **Where:** Settings → Agents → Self-hosted models → Background jobs: a model for each kind, "N jobs waiting for <name>" with Run them now, and held jobs with Run as now or Try again, and Discard.
- **Undo / turn off:** Set a kind back to As now. The choices are `self-hosted-jobs.json` in Hollow's app data; waiting jobs are `self-hosted-job-queue.json`.

### A self-hosted model's usage reads billed by GPU time
- **What:** A turn on a self-hosted model records its tokens under the model's served name, and its usage in the chat shows the token count with no price: the hover note says it is billed by GPU time on that endpoint, not per token. It is never shown as $0 and never priced as if it were a Claude model. Usage totals, the overnight budget and the morning report count such turns as billed by GPU time rather than unknown, so a self-hosted night turn never stops the night's budget.
- **Where:** A chat on a self-hosted model, in the turn's usage; Usage totals and the morning report.
- **Undo / turn off:** No setting.

### A self-hosted model's chat runs on this Mac
- **What:** A chat on a self-hosted model now runs on this Mac even when Runs on sends turns to the runner: a workspace's own Runs on, Settings' Runs on, or Overnight and worker chats. Hollow serves the model through this Mac's local forwarder and signs in to it here, so the runner can't use it. The turn's first line says so, for example "The runner (runner-host) can't use Qwen3.8 27B: Hollow serves it through this Mac's local forwarder and signs in with this Mac's gcloud, so this turn runs on this Mac." With Runs on set to the runner but no runner set up, it says that instead. Nothing is sent to the runner for that turn, with one exception: a turn the chat still had running there when Hollow quit is stopped and brought back first, as for any message, and its line follows the first one. The first line also says what of an earlier turn is still on the runner: if the runner didn't answer (or wasn't asked, because it didn't answer minutes ago), that turn's work is still there and a later message in the chat brings it back once the runner answers; if its work couldn't come back, only a turn there on a model that isn't self-hosted brings it back. Runs on itself doesn't change: turns on a model that isn't self-hosted in the same chat or workspace still go to the runner, and the Runs on help in a workspace and in Settings says a self-hosted model's turns stay on this Mac.
- **Where:** Chats on a self-hosted model whose Runs on is the runner; nothing to set.
- **Undo / turn off:** No setting. Choose a model that isn't self-hosted in the chat's model menu to run there.

### Set up a runner from scratch
- **What:** Remote runner's Set up now walks you through a runner of your own, step by step: a Mac you own that stays on (a hidden standard account for the work, a key of its own, the toolchains, a preflight, no sleep), or a Google Cloud machine with no public address that stops itself when idle. Every step shows the exact commands it runs, so you can read or copy them. Check only reads and changes nothing; each step's Apply is the only thing that changes anything, and the ones that touch accounts, keys, sleep or sshd ask first. When sudo there needs a password, the step says so and gives you the commands to run in Terminal. Google Cloud shows a monthly estimate first, and Create asks with that estimate before it makes anything; it hasn't been tried against a real project yet.
- **Where:** Settings → Agents & models → Remote runner → Set up (Set up again once a runner is set).
- **Undo / turn off:** Turn off in the same row stops using the runner. What an Apply made stays on that machine: each step's commands say what it changed.

### A self-hosted chat shows when its model is starting
- **What:** A chat on a self-hosted model, such as Qwen on Cloud Run, has a line above the message box saying whether the model is up. Before anything wakes it, it reads "Qwen3.8 27B sleeps when idle" with a Start now button. Hollow wakes the model when you start typing or send a message in that chat, at most once every 2 minutes, or when you press Start now; opening, scrolling or reading a chat never does, and neither does typing a / command such as /model, because its GPU costs money while it is up. The line says Checking for a few seconds; if the model is asleep it then says it is starting, about 4 minutes, and counts the time so far, and a message you send meanwhile waits for it, with no error. Then it says the model is ready, or what went wrong in plain words. Two chats on one model share one start, and a reloaded window picks up a start that is already running. The usage ring measures the chat against the model's own window (131,072 tokens for Qwen), not 200K.
- **Where:** Above the message box in a chat on a self-hosted model; Start now is on that line.
- **Undo / turn off:** No setting. To stop wakes, switch the model off in Settings → Agents → Self-hosted models.

### Chat with a self-hosted model
- **What:** Each self-hosted model you switch on is in a chat's model menu under Claude, named like "Qwen3.8 27B · self-hosted". Choose it and that chat's turns run on your own server through Hollow's local forwarder, with Hollow's tools, streaming answers and Stop as on any Claude turn; your Claude sign-in never goes there, and a turn never moves to another Claude account. If the model was switched off or removed, or its list can't be read, the turn stops with a message saying why instead of using another model. Moving a chat between a self-hosted model and a Claude model starts a fresh session with the conversation so far. The first turn after the model has been idle can wait several minutes while it starts. Settings' default model, the extraction model and an automation's model don't take them (Background jobs sends Hollow's own calls to one instead), and a chat set to run on the Mac mini runs them on this Mac (see A self-hosted model's chat runs on this Mac). In such a chat, claude.ai connectors and Claude teammates stay out (they would use your Claude login), and the chat keeps its first title and its turns aren't used for learning suggestions unless Background jobs sends those kinds to that same model.
- **Where:** A chat's model button → Claude → the model's "· self-hosted" row (the palette's Switch model… opens the same menu). Switch models on in Settings → Agents → Self-hosted models.
- **Undo / turn off:** Choose another model in the menu, or switch the model off in Settings.

### App builds can compile on the remote runner
- **What:** Build app in a Hollow build workspace can compile Hollow on the remote runner instead of this Mac, so this Mac stays free while it builds. The runner has no signing key, so this Mac signs the finished app with its own identity before Install and restart, which works as usual. If the runner can't build it, it builds here. It is off until you turn it on. Use it only with a runner you control, since what it builds gets this Mac's signature and Hollow's permissions, and try one build there by hand first, since the runner may use an older macOS SDK than this Mac.
- **Where:** Settings → Agents & models → App builds, under Builds and tests.
- **Undo / turn off:** Set App builds back to This Mac.

### Database tests can run on the Mac mini
- **What:** An agent can run a test suite that needs a database with its own throwaway Postgres, on the Mac mini or on this Mac, wherever its build and test commands go. The database exists only while the command runs and is deleted after, and the usual database settings point at it, not at a real one. Portal's whole integration suite now runs on the Mac mini this way.
- **Where:** Build and test commands in any chat that can run commands (`hollow-slot --postgres <command>`).
- **Undo / turn off:** No setting. Leave out `--postgres` to use a database you started yourself.

### Self-hosted models in Settings
- **What:** Settings keeps a list of models you run on your own GPU or server, such as Qwen on Cloud Run served by vLLM. Each one has a name, address, served model id, context window, the effort levels it offers and how Hollow signs in to it (a token command such as `gcloud auth print-identity-token`, or none). Use the Qwen on Cloud Run preset, beside Add or inside the form, fills the form for Qwen3.8 27B (Settings search finds it too). Test wakes the model through Hollow's local forwarder, checks that its model list names it and its window matches, and asks it for one token: while it runs it says "Starting the model, this can take about 5 minutes", then shows the health check and first byte times or the error. Each one switched on is offered in chats (see Chat with a self-hosted model).
- **Where:** Settings → Agents → Self-hosted models (Add a self-hosted model, then each model's Test, Edit, Remove and on/off switch).
- **Undo / turn off:** Switch a model off, or Remove it. The list is `self-hosted-models.json` in Hollow's app data.

### A remote runner is yours alone
- **What:** Settings → Agents & models has a Remote runner row. It says Off until you set one up: Set up takes the SSH host of a computer of yours that stays on, Check asks it how it is without changing anything, and Use this runner saves it. Hollow ships no machine names, so a new Hollow never reaches anyone's runner. Once set, Record me as owner marks the runner as yours, and its state warns when another SSH key can log in there, when another Hollow recorded itself, or when no owner is recorded; hollow-slot says the same about extra keys. Runs on's choice and the turn notices now say Remote runner and the runner's name instead of Mac mini.
- **Where:** Settings → Agents & models → Remote runner.
- **Undo / turn off:** Turn off in the same row; Runs on goes back to this Mac for everything.

### Builds and tests pick the Mac mini or this Mac
- **What:** When the Mac mini is set up as this Mac's runner, an agent's build and test commands decide for themselves where to run, and each says where and why in its first line, such as "on the Mac mini (gate, usually 3 min; this Mac 72% free)". Gates, Rust builds, whole test suites, typechecks, lints and builds go to the Mac mini; one test file, anything that needs this Mac (its database, gcloud, GitHub, the keychain, files outside the checkout) and anything that writes into the checkout stay here. A command also stays here when the Mac mini lacks a tool it needs, and comes back here when the Mac mini is busy and this Mac isn't. Settings shows the latest decisions and what the Mac mini has.
- **Where:** Settings → Agents & models → Builds and tests, under Runs on. Build and test commands in any chat that can run commands.
- **Undo / turn off:** Set Builds and tests to This Mac (a command that asks for the Mac mini itself still goes), or write `off` in `~/.config/hollow/slot-remote`.

### Mac mini turns can open pull requests and reach Google Cloud
- **What:** A workspace can let its turns on the Mac mini push, open and merge pull requests, and use Google Cloud. Both are off until you turn them on for that workspace. GitHub uses a fine-grained token you make for the runner (saved in this Mac's Keychain) or, if you choose, this Mac's gh login. Google Cloud uses your account without production changes by default, your full account, or a service account you name. The credential stays on this Mac and reaches the Mac mini one git, gh or gcloud call at a time; nothing is stored there. The turn's first line says what it can reach.
- **Where:** Runs on → On the runner (a build workspace's Changes, or a new conversation), and Settings → Agents → Runner GitHub access for the token.
- **Undo / turn off:** Untick GitHub or Google Cloud there; Remove token in Settings.

### A video named with % or # plays
- **What:** When a chat reply names a video in code formatting or as a plain path, such as `/Users/me/take#2.mov` or `/Users/me/50%20off.mov`, the player under the reply now plays that file. A `#` in the name used to hide the video, and a `%` code such as `%20` made the player look for a different file. Links and `<video>` tags in a reply work as before.
- **Where:** The videos under a finished chat reply.
- **Undo / turn off:** No setting.

### Import a source opens the Add sheet
- **What:** Import a source, quick capture and Add are one sheet. Library's ⋯ → Import a source, the palette, ⌘⌥⇧I and the menu-bar icon open Add set to Library source, with optional Domain and Purpose; ⌘⌥⇧T opens it set to Task, reading the line as before (fri 4pm · for 90m · p1 · #project · @label). Enter adds a task or imports a single link; imports run in the Activity dock, and a failed one puts your text back. After a save from another app, Hollow steps back to it. The two separate quick windows are gone.
- **Where:** Library → ⋯ → Import a source; New → Add… → Library source or Task; ⌘⌥⇧I, ⌘⌥⇧T or the menu-bar icon from any app.
- **Undo / turn off:** No setting. Settings → Interface → Keyboard shortcuts changes or removes ⌘⌥⇧I and ⌘⌥⇧T, and a key you set for either before keeps working.

### A Mac mini turn goes on when Hollow quits
- **What:** If Hollow quits while a turn runs on the Mac mini, Continue picks that run up there: the rest of its output, then its work. Any other message first brings its work back.
- **Where:** That conversation's Continue, or Waiting on you → Resume.

### Settings in seven sections
- **What:** Settings lists seven sections instead of fourteen: Interface, Agents, Connections, Knowledge, Automation, Usage and System. Nothing is gone and no setting changed: each former section now sits whole, under its own heading, inside the section it belongs with. Keyboard shortcuts is in Interface; Chat, Accounts and Skills are in Agents (Agents & models' rows come first); Companion devices moved from System to Connections; Search & indexes is in Knowledge (About you & memory's new name); the Hollow folder and History & recovery are at the end of System. Settings search, the command palette and every link that used to open one of the old sections (Edit in the model menu, Manage key, a notification) land on its heading in the new place. A new Settings opens on Interface.
- **Where:** Settings (the index on the left; below 860 px, the section picker).
- **Undo / turn off:** No setting. Search Settings by the old name to find any former section.

### Files your .gitignore names go to the Mac mini
- **What:** Workspace turns there now take and bring back what a `.gitignore` names (up to 2,000 files, 200 MB). Caches, files over 50 MB and secret-looking files stay put; the first and last lines name them.
- **Where:** Workspace turns on the Mac mini.
- **Undo / turn off:** No setting.

### Conversation history is in Library
- **What:** Conversation history is no longer a page of its own: it is Library's last tab, Conversations, with the same list, search and project filter, working folder, open conversation and message box, and About this conversation. Show archived is now a Hollow · Archived switch above the list. Settings' Conversation history button, a project's "N older in Conversation history" in the sidebar and a Library file's conversations open the tab, and Back and Forward step to and from it. Opening Library from the sidebar still shows the view you had there before.
- **Where:** Library → Conversations (also Settings → System → History & recovery → Conversation history).
- **Undo / turn off:** No setting. Your conversations, archived ones included, are unchanged.

### One Ask: every question is a chat
- **What:** Asking has one place: Ask anything on Your work. Where to ask offers Across all my work (the Main agent, on its page) and Quick chat (a new conversation; Add to project starts it in a project instead). Answer from my notes (the Ask your vault view) and Plan it in Hollow (the Ask Hollow dialog) are gone: ask in a chat, which answers from your notes and does the work in the conversation. A chat reply's More actions now has Save as note: in a project it saves the reply into that project's Results (`outputs/answers/`), elsewhere into the Library (`library/`), never over a file that is already there, and the message that says where has Open. A sentence typed in the command bar gets one Ask row, sent where Ask anything is set to send. Ask anything… in the command bar, and g v, open the field. Plans Ask Hollow already ran stay in History & recovery, with Undo.
- **Where:** Your work → Ask anything → Where to ask; a reply's More actions → Save as note.
- **Undo / turn off:** No setting. If you had last chosen Answer from my notes, Ask starts on Quick chat; if Plan it in Hollow, on Across all my work.

### A reply's cited lines open at those lines
- **What:** When a chat reply names a file with a line, such as `docs/plan.md:42`, `src/app.ts:10-20` or a link ending in `#L12`, clicking it opens the file in the Preview pane at those lines: its source, with the cited lines marked and scrolled into view. A line range such as `src/app.ts:10-20` is now a file link too. A file named without a line still opens at its top.
- **Where:** A chat reply's file links (click, or Enter on a focused one) → the Preview pane.
- **Undo / turn off:** No setting. For a Markdown, HTML, JSON or table file, the pane's Preview button shows it rendered again.

### Next 7 days is now Agenda's week
- **What:** Tasks has three layouts: List, Board and Agenda. Next 7 days is gone; Agenda's Week does what it did, beside your meetings and focus blocks: drag a task onto a day to set its due date, and the tasks outside those seven days sit under them in Not this week (Earlier, Later, No date), ready to drag onto a day. Agenda now remembers whether you last chose Day or Week. If Tasks last opened on Next 7 days, it now opens on Agenda's Week.
- **Where:** Tasks → Agenda → Week (`g c`, the palette's Open Agenda and Today's Open Agenda open Agenda on the range you last chose). Without dragging: a task's ⋯ (Due today, Due tomorrow, Due next Monday) or its details' Edit task.
- **Undo / turn off:** No setting. Choose Day in Agenda, or List or Board in Tasks.

### Weekly review moves into Today
- **What:** Your work has four tabs: Home, Today, Map and Running. Today has a **Day / Week** switch at its top. Day is today's brief, as before; Week is the weekly review, unchanged (its weeks, the take, the cards and Save to Results). The weekly review's Friday-to-Monday count is now on the Today tab: choose Week to read it (on Monday morning it opens last week). Saved weekly reviews and what you have already read stay as they were.
- **Where:** Your work → Today → Week. `g e` and the palette's **Weekly review** open it there; `g d` and **Open Today** open Day.
- **Undo / turn off:** No setting.

# What's new: 2026-11-02

### Commits made on the Mac mini are yours
- **What:** Commits an agent makes in a turn on the Mac mini now carry the same git name and email this Mac uses for that worktree (a work identity included, where your git config picks one for that repository), instead of a made-up "Hollow runner" one. Gates and tests sent to the Mac mini get it too. If this Mac has no complete git name and email for the worktree, the turn's first line says so and the Mac mini's default is used.
- **Where:** Turns set to run on the Mac mini (Runs on: Mac mini), and overnight and worker chats that go there.
- **Undo / turn off:** No setting. Set `user.name` and `user.email` in the worktree's git config to change who commits.

### The morning report says what's still on the Mac mini
- **What:** A night-queue item whose turn ran on the Mac mini but whose work didn't come back (the return failed, or Hollow quit before the turn finished) now says so in the morning report: its work is still on the Mac mini, and the next turn there from that conversation brings it back (set the workspace's Runs on to Mac mini first if it runs on this Mac).
- **Where:** Automations → Last night (the item's line), and the written morning report.
- **Undo / turn off:** No setting.

### Cut-off Mac mini work that can't come back is set aside, not cleared
- **What:** When the Mac mini restarted mid-turn and the next turn can't save what that turn left (it keeps trying for an hour from the first failure, no longer giving up at once when the turn stopped hours earlier), the Mac mini now moves the leftover copy aside instead of clearing it, keeps it there for up to 14 days (sooner when its disk runs low), and that turn's first line says where.
- **Where:** Turns set to run on the Mac mini.
- **Undo / turn off:** No setting.

### Unvouched Mac mini changes come back as copies
- **What:** Changes an earlier workspace turn left on the Mac mini, made on a copy of the folder this Mac can't confirm it sent (an old one it no longer remembers, or one made up there), no longer sit there unseen forever. The next Mac mini turn in that conversation brings them here only as conflicted copies beside your files, writing over and removing nothing, says so in its first line, and clears them from the Mac mini. When a worktree turn's work can't be put on a branch here, the message now says what git reported (a locked ref, for example).
- **Where:** Turns set to run on the Mac mini in a workspace (Runs on: Mac mini).
- **Undo / turn off:** No setting. Delete the copies you don't want.

### Work from a cut-off Mac mini turn comes back
- **What:** If the Mac mini restarts in the middle of a turn (a power cut, an update), what the agent had already changed there is no longer lost: the next Mac mini turn in that worktree or workspace conversation keeps it and brings it back, a worktree's on a separate branch, a workspace folder's merged in like any change made there. Changes kept on the Mac mini from an earlier turn are also only applied against a copy this Mac sent for that turn, never a starting point made up there.
- **Where:** Turns set to run on the Mac mini.
- **Undo / turn off:** No setting.

### Overnight and worker chats run on the Mac mini
- **What:** Work nobody is watching now goes to the Mac mini by default, so this Mac can sleep: night-queue items, chats an unattended run starts, and the chats a Main agent worker batch starts, in a build workspace that hasn't chosen where it runs. Unlike a turn you send there yourself, such a turn never fails because the Mac mini can't take it: when it doesn't answer, is short of disk or memory, or can't run that agent (no added Claude account, Codex without an API key), the turn runs here and its first line says why. Once one turn finds it down, turns in the next few minutes run here at once instead of each waiting for it. Your own chats keep following Runs on, and workspace chats still move only on their own choice.
- **Where:** Settings → Agents & models → Overnight and worker chats (Mac mini when it answers, or Follow Runs on).
- **Undo / turn off:** Set Overnight and worker chats to Follow Runs on.

### Video renders and transcriptions can run on the Mac mini
- **What:** An agent can send a long media job to the Mac mini instead of running it on this Mac: a HyperFrames video render or a whisper transcription of a recording. The files it needs go there, the job runs on the Mac mini's GPU, and the finished video or transcript comes back to the same folder here. One job runs there at a time, with at most two render workers. If the Mac mini doesn't answer, the job runs here as before.
- **Where:** Chats that render videos or transcribe recordings, when the Mac mini is set up as this Mac's runner.
- **Undo / turn off:** No setting. Ask the agent to run it on this Mac.

### The Mac mini keeps room for its work
- **What:** The Mac mini clears out its oldest working copies when its disk runs low (under 40 GB free), not only after 14 days unused, so turns and gates there keep finding space. It only clears copies nothing has used for a while, and a turn's first line says when it did. A gate run there no longer removes a Mac mini conversation's build folder, which it could do even while that conversation was running.
- **Where:** Turns and gates on the Mac mini (Runs on: Mac mini, or a gate an agent runs there).
- **Undo / turn off:** No setting.

### See whether the Mac mini is ready
- **What:** Settings shows whether the Mac mini answers, how much disk and memory it has free, its Rust, Node and browser versions, what is running there now and how its last gate went. Check now asks again. When it doesn't answer, the message names what to check for that kind of failure: after a restart it waits at the FileVault unlock screen until someone signs in on it, Tailscale when its name can't be looked up, Remote Login when it refuses. A turn that can't reach it says the same.
- **Where:** Settings → Agents & models → Runs on, under the choice.
- **Undo / turn off:** No setting. It shows only while a runner is set up.

### Mac mini turns survive a dropped connection
- **What:** A turn running on the Mac mini keeps going when the connection to it drops, for example when this Mac sleeps or changes networks. Hollow reconnects on its own and picks the turn's output up where it left off, so nothing shows twice or goes missing, and the agent keeps Hollow's tools (a tool call caught by the drop gets an error instead of hanging). Stop still ends the turn there. If Hollow quits mid-turn, the turn goes on there until the next turn in that folder, which stops it and brings its work back on a separate branch.
- **Where:** Any turn set to run on the Mac mini (Runs on: Mac mini).
- **Undo / turn off:** No setting. Set Runs on to This Mac to run turns here.

### Workspace chats can run on the Mac mini
- **What:** A workspace's own chats can run their Claude and Codex turns on the Mac mini too, not only build workspaces. The turn works in its own copy of the workspace's folder there (brief, plan, sources, outputs), and its changes come back when the turn ends. Changes made here meanwhile, by you or another chat, stay; a file both sides changed is merged when the edits don't overlap, and otherwise (and always for files that aren't Markdown or plain text) this Mac's version stays with the Mac mini's beside it as a conflicted copy, which Waiting on you offers like any other. A build workspace's turn on the Mac mini now gets its workspace folder beside the worktree the same way. Files over 50 MB stay here. The Main agent and quick chats always run on this Mac.
- **Where:** A new conversation in the workspace (Runs on: This Mac, Mac mini). A workspace moves only when it chooses the Mac mini there; the Settings default stays for build workspaces.
- **Undo / turn off:** Set that workspace's Runs on to This Mac.

# What's new: 2026-11-01

### Build workspaces can run on the Mac mini
- **What:** A build workspace's Claude and Codex turns can run on the Mac mini instead of this Mac. Only the agent moves: the conversation, History, tools and Stop stay here, the turn works in its own copy of the worktree there, and its work comes back to the worktree when the turn ends (onto a separate branch if you changed the worktree meanwhile). The chat says where a turn ran and how its work came back. If the Mac mini doesn't answer, the turn stops and says why, including that after a restart it waits at the FileVault unlock screen. Claude uses one of your added accounts; the Claude Code login stays on this Mac. Codex goes only with an API key: a Codex turn signed in with ChatGPT stops before anything is sent and says to run Codex on this Mac.
- **Where:** Settings → Agents & models → Runs on (the default), and a build workspace's Changes tab or a new conversation there (Runs on: Default, This Mac, Mac mini).
- **Undo / turn off:** Set Runs on to This Mac.

### Builds wait while memory is low
- **What:** A build, test or dev server an agent starts waits while less than 30% of this Mac's memory is free, then starts; after five minutes it gives up and says why. This now holds for every agent, not only Claude. Gates run on the build runner also count in this Mac's gate statistics, and a Codex chat that can't reach the runner says why at once.
- **Where:** Build and test commands in any chat that can run commands.
- **Undo / turn off:** No setting.

### Chat uses the whole pane again
- **What:** The conversation and the message box fill the width beside the sidebar instead of a narrow centred column.
- **Where:** Every project and quick chat.
- **Undo / turn off:** No setting. Narrow the window or open the Preview pane for shorter lines.

### A quieter message box
- **What:** Fewer buttons under the message box, and none overlap. + holds Attach an image… and the commands. The model button holds the model, effort, Permissions and Canvases, and shows the permission mode beside the model (red in Bypass). Context lists what goes with your message, now including Connectors for the next turn; its button shows a plug mark when a connector needs attention.
- **Where:** The bottom row of the message box.
- **Undo / turn off:** No setting. Paste or drag an image to attach it as before; typing / still lists the commands.

### Tests and gates can run on another Mac
- **What:** An agent can run a test or merge gate on another Mac set up as a build runner, with `hollow-slot --remote <command>`, so this Mac's memory and processors stay free. The exact files go along, committed or not, the output streams back, and Stop stops it there too. When the other Mac doesn't answer or is short of memory or disk, the command runs here and the agent is told why.
- **Where:** Build and test commands in any chat that can run commands. The runner is an SSH host named on one line of `~/.config/hollow/slot-remote`; a build that runs here mentions it.
- **Undo / turn off:** Write `off` in that file, or delete it.

# What's new: 2026-10-30

### Workers when you request them
- **What:** Ask a project or global Main agent to do five tasks and it can create five separate local worker chats. Questions about your projects get a direct answer. Workers keep the main agent's model and permissions; existing limits determine when each runs.
- **Where:** Project sidebar → Main agent, or the global Main agent.
- **Undo / turn off:** Stop a worker from its chat. Stopping the parent turn stops the workers it started in that turn.

### Local projects without Notion setup
- **What:** Agent guidance now uses Hollow's local project tools for ordinary project requests. A missing legacy folder no longer tells the agent that Notion is required.
- **Where:** Ask a main agent to find or create a project, or use the Projects plus button.
- **Undo / turn off:** No setting. Notion project creation remains a separate request.

### A main agent for every project
- **What:** Each project has a persistent Main agent conversation. New projects open there; ordinary task chats and their drafts stay separate. Reopening keeps the chat's model and permissions.
- **Where:** Project sidebar → Main agent, or the project's menu → Open main agent.
- **Undo / turn off:** Use New conversation for an independent task. The main agent runs only when you send a message.

### Cleaner project creation
- **What:** The plus beside Projects opens a compact dialog that keeps the existing chat in place. Name and instructions come first; templates and repository settings sit under Templates and location. Keyboard focus stays in the dialog.
- **Where:** Sidebar → Projects → plus, or New → New project.
- **Undo / turn off:** Escape or Cancel closes the dialog before creation. Create project opens the main agent without sending; Create and start also sends the instructions.

### Smoother long conversations
- **What:** Tool results and new reply bubbles update their own message instead of repeatedly traversing the entire chat history. This reduces local update work in long chats without changing the selected model.
- **Where:** All conversations, automatically.
- **Undo / turn off:** No setting. This improves local responsiveness; it does not change the provider's generation speed.

# What's new: 2026-10-28

### Fold for Hollow and Main agent
- **What:** A dithered, layered doorway is the stock app icon and matching Main agent artwork. Existing uploads stay yours.
- **Existing default:** Cut Glass previously stored no override, so anyone using that default now sees Fold. Choose Cut Glass from the presets to keep the earlier look.
- **Where:** Settings → Interface → Appearance. App icon offers Fold (default), Threshold, Cut Glass, Aurora and Monolith. The stock mark appears in the Dock, Finder, top-left brand and favicon; Main agent keeps its status ring and badges.
- **Undo / turn off:** Pick another app preset, or Upload/Make one for either target. Reset to Fold restores the app; Main agent's Reset to default restores its matching Fold art. Reset an earlier custom upload to see the new stock artwork.

# What's new: 2026-10-27

### Wait or continue when an agent runs out
- **What:** When a chat's agent runs out of usage, the chat now says when it can go on ("Waiting for Claude until 16:40"). You can also choose another agent to continue on: the turn moves there only if the one that ran out changed nothing, starting from the conversation and its task state.
- **Where:** Settings → Agents & models → When an agent runs out.
- **Undo / turn off:** Set it back to Wait for it, the default; check-ins, automations, night lanes and child tasks always wait.

### Let a chat read a tab you share
- **What:** Share a Preview tab with the conversation on screen and its agent can read the page's text, so it can work from a signed-in report or a running app. It reads only: nothing is clicked or typed, and the share lasts one turn, the one running or else the next.
- **Where:** Preview → a web tab → Share with chat; the tab shows "In use by" with Stop.
- **Undo / turn off:** Stop on the tab, or close it; automations, check-ins and other unattended runs can never read a shared tab.

### See where each started chat stands
- **What:** Chats the Main agent starts end their answer with Done, Partial or Blocked and what would unblock them; the Main agent page shows it as a chip, so you can see "Blocked: needs your choice" without opening the chat.
- **Where:** Main agent → Chats I started.
- **Undo / turn off:** Nothing to turn off; a chat that gives no such line shows "No status line".

### Import a link with any agent
- **What:** Hollow reads a public web page itself, signed out, and hands its text to your agent, so importing a link works with Codex as well as Claude Code. Pages behind a sign-in are refused; YouTube links work as before.
- **Where:** Library → ⋯ → Import a source → Link, or New → Add… with a link.
- **Undo / turn off:** Nothing to turn on; local and private addresses are never fetched.

### Measure text on the picture
- **What:** When a canvas has text over a gradient, picture or texture, the design check can now measure it on the drawn canvas instead of leaving it unresolved.
- **Where:** Under a reply canvas → "N layout and design problems" → Measure on the picture.
- **Undo / turn off:** Runs only when you click it; nothing is saved or sent, and the canvas is never changed.

### Services in one place
- **What:** Sam's Calendar, Notion and GitHub setup cards bring service setup together. Notion keeps one connection editor; GitHub starts with a read-only preset and preserves existing access.
- **Where:** Settings → Connectors → Services.
- **Undo / turn off:** Cancel a setup draft; use each service's existing controls to change or disconnect access.

### Warnings for agent versions known to break Hollow
- **What:** Hollow carries a short list of agent CLI versions known to break it, each with a reason. Agent updates flags one you have, warns before updating to one, and Automatic won't install one by itself.
- **Where:** Settings → Agents & models → Agent updates.
- **Undo / turn off:** Warnings only: Update anyway still installs a listed version, and Restore can go back.

### Drafts stay with their chat
- **What:** Each conversation keeps its unsent text and images separately. New chat has its own draft; opening another chat no longer carries the previous text across.
- **Where:** Project conversations and Quick chats. The 100 most recent drafts are retained within a one-million-character text budget; images stay in this window.
- **Undo / turn off:** Clear a chat's composer to clear its draft. A notice says when text could not be saved for restart.

### Projects and Quick chat weeks
- **What:** Sam's sidebar groups Quick chats by creation week and names your work Projects. One Add to project picker works from Home and conversation actions.
- **Where:** Sidebar → New project / Quick chats; Your work → Tasks; chat → Add to project.
- **Undo / turn off:** Collapse a week; moving a chat offers Undo and Move to Quick chats.

### Charts that fit the data
- **What:** Agents choose a canvas's chart from the shape of the data: a number for one value, bars for a few categories, a line over time, a donut for parts of a whole, a table for more.
- **Where:** Canvases in replies.
- **Undo / turn off:** No setting of its own; Settings → Chat → Canvases sets how often canvases appear.

### Know where a skill came from
- **What:** Hollow remembers where each skill you import came from (the file, folder or git repository and commit) and says when its files changed outside Hollow. Check for update… shows what a new version changes before anything is replaced, and a local note stays in the skill through updates.
- **Where:** Settings → Skills → a shared skill → Source.
- **Undo / turn off:** Nothing updates by itself; Cancel on the update card keeps the installed copy, and Remove note takes the note out.

### Start with a conversation
- **What:** Sam’s Home puts Ask first, followed by five recent projects.
- **Where:** Your work → Home; All projects for search and recovery. Saved canvases are in Today; unassigned Notion tasks are in Tasks.
- **Undo / turn off:** Project data and sidebar pin order remain intact.

### Documents and pages together
- **What:** Sam's mixed Preview tabs and PDF paging, zoom and readable text.
- **Where:** Open a file or web link → Preview; PDFs also preview in Results.
- **Undo / turn off:** Close a tab or close Preview.

### Share a page's text from Annotate
- **What:** Annotate can add the page's readable text (title, address, main text and tables, up to 24 KB) next to the marked-up picture, so the agent can read a page you're signed into. It shows as a chip you can remove before you send.
- **Where:** Preview pane → a web page → Annotate → Include page text → Add to chat.
- **Undo / turn off:** Leave Include page text unticked (it starts off), or remove the chip with × before sending.

### Crimson accent
- **What:** Sam's neutral surfaces and red accent; distinct warning/error marks.
- **Where:** Stock themes; Settings → Interface.
- **Undo / turn off:** Choose a custom theme.

### Each chat keeps its permissions
- **What:** Sam's per-chat modes; new chats keep your saved default.
- **Where:** Chat → Permissions.
- **Undo / turn off:** Choose another mode.

### A design check for canvases
- **What:** A reply canvas's findings row also checks the HTML and CSS the agent wrote: fixed text colours, text under 11px, low contrast, text over a texture, its own motion, emoji icons and unlabelled SVG. Up to five, each with a fix; the receipt's Canvases row gives the count.
- **Where:** Under a settled reply canvas → "N layout and design problems" → Ask to fix.
- **Undo / turn off:** Advice only: the canvas is never changed or hidden, and Ask to fix only fills the message box.

### Clearer Claude quota recovery
- **What:** Sam's quota recovery and clear account holds.
- **Where:** Settings → Accounts; chat.
- **Undo / turn off:** Disable account failover.

### Clearer daily facts
- **What:** Ruled utility views.
- **Where:** Today, Weekly, Usage, Build, History, Commitments.
- **Undo / turn off:** No setting.

### Ruled Automations and Map
- **What:** Flatter surfaces.
- **Where:** Automations; Your work → Map.
- **Undo / turn off:** No setting.

### A simpler Settings sheet
- **What:** Quiet navigation and grouped controls.
- **Where:** Settings.
- **Undo / turn off:** No setting.

### Find Main actions
- **What:** Actions opens and focuses its evidence.
- **Where:** Main agent → Actions.
- **Undo / turn off:** Close the disclosure; navigation and drafts keep their guards.

### Choose the amount of texture
- **What:** Full, Quiet or Off artwork with a live preview; Ambient motion keeps your saved choice.
- **Where:** Settings → Interface.
- **Undo / turn off:** Texture Off keeps plain drawings. Motion is separate; accessibility wins.

### One waiting list
- **What:** Waiting filters decisions, results and failures; Incoming lists sources.
- **Where:** Review → Waiting or Incoming; Your work → Running.
- **Undo / turn off:** All restores the full waiting list; existing filters and shortcuts still work.

### Fewer, clearer places
- **What:** The Main agent drawer, From other apps and Claude history are gone. ⌘⇧M opens the Main agent page.
- **Where:** The orb in the top bar; Ask anything → Across all my work sends there.
- **Undo / turn off:** No setting; saved data stays on disk.

### Files and canvases together
- **What:** One filtered Results list for files and canvases; files open in Preview.
- **Where:** Library → Results. Workspace Canvases links and Open canvases still reach the filtered list.
- **Undo / turn off:** Choose a type filter; Close hides Preview. Review badges remain on rows; Review owns the review queue.

### Still, architectural empty views
- **What:** Static stippled drawings and a folded-threshold release cover.
- **Where:** Empty tasks, sources, Review and What's new.
- **Undo / turn off:** Texture Off or accessibility settings keep plain drawings.

### Long chats keep up while a reply streams
- **What:** A streaming reply no longer re-reads the whole conversation for every word, so long chats stay light while the answer arrives.
- **Where:** Every chat, most in long ones.
- **Undo / turn off:** No setting.

### Quieter choices and overlays
- **What:** Flat question docks, compact palette, opaque dialogs and still menus.
- **Where:** Search, confirmations, menus and chat questions.
- **Undo / turn off:** Theme, text size and accessibility settings apply.

### Room for workspace documents
- **What:** Workspace documents have a readable central page, ruled sections and clearer navigation. Sources and History use compact continuous lists.
- **Where:** Open a workspace → Workspace → Overview, Brief, Plan, Results, Sources or History.
- **Undo / turn off:** Chat returns to your conversation; Markdown and section actions stay available.

### A focused Main agent space
- **What:** Orbital art, a ruled day column and linked counts beside Main's conversation.
- **Where:** Main agent → Review or Actions; Automations → Main agent actions.
- **Undo / turn off:** Hide its day for more chat room; accessibility simplifies artwork.

### A denser task workbench
- **What:** Ruled board and week lanes, compact list rows and a quieter detail/editor pane.
- **Where:** Tasks, in every existing layout.
- **Undo / turn off:** Theme and text-size choices remain available; saved tasks and views stay intact.

### Claude answers start sooner
- **What:** Claude starts up while Hollow gathers your context, instead of after it.
- **Where:** Every Claude chat.
- **Undo / turn off:** No setting.

### Resume from a quieter Home
- **What:** Compact workspace rows; status links open Review and Last night's details.
- **Where:** Your work → Workspaces.
- **Undo / turn off:** Text size, theme and density settings apply; data and actions stay.

### Easier Library scanning
- **What:** Notes and results use ruled rows with compact previews; recordings keep transcript times and task proposals clearly separated.
- **Where:** Library → Notes, Results and Recordings.
- **Undo / turn off:** Existing theme and accessibility choices still apply.

### Previews open only when you click
- **What:** The side panel is gone. A file or link you click opens in a Preview pane beside any page.
- **Where:** Reply file chips, links and citations; workspace documents are under the top bar's Workspace.
- **Undo / turn off:** × or Esc closes it; ⌘\ brings back the last one.

### Web pages open, or say why
- **What:** A page that doesn't answer says so, with Try again, instead of staying blank. Plain-http sites load; a ninth tab closes the oldest.
- **Where:** Any web link you click; the address field also opens a file path.
- **Undo / turn off:** Hold ⌘ on a link to open it in your browser instead.

### One continuous conversation
- **What:** One reading column, labelled prompts, compact work folds, flat composer and doorway art.
- **Where:** Every chat; file actions stay on replies.
- **Undo / turn off:** Settings → Interface controls text size, density and motion.

### A clearer workspace list
- **What:** Compact workspace headers, conversation guides, a quiet context bar and still selection.
- **Where:** Sidebar; running-work popover → Manage → Your work → Running.
- **Undo / turn off:** Settings → Interface keeps sidebar, density and text-size controls.

### A quieter canvas workbench
- **What:** Flat evidence planes, compact tables and steps, comparison rails and continuous question rows. ASCII flows and exact fractional progress keep data clear.
- **Where:** Canvas replies, saved canvases and ask-canvases.
- **Undo / turn off:** Existing themes and accessibility choices still apply; answer controls stay native.

### Textured instruments, exact values
- **What:** Static gauge and donut patterns, ASCII calendar bins and explicit missing-data states. Small trends and numbers stay crisp.
- **Where:** Ask for a textured instrument canvas in chat; Data table retains exact values.
- **Undo / turn off:** Request plain texture; accessibility preferences always win.

### Chats get ready sooner
- **What:** Each message's search of your connected project files scores them about five times faster, with the same evidence.
- **Where:** Every chat, before the agent starts.
- **Undo / turn off:** No setting.

### Patterned chart evidence
- **What:** Static dot fills on bar, line and area charts; solid edges, values and tables remain clear.
- **Where:** Ask for a chart canvas with dot fills in chat.
- **Undo / turn off:** Ask for plain charts; accessibility preferences always win.

### Canvas texture choices
- **What:** Canvases can request a static texture; existing canvases stay plain.
- **Where:** Canvas answers in chat and saved canvas views.
- **Undo / turn off:** Ask for a plain canvas.

### A quieter workbench
- **What:** Paper and graphite planes, compact controls and glyph doorway art; no pointer spotlights.
- **Where:** Throughout Hollow; doorway art on Home and empty chats.
- **Undo / turn off:** Settings → Interface keeps theme, density, motion and texture choices. Accessibility wins.

# What's new: 2026-10-26

### Read source freshness
- **What:** Freshness words on closed source rows.
- **Where:** Workspace → Sources.
- **Undo / turn off:** No setting.

### Retry an interrupted view
- **What:** Local Retry for unexpected Review, Incoming or Runbooks failures.
- **Where:** In the failed view; tabs remain usable.
- **Undo / turn off:** No setting.

### Read Companion workspace names
- **What:** Full wrapping names and clearer shared-data counts.
- **Where:** Settings → System → Companion devices.
- **Undo / turn off:** No setting; sharing choices stay unchanged.

### A quieter Main day
- **What:** Compact empty sections and aligned action times; errors stay prominent.
- **Where:** Main agent → Show its day.
- **Undo / turn off:** Hide its day closes the band.

### Clearer saved canvas surfaces
- **What:** Solid pinned cards and grouped copy items.
- **Where:** Your work pins; canvas actions → Add items to Plan or Tasks.
- **Undo / turn off:** No setting; controls stay in place.

### Plan progress comes first
- **What:** Progress first, full preamble in Plan notes.
- **Where:** Workspace → Plan → Summary.
- **Undo / turn off:** Open Plan notes.

### Compact automation covers
- **What:** Less spacing, stronger Next run, usage note below actions.
- **Where:** Automations → an automation.
- **Undo / turn off:** No setting; all controls remain.

### Recover a result search
- **What:** Clear search and Reset filters act independently.
- **Where:** Library → Results → Files, when nothing matches.
- **Undo / turn off:** Reapply the search or filters.

### Read plan progress by section
- **What:** Visible section names/counts beneath Overview's plan bar.
- **Where:** Workspace → Overview; All sections opens the full list.
- **Undo / turn off:** Close All sections to collapse the full list.

### Compare pictures before source
- **What:** Before/After previews with optional source changes.
- **Where:** Library → Results → Canvases → Compare versions.
- **Undo / turn off:** View source changes restores the diff.

### Results take shape while loading
- **What:** Four quiet placeholders; search stays visible and refresh keeps card focus.
- **Where:** Library → Results → Files.
- **Undo / turn off:** No setting; cards replace placeholders when ready.

### See transcript matches
- **What:** Highlighted words and speakers, matching-segment counts and Clear search.
- **Where:** Library → Recordings → Find in transcript.
- **Undo / turn off:** Clear search restores the full transcript.

### A more welcoming start
- **What:** Small architectural illustrations and a visual release cover.
- **Where:** Empty views across Hollow; Settings → What’s new.
- **Undo / turn off:** Existing actions and motion preferences stay in place.

### Clearer utility pages
- **What:** Solid reading cards, clearer totals and wrapping titles.
- **Where:** Build, Usage, History, Imports, Runbooks and Commitments.
- **Undo / turn off:** No setting.

### Canvases feel at home
- **What:** Clearer numbers, solid callouts, calm charts, timelines and comparisons.
- **Where:** Existing canvases throughout Hollow.
- **Undo / turn off:** No setting.

### Calmer notices and questions
- **What:** Consistent notice surfaces, clearer question choices and gentler dialog entrances.
- **Where:** Notices, menus, dialogs and agent questions.
- **Undo / turn off:** Reduced motion keeps entrances still; existing actions stay in place.

### Clearer automation instruments
- **What:** Schedule timelines, recorded durations and solid Map labels.
- **Where:** Automations; Your work → Map.
- **Undo / turn off:** Visual styling has no switch. Map → List is the text alternative.

### Preview a palette result
- **What:** Grouped results and roomy workspace/task/note previews.
- **Where:** Search anything; Settings → Interface → Experimental visuals.
- **Undo / turn off:** Turn Experimental visuals off. Highlighting never opens or runs a result.

### Easier Library reading
- **What:** Files sit closer to search on paper-like cards. Transcript speakers sit above aligned text.
- **Where:** Library → Notes / Recordings.
- **Undo / turn off:** No setting.

### Main agent at a glance
- **What:** Running chats, all-work Review counts and provider-colored working orbs.
- **Where:** Top-bar Main agent, its page and drawer.
- **Undo / turn off:** Experimental visuals off stops waiting warmth; idle stays still.

### A satisfying task check
- **What:** Newly completed checks settle once, retaining their row through save.
- **Where:** Tasks.
- **Undo / turn off:** Settings → Interface → Experimental visuals off, or Reduced motion.

### A calmer conversation
- **What:** Clearer prose, a quiet Worked for ribbon, framed canvases and a fine-edged composer.
- **Where:** Workspace and Main agent conversations.
- **Undo / turn off:** Experimental visuals off leaves a static caret; chat controls stay in place.

### Failures first in Review
- **What:** Failures lead the waiting list; Review and Activity gain clearer cards and readable headings.
- **Where:** Review; a workspace's Activity section; Running.
- **Undo / turn off:** Existing filters and recovery actions stay in place.

### Experimental light and depth
- **What:** Working light, a warm waiting orb, pointer-lit edges and subtle artwork depth.
- **Where:** Settings → Interface → Experimental visuals, on by default.
- **Undo / turn off:** Turn it off there. Reduced motion and accessibility preferences win; effects pause away.

### Clearer settings and spending caps
- **What:** Readable theme samples and cap meters; incomplete costs stay worded.
- **Where:** Settings → Interface / Usage.
- **Undo / turn off:** Theme and limit controls stay in place.

### A more tangible shell
- **What:** Fine-edged chrome and selection, with task rows growing into their details.
- **Where:** The top bar, sidebar and Tasks.
- **Undo / turn off:** Settings → Interface → Motion → Reduced makes plain cuts.

### More readable task cards
- **What:** Three-line titles, label chips and status-aware empty columns.
- **Where:** Tasks → Board / Next 7 days.
- **Undo / turn off:** List keeps compact rows.

### A shared material language
- **What:** Fine edges, clearer Glass, optical titles and steady counters.
- **Where:** Buttons, fields, menus and dialogs throughout Hollow.
- **Undo / turn off:** Existing appearance and accessibility choices still apply.

### A clearer day and week
- **What:** Draft previews, a compact day ring, and distinct all-day/untimed groups.
- **Where:** Your work → Today / Weekly review; Tasks → Agenda → Week.
- **Undo / turn off:** Open Preview draft for the full text; drafting still waits for you to send.

### Edit your own skills
- **What:** Use in names its workspace. Skill descriptions keep line breaks; save/delete errors stay visible.
- **Where:** Settings → Skills.
- **Undo / turn off:** Edit skill again.

### Room to read your work
- **What:** Two-line titles, metadata, project art, balanced grid and local-time greeting, with compact spacing at Largest text.
- **Where:** Your work → Workspaces.
- **Undo / turn off:** No setting.

### An open window stays reachable
- **What:** A focused window stays reachable despite stale tray state.
- **Fixed 2026-11-16:** Opening a browser tab no longer makes approval cards
  or navigation report that Hollow's visible window is closed. Dock, tray and
  menu reopening keep working with browser tabs attached.
- **Where:** Agent navigation and approval cards; Dock, tray and menu reopening.
- **Undo / turn off:** Close to tray and approval choices stay.

### Clearer workspace summaries
- **What:** Next leads Brief; context folds. Covers show format/size; Sources shows coverage.
- **Where:** Workspace side panel → Brief, Results and Sources.
- **Undo / turn off:** Older context or Markdown shows the full brief.

### A lighter workspace shell
- **What:** Your work loads separately; older release notes are shorter.
- **Where:** Your work; Settings → What's new.
- **Undo / turn off:** No setting; controls stay.

# What's new: 2026-10-25

Changes prepared for the next Hollow build.

### Dictation stops when it can't help
- **What:** Sending/editing stops dictation; click/Esc cancels startup. Errors link to Privacy.
- **Where:** Composer → Dictate.
- **Undo / turn off:** No setting.

### Canvas items become Plan rows or tasks
- **What:** Edit saved items, preview, then Confirm; retries avoid duplicates.
- **Where:** Canvas actions → Add items to Plan or Tasks….
- **Undo / turn off:** Close before Confirm; afterward edit Plan/delete copied tasks.

### Find what failed in Activity and Logs
- **What:** Activity filters status/kind/day. Logs has Failed, Jump to latest failure and Copy shown text.
- **Where:** Workspace Activity; panel ⋯ → Logs.
- **Undo / turn off:** Any status / All tools.

### Results and Plan say checks in words
- **What:** Results name checks, review and source. Refused Plan ticks offer Tick it now; Compare opens PLAN.md.
- **Where:** Workspace Overview, Results and Plan.
- **Undo / turn off:** No setting.

### Know which build is running
- **What:** The window and tray name the version and commit.
- **Where:** Tray → Copy Build Identity; search “version” in the palette. System shows full build details.
- **Undo / turn off:** Nothing to turn off.

### Shortcuts follow macOS
- **What:** Keys use macOS notation. System-wide keys need ⌥⇧⌘; unavailable menu actions explain why. Go-to cards wait for your key.
- **Where:** Settings → Keyboard shortcuts; the menu bar.
- **Undo / turn off:** Each key has Reset and Remove, Go to keys too.

### Annotate with the keyboard
- **What:** Add comments, boxes and whole-page notes; edit the numbered marks.
- **Where:** Browser → Annotate. Arrows and Enter place marks; Add to chat fills the draft.
- **Undo / turn off:** Undo/Redo restore marks; Close keeps the draft; Discard asks first.

### The command palette finds what you mean
- **What:** Search by word; workspace + “plan/results” opens that section. “Open Brief” uses the current workspace. ⌘K closes; ⌘J searches Workspaces. Search failures stay visible.
- **Where:** Palette (⌘K).
- **Undo / turn off:** No setting.

### More room for larger text
- **What:** Compact layouts follow text size; Tasks stays reachable when bulk actions wrap.
- **Where:** Settings → Interface → Text size, then resize any page.
- **Undo / turn off:** Default text or a wider window restores room. Sidebar choice persists within a layout range.

### The browser panel and Annotate keep their keys
- **What:** Browser ⌘[/⌘]/⌘R act on its focused page. Annotate ⌘Z edits notes or undoes marks; ⇧⌘Z restores marks. Toolbar actions wrap.
- **Where:** Side panel → Browser; its Annotate bar (Undo, Redo, Discard).
- **Undo / turn off:** Redo restores removed marks; cancelling Discard keeps them.

### A build workspace's Changes tell one story
- **What:** Changes separates built, committed and installed; failed reads offer Retry. Update from origin/main rolls back conflicts and is disabled while chats run. Queued messages name their worktree with Open it.
- **Where:** A build workspace's side panel → Changes; a queued message's badge in a build workspace's chat.
- **Undo / turn off:** Nothing changes until you confirm Update; a conflicting update leaves the branch as it was.

### Reduced motion is consistent
- **What:** Reduced motion closes overlays immediately and skips smooth search scrolling. Spinners keep working away; launch content stays visible.
- **Where:** Settings → Interface → Motion; macOS Reduce Motion also applies.
- **Undo / turn off:** Full plus macOS Reduce Motion off restores motion; session marks pause while the window is away.

### Appearance follows you
- **What:** Text size reaches canvases and HTML. Dark controls match; system contrast and transparency preferences apply. Custom themes show measured contrast. Match macOS follows appearance.
- **Where:** Settings → Interface → Appearance / Text size.
- **Undo / turn off:** Pick a theme or Default text size. Custom files stay unchanged.

### Back and Forward step through workspaces
- **What:** Back/Forward (⌘[/⌘]) visit workspaces/chats. Return/New name the workspace; disabled actions explain why. Reply text has native Copy/Look Up. Older chats open workspace history with Show all.
- **Where:** Top-bar Back/Forward/return/New; selected reply text; workspace conversations in the sidebar.
- **Undo / turn off:** Conversation history → Show all lists all chats.

### Clearer agent failures and command help
- **What:** Failures name recovery; declined approvals hide their request. The model menu shows catalog Refresh and agent update status. Steering needs a qualified stable Codex; others queue messages. Help follows the selected agent. Retry runs refused during updates afterward.
- **Where:** Model menu, failure and send notices → Agent updates; Settings → Agents & models → Agent updates; Commands → /help.
- **Undo / turn off:** No settings change; continue or retry when ready.

### One menu for every conversation, with Undo
- **What:** Conversation menus share Open, New conversation, Rename…, Move to…, New workspace from this chat, Copy conversation ID and Archive. Disabled items explain why; working chats cannot archive. Move/Archive offer Undo; failed moves into new workspaces remove the new workspace. History has archive/restore and Show archived; sidebar workspace menus appear on hover/focus.
- **Where:** A conversation's right-click or ⋯ menu; Settings → History & recovery → Conversation history; a workspace's header in the sidebar.
- **Undo / turn off:** Undo on the message after an archive or move; Restore conversation under Show archived.

### Archive, Hide, status and Merge each do one thing
- **What:** Archive removes the row; Unarchive restores its status. Hide lasts until you pick/start a chat. Mark as done has Undo. Merged rows and target Overview offer Undo merge…. Merge's confirm defaults to Cancel, explains next-turn folders and refuses running work in either workspace.
- **Where:** A workspace's right-click or ⋯ menu (Archive…, Unarchive, Mark as done, Merge into…); Your work → Show all workspaces → All work; the merged-into workspace's Overview.
- **Undo / turn off:** Unarchive, the Undo on Mark as done, and Undo merge… put each change back.

### One run record everywhere
- **What:** Chat, Activity and Automations share What happened and Run record. Activity shows failure output/Timeline. Check commands copy exactly; shortened records say so. Repository files and new citations open in the panel with cited lines highlighted.
- **Where:** A reply's More actions → Show run record; a workspace's Activity → a run; Automations → an automation. In Chat, Ask to rerun adds a request after your draft for you to review and send.
- **Undo / turn off:** Close the Run record disclosure or remove the drafted request. Viewing and copying do not rerun a command.

### Undo a workspace delete, and Recently deleted
- **What:** Delete offers Undo for the workspace and its conversations. All work → Recently deleted lists deletions newest first; Restore explains name conflicts. Failed rename, status, archive and delete actions show messages on any page. Open brief in Obsidian explains when Obsidian isn't installed.
- **Where:** A workspace's right-click or ⋯ menu → Delete…, then Undo; Your work → Show all workspaces → All work → Recently deleted.
- **Undo / turn off:** Delete again moves it back to the trash.

### Pick which interrupted chats to resume
- **What:** After Hollow quits mid-turn, Resume interrupted chats lists each chat with a tick, its workspace, agent, model and permission mode (and Ultracode when it's on), so you can resume some and leave the rest for later; Not now holds the focus. Chats an agent started that the same quit stopped are listed below with Open, never resumed from here. The message after a resume names every chat that didn't resume and why. A chat Hollow's quit cut short shows a pause mark and "interrupted" (Hollow quit before this turn finished. Open it and press Continue.) instead of a red "failed".
- **Where:** Review → Interrupted by a restart → Resume N interrupted chats; the sidebar's conversation rows and the workspace map.
- **Undo / turn off:** Untick a chat or press Not now; unticked chats stay in Review to resume later.

### Canvas answers stay put, queue while the agent works, and say when they didn't go
- **What:** What you chose in a canvas's questions stays when you scroll far away and back or use Read as text. While the agent is still answering, the canvas's Send reads Queue answers and sends them once the turn ends. If answers don't go, the canvas says so with Send now and Take them back instead of moving them into the message box. An older version of a canvas offers Go to the current version instead of Send.
- **Where:** A conversation: the Send button under a canvas that asks you questions, and the same canvas in the side panel.
- **Undo / turn off:** Take them back drops a failed send and leaves your answers in the canvas; Cancel on the waiting message removes queued answers.

### Clear keyboard focus
- **What:** Keyboard focus keeps a solid outline on Incoming, note-answer chips and commitment sources. Selected Incoming rows keep their bar, tab strips keep the whole ring, and row quick actions stay visible with larger targets.
- **Where:** Review → Incoming, Answer from my notes, Tasks → Commitments, Automations and conversation chains.
- **Undo / turn off:** No setting needed; the outline appears when using the keyboard.

### Steering says what happened, in words
- **What:** A message sent into a running turn that doesn't go in says why: the turn hadn't started or was finishing, the agent can't take one mid-turn, or the message was too long or the turn already took 20. Enter queues on its own, and the hint says why (a long draft, a queued turn, an image, or Hollow still checking the agent). A steer the agent didn't take shows its reason, with Send as next message and Put back in the box. While a turn runs, /model, /agent, /cwd and /permissions with a value, /new, /mcp and /plugin aren't sent as words: Hollow says to send them once the turn ends, and your text stays. /help and the bare commands still answer at once. In the Main agent drawer, Stop matches the chat's and reports a failed stop, and Enter follows your send key.
- **Where:** A conversation while a turn runs: the hint under the message box, Send options, and a steer row that wasn't taken; the Main agent drawer.
- **Undo / turn off:** Send options still picks Queue for after or Stop and send for one message.

### Retry a failed message, copy a conversation, and Esc that stays in its popover
- **What:** Failed replies offer Retry and Retry with another agent. Esc closes Connectors first and stops a run only with an empty message box. Feedback appends to drafts. Find opens matching Worked for folds; Copy conversation keeps all Markdown. Finished labels completed runs, Preferences opens About you & memory, and /clear still works.
- **Where:** A conversation: under a failed reply; Conversation tools (top right of the transcript) → Copy conversation (Markdown); the composer's Connectors.
- **Undo / turn off:** Retry sends only when you press it. To stop with text in the box, clear it first or press Stop; Settings → Keyboard shortcuts can give Stop a ⌘ chord instead of Esc, which works with text in the box too.

### Icon edits keep your draft
- **What:** Main agent, What's new and Memory have distinct symbols. Icon edits ask before discarding changes, failed reads prevent saving, and a changed saved icon keeps your draft for an explicit replacement choice. Recovery names the correct Reset button.
- **Where:** Settings → Interface → Appearance → App icon or Main agent icon; Main agent → Change icon….
- **Undo / turn off:** Keep editing retains your draft. Reset to Fold restores the app icon; Reset to default restores the Main agent icon.

### Settings has 15 sections
- **What:** Chat now holds Enter sends a message, Retitle new chats and the permission mode for new chats, next to Canvases, with a link to Chats at once; the model menu's Edit opens it. Response feedback is Usage → Votes; each vote opens its conversation at that turn. Context prioritizer, Search by meaning and Search inside project files are one section, Search & indexes. About you and Memory are About you & memory, with the adaptation map folded. Companion devices moved into System, which folds its developer readouts; Warn under shows before you check disk space. Agents & models folds agents that aren't installed. Manage key opens Accounts on the Jev row, and both Jev switches say to add a key first. A profile question's Skip is now Don't ask again.
- **Where:** Settings; Find a setting still finds every old section name.
- **Undo / turn off:** Nothing to undo: old links and searches land on the new places.

### A compact Home with one workspace list
- **What:** Recent workspace cards lead under smaller artwork. Every workspace starts folded, pin actions use pin symbols, and closed Connections stops its background reads. Bursts of conversation updates refresh recent cards together; a failed refresh keeps the last cards and a quiet status.
- **Where:** Your work → Workspaces → Show all N workspaces or Find; sidebar → All workspaces. Connections & imported work opens the local mirror view.
- **Undo / turn off:** Hide workspaces folds the list. Close Connections to pause its reads; Retry refreshes recent cards immediately.

### Map controls and a text list
- **What:** Scroll pans the map, pinch zooms, and clicking a star focuses its system. Zoom in, Zoom out and Fit are visible. List shows the same workspaces with activity, open work and live state, plus guarded Merge into… and Archive actions.
- **Where:** Your work → Map → List; each list row's options menu.
- **Undo / turn off:** Show all clears system focus. Turn List off to return to the map; Cancel leaves a merge or archive unchanged, and Archive offers Undo.

### Settings asks before dropping unsaved changes
- **What:** Opening another Settings section while a form has unsaved changes (Hollow folder, Automation windows, Agent defaults, a custom theme, the companion backend, the disk-space warning) asks first: Keep editing, or discard and go. Leaving Settings itself still doesn't ask. A new section opens at its top with its heading focused, and until Hollow quits Settings reopens on the section you left. Find a setting now reaches every control it lists (swarm limits, account failover, Import skill, the icon presets by name), Enter opens the top hit, and the search stays above the section picker in narrow windows. Interface's switches, Search inside project files and effort say Saved; Agent defaults frames what its Save keeps.
- **Where:** Settings → any section; Settings → Find a setting.
- **Undo / turn off:** Keep editing returns to the form with your changes; nothing is saved or dropped until you choose.

### Notices wait for you and keep a history
- **What:** Notices pause while Hollow is behind another window or you hover or focus them. App messages keeps the last 50 notices from this session, including dismissed ones, with Copy. Install results stay until dismissed and offer Reveal log.
- **Where:** Review → Recent notifications → App messages; install results appear in the notice stack.
- **Undo / turn off:** Dismiss closes a notice without removing its history. Undo remains available only on the live notice; the history clears when the app session ends.

### Plaud's two sign-ins are named apart
- **What:** Hollow says plainly how meetings reach it: Recordings in Hollow uses Hollow's own Plaud sign-in and copies nothing unless you choose Keep a transcript copy; Plaud tools for agents come from the Plaud connector, with its own sign-in; calendar comes from the source you connect. The Plaud row shows both statuses together, and Disconnect asks first. Signed out, Library → Recordings' Sign in to Plaud for Recordings lands on that sign-in's Connect.
- **Where:** Settings → Connectors → Recordings; Library → Recordings.
- **Undo / turn off:** Cancel keeps you signed in; Connect signs in again after a Disconnect.

### Live state stays in the collapsed sidebar
- **What:** Pinned workspaces and Quick chats keep live-state badges; Your work summarizes other projects. Workspace lists show loading and Retry, timestamps age, and build conversations show their provider instead of repeating the PR glyph.
- **Where:** Sidebar → pinned workspace, Quick chats or Your work; the open workspace's status remains in the narrow top bar.
- **Undo / turn off:** Show sidebar expands the conversation trees. Hide sidebar returns to the compact icons; neither stops running work.

### Library's tabs keep their state honest
- **What:** Results appear on save and offer Refresh. By meaning names its collection; saved views remember it. Show/Hide file history works in every tab. New note focuses its name; tab changes close it. Result files and Kept transcripts link to their tabs.
- **Where:** Library → Notes, Results, Recordings and Connected projects; Library actions (⋯) → Show file history.
- **Undo / turn off:** Turn By meaning off to search by name again; Hide file history closes the panel.

### Quick entry keeps what you typed
- **What:** Quick Capture and Quick Ingest keep drafts and errors when you return. Quick Ingest preserves paragraphs; Enter imports and Shift-Enter adds a line. The Add sheet keeps unsent text for the session and shows file-picker failures.
- **Where:** Quick Capture / Quick Ingest; Top bar → New → Add a task, note, link, file or recording…
- **Undo / turn off:** Escape clears an overlay draft. In Add, use Discard draft; Escape keeps its text for later. An operation already submitted may still finish.

### Rename, move and trash notes in Library
- **What:** Rename a note, move it to another folder, or move it to the vault's trash without leaving Hollow. A rename warns when other notes link to the note by name. A note's single line breaks now read the same in Library, the preview window and Skills, and "Linked from" shows note titles.
- **Where:** Library → a note's ⋯, or File actions in the reader. Snapshots and a workspace's own files don't offer these.
- **Undo / turn off:** Undo right after Move to trash puts the note back. Rename or move it again to change it back.

### Create a workspace and start from its goal
- **What:** The form explains why Create is unavailable, rejects untouched starters and shows repository read errors with Retry. Command-Enter creates; Create and start also sends the goal in the new conversation. A failed open can retry without making another workspace.
- **Where:** Top bar → New → New workspace.
- **Undo / turn off:** Cancel or Escape closes the idle form. Create workspace saves the goal without sending it.

### What's new remembers where you left off
- **What:** A new-release divider and count show what arrived since you last opened the reader. Small windows have a Release picker. If earlier releases cannot load, search names its limited coverage and offers Try again.
- **Where:** Settings → System → Help → What's new, or What's new in this build after a confirmed installation. Take the tour opens Where things are, also offered once after setup.
- **Undo / turn off:** Close the reader or tour. Opening the reader acknowledges its latest release; older releases remain available.

### Canvas changes say what is kept
- **What:** Read-only changes are labelled, failed saves offer Try again, and stale pins offer Update to the current version. Pin notices offer Show, with Replace and Unpin in the menu.
- **Where:** Canvas heads; Results and Library → Results → Canvases → Canvas actions → Open source chat or Export canvas. Exported files offer Show in Results.
- **Undo / turn off:** Unpin removes a pin. Replace or Update changes its saved version only after verification. Close the canvas to leave its read-only view.

### Scheduled work names its configured window
- **What:** The scheduled-work section no longer calls a future weekday "tonight". Its dated interval and next-run coverage remain visible.
- **Where:** Automations → Tonight → Scheduled in this window.
- **Undo / turn off:** Nothing to turn off; this changes labels only.

### Usage names its provider and keeps partial Codex tokens
- **What:** Estimates name their source. Stopped or failed Codex turns keep known tokens as partial. Plan meters show percent used, including Codex limits when it reports them.
- **Where:** The usage ring beside the composer and Settings → Usage. Refresh Codex limits reads the last passive snapshot.
- **Undo / turn off:** These views start no provider work; close the card to hide it.

### Reply images, code and media are easier to use
- **What:** Local reply images show clickable thumbnails, home-folder file links open, long code folds without shortening Copy, Bash rows copy commands and output, and failed videos offer Try again. Remote images explain why they wait and offer Load N images.
- **Where:** Conversation replies; expand a Bash row for its Copy buttons. Click a local thumbnail to open the side panel.
- **Undo / turn off:** Show fewer lines folds code again. Remote images load only when you click their loading controls.

### Choose automation windows and triggers by clicking
- **What:** Schedule has saved-window and catch-up choices, three upcoming window starts, and local trigger add/remove. Saving a new trigger lists it in the approval.
- **Where:** Automations → open an automation → Automation options → Edit → Schedule. Settings → Automation → Automation windows lists linked automations and Add automation.
- **Undo / turn off:** Choose Manual, remove a trigger, or cancel before saving. Settings links wait until window changes are saved or discarded.

### Spending caps in one list, and the Main agent’s day in one line
- **What:** Settings → Usage → Budgets lists the caps Hollow keeps above your own budgets: the Main agent's daily
  hard stop and the overnight cap (spend, limit and state, changed right there) and the bake-off monthly cap
  (read-only). Automation's cap inputs became links to those rows. The Main agent's band says what today cost on its
  own and for what you asked, with Change hard stop.
- **Where:** Settings → Usage → Budgets; top bar → Main agent → Show its day.
- **Undo / turn off:** change a cap back in the same row; nothing to turn off.

### Review has one entry, an honest count and ⌘5
- **What:** the top bar's bell is gone; the sidebar's Review row is the one way in, with the same count ("–" until
  Hollow has read what waits, or after a read fails). Unsaved file drafts are counted apart, after ten idle minutes
  with no editor open, and wait in All waiting, where Discard (with Undo) drops one. ⌘5 opens Review.
- **Where:** sidebar → Review (⌘5); Go → Review; Review → All waiting → Unsaved drafts → Discard.
- **Undo / turn off:** Discard's Undo brings the draft back; change ⌘5 in Settings → Keyboard shortcuts (a ⌘5 you
  gave another command stays there).

### Opening a failure in Review leaves it waiting
- **What:** opening a failure from Failures or Incoming follows its link and keeps it in Failures until you clear it, so a failure
  you only glanced at isn't lost. Mark handled, x or resolving it clears it. A failure with nothing to open says so.
- **Where:** Rail → Review → Failures (and Incoming).
- **Undo / turn off:** nothing to turn off.

### Task rows have a ⋯, and their keys work once you Tab to a row
- **What:** every task row and card has a ⋯ (More actions) with the same items as its right-click. With a row
  focused, j/k, the arrows, x (select) and . or ⇧F10 (its menu, which takes the keyboard) all work; on Board and
  Next 7 days the arrows stay in a column, and in Agenda they leave the list alone. The select box sits inside Board and
  Next 7 days cards. In Commitments, Drop is now the delete key instead of x (u still undoes), and the key hints hide
  when keyboard shortcuts are off.
- **Where:** Rail → Tasks (List, Board, Next 7 days); Rail → Tasks → Commitments.
- **Undo / turn off:** Settings → Keyboard shortcuts turns the letter keys off; the ⋯ and the arrows stay.

### Tasks filters have one home, and the layouts clear names
- **What:** saved filters live only in Filters and narrow the scope tab you're on, with one "Filtered by … · Clear
  all" row above the list. The Week layout is now **Next 7 days**; the built-in All and Today filters are gone, and a
  **Ready** filter finds tasks nothing blocks. The bulk bar adds Select all shown, Reopen and Move to. Agenda shows
  only the tasks your search and filters show. A new task keeps the filters it matches, else they clear, with Restore.
- **Where:** Rail → Tasks → Filters; the layout switch; the bulk bar after selecting a task.
- **Undo / turn off:** Clear all, or Restore on the notice after a new task.

### Notion-linked tasks move by drag, menu and bulk edits too
- **What:** a task linked to Notion now takes a drag to a Board column or a day, its menu's Move to and Due items,
  and bulk Complete, Reopen, Move to and due dates the way its tick does: Hollow picks the matching option in your
  Notion database (In progress becomes your "Doing") and offers Undo. A column with no matching option doesn't light
  up, and the menu item says why it's off. Conflicts show dates and names instead of code.
- **Where:** Rail → Tasks (Board, Next 7 days, a row's ⋯, the bulk bar, the task's panel → Edit other fields).
- **Undo / turn off:** Undo on the notice after each change, or Undo on the bulk edit's notice.

# What's new: 2026-10-24

What the night of 10-23/24 added, from the polish plan. Installed with `aac51c9b` on 10-24.

### Tonight includes scheduled work
- **What:** Tonight includes scheduled automations and background jobs, with origin, agent, budget and conversation. Automations group by attention, automatic and manual work.
- **Where:** Automations → Tonight; Runbooks → New runbook opens a two-step template. Dry-run tonight adds a dated report and Copy report. Open **Background work windows** to see the clock and its window controls.
- **Undo / turn off:** Views start nothing; Create note creates a runbook. Queue and schedule controls stay in place.

### Review puts what unblocks running work first
- **What:** Running chats' asks get priority within groups. Filter by workspace; x dismisses, a accepts, Shift-R rejects, × deletes notifications. Snooze lasts until 9:00 AM; Wake now reverses it. Failures offer Mark handled, read errors Try again. Open cards and scroll survive returning.
- **Where:** Rail → Review (All waiting, Decisions, Failures).
- **Undo / turn off:** Undo on the Rejected and Snoozed notices; Wake now in Decisions.

### Today says when its facts changed, and its rows open what they name
- **What:** Today labels changed facts and writes only in the foreground. Due excludes closed/cancelled tasks. Waiting rows open their items, commitments show overflow counts, and weekly drafts preview. Close dismisses the idle map.
- **Where:** Today (in the command palette); Your work → Weekly review; Tasks → Agenda; the workspace map.
- **Undo / turn off:** nothing to turn off.

### The Main agent shows its day clearly
- **What:** Wrapping status and separate day/drawer buttons. Its day labels Review, mode, suggestions, seven-day chats and dated actions. Failed reads offer Retry; missing standing orders can be restored without replacing files.
- **Where:** top bar → Main agent → Show its day.
- **Undo / turn off:** Hide its day closes the band. Edit standing orders changes the restored file.

### Ask says what Enter does
- **What:** The two-line Ask field explains each destination and remembers your last choice. Across all my work sends to the Main agent drawer; ⌥-Enter only drafts. Plan it in Hollow and Answer from my notes open windows with matching names.
- **Where:** Your work → Ask anything → Where to ask.
- **Undo / turn off:** press ⌥-Enter (or hold ⌥ and click Put it in the box) to place a question without sending it.

### Running now stays visible all week
- **What:** Your work keeps Running now on weekends, shows overdue and due-today counts, and gives Waiting items their own titled links. Last night always opens Automations; Night details expands its summary. The weekly reminder is a count on the Weekly review tab.
- **Where:** Your work → Workspaces → At a glance; Your work → Weekly review.
- **Undo / turn off:** nothing to turn off. Opening Weekly review clears that week's reminder.

### A quieter composer while the agent works
- **What:** Ultracode stays with its conversation, including Resume all. While working, the empty composer explains Enter and Stop; typing reveals the hint below. Stop names only an assigned key. Permissions announces its mode and unrestricted Bypass; Esc closes its menu and returns focus.
- **Where:** a conversation's composer → the model menu → Effort → Ultracode; the permissions button beside it.
- **Undo / turn off:** pick a saved effort level to turn Ultracode off for that chat.

### Workspace cards show their state in words
- **What:** recent workspace cards show run state, result review state and open plan items. Queued work says Queued; unreadable Results say so. Waiting questions and results to review have buttons into Review. Loading cards and Retry with Details explain reads still in progress or failed.
- **Where:** Your work → Workspaces → Pick up where you left off.
- **Undo / turn off:** nothing to turn off.

### Choose the calendar Agenda reads
- **What:** use Calendars on this Mac, Hollow's Microsoft 365 sign-in, or Off. Mac calendars also supply Today and meeting prep, with a choice of calendars to include. Hollow reads events and never changes them.
- **Where:** Settings → Connectors → Agenda calendar. If macOS denies access, Open System Settings and then Refresh calendars.
- **Undo:** choose Microsoft 365 to return to the previous source, or Off to stop calendar reads. Tasks and local focus blocks remain available.

### One word, one meaning: Running, History, Summary, Blocked
- **What:** Activity is Running, History or Log by scope. Labels use Background tasks, Recent notifications, Summary, Blocked and Last active. Draft for the Main agent fills its box; Apply ticked and Do it now name their actions.
- **Where:** Your work → Running; a workspace's side panel → History; Review → Recent notifications; Tasks →
  Blocked; Your work → Today → Draft for the Main agent.
- **Undo / turn off:** nothing to turn off.

### Plainer words: automations, imports, schedules and toasts
- **What:** Labels use automation, Import a source…, History & recovery and Tonight. Schedules read Mondays 8:00 AM; run states read Finished / Failed / Stopped. Sources explains native office-file reads. Changing an automation's workspace preserves its results folder. Close buttons, notices and Silent heartbeat use consistent wording.
- **Where:** Automations; the menu bar; a workspace's Sources → Context and freshness limits; Settings → Automation.
- **Undo / turn off:** nothing to turn off.

### Section toolbar: views switch, actions ask, Markdown opens the file
- **What:** Canvas / Summary / Markdown switch views; Ask and Prepare refresh request draft only. Markdown opens the section's file. Brief retains history; Sources uses one card list. Overview flags later failures; Results and Full review share a filter.
- **Where:** a workspace's side panel → Overview, Brief, Plan, Sources or Results → the toolbar at the top.
- **Undo / turn off:** nothing to turn off.

### Your work opens on Workspaces every time
- **What:** Your work opens on Workspaces from the rail, ⌘1, palette and menu bar. Workspace panels keep Overview. Palette icons match the rail; canvas menus say Pin to Your work. Main-agent navigation names the tab it opened.
- **Where:** Rail → Your work → Workspaces; a canvas's pin menu → Pin to Your work.
- **Undo / turn off:** nothing to turn off.

### Panel requests open the right workspace conversation
- **What:** revision, starting-request, brief-update and file/passage Ask actions open their workspace before adding text. A failed open leaves the draft untouched; repeated clicks add one request. Nothing sends automatically.
- **Where:** workspace side panel → Results, Plan, Brief or a file → its Ask action.
- **Undo:** edit or clear the drafted request before sending.

# What's new: 2026-10-23

What the night of 10-22/23 added, from the polish plan. Installed with `8bb59a92` on 10-23.

### Weekly review's next week labels events that run over several days
- **What:** in Weekly review's Next week, an event that runs over several days shows its start time on the day it
  starts, "continues" on the days between, and "until" its end time on the day it ends. Before, a trip from Sunday
  evening showed as a 6 PM event on Monday, and the agent's draft read it the same way.
- **Where:** Your work → Weekly review → Next week.
- **Undo / turn off:** nothing to turn off.

### The side panel remembers where you were in each workspace
- **What:** switching workspaces no longer sends the side panel back to Overview on the Workspace tab. Each
  workspace's panel opens on the section, tab and wide reading you left it on, for as long as Hollow is open; a
  workspace you haven't opened yet starts on Overview. Opening a result or the plan from elsewhere still goes there.
  In a narrow panel, a tab that isn't selected shows a dot when it has something new.
- **Where:** a workspace's side panel: the sections under Workspace, the wide reading, and the side tab (Files and
  Visuals belong to the conversation, so a switch opens Workspace instead).
- **Undo / turn off:** nothing to turn off; pick another section or tab as before.

### Task drafts survive linked edits and can resolve conflicts
- **What:** sheet labels and steps stay saved as drafts while you edit synced fields. Changed tasks offer Review / Discard, with per-field choices before saving against the reviewed version. A slow or unavailable Notion mirror no longer blocks local completion ticks.
- **Where:** Tasks → Edit task, the task sheet, and a workspace's Notion view → Edit.
- **Undo:** close to keep a draft, or Discard to remove only that form's draft. Reviewing changes does not save until you press Save.

### Wikilinks open in Library; the palette keeps your pick; Prepare works during a meeting
- **What:** a `[[link]]` in a Library note opens that note, from the reader or the editor; a link to a note that
  doesn't exist offers **Create note** with the name filled in. In the command palette, the row you moved to stays
  highlighted when live data changes the list, so Enter runs the row you picked. **Prepare** works for a meeting
  that has already started. Failure messages stay at least 7 seconds, and image files on a conversation's files shelf
  show an image icon instead of a blank tile.
- **Where:** Library (a note's reader and editor); the command palette (⌘K); Today → Prepare; Conversation files.
- **Undo / turn off:** nothing to turn off.

### The Hollow app switch says what agents can do
- **What:** the Hollow app switch now says that agents can see and change Hollow (workspaces, chats, automations,
  settings, builds and installs), and that deleting, accounts and anything that leaves this Mac ask you first, except
  in Bypass, where only a short always-ask list still asks. It no longer says every call is logged, since no view
  shows such a log. Agents can no longer move a brief's older entries into an archive: Brief → Section options →
  **Archive older entries…** is the one way, and it is yours.
- **Where:** Settings → Connectors → Built into Hollow (Hollow app); a workspace's Brief → Section options (⋯).
- **Undo / turn off:** the switch turns the Hollow app tools off for agents.

### The Main agent keeps answering beside your page
- **What:** Open beside keeps the selected conversation's live answer and Stop button. Hollow's wakes are expandable notes, and quiet check-ins use plain words. The drawer has its own permission mode; changing it starts a fresh native session with the retained conversation context.
- **Where:** Main agent → Open beside your page. After its answer, Continue here brings back the composer when no work is pending. Open the Main agent's space returns to its full controls and pending messages.
- **Undo:** close the drawer or reopen the Main agent's space.

### A reply can't look verified when its checks are stale or missing
- **What:** when a reply says its checks pass but Hollow saw no check after its last edit (or its last check
  failed), the line under the reply now says so, for example "Says it passes · no check after its last edit".
  Checks that ran before the last change ("Checks out of date")
  and cites to lines or files that don't exist or can't be found ("2 cites don't resolve") show there too. "Not verified" shows only when the reply claimed it was done, fixed, or that its checks pass, and nothing confirmed it. Click it to open the
  reply's Run record. A verified reply shows nothing new.
- **Where:** under an agent's reply in any conversation, beside copy and the changed-file chips.
- **Undo / turn off:** nothing to turn off; the Run record still opens from More → Show run details.

### Settings sets the permission mode for new chats; a chat's own menu changes that chat
- **What:** Settings → Agents & models now sets the **Permission mode for new chats**: a new chat starts in it, and
  changing it never touches the chat you're in (that chat's mode is in its own menu). **Revert to Edits only** under
  the danger warning acts on the chat on screen; while its turn runs it is off and says "Stop or finish the running
  turn first", and when it applies it says so. Reasoning effort saves the moment you pick it, from Settings or the
  composer's Effort list, and a later Save in Settings no longer writes an older effort back.
- **Where:** Settings → Agents & models (Permissions, Reasoning effort); the composer's model menu (Effort).
- **Undo / turn off:** pick the earlier mode or effort again.

### One-click deletes and runs ask first
- **What:** Clear all notifications, Reset all shortcuts, Remove key (Jev) and Stop on a chat the Main agent started
  each ask first, and Cancel changes nothing; Clear all is off when the bell is empty. Delete… on a workspace says
  why it is off while its worktree is still there or a conversation in it is running, and a refused delete keeps you
  in the workspace. Add to Notion remembers what it sent: picking a page this canvas already went to says when, and
  the confirm names it. On Automations, a row's ⚡ is named "Run <name> now" and, like the row menu's Run now, asks
  first, naming the agent and where it works.
- **Where:** Settings → Interface (Clear all), Keyboard shortcuts, Accounts (Jev); the Main agent page's Chats I
  started; a workspace's right-click menu; a canvas's Add to Notion page…; Automations.
- **Undo / turn off:** Cancel in each confirm. The review's own Run now still runs at once.

### Main agent model choices explain what can run
- **What:** other providers are disabled with a reason in the Main agent's model menu. Fresh Main agent threads start on Claude; the drawer names its own provider and model. Slash commands refuse unsupported switches before changing the conversation.
- **Where:** Main agent → model menu, or Open beside your page → drawer header.
- **Undo:** no saved conversation provider is migrated. Choose a different Claude model in the menu; other conversations keep their full provider choices.

### Review's keys act only on what you are looking at
- **What:** with the Main agent open beside Review, the keys you press in its question card (1–9, Enter, x, a)
  answer the card and no longer also filter, open or reject rows in Incoming, or move the selection in the other
  Review views. Rejecting one suggestion (r on its card, or x on its row in Incoming) shows a notice with **Undo**,
  which brings the same item back (not for a budget pause's Keep paused). Incoming no longer accepts a Result it never shows you: Enter opens the Result, and you accept it
  there. The bulk learnings bar shows only in All waiting and Decisions, and **Accept N** names any selected
  learnings that aren't on screen before it saves them.
- **Where:** Review (All waiting, Decisions, Incoming); the Main agent drawer (⌥-click the orb, or the Main agent
  page's Open beside your page).
- **Undo / turn off:** Undo on the notice after a Reject; Cancel in the bulk Accept confirm saves nothing.

### A failed view has a way back
- **What:** a page or Settings section that cannot render names the failure and offers Retry or Reload page, plus Copy details. Read failures retain neighboring blocks and have local retries. Memory and History refresh without hiding Settings.
- **Where:** the affected page or Settings section; Your work → Running for run-list and spend retries.
- **Undo:** retrying a read changes no saved data. Leaving and reopening a failed page also retries it.

### Stop a running automation and review every night item
- **What:** Automations recognizes runs that started before you opened the page. Stop run is available in the run review and editor. Last night includes code, chat and runbook results, with recorded costs, branches and conversation links when available.
- **Where:** Automations → an automation → Stop run; Automations → Last night.
- **Undo:** Stop also cancels queued follow-ups. A stopped run can be started again with Run now. Reading Last night changes nothing.

### No paid read starts just because you opened a page
- **What:** with **Keep learning** off, opening Settings → About you no longer reads what your agents know on its
  own (it did every few days, while the screen said "Only when you ask"). With it on, a read that starts when you
  open the page says which agent is reading ("Reading now with Claude"). Claude history in Chat stops re-listing
  your sessions and titling new ones once you switch to Hollow conversations, hide the conversations list or leave
  the page; it titles only the top 40 rows it shows, and catches up when you come back.
- **Where:** Settings → About you (Keep learning, Learn about me / Look again); Chat → Claude history.
- **Undo / turn off:** turn Keep learning on to read again every few days; Look again reads now.

### A file dropped on the message box is attached or refused in words
- **What:** dropping a PDF, a log or any other file that isn't an image on a chat's message box now says "Only images
  attach to a message. Add files in Sources." with an **Add to Sources…** button that opens the file picker (a
  dropped file carries no path Hollow can copy). Before, nothing happened and nothing said why. While you drag, the
  box reads "Images only here" when it can tell none of the files is an image. Images attach as before.
- **Where:** any workspace chat's message box. In a quick chat or on the Chat page the notice has no button (no
  workspace Sources to add to).
- **Undo / turn off:** nothing to turn off. Cancel the picker to add nothing.

### Tidying Recent notifications never clears a failure you haven't seen
- **What:** the buttons under Recent notifications are **Mark these read** and **Clear these**, and they act only on the
  notifications the list shows. Failures waiting in Review → Failures stay unread and stay listed. **Clear these**
  asks first, with the count ("Remove 12 notifications? Failures waiting in Review stay."). When the notification
  file fills up, Hollow drops read and ordinary notifications first and keeps unread failures; if even those pile
  past 1,000, the oldest fold into one "N older failures were trimmed" failure. When more than 200 are kept, the
  list says "Newest 200 of N".
- **Where:** Review → Recent notifications.
- **Undo / turn off:** cleared notifications don't come back; Cancel in the confirm clears nothing.

### "calendar" finds the Agenda
- **What:** typing "calendar" (or "agenda", "schedule", "meetings") in the command palette offers **Open Agenda
  (Tasks)**. An old link or saved page that pointed at the calendar opens Tasks → Agenda instead of a Library folder.
  Agents open the same pages the `g` keys do (Agenda, Running, Commitments, Runbooks, Weekly review, Usage and the
  rest), are told the page that opened, and get "retired; use agenda" for pages Hollow no longer has.
- **Where:** the command palette (the top bar's Search anything button, or ⌘K); Tasks → Agenda.
- **Undo / turn off:** nothing to turn off.

### What's new names the build you are running
- **What:** What's new's header says which build is running ("This build: v0.5.15 · e928b5bd"), and release intros
  no longer claim a release isn't installed (five did, including releases the 10-19 install had). The Agents only
  list says which of its actions now have a button, and the disk-space warning points to Settings → System → Disk
  space instead of asking an agent.
- **Where:** Settings → System → Help → What's new (or the palette's What's new).
- **Undo / turn off:** nothing to turn off.

### New running work gets a fresh restart choice
- **What:** saving a Notion connection checks running work again before restarting. If another run starts while the confirmation is open, Hollow names it and asks again. A failed check keeps your saved connection and leaves Hollow running.
- **Where:** Settings → Connectors → Configure Notion connection → Save connection & restart.
- **Undo:** choose Restart later or Cancel in the second choice; the connection remains saved for your next restart.

### Automation edits stay yours
- **What:** editing an automation's name keeps its chosen schedule. Leaving an unsaved editor asks whether to keep editing or discard; a failed first list read offers Retry.
- **Where:** Automations → New automation, or Automation options → Edit. Right-click → Edit opens the same editor.
- **Undo:** choose Keep editing to preserve the draft, or Discard to return to the saved automation.

### Every Review view lists all of its items
- **What:** Decisions, Results to review and Failures each read their own items, so older failures are no longer
  hidden behind a page of newer approvals, and every tab's count is what that view lists (drafts count under
  Decisions and All). An empty view says which one is empty ("No failures waiting."). Following a page's review
  link (Tasks' "Open in Review") filters All only: another tab, or Review in the sidebar, shows everything again.
- **Where:** Review (the sidebar, or the bell).
- **Undo / turn off:** nothing to turn off.

### ⌘K then Enter never stops or starts agent work
- **What:** the command palette opens with a row that only shows, opens or goes somewhere selected, so a quick ⌘K
  and Enter no longer stops a running turn. Rows that start or stop agent work, or change files without asking
  first (Stop the running turn, Make a canvas view of the plan, Clear debug logs), no longer join Recent. Every row
  that starts or stops agent work, changes files or sends something outside this Mac says so under its name
  ("Starts or stops agent work, which may cost money"), including the ones that ask before they act.
- **Where:** the command palette (the top bar's Search anything button, or ⌘K), before you type.
- **Undo / turn off:** nothing to turn off; the rows this moved are still in the list, and every row is found by
  typing.

### Replacing your work asks first
- **What:** creating a skill with an existing name offers Replace, Save under a new name, or Cancel. Save as skill waits for unsaved automation edits. Rewriting an edited meeting note asks first and preserves changes made while it runs. Replacing a custom icon explains that its original will be deleted.
- **Where:** Settings → Skills → New skill; Automations → Edit → Save as skill; a meeting's Prep or Recap → Write it again; Settings → Interface → Appearance.
- **Undo:** Cancel keeps the existing work. Skill replacement is outside History; save under a new name to keep both. Open a meeting note to keep a copy before replacing edits.

### Installing Hollow always asks you first
- **What:** an agent can no longer install a new build of Hollow without your say, even in a **Bypass** chat. Its
  install card shows in the chat, and nothing installs until you press Allow. Automations and check-ins still
  can't install at all. Bypass's description in Settings now lists six actions that always ask.
- **Where:** the install card in the chat that asked; Settings → Agents → permission mode descriptions.
- **Undo / turn off:** nothing to turn off. The card times out after about two minutes, so answer it at your desk.

### Merge, install and deploy suggestions need a second Enter
- **What:** when a reply leaves a pull request open, the suggestion in the empty message box (Merge, Merge and install,
  Deploy, or the agent's own "Merge and install" line) is no longer sent by one Enter. The first Enter, or a click on
  the button (now named **Place suggestion**), puts the words in the box; a second Enter sends them. Merge and install
  is offered only in a Hollow build workspace, and `npm install` in a reply no longer counts as a plan to install.
- **Where:** any chat's message box after a reply; the hint under it reads "↵ place to check · Tab edit · Esc dismiss".
- **Undo / turn off:** nothing to turn off. Other suggestions still send on one Enter.

### Changes stays current while other conversations work
- **What it does:** Commit, Push, Open PR and Install show why they must wait for running work. Each checks again before asking you to confirm. Changes refreshes when work finishes and every 15 seconds while visible, with a Read time beside Refresh.
- **Where:** Build workspace → Changes. Existing actions and draft fields stay in place.
- **Undo:** Reading changes nothing; cancel any action's confirmation to leave the repository alone.

### A source Hollow couldn't read says so
- **What:** when Today or Weekly review can't read the calendar (for example when the Microsoft 365 sign-in has
  expired), commitments, last night or what waits on you, the card says "Couldn't read …" with the reason and a
  **Retry** button, instead of "No meetings today" or "Nothing waits on you". An expired sign-in also offers
  **Open Connectors**. Today's brief is told the calendar is unknown, so it no longer describes a free day. While
  Today is still gathering, it names what it waits for, with **Retry**, and the rewrite buttons wait too.
- **Where:** Your work → Today; Weekly review → Next week.
- **Undo / turn off:** nothing to turn off.

### Quit without losing track of running work
- **What:** Quit asks before stopping runs. An approved install shows what it is waiting for and can be cancelled. An interrupted-chat offer survives another restart while still eligible.
- **Where:** Quit in the app or menu bar; Cancel install in the banner or Changes → Hollow app. Notion's connection confirmation offers Save now, restart later.
- **Undo:** choose Cancel or Keep running in the menu bar to leave work running. Cancel install keeps the finished build available.

### Today keeps the right day
- **What:** Today counts a calendar day, so on Sunday 1 November (a 25-hour day when the clocks go back) a meeting at
  or after 11 pm still shows on Today and in its brief. Left open past midnight, Today's date and the Agenda's
  "today" and Overdue move to the new day by themselves. Weekly review lists a multi-day all-day event under each day
  of next week it covers, and the Agenda's Week view is titled with its dates ("Oct 22 – 28").
- **Where:** Your work → Today; Tasks → Agenda; Weekly review → Next week.
- **Undo / turn off:** nothing to turn off.

# What's new: 2026-10-21 (afternoon)

Full details are in the [October 18 to 21 archive](history/WHATS_NEW-2026-10-18-to-21.md).

### Four Hollow app icons
Cut Glass is the new stock icon.

### Steer a chat while it works
You can type in a chat while its agent is working.

# What's new: 2026-10-21

Full details are in the [October 18 to 21 archive](history/WHATS_NEW-2026-10-18-to-21.md).

### Pin workspaces to the top of the sidebar
Like pinned chats in the ChatGPT app, a pinned workspace sits in a **Pinned** group at the top of the sidebar, above Workspaces, and is listed only there.

### Your own app and Main agent icons
Upload a PNG, JPEG, static SVG or modern ICNS, or make an icon with a shape, colours, Hollow mark, emoji, letters or symbol.

### Ask ⌄ leaves the top bar
The small **Ask ⌄** beside the Main agent's orb is gone, because each place it offered has its own button: a workspace's own message box, the orb for the Main agent, and **New → New quick chat**.

### Ten chats at once
Up to 10 chats run at once (5 in one workspace), up from 3.

### Each chat gets credit for its own files
When several chats work in one workspace folder at once, each chat's run record and Results credit only the files its own edits named.

### Chats in one workspace keep each other's brief edits
When two conversations run in one workspace and one writes WORKSPACE.md, PLAN.md or SOURCES.md back from an older copy, the other's lost edit isn't gone: when the run ends, that version is kept beside the file as a conflict copy for you to pick from (Hollow doesn't rewrite the file, in case the removal was on purpose).

### Clearer view controls
Destination tabs keep an underline; task scopes, result filters and Running layouts share quiet pills.

### Background windows on a clock
A 24-hour dial shows configured windows in their own time zone, including overnight tails.

### A canvas view that fails keeps your Summary choice
If Make a canvas view brings no view (the request is withdrawn, the reply has no canvas, or the send never starts), the section goes back to Summary where you had chosen it.

### Settings at a glance
Three compact labels show what a section affects, when it takes effect and how it saves.

### A calmer task list
Scope chips and compact rows give tasks more room.

### One automation run strip
Run now, status and schedule share a compact strip.

### A quiet place to start
Small drawings accompany the empty Tasks list and Automations detail.

### Library in pictures
Results uses thumbnail cards with workspace chips.

### Archive older entries
Move older brief entries and finished plan sections out of WORKSPACE.md and PLAN.md, word for word, so agents read less.

### Your work at a glance
Four tiles show Waiting, Due, Last night and Running now.

### One title per screen
Less repeated text above Incoming, Running, Canvases, Commitments, Runbooks and the Map.

### Create in Notion
A new task can start in your Notion Tasks database, with a linked copy here that two-way sync keeps in step.

### New project in Notion
A new project can start in your Notion Projects database, arriving here with its linked copy and its workspace, as a project made in Notion does after a sync.

### A failed install comparison says so
When "What this install brings" couldn't be compared (for example the installed base commit is unavailable), the panel shows that reason instead of "0 pull requests, 0 direct commits".

### Install questions read the right checkout
When you ask an agent whether `origin/main` is ready to install, or what the next install would bring, it now reads the Hollow checkout the installed app was built from (the last install's build workspace) instead of whichever known checkout sorted first, and names it.

### Integrity check counts files it couldn't open
After Check integrity, the summary says how many files couldn't be opened ("1 couldn't be opened."), and no longer says "Every file checked ok." when any couldn't, including when none could.

# What's new: 2026-10-19

Full details are in the [October 18 to 21 archive](history/WHATS_NEW-2026-10-18-to-21.md).

### Files and Visuals on a shelf
Picture tiles replace the panel dropdowns.

### Sources in a row
Sources show one compact row each, with size, coverage and freshness.

### Brief, headline first
The outcome leads, followed by a short newest update.

### History by day
A two-week strip and status rail show recent activity with less repeated text.

### Quieter Plan rows
Checkbox and text lead each row.

### Compact workspace notices
Brief freshness, unsaved drafts and plan-item changes use short chips.

### Make the missing canvas views together
One request makes the missing Overview, Brief, Plan and Sources views, with stats, progress and short labels.

### Overview at a glance
A short outcome, four summary tiles, one plan-progress bar, three next steps and four recent results.

### Results in pictures
Image thumbnails, video frames and tiny canvas sketches replace word-heavy cards.

### Panel summaries at a glance
Overview, Plan and Sources show a number or progress ring and one short label per tile.

### More room for panel canvases
Canvas views and expanded Visuals replace the repeated heading with a small permission lock and ⋯.

### Canvas, visual and text together
One slim toolbar stays at the top of each workspace section.

### One clear workspace panel header
One Workspace tab and one line of section chips replace the repeated section menu.

### The tool-folder list no longer waits on a stalled network folder
A folder on a network share that stops answering used to hold the list open on "Reading the folders…".

### A broken PDF in Sources can't keep stopping Hollow
Hollow reads the PDFs and Office files in a workspace's `sources/` folder on every message.

### Skill descriptions without a stray "> "
HyperFrames, humanizer and seven other skills whose description spans several lines no longer start with "> " or "| ".

### A busy Codex retries once instead of stopping
When OpenAI says "Selected model is at capacity" (or another outage stops a Codex, Grok or Antigravity turn), Hollow waits a minute or two and resumes the same conversation once.

### Codex chats show how full their context is
A Codex chat's usage ring now fills like a Claude one, for example "231k/258.4k", and offers **Continue in a new conversation** past 75%.

### Codex spend, estimated at the price you enter
Codex reports tokens but no price, so its runs showed no cost.

### Canvas views open as you left them everywhere
A Plan, Brief, Overview or Sources canvas view opened from Results or the Library's Canvases now restores the tabs, sorting and open sections you left in the panel.

### The Main agent starts fresh after an hour away
When a Main agent chat already holds 350K or more and you come back after more than an hour, Hollow starts a fresh Claude session with the recent conversation and its task state.

### "Tests pass" with nothing to back it is flagged
When an agent's last reply says its tests or checks pass but it ran no check after its last edit (or its last check failed), the run record's verification line gets a chip saying so.

# What's new: 2026-10-18

Full details are in the [October 18 to 21 archive](history/WHATS_NEW-2026-10-18-to-21.md).

### Agents find project files by what they say
Once you switch it on, Hollow keeps a word index of the text in your connected project files, PDFs and Office files included, so an agent's evidence comes from the files whose contents match the request, not only from file names and the newest files.

### Hollow reads the PDFs and Office files in your projects
PDFs, Word, PowerPoint and Excel files in your connected projects and in a workspace's `sources/` now open to their text, and agents get passages from them with line numbers they can cite, the same for Claude, Codex and Grok because Hollow does the reading on this Mac.

### Recap a meeting after it ends
Write a timecoded recap from a linked Plaud recording and add checked proposals to Review.

### See Hollow's databases, and remove other vaults' old indexes
A new **Hollow's databases** list shows the SQLite files Hollow keeps on this Mac: your conversations, the search and meaning-search indexes and the companion ledger, each with whose it is (this vault, another vault, or Hollow), its size, its WAL and its version.

### Export with a workspace or custom canvas look
PNG, PDF and flow SVG can use App theme, This workspace's look, or a custom theme on this Mac.

### Agents hear how full their context is when a message starts
When a long agent chat's last turn ended past 300K, 350K, 400K or another 100K step, and the agent wasn't told that step yet, your next message reaches it with one short line in front saying how full its context is and asking it to finish its step and hand over on a long job.

### Present a Result or release as slides
Read a saved text Result or a What's new release one section at a time, in the same full-screen player as the Welcome tour.

### The Run record checks the lines a reply cites
When a conversation turn or an automation run ends, Hollow checks every `path:line` its final reply names: whether the file exists, whether the line is in it, and whether the run actually read that file and line.

### See what a finished build brings before installing
The install confirmation names the build's pull requests, notes about checks not done by hand, and any commits it drops from the installed base recorded when it finished.

### Agents search a conversation for what was said
An agent that needs a figure or a decision from earlier in a long chat (its own or another one) can search that conversation instead of rereading it: it gets up to 12 short snippets, each with its turn number and, for a tool result, the tool's id, then reads just those turns in full.

### Give one workspace its own canvas look
Canvases can use a custom theme's palette while the rest of Hollow keeps the app theme.

### The usage ring says what fills the context, and offers a fresh start
The usage ring's Context section gains two collapsed sections.

### Check a flow layout and draft a fix
A current reply canvas lists possible crossing links, overlapping or shortened labels, and connections kept in its text list.

### Save a flow diagram as SVG
Save a static, scalable flow drawing with its labels and chosen colours.

### Agents are told how full their context is as a long turn grows
A long-running agent turn used to hand over at a median of 615K tokens, well past the 350K point where each call gets slower and dearer, because nothing told it the figure.

### Mermaid flowcharts draw in replies and files
A Mermaid flowchart becomes a Hollow diagram, with a line saying what the conversion kept or left out.

### Agents ask a long saved log for what they need
A Claude or Codex agent can save a long command output (a failing build, a test run) or a big export into its run's scratch folder and ask Hollow's file reader for just the part it needs, such as the error or the total: it gets the matching lines and the last 40 lines instead of the whole file, which costs a few hundred bytes instead of tens of KB.

### Save a canvas as image or PDF
Save a canvas as PNG or PDF in your workspace's Results.

### Agents are reminded of your rules after they compact
When an agent summarizes its own history to make room (a compaction), the summary can drop your "always/never" rules and the steps it already finished.

### Search indexes no longer keep a big leftover file
Hollow's keyword and meaning-search indexes each keep a side file of recent changes (`-wal`), which never shrank: on this Mac it had reached 90 MB beside the keyword index and 30 MB beside the meaning one.

# What's new: 2026-10-17 (evening)

Full details are in the [October 13 to 17 archive](history/WHATS_NEW-2026-10-13-to-17.md).

### Videos play in the side panel and in Results
An mp4, m4v or mov an agent makes (or any you open) now plays inside Hollow, with the usual controls: play, scrub, volume and fullscreen.

### Videos play right under the reply that made them
When an agent makes or names a video (an mp4, m4v or mov, rendered by any tool, even through a script), it now shows under its reply as a player, with Open in QuickTime Player, Reveal in Finder and Open in side panel.

### Video Results show a frame of the video
A video in a workspace's Results now shows a still from the video on its card instead of a plain icon, and a video under a reply that hasn't loaded yet shows one behind its Play button.

# What's new: 2026-10-17

Full details are in the [October 13 to 17 archive](history/WHATS_NEW-2026-10-13-to-17.md).

### An agent no longer waits two minutes on a card you can't see
When Hollow's window is closed to the tray and an agent in a chat asks, through Hollow's app tools, to do something that needs your Allow, it's told at once that nobody can see the card and to ask you in its reply or a suggestion instead.

### Context in one place
The composer's usage ring shows recorded message shares, one Last turn heading and an explained Cache section. Folder and connectors keeps two resource lines with a Settings link. Project notes remain in the side panel's Files picker.

### A chat that started other chats can be woken when they finish
Before, only the Main agent was woken when a chat it started finished; any other chat had to keep its turn open and check again and again.

### A compaction window for Claude and Codex (off; no switch yet)
A new agent setting can make Claude and Codex compact their conversation at 300K, 400K or 600K tokens instead of their own default.

### Build slots in every chat that can run commands
Every chat turn that can run commands (any mode but Plan; Antigravity only in Bypass all) now gets Hollow's `hollow-slot` tool and this Mac's build limits (8 Rust jobs, 4 test workers), not only build workspaces.

# What's new: 2026-10-16

Full details are in the [October 13 to 17 archive](history/WHATS_NEW-2026-10-13-to-17.md).

### Agent updates wait for a tool you started in Terminal
Hollow doesn't update an agent's command-line tool while something is using it.

### Make a canvas view of a workspace section
A workspace's Overview, Brief, Plan and Sources sections, when they have no canvas view yet, show **Make a canvas view**.

### Automations say when a schedule can't be read
An automation whose schedule was edited by hand (or by an agent) into something Hollow can't read never runs on schedule.

### Undo last batch stays after the snackbar
After you accept or reject several learnings at once, **Undo last batch** (for example "Undo last batch (2 learnings rejected)") stays above the list once the 10-second Undo has gone, so a mistaken batch can still be put back.

### Companion backend: says when it can't be reached
When the companion backend stops answering, Settings now says **Can't reach the backend** instead of "Waiting for the backend", and explains in a plain sentence: Hollow couldn't reach the address, so check it, that the backend is running and that this Mac can reach it.

### See the folders agents can run tools from
A list of every folder agents look in for command-line tools, in order, each labelled with where it came from: Hollow's own folders, your shell's PATH (what `~/.zshrc` adds), the PATH Hollow was started with, or the system folders.

### Remove finished worktrees keeps a build waiting to install
**Remove finished worktrees…** no longer removes the worktree of a build that has finished and is waiting to install, so **Install and restart** no longer fails with "The worktree is gone".

# What's new: 2026-10-15 (evening)

Full details are in the [October 13 to 17 archive](history/WHATS_NEW-2026-10-13-to-17.md).

### Agent updates wait for bunx and bun run too
Hollow doesn't update an agent's command-line tool while something is using it.

### Changed files: clearer names, Results from run details
When two files a reply changed have the same name, each now shows its folder, on the file and in the reply's ⋯ menu.

### Companion fixes: Discard always answers, removing a backend forgets it
**Discard it** on an "Old change from a device" card now always tells the device the change was dropped, even if the card's text was changed after Hollow made it.

# What's new: 2026-10-15

Full details are in the [October 13 to 17 archive](history/WHATS_NEW-2026-10-13-to-17.md).

### Question cards and canvas Send: no answers by accident
Enter in a question card's typed answer (or its "Other…" box) now waits half a second after the card shows, like the choosing keys already did, so a key meant for the message box can't send your answers.

### A changed file in outputs/ opens in Results
In a workspace conversation, clicking a file the reply changed in the workspace's `outputs/` folder now opens it in the workspace's Results, where you can **Accept** it or **Ask for a revision**.

### Open never launches an app hiding behind a document's name
Open looks at what a file really is before opening it, following any link.

### Settings show an agent's change right away
When an agent changes Learning suggestions, the extraction model or Retitle new chats for you (on its card, when your permission mode asks), Settings shows the new value at once.

### A signed-out calendar says how to fix it
When Microsoft 365 has ended Hollow's sign-in, Today and the Agenda now say "Microsoft 365 needs you to sign in again" instead of "Calendar unavailable", with an **Open Connectors** button that takes you to where you sign in.

### Old changes from your devices ask first
A change a companion device sent more than 14 days before Hollow sees it (a phone that was offline for weeks, say) is no longer made on its own.

### Free disk space from Settings
Settings → System → Disk space now has two buttons that free space, instead of only naming the cleanups for an agent.

# What's new: 2026-10-14

Full details are in the [October 13 to 17 archive](history/WHATS_NEW-2026-10-13-to-17.md).

### Questions one at a time over the message box, and an ask-canvas's Send sends
When an agent asks you questions, the card now takes the message box's place and asks one at a time ("1 of 3").

### Test the companion backend connection
**Test connection** says in one sentence whether the backend for the web app answers and takes your token: connected (with how many devices and tasks it has), token refused, another Hollow is connected, or the address can't be reached.

### Plainer words: "Run record" instead of "receipt"
The record of what a run changed and did is now called its **run record** (the chat's "Evidence receipt" is now "Run record").

### What's new opens on the newest releases
What's new now opens on its newest releases, so it opens faster; **Show earlier releases** (or a title under Earlier in the contents column) adds the rest, and typing in the filter searches everything.
