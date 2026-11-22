import type { IconName } from './icons';
import { ICON_MEANING } from './iconMeaning.ts';
/**
 * The app's shared action definitions (plan completion L4, 2026-10-13;
 * review F11/V06, matrix N9/N10). One row per action a person can run from
 * more than one place: the palette, a `g` go-to key, a ⌘ shortcut, the
 * native menu bar. Each row says what the action does to their work (its
 * effect), when it is offered (its condition, with what the person is told
 * when it isn't), which capability of the route ledger it belongs to (its
 * visible click path and the tests that walk it) and its words.
 *
 * Adapters read this table instead of keeping their own: the palette's
 * fixed rows (CommandPalette.tsx), the go-to keys (goTo.ts), the ⌘ keymap's
 * labels (shortcuts.ts) and the native menu (appMenu.ts, app_menu.rs) run
 * one dispatcher, `runAction` (actionRun.ts). A surface that needs a
 * different word or a different behaviour says so here (`labels`) or in
 * `PALETTE_OWN_RUN` (CommandPalette.tsx), each with its reason, and the
 * tests list them.
 *
 * Pure data; imports nothing.
 */

/**
 * What running an action does to the person's work:
 * - `navigate`: shows another place; nothing changes.
 * - `view`: shows, hides or opens something in place; nothing changes.
 * - `draft`: opens a form, sheet or message box; nothing is saved or sent
 *   until the person submits it.
 * - `write`: changes files or settings on this Mac.
 * - `run`: starts or stops agent work, which may cost money.
 * - `external`: sends something outside this Mac.
 * `confirm` marks an action whose effect waits for a preview or
 * confirmation the person answers.
 */
export type Effect = 'navigate' | 'view' | 'draft' | 'write' | 'run' | 'external';

export const EFFECT_WORDS: Readonly<Record<Effect, string>> = {
  navigate: 'Goes somewhere; changes nothing',
  view: 'Shows or hides something; changes nothing',
  draft: 'Opens a form or message box; nothing is saved or sent until you do',
  write: 'Changes files or settings on this Mac',
  run: 'Starts or stops agent work, which may cost money',
  external: 'Sends something outside this Mac',
};

/** When an action is offered, and what a person who reaches it anyway
 *  (a menu item, a shortcut) is told. `actionRun.ts` evaluates them. */
export const CONDITIONS = {
  'running-turn': { when: 'While a conversation is running a turn', unavailable: 'Nothing is running in this conversation.' },
  'chat-action-undo': { when: 'For 5 seconds after a chat is settled, snoozed, pinned or woken', unavailable: 'Nothing to undo: Undo shows for 5 seconds after Settle, Snooze, Pin or Wake.' },
  'workspace-open': { when: 'While a workspace is open', unavailable: 'Open a project first: Projects → a project, or the sidebar.' },
  'side-pane': { when: 'While the Preview pane shows something, has an item to bring back, or a workspace is open (its side panel card)', unavailable: 'Open a workspace, a file or a link first: it shows in the Preview pane.' },
  'can-leave': { when: 'Unless a message is still being sent', unavailable: 'Wait a moment: the current message is still being sent.' },
  'build-workspace': { when: 'In a build workspace (one with its own git worktree)', unavailable: 'Changes are shown only in a build workspace, one with its own git worktree.' },
  'conversation-agent': { when: 'With an agent installed, in a conversation or workspace', unavailable: 'Open a conversation first; models belong to the agent that answers it. No agent installed? Settings → Models.' },
  'can-merge': { when: 'In a workspace you made, with nothing running', unavailable: 'Open a workspace you made first. Chats and the COO can’t be merged, and nothing may be running.' },
  'can-go-back': { when: 'After you have moved between pages', unavailable: 'Nothing to go back to yet.' },
  'can-go-forward': { when: 'After going back', unavailable: 'Nothing to go forward to.' },
  'recent-saved-canvas': { when: 'After using a workspace canvas', unavailable: 'Open a workspace canvas first.' },
  'closed-view': { when: 'After a Preview pane tab was closed', unavailable: 'No closed tab to bring back yet.' },
  'side-chat': { when: 'In a project chat or Chats (never the COO), or while a side chat is open', unavailable: 'Open an ordinary chat first: a side chat sits beside a project chat or a chat in Chats. The COO keeps its own page.' },
} as const;
export type Condition = keyof typeof CONDITIONS;

/** The palette's section for a row. */
export type PaletteGroup = 'Actions' | 'Conversation';

export interface ActionDef {
  /** Stable id. The ⌘ command / native menu id where there is one. */
  id: string;
  /** What the person can do, in the app's words: the palette row, the
   *  keymap's row in Settings and the native menu item (title-cased) all
   *  say this unless `labels` gives a surface its own. */
  label: string;
  effect: Effect;
  confirm?: boolean;
  /** The route ledger row (src/routeLedger.ts) holding its visible click
   *  path and the tests that walk it. */
  route: string;
  when?: Condition;
  /** Palette row id (`act:…`). */
  palette?: string;
  /** Go-to target (`g` + key, keyboardLeader.ts). */
  go?: string;
  /** ⌘ shortcut command (shortcuts.ts COMMANDS). */
  key?: string;
  /** Native menu item id (app_menu.rs ITEMS); the same as `key` where both exist. */
  menu?: string;
  /** Where the palette offers it, when not in its main list: the Move
   *  to… list of one conversation. */
  paletteIn?: 'move-session';
  /** Why the palette runs its own variant instead of `runAction` (its
   *  implementation is CommandPalette.tsx PALETTE_OWN). */
  paletteOwn?: string;
  /** A surface's own words, each a deliberate exception (tested). */
  labels?: { palette?: string; key?: string; menu?: string };
  /** The palette row's look and search terms. */
  icon?: IconName;
  keywords?: string;
  group?: PaletteGroup;
}

export const ACTION_DEFS: readonly ActionDef[] = [
  // ── Create ──────────────────────────────────────────────────────────
  { id: 'new-conversation', label: 'New conversation', effect: 'draft', route: 'create.conversation', palette: 'act:new-conversation', key: 'new-conversation', menu: 'new-conversation',
    paletteOwn: 'With no workspace open it asks which workspace (Chats first); ⌘N with no project open and the sidebar Quick chats + start a quick chat (2026-11-14).',
    icon: 'message', keywords: 'chat thread agent start fresh' },
  { id: 'new-quick-chat', label: 'New chat', effect: 'draft', route: 'create.quick-chat', palette: 'act:new-quick-chat', go: 'quick-chat', key: 'new-quick-chat', menu: 'new-quick-chat',
    icon: 'message', keywords: 'chat chats quick chat scratch ask anything no workspace project' },
  { id: 'new-workspace', label: 'New project', effect: 'draft', route: 'create.workspace', when: 'can-leave', palette: 'act:new-workspace', key: 'new-workspace', menu: 'new-workspace',
    icon: 'plus', keywords: 'outcome result start project' },
  { id: 'jump', label: 'Jump to a workspace or chat', effect: 'view', route: 'find.search', palette: 'act:jump', go: 'jump', key: 'jump',
    paletteOwn: 'The palette is already open: it switches itself to its workspaces-and-chats mode.',
    icon: 'search', keywords: 'switcher switch go to open workspace conversation chat recent quick open' },
  // ⌘⇧M and View → Main Agent (run 15 P3): the drawer this id toggled is
  // gone; it opens the page, and the palette's one row for it is main-agent.
  { id: 'main-drawer', label: 'COO', effect: 'navigate', route: 'ask.main-agent', key: 'main-drawer', menu: 'main-drawer',
    icon: ICON_MEANING.mainAgent, keywords: 'coo main agent orb chief of staff page' },
  { id: 'main-agent', label: 'COO', effect: 'navigate', route: 'ask.main-agent', palette: 'act:main-agent', go: 'main-agent', key: 'main-agent',
    labels: { palette: 'Open the COO' },
    icon: ICON_MEANING.mainAgent, keywords: 'coo main agent orb chief of staff today check-in standing orders subagents' },

  // ── Work, Review, Tasks: places ────────────────────────────────
  // Today, Weekly review, Incoming, Commitments and Agenda were cut
  // (overnight 11-08 S3). The jobs are in chat::SCREEN_JOBS.
  { id: 'dictate', label: 'Dictate into the message box', effect: 'draft', route: 'ask.dictate', palette: 'act:dictate', key: 'dictate', menu: 'dictate',
    labels: { key: 'Dictate (tap, or hold and release)', menu: 'Dictate' },
    icon: 'mic', keywords: 'dictate dictation voice speak talk push to talk microphone mic speech transcribe say' },
  // One Ask (2026-11-03): Work's Ask field, focused. Keeps the id, palette
  // id and `g v` of the Ask your vault view it replaced (and Ask Hollow…'s
  // words), so a custom key and the agent's navigate page still work.
  // What's new, the welcome tour, the workspace map, Runbooks, dry-run
  // tonight, archive suggestions, pressure tests and personas were cut
  // (overnight 11-08 S4). The jobs stay in chat::SCREEN_JOBS.
  { id: 'ask-vault', label: 'Ask anything…', effect: 'navigate', route: 'ask.scoped', palette: 'act:ask-vault', go: 'ask-vault',
    paletteOwn: 'What was typed in the palette, if it reads as a sentence, goes into Ask anything when the field is empty.',
    icon: ICON_MEANING.ask, keywords: 'ask anything ask hollow ask your vault question answer quick chat coo main agent across all my work sentence natural language what did we decide who said find out' },
  { id: 'activity', label: 'Running', effect: 'navigate', route: 'work.activity', palette: 'act:mission-control', go: 'mission-control', labels: { palette: 'Open Running (Projects)' },
    icon: 'zap', keywords: 'activity your work mission control running agents turns queue live stop watch every agent orbit cost' },
  // Parity 7.3: a palette row and the Projects tab; no go-to key or chord (nothing to guard).
  { id: 'pull-requests', label: 'Pull requests', effect: 'navigate', route: 'work.pulls', palette: 'act:pull-requests', labels: { palette: 'Open Pull requests (Projects)' },
    icon: 'gitPullRequest', keywords: 'github pull requests issues prs triage review checks labels assign comment close gh linear' },
  { id: 'canvases', label: 'Canvases', effect: 'navigate', route: 'library.canvases', palette: 'act:canvas-gallery', go: 'canvas-gallery', labels: { palette: 'Open Canvases (Library)' },
    icon: 'appWindow', keywords: 'canvas gallery library results all canvases visuals saved charts diagrams versions revisions compare history copy html browse' },
  // Library → All (overnight 11-12 L3): one searchable list, Library's first tab.
  { id: 'library-all', label: 'Library: All', effect: 'navigate', route: 'library.all', palette: 'act:library-all',
    icon: 'list', keywords: 'all everything search find recent meetings people outputs notes recordings results canvases one list' },
  // All's views (overnight 11-12 L4): Notes, Results and Recordings left the
  // tab strip and open from All; these rows still open each one.
  { id: 'library-notes', label: 'Library: Notes', effect: 'navigate', route: 'library.notes', palette: 'act:library-notes',
    icon: 'fileText', keywords: 'notes browse folders collections records decisions saved views templates reader editor sources' },
  { id: 'library-results', label: 'Library: Results', effect: 'navigate', route: 'library.results', palette: 'act:library-results',
    icon: 'layers', keywords: 'results grid outputs files canvases thumbnails review workspace filter compare versions' },
  { id: 'library-recordings', label: 'Library: Recordings', effect: 'navigate', route: 'library.recordings', palette: 'act:library-recordings',
    icon: 'mic', keywords: 'recordings meetings plaud transcripts audio summary link proposed tasks' },
  // Library's Skills and History tabs (overnight 11-12 L2): Settings' skills
  // catalog and History & recovery, opened by store/libraryMode openLibraryTab.
  { id: 'library-skills', label: 'Library: Skills', effect: 'navigate', route: 'library.skills', palette: 'act:library-skills',
    icon: ICON_MEANING.skill, keywords: 'skills catalog browse make new import install use edit delete agent capabilities slash commands' },
  { id: 'library-history', label: 'Library: History', effect: 'navigate', route: 'library.history', palette: 'act:library-history',
    icon: 'history', keywords: 'history recovery restore undo automated writes runs vault changes snapshots versions' },
  { id: 'conflict-copies', label: 'Resolve conflict copies…', effect: 'write', confirm: true, route: 'review.conflict-copies', palette: 'act:conflict-copies',
    icon: 'copy', keywords: 'icloud finder duplicate copy 2 conflicted copy note vault keep one trash' },
  { id: 'save-canvas-image', label: 'Export canvas…', effect: 'draft', route: 'canvas.save-image', when: 'recent-saved-canvas', palette: 'act:save-canvas-image',
    icon: 'copy', keywords: 'export png picture clipboard' },
  { id: 'new-note', label: 'New note', effect: 'draft', route: 'library.new-note', palette: 'act:new-note', key: 'new-note', menu: 'new-note',
    icon: 'plus', keywords: 'vault create markdown' },
  { id: 'add', label: 'Add a task, note, link, file or recording…', effect: 'draft', route: 'tasks.add', palette: 'act:add',
    icon: 'filePlus', keywords: 'add capture anything paste drop file note task link recording export inbox' },
  { id: 'new-task', label: 'New task', effect: 'draft', route: 'tasks.new', palette: 'act:new-task', key: 'new-task', menu: 'new-task',
    labels: { menu: 'New Task…' },
    icon: 'checkSquare', keywords: 'todo create capture quick add new natural language' },
  { id: 'quick-capture', label: 'Quick capture task', effect: 'draft', route: 'tasks.add', key: 'quick-capture' },
  { id: 'quick-ingest', label: 'Import a source', effect: 'draft', route: 'library.import', palette: 'act:quick-ingest', key: 'quick-ingest',
    labels: { key: 'Import a source' },
    icon: ICON_MEANING.import, keywords: 'capture paste link url pdf image text youtube source import note knowledge agent' },
  { id: 'connectors', label: 'Manage connectors', effect: 'navigate', route: 'settings.connectors', palette: 'act:connectors',
    icon: 'globe', keywords: 'connectors mcp servers share claude codex import superhuman mail tools integrations' },
  { id: 'appearance', label: 'Theme and appearance', effect: 'navigate', route: 'settings.appearance', palette: 'act:open-appearance',
    icon: 'moon', keywords: 'theme dark light custom palette appearance' },
  { id: 'toggle-sidebar', label: 'Toggle sidebar', effect: 'view', route: 'nav.sidebar', palette: 'act:toggle-sidebar', key: 'toggle-sidebar', menu: 'toggle-sidebar',
    icon: 'chevronRight', keywords: 'navigation collapse' },
  { id: 'shortcuts', label: 'Keyboard shortcuts', effect: 'view', route: 'settings.keys', palette: 'act:shortcuts', key: 'shortcuts', menu: 'shortcuts',
    labels: { key: 'Keyboard shortcuts card' },
    icon: 'zap', keywords: 'keymap help keys hotkeys bindings cheat sheet' },
  { id: 'save-diagnostics', label: 'Save diagnostics…', effect: 'write', confirm: true, route: 'settings.diagnostics', palette: 'act:save-diagnostics',
    icon: 'download', keywords: 'bug report support logs zip troubleshoot' },
  { id: 'export-settings', label: 'Export settings…', effect: 'write', confirm: true, route: 'settings.transfer', palette: 'act:export-settings',
    icon: 'download', keywords: 'backup preferences json move mac' },
  { id: 'import-settings', label: 'Import settings…', effect: 'write', confirm: true, route: 'settings.transfer', palette: 'act:import-settings',
    icon: 'upload', keywords: 'restore preferences json move mac' },
  { id: 'clear-debug-logs', label: 'Clear debug logs', effect: 'write', route: 'settings.diagnostics', palette: 'act:clear-debug-logs',
    icon: 'trash', keywords: 'privacy diagnostics' },

  // ── This conversation and workspace (offered only when they can act) ─
  { id: 'stop-run', label: 'Stop the running turn', effect: 'run', route: 'conversation.stop', when: 'running-turn', palette: 'act:stop-run', key: 'stop-run', menu: 'stop-run',
    labels: { menu: 'Stop Running Turn' },
    icon: 'stop', keywords: 'cancel halt abort agent chat', group: 'Conversation' },
  // Edit → Undo Chat Action (3.1): no ⌘Z of its own, so text fields keep theirs;
  // the Undo button on the notice is the other route.
  { id: 'undo-chat-action', label: 'Undo the last chat action', effect: 'write', route: 'conversation.menu', when: 'chat-action-undo', palette: 'act:undo-chat-action', menu: 'undo-chat-action',
    labels: { menu: 'Undo Chat Action' },
    icon: 'history', keywords: 'undo settle snooze pin wake restore sidebar chat', group: 'Conversation' },
  { id: 'switch-model', label: 'Switch model…', effect: 'view', route: 'conversation.model', when: 'conversation-agent', palette: 'act:switch-model',
    icon: ICON_MEANING.agent, keywords: 'agent claude codex grok antigravity model provider effort', group: 'Conversation' },
  { id: 'open-changes', label: 'Open Changes', effect: 'view', route: 'workspace.changes', when: 'build-workspace', palette: 'act:open-changes',
    icon: 'gitBranch', keywords: 'diff git branch worktree commit pull request review changes', group: 'Conversation' },
  { id: 'open-project-files', label: 'Open project files', effect: 'view', route: 'workspace.files', when: 'workspace-open', palette: 'act:open-project-files',
    icon: 'fileText', keywords: 'workspace brief plan sources agents notes WORKSPACE.md PLAN.md SOURCES.md AGENTS.md files preview', group: 'Conversation' },
  { id: 'toggle-side-pane', label: 'Toggle preview pane', effect: 'view', route: 'preview.pane', when: 'side-pane', palette: 'act:toggle-side-pane', key: 'toggle-side-pane', menu: 'toggle-side-pane',
    icon: 'panelRight', keywords: 'preview pane file web page browser show hide toggle side panel', group: 'Conversation' },
  // t3code parity 4.6 (A14): no chord (⌘⇧T is every browser's Reopen closed tab, and Hollow keeps off it).
  { id: 'reopen-closed-view', label: 'Reopen closed view', effect: 'view', route: 'preview.pane', when: 'closed-view', palette: 'act:reopen-closed-view', menu: 'reopen-closed-view',
    icon: 'panelRight', keywords: 'reopen restore bring back closed tab preview pane recently closed undo close view', group: 'Conversation' },
  // t3code parity 7.6: a second chat beside the thread. No chord: the button beside the chat's name, this row and
  // the conversation menu are the routes.
  { id: 'toggle-side-chat', label: 'Toggle side chat', effect: 'view', route: 'conversation.side-chat', when: 'side-chat', palette: 'act:toggle-side-chat',
    icon: 'columns', keywords: 'side chat second chat beside aside quick question btw parallel ask about this chat', group: 'Conversation' },
  { id: 'merge-workspace', label: 'Merge this workspace into…', effect: 'write', confirm: true, route: 'workspace.menu', when: 'can-merge', palette: 'act:merge-workspace',
    icon: 'folder', keywords: 'combine merge workspace into another duplicate move files conversations', group: 'Conversation' },
  { id: 'prev-conversation', label: 'Previous conversation', effect: 'navigate', route: 'conversation.switch', palette: 'act:prev-conversation', key: 'prev-conversation', menu: 'prev-conversation',
    icon: 'arrowUp', keywords: 'switch conversation thread up project lead', group: 'Conversation' },
  { id: 'next-conversation', label: 'Next conversation', effect: 'navigate', route: 'conversation.switch', palette: 'act:next-conversation', key: 'next-conversation', menu: 'next-conversation',
    icon: 'chevronDown', keywords: 'switch conversation thread down project lead', group: 'Conversation' },
  { id: 'new-workspace-from-chat', label: 'New project from this chat', effect: 'draft', route: 'conversation.menu', palette: 'act:new-workspace-from-chat', paletteIn: 'move-session',
    paletteOwn: 'It needs the conversation being moved, which only the Move to… list knows.' },

  // ── Navigation: pages, history ──────────────────────────────────────
  { id: 'go-back', label: 'Back', effect: 'navigate', route: 'nav.back-forward', when: 'can-go-back', palette: 'act:nav-back', key: 'go-back', menu: 'go-back',
    icon: 'chevronRight', keywords: 'history previous' },
  { id: 'go-forward', label: 'Forward', effect: 'navigate', route: 'nav.back-forward', when: 'can-go-forward', palette: 'act:nav-forward', key: 'go-forward', menu: 'go-forward',
    icon: 'chevronRight', keywords: 'history next' },
  // Work is called Projects since L1 (11-12); the id stays.
  { id: 'go-work', label: 'Projects', effect: 'navigate', route: 'work.resume', go: 'home', key: 'go-work', menu: 'go-work' },
  { id: 'go-tasks', label: 'Tasks', effect: 'navigate', route: 'tasks.list', go: 'tasks', key: 'go-tasks', menu: 'go-tasks' },
  { id: 'go-library', label: 'Library', effect: 'navigate', route: 'library.all', go: 'library', key: 'go-library', menu: 'go-library' },
  { id: 'go-automations', label: 'Automations', effect: 'navigate', route: 'automations.list', go: 'automations', key: 'go-automations', menu: 'go-automations' },
  { id: 'review', label: 'Needs you', effect: 'navigate', route: 'review.waiting', go: 'waiting', key: 'go-review', menu: 'go-review' },
  { id: 'history', label: 'Conversation history', effect: 'navigate', route: 'library.conversations', go: 'history' },
  { id: 'usage', label: 'Usage', effect: 'navigate', route: 'settings.usage', go: 'usage' },
  { id: 'settings', label: 'Settings', effect: 'navigate', route: 'settings.page', go: 'settings', key: 'settings', menu: 'settings', labels: { menu: 'Settings…' } },
  { id: 'workspace-conversation', label: 'Workspace conversation', effect: 'navigate', route: 'workspace.back-to-conversation', key: 'workspace-conversation' },
  { id: 'search', label: 'Command palette', effect: 'view', route: 'find.search', key: 'search', menu: 'search', labels: { menu: 'Search…' } },
  { id: 'search-alt', label: 'Command palette (second key)', effect: 'view', route: 'find.search', key: 'search-alt' },
  // The top bar's search icon (L1, 11-12): the sidebar's Search chats.
  { id: 'search-chats', label: 'Search chats', effect: 'view', route: 'find.search', key: 'search-chats' },
  { id: 'diagnostics', label: 'Diagnostics', effect: 'navigate', route: 'settings.diagnostics', menu: 'diagnostics' },
  { id: 'copy-build', label: 'Copy build identity', effect: 'view', route: 'settings.build-identity', menu: 'copy-build', palette: 'act:copy-build',
    labels: { palette: 'Copy build identity (version, commit)' }, icon: 'copy', keywords: 'version commit build installed' },
];

const by = (k: 'id' | 'palette' | 'go' | 'key' | 'menu') =>
  Object.fromEntries(ACTION_DEFS.filter(d => d[k]).map(d => [d[k]!, d])) as Readonly<Record<string, ActionDef>>;
export const ACTION_BY_ID = by('id');
export const ACTION_BY_PALETTE = by('palette');
export const ACTION_BY_GO = by('go');
export const ACTION_BY_KEY = by('key');
export const ACTION_BY_MENU = by('menu');

/** A surface's word for an action: its own where `labels` gives one. The
 *  native menu title-cases the rest ("New Conversation"). */
export function actionLabel(def: ActionDef, surface: 'palette' | 'key' | 'menu' = 'palette'): string {
  const own = def.labels?.[surface];
  if (own) return own;
  if (surface !== 'menu') return def.label;
  return def.label.replace(/(^|\s)([a-z])/g, (_, s: string, c: string) => s + c.toUpperCase());
}
