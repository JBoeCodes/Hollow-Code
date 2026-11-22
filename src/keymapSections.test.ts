import { describe, expect, it } from 'vitest';
import { COMMANDS } from './shortcuts';
import { LOCAL_COMMANDS } from './localKeys';
import { keymapSections, pageKeys } from './keymapSections';

describe('keyboard shortcuts card (overnight F4)', () => {
  it('documents the existing task bulk-selection key in Lists', () => {
    expect(pageKeys('⌘').find(k=>k.section==='Lists'&&k.keys.includes('x')))
      .toEqual({section:'Lists',label:'select a task for bulk edit',keys:['x'],letters:['x']});
  });
  it('every shortcut in the registry has a label and a group', () => {
    for (const c of COMMANDS) {
      expect(c.label.trim(), `command ${c.id} has no label`).not.toBe('');
      expect(c.group.trim(), `command ${c.id} has no group`).not.toBe('');
    }
    for (const l of LOCAL_COMMANDS) {
      expect(l.label.trim(), `an in-surface key for ${l.surface} has no label`).not.toBe('');
      expect(l.chord, `"${l.label}" has no key`).not.toBe('');
    }
    for (const k of pageKeys('⌘')) {
      expect(k.label.trim(), `a ${k.section} key has no label`).not.toBe('');
      expect(k.keys.length, `"${k.label}" has no keys`).toBeGreaterThan(0);
      for (const l of k.letters ?? []) expect(k.keys, `"${k.label}" lists a letter it doesn't have`).toContain(l);
    }
  });

  it('builds the sections from the registries, keyless commands left out', () => {
    const commands = [
      { label: 'Tasks', group: 'Navigate', key: '⌘4' },
      { label: 'New task', group: 'Create', key: '' },
      { label: 'Command palette', group: 'View', key: '⌘K' },
      { label: 'COO', group: 'Anywhere', key: '⌘⌥⇧H' },
      { label: 'Stop the running turn', group: 'Chat', key: 'Esc' },
    ];
    const local = [{ surface: 'File editor', label: 'Save', key: '⌘S' }];
    const s = keymapSections(commands, pageKeys('⌘'), true, local);
    expect(s.map(x => x.title)).toEqual(['Navigate', 'Act', 'Lists', 'Suggestions (panel focused)', 'Notification history', 'Annotate', 'New project', 'Ask', 'Editing a note', 'Running', 'Chat', 'In a surface', 'Anywhere (global)']);
    expect(s[1].rows.map(r => r.label)).toEqual(['command palette']);
    expect(s.find(x => x.title === 'In a surface')!.rows).toEqual([{ label: 'save (file editor)', keys: ['⌘S'] }]);
    expect(s.find(x => x.title === 'Chat')!.rows.find(r => r.label === 'stop the running turn')?.keys, 'from COMMANDS').toEqual(['Esc']);
  });

  it('lists the go-to keys as g then a key, and drops them with shortcuts off (A4)', () => {
    const go = [{ label: 'Tasks', leader: 'g', key: 't' }, { label: 'Cleared', leader: 'g', key: '' }];
    const on = keymapSections([], pageKeys('⌘'), true, [], go);
    expect(on.find(x => x.title === 'Go to (press g, then…)')!.rows).toEqual([{ label: 'tasks', keys: ['g t'] }]);
    expect(keymapSections([], pageKeys('⌘'), false, [], go).some(x => x.title.startsWith('Go to'))).toBe(false);
  });

  it('with shortcuts off, letter keys go and arrows, Enter and send stay', () => {
    const s = keymapSections([], pageKeys('⌘'), false, [{ surface: 'Conversation', label: 'Find in the conversation', key: '⌘F' }]);
    expect(s.find(x => x.title === 'In a surface')!.rows.length, 'in-surface keys stay on').toBe(1);
    const suggestions = s.find(x => x.title === 'Suggestions (panel focused)')!;
    expect(suggestions.rows).toEqual([{ label: 'walk suggestions', keys: ['↑', '↓'] }]);
    expect(s.find(x => x.title === 'Lists')!.rows[0]).toEqual({ label: 'walk rows', keys: ['↑', '↓'] });
    expect(s.find(x => x.title === 'Notification history'), 'letter-only rows go').toBeUndefined();
    expect(s.find(x => x.title === 'Annotate')!.rows.some(r => r.keys.includes('←'))).toBe(true);
  });

  it('every page-key section is on the card (cross-PR review S2: Ask Hollow\'s rows had no section)', () => {
    const titles = new Set(keymapSections([], pageKeys('⌘'), true).map(x => x.title));
    for (const k of pageKeys('⌘')) expect(titles.has(k.section), k.section).toBe(true);
  });

  it('every command with a key reaches the card, whatever its group', () => {
    const withKeys = COMMANDS.filter(c => c.default).map(c => ({ label: c.label, group: c.group, key: c.default }));
    const shown = new Set(keymapSections(withKeys, pageKeys('⌘'), true).flatMap(sec => sec.rows.map(r => r.label)));
    for (const c of withKeys) expect(shown.has(c.label.toLowerCase()), `${c.label} (${c.group}) isn't on the card`).toBe(true);
  });
});
