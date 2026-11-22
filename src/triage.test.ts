import { describe, expect, it } from 'vitest';
import type { NeedsYouItem } from './workspaceTypes';
import {
  buildTriage, countByKind, dayGroup, filterTriage, groupByDay, needsKind, nextSelection, recordingAt, suggestText, taskFrom,
  triageActions, triageKey, TRIAGE_FILTERS as incomingFilters, type TriageArrivals, type TriageItem,
} from './triage';

const needs = (over: Partial<NeedsYouItem>): NeedsYouItem => ({
  id: 'review:r1', kind: 'review', sourceId: 'r1', title: 'Learn: Pat prefers Monday', detail: 'Learning', state: 'pending',
  at: '2026-10-10T12:00:00Z', workspaceId: null, sessionKey: null, path: null, category: 'learning', link: null, ...over,
});

const arrivals = (over: Partial<TriageArrivals> = {}): TriageArrivals => ({
  imports: [{ id: 'import:tips', title: 'Tip pooling', path: 'library/imports/tips', files: ['library/imports/tips/a.md'], fileCount: 3, at: '2026-10-10T15:00:00Z', excerpt: 'Pooling splits tips.' }],
  tasks: [{ id: 'task:tasks/x.md', path: 'tasks/x.md', title: 'Call Dana', from: 'Notion', at: '2026-10-09T10:00:00Z', project: 'Q3', due: null }],
  state: { dismissed: {}, filed: {} },
  unavailable: [],
  ...over,
});

const base = () => ({
  needs: [needs({}), needs({ id: 'question:q1', kind: 'question', sourceId: 'q1', title: 'Which lot?', state: 'pending', at: '2026-10-10T16:00:00Z', sessionKey: 's-1' })],
  recordings: [{ id: 'rec1', name: 'TipFlow sync', date: '2026-10-10T11:00:00Z', duration: '32m' }],
  recordingLinks: {},
  arrivals: arrivals(),
  state: { dismissed: {}, filed: {} },
});

describe('buildTriage', () => {
  it('drops a row once a task was made from it here (overnight S7)', () => {
    const src = { ...base(), state: { dismissed: {}, filed: {}, tasked: { 'import:tips': { task: 'vault:tasks/tip-pooling.md', at: '2026-10-13T07:00:00Z' } } } };
    expect(buildTriage(src).map(i => i.id)).not.toContain('import:tips');
    expect(buildTriage(base()).map(i => i.id)).toContain('import:tips');
  });

  it('merges every source newest first', () => {
    const items = buildTriage(base());
    expect(items.map(i => i.id)).toEqual(['question:q1', 'import:tips', 'review:r1', 'recording:rec1', 'task:tasks/x.md']);
    expect(items.map(i => i.kind)).toEqual(['ask', 'import', 'review', 'recording', 'task']);
  });

  it('leaves out what was dismissed or filed here, linked recordings and dead questions', () => {
    const src = base();
    src.state = { dismissed: { 'import:tips': 'x' }, filed: { 'task:tasks/x.md': { workspaceId: 'w', at: 'x' } } };
    src.recordingLinks = { rec1: { workspaceId: 'w', eventId: null, eventTitle: null, name: null, linkedAt: 'x', keptCopy: null } } as never;
    src.needs = [needs({}), needs({ id: 'question:q2', kind: 'question', state: 'stale' })];
    expect(buildTriage(src).map(i => i.id)).toEqual(['review:r1']);
  });

  it('leaves restart offers, conflicts and conflict copies to Waiting on you', () => {
    expect(needsKind('interrupted')).toBeNull();
    expect(needsKind('conflict')).toBeNull();
    expect(needsKind('copies')).toBeNull();
    expect(needsKind('canvas')).toBe('ask');
    expect(needsKind('approval')).toBe('ask');
  });

  it('never lists one id twice', () => {
    const src = base();
    src.needs = [needs({}), needs({})];
    expect(buildTriage(src).filter(i => i.id === 'review:r1')).toHaveLength(1);
  });
});

describe('filters and groups', () => {
  it('counts by kind and filters', () => {
    const items = buildTriage(base());
    const counts = countByKind(items);
    expect(counts.all).toBe(5);
    expect(counts.recording).toBe(1);
    expect(filterTriage(items, 'import').map(i => i.id)).toEqual(['import:tips']);
  });

  it('puts rows under day headers', () => {
    const now = new Date(2026, 9, 10, 18, 0);
    expect(dayGroup(new Date(2026, 9, 10, 1).toISOString(), now)).toBe('Today');
    expect(dayGroup(new Date(2026, 9, 9, 23).toISOString(), now)).toBe('Yesterday');
    expect(dayGroup(new Date(2026, 9, 6).toISOString(), now)).toBe('Earlier this week');
    expect(dayGroup(new Date(2026, 8, 1).toISOString(), now)).toBe('Older');
    expect(dayGroup(null, now)).toBe('Undated');
    const groups = groupByDay(buildTriage(base()), new Date('2026-10-10T20:00:00Z'));
    expect(groups[0].items.length).toBeGreaterThan(0);
  });

  it('reads Plaud dates with a space', () => {
    expect(recordingAt('2026-10-10 09:00')).not.toBeNull();
    expect(recordingAt('yesterday-ish')).toBeNull();
    expect(recordingAt(null)).toBeNull();
  });
});

describe('actions', () => {
  const items = () => Object.fromEntries(buildTriage(base()).map(i => [i.kind, i])) as Record<string, TriageItem>;

  it('files only recordings and imports, and only with a workspace', () => {
    expect(triageActions(items().recording, null).file.reason).toMatch(/Pick a workspace/);
    expect(triageActions(items().recording, 'TipFlow').file).toMatchObject({ label: 'File into TipFlow', reason: null });
    expect(triageActions(items().import, 'Payroll').file.reason).toBeNull();
    expect(triageActions(items().review, 'X').file.reason).not.toBeNull();
    expect(triageActions(items().task, 'X').file.reason).toMatch(/project/);
  });

  it('accepts only review items; a Result is accepted where it is read, and questions in their conversation', () => {
    expect(triageActions(items().review, null).accept.reason).toBeNull();
    expect(triageActions(items().ask, null).accept.reason).toMatch(/conversation/);
    // P0-10: Incoming never shows a Result's content, so it never accepts one.
    const result: TriageItem = { id: 'result:a', kind: 'result', title: 'Deck', at: null, workspaceId: 'w', needs: needs({ kind: 'result' }) };
    expect(triageActions(result, null).accept.reason).toMatch(/Open it \(Enter\)/);
    const changed = { ...result, needs: needs({ kind: 'result', state: 'changed' }) } as TriageItem;
    expect(triageActions(changed, null).accept.reason).toMatch(/Open it \(Enter\)/);
  });

  it('dismiss means Reject for a review item and Mark read for a failure', () => {
    expect(triageActions(items().review, null).dismiss.label).toBe('Reject');
    const failure: TriageItem = { id: 'failure:n', kind: 'failure', title: 'Sync failed', at: null, workspaceId: null, needs: needs({ kind: 'failure' }) };
    expect(triageActions(failure, null).dismiss.label).toBe('Mark read');
    expect(triageActions(items().import, null).dismiss.label).toBe('Dismiss');
  });

  it('a suggested-task review points t at Accept; a task is a task already', () => {
    const proposal: TriageItem = { id: 'review:t', kind: 'review', title: 'Do X', at: null, workspaceId: null, needs: needs({ category: 'meeting_task' }) };
    expect(triageActions(proposal, null).task.reason).toMatch(/Accept/);
    expect(triageActions(items().task, null).task.reason).toMatch(/already/);
  });

  it('a task made from a row is keyed to the row, so a second t makes no twin', () => {
    const t = taskFrom(items().recording);
    expect(t.title).toBe('Follow up: TipFlow sync');
    expect(t.idempotencyKey).toBe('triage:recording:rec1');
    expect(t.origin.startsWith('triage:')).toBe(true);
    expect(taskFrom(items().import).description).toContain('library/imports/tips');
  });

  it('suggestion text carries the import excerpt', () => {
    expect(suggestText(items().import)).toContain('Pooling splits tips.');
  });
});

describe('keys', () => {
  const k = (key: string, mods: Partial<{ metaKey: boolean; ctrlKey: boolean; altKey: boolean }> = {}) => ({ key, metaKey: false, ctrlKey: false, altKey: false, ...mods });

  it('maps bare letters, digits, arrows, Enter and Escape', () => {
    expect(triageKey(k('j'), true)).toEqual({ kind: 'move', dir: 1 });
    expect(triageKey(k('ArrowUp'), true)).toEqual({ kind: 'move', dir: -1 });
    expect(triageKey(k('e'), true)).toEqual({ kind: 'action', action: 'file' });
    expect(triageKey(k('a'), true)).toEqual({ kind: 'action', action: 'accept' });
    expect(triageKey(k('t'), true)).toEqual({ kind: 'action', action: 'task' });
    expect(triageKey(k('x'), true)).toEqual({ kind: 'action', action: 'dismiss' });
    expect(triageKey(k('Enter'), true)).toEqual({ kind: 'action', action: 'open' });
    expect(triageKey(k('u'), true)).toEqual({ kind: 'undo' });
    expect(triageKey(k('2'), true)).toEqual({ kind: 'filter', filter: 'recording' });
    expect(triageKey(k('Escape'), true)).toEqual({ kind: 'close' });
  });

  it('never takes a chord (no macOS shortcut), and letters go with shortcuts off', () => {
    for (const key of ['a', 'e', 'x', 'j', '1']) {
      expect(triageKey(k(key, { metaKey: true }), true)).toBeNull();
      expect(triageKey(k(key, { ctrlKey: true }), true)).toBeNull();
      expect(triageKey(k(key, { altKey: true }), true)).toBeNull();
    }
    expect(triageKey(k('a'), false)).toBeNull();
    expect(triageKey(k('ArrowDown'), false)).toEqual({ kind: 'move', dir: 1 });
    expect(triageKey(k('9'), true)).toBeNull();
  });
});

describe('nextSelection', () => {
  it('selects the row that took the place, else the one before', () => {
    expect(nextSelection(['a', 'b', 'c'], ['a', 'c'], 'b')).toBe('c');
    expect(nextSelection(['a', 'b', 'c'], ['a', 'b'], 'c')).toBe('b');
    expect(nextSelection(['a'], [], 'a')).toBeNull();
  });
});
it('names source arrivals without changing their stored kind or keyboard position', () => {
  expect(incomingFilters[2]).toEqual({ id: 'import', label: 'Sources' });
});
