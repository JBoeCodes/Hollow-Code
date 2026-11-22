import { afterEach, describe, expect, it } from 'vitest';
import { DEFAULT_SHORTCUTS, goHint, setShortcutSettings } from './shortcuts';
import { keymapSections, pageKeys } from './keymapSections';
import paletteSource from './components/CommandPalette.tsx?raw';
import prepPackSource from './components/PrepPackDialog.tsx?raw';
import dictationSource from './store/dictation.ts?raw';
import overlaysSource from './store/overlays.ts?raw';
import todayStoreSource from './store/todayBrief.ts?raw';
import appSource from './App.tsx?raw';
import goToSource from './goTo.ts?raw';
import runSource from './actionRun.ts?raw';
import { ACTION_BY_PALETTE } from './actionDefs';
import missionSource from './components/MissionControl.tsx?raw';

// Overnight 10-11 S2: the cross-PR review of tonight's 18 PRs. One check per
// interaction it found between features that were each reviewed alone.
describe('cross-PR review S2', () => {
  afterEach(() => setShortcutSettings(DEFAULT_SHORTCUTS));

  it('go-to hints follow the keys in effect, and vanish with shortcuts off', () => {
    expect(goHint('tasks')).toBe('g t');
    setShortcutSettings({ enabled: true, keys: {}, leader: { tasks: '', library: 'z' } });
    expect(goHint('tasks'), 'a cleared key is not advertised').toBe('');
    expect(goHint('library')).toBe('g z');
    setShortcutSettings({ ...DEFAULT_SHORTCUTS, enabled: false });
    expect(goHint('tasks')).toBe('');
  });

  it('the palette writes no go-to hint as text, and shows the COO and dictate chords', () => {
    expect(paletteSource).not.toMatch(/shortcut: 'g [a-z0-9]'/);
    // Every fixed row's chip is its shared action's key or go keys, as set now.
    expect(paletteSource).toContain('shortcut: actionHint(def)');
    expect(ACTION_BY_PALETTE['act:main-agent'].key).toBe('main-agent');
    expect(ACTION_BY_PALETTE['act:dictate'].key).toBe('dictate');
  });

  it('a prep pack closes itself, and opening a note leaves it', () => {
    // Today was cut (overnight 11-08 S3). The pack no longer closes that screen.
    expect(prepPackSource).toContain('const leave = () => { closePrepPack(); };');
  });

  it('dictation behind a full-screen view types only into a focused field', () => {
    const at = dictationSource.indexOf('if (modalOpen()) return null;');
    expect(at).toBeGreaterThan(0);
    expect(at, 'before the composer fallback').toBeLessThan(dictationSource.indexOf("textarea[aria-label=\"Chat message\"]"));
  });

  it('opening the palette closes the keys card', () => {
    const open = overlaysSource.slice(overlaysSource.indexOf('export function openCommandPalette'));
    expect(open.slice(0, open.indexOf('\n}'))).toContain('setShortcutsOverlayOpen(false)');
    expect(appSource).not.toContain('scheduleWelcome(');
  });

  it("the keys card has a section for every view's keys", () => {
    const titles = keymapSections([], pageKeys('⌘'), true).map(s => s.title);
    for (const t of ['Ask', 'Editing a note', 'Running', 'Annotate']) {
      expect(titles, t).toContain(t);
    }
  });

  it("Today reads the shared commitments snapshot, so a kept one leaves Today's count", () => {
    expect(todayStoreSource).toContain('enableCommitmentsResource();');
    expect(todayStoreSource).not.toContain('api.listCommitments()');
  });

  it('the shell, the action runner and go-to do not open Incoming', () => {
    // Incoming was cut (overnight 11-08 S3). Nothing in the shell opens it.
    expect(appSource).not.toMatch(/store\/triage'/);
    expect(runSource).not.toMatch(/store\/triage/);
    expect(goToSource).not.toMatch(/store\/triage'/);
  });

  it('motion and shortcuts-off gates hold in the views that missed them', () => {
    expect(missionSource).toContain('if (e.key.length === 1 && !shortcutsEnabled()) return;');
  });
});
