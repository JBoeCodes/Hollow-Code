# Keyboard shortcuts

Part of the agent manual. [AGENTS.md](../../AGENTS.md) is the entry point and
links every reference doc; this one holds CLAUDE.md's "Keyboard shortcuts",
moved there word for word on 2026-10-13 (only relative links were re-based),
and the local-command keyboard policy (2026-10-15, from the next-level ledger).
Read it before adding or changing a shortcut: the shortcut table, the overlay
and the page listeners must stay in step. Keep it current with the code in the
same change, as AGENTS.md asks.

## Keyboard shortcuts

Library Results owns Escape on a focused result row while a canvas detail is
open outside the Canvases filter. It matches Back to all results and restores
row focus. The shortcut card lists it under Lists, including Settings →
Keyboard shortcuts → Show all keys. It registers no global chord.

Browser → Annotate's focused stage uses arrows (8 snapshot pixels),
Shift+arrows (40), Enter (place Comment or each Box corner) and Escape
(cancel a pending box, otherwise close). A numbered entry uses Enter to edit,
Delete/Backspace to remove; its note uses Enter/Done or Escape to return.
These are local editing keys, never registered system-wide. Comment on the
whole page, tool selection and each mark's Remove button are visible routes.

The New workspace form owns Command-Enter (Create workspace) and Escape
(Cancel, when idle). The shortcut card lists both under New workspace, as
form keys in `pageKeys` rather than configurable app chords. Both have visible buttons and stay inside the form;
neither registers a global shortcut or overlaps a macOS system command.

Every shortcut below marked as a command is a **default**: Settings →
Keyboard shortcuts (2026-10-01) changes a command's key, removes it or resets
it, and one switch, **Use keyboard shortcuts**, turns them all off. Off means
no command key in the window, no menu-bar accelerator, no system-wide quick
capture / ingest key, no `?` card, no Esc-to-stop, and no single-letter list
keys (`j`/`k`, a review card's `e`/`a`/`r`/`z`, Recent notifications'
`x`/`r`, the Agenda's `t`, which followed the switch only from overnight F4). The arrow keys, Enter, Escape closing things, the chat's
send keys, `/` and editor keys (⌘S, ⌘Z) are not shortcuts and always work.

**Lazy confirmation recovery (overnight S1).** `components/ConfirmFallback.tsx` supplies
an immediate cancel-only dialog while the full chunk loads; it traps keys and focus so
shortcuts cannot reach the page beneath. If loading/rendering fails, its reactive error
fallback resolves every current and later request as cancelled (prompt/checklist: null).
Nothing is accepted by a fallback. The failure message asks the person to reload. This
retains the lazy boundary and doesn't add the full dialog to the launch chunk.

**Dialog focus (overnight B8).** `ConfirmDialog` keeps its own fixed-order
Tab trap: a body taller than its box joins it (`tabIndex=0`), the body is the
dialog's `aria-describedby`, and focus goes back to what had it on close.
`focusCancel` starts on Cancel without the red (the resume-chats confirm:
each resume is a billed message). Hand-made dialogs (conflict copies, night
dry run, workspace map) use `holdFocus(card)` from `src/focusHold.ts` (a Tab
ring plus focus return) and label their `esc` button "Close … (esc)". The
palette input is a `combobox` whose `aria-activedescendant` names the
selected `option`; dismissing the palette (Esc, the backdrop) gives focus back
to what had it when it opened (`src/focusReturn.ts`, H12), while running a row
leaves focus where the row puts it. Nested overlays close one layer per Esc:
the palette over the Main agent page, a confirm over Settings
(`tests/smoke/keyboard-only.spec.ts`). An Undo snackbar holds its countdown while it has focus
or the pointer (`holdSnackbar`/`releaseSnackbar`, at least 2 s more after).
Vitest has no DOM here, so these are pinned in `src/a11yPass.test.ts` (palette focus return: `src/focusReturn.test.ts`).

**macOS's own shortcuts are never taken** (five asks A4, `src/macReserved.ts`):
`MAC_RESERVED` lists ⌘Q W H M Tab Space \` , C V X Z ⇧Z A S F G ⇧G P N O T R,
⌘⌥H/M/Esc/D, ⌘⇧3/4/5, ⌘⇧/, ⌘-arrows (caret moves), ⌘F1–F12, and (P1-49)
Accessibility Zoom's ⌥⌘8 ⌥⌘= ⌥⌘- ⌥⌘\, ⌘. Cancel and ⇧⌘6 (Touch Bar
screenshot). A system-wide command's chord must hold all of ⌥⇧⌘
(`systemWideProblem` in `settings/shortcutRules.ts`; Rust `validate`, and `drop_reserved`
drops a shorter one from an older file), and no command may take selected
text's ⌥⌘T or ⌥⌘A (`localAltProblem`; Rust `LOCAL_ALT`). A chord may
be reserved only for its macOS meaning (`STANDARD_MEANING`: ⌘, Settings, ⌘N
New conversation; local ⌘S Save and ⌘F Find likewise). `macReserved.test.ts`
fails on any default (app, menu, system-wide) that breaks this; Settings
refuses to record one (`reservedReason`), and Rust `validate` refuses one from
an agent too (`shortcuts.rs` `MAC_RESERVED`/`STANDARD_MEANING`/`reserved`, any
F-key by rule; a vitest keeps the lists equal; `load` drops such a change from
an older file instead of resetting everything). Chords are always ⌘(+⌥/⇧)+key,
shown in macOS's order, ⌃⌥⇧⌘ then the key (`displayChord`: ⇧⌘N), and a
⌥ chord runs in the window (`windowCommandFor`) only for a command with no
menu item (`noMenuItem`: Jump, Workspace conversation, the palette's second
key; an item's own key reaches the menu) and never on an in-surface key's
chord (`LOCAL_ALT_CHORDS`, a launch-bundle copy a vitest holds to
`LOCAL_COMMANDS`). Behind a dialog a menu item flashes "Close the dialog first"
(`appMenu.ts`); Stop Running Turn, Back and Forward grey out when they can't
act (`menuState.ts` → `set_menu_enabled`). The tray hints only system-wide keys
that registered (`tray_hint`); keys that failed at launch post one bell item
(`boot_notice`). `aria-keyshortcuts` never holds a `g` sequence (a vitest).
So ⌥-letter/⌥-digit, ⌃⌥ (Magnet) and ⌃-arrow (Mission Control) can't be
written, and on a Mac ⌃ never stands in for ⌘ (`hasChordModifier`: ⌃K, ⌃A, ⌃E
keep their Cocoa text meanings). The leader ignores IME composition and dead
keys, reads Caps Lock's capital and other layouts' physical keys, treats type-
ahead lists/menus as typing, passes ⌘ chords through, and ends on a pointer
press, a focus move or window blur.

Global (work from any app):

| Shortcut | Action |
|---|---|
| `Cmd+Alt+Shift+T` | Quick capture: brings Hollow forward on the Add sheet set to a task; Enter adds it, and Hollow steps back to where you were (was ⌘⇧T: taken from every browser's Reopen closed tab, five asks A4; its own overlay window until S5, 2026-11-03; command id `quick-capture`) |
| `Cmd+Alt+Shift+I` | Import a source: brings Hollow forward on the Add sheet set to a Library source; Enter imports a single link, and Hollow steps back to where you were (was ⌘⇧I; its own overlay window until S5, 2026-11-03; command id `quick-ingest`) |

In-app (wired in [`App.tsx`](../../src/App.tsx), [`navigation.ts`](../../src/navigation.ts),
and [`shortcuts.ts`](../../src/shortcuts.ts)):

| Shortcut | Action |
|---|---|
| `Cmd+1`–`Cmd+4` | Projects (Work until L1, 11-12), Tasks, Library, Automations (rail order); other digits unbound |
| `Cmd+,` | Settings |
| `Cmd+B` / `Cmd+\` | Collapse the sidebar / hide the Preview pane, or reopen its last item (command id `toggle-side-pane`, kept so custom bindings still work; run 15 P1) |
| `Cmd+L` | Focus the Preview pane's web address field |
| `Cmd+Shift+D` | Dictate into the focused field, else the chat composer on screen (overnight 10-11 B7, command `dictate`, Edit menu): a tap starts and a second tap stops; held over 0.35 s, releasing the key or a modifier stops (`store/dictation.ts` keyup; repeats ignored); Esc in the field stops (not the running turn). Works in dialogs too. Not a macOS chord |
| `Cmd+Shift+M` | Open the Main agent page; already open, focus its box (command `main-drawer`, an id kept so custom bindings still work; ⌘M is Minimize, ⌘⇧M no macOS chord). Until run 15 P3 (2026-10-27) it toggled the Main agent drawer beside the page, removed with the page's **Open beside your page** and ⌥-click on the orb. Click route: the Main agent orb, or View → Main Agent |
| `Option+Enter` | In Work's Ask field with **Across all my work**: open the Main agent page and put the text in its box without sending (P1-11, D2; plain Enter sends it there, Shift+Enter starts a line). Not a macOS chord. Click route: hold ⌥ and click **Put it in the box** |
| `Cmd+N` / `Cmd+Shift+N` | New conversation in the open workspace (a chat in Chats with none open) / new note |
| `Cmd+K`, `Cmd+Shift+P` | Command palette (also finds conversations and the open conversation's actions) |
| In the open palette (t3code parity 4.1, `paletteNav.ts`) | `>` first in the box is the **Actions** chip (actions, pages and conversation actions; the chip is its click route). Backspace in an empty box goes back one level (chip to All, or New conversation… to the root); the footer's **Back** button does the same; Esc closes at every depth. `Cmd+1` to `Cmd+9` choose the row that digit labels (shown at the row's right end), a local key read only while the palette is open: the page digits ⌘1 to ⌘5 keep their meaning once it is closed, and the native menu's page items answer the same way while it is open. No new chord is registered |
| `Cmd+Shift+C` | Return to the current workspace conversation |
| `Cmd+Shift+K` | Search chats: the top bar's search icon (command `search-chats`, L1 11-12; no menu item). Shows a hidden sidebar for the search; Escape returns focus to where it was. No macOS chord |
| `Cmd+J` | Jump to a workspace or chat: the palette on its Workspaces chip (`openCommandPalette('jump')` → `paletteSearchRequest`; five asks A4) |
| `j` / `k` (↓ / ↑ inside a list), `.`, `r`, `e` (not typing) | List keys in lists marked `data-key-list` (five asks A4, `listKeys.ts` + `store/listKeys.ts` on the document, after the leader and before pages' own window listeners; skipped when a page handler already prevented the key or a menu is open): the rail (primary, workspace and conversation rows), Home's workspace cards and Automations (each row's open button); the last two are `data-key-default`, so j / k reach them with focus nowhere (arrows never pull focus in). Focus on a toggle or + keeps its place. `.` sends the row a `contextmenu` event; a keyboard-opened `ContextMenu` focuses its first item (`takeMenuFocus`), ↑/↓/j/k/Home/End walk it, Tab/Esc close it and focus returns to the row; `r` / `e` open it on Rename / Edit-or-Open (`MENU_FOCUS`) unless the row has its own `data-key-action` button. Tasks and Waiting on you keep their own |
| `g` then a key (not typing) | Go-to leader (five asks A4, `keyboardLeader.ts` + `store/leader.ts` in window capture, `goTo.ts`): h Projects, m COO, v Ask anything (the Projects page's Ask field, focused; id `ask-vault`, kept from the removed Ask your vault, One Ask 2026-11-03), t Tasks, l Library, a Automations, o Running, x Canvases, b Needs you, p Conversation history, u Usage, s Settings, q New chat, j Jump (Today, Agenda, Inbox, Runbooks and the workspace map lost their keys when they were cut; a saved binding for one is dropped on load). A which-key card (`WhichKey.tsx`) shows after 350 ms and then waits for the key (no timeout since P1-49); Esc, a click or leaving the window cancels. Settings' Go to rows have Remove. Second keys rebindable (Settings → Keyboard shortcuts → Go to; saved as `ShortcutSettings.leader`, Rust `LEADER_TARGETS`/`leader_key_ok`); `g` is fixed. Off with Use keyboard shortcuts |
| `Cmd+[` / `Cmd+]`, mouse buttons 3/4, two-finger trackpad swipe | Nav history back / forward. The swipe (t3code parity 4.6, `historySwipe.ts`): right goes Back, left goes Forward, once per gesture, only when horizontal clearly beats vertical, never over an editor, a dialog, a shortcut being recorded or a box that can still scroll sideways that way. Settings → Keyboard shortcuts → **Swipe to go back and forward** turns it off (on by default; the buttons and keys work either way) |
| `Cmd+Shift+[` / `Cmd+Shift+]` | Previous / next conversation in rail order (matched by `KeyboardEvent.code`) |
| `Esc` (focus in the conversation) | Stop the running or queued turn, unless something else owns Escape (`src/escStop.ts`) |
| `?` | Keymap card (`ShortcutsOverlay.tsx`): app-wide rows from `COMMANDS`, a "Go to (press g, then…)" section from `GO_TARGETS`, an "In a surface" section from `LOCAL_COMMANDS`, and the unmodified page keys (lists, suggestions, Recent notifications, the workspace map, the Agenda, Ask (the command bar's Ask row), editing a note, Running, the Inbox, Commitments, the weekly review, What's new and the tour, the chat) from `pageKeys`, every section listed (a test fails on a section with no place); every COMMANDS group is rendered (`src/keymapSections.ts`, overnight F4; `keymapSections.test.ts` fails on a row without a label) |
| `Enter` / `Cmd+Enter` (chat input) | Send (Settings → General → "Enter sends a message"); `Shift+Enter` new line |
| `↑` / `↓` (empty chat input) | Prompt history |
| `j` / `k` | Walk lists (Tasks, Waiting on you); `e`/`a`/`r`/`z` on a focused review card |
| `/` (start of chat input) | Slash-command popup |

**One keymap (frontier F6; customizable 2026-10-01).** Every command is a
row in `COMMANDS` (`src/shortcuts.ts`): an id (the native menu item's id where
there is one), a group and a default chord written as the backend and
the menu write it (`CmdOrCtrl+Shift+[`; `?` and `Escape` are the two bare keys
the webview handles itself; `''` is none). Its label comes from its shared
action. App.tsx asks `shortcutFor(event)` for the command and runs
`runMenuAction(id)` (`appMenu.ts`), which runs the shared action through
`runAction` (`actionRun.ts`), so a key, its menu item, its palette row and its
`g` key do the same thing. `keyFor(id)`, `ACTION_SHORTCUT` (getters, for the
palette's chips), `pageShortcut`, the TopBar tooltips and the keymap card all
read the keys **in effect**, so they follow the user's changes and go blank
when shortcuts are off.

**Shared actions (plan completion L4, 2026-10-13).** Everything a key, a `g`
target, a palette row, the top bar's New menu or a menu-bar item runs is one
row of `ACTION_DEFS` (`src/actionDefs.ts`): its effect (`navigate`, `view`,
`draft`, `write`, `run`, `external`, plus `confirm`), its condition with the
words shown when it can't run (Back with nowhere to go says "Nothing to go back
to yet."), its route-ledger row, its ids per surface and any surface's own
words. `docs/ACTIONS.md` is rendered from it. The composer's slash commands are
reachable by a click too: the composer's **Commands** menu
(`components/ComposerCommands.tsx`).

Settings → Keyboard shortcuts stores `{enabled, keys}` in
`{app data}/shortcuts.json`, owned by `src-tauri/src/shortcuts.rs` (the
system-wide keys and menu accelerators must be right before the webview
loads). In-surface keys are `LOCAL_COMMANDS` in `src/localKeys.ts`; they never
take an app or global chord, and a `RESERVED` one only for its macOS meaning
(see [Local-command keyboard policy](#local-command-keyboard-policy)).

To add a shortcut: add its `ACTION_DEFS` row (`key`, and `menu` / `palette` /
`go` where it has them, `route` naming its ledger row) and its runner in
`ACTION_RUN` (`actionRun.ts`), then a `COMMANDS` row; if it belongs in the menu
bar, an `ITEMS` entry in `app_menu.rs` with the same default and the action's
menu words. The palette shows it with no further code. Run
`node scripts/route-ledger.mjs` to refresh `docs/ROUTE_LEDGER.md` and
`docs/ACTIONS.md`. A system-wide one
also needs a `GLOBALS` row in `shortcuts.rs` and a case in lib.rs's
global-shortcut handler. `src/shortcuts.test.ts` fails when a menu default or
the backend's `GLOBALS`/`KEYS` differ from the table or a hint is typed by
hand.

Archive: keyboard shortcut detail.

## Reference chord ban fixture (0.1, 2026-11-20)

`macReserved.test.ts` carries the whole default chord table of the t3code client (read
2026-11-19, its shared keybindings module) as `REFERENCE_CHORDS`, one verdict per chord:
`reserved` (macOS owns it: `reservedReason` or the in-surface ⌥ rule refuses it, and no default
may sit on it), `owned:<id>` (Hollow already binds it to that command), `local:<label>` (an
in-surface key) or `free` (unbound on purpose). A lane that wants a `free` chord changes its row
in the same PR and ships a visible click path. The same file pins the palette rule: ⌘1 to ⌘5 are
the only global digits; a digit hint on a palette row is read by the open palette itself, never
registered as an app shortcut, and ⌘6 to ⌘9 stay unbound outside it.

## Shortcut contexts (`when`, t3code parity 4.6)

A command's `when` (`shortcuts.ts` `ShortcutContext`) says where its key works: `anywhere` (the default),
`chat` (acts on the conversation on screen: Stop the running turn) or `palette` (read only by the open
command palette). `shortcutFor(event, active)` runs only commands whose context is active, a specific
context beating `anywhere`; the window passes `WINDOW_CONTEXTS`, the palette `PALETTE_CONTEXTS`.
`assignChord` displaces another command only when the two contexts can both be active, so a future
`chat` key may share a chord with a `palette` one. Settings shows a context chip on any row that is not
`anywhere`, and lists the palette's own keys (`CONTEXT_KEYS`: `>`, the digits, Backspace) read only,
under "In the palette". Contexts are fixed in code: a person cannot set one, so nothing is saved
for them and `shortcuts.rs` is unchanged.

## Local-command keyboard policy

Moved here on 2026-10-15 from the next-level programme ledger
(NEXT_LEVEL_PROGRAMME.md, P18), which is history,
and brought up to date: the board keys went with whiteboards on 2026-10-07, and
⌥⌘V (make a canvas from the selection) came with overnight 10-11/12; its
action and its reservation went with COO rebuild L4 (2026-11-10).

App shortcuts (Settings → Keyboard shortcuts, master switch) cover the menu
and global keys; off frees every system-wide registration
(`shortcuts::effective` returns none). In-surface commands (`LOCAL_COMMANDS`
in `src/localKeys.ts`, listed on the `?` card, opened by Settings → Keyboard shortcuts → Show all keys) work only while their surface is in front: the
conversation on screen (⌘F), the browser panel showing (⌘L, and ⌘[ ⌘] ⌘R for
its page's Back, Forward and Reload), text selected in
a file or note (⌥⌘T / ⌥⌘A; `components/SelectionActions.tsx`), the file editor (⌘S). They stay active
when app shortcuts are off: they cannot shadow another app and do not register
globally. None of them takes a system-wide chord; a default app chord only for
the same command in that surface (`sameAs`: the panel's Back and Forward on
Hollow's ⌘[ ⌘]); and a macOS
chord (`RESERVED` in `src/shortcuts.ts`, i.e. `MAC_RESERVED` in
`src/macReserved.ts`) only for its macOS meaning (Save
on ⌘S, Find on ⌘F, Reload on ⌘R); `src/localKeys.test.ts` enforces all three.
A surface that handles a key calls `preventDefault` from a listener on its own
element (not Solid's delegated `onKeyDown`, which runs on `document` after
App's keymap), and App's keymap skips a `defaultPrevented` key (P1-47). Keys
pressed in the browser panel's native page reach only the native menu:
`appMenu.ts` `runNativeMenu` sends its Back and Forward to the page while
`store/browser.ts` `browserTakesNav()` says the page has the keys (focus in
the panel, or the app's document lost focus to the page), so there they follow
the app's Back and Forward keys and the shortcuts switch; ⌘R has no menu item
and reloads only with focus in the panel's toolbar. Annotate's ⌘Z
(Undo) and ⇧⌘Z (Redo) act only on keys pressed on the snapshot or with nothing
focused. ⌥⇧⌘T is the
system-wide quick capture (the Add sheet set to a task), not a selection key. The Edit keys ⌘Z/⌘C/⌘V/⌘A keep
their Edit meaning inside a surface. Recording an app shortcut on a chord a
surface uses says so on the row (`localUsesOf`).

To add an in-surface key: a `LOCAL_COMMANDS` row (surface, label, chord), a
listener that fires only with focus inside the surface, and this paragraph.

---

**Project script chords (parity 4.5, `src/projectScripts.ts`).** A script's chord is always ⇧⌘ and one
letter or digit, picked from a list in the script editor where each refused key is disabled with its
reason (macOS's, a Hollow command's, or another script's). `scriptForChord` runs it in the open project
only when no Hollow command holds the key (App.tsx asks `windowCommandFor` first), and re-checks the
guard at press time, so a chord a command took later stops running the script instead of shadowing it.
`projectScripts.test.ts` fails if a `MAC_RESERVED` or default command chord of that shape is accepted.
