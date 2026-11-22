import { describe, expect, it } from 'vitest';
import { EFFECT_WORDS } from './actionDefs';
import { joinsRecent, paletteDefaultOrder, paletteEffectLine, type DefaultRow } from './paletteDefault';

// The rows the palette offers before anything is typed, while a turn runs in
// a workspace (stop-run available; Make a canvas view of the plan went with
// the curated canvas views, COO rebuild L4).
const row = (id: string, group: string): DefaultRow => ({ id, group });
const BUSY: DefaultRow[] = [
  row('nav:tasks', 'Navigation'), row('nav:library', 'Navigation'),
  row('act:new-conversation', 'Actions'), row('act:clear-debug-logs', 'Actions'), row('act:new-note', 'Actions'),
  row('act:stop-run', 'Conversation'), row('act:switch-model', 'Conversation'), row('act:open-files', 'Conversation'),
  row('conversation:s-1', 'Conversations'), row('conversation:s-2', 'Conversations'),
];
const ids = (rows: DefaultRow[]) => rows.map(r => r.id);
const recentIds = (rows: DefaultRow[]) => ids(rows.filter(r => r.group === 'Recent'));

describe('the palette before anything is typed (P0-02)', () => {
  it('never puts Stop the running turn on row 0 while a turn runs', () => {
    const list = paletteDefaultOrder(BUSY, [], 's-1');
    expect(list[0].id).not.toBe('act:stop-run');
    // Nor any row that acts without asking.
    expect(joinsRecent(list[0].id)).toBe(true);
  });

  it('keeps acting rows off row 0 even when they are the only Conversation rows', () => {
    const only = BUSY.filter(r => r.group !== 'Conversation' || r.id === 'act:stop-run');
    const list = paletteDefaultOrder(only, [], null);
    expect(list[0].id).not.toBe('act:stop-run');
    // Still offered, after the rows that only show or open something.
    expect(ids(list)).toContain('act:stop-run');
    expect(ids(list).indexOf('act:stop-run')).toBeGreaterThan(ids(list).indexOf('nav:library'));
  });

  it('leaves run and write rows out of Recent, and keeps a navigation row there', () => {
    const list = paletteDefaultOrder(BUSY, ['act:stop-run', 'act:clear-debug-logs', 'nav:tasks'], 's-1');
    expect(recentIds(list)).toEqual(['nav:tasks']);
    expect(list[0].id).toBe('nav:tasks');
    // Each still lists once, in its own group.
    expect(ids(list).filter(id => id === 'act:clear-debug-logs')).toHaveLength(1);
    expect(ids(list).filter(id => id === 'act:stop-run')).toHaveLength(1);
  });

  it('records only rows that ask first or change nothing', () => {
    expect(joinsRecent('act:clear-debug-logs')).toBe(false);
    expect(joinsRecent('act:stop-run')).toBe(false);
    // A confirm row waits for the person's answer, so it may lead.
    expect(joinsRecent('act:conflict-copies')).toBe(true);
    expect(joinsRecent('nav:tasks')).toBe(true);
    expect(joinsRecent('act:open-files')).toBe(true);
    expect(joinsRecent('conversation:s-1')).toBe(true);
  });

  it('puts this conversation’s view rows before its acting ones', () => {
    const list = ids(paletteDefaultOrder(BUSY, [], 's-1'));
    expect(list.slice(0, 3)).toEqual(['act:switch-model', 'act:open-files', 'act:stop-run']);
    expect(list.indexOf('act:new-conversation')).toBeLessThan(list.indexOf('act:clear-debug-logs'));
    // The conversation on screen isn't offered as a place to go.
    expect(list).not.toContain('conversation:s-1');
  });

  it('tags run and write rows with what they do', () => {
    expect(paletteEffectLine('act:stop-run')).toBe(EFFECT_WORDS.run);
    expect(paletteEffectLine('act:clear-debug-logs')).toBe(EFFECT_WORDS.write);
    expect(paletteEffectLine('act:open-files')).toBeUndefined();
    expect(paletteEffectLine('nav:tasks')).toBeUndefined();
  });
});
