import { describe, expect, it } from 'vitest';
import APP_MENU_RS from '../src-tauri/src/app_menu.rs?raw';
import SHORTCUTS_RS from '../src-tauri/src/shortcuts.rs?raw';
import { MAC_RESERVED, STANDARD_MEANING, reservedReason } from './macReserved';
import { COMMANDS, shortcutFor } from './shortcuts';
import { localAltProblem } from './settings/shortcutRules';
import { LOCAL_COMMANDS } from './localKeys';
import { GO_TARGETS } from './keyboardLeader';

// Five asks A4, Alex: "make sure the shortcuts don't overlap with mac
// shortcuts". This file fails on any default binding macOS owns.
describe('no Hollow shortcut takes a macOS one', () => {
  it('lists every chord the backlog reserves', () => {
    const backlog = [
      'Q', 'W', 'H', 'M', 'Tab', 'Space', '`', ',', 'C', 'V', 'X', 'Z', 'Shift+Z', 'A', 'S', 'F', 'G', 'Shift+G', 'P', 'N', 'O', 'T', 'R',
      'Alt+H', 'Alt+M', 'Alt+Escape', 'Alt+D', 'Shift+3', 'Shift+4', 'Shift+5', 'Shift+/',
      ...Array.from({ length: 12 }, (_, i) => `F${i + 1}`),
    ];
    for (const k of backlog) expect(MAC_RESERVED[`CmdOrCtrl+${k}`], k).toBeTruthy();
  });

  it('no default app, system-wide or menu chord is reserved, except for its own macOS meaning', () => {
    for (const c of COMMANDS) expect(reservedReason(c.default, c.id), `${c.id} on ${c.default}`).toBeNull();
    for (const [chord, owner] of Object.entries(STANDARD_MEANING)) {
      expect(COMMANDS.find(c => c.id === owner)?.default, owner).toBe(chord);
      expect(reservedReason(chord, 'new-task')).toBeTruthy();
    }
    // The native menu's own accelerators (app_menu.rs, items spanning lines too).
    const menu = [...APP_MENU_RS.matchAll(/\(\s*"([a-z-]+)",\s*"[^"]+",\s*Some\("([^"]+)"\),?\s*\)/g)];
    expect(menu.length).toBeGreaterThanOrEqual(14);
    // Review's ⌘5 (P1-14) is no macOS chord, and the menu gives it the same key.
    expect(COMMANDS.find(c => c.id === 'go-review')?.default).toBe('CmdOrCtrl+5');
    expect(reservedReason('CmdOrCtrl+5', 'go-review')).toBeNull();
    expect(menu.find(m => m[1] === 'go-review')?.[2]).toBe('CmdOrCtrl+5');
    for (const m of menu) expect(reservedReason(m[2], m[1]), `${m[1]} on ${m[2]}`).toBeNull();
  });

  it('a chord is always ⌘ (+⌥/⇧) + one key: no ⌥-letter, ⌥-digit, ⌃⌥ or ⌃-arrow chord can exist', () => {
    for (const c of COMMANDS.filter(c => c.default && c.default !== '?' && c.default !== 'Escape')) {
      expect(c.default, c.id).toMatch(/^CmdOrCtrl\+(Alt\+)?(Shift\+)?[^+]+$/);
    }
    for (const c of LOCAL_COMMANDS) expect(c.chord).toMatch(/^CmdOrCtrl\+/);
  });

  it('shortcuts.rs refuses the same chords (an agent can’t set one either)', () => {
    const block = (name: string) => { const at = SHORTCUTS_RS.indexOf(name); return SHORTCUTS_RS.slice(at, SHORTCUTS_RS.indexOf('];', at)); };
    // Rust writes ⌘⌥\ as "CmdOrCtrl+Alt+\\".
    const rs = [...block('pub const MAC_RESERVED').matchAll(/"((?:[^"\\]|\\.)*)"/g)].map(m => m[1].replace(/\\\\/g, '\\')).sort();
    const ts = Object.keys(MAC_RESERVED).filter(c => !/\+F\d+$/.test(c)).sort();
    expect(rs).toEqual(ts);
    const std = [...block('pub const STANDARD_MEANING').matchAll(/\("([^"]+)", "([a-z-]+)"\)/g)].map(m => [m[1], m[2]]);
    expect(Object.fromEntries(std)).toEqual(STANDARD_MEANING);
  });

  it('every F-key is refused with any modifiers', () => {
    expect(reservedReason('CmdOrCtrl+F5', 'search')).toBeTruthy();
    expect(reservedReason('CmdOrCtrl+Alt+Shift+F12', 'search')).toBeTruthy();
    expect(reservedReason('CmdOrCtrl+F', 'search')).toBe('Find');
  });

  it('go-to keys are single letters or digits after g, so they never meet a ⌘ chord', () => {
    for (const t of GO_TARGETS) expect(t.key, t.id).toMatch(/^[a-z0-9]$/);
    expect(new Set(GO_TARGETS.map(t => t.key)).size).toBe(GO_TARGETS.length);
  });
});

// 0.1 (overnight 11-20): the whole default chord table of the reference client (a design
// study, MIT, read 2026-11-19 at t3code `packages/shared/src/keybindings.ts`), as a ban
// fixture. Every chord has a verdict, so a later lane cannot land a default that collides
// with a macOS chord just because the reference used it:
//   reserved  macOS owns it; no Hollow command may default to it (reservedReason or the
//             in-surface ⌥ rule refuses it);
//   owned     Hollow already binds it, to the named command or in-surface key;
//   free      no Hollow default may take it unless this row is changed on purpose, with
//             a visible click path, in the same PR.
type Verdict = 'reserved' | 'altlocal' | `owned:${string}` | `local:${string}` | 'free';
const REFERENCE_CHORDS: readonly [string, Verdict][] = [
  ['CmdOrCtrl+B', 'owned:toggle-sidebar'], ['CmdOrCtrl+F6', 'reserved'], ['CmdOrCtrl+Shift+F6', 'reserved'],
  ['F6', 'free'], ['Shift+F6', 'free'],
  ['CmdOrCtrl+[', 'owned:go-back'], ['CmdOrCtrl+]', 'owned:go-forward'], ['CmdOrCtrl+J', 'owned:jump'],
  ['CmdOrCtrl+Alt+B', 'free'], ['CmdOrCtrl+Shift+B', 'free'], ['CmdOrCtrl+T', 'reserved'],
  ['CmdOrCtrl+D', 'free'], ['CmdOrCtrl+Shift+D', 'owned:dictate'], ['CmdOrCtrl+N', 'owned:new-conversation'],
  ['CmdOrCtrl+W', 'reserved'], ['CmdOrCtrl+Shift+T', 'free'], ['CmdOrCtrl+Shift+J', 'free'],
  ['CmdOrCtrl+R', 'reserved'], ['CmdOrCtrl+L', 'local:Go to the address bar'],
  ['CmdOrCtrl+=', 'free'], ['CmdOrCtrl++', 'free'], ['CmdOrCtrl+-', 'free'], ['CmdOrCtrl+0', 'free'],
  ['CmdOrCtrl+F', 'reserved'], ['CmdOrCtrl+K', 'owned:search'], ['CmdOrCtrl+P', 'reserved'],
  ['CmdOrCtrl+Shift+F', 'free'], ['CmdOrCtrl+U', 'free'],
  ['CmdOrCtrl+Alt+A', 'altlocal'], ['CmdOrCtrl+Alt+Shift+A', 'free'], ['CmdOrCtrl+Alt+Shift+T', 'owned:quick-capture'],
  ['CmdOrCtrl+S', 'reserved'], ['CmdOrCtrl+Shift+Enter', 'free'], ['Alt+ArrowUp', 'free'],
  ['CmdOrCtrl+Enter', 'free'], ['CmdOrCtrl+Alt+Enter', 'free'],
  ['CmdOrCtrl+Shift+O', 'free'], ['CmdOrCtrl+Shift+N', 'owned:new-note'], ['CmdOrCtrl+Alt+N', 'free'],
  ['CmdOrCtrl+Shift+M', 'owned:main-drawer'], ['CmdOrCtrl+Shift+H', 'free'], ['CmdOrCtrl+Shift+E', 'free'],
  ['CmdOrCtrl+Shift+A', 'free'], ['CmdOrCtrl+Shift+X', 'free'], ['CmdOrCtrl+Shift+G', 'reserved'],
  ['CmdOrCtrl+Shift+L', 'free'], ['CmdOrCtrl+Shift+K', 'owned:search-chats'],
  ['CmdOrCtrl+Shift+ArrowUp', 'free'], ['CmdOrCtrl+Shift+ArrowDown', 'free'],
  ['CmdOrCtrl+O', 'reserved'], ['CmdOrCtrl+Shift+[', 'owned:prev-conversation'], ['CmdOrCtrl+Shift+]', 'owned:next-conversation'],
  ['CmdOrCtrl+Shift+C', 'owned:workspace-conversation'], ['CmdOrCtrl+Shift+S', 'free'], ['CmdOrCtrl+Shift+P', 'owned:search-alt'],
  ['CmdOrCtrl+Z', 'reserved'],
  ['CmdOrCtrl+1', 'owned:go-work'], ['CmdOrCtrl+2', 'owned:go-tasks'], ['CmdOrCtrl+3', 'owned:go-library'],
  ['CmdOrCtrl+4', 'owned:go-automations'], ['CmdOrCtrl+5', 'owned:go-review'],
  ['CmdOrCtrl+6', 'free'], ['CmdOrCtrl+7', 'free'], ['CmdOrCtrl+8', 'free'], ['CmdOrCtrl+9', 'free'],
  ['CmdOrCtrl+Shift+1', 'free'], ['CmdOrCtrl+Shift+2', 'free'], ['CmdOrCtrl+Shift+3', 'reserved'], ['CmdOrCtrl+Shift+4', 'reserved'],
];

describe('the reference client chord table is banned wherever macOS owns the chord', () => {
  const defaults = new Map<string, string[]>();
  for (const c of COMMANDS) if (c.default) defaults.set(c.default, [...(defaults.get(c.default) ?? []), c.id]);

  it('has one verdict per chord', () => {
    expect(new Set(REFERENCE_CHORDS.map(([c]) => c)).size).toBe(REFERENCE_CHORDS.length);
  });

  for (const [chord, verdict] of REFERENCE_CHORDS) {
    it(`${chord}: ${verdict}`, () => {
      const ids = defaults.get(chord) ?? [];
      if (verdict === 'reserved') {
        expect(reservedReason(chord, 'probe'), chord).toBeTruthy();
        expect(ids, `no default on ${chord}`).toEqual([]);
      } else if (verdict === 'altlocal') {
        expect(localAltProblem(chord), chord).toBeTruthy();
        expect(ids).toEqual([]);
      } else if (verdict === 'free') {
        expect(ids, `${chord} is unbound on purpose; change the row with a click path`).toEqual([]);
        expect(LOCAL_COMMANDS.filter(l => l.chord === chord).map(l => l.label), chord).toEqual([]);
      } else if (verdict.startsWith('owned:')) {
        expect(ids, chord).toEqual([verdict.slice(6)]);
        expect(reservedReason(chord, verdict.slice(6)), chord).toBeNull();
      } else {
        expect(LOCAL_COMMANDS.some(l => l.chord === chord && l.label === verdict.slice(6)), chord).toBe(true);
      }
    });
  }

  it('no default chord, app or in-surface, equals a reserved reference chord by another route', () => {
    for (const [chord, verdict] of REFERENCE_CHORDS) {
      if (verdict !== 'reserved') continue;
      for (const l of LOCAL_COMMANDS.filter(l => l.chord === chord)) {
        // A surface may use a reserved chord only for its own macOS meaning (Save in the file editor, Find in a conversation).
        expect(['Save', 'Find in the conversation', 'Reload'], `${l.surface}: ${l.label}`).toContain(l.label);
      }
    }
  });
});

describe('palette digit hints are palette-local', () => {
  // The palette may label rows with a digit (open palette: ⌘1 to ⌘9 picks row n). That is a key the
  // open palette reads itself. It never becomes an app shortcut: only the page digits are global.
  const ev = (key: string) => ({ key, metaKey: true, ctrlKey: false, altKey: false, shiftKey: false });

  it('binds only the page digits globally', () => {
    const digits = COMMANDS.filter(c => /^CmdOrCtrl\+[0-9]$/.test(c.default)).map(c => c.default).sort();
    expect(digits).toEqual(['CmdOrCtrl+1', 'CmdOrCtrl+2', 'CmdOrCtrl+3', 'CmdOrCtrl+4', 'CmdOrCtrl+5']);
    for (const d of ['0', '6', '7', '8', '9']) expect(shortcutFor(ev(d)), `⌘${d} outside the palette`).toBeNull();
  });

  it('keeps the shifted digits for the screenshot chords macOS owns', () => {
    for (const d of ['3', '4', '5', '6']) expect(MAC_RESERVED[`CmdOrCtrl+Shift+${d}`], `⌘⇧${d}`).toBeTruthy();
  });

  it('never lists a palette digit as an in-surface command', () => {
    expect(LOCAL_COMMANDS.filter(l => /^CmdOrCtrl\+[0-9]$/.test(l.chord))).toEqual([]);
  });
});
