import { describe, expect, it } from 'vitest';
import type { SyncClash } from './generated/ipc';
import { choiceBlocked, choiceConfirm, clashCountText, clashKey, clashSummary, clashTitle, clashesOf, rootLabel, sideCaption } from './syncClashes';

const clash = (over: Partial<SyncClash> = {}): SyncClash => ({
  root: 'personal', path: 'me/Identity.md', copy: 'me/Identity (from mac-a).md', mineMachine: 'mac-b', theirsMachine: 'mac-a',
  at: '2026-11-15T09:00:00Z', replaceable: true, editing: false, changed: false,
  mine: { lines: ['title: b', 'shared'], totalLines: 2, unreadable: null },
  theirs: { lines: ['title: a', 'shared'], totalLines: 2, unreadable: null },
  onlyMine: 1, onlyTheirs: 1, ...over,
});

describe('sync clash cards', () => {
  it('names the file, the root and how many wait', () => {
    expect(clashTitle(clash())).toBe('Identity.md');
    expect(rootLabel('vault')).toBe('Vault');
    expect([0, 1, 3].map(clashCountText)).toEqual(['', '1 clash', '3 clashes']);
    expect(clashesOf([clash(), clash({ root: 'vault' }), clash()], 'personal')).toBe(2);
    expect(clashesOf(undefined, 'vault')).toBe(0);
  });

  it('keys a card by root, file and copy, so two clashes of one file stay apart', () => {
    const second = clash({ copy: 'me/Identity (from mac-a 2).md' });
    expect(clashKey(clash())).toBe('personal:me/Identity.md:me/Identity (from mac-a).md');
    expect(clashKey(second)).not.toBe(clashKey(clash()));
    expect(clashKey(clash({ root: 'vault' }))).not.toBe(clashKey(clash()));
  });

  it('summarises what differs without a diff, and says when a side cannot show', () => {
    expect(clashSummary(clash())).toBe('1 line only in this Mac’s version, 1 line only in the other’s.');
    expect(clashSummary(clash({ onlyMine: 0, onlyTheirs: 0 }))).toMatch(/Same lines/);
    expect(clashSummary(clash({ theirs: { lines: [], totalLines: 0, unreadable: 'too large to show' } }))).toMatch(/can’t be shown/);
    expect(clashSummary(clash({ mine: null, theirs: null }))).toBe('');
    expect(sideCaption('This Mac', { lines: ['a', 'b'], totalLines: 40, unreadable: null })).toBe('This Mac: first 2 of 40 lines');
    expect(sideCaption('mac-a', { lines: [], totalLines: 0, unreadable: 'missing' })).toBe('mac-a: missing');
  });

  it('blocks Keep theirs on a changed file and both keeps on a folder clash', () => {
    expect(choiceBlocked(clash(), 'keep_theirs')).toBeNull();
    expect(choiceBlocked(clash({ changed: true }), 'keep_theirs')).toMatch(/changed since the clash/);
    expect(choiceBlocked(clash({ changed: true }), 'keep_mine')).toBeNull();
    expect(choiceBlocked(clash({ replaceable: false }), 'keep_mine')).toMatch(/folder/);
    expect(choiceBlocked(clash({ replaceable: false }), 'keep_both')).toBeNull();
  });

  it('confirms only the choices that move a file, in words without dashes', () => {
    expect(choiceConfirm(clash(), 'keep_mine')?.body).toContain('Identity (from mac-a).md');
    expect(choiceConfirm(clash(), 'keep_theirs')?.title).toBe('Keep mac-a’s version?');
    expect(choiceConfirm(clash(), 'keep_both')).toBeNull();
    expect(choiceConfirm(clash(), 'edit')).toBeNull();
    for (const c of ['keep_mine', 'keep_theirs'] as const) expect(JSON.stringify(choiceConfirm(clash(), c))).not.toMatch(/[–—]/);
  });
});
