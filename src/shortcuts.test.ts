import { afterEach, describe, expect, it } from 'vitest';
// Read as text, never executed: the native menu's accelerators, the
// backend's system-wide defaults and the palette/keymap sources, so a key
// can't be bound in one place and hinted differently in another.
import APP_MENU_RS from '../src-tauri/src/app_menu.rs?raw';
import SHORTCUTS_RS from '../src-tauri/src/shortcuts.rs?raw';
import APP_MENU_TS from './appMenu.ts?raw';
import PALETTE_TSX from './components/CommandPalette.tsx?raw';
import OVERLAY_TSX from './components/ShortcutsOverlay.tsx?raw';
import {
  CHORD_KEYS, COMMANDS, COMMAND_BY_ID, DEFAULT_SHORTCUTS, MOD, PAGE_KEY, RESERVED,
  activeChord, assignChord, chordFromEvent, chordOf, displayChord, keyFor, pageShortcut,
  setRecordingShortcut, setShortcutSettings, shortcutFor,
  hasChordModifier, keyInView, windowCommandFor, LOCAL_ALT_CHORDS,
} from './shortcuts';
import { localAltProblem, systemWideProblem } from './settings/shortcutRules';
import { LOCAL_COMMANDS } from './localKeys';
import { ACTION_BY_KEY, ACTION_BY_MENU } from './actionDefs';
import { reservedReason } from './macReserved';

const chord = (key: string, extra: Partial<{ shiftKey: boolean; code: string; altKey: boolean; metaKey: boolean; ctrlKey: boolean }> = {}) =>
  shortcutFor({ key, metaKey: true, ctrlKey: false, altKey: false, shiftKey: false, ...extra })?.id ?? null;

afterEach(() => { setShortcutSettings(DEFAULT_SHORTCUTS); setRecordingShortcut(false); });

describe('the keymap (frontier F4/F6)', () => {
  it('maps each default chord to one command', () => {
    expect(chord('n')).toBe('new-conversation');
    expect(chord('N', { shiftKey: true })).toBe('new-note');
    expect(chord('k')).toBe('search');
    expect(chord('P', { shiftKey: true })).toBe('search-alt');
    expect(chord('b')).toBe('toggle-sidebar');
    expect(chord('\\', { code: 'Backslash' })).toBe('toggle-side-pane');
    expect(chord('[')).toBe('go-back');
    expect(chord(']')).toBe('go-forward');
    // Shift+[ reports "{" on a US layout; the physical key still matches.
    expect(chord('{', { shiftKey: true, code: 'BracketLeft' })).toBe('prev-conversation');
    expect(chord('}', { shiftKey: true, code: 'BracketRight' })).toBe('next-conversation');
    expect(chord('{', { shiftKey: true })).toBe('prev-conversation');
    expect(chord('C', { shiftKey: true })).toBe('workspace-conversation');
    // ⌃N is ⌘N only off a Mac (five asks A4; see the ⌃ test below).
    expect(chord('n', { metaKey: false, ctrlKey: true })).toBe(MOD === '⌘' ? null : 'new-conversation');
  });

  it('jumps to pages with contiguous digits and ignores unbound or unchorded keys', () => {
    expect(chord('1')).toBe('go-work');
    expect(chord('2')).toBe('go-tasks');
    expect(chord('3')).toBe('go-library');
    expect(chord('4')).toBe('go-automations');
    // Review keeps the rail's second row but takes the next free digit (P1-14).
    expect(chord('5')).toBe('go-review');
    expect(COMMAND_BY_ID['go-review'].page).toBe('review');
    expect(pageShortcut('review')).toBe(`${MOD}5`);
    expect(chord(',')).toBe('settings');
    expect(COMMAND_BY_ID['go-tasks'].page).toBe('tasks');
    expect(chord('6')).toBeNull();
    expect(chord('n', { altKey: true, code: 'KeyN' })).toBeNull();
    expect(shortcutFor({ key: 'n', metaKey: false, ctrlKey: false, altKey: false, shiftKey: false })).toBeNull();
    expect(pageShortcut('tasks')).toBe(`${MOD}2`);
  });

  it('gives every page key a command and binds no default chord twice', () => {
    for (const key of Object.values(PAGE_KEY)) expect(COMMANDS.some(c => c.default === `CmdOrCtrl+${key}`), key).toBe(true);
    const defaults = COMMANDS.map(c => c.default).filter(Boolean);
    expect(new Set(defaults).size).toBe(defaults.length);
    expect(new Set(COMMANDS.map(c => c.id)).size).toBe(COMMANDS.length);
    // A reserved chord only for its macOS meaning (⌘, Settings, ⌘N New).
    for (const c of COMMANDS) expect(reservedReason(c.default, c.id), c.id).toBeNull();
    expect(Object.keys(RESERVED).length).toBeGreaterThan(30);
  });

  it('on a Mac only ⌘ makes a chord: ⌃ keeps its Cocoa text keys (⌃K, ⌃A…)', () => {
    const ev = (o: Partial<{ metaKey: boolean; ctrlKey: boolean }>) => ({ key: 'k', code: 'KeyK', metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...o });
    expect(hasChordModifier(ev({ metaKey: true }), true)).toBe(true);
    expect(hasChordModifier(ev({ ctrlKey: true }), true)).toBe(false);
    expect(hasChordModifier(ev({ metaKey: true, ctrlKey: true }), true)).toBe(false);
    expect(chordFromEvent(ev({ ctrlKey: true }), true)).toBeNull();
    expect(chordFromEvent(ev({ metaKey: true }), true)).toBe('CmdOrCtrl+K');
    expect(hasChordModifier(ev({ ctrlKey: true }), false), 'Ctrl elsewhere').toBe(true);
  });

  it('leaves the system-wide commands to the backend', () => {
    expect(chord('T', { shiftKey: true, altKey: true })).toBeNull();
    // Off ⌘⇧T (five asks A4): every browser's Reopen closed tab.
    expect(keyFor('quick-capture')).toBe(`⌥⇧${MOD}T`);
  });
});

describe('the keymap and the backend agree', () => {
  it('keeps the native menu defaults equal to the webview table', () => {
    const items = [...APP_MENU_RS.matchAll(/\("([a-z-]+)", "[^"]+", (?:Some\("([^"]+)"\)|None)\)/g)];
    expect(items.length).toBeGreaterThan(15);
    for (const [, id, accel] of items) {
      // Every menu id is a shared action (the Rust test checks the same from its side).
      expect(ACTION_BY_MENU[id], id).toBeTruthy();
      const command = COMMAND_BY_ID[id];
      if (!command) { expect(accel, `menu item ${id} has a key but no Settings row`).toBeUndefined(); continue; }
      const def = command.default.startsWith('CmdOrCtrl+') ? command.default : undefined;
      expect(accel?.replace('\\\\', '\\'), `menu default for ${id}`).toBe(def);
    }
  });

  it('keeps the system-wide defaults and key set equal to shortcuts.rs', () => {
    for (const c of COMMANDS.filter(c => c.global)) expect(SHORTCUTS_RS).toContain(`("${c.id}", "${c.default}"`);
    const start = SHORTCUTS_RS.indexOf('const KEYS');
    const rsKeys = SHORTCUTS_RS.slice(start, SHORTCUTS_RS.indexOf('];', start));
    const parsed = [...rsKeys.matchAll(/"((?:[^"\\]|\\.)*)"/g)].map(m => m[1].replace('\\\\', '\\'));
    expect(parsed).toEqual([...CHORD_KEYS]);
  });

  it('lists the same commands in shortcuts.rs', () => {
    const start = SHORTCUTS_RS.indexOf('pub const COMMAND_IDS');
    const ids = [...SHORTCUTS_RS.slice(start, SHORTCUTS_RS.indexOf('];', start)).matchAll(/"([a-z-]+)"/g)].map(m => m[1]);
    expect(ids).toEqual(COMMANDS.map(c => c.id));
  });

  it('runs every non-global command through the shared action dispatcher the menu also uses', () => {
    for (const c of COMMANDS.filter(c => !c.global)) expect(ACTION_BY_KEY[c.id], c.id).toBeTruthy();
    expect(APP_MENU_TS).toContain('m.runMenuOrKey(id)');
  });

  it('reads hints from the one table in the palette and the keymap card', () => {
    // The palette's fixed rows take their chip from the shared action's key.
    expect(PALETTE_TSX).toContain('shortcut: actionHint(def)');
    // No hand-typed chord for a bound action (⌘S is the Library editor's own key).
    expect(OVERLAY_TSX).not.toMatch(/keys: \[`\$\{MOD\}[^S↵]/);
  });
});

describe('custom shortcuts and the off switch', () => {
  it('matches a changed key and drops the old one', () => {
    setShortcutSettings({ enabled: true, keys: { 'toggle-sidebar': 'CmdOrCtrl+Alt+Shift+J' } });
    expect(chord('b')).toBeNull();
    expect(chord('Dead', { altKey: true, shiftKey: true, code: 'KeyJ' })).toBe('toggle-sidebar');
    expect(keyFor('toggle-sidebar')).toBe(`⌥⇧${MOD}J`);
  });

  it('gives a command with no default a key', () => {
    setShortcutSettings({ enabled: true, keys: { 'new-task': 'CmdOrCtrl+F5' } });
    expect(chord('F5', { code: 'F5' })).toBe('new-task');
  });

  it('turns every shortcut off, hints included', () => {
    setShortcutSettings({ enabled: false, keys: {} });
    expect(chord('n')).toBeNull();
    expect(chord('1')).toBeNull();
    expect(keyFor('new-conversation')).toBe('');
    expect(pageShortcut('tasks')).toBe('');
    expect(activeChord('shortcuts')).toBe('');
    expect(activeChord('stop-run')).toBe('');
    expect(chordOf('new-conversation')).toBe('CmdOrCtrl+N');
  });

  it('runs nothing while Settings records a key', () => {
    setRecordingShortcut(true);
    expect(chord('n')).toBeNull();
  });

  it('moves a key from the command that had it, and drops a default back out of keys', () => {
    const { next, displaced } = assignChord(DEFAULT_SHORTCUTS, 'new-task', 'CmdOrCtrl+N');
    expect(displaced).toBe('new-conversation');
    expect(next.keys).toEqual({ 'new-task': 'CmdOrCtrl+N', 'new-conversation': '' });
    const back = assignChord(next, 'new-conversation', 'CmdOrCtrl+N');
    expect(back.displaced).toBe('new-task');
    expect(back.next.keys, 'both are on their defaults again').toEqual({});
    expect(assignChord(DEFAULT_SHORTCUTS, 'search', '').next.keys).toEqual({ search: '' });
  });

  it('records chords as the backend writes them', () => {
    const ev = (key: string, code: string, mods: Partial<{ metaKey: boolean; shiftKey: boolean; altKey: boolean }> = {}) =>
      chordFromEvent({ key, code, metaKey: true, ctrlKey: false, altKey: false, shiftKey: false, ...mods });
    expect(ev('j', 'KeyJ')).toBe('CmdOrCtrl+J');
    expect(ev('∆', 'KeyJ', { altKey: true })).toBe('CmdOrCtrl+Alt+J');
    expect(ev('{', 'BracketLeft', { shiftKey: true })).toBe('CmdOrCtrl+Shift+[');
    expect(ev('ArrowUp', 'ArrowUp')).toBe('CmdOrCtrl+ArrowUp');
    expect(ev('Enter', 'Enter')).toBeNull();
    expect(ev('Meta', 'MetaLeft')).toBeNull();
    expect(chordFromEvent({ key: 'j', code: 'KeyJ', metaKey: false, ctrlKey: false, altKey: false, shiftKey: false })).toBeNull();
    expect(displayChord('CmdOrCtrl+Shift+[')).toBe(`⇧${MOD}[`);
    expect(displayChord('Escape')).toBe('Esc');
    expect(displayChord('')).toBe('');
  });
});

describe('shortcut rules (P1-49)', () => {
  const press = (key: string, code: string, mods: Partial<{ shiftKey: boolean; altKey: boolean }> = {}) =>
    windowCommandFor({ key, code, metaKey: true, ctrlKey: false, altKey: false, shiftKey: false, ...mods })?.id ?? null;

  it('writes modifiers in macOS order, ⌃⌥⇧⌘, then the key', () => {
    expect(displayChord('CmdOrCtrl+Shift+N', true)).toBe('⇧⌘N');
    expect(displayChord('CmdOrCtrl+Alt+Shift+H', true)).toBe('⌥⇧⌘H');
    expect(displayChord('CmdOrCtrl+Alt+J', true)).toBe('⌥⌘J');
    expect(displayChord('CmdOrCtrl+K', true)).toBe('⌘K');
    expect(displayChord('CmdOrCtrl+Alt+Shift+H', false)).toBe('Ctrl+Alt+Shift+H');
  });

  it('runs a ⌥ chord saved for a command with no menu item (⌘⌥J for Jump)', () => {
    setShortcutSettings({ enabled: true, keys: { jump: 'CmdOrCtrl+Alt+J' } });
    expect(press('∆', 'KeyJ', { altKey: true })).toBe('jump');
    expect(press('j', 'KeyJ'), 'the old key is gone').toBeNull();
  });

  it('leaves a ⌥ chord to the menu item that has it, and to an in-surface key', () => {
    // The native menu runs its own item's key; the window running it too would run it twice.
    setShortcutSettings({ enabled: true, keys: { 'toggle-sidebar': 'CmdOrCtrl+Alt+Shift+J' } });
    expect(press('Dead', 'KeyJ', { altKey: true, shiftKey: true })).toBeNull();
    // ⌥⌘T makes a task from selected text (localKeys.ts) even if Jump is given it.
    setShortcutSettings({ enabled: true, keys: { jump: 'CmdOrCtrl+Alt+T' } });
    expect(press('†', 'KeyT', { altKey: true })).toBeNull();
    // Plain ⌘ chords are unchanged.
    expect(press('k', 'KeyK')).toBe('search');
    // The launch bundle's copy of the in-surface ⌥ chords matches localKeys.ts and shortcuts.rs.
    expect([...LOCAL_ALT_CHORDS].sort()).toEqual(LOCAL_COMMANDS.filter(c => c.chord.startsWith('CmdOrCtrl+Alt+')).map(c => c.chord).sort());
    const at = SHORTCUTS_RS.indexOf('pub const LOCAL_ALT');
    expect([...SHORTCUTS_RS.slice(at, SHORTCUTS_RS.indexOf('];', at)).matchAll(/"([^"]+)"/g)].map(m => m[1]).sort()).toEqual([...LOCAL_ALT_CHORDS].sort());
  });

  it('refuses a command an in-surface ⌥ chord, which the window would leave to its surface', () => {
    expect(localAltProblem('CmdOrCtrl+Alt+T')).toMatch(/make a task from the selection/);
    expect(localAltProblem('CmdOrCtrl+Alt+J')).toBeNull();
  });

  it('marks exactly the commands the native menu has no item for', () => {
    const items = new Set([...APP_MENU_RS.matchAll(/\(\s*"([a-z-]+)",\s*"[^"]+",\s*(?:Some\("[^"]+"\)|None),?\s*\)/g)].map(m => m[1]));
    expect(items.has('toggle-side-pane'), 'items written over several lines count').toBe(true);
    for (const c of COMMANDS.filter(c => !c.global)) expect(!c.noMenuItem, c.id).toBe(items.has(c.id));
  });

  it('asks ⌘⌥⇧ of a system-wide key', () => {
    expect(systemWideProblem('quick-capture', 'CmdOrCtrl+K')).toMatch(/⌥⇧⌘/);
    expect(systemWideProblem('main-agent', 'CmdOrCtrl+Alt+H')).toMatch(/⌥⇧⌘/);
    expect(systemWideProblem('quick-capture', 'CmdOrCtrl+Alt+Shift+K')).toBeNull();
    expect(systemWideProblem('quick-capture', '')).toBeNull();
    expect(systemWideProblem('search', 'CmdOrCtrl+K'), 'only system-wide commands').toBeNull();
  });

  it('refuses macOS Zoom, Cancel and the Touch Bar screenshot', () => {
    for (const c of ['CmdOrCtrl+Alt+8', 'CmdOrCtrl+Alt+=', 'CmdOrCtrl+Alt+-', 'CmdOrCtrl+Alt+\\', 'CmdOrCtrl+.', 'CmdOrCtrl+Shift+6']) {
      expect(reservedReason(c, 'jump'), c).toBeTruthy();
    }
  });

  it('puts no key sequence in aria-keyshortcuts (a space there means two separate keys)', () => {
    const sources = import.meta.glob('./**/*.tsx', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;
    const bad: string[] = [];
    for (const [file, text] of Object.entries(sources)) {
      for (const m of text.matchAll(/aria-keyshortcuts=(\{[^}]*\}|"[^"]*")/g)) {
        if (/goHint|\w \w/.test(m[1])) bad.push(`${file}: ${m[1]}`);
      }
    }
    expect(bad).toEqual([]);
  });
});

describe('keyInView (P0-10)', () => {
  const [inside, drawer, body, html] = [{}, {}, {}, {}];
  const root = { contains: (node: unknown) => node === inside };
  const page = { body, documentElement: html };
  it('takes keys aimed inside its root or at the page itself', () => {
    for (const target of [inside, body, html, null]) expect(keyInView(root, target as EventTarget | null, page)).toBe(true);
  });
  it('leaves a key aimed at a drawer or card beside it alone', () => {
    expect(keyInView(root, drawer as EventTarget, page)).toBe(false);
    expect(keyInView(undefined, drawer as EventTarget, page)).toBe(false);
  });
});
