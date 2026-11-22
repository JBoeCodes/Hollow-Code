import type { Task } from './types';

/// Pure task filtering and ranking — extracted from store.ts and the pages so
/// it can be unit tested (and reused by every surface that lists tasks) without
/// booting the store's module-init fetches. Clause grammar is documented on
/// `matchesTaskQuery`; ordering on `compareTasks`.

const PRIO_RANK: Record<string, number> = { P1: 0, P2: 1, P3: 2, P4: 3 };

/// Priority rank (0 highest) with the avoided-work boost: work 2+ days past
/// due moves up one tier so the most-dodged task can't hide below fresher
/// high-priority work forever. Completed tasks never get the boost — lateness
/// only says something about work still owed.
export function effectivePriority(t: Task): number {
  const base = t.priority ? PRIO_RANK[t.priority] ?? 4 : 4;
  if (t.bucket === 'done') return base;
  return (t.daysUntilDue ?? 0) <= -2 ? Math.max(0, base - 1) : base;
}

/// THE task comparator: completed work sinks, overdue leads, then effective
/// priority, then due date. Every surface that orders tasks reads this, so the
/// same two tasks can't sit in opposite order depending on which page you're
/// on. Pair with `rankTasks` unless you need it inline.
export function compareTasks(a: Task, b: Task): number {
  const da = (a.bucket === 'done' ? 1 : 0) - (b.bucket === 'done' ? 1 : 0);
  if (da !== 0) return da;
  const bu = (a.bucket === 'overdue' ? 0 : 1) - (b.bucket === 'overdue' ? 0 : 1);
  if (bu !== 0) return bu;
  const pa = effectivePriority(a);
  const pb = effectivePriority(b);
  if (pa !== pb) return pa - pb;
  return (a.due || '9999').localeCompare(b.due || '9999');
}

/// "What should I do next", ranked. Returns a new array; never mutates.
export function rankTasks(pool: Task[]): Task[] {
  return [...pool].sort(compareTasks);
}

function matchTaskClause(t: Task, c: string): boolean {
  if (c === 'today') return t.bucket === 'today';
  if (c === 'overdue') return t.bucket === 'overdue';
  if (c === 'upcoming') return t.bucket === 'today' || t.bucket === 'thisweek' || t.bucket === 'later';
  if (c === 'nodate' || c === 'no date') return t.bucket === 'undated';
  if (/^p[1-4]$/.test(c)) return (t.priority ?? '').toLowerCase() === c;
  if (c === 'done' || c === 'delivered') return /deliver|done|complete/.test(t.status.toLowerCase());
  if (c === 'active') return !/deliver|done|complete/.test(t.status.toLowerCase());
  if (c === 'ready') return t.ready;
  if (c.startsWith('@')) { const l = c.slice(1); return t.labels.some(x => x.toLowerCase() === l); }
  if (c.startsWith('#')) { const p = c.slice(1); return (t.projectName ?? '').toLowerCase().includes(p); }
  return t.title.toLowerCase().includes(c) || (t.description ?? '').toLowerCase().includes(c);
}

/// Evaluate a task against a query string. Clauses are AND-joined with `&`
/// (e.g. `today & p1 & @urgent`). Empty query matches everything.
export function matchesTaskQuery(t: Task, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return q.split('&').map(s => s.trim()).filter(Boolean).every(c => matchTaskClause(t, c));
}

/** The custom filter's help: every token `matchTaskClause` reads (a unit
 *  test checks the list stays complete). */
export const TASK_QUERY_HELP = 'Combine with &: today, overdue, upcoming, nodate (or no date), ready (open, not on hold, nothing blocks it), active, done (or delivered), p1–p4, @label, #project, or any words.';

/** The row's Ready chip: an open task whose blockers are all done, so the
 *  word means something (an unblocked task with no blockers says nothing). */
export function showsReady(t: Task): boolean {
  return t.ready && t.blockedBy.length > 0;
}

const STRUCTURAL = new Set([
  'today', 'overdue', 'upcoming', 'nodate', 'no date', 'done', 'delivered', 'active', 'ready',
]);
/// Conversational filler dropped when normalizing a natural-language query
/// ("show me overdue p1 tasks" → "overdue & p1").
const FILLER = new Set([
  'show', 'me', 'my', 'all', 'the', 'a', 'list', 'find', 'what', 'whats',
  "what's", 'are', 'is', 'that', 'with', 'task', 'tasks', 'items', 'things',
]);

/// Normalize a free-text palette query into the task-filter grammar, or
/// return null when it doesn't read as a task query. A query qualifies when
/// at least one token (after dropping conversational filler) is structural —
/// `overdue`, `p1`..`p4`, `@label`, `#project`, `nodate`, `done`, `active`.
/// Explicit `&` queries pass through; plain-word tokens become AND-joined
/// substring clauses ("overdue budget" → "overdue & budget").
export function toTaskQuery(q: string): string | null {
  const raw = q.trim().toLowerCase();
  if (!raw) return null;
  const tokens = raw
    .split(raw.includes('&') ? '&' : /\s+/)
    .map(s => s.trim())
    .filter(Boolean)
    .filter(t => !FILLER.has(t));
  const structural = tokens.some(
    t => STRUCTURAL.has(t) || /^p[1-4]$/.test(t) || t.startsWith('@') || t.startsWith('#'),
  );
  if (!structural || tokens.length === 0) return null;
  return tokens.join(' & ');
}
