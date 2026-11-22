import { describe, expect, it } from 'vitest';
import { childCount, childFoldLabel, descendantState, flatDescendants, groupState, splitChildren } from './railChildren';
import type { AgentSessionSummary } from './types';
import type { SessionState } from './sessionIndicator';

const s = (key: string, over: Partial<AgentSessionSummary> = {}) => ({ key, title: key, createdAt: `2030-01-0${key.length}T00:00:00Z`, forkKind: null, parentKey: null, ...over }) as AgentSessionSummary;

describe('child chats nest under their lead', () => {
  it('nests listed subagent chats oldest first and keeps every other chat on top', () => {
    const lead = s('lead'), b = s('b2', { forkKind: 'subagent', parentKey: 'lead', createdAt: '2030-02-02T00:00:00Z' }), a = s('a1', { forkKind: 'subagent', parentKey: 'lead', createdAt: '2030-02-01T00:00:00Z' });
    const other = s('other', { forkKind: 'offline', parentKey: 'lead' });
    const { top, kids } = splitChildren([lead, b, a, other]);
    expect(top.map(x => x.key)).toEqual(['lead', 'other']);
    expect(kids.get('lead')!.map(x => x.key)).toEqual(['a1', 'b2']);
  });
  it('keeps a subagent chat as a row when its parent is not listed or names itself', () => {
    expect(splitChildren([s('x', { forkKind: 'subagent', parentKey: 'gone' })]).top.map(x => x.key)).toEqual(['x']);
    expect(splitChildren([s('y', { forkKind: 'subagent', parentKey: 'y' })]).top.map(x => x.key)).toEqual(['y']);
  });
  it('a parent shows its most urgent descendant state when it is idle itself', () => {
    const { kids } = splitChildren([s('p'), s('c', { forkKind: 'subagent', parentKey: 'p' }), s('g', { forkKind: 'subagent', parentKey: 'c' })]);
    const states: Record<string, SessionState> = { p: 'idle', c: 'queued', g: 'waiting' };
    expect(groupState('p', kids, k => states[k] ?? 'idle')).toBe('waiting');
    states.p = 'running';
    expect(groupState('p', kids, k => states[k] ?? 'idle')).toBe('waiting');
    states.g = 'idle'; states.c = 'idle'; states.p = 'idle';
    expect(groupState('p', kids, k => states[k] ?? 'idle')).toBe('idle');
  });
  it('counts every level and words the fold', () => {
    const { kids } = splitChildren([s('p'), s('c', { forkKind: 'subagent', parentKey: 'p' }), s('g', { forkKind: 'subagent', parentKey: 'c' })]);
    expect(childCount('p', kids)).toBe(2);
    expect(childFoldLabel(1, false)).toBe('Show 1 subagent chat');
    expect(childFoldLabel(2, true)).toBe('Hide 2 subagent chats');
  });
  it('puts a cycle back on top so no chat disappears', () => {
    const { top, kids } = splitChildren([s('a', { forkKind: 'subagent', parentKey: 'b' }), s('b', { forkKind: 'subagent', parentKey: 'a' }), s('ok')]);
    expect(top.map(x => x.key).sort()).toContain('ok');
    const reached = new Set(top.map(x => x.key));
    for (const t of top) for (const c of flatDescendants(t.key, kids)) reached.add(c.key);
    expect([...reached].sort()).toEqual(['a', 'b', 'ok']);
  });
  it('lists deep descendants flat and reads only descendants for the state', () => {
    const { kids } = splitChildren([s('p'), s('c', { forkKind: 'subagent', parentKey: 'p' }), s('g', { forkKind: 'subagent', parentKey: 'c' }), s('h', { forkKind: 'subagent', parentKey: 'g' })]);
    expect(flatDescendants('p', kids).map(x => x.key)).toEqual(['c', 'g', 'h']);
    const states: Record<string, SessionState> = { p: 'running', c: 'idle', g: 'idle', h: 'idle' };
    expect(descendantState('p', kids, k => states[k] ?? 'idle')).toBe('idle');
    states.h = 'waiting';
    expect(descendantState('p', kids, k => states[k] ?? 'idle')).toBe('waiting');
  });
});
