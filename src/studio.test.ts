import { describe, expect, it } from 'vitest';
import { latestWorkspaceSession, orderWorkspaces, studioDate } from './studio';
import type { WorkWorkspace, WorkspaceSessions } from './workspaceTypes';
import type { AgentSessionSummary } from './types';
const workspace = (id: string, title: string, modified: string): WorkWorkspace => ({ id, title, modified, status: 'active', path: id, cwd: id });
const session = (key: string, when: string) => ({ key, lastActivityAt: when }) as AgentSessionSummary;
const group = (sessions: AgentSessionSummary[]): WorkspaceSessions => ({ workspaceId: 'a', total: sessions.length, sessions, changes: [] });

describe('studio workspace ordering', () => {
  it('resumes the latest actual conversation, even when the provider list is out of order', () => {
    expect(latestWorkspaceSession(group([session('old', '2026-01-01'), session('new', '2026-09-27'), session('invalid', 'invalid')]))?.key).toBe('new');
    expect(latestWorkspaceSession(undefined)).toBeUndefined();
  });
  it('uses conversation activity instead of a newer brief edit, with brief fallback for untouched work', () => {
    const list = [workspace('a', 'Alpha', '2026-09-27'), workspace('b', 'Beta', '2026-09-20')];
    expect(orderWorkspaces(list, {a: group([session('a1','2026-08-01')])}, [], 'recent').map(w=>w.id)).toEqual(['b','a']);
    expect(list.map(w=>w.id)).toEqual(['a','b']);
  });
  it('keeps pins first, ignores deleted pins, and sorts names within each group', () => {
    const list = [workspace('z', 'Zebra', 'invalid'), workspace('a', 'Alpha', 'invalid'), workspace('b', 'Beta', 'invalid')];
    expect(orderWorkspaces(list, {}, ['z','gone'], 'name').map(w=>w.id)).toEqual(['z','a','b']);
    expect(orderWorkspaces(list, {}, [], 'recent').map(w=>w.id)).toEqual(['a','b','z']);
  });
  it('keeps pins in pin order, whatever the sort (the sidebar Pinned group order)', () => {
    const list = [workspace('z', 'Zebra', 'invalid'), workspace('a', 'Alpha', 'invalid'), workspace('b', 'Beta', 'invalid')];
    expect(orderWorkspaces(list, {}, ['b','z'], 'name').map(w=>w.id)).toEqual(['b','z','a']);
    expect(orderWorkspaces(list, {}, ['z','b'], 'recent').map(w=>w.id)).toEqual(['z','b','a']);
  });
});

describe('studioDate (ST2: one formatter for every row)', () => {
  it('formats like toLocaleDateString with the same options, and says when a date is unreadable', () => {
    for (const iso of ['2026-10-14T05:00:00Z', '2026-01-02T23:59:00Z', '2025-12-31T12:00:00Z']) {
      expect(studioDate(iso)).toBe(new Date(iso).toLocaleDateString(undefined, { month: 'short', day: 'numeric' }));
    }
    expect(studioDate('not a date')).toBe('Date unavailable');
  });
});
