import { describe, expect, it } from 'vitest';
import type { TrackerState, TrackerViewRow } from './generated/ipc';
import {
  DEFAULT_VIEW, TAB_VIEWS, VIEW_KEY, applyView, chooseView, comparator, conflictCount, dueMatches, emptyFilters, filterOptions, groupRows,
  parseView, readSavedView, showConflicts, tracksOnly, writeSavedView, type ViewState,
} from './trackerViews';

const STATES: TrackerState[] = [
  { id: 'b', name: 'Backlog', kind: 'backlog', position: 0, color: null },
  { id: 't', name: 'Todo', kind: 'unstarted', position: 1, color: null },
  { id: 'p', name: 'In Progress', kind: 'started', position: 2, color: null },
  { id: 'd', name: 'Done', kind: 'completed', position: 3, color: null },
];
const row = (key: string, statusName: string, over: Partial<TrackerViewRow> = {}): TrackerViewRow => ({
  id: key.toLowerCase(), key, title: `Title ${key}`, statusName, statusKind: STATES.find(s => s.name === statusName)?.kind ?? 'unstarted',
  priority: 3, labels: [], project: null, projectId: null, due: null, assignee: null, assigneeId: null, estimate: null, cycle: null,
  milestone: null, updatedAt: '2026-11-10T00:00:00Z', url: '', path: null, lock: null, pending: false, conflict: false, archived: false, teamKey: 'ENG', ...over,
});
const view = (over: Partial<ViewState> = {}): ViewState => ({ ...DEFAULT_VIEW, filters: emptyFilters(), ...over });
const keys = (rows: TrackerViewRow[]) => rows.map(r => r.key);
const NOW = new Date(2026, 10, 10, 12);

const ROWS = [
  row('ENG-1', 'Todo', { assigneeId: 'me', assignee: 'Me', priority: 1, labels: ['Bug'], projectId: 'p1', project: 'One', due: '2026-11-09', cycle: 'Cycle 4' }),
  row('ENG-2', 'In Progress', { assigneeId: 'them', assignee: 'Them', priority: 2, projectId: 'p2', project: 'Two', due: '2026-11-12' }),
  row('ENG-3', 'Backlog', { priority: 0 }),
  row('ENG-4', 'Done', { assigneeId: 'me', assignee: 'Me', projectId: 'p1', project: 'One', due: '2026-11-10' }),
];

describe('Tasks views (B2)', () => {
  it('My issues keeps the viewer’s issues, and none without a viewer', () => {
    expect(keys(applyView(ROWS, view({ view: 'mine' }), 'me'))).toEqual(['ENG-1', 'ENG-4']);
    expect(applyView(ROWS, view({ view: 'mine' }), null)).toEqual([]);
  });

  it('Active keeps started and not started; Backlog the backlog; the by-views drop closed issues', () => {
    expect(keys(applyView(ROWS, view({ view: 'active' }), 'me'))).toEqual(['ENG-1', 'ENG-2']);
    expect(keys(applyView(ROWS, view({ view: 'backlog' }), 'me'))).toEqual(['ENG-3']);
    expect(keys(applyView(ROWS, view({ view: 'byProject' }), 'me'))).toEqual(['ENG-1', 'ENG-2', 'ENG-3']);
    expect(keys(applyView(ROWS, view({ view: 'all' }), 'me'))).toHaveLength(4);
  });

  it('each filter narrows, with none values for unassigned, no project and no cycle', () => {
    const f = (over: Partial<ViewState['filters']>) => view({ filters: { ...emptyFilters(), ...over } });
    expect(keys(applyView(ROWS, f({ status: ['Todo', 'Done'] }), null))).toEqual(['ENG-1', 'ENG-4']);
    expect(keys(applyView(ROWS, f({ priority: ['1', '2'] }), null))).toEqual(['ENG-1', 'ENG-2']);
    expect(keys(applyView(ROWS, f({ assignee: [''] }), null))).toEqual(['ENG-3']);
    expect(keys(applyView(ROWS, f({ label: ['Bug'] }), null))).toEqual(['ENG-1']);
    expect(keys(applyView(ROWS, f({ label: [''] }), null))).toEqual(['ENG-2', 'ENG-3', 'ENG-4']);
    expect(keys(applyView(ROWS, f({ project: ['p1', ''] }), null))).toEqual(['ENG-1', 'ENG-3', 'ENG-4']);
    expect(keys(applyView(ROWS, f({ cycle: ['Cycle 4'] }), null))).toEqual(['ENG-1']);
    expect(keys(applyView(ROWS, f({ due: ['overdue'] }), null, NOW))).toEqual(['ENG-1']);
    expect(keys(applyView(ROWS, f({ due: ['today', 'none'] }), null, NOW))).toEqual(['ENG-3', 'ENG-4']);
  });

  it('due filters read the local day', () => {
    expect(dueMatches('2026-11-17', 'week', NOW)).toBe(true);
    expect(dueMatches('2026-11-18', 'week', NOW)).toBe(false);
    expect(dueMatches(null, 'set', NOW)).toBe(false);
    expect(dueMatches('garbage', 'overdue', NOW)).toBe(false);
  });

  it('choosing a view resets the grouping and keeps the filters and order', () => {
    const start = view({ ordering: 'due', filters: { ...emptyFilters(), label: ['Bug'] } });
    const next = chooseView(start, 'byAssignee');
    expect(next).toMatchObject({ view: 'byAssignee', grouping: 'assignee', ordering: 'due' });
    expect(next.filters.label).toEqual(['Bug']);
    expect(chooseView(next, 'all').grouping).toBe('status');
  });

  it('groups by assignee with yours first and nobody last, by priority urgent first', () => {
    const open = applyView(ROWS, view({ view: 'byAssignee' }), 'me');
    const byPerson = groupRows(open, { grouping: 'assignee', ordering: 'priority' }, STATES, 'me');
    expect(byPerson.map(g => g.name)).toEqual(['Me', 'Them', 'No assignee']);
    expect(byPerson[0]).toMatchObject({ by: 'assignee', person: 'Me' });
    const byPriority = groupRows(ROWS, { grouping: 'priority', ordering: 'priority' }, STATES, null);
    expect(byPriority.map(g => g.name)).toEqual(['Urgent', 'High', 'Medium', 'No priority']);
    const byProject = groupRows(ROWS, { grouping: 'project', ordering: 'priority' }, STATES, null);
    expect(byProject.map(g => g.name)).toEqual(['One', 'Two', 'No project']);
    expect(groupRows(ROWS, { grouping: 'status', ordering: 'priority' }, STATES, null).map(g => g.name)).toEqual(['In Progress', 'Todo', 'Backlog', 'Done']);
  });

  it('orders by due date (none last) and by last updated', () => {
    const a = row('ENG-1', 'Todo', { due: '2026-11-20', updatedAt: '2026-11-01T00:00:00Z' });
    const b = row('ENG-2', 'Todo', { due: null, updatedAt: '2026-11-09T00:00:00Z' });
    const c = row('ENG-3', 'Todo', { due: '2026-11-12', updatedAt: '2026-11-05T00:00:00Z' });
    expect(keys([a, b, c].sort(comparator('due')))).toEqual(['ENG-3', 'ENG-1', 'ENG-2']);
    expect(keys([a, b, c].sort(comparator('updated')))).toEqual(['ENG-2', 'ENG-3', 'ENG-1']);
  });

  it('local tasks hide only under a filter an issue alone has', () => {
    expect(tracksOnly(emptyFilters())).toBe(false);
    expect(tracksOnly({ ...emptyFilters(), label: ['Bug'], due: ['overdue'] })).toBe(false);
    expect(tracksOnly({ ...emptyFilters(), assignee: [''] })).toBe(true);
  });

  it('filter options come from the team, with the none choices first', () => {
    const meta = { team: { id: 't', key: 'ENG', name: 'ENG' }, states: STATES, labels: [{ id: 'l', name: 'Bug', color: null, group: null }],
      members: [{ id: 'me', name: 'Me', email: null }], cycles: [{ id: 'c', number: 4, name: null, startsAt: '', endsAt: '' }],
      milestones: [], projects: [{ id: 'p1', name: 'One' }], viewer: null };
    expect(filterOptions('status', meta).map(o => o.label)).toEqual(['In Progress', 'Todo', 'Backlog', 'Done']);
    expect(filterOptions('assignee', meta).map(o => o.label)).toEqual(['No assignee', 'Me']);
    expect(filterOptions('cycle', meta).map(o => o.value)).toEqual(['', 'Cycle 4']);
  });

  it('a saved view reads back; anything unknown falls back', () => {
    const saved = view({ view: 'backlog', grouping: 'priority', ordering: 'updated', filters: { ...emptyFilters(), due: ['week', 'soon'] } });
    expect(parseView(JSON.stringify(saved))).toMatchObject({ view: 'backlog', grouping: 'priority', ordering: 'updated', filters: { due: ['week'] } });
    expect(parseView('{"view":"board","grouping":"weird"}')).toMatchObject({ view: 'all', grouping: 'status', ordering: 'priority' });
    expect(parseView('not json')).toBeNull();
  });

  it('the Projects tab is a view that persists like the others (B4)', () => {
    expect(parseView(JSON.stringify(view({ view: 'projects' })))?.view).toBe('projects');
    expect(chooseView(view(), 'projects').grouping).toBe('status');
  });

  it('this window’s view wins; a new window starts from the last one chosen', () => {
    const store = () => { const m = new Map<string, string>(); return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) }; };
    const session = store(), local = store();
    writeSavedView(view({ view: 'active' }), session, local);
    expect(readSavedView(session, local).view).toBe('active');
    local.setItem(VIEW_KEY, JSON.stringify(view({ view: 'backlog' })));
    expect(readSavedView(session, local).view).toBe('active');
    expect(readSavedView(store(), local).view).toBe('backlog');
    expect(readSavedView(null, null).view).toBe('all');
  });
});

describe('Conflicts in the page (T2, design section 7)', () => {
  const rows = [
    row('ENG-1', 'Todo', { conflict: true }),
    row('ENG-2', 'In Progress'),
    row('ENG-4', 'Done', { conflict: true, archived: true }),
  ];

  it('keeps only issues with an open conflict, closed ones too, under the filters', () => {
    expect(keys(applyView(rows, view({ view: 'conflicts' }), 'me'))).toEqual(['ENG-1', 'ENG-4']);
    expect(keys(applyView(rows, view({ view: 'conflicts', filters: { ...emptyFilters(), status: ['Todo'] } }), 'me'))).toEqual(['ENG-1']);
    expect(applyView([row('ENG-2', 'Todo')], view({ view: 'conflicts' }), 'me')).toEqual([]);
  });

  it('counts conflicts, and shows the control only while there are some or it is chosen', () => {
    expect(conflictCount(rows)).toBe(2);
    expect(conflictCount([])).toBe(0);
    expect(showConflicts(view(), 2)).toBe(true);
    expect(showConflicts(view(), 0)).toBe(false);
    expect(showConflicts(view({ view: 'conflicts' }), 0)).toBe(true);
  });

  it('is not a tab, groups by status and persists like the other views', () => {
    expect(TAB_VIEWS.map(v => v.id)).not.toContain('conflicts');
    expect(TAB_VIEWS.map(v => v.id)).toContain('projects');
    expect(chooseView(view({ grouping: 'assignee' }), 'conflicts')).toMatchObject({ view: 'conflicts', grouping: 'status' });
    expect(parseView(JSON.stringify(view({ view: 'conflicts' })))?.view).toBe('conflicts');
  });
});
