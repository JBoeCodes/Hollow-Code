import { describe, expect, it } from 'vitest';
import { activityGroups, railGroups, railWorkspaceOrder } from './railWorkspaces';
import type { SessionState } from './sessionIndicator';

describe('sidebar activity groups', () => {
  const row = (id: string, state: SessionState = 'idle') => ({ id, state });
  const group = (rows: ReturnType<typeof row>[]) => activityGroups(rows, r => r.state, r => r.id === 'current', 1);
  it('keeps attention ahead of recent and never caps it', () => {
    const rows = [row('old'), row('extra'), row('needs', 'waiting'), row('error', 'failed'), row('cut', 'interrupted')];
    expect(group(rows).active.map(r => r.id)).toEqual(['needs', 'error', 'cut', 'old']);
  });
  it('gives running and queued rows one working home but keeps the current row visible', () => {
    const rows = [row('running', 'running'), row('current', 'running'), row('queued', 'queued'), row('idle')];
    const result = group(rows);
    expect(result.active.map(r => r.id)).toEqual(['current', 'idle']);
    expect(result.working.map(r => r.id)).toEqual(['running', 'queued']);
    expect(new Set([...result.active, ...result.working]).size).toBe(4);
  });
  it('returns a working row to attention when it needs an answer', () => {
    const rows = [row('first'), row('worker', 'running')];
    expect(group(rows).working.map(r => r.id)).toEqual(['worker']);
    rows[1].state = 'waiting';
    expect(group(rows).active.map(r => r.id)).toEqual(['worker', 'first']);
    expect(group(rows).working).toEqual([]);
  });
  it('keeps paused rows beyond the recent cap reachable for resume', () => {
    const rows = [row('first'), row('second'), row('paused', 'paused')];
    expect(group(rows).active.map(r => r.id)).toEqual(['paused', 'first']);
    expect(group(rows).working).toEqual([]);
  });
  it('keeps the current idle row past the cap without duplicating it', () => {
    const rows = [row('first'), row('second'), row('current')];
    expect(group(rows).active.map(r => r.id)).toEqual(['first', 'current']);
  });
});

const w = (id: string, modified = '2026-09-01T00:00:00Z') => ({ id, status: 'active', modified });

describe('rail workspace order', () => {
  it('lists only workspaces with a conversation, never every active one', () => {
    const all = [w('idle-a', '2026-09-25'), w('used', '2026-08-01'), w('talked', '2026-09-10'), w('idle-b')];
    expect(railWorkspaceOrder(all, ['used'], [], ['talked']).map(x => x.id)).toEqual(['used', 'talked']);
  });

  it('puts used workspaces first, then others with conversations by modified time', () => {
    const all = [w('old', '2026-09-01'), w('new', '2026-09-20'), w('used', '2026-08-01')];
    expect(railWorkspaceOrder(all, ['used'], [], ['old', 'new', 'used']).map(x => x.id)).toEqual(['used', 'new', 'old']);
  });

  it('leaves out hidden workspaces and ids that no longer exist', () => {
    const all = [w('a'), w('b'), w('c')];
    expect(railWorkspaceOrder(all, ['gone', 'b', 'b'], ['c'], ['a', 'c']).map(x => x.id)).toEqual(['b', 'a']);
  });

  it('leaves out archived workspaces, used or with conversations (P1-38)', () => {
    const all = [w('a'), { ...w('b'), status: 'Archived' }, { ...w('c'), status: 'archived' }, w('d')];
    expect(railWorkspaceOrder(all, ['b', 'a'], [], ['c', 'd']).map(x => x.id)).toEqual(['a', 'd']);
  });

  it('is empty when nothing has a conversation', () => {
    expect(railWorkspaceOrder([w('a'), w('b')], [], [])).toEqual([]);
  });
});

describe('rail tree order (next-level P02)', () => {
  const w = (id: string) => ({ id, modified: '' });
  it('puts pins first, in pin order, even without a conversation; skips gone pins', () => {
    const all = [w('a'), w('b'), w('c'), w('d')];
    const groups = railGroups([w('a'), w('b')], all, ['d', 'gone', 'b']);
    // Pinned in pin order; a pinned workspace is listed once, in Pinned.
    expect(groups.pinned.map(x => x.id)).toEqual(['d', 'b']);
    expect(groups.rest.map(x => x.id)).toEqual(['a']);
    expect(railGroups([w('a')], all, [])).toEqual({ pinned: [], rest: [w('a')] });
  });
});
