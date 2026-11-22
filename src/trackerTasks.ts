// The Tasks page laid out like a tracker's issue list (overhaul B1, board
// S3, design section 12): the pure parts behind `pages/Tasks.tsx`,
// `pages/tasks/IssueList.tsx` and `pages/tasks/IssueDetail.tsx`. Rows come
// from `tracker_view` (team AI's mirrored issues) and the vault's local,
// untracked tasks; groups follow the team's state order; edits become a
// compare-and-swap `TrackerEdit` built from the values the detail read.

import type {
  TrackerEdit, TrackerField, TrackerIssue, TrackerIssueDetail, TrackerState, TrackerStateKind, TrackerTeamMeta,
  TrackerViewRow,
} from './generated/ipc';
import { keyCompare } from './collate';

/** Linear's priority numbers: 0 none, 1 urgent, 2 high, 3 medium, 4 low. */
export const PRIORITY_LABELS = ['No priority', 'Urgent', 'High', 'Medium', 'Low'] as const;
export const priorityLabel = (p: number) => PRIORITY_LABELS[p] ?? PRIORITY_LABELS[0];
/** The order a person reads priorities in: urgent first, none last. */
export const PRIORITY_ORDER = [1, 2, 3, 4, 0] as const;
const priorityRank = (p: number) => { const i = PRIORITY_ORDER.indexOf(p as 0); return i < 0 ? PRIORITY_ORDER.length : i; };

export const KIND_ORDER: TrackerStateKind[] = ['started', 'unstarted', 'backlog', 'completed', 'canceled'];
export const KIND_LABELS: Record<TrackerStateKind, string> = {
  backlog: 'Backlog', unstarted: 'Not started', started: 'In progress', completed: 'Done', canceled: 'Canceled',
};
export const isClosedKind = (k: TrackerStateKind) => k === 'completed' || k === 'canceled';

export interface IssueGroup {
  /** The state's id, or `local` for the vault's own tasks. */
  id: string;
  name: string;
  kind: TrackerStateKind | 'local';
  /** What the group stands for (B2's grouping): its header shows a status
   *  ring, an avatar or a priority mark. Status when unset. */
  by?: 'status' | 'assignee' | 'project' | 'priority';
  /** The assignee's name, for an assignee group's avatar. */
  person?: string | null;
  /** The priority number, for a priority group's mark. */
  priority?: number;
  rows: TrackerViewRow[];
}

/** Rows grouped by status, in the order a tracker shows them: what is in
 *  progress first, then to do, backlog, done and canceled; within a kind,
 *  the team's own state order. Within a group: priority, then the most
 *  recently updated. Empty states are left out. */
export function groupByStatus(rows: readonly TrackerViewRow[], states: readonly TrackerState[]): IssueGroup[] {
  const ordered = [...states].sort((a, b) => KIND_ORDER.indexOf(a.kind) - KIND_ORDER.indexOf(b.kind) || a.position - b.position);
  const groups: IssueGroup[] = ordered.map(s => ({ id: s.id, name: s.name, kind: s.kind, rows: [] }));
  const byName = new Map(groups.map(g => [g.name.toLowerCase(), g]));
  for (const row of rows) {
    let group = byName.get(row.statusName.toLowerCase());
    if (!group) {
      // A state the team no longer lists (renamed in the tracker since the
      // last pass): its own group, so the row is never hidden.
      group = { id: `state:${row.statusName}`, name: row.statusName, kind: row.statusKind, rows: [] };
      groups.push(group);
      byName.set(row.statusName.toLowerCase(), group);
    }
    group.rows.push(row);
  }
  for (const g of groups) g.rows.sort(compareRows);
  return groups.filter(g => g.rows.length > 0);
}

export function compareRows(a: TrackerViewRow, b: TrackerViewRow): number {
  return priorityRank(a.priority) - priorityRank(b.priority) || b.updatedAt.localeCompare(a.updatedAt) || keyCompare(a.key, b.key);
}

/** Which rows a search keeps: the key or title, labels, project or assignee. */
export function matchesSearch(row: TrackerViewRow, query: string): boolean {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return true;
  const hay = [row.key, row.title, ...row.labels, row.project ?? '', row.assignee ?? 'unassigned', row.statusName].join(' ').toLowerCase();
  return words.every(w => hay.includes(w));
}

/** "AR" for Alex Rivera; "?" for nobody. */
export function initials(name: string | null | undefined): string {
  const parts = (name ?? '').trim().split(/\s+/).filter(Boolean);
  if (!parts.length) return '?';
  return (parts[0][0] + (parts.length > 1 ? parts[parts.length - 1][0] : '')).toUpperCase();
}

/** "Nov 12", "Today", "Tomorrow", "Yesterday"; with `overdue` for styling. */
export function dueText(due: string | null | undefined, now = new Date()): { text: string; overdue: boolean } | null {
  if (!due) return null;
  const [y, m, d] = due.slice(0, 10).split('-').map(Number);
  if (!y || !m || !d) return null;
  const day = new Date(y, m - 1, d);
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const diff = Math.round((day.getTime() - today.getTime()) / 86_400_000);
  const text = diff === 0 ? 'Today' : diff === 1 ? 'Tomorrow' : diff === -1 ? 'Yesterday'
    : day.toLocaleDateString('en-US', { month: 'short', day: 'numeric', ...(day.getFullYear() !== today.getFullYear() ? { year: 'numeric' } : {}) });
  return { text, overdue: diff < 0 };
}

/** "Nov 7, 2026, 4:27 PM" for created, updated and comment times. */
export function stamp(iso: string | null | undefined): string {
  if (!iso) return '';
  const d = new Date(iso);
  if (!Number.isFinite(d.getTime())) return '';
  return d.toLocaleString('en-US', { month: 'short', day: 'numeric', year: 'numeric', hour: 'numeric', minute: '2-digit' });
}

/** The value of one field as the detail read it, in the shape the edit sends. */
export function fieldValue(issue: TrackerIssue, field: TrackerField): unknown {
  switch (field) {
    case 'title': return issue.title;
    case 'description': return issue.description;
    case 'status': return issue.state.id;
    case 'priority': return issue.priority;
    case 'assignee': return issue.assignee?.id ?? '';
    case 'labels': return [...issue.labels];
    case 'project': return issue.projectId ?? '';
    case 'due': return issue.due ?? '';
    case 'parent': return issue.parentId ?? '';
    case 'estimate': return issue.estimate;
    case 'cycle': return issue.cycle?.id ?? '';
    case 'milestone': return issue.milestone?.id ?? '';
  }
}

/** One field's compare-and-swap edit: the new value and the value read. */
export function fieldEdit(issue: TrackerIssue, field: TrackerField, value: unknown): TrackerEdit {
  return { id: issue.id, values: { [field]: value }, expected: { [field]: fieldValue(issue, field) } };
}

/** Whether the person may change this field here, and if not, why. */
export function fieldLock(detail: TrackerIssueDetail, field: TrackerField): string | null {
  if (detail.issue.archivedAt || detail.issue.trashed) return 'Archived in the tracker. Restore it there to edit it.';
  if (detail.lock && detail.lock.kind !== 'linked') return LOCK_WORDS[detail.lock.kind];
  if (!detail.writableFields.includes(field)) return 'The tracker owns this field; it can’t be changed from Hollow.';
  if (detail.pending.includes(field)) return 'An earlier edit to this field is still waiting to send.';
  return null;
}

const LOCK_WORDS: Record<Exclude<NonNullable<TrackerIssueDetail['lock']>['kind'], 'linked'>, string> = {
  remoteMissing: 'The tracker no longer has this issue. Its last values stay in your vault.',
  moved: 'This issue moved out of the team Hollow syncs, so sync stopped for it.',
  unreadable: 'Hollow can’t read this issue’s file. Fix the file, then sync.',
  staleRelations: 'Its relations changed in the tracker; sync to read them again.',
  outOfScope: 'This issue belongs to another team. Hollow never writes outside its team.',
};

/** Options for the detail's pickers, from the team's metadata. */
export function stateOptions(meta: TrackerTeamMeta | undefined, current: TrackerState) {
  const states = meta?.states.length ? meta.states : [current];
  return [...states].sort((a, b) => KIND_ORDER.indexOf(a.kind) - KIND_ORDER.indexOf(b.kind) || a.position - b.position)
    .map(s => ({ value: s.id, label: s.name }));
}

/** The relation words a tracker uses, for the detail's Relations list. */
export const RELATION_LABELS = { blocks: 'Blocks', blockedBy: 'Blocked by', related: 'Related to', duplicateOf: 'Duplicate of' } as const;

/** Index moves for j/k and the arrows: clamps, and -1 starts at the top. */
export function step(index: number, delta: number, length: number): number {
  if (!length) return -1;
  if (index < 0) return delta > 0 ? 0 : length - 1;
  return Math.min(length - 1, Math.max(0, index + delta));
}

/** One run of an agent on this issue, for the detail's Agent activity
 *  section. Lane 4 of the COO rebuild plans these as `tracker_issue_work`
 *  (`WorkRun`: sessionKey, title, agent, model, effort, status, startedAt,
 *  endedAt, workingMs, tokens, costUsd, linkedBy; its note
 *  `linear-tracking-audit.md` section 5). Until that IPC lands this reads an
 *  optional `TrackerIssueDetail.agentRuns`, and the section says nothing has
 *  run; when it lands, `agentRunsOf` is the one place to repoint. */
export interface IssueAgentRun {
  /** Who ran it: the COO, a project lead, or a subagent's title. */
  who: string;
  /** Agent and model, e.g. "Claude Opus 5.5". */
  model: string | null;
  startedAt: string;
  endedAt: string | null;
  /** done, failed, stopped or running. */
  outcome: string;
  /** The session to open in the side panel's Subagents tab. */
  sessionKey: string | null;
  summary: string | null;
}

/** The detail's agent runs, newest first, once the backend sends them. */
export function agentRunsOf(detail: TrackerIssueDetail): IssueAgentRun[] {
  const runs = (detail as TrackerIssueDetail & { agentRuns?: IssueAgentRun[] }).agentRuns;
  return Array.isArray(runs) ? [...runs].sort((a, b) => b.startedAt.localeCompare(a.startedAt)) : [];
}
