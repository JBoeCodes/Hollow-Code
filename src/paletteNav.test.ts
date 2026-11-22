import { describe, expect, it } from 'vitest';
import PALETTE from './components/CommandPalette.tsx?raw';
import { backTarget, digitOfKey, digitOfMenuCommand, rowDigit, rowForDigit, splitActionPrefix, PALETTE_DIGITS } from './paletteNav';
import { COMMANDS, shortcutFor } from './shortcuts';
import { LOCAL_COMMANDS } from './localKeys';
import { PALETTE_SCOPES, inScope, wantsProjectFiles, wantsVaultSearch } from './paletteSearch';

const mod = (key: string, extra: Partial<{ altKey: boolean; shiftKey: boolean; ctrlKey: boolean; metaKey: boolean }> = {}) =>
  ({ key, code: /^[0-9]$/.test(key) ? `Digit${key}` : undefined, metaKey: true, ctrlKey: false, altKey: false, shiftKey: false, ...extra });

describe('the > actions prefix', () => {
  it('a leading > opens the actions scope and is not part of the query', () => {
    expect(splitActionPrefix('>')).toEqual({ actions: true, query: '' });
    expect(splitActionPrefix('>new task')).toEqual({ actions: true, query: 'new task' });
    expect(splitActionPrefix('> new task')).toEqual({ actions: true, query: 'new task' });
    expect(splitActionPrefix('  >x')).toEqual({ actions: true, query: 'x' });
  });
  it('a > later in the text is plain text', () => {
    expect(splitActionPrefix('a > b')).toEqual({ actions: false, query: 'a > b' });
    expect(splitActionPrefix('')).toEqual({ actions: false, query: '' });
  });
  it('the Actions chip lists actions, navigation and conversation actions, and nothing that needs a search', () => {
    expect(PALETTE_SCOPES.map(s => s.value)).toContain('actions');
    expect(inScope('actions', 'Actions')).toBe(true);
    expect(inScope('actions', 'Navigation')).toBe(true);
    expect(inScope('actions', 'Conversation')).toBe(true);
    expect(inScope('actions', 'Notes')).toBe(false);
    expect(inScope('actions', 'Settings')).toBe(false);
    expect(wantsVaultSearch('actions')).toBe(false);
    expect(wantsProjectFiles('actions')).toBe(false);
  });
});

describe('Backspace goes back one level', () => {
  it('only in an empty box', () => {
    expect(backTarget({ query: 'x', scope: 'actions', subpageFromRoot: false })).toBeNull();
    expect(backTarget({ query: '', scope: 'actions', subpageFromRoot: false })).toBe('to-all');
    expect(backTarget({ query: '', scope: 'notes', subpageFromRoot: false })).toBe('to-all');
  });
  it('is nothing at the root, and a sub-page opened from the root returns there first', () => {
    expect(backTarget({ query: '', scope: 'all', subpageFromRoot: false })).toBeNull();
    expect(backTarget({ query: '', scope: 'all', subpageFromRoot: true })).toBe('to-root');
    expect(backTarget({ query: 'a', scope: 'all', subpageFromRoot: true })).toBeNull();
  });
});

describe('palette digits', () => {
  it('rows 1 to 9 carry a digit in displayed order, the tenth none', () => {
    expect([0, 1, 8].map(rowDigit)).toEqual([1, 2, 9]);
    expect(rowDigit(PALETTE_DIGITS)).toBeNull();
    expect(rowDigit(-1)).toBeNull();
  });
  it('a digit chooses its row only when the list is that long', () => {
    expect(rowForDigit(1, 3)).toBe(0);
    expect(rowForDigit(3, 3)).toBe(2);
    expect(rowForDigit(4, 3)).toBeNull();
    expect(rowForDigit(0, 9)).toBeNull();
    expect(rowForDigit(9, 40)).toBe(8);
  });
  it('reads ⌘ plus one digit 1 to 9, and nothing with another modifier or while composing', () => {
    for (let d = 1; d <= 9; d++) expect(digitOfKey(mod(String(d)))).toBe(d);
    expect(digitOfKey(mod('0'))).toBeNull();
    expect(digitOfKey(mod('1', { shiftKey: true }))).toBeNull();
    expect(digitOfKey(mod('1', { altKey: true }))).toBeNull();
    expect(digitOfKey(mod('1', { metaKey: false }))).toBeNull();
    expect(digitOfKey({ ...mod('1'), isComposing: true })).toBeNull();
    expect(digitOfKey(mod('a'))).toBeNull();
  });
  it('the native page items carry their digits, so the palette answers the menu too', () => {
    expect([['go-work', 1], ['go-tasks', 2], ['go-library', 3], ['go-automations', 4], ['go-review', 5]].map(([id]) => digitOfMenuCommand(id as string)))
      .toEqual([1, 2, 3, 4, 5]);
    expect(digitOfMenuCommand('search')).toBeNull();
    expect(digitOfMenuCommand('new-conversation')).toBeNull();
  });
  it('stay palette-local: no app command or in-surface key binds ⌘6 to ⌘9, and the page digits are unchanged', () => {
    for (const d of ['6', '7', '8', '9']) {
      expect(COMMANDS.filter(c => c.default === `CmdOrCtrl+${d}`)).toEqual([]);
      expect(LOCAL_COMMANDS.filter(l => l.chord === `CmdOrCtrl+${d}`)).toEqual([]);
      expect(shortcutFor(mod(d)), `⌘${d} outside the palette`).toBeNull();
    }
    expect(shortcutFor(mod('1'))?.id).toBe('go-work');
    expect(shortcutFor(mod('4'))?.id).toBe('go-automations');
  });
});

describe('the palette wires them with visible routes', () => {
  it('reads the digit before the keymap, goes back on Backspace, and shows both in the footer', () => {
    expect(PALETTE).toContain('digitOfKey(e)');
    expect(PALETTE).toContain("e.key === 'Backspace'");
    expect(PALETTE).toContain('pick a row');
    expect(PALETTE).toContain('<kbd>&gt;</kbd> actions');
    expect(PALETTE).toContain('palette-row-digit');
  });
  it('has a Back button and an Actions chip as the click routes', () => {
    expect(PALETTE).toMatch(/<Button[^>]*onClick=\{\(\) => goBack\(true\)\}>Back<\/Button>/);
    expect(PALETTE_SCOPES.some(s => s.value === 'actions' && s.label === 'Actions')).toBe(true);
  });
  it('a held Backspace never goes back on its repeats, and the Back button clears typed text as it goes', () => {
    expect(PALETTE).toContain("e.key === 'Backspace' && !e.repeat");
    expect(PALETTE).toContain("query: button ? '' : query()");
    expect(PALETTE).toMatch(/setQuery\(''\);\s*if \(inputRef\) inputRef\.value = '';\s*setScope\('all'\)/);
  });
  it('Escape still closes at every depth', () => {
    expect(PALETTE).toMatch(/e\.key === 'Escape'\) \{\s*e\.preventDefault\(\);\s*dismiss\(\);/);
  });
});
