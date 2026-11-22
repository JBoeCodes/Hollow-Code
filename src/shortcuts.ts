import { createSignal } from 'solid-js';
import type { PageId } from './store';
import { PAGE_KEY, PAGE_NAV_BY_ID } from './navigation';
import { MAC_RESERVED } from './macReserved';
import { leaderKeyOf } from './keyboardLeader';

// The one keymap. App.tsx asks `shortcutFor(event)` which command a
// keystroke runs; CommandPalette, the TopBar's tooltips and the keymap card
// (ShortcutsOverlay) read the display strings below, so a binding and its
// hint cannot drift. Settings, Keyboard shortcuts changes the keys or turns
// them all off; the backend (src-tauri/src/shortcuts.rs) saves that and puts
// it into the native menu (app_menu.rs) and the two system-wide keys. A unit
// test (shortcuts.test.ts) checks the menu's default accelerators against
// this table.
export { PAGE_KEY, PAGE_ORDER } from './navigation';

const IS_MAC = typeof navigator !== 'undefined' && navigator.platform.toLowerCase().includes('mac');
export const MOD = IS_MAC ? '⌘' : 'Ctrl+';

/** A chord as the backend and the native menu write it: `CmdOrCtrl+`, then
 *  optionally `Alt+` and `Shift+`, then one key (`CmdOrCtrl+Shift+[`). Two
 *  bare keys the webview handles itself: `?` (the keymap card) and `Escape`
 *  (stop a running turn). `''` is none. */
export type Chord = string;

/** Where a command's key works (t3code parity 4.6, A11). A command without
 *  `when` works anywhere in the window. `chat` acts on the conversation on
 *  screen; `palette` keys are read only by the open palette. Two commands may
 *  share a chord only when their contexts cannot both be active, and Settings
 *  says which context a row is in. */
export type ShortcutContext = 'anywhere' | 'chat' | 'palette';
export const CONTEXT_LABEL: Record<ShortcutContext, string> = { anywhere: 'Anywhere', chat: 'In a chat', palette: 'In the palette' };
/** The contexts active in the window when nothing says otherwise. */
export const WINDOW_CONTEXTS: readonly ShortcutContext[] = ['anywhere', 'chat'];
/** The contexts active while the command palette is open. */
export const PALETTE_CONTEXTS: readonly ShortcutContext[] = ['anywhere', 'palette'];
/** Contexts that can be active together: `anywhere` overlaps all, others only themselves,
 *  and `chat` never meets `palette` (the palette is modal). */
export function contextsOverlap(a: ShortcutContext = 'anywhere', b: ShortcutContext = 'anywhere'): boolean {
  return a === 'anywhere' || b === 'anywhere' || a === b;
}

export interface ShortcutCommand {
  /** The native menu item's id where there is one (app_menu.rs ITEMS). */
  id: string;
  /** Where its key works; anywhere when absent. */
  when?: ShortcutContext;
  label: string;
  group: 'Navigate' | 'Create' | 'View' | 'Chat' | 'Anywhere';
  default: Chord;
  /** Registered system-wide by the backend: works while another app is in front. */
  global?: boolean;
  /** Where a page jump goes. */
  page?: PageId;
  /** The native menu has no item for it, so only the window runs its key
   *  (a ⌥ chord included; `windowCommandFor`). */
  noMenuItem?: true;
}

/** Page jumps' command ids (their native menu items). */
const PAGE_COMMAND: Partial<Record<PageId, string>> = {
  today: 'go-work', tasks: 'go-tasks', vault: 'go-library', workflows: 'go-automations', review: 'go-review', settings: 'settings',
};

const pageCommands: ShortcutCommand[] = (Object.entries(PAGE_KEY) as [PageId, string][])
  .filter(([page]) => PAGE_COMMAND[page])
  .map(([page, key]) => ({ id: PAGE_COMMAND[page]!, label: PAGE_NAV_BY_ID[page]?.label ?? page, group: 'Navigate', default: `CmdOrCtrl+${key}`, page }));

/** Every command a shortcut can run, in Settings order. Each is a shared
 *  action (actionDefs.ts `key`); its label here is that action's word for
 *  the keymap, kept literal so the launch bundle needn't carry the action
 *  table, and actionDefs.test.ts fails if the two differ. */
export const COMMANDS: readonly ShortcutCommand[] = [
  ...pageCommands,
  { id: 'go-back', label: 'Back', group: 'Navigate', default: 'CmdOrCtrl+[' },
  { id: 'go-forward', label: 'Forward', group: 'Navigate', default: 'CmdOrCtrl+]' },
  { id: 'prev-conversation', label: 'Previous conversation', group: 'Navigate', default: 'CmdOrCtrl+Shift+[' },
  { id: 'next-conversation', label: 'Next conversation', group: 'Navigate', default: 'CmdOrCtrl+Shift+]' },
  { id: 'workspace-conversation', label: 'Workspace conversation', group: 'Navigate', default: 'CmdOrCtrl+Shift+C', noMenuItem: true },
  { id: 'jump', label: 'Jump to a workspace or chat', group: 'Navigate', default: 'CmdOrCtrl+J', noMenuItem: true },
  // The top bar's search icon (L1, 11-12). ⌘⇧K is no macOS chord, and no
  // other Hollow command has it.
  { id: 'search-chats', label: 'Search chats', group: 'Navigate', default: 'CmdOrCtrl+Shift+K', noMenuItem: true },
  { id: 'new-conversation', label: 'New conversation', group: 'Create', default: 'CmdOrCtrl+N' },
  { id: 'new-quick-chat', label: 'New chat', group: 'Create', default: '' },
  { id: 'new-workspace', label: 'New project', group: 'Create', default: '' },
  { id: 'new-note', label: 'New note', group: 'Create', default: 'CmdOrCtrl+Shift+N' },
  { id: 'new-task', label: 'New task', group: 'Create', default: '' },
  { id: 'search', label: 'Command palette', group: 'View', default: 'CmdOrCtrl+K' },
  { id: 'search-alt', label: 'Command palette (second key)', group: 'View', default: 'CmdOrCtrl+Shift+P', noMenuItem: true },
  { id: 'toggle-sidebar', label: 'Toggle sidebar', group: 'View', default: 'CmdOrCtrl+B' },
  // The Preview pane (run 15 P1); the id stays, so a saved custom key keeps working.
  { id: 'toggle-side-pane', label: 'Toggle preview pane', group: 'View', default: 'CmdOrCtrl+\\' },
  // The Main agent page (run 15 P3; it toggled a drawer beside any page
  // until then, hence the id, kept for custom bindings). ⌘M is Minimize;
  // ⌘⇧M is no macOS chord.
  { id: 'main-drawer', label: 'COO', group: 'View', default: 'CmdOrCtrl+Shift+M' },
  { id: 'shortcuts', label: 'Keyboard shortcuts card', group: 'View', default: '?' },
  { id: 'stop-run', label: 'Stop the running turn', group: 'Chat', default: 'Escape', when: 'chat' },
  // Push-to-talk (overnight 10-11 B7): tap to start and stop, or hold and
  // release. ⌘⇧D is no macOS chord.
  { id: 'dictate', label: 'Dictate (tap, or hold and release)', group: 'Chat', default: 'CmdOrCtrl+Shift+D' },
  // Three modifiers (five asks A4): a system-wide key takes its chord from
  // every app, and ⌘⇧T is every browser's Reopen closed tab.
  { id: 'quick-capture', label: 'Quick capture task', group: 'Anywhere', default: 'CmdOrCtrl+Alt+Shift+T', global: true },
  { id: 'quick-ingest', label: 'Import a source', group: 'Anywhere', default: 'CmdOrCtrl+Alt+Shift+I', global: true },
  { id: 'main-agent', label: 'COO', group: 'Anywhere', default: 'CmdOrCtrl+Alt+Shift+H', global: true },
];

export const COMMAND_BY_ID: Record<string, ShortcutCommand> = Object.fromEntries(COMMANDS.map(c => [c.id, c]));

/** Keys a chord may end in (the backend's KEYS). */
export const CHORD_KEYS: readonly string[] = [
  ...'ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789'.split(''),
  '[', ']', '\\', ',', '.', '/', ';', "'", '-', '=', '`',
  ...Array.from({ length: 12 }, (_, i) => `F${i + 1}`),
  'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight',
];

/** Chords macOS or the Edit and File menus already own (five asks A4: the
 *  full list in `macReserved.ts`); a command can't take them, except the
 *  two Hollow uses for their macOS meaning (`reservedReason`). */
export const RESERVED: Readonly<Record<Chord, string>> = MAC_RESERVED;

// ── The saved settings ───────────────────────────────────────────────────

export interface ShortcutSettings {
  /** Off: no Hollow shortcut works, in the window or system-wide. */
  enabled: boolean;
  /** Command id → chord, `''` for none. Only commands the user changed. */
  keys: Record<string, Chord>;
  /** Go-to target id → its key after `g` (`keyboardLeader.ts`), `''` for
   *  none. Only targets the user changed. */
  leader?: Record<string, string>;
}
export interface ShortcutsView extends ShortcutSettings { warnings: string[] }

export const DEFAULT_SHORTCUTS: ShortcutSettings = { enabled: true, keys: {}, leader: {} };

/** The settings in effect. Defaults until the backend answers (App.tsx). */
export const [shortcutSettings, setShortcutSettings] = createSignal<ShortcutSettings>(DEFAULT_SHORTCUTS);
/** What the backend could not apply (a system-wide key another app holds). */
export const [shortcutWarnings, setShortcutWarnings] = createSignal<string[]>([]);
/** Settings is recording a key: no shortcut runs. */
export const [recordingShortcut, setRecordingShortcut] = createSignal(false);

export function acceptShortcutsView(v: ShortcutsView) {
  setShortcutSettings({ enabled: v.enabled !== false, keys: { ...(v.keys ?? {}) }, leader: { ...(v.leader ?? {}) } });
  setShortcutWarnings(v.warnings ?? []);
}

export const shortcutsEnabled = () => shortcutSettings().enabled;

/** Whether a key aimed at `target` is for the view rooted at `root` (P0-10):
 *  inside it, or at the page itself (nothing focused). A key typed in a
 *  dialog, a pane or a card beside the view is never its own,
 *  so a view listening on document leaves it to them. */
export function keyInView(
  root: { contains(node: Node | null): boolean } | null | undefined,
  target: EventTarget | null,
  page: { body: unknown; documentElement: unknown } = document,
): boolean {
  if (!target || target === page.body || target === page.documentElement) return true;
  return !!root && root.contains(target as Node);
}

/** A command's chord, on or off (what Settings shows). */
export function chordOf(id: string, s: ShortcutSettings = shortcutSettings()): Chord {
  return id in s.keys ? s.keys[id] : COMMAND_BY_ID[id]?.default ?? '';
}

/** A command's chord while shortcuts are on; `''` when they're off. */
export function activeChord(id: string): Chord {
  return shortcutsEnabled() ? chordOf(id) : '';
}

/** Give `id` this chord. A command that had it loses it (`displaced`). A
 *  command set back to its default drops out of `keys`. */
export function assignChord(s: ShortcutSettings, id: string, chord: Chord): { next: ShortcutSettings; displaced: string | null } {
  const keys = { ...s.keys };
  let displaced: string | null = null;
  if (chord) {
    for (const c of COMMANDS) {
      if (c.id !== id && chordOf(c.id, s) === chord && contextsOverlap(c.when, COMMAND_BY_ID[id]?.when)) {
        displaced = c.id;
        if (c.default === '') delete keys[c.id]; else keys[c.id] = '';
      }
    }
  }
  if (COMMAND_BY_ID[id]?.default === chord) delete keys[id]; else keys[id] = chord;
  return { next: { ...s, keys }, displaced };
}

// ── Display ──────────────────────────────────────────────────────────────

const KEY_GLYPH: Record<string, string> = { ArrowUp: '↑', ArrowDown: '↓', ArrowLeft: '←', ArrowRight: '→', Escape: 'Esc' };

/** "CmdOrCtrl+Shift+N" → "⇧⌘N" (macOS's order, ⌃⌥⇧⌘, as its menus write
 *  it; P1-49), "Ctrl+Shift+N" elsewhere; `''` → `''`. */
export function displayChord(chord: Chord, mac = IS_MAC): string {
  if (!chord) return '';
  if (!chord.startsWith('CmdOrCtrl+')) return KEY_GLYPH[chord] ?? chord;
  const parts = chord.split('+');
  // A chord ends in its key; no key a chord may use is '+'.
  const key = KEY_GLYPH[parts[parts.length - 1]] ?? parts[parts.length - 1];
  const alt = parts.includes('Alt'), shift = parts.includes('Shift');
  return mac
    ? `${alt ? '⌥' : ''}${shift ? '⇧' : ''}⌘${key}`
    : `Ctrl+${alt ? 'Alt+' : ''}${shift ? 'Shift+' : ''}${key}`;
}

/** The hint for a command now (e.g. "⌘N"), `''` when it has none or shortcuts are off. */
export const keyFor = (id: string) => displayChord(activeChord(id));

/** The go-to hint for target `id` now ("g d"), `''` when it has no key or
 *  shortcuts are off: a rebound or cleared key must not be advertised
 *  (cross-PR review S2: hints were written in as text). */
export function goHint(id: string): string {
  if (!shortcutsEnabled()) return '';
  const k = leaderKeyOf(id, shortcutSettings().leader ?? {});
  return k ? `g ${k}` : '';
}

export function pageShortcut(p: PageId): string {
  const id = PAGE_COMMAND[p];
  return id ? keyFor(id) : '';
}

// ── Matching ─────────────────────────────────────────────────────────────

export interface ChordEvent { key: string; code?: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean }

const CODE_KEY: Record<string, string> = {
  BracketLeft: '[', BracketRight: ']', Backslash: '\\', Comma: ',', Period: '.', Slash: '/',
  Semicolon: ';', Quote: "'", Minus: '-', Equal: '=', Backquote: '`',
};
// What Shift turns a US key into, for events with no `code` (tests).
const SHIFTED: Record<string, string> = {
  '{': '[', '}': ']', '|': '\\', '<': ',', '>': '.', '?': '/', ':': ';', '"': "'", '_': '-', '+': '=', '~': '`',
  '!': '1', '@': '2', '#': '3', '$': '4', '%': '5', '^': '6', '&': '7', '*': '8', '(': '9', ')': '0',
};

/** The key a keystroke names, as a chord writes it, or null for a key no
 *  chord can use (a modifier alone, Enter, Space…). A Latin letter is read
 *  from `key`, as the native menu does; with ⌥ held, or for punctuation and
 *  digits (Shift+[ reports "{"), from the physical `code`. */
export function chordKey(e: ChordEvent): string | null {
  if (!e.altKey && /^[a-z]$/i.test(e.key)) return e.key.toUpperCase();
  const code = e.code ?? '';
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (CODE_KEY[code]) return CODE_KEY[code];
  if (CHORD_KEYS.includes(code)) return code;
  if (code) return null;
  const raw = SHIFTED[e.key] ?? e.key;
  const key = raw.length === 1 ? raw.toUpperCase() : raw;
  return CHORD_KEYS.includes(key) ? key : null;
}

/** Whether a keystroke holds the chord modifier: ⌘ alone on a Mac (⌃ keeps
 *  its Cocoa meanings, ⌃K delete-to-end, ⌃A line start…; five asks A4),
 *  Ctrl elsewhere. */
export function hasChordModifier(e: { metaKey: boolean; ctrlKey: boolean }, mac = IS_MAC): boolean {
  return mac ? e.metaKey && !e.ctrlKey : e.ctrlKey || e.metaKey;
}

/** A ⌘ (or Ctrl) keystroke as a chord; null without it or a usable key. */
export function chordFromEvent(e: ChordEvent, mac = IS_MAC): Chord | null {
  if (!hasChordModifier(e, mac)) return null;
  const key = chordKey(e);
  if (!key) return null;
  return `CmdOrCtrl+${e.altKey ? 'Alt+' : ''}${e.shiftKey ? 'Shift+' : ''}${key}`;
}

/** Which command a ⌘ keystroke runs, or null. The system-wide commands are
 *  left to the backend (it already took the keystroke); `?` and Esc are
 *  matched where they're handled (App.tsx, ChatBody). */
export function shortcutFor(e: ChordEvent, active: readonly ShortcutContext[] = WINDOW_CONTEXTS): ShortcutCommand | null {
  if (!shortcutsEnabled() || recordingShortcut()) return null;
  const chord = chordFromEvent(e);
  if (!chord) return null;
  const hits = COMMANDS.filter(c => !c.global && activeChord(c.id) === chord && active.includes(c.when ?? 'anywhere'));
  // A command bound to the open context beats one that works anywhere.
  return hits.find(c => c.when && c.when !== 'anywhere') ?? hits[0] ?? null;
}

/** Keys only a surface reads, listed read-only in Settings under their
 *  context (not commands: they cannot be rebound, and none is a chord the
 *  app registers). */
export const CONTEXT_KEYS: readonly { when: ShortcutContext; keys: string; label: string }[] = [
  { when: 'palette', keys: '>', label: 'Actions only' },
  { when: 'palette', keys: `${MOD}1 to ${MOD}9`, label: 'Choose the row with that number' },
  { when: 'palette', keys: '⌫', label: 'Back one level (empty box)' },
];

/** Which command a ⌘ keystroke in the window runs (App.tsx). A ⌥ chord runs
 *  here only for a command the native menu has no item for (an item's key
 *  reaches the menu, which runs it), and never when an in-surface key uses
 *  it (⌥⌘T makes a task from selected text; P1-49). */
export function windowCommandFor(e: ChordEvent): ShortcutCommand | null {
  const command = shortcutFor(e);
  if (!command || !e.altKey) return command;
  return command.noMenuItem && !LOCAL_ALT_CHORDS.includes(activeChord(command.id)) ? command : null;
}

/** The in-surface ⌥ chords (`localKeys.ts` LOCAL_COMMANDS, kept out of the
 *  launch bundle; shortcuts.test.ts holds the two equal). */
export const LOCAL_ALT_CHORDS: readonly Chord[] = ['CmdOrCtrl+Alt+T', 'CmdOrCtrl+Alt+A'];
