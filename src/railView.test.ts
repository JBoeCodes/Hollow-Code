import { describe, expect, it } from 'vitest';
import { DEFAULT_RAIL_VIEW, addSavedView, applyRailView, groupRailRows, isDefaultView, matchesFilter, readRailView, readSavedRailViews, removeSavedView, viewSummary, type RailFacts } from './railView';
import type { AgentSessionSummary } from './types';

const s = (key: string, over: Partial<AgentSessionSummary> = {}) => ({ key, title: key, agent: 'claude', createdAt: '2030-01-01T00:00:00Z', lastActivityAt: '2030-01-01T00:00:00Z', ...over }) as AgentSessionSummary;
const quiet: RailFacts = { state: 'idle', marks: [] };

describe('rail view', () => {
  it('filters by what needs you, what works, unread and pull requests', () => {
    expect(matchesFilter('all', quiet)).toBe(true);
    expect(matchesFilter('attention', { state: 'waiting', marks: [] })).toBe(true);
    expect(matchesFilter('attention', { state: 'idle', marks: ['input'] })).toBe(true);
    expect(matchesFilter('attention', { state: 'running', marks: [] })).toBe(false);
    expect(matchesFilter('attention', { state: 'paused', marks: [] })).toBe(true);
    expect(matchesFilter('working', { state: 'queued', marks: [] })).toBe(true);
    expect(matchesFilter('unread', { state: 'idle', marks: ['unread'] })).toBe(true);
    expect(matchesFilter('pr', { state: 'idle', marks: ['pr'] })).toBe(true);
    expect(matchesFilter('pr', quiet)).toBe(false);
  });
  it('sorts by activity, title or start and keeps equal rows in order', () => {
    const rows = [s('b', { lastActivityAt: '2030-01-02T00:00:00Z', createdAt: '2030-01-01T00:00:00Z' }), s('a', { lastActivityAt: '2030-01-03T00:00:00Z', createdAt: '2030-01-05T00:00:00Z' }), s('c', { lastActivityAt: '2030-01-03T00:00:00Z', createdAt: '2030-01-03T00:00:00Z' })];
    const keys = (sort: 'recent' | 'title' | 'created') => applyRailView(rows, { ...DEFAULT_RAIL_VIEW, sort }, () => quiet).map(x => x.key);
    expect(keys('recent')).toEqual(['a', 'c', 'b']);
    expect(keys('title')).toEqual(['a', 'b', 'c']);
    expect(keys('created')).toEqual(['a', 'c', 'b']);
  });
  it('groups by agent and by day', () => {
    const now = new Date(2030, 0, 10, 12).getTime();
    const rows = [s('a', { agent: 'claude', lastActivityAt: new Date(2030, 0, 10, 9).toISOString() }), s('b', { agent: 'codex', lastActivityAt: new Date(2030, 0, 9, 9).toISOString() }), s('c', { agent: 'claude', lastActivityAt: new Date(2030, 0, 2, 9).toISOString() })];
    expect(groupRailRows(rows, 'agent', now).map(g => [g.label, g.rows.map(r => r.key)])).toEqual([['Claude', ['a', 'c']], ['Codex', ['b']]]);
    expect(groupRailRows(rows, 'day', now).map(g => g.label)).toEqual(['Today', 'Yesterday', 'Older']);
    expect(groupRailRows([...rows].reverse(), 'day', now).map(g => g.label)).toEqual(['Today', 'Yesterday', 'Older']);
    expect(groupRailRows(rows, 'none', now)).toHaveLength(1);
    expect(groupRailRows([], 'none', now)).toEqual([]);
  });
  it('saves, renames by the same name or view, caps and removes views; the default is never saved', () => {
    const v = { sort: 'title', filter: 'unread', group: 'none' } as const;
    expect(addSavedView([], 'x', DEFAULT_RAIL_VIEW)).toEqual([]);
    expect(addSavedView([], '  ', v)).toEqual([]);
    const one = addSavedView([], 'Unread A-Z', v);
    expect(one).toHaveLength(1);
    expect(addSavedView(one, 'Renamed', v)).toEqual([{ ...one[0], name: 'Renamed' }]);
    let many = one;
    for (let i = 0; i < 20; i++) many = addSavedView(many, `view ${i}`, { sort: 'created', filter: 'pr', group: i % 2 ? 'day' : 'agent' });
    expect(many.length).toBeLessThanOrEqual(12);
    expect(removeSavedView(one, one[0].id)).toEqual([]);
  });
  it('reads damaged storage as the default', () => {
    expect(readRailView('not json')).toEqual(DEFAULT_RAIL_VIEW);
    expect(readRailView(JSON.stringify({ sort: 'x', filter: 'unread' }))).toEqual({ ...DEFAULT_RAIL_VIEW, filter: 'unread' });
    expect(readSavedRailViews('[1,{"id":"a","name":"n","sort":"title"}]')).toEqual([{ id: 'a', name: 'n', ...DEFAULT_RAIL_VIEW, sort: 'title' }]);
    expect(isDefaultView(DEFAULT_RAIL_VIEW)).toBe(true);
    expect(viewSummary({ sort: 'title', filter: 'pr', group: 'agent' })).toBe('With a pull request · Title, A to Z · By agent');
    expect(viewSummary(DEFAULT_RAIL_VIEW)).toBe('Default view');
  });
});
