// The Tasks page's views, filters, grouping and ordering (overhaul B2, board
// S3b, design section 12): the pure parts behind `pages/tasks/ViewBar.tsx`
// and `pages/Tasks.tsx`. A view is a preset (which issues, how they group);
// the filter bar narrows it; Display picks the grouping and the order. The
// whole choice persists per window (sessionStorage), and a new window starts
// from the last one chosen anywhere (localStorage). Replaced the Notion-era
// saved-filter store and the List/Board layout (11-10).

import type { TrackerState, TrackerStateKind, TrackerTeamMeta, TrackerViewRow } from './generated/ipc';
import { KIND_ORDER, PRIORITY_ORDER, compareRows, groupByStatus, isClosedKind, priorityLabel, type IssueGroup } from './trackerTasks';

export type ViewId = 'mine' | 'all' | 'active' | 'backlog' | 'byProject' | 'byAssignee' | 'projects' | 'conflicts';
export type Grouping = 'status' | 'assignee' | 'project' | 'priority';
export type Ordering = 'priority' | 'due' | 'updated';
export type DueFilter = 'overdue' | 'today' | 'week' | 'set' | 'none';
export type FilterField = 'status' | 'priority' | 'assignee' | 'label' | 'project' | 'cycle' | 'due';

/** Each field's chosen values (empty: not filtered). Status holds state
 *  names, priority its numbers as text, assignee and project ids ('' for
 *  none), label and cycle names ('' for no cycle), due one `DueFilter`. */
export type Filters = Record<FilterField, string[]>;

export interface ViewState {
  view: ViewId;
  grouping: Grouping;
  ordering: Ordering;
  filters: Filters;
}

export const VIEWS: readonly { id: ViewId; label: string; grouping: Grouping; kinds: readonly TrackerStateKind[] | null; hint: string }[] = [
  { id: 'mine', label: 'My issues', grouping: 'status', kinds: null, hint: 'Assigned to you' },
  { id: 'all', label: 'All', grouping: 'status', kinds: null, hint: 'Every issue in the team' },
  { id: 'active', label: 'Active', grouping: 'status', kinds: ['started', 'unstarted'], hint: 'Started and not started' },
  { id: 'backlog', label: 'Backlog', grouping: 'status', kinds: ['backlog'], hint: 'In the backlog' },
  { id: 'byProject', label: 'By project', grouping: 'project', kinds: null, hint: 'Open issues, grouped by project' },
  { id: 'byAssignee', label: 'By assignee', grouping: 'assignee', kinds: null, hint: 'Open issues, grouped by assignee' },
  // The team's projects, not issues (B4): the page shows the project list.
  { id: 'projects', label: 'Projects', grouping: 'status', kinds: null, hint: 'The team’s projects' },
  // Issues with an open conflict (design section 7's conflicts panel, in the
  // page): not a tab; the view bar shows "Conflicts (n)" only while n > 0.
  { id: 'conflicts', label: 'Conflicts', grouping: 'status', kinds: null, hint: 'Issues where the tracker changed a field you edited' },
];
export const isProjectsView = (s: Pick<ViewState, 'view'>) => s.view === 'projects';
/** The views shown as tabs; Conflicts has its own control. */
export const TAB_VIEWS = VIEWS.filter(v => v.id !== 'conflicts');
/** How many issues have an open conflict (archived ones included: the
 *  conflict still waits on a choice). */
export const conflictCount = (rows: readonly Pick<TrackerViewRow, 'conflict'>[]) => rows.reduce((n, r) => n + (r.conflict ? 1 : 0), 0);
/** Show the Conflicts control: while any issue has one, or while it is the
 *  chosen view (so the empty state and the way back stay put). */
export const showConflicts = (s: Pick<ViewState, 'view'>, count: number) => count > 0 || s.view === 'conflicts';

export const GROUPINGS: readonly { value: Grouping; label: string }[] = [
  { value: 'status', label: 'Status' }, { value: 'assignee', label: 'Assignee' },
  { value: 'project', label: 'Project' }, { value: 'priority', label: 'Priority' },
];
export const ORDERINGS: readonly { value: Ordering; label: string }[] = [
  { value: 'priority', label: 'Priority' }, { value: 'due', label: 'Due date' }, { value: 'updated', label: 'Last updated' },
];
export const FILTER_FIELDS: readonly { field: FilterField; label: string }[] = [
  { field: 'status', label: 'Status' }, { field: 'priority', label: 'Priority' }, { field: 'assignee', label: 'Assignee' },
  { field: 'label', label: 'Label' }, { field: 'project', label: 'Project' }, { field: 'cycle', label: 'Cycle' },
  { field: 'due', label: 'Due date' },
];
export const DUE_OPTIONS: readonly { value: DueFilter; label: string }[] = [
  { value: 'overdue', label: 'Overdue' }, { value: 'today', label: 'Due today' }, { value: 'week', label: 'Due in the next 7 days' },
  { value: 'set', label: 'Has a due date' }, { value: 'none', label: 'No due date' },
];

export const emptyFilters = (): Filters => ({ status: [], priority: [], assignee: [], label: [], project: [], cycle: [], due: [] });
export const DEFAULT_VIEW: ViewState = { view: 'all', grouping: 'status', ordering: 'priority', filters: emptyFilters() };

const viewOf = (id: ViewId) => VIEWS.find(v => v.id === id)!;
/** Choosing a view resets the grouping to the view's own; filters and the
 *  order stay, as a tracker keeps them across its tabs. */
export const chooseView = (s: ViewState, view: ViewId): ViewState => ({ ...s, view, grouping: viewOf(view).grouping });
export const hasFilters = (f: Filters) => Object.values(f).some(v => v.length > 0);
/** Filters that only a tracked issue has; local tasks hide while one is on. */
export const tracksOnly = (f: Filters) => (['status', 'priority', 'assignee', 'cycle'] as const).some(k => f[k].length > 0);

/** Which rows a view and its filters keep. `viewer` is whom the tracker key
 *  belongs to; without it My issues keeps nothing (the page says why). */
export function applyView(rows: readonly TrackerViewRow[], s: ViewState, viewer: string | null, now = new Date()): TrackerViewRow[] {
  const v = viewOf(s.view);
  const f = s.filters;
  const has = (field: FilterField, value: string) => f[field].length === 0 || f[field].includes(value);
  return rows.filter(r =>
    (s.view !== 'mine' || (!!viewer && r.assigneeId === viewer))
    && (s.view !== 'conflicts' || r.conflict)
    && (!v.kinds || v.kinds.includes(r.statusKind))
    && ((s.view !== 'byProject' && s.view !== 'byAssignee') || !isClosedKind(r.statusKind))
    && has('status', r.statusName)
    && has('priority', String(r.priority))
    && has('assignee', r.assigneeId ?? '')
    && (f.label.length === 0 || r.labels.some(l => f.label.includes(l)) || (f.label.includes('') && r.labels.length === 0))
    && has('project', r.projectId ?? '')
    && has('cycle', r.cycle ?? '')
    && (f.due.length === 0 || f.due.some(d => dueMatches(r.due, d as DueFilter, now))));
}

export function dueMatches(due: string | null | undefined, filter: DueFilter, now = new Date()): boolean {
  if (filter === 'none') return !due;
  if (!due) return false;
  if (filter === 'set') return true;
  const [y, m, d] = due.slice(0, 10).split('-').map(Number);
  if (!y || !m || !d) return false;
  const diff = Math.round((new Date(y, m - 1, d).getTime() - new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()) / 86_400_000);
  return filter === 'overdue' ? diff < 0 : filter === 'today' ? diff === 0 : diff >= 0 && diff <= 7;
}

/** The order rows take inside a group. */
export function comparator(ordering: Ordering): (a: TrackerViewRow, b: TrackerViewRow) => number {
  if (ordering === 'updated') return (a, b) => b.updatedAt.localeCompare(a.updatedAt) || compareRows(a, b);
  if (ordering === 'due') return (a, b) => (a.due ? 0 : 1) - (b.due ? 0 : 1) || (a.due ?? '').localeCompare(b.due ?? '') || compareRows(a, b);
  return compareRows;
}

/** Rows in groups for the chosen grouping. Status groups follow the team's
 *  states (`groupByStatus`); the others sort by name, your own group first
 *  when grouping by assignee, and "No assignee" or "No project" last;
 *  priority runs urgent to none. Empty groups are left out. */
export function groupRows(rows: readonly TrackerViewRow[], s: Pick<ViewState, 'grouping' | 'ordering'>, states: readonly TrackerState[], viewer: string | null): IssueGroup[] {
  const order = comparator(s.ordering);
  if (s.grouping === 'status') return groupByStatus(rows, states).map(g => ({ ...g, rows: [...g.rows].sort(order) }));
  const groups = new Map<string, IssueGroup>();
  for (const r of rows) {
    const id = s.grouping === 'assignee' ? `assignee:${r.assigneeId ?? ''}` : s.grouping === 'project' ? `project:${r.projectId ?? ''}` : `priority:${r.priority}`;
    let g = groups.get(id);
    if (!g) {
      g = s.grouping === 'assignee' ? { id, name: r.assignee ?? 'No assignee', kind: 'unstarted', by: 'assignee', person: r.assignee, rows: [] }
        : s.grouping === 'project' ? { id, name: r.project ?? (r.projectId ? 'Unknown project' : 'No project'), kind: 'unstarted', by: 'project', rows: [] }
        : { id, name: priorityLabel(r.priority), kind: 'unstarted', by: 'priority', priority: r.priority, rows: [] };
      groups.set(id, g);
    }
    g.rows.push(r);
  }
  const list = [...groups.values()];
  for (const g of list) g.rows.sort(order);
  const rank = (g: IssueGroup): number => {
    if (g.by === 'priority') return PRIORITY_ORDER.indexOf(g.priority as 0);
    if (g.id.endsWith(':')) return 2;
    return g.by === 'assignee' && viewer && g.id === `assignee:${viewer}` ? 0 : 1;
  };
  return list.sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name));
}

/** What a filter chip says it holds: "In Progress, Todo", "Alex Rivera". */
export function filterSummary(field: FilterField, values: readonly string[], meta: TrackerTeamMeta | undefined): string {
  return values.map(v => filterValueLabel(field, v, meta)).join(', ');
}

export function filterValueLabel(field: FilterField, value: string, meta: TrackerTeamMeta | undefined): string {
  switch (field) {
    case 'priority': return priorityLabel(Number(value));
    case 'assignee': return value ? meta?.members.find(m => m.id === value)?.name ?? 'Someone else' : 'No assignee';
    case 'project': return value ? meta?.projects.find(p => p.id === value)?.name ?? 'Unknown project' : 'No project';
    case 'cycle': return value || 'No cycle';
    case 'label': return value || 'No labels';
    case 'due': return DUE_OPTIONS.find(o => o.value === value)?.label ?? value;
    default: return value;
  }
}

/** The values a filter offers, from the team's metadata. */
export function filterOptions(field: FilterField, meta: TrackerTeamMeta | undefined): { value: string; label: string }[] {
  switch (field) {
    case 'status': return [...(meta?.states ?? [])].sort((a, b) => KIND_ORDER.indexOf(a.kind) - KIND_ORDER.indexOf(b.kind) || a.position - b.position).map(s => ({ value: s.name, label: s.name }));
    case 'priority': return PRIORITY_ORDER.map(p => ({ value: String(p), label: priorityLabel(p) }));
    case 'assignee': return [{ value: '', label: 'No assignee' }, ...(meta?.members ?? []).map(m => ({ value: m.id, label: m.name }))];
    case 'label': return [{ value: '', label: 'No labels' }, ...(meta?.labels ?? []).map(l => ({ value: l.name, label: l.name }))];
    case 'project': return [{ value: '', label: 'No project' }, ...(meta?.projects ?? []).map(p => ({ value: p.id, label: p.name }))];
    case 'cycle': return [{ value: '', label: 'No cycle' }, ...(meta?.cycles ?? []).map(c => { const name = c.name ?? `Cycle ${c.number}`; return { value: name, label: name }; })];
    case 'due': return DUE_OPTIONS.map(o => ({ ...o }));
  }
}

// Persistence: this window first, then the last choice in any window.
export const VIEW_KEY = 'hollow.tasks.view';

/** A saved view as it reads now: anything unknown falls back to the default. */
export function parseView(text: string | null | undefined): ViewState | null {
  if (!text) return null;
  let raw: unknown;
  try { raw = JSON.parse(text); } catch { return null; }
  if (!raw || typeof raw !== 'object') return null;
  const o = raw as Record<string, unknown>;
  const view = VIEWS.some(v => v.id === o.view) ? o.view as ViewId : DEFAULT_VIEW.view;
  const grouping = GROUPINGS.some(g => g.value === o.grouping) ? o.grouping as Grouping : viewOf(view).grouping;
  const ordering = ORDERINGS.some(g => g.value === o.ordering) ? o.ordering as Ordering : DEFAULT_VIEW.ordering;
  const filters = emptyFilters();
  const saved = (o.filters && typeof o.filters === 'object' ? o.filters : {}) as Record<string, unknown>;
  for (const { field } of FILTER_FIELDS) {
    const values = saved[field];
    if (Array.isArray(values)) filters[field] = values.filter((x): x is string => typeof x === 'string').slice(0, 50);
  }
  filters.due = filters.due.filter(d => DUE_OPTIONS.some(o => o.value === d));
  return { view, grouping, ordering, filters };
}

export function readSavedView(session: Pick<Storage, 'getItem'> | null, local: Pick<Storage, 'getItem'> | null): ViewState {
  const read = (s: Pick<Storage, 'getItem'> | null) => { try { return parseView(s?.getItem(VIEW_KEY)); } catch { return null; } };
  return read(session) ?? read(local) ?? { ...DEFAULT_VIEW, filters: emptyFilters() };
}

export function writeSavedView(s: ViewState, session: Pick<Storage, 'setItem'> | null, local: Pick<Storage, 'setItem'> | null): void {
  const text = JSON.stringify(s);
  for (const store of [session, local]) { try { store?.setItem(VIEW_KEY, text); } catch { /* this launch only */ } }
}
