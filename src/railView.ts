// Sort, filter, group and saved views for the sidebar's Quick chats list (t3code parity 3.5,
// B12 and A12). A view is three choices; a saved view is a named view. Pure: the facts a filter
// reads come from `store/rowStatus.ts`, the persistence from `store/railView.ts`.
import type { AgentSessionSummary } from './types';
import type { SessionState } from './sessionIndicator';
import type { RowMarkId } from './rowStatus';
import { agentLabel } from './agentLabel';

export type RailSort = 'recent' | 'title' | 'created';
export type RailFilter = 'all' | 'attention' | 'working' | 'unread' | 'pr';
export type RailGroup = 'none' | 'agent' | 'day';
export interface RailView { sort: RailSort; filter: RailFilter; group: RailGroup }
export interface SavedRailView extends RailView { id: string; name: string }

export const DEFAULT_RAIL_VIEW: RailView = { sort: 'recent', filter: 'all', group: 'none' };
export const SORT_LABELS: Record<RailSort, string> = { recent: 'Recent activity', title: 'Title, A to Z', created: 'Newest started' };
export const FILTER_LABELS: Record<RailFilter, string> = { all: 'All chats', attention: 'Needs you', working: 'Working', unread: 'Unread', pr: 'With a pull request' };
export const GROUP_LABELS: Record<RailGroup, string> = { none: 'No grouping', agent: 'By agent', day: 'By day' };

export const isDefaultView = (v: RailView) => v.sort === 'recent' && v.filter === 'all' && v.group === 'none';
export const sameView = (a: RailView, b: RailView) => a.sort === b.sort && a.filter === b.filter && a.group === b.group;

/** What a filter reads about one chat. */
export interface RailFacts { state: SessionState; marks: readonly RowMarkId[] }

export function matchesFilter(filter: RailFilter, f: RailFacts): boolean {
  switch (filter) {
    case 'attention': return f.state === 'waiting' || f.state === 'failed' || f.state === 'interrupted' || f.state === 'paused' || f.marks.includes('approval') || f.marks.includes('input');
    case 'working': return f.state === 'running' || f.state === 'queued';
    case 'unread': return f.marks.includes('unread');
    case 'pr': return f.marks.includes('pr');
    default: return true;
  }
}

/** Filter then sort a list; the order of equal rows is kept. */
export function applyRailView<T extends AgentSessionSummary>(rows: readonly T[], view: RailView, facts: (s: T) => RailFacts): T[] {
  const out = rows.filter(s => matchesFilter(view.filter, facts(s)));
  const by: Record<RailSort, (a: T, b: T) => number> = {
    recent: (a, b) => b.lastActivityAt.localeCompare(a.lastActivityAt),
    title: (a, b) => (a.title?.trim() || 'New chat').localeCompare(b.title?.trim() || 'New chat', undefined, { sensitivity: 'base', numeric: true }),
    created: (a, b) => b.createdAt.localeCompare(a.createdAt),
  };
  return out.map((s, i) => ({ s, i })).sort((a, b) => by[view.sort](a.s, b.s) || a.i - b.i).map(x => x.s);
}

export interface RailGroupRows<T> { id: string; label: string; rows: T[] }

const dayStart = (ms: number) => { const d = new Date(ms); return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime(); };

/** Split a sorted list under headings; `none` is one headless group. Agent groups appear in the order
 *  their first row does; day groups run Today, Yesterday, Earlier this week, Older. */
export function groupRailRows<T extends AgentSessionSummary>(rows: readonly T[], group: RailGroup, now: number): RailGroupRows<T>[] {
  if (group === 'none') return rows.length ? [{ id: 'all', label: '', rows: [...rows] }] : [];
  const out = new Map<string, RailGroupRows<T>>();
  const today = dayStart(now);
  for (const s of rows) {
    let id: string, label: string;
    if (group === 'agent') { id = s.agent || 'unknown'; label = agentLabel(s.agent) ?? 'Other'; }
    else {
      const days = Math.round((today - dayStart(Date.parse(s.lastActivityAt) || 0)) / 86_400_000);
      [id, label] = days <= 0 ? ['today', 'Today'] : days === 1 ? ['yesterday', 'Yesterday'] : days < 7 ? ['week', 'Earlier this week'] : ['older', 'Older'];
    }
    const g = out.get(id) ?? { id, label, rows: [] };
    g.rows.push(s);
    out.set(id, g);
  }
  const all = [...out.values()];
  // Days read newest first whatever the sort inside them.
  if (group === 'day') { const rank = ['today', 'yesterday', 'week', 'older']; all.sort((a, b) => rank.indexOf(a.id) - rank.indexOf(b.id)); }
  return all;
}

export function viewSummary(v: RailView): string {
  return [v.filter !== 'all' ? FILTER_LABELS[v.filter] : '', v.sort !== 'recent' ? SORT_LABELS[v.sort] : '', v.group !== 'none' ? GROUP_LABELS[v.group] : ''].filter(Boolean).join(' · ') || 'Default view';
}

/** Saved views kept (the newest). */
export const SAVED_VIEWS_KEPT = 12;

export function addSavedView(views: readonly SavedRailView[], name: string, v: RailView): SavedRailView[] {
  const n = name.trim().slice(0, 40);
  if (!n || isDefaultView(v)) return [...views];
  const same = views.find(x => x.name.toLowerCase() === n.toLowerCase() || sameView(x, v));
  if (same) return views.map(x => x === same ? { ...x, ...v, name: n } : x);
  return [...views, { ...v, name: n, id: `rv${Date.now().toString(36)}${views.length}` }].slice(-SAVED_VIEWS_KEPT);
}
export const removeSavedView = (views: readonly SavedRailView[], id: string) => views.filter(v => v.id !== id);

const oneOf = <T extends string>(v: unknown, all: readonly T[], fallback: T): T => all.includes(v as T) ? v as T : fallback;
export function readRailView(raw: string | null): RailView {
  try {
    const v = JSON.parse(raw ?? '{}') as Partial<RailView>;
    return { sort: oneOf(v.sort, ['recent', 'title', 'created'], 'recent'), filter: oneOf(v.filter, ['all', 'attention', 'working', 'unread', 'pr'], 'all'), group: oneOf(v.group, ['none', 'agent', 'day'], 'none') };
  } catch { return { ...DEFAULT_RAIL_VIEW }; }
}
export function readSavedRailViews(raw: string | null): SavedRailView[] {
  try {
    const v = JSON.parse(raw ?? '[]');
    return Array.isArray(v) ? v.filter(x => x && typeof x.id === 'string' && typeof x.name === 'string').slice(-SAVED_VIEWS_KEPT).map(x => ({ id: x.id, name: String(x.name).slice(0, 40), ...readRailView(JSON.stringify(x)) })) : [];
  } catch { return []; }
}

/** The list's cap while a view narrows or reorders it (the default keeps the short list). */
export const VIEW_LIST_CAP = 50;
