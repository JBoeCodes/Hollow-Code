import { describe, expect, it } from 'vitest';
import runSource from './actionRun.ts?raw';
import menuRs from '../src-tauri/src/app_menu.rs?raw';
import { ACTION_BY_ID, ACTION_DEFS, CONDITIONS, EFFECT_WORDS, actionLabel } from './actionDefs';
import { COMMANDS } from './shortcuts';
import { GO_TARGETS } from './keyboardLeader';

// Plan completion L4 (N9/N10): one typed definition per shared action, with
// its effect, condition, destination row and words; every adapter reads it.

/** ACTION_RUN's keys, read from actionRun.ts (it imports the stores, which
 *  this Node runner can't load). */
const runKeys = (() => {
  const body = runSource.slice(runSource.indexOf('export const ACTION_RUN'), runSource.indexOf('\n};', runSource.indexOf('export const ACTION_RUN')));
  return new Set([...body.matchAll(/^\s+'([a-z0-9-]+)': /gm)].map(m => m[1]));
})();

/** app_menu.rs ITEMS: (id, label). */
const menuItems = [...menuRs.slice(menuRs.indexOf('pub const ITEMS'), menuRs.indexOf('];', menuRs.indexOf('pub const ITEMS')))
  .matchAll(/\(\s*"([a-z0-9-]+)",\s*"([^"]+)"/g)].map(m => ({ id: m[1], label: m[2] }));

describe('shared action definitions', () => {
  it('opens project files and Changes in the Preview pane, with no workspace section actions (COO rebuild L4)', () => {
    expect(ACTION_BY_ID['open-project-files']).toMatchObject({ label: 'Open project files', route: 'workspace.files', when: 'workspace-open' });
    expect(ACTION_BY_ID['open-changes']).toMatchObject({ route: 'workspace.changes', when: 'build-workspace' });
    expect(ACTION_BY_ID['open-project-files']?.key).toBeUndefined();
    expect(ACTION_DEFS.filter(d => d.id.startsWith('open-workspace-'))).toEqual([]);
    expect(ACTION_BY_ID['make-canvas-plan']).toBeUndefined();
  });
  it('build identity is discoverable by version, commit, build and installed', () => {
    const copy = ACTION_BY_ID['copy-build'];
    expect(copy.palette).toBe('act:copy-build');
    for (const word of ['version', 'commit', 'build', 'installed']) expect(copy.keywords).toContain(word);
  });

  it('ids and each surface id are unique', () => {
    for (const k of ['id', 'palette', 'go', 'key', 'menu'] as const) {
      const ids = ACTION_DEFS.map(d => d[k]).filter(Boolean);
      expect(new Set(ids).size, k).toBe(ids.length);
    }
  });

  it('every action declares an effect and, when conditional, a known condition', () => {
    for (const d of ACTION_DEFS) {
      expect(Object.keys(EFFECT_WORDS), d.id).toContain(d.effect);
      if (d.when) expect(Object.keys(CONDITIONS), d.id).toContain(d.when);
    }
  });

  it('only actions that change, run or send can ask for confirmation', () => {
    expect(ACTION_DEFS.filter(d => d.confirm && ['navigate', 'view', 'draft'].includes(d.effect)).map(d => d.id)).toEqual([]);
    // Sending outside this Mac always confirms.
    expect(ACTION_DEFS.filter(d => d.effect === 'external' && !d.confirm).map(d => d.id)).toEqual([]);
  });

  it('one dispatcher runs every action, and nothing else', () => {
    expect(ACTION_DEFS.filter(d => !runKeys.has(d.id)).map(d => d.id)).toEqual([]);
    expect([...runKeys].filter(k => !ACTION_BY_ID[k])).toEqual([]);
  });

  it('every go target and ⌘ command is a shared action', () => {
    expect(GO_TARGETS.filter(g => !ACTION_DEFS.some(d => d.go === g.id)).map(g => g.id)).toEqual([]);
    expect(COMMANDS.filter(c => !ACTION_DEFS.some(d => d.key === c.id)).map(c => c.id)).toEqual([]);
    expect(ACTION_DEFS.filter(d => d.go && !GO_TARGETS.some(g => g.id === d.go)).map(d => d.id)).toEqual([]);
    expect(ACTION_DEFS.filter(d => d.key && !COMMANDS.some(c => c.id === d.key)).map(d => d.id)).toEqual([]);
  });

  it('the ⌘ keymap says what the action says', () => {
    for (const c of COMMANDS) expect(c.label, c.id).toBe(actionLabel(ACTION_DEFS.find(d => d.key === c.id)!, 'key'));
  });

  it('the macOS menu bar has exactly the actions declared for it, in their words', () => {
    expect(menuItems.length).toBeGreaterThan(20);
    expect(menuItems.map(m => m.id).sort()).toEqual(ACTION_DEFS.filter(d => d.menu).map(d => d.menu!).sort());
    for (const m of menuItems) expect(m.label, m.id).toBe(actionLabel(ACTION_DEFS.find(d => d.menu === m.id)!, 'menu'));
  });

  it('a surface’s own words are rare and named', () => {
    const own = ACTION_DEFS.flatMap(d => Object.keys(d.labels ?? {}).map(k => `${d.id}:${k}`));
    // Each is a deliberate exception: "Open …" in the palette names the
    // home a view lives in; the menu bar keeps macOS's "…" and title case.
    expect(own.length).toBeLessThanOrEqual(20);
  });

  it('⌘⇧M and View → COO open the COO page under their old id (run 15 P3)', () => {
    // The drawer is gone; the id stays so a custom binding in shortcuts.json
    // keeps working. The palette keeps one row for the page (act:main-agent).
    const d = ACTION_BY_ID['main-drawer'];
    expect(runSource).toMatch(/^\s+'main-drawer': \(\) => openMainAgent\(\),$/m);
    expect([d.key, d.menu, d.palette, d.route]).toEqual(['main-drawer', 'main-drawer', undefined, 'ask.main-agent']);
    expect(actionLabel(d, 'menu')).toBe('COO');
    expect(`${actionLabel(d, 'key')} ${d.keywords}`).not.toMatch(/drawer|beside/i);
  });

  it('the palette, keys and menu reach the same action ids as before (no lost accelerator)', () => {
    // A snapshot of the ways in, so dropping one is a visible change. Run 15
    // P1 removed Open Files, Open Logs and Open Browser with the side panel's
    // tabs (the Preview pane opens on a click; Logs is the tool ledger). P3
    // removed "Toggle Main agent drawer" (⌘⇧M opens the page; the palette's
    // row for it is Open Main agent). One Ask (2026-11-03) removed Ask
    // Hollow… with its dialog: Ask anything… (act:ask-vault) is the one Ask.
    // Overnight 11-08 S3 removed five palette rows: Today, Incoming,
    // Commitments, Weekly review and Agenda (55 to 50). S4 removed eight
    // more: What's new, the welcome tour, the workspace map, Runbooks,
    // dry-run tonight, archive suggestions, pressure-test and personas (50 to 42).
    // COO rebuild L4 removed the six Open <section> rows and Make a canvas view
    // of the plan with the workspace panel, and added Open project files (42 to 36).
    // The Notion removal (2026-11-10) took Add a canvas to a Notion page… (36 to 35).
    // Overnight 11-12 L2 added Library: Skills and Library: History (35 to 37),
    // L3 Library: All (38), L4 Library: Notes, Results and Recordings, whose
    // tabs left the strip for All (41); Undo chat action (3.1) makes 42;
    // Reopen closed view (t3code parity 4.6) makes 43; Toggle side chat (7.6) makes 44. Pull requests (7.3) makes 45.
    expect(ACTION_DEFS.filter(d => d.palette).length).toBe(45);
    expect(ACTION_DEFS.filter(d => d.go).length).toBe(GO_TARGETS.length);
    expect(ACTION_DEFS.filter(d => d.key).length).toBe(COMMANDS.length);
  });
});
