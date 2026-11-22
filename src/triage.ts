// The triage inbox (overnight 10-11 B6), pure. The screen was cut
// (overnight 11-08 S3). The store (`store/triage.ts`) still loads the sources.
// Unit-tested in triage.test.ts.
//
// Rows come from three places, and none is a second store of decisions:
// - Waiting on you's own query (`list_needs_you`): review items (learnings,
//   suggested tasks…), question and approval cards, ask-canvases, Results to
//   review, unread failures;
// - the Plaud reader (`list_recordings`): recordings from the last 14 days
//   that aren't linked to a workspace;
// - `triage_arrivals` (src-tauri/src/triage.rs): recent imports and tasks
//   other sources made, plus the ids the person dismissed or filed here.
// Accepting a review item or a Result is always the person's own keypress.
import type { NeedsYouItem } from './workspaceTypes';
import type { RecordingLink, RecordingRow } from './recordingTypes';

export interface TriageImport { id: string; title: string; path: string; files: string[]; fileCount: number; at: string | null; excerpt: string }
export interface TriageTask { id: string; path: string; title: string; from: string; at: string | null; project: string | null; due: string | null }
export interface TriageFiled { workspaceId: string; at: string }
/** The task made from a row here (`vault:tasks/…`), and when. */
export interface TriageTasked { task: string; at: string }
export interface TriageState { dismissed: Record<string, string>; filed: Record<string, TriageFiled>; tasked?: Record<string, TriageTasked> }
export interface TriageArrivals { imports: TriageImport[]; tasks: TriageTask[]; state: TriageState; unavailable: string[] }
export interface TriageSuggestion { workspaceId: string; title: string; score: number; reasons: string[] }

export type TriageKind = 'recording' | 'import' | 'task' | 'review' | 'ask' | 'result' | 'failure';

export type TriageItem =
  | { id: string; kind: 'recording'; title: string; at: string | null; workspaceId: null; recording: RecordingRow }
  | { id: string; kind: 'import'; title: string; at: string | null; workspaceId: null; imp: TriageImport }
  | { id: string; kind: 'task'; title: string; at: string | null; workspaceId: null; task: TriageTask }
  | { id: string; kind: 'review' | 'ask' | 'result' | 'failure'; title: string; at: string | null; workspaceId: string | null; needs: NeedsYouItem };

/** The filter chips, in order; keys 1–8 pick them. */
export const TRIAGE_FILTERS: readonly { id: 'all' | TriageKind; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'recording', label: 'Recordings' },
  { id: 'import', label: 'Sources' },
  { id: 'task', label: 'Tasks' },
  { id: 'review', label: 'Reviews' },
  { id: 'ask', label: 'Questions' },
  { id: 'result', label: 'Results' },
  { id: 'failure', label: 'Failures' },
];
export type TriageFilter = (typeof TRIAGE_FILTERS)[number]['id'];

/** How far back recordings arrive (days). */
export const RECORDING_DAYS = 14;

/** A Waiting-on-you kind's inbox kind, or null for what the inbox leaves to
 *  Waiting on you (restart offers, sync conflicts, conflict copies: they
 *  aren't arrivals, and each has its own dialog there). */
export function needsKind(kind: NeedsYouItem['kind']): 'review' | 'ask' | 'result' | 'failure' | null {
  switch (kind) {
    case 'review': return 'review';
    case 'approval': case 'question': case 'canvas': return 'ask';
    case 'result': return 'result';
    case 'failure': return 'failure';
    default: return null;
  }
}

/** A recording row's date as a time, when it reads as one. */
export function recordingAt(date: string | null): string | null {
  if (!date) return null;
  const t = Date.parse(date.includes('T') || date.length <= 10 ? date : date.replace(' ', 'T'));
  return Number.isFinite(t) ? new Date(t).toISOString() : null;
}

export interface TriageSources {
  needs: readonly NeedsYouItem[];
  recordings: readonly RecordingRow[];
  recordingLinks: Readonly<Record<string, RecordingLink>>;
  arrivals: TriageArrivals | null;
  state: TriageState;
}

/** Whether `id` left the inbox (dismissed, filed, or made into a task here). */
export function handled(state: TriageState, id: string): boolean {
  return id in state.dismissed || id in state.filed || id in (state.tasked ?? {});
}

/** Every row, newest first (undated last), without what the person
 *  dismissed or filed, recordings already linked to a workspace, and the
 *  question cards that can no longer be answered. */
export function buildTriage(src: TriageSources): TriageItem[] {
  const out: TriageItem[] = [];
  for (const n of src.needs) {
    const kind = needsKind(n.kind);
    if (!kind || (n.kind === 'question' && n.state !== 'pending')) continue;
    out.push({ id: n.id, kind, title: n.title, at: n.at ?? null, workspaceId: n.workspaceId ?? null, needs: n });
  }
  for (const r of src.recordings) {
    if (src.recordingLinks[r.id]?.workspaceId) continue;
    out.push({ id: `recording:${r.id}`, kind: 'recording', title: r.name || 'Untitled recording', at: recordingAt(r.date), workspaceId: null, recording: r });
  }
  for (const imp of src.arrivals?.imports ?? []) out.push({ id: imp.id, kind: 'import', title: imp.title, at: imp.at, workspaceId: null, imp });
  for (const task of src.arrivals?.tasks ?? []) out.push({ id: task.id, kind: 'task', title: task.title, at: task.at, workspaceId: null, task });
  const seen = new Set<string>();
  return out
    .filter(item => !handled(src.state, item.id) && !seen.has(item.id) && (seen.add(item.id), true))
    .sort((a, b) => (a.at && b.at ? b.at.localeCompare(a.at) : a.at ? -1 : b.at ? 1 : 0) || a.id.localeCompare(b.id));
}

/** The text a row's suggested workspace is matched against. */
export function suggestText(item: TriageItem): string {
  switch (item.kind) {
    case 'recording': return item.title;
    case 'import': return `${item.title}\n${item.imp.excerpt}`;
    case 'task': return [item.title, item.task.project].filter(Boolean).join('\n');
    default: return [item.title, item.needs.detail].filter(Boolean).join('\n');
  }
}

/** Rows to ask a suggestion for: those that belong nowhere yet. */
export function needsSuggestion(item: TriageItem): boolean {
  return !item.workspaceId;
}

export function filterTriage(items: readonly TriageItem[], filter: TriageFilter): TriageItem[] {
  return filter === 'all' ? [...items] : items.filter(item => item.kind === filter);
}

export function countByKind(items: readonly TriageItem[]): Record<TriageFilter, number> {
  const counts = Object.fromEntries(TRIAGE_FILTERS.map(f => [f.id, 0])) as Record<TriageFilter, number>;
  for (const item of items) { counts[item.kind] += 1; counts.all += 1; }
  return counts;
}

/** The day header a row sits under. */
export function dayGroup(at: string | null, now: Date): string {
  if (!at) return 'Undated';
  const t = new Date(at);
  if (Number.isNaN(t.getTime())) return 'Undated';
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const day = 86_400_000;
  if (t.getTime() >= start) return 'Today';
  if (t.getTime() >= start - day) return 'Yesterday';
  if (t.getTime() >= start - 6 * day) return 'Earlier this week';
  return 'Older';
}

/** Rows under their day headers, in order. */
export function groupByDay(items: readonly TriageItem[], now: Date): { label: string; items: TriageItem[] }[] {
  const groups: { label: string; items: TriageItem[] }[] = [];
  for (const item of items) {
    const label = dayGroup(item.at, now);
    const last = groups[groups.length - 1];
    if (last && last.label === label) last.items.push(item);
    else groups.push({ label, items: [item] });
  }
  return groups;
}

export type TriageAction = 'file' | 'accept' | 'task' | 'dismiss' | 'open';

export interface ActionView { label: string; key: string; reason: string | null }

/** Review kinds whose Accept makes a task, so `t` points there instead. */
const TASK_PROPOSALS = new Set(['heartbeat_task', 'meeting_task', 'agent_task']);

/** What each action does to `item` (label, key) or why it can't (`reason`).
 *  `target`: the workspace `e` would file into (the picked or suggested
 *  one), by title. */
export function triageActions(item: TriageItem, target: string | null): Record<TriageAction, ActionView> {
  const fileable = item.kind === 'recording' || item.kind === 'import';
  const file: ActionView = fileable
    ? { label: target ? `File into ${target}` : 'File into a workspace', key: 'e', reason: target ? null : 'Pick a workspace first.' }
    : { label: 'File', key: 'e', reason: item.kind === 'task' ? 'Tasks file by project: open it to set one.' : 'Only recordings and imports file from here.' };
  const accept: ActionView = item.kind === 'review'
    ? { label: 'Accept', key: 'a', reason: null }
    // Incoming never shows a Result's content, so it never accepts one
    // (P0-10): Enter opens it, and it is accepted there.
    : item.kind === 'result'
      ? { label: 'Accept Result', key: 'a', reason: 'Open it (Enter) to read it, and accept it there.' }
      : { label: 'Accept', key: 'a', reason: item.kind === 'ask' ? 'Answer it in its conversation (Enter).' : 'Nothing to accept here.' };
  const task: ActionView = item.kind === 'task'
    ? { label: 'Make a task', key: 't', reason: 'It is a task already.' }
    : item.kind === 'review' && TASK_PROPOSALS.has(item.needs.category ?? '')
      ? { label: 'Make a task', key: 't', reason: 'Accept (a) makes this task.' }
      : { label: 'Make a task', key: 't', reason: null };
  const dismiss: ActionView = item.kind === 'review'
    ? { label: 'Reject', key: 'x', reason: null }
    : item.kind === 'failure'
      ? { label: 'Mark read', key: 'x', reason: null }
      : { label: 'Dismiss', key: 'x', reason: null };
  return { file, accept, task, dismiss, open: { label: 'Open', key: 'Enter', reason: null } };
}

/** The task `t` makes from `item`: a title, a line saying where it came
 *  from, and the idempotency key that keeps a second `t` from making a twin. */
export function taskFrom(item: TriageItem): { title: string; description: string; idempotencyKey: string; origin: string } {
  const verb = item.kind === 'recording' ? 'Follow up' : item.kind === 'import' ? 'Read' : item.kind === 'failure' ? 'Fix'
    : item.kind === 'ask' ? 'Answer' : item.kind === 'result' ? 'Review' : 'Do';
  const where = item.kind === 'import' ? ` (${item.imp.path})`
    : item.kind === 'recording' ? ` (Plaud recording ${item.recording.id})`
      : item.kind !== 'task' && item.needs.path ? ` (${item.needs.path})` : '';
  const title = `${verb}: ${item.title}`.slice(0, 200);
  return { title, description: `From the inbox${where}.`, idempotencyKey: `triage:${item.id}`, origin: `triage:${item.id}`.slice(0, 200) };
}

export type TriageKeyCommand =
  | { kind: 'move'; dir: 1 | -1 }
  | { kind: 'action'; action: TriageAction }
  | { kind: 'filter'; filter: TriageFilter }
  | { kind: 'undo' }
  | { kind: 'close' };

/** A keystroke's command inside the inbox, or null. Letters and digits only
 *  bare (no ⌘/⌃/⌥, so never a macOS chord); `letters` false (Use keyboard
 *  shortcuts off) leaves only the arrows, Enter and Escape. */
export function triageKey(e: { key: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean }, letters: boolean): TriageKeyCommand | null {
  if (e.metaKey || e.ctrlKey || e.altKey) return null;
  if (e.key === 'Escape') return { kind: 'close' };
  if (e.key === 'ArrowDown') return { kind: 'move', dir: 1 };
  if (e.key === 'ArrowUp') return { kind: 'move', dir: -1 };
  if (e.key === 'Enter') return { kind: 'action', action: 'open' };
  if (!letters) return null;
  switch (e.key) {
    case 'j': return { kind: 'move', dir: 1 };
    case 'k': return { kind: 'move', dir: -1 };
    case 'e': return { kind: 'action', action: 'file' };
    case 'a': return { kind: 'action', action: 'accept' };
    case 't': return { kind: 'action', action: 'task' };
    case 'x': return { kind: 'action', action: 'dismiss' };
    case 'u': return { kind: 'undo' };
  }
  if (/^[1-8]$/.test(e.key)) return { kind: 'filter', filter: TRIAGE_FILTERS[Number(e.key) - 1].id };
  return null;
}

/** After a row leaves, the row to select: the one that took its place, else
 *  the new last one. */
export function nextSelection(before: readonly string[], after: readonly string[], gone: string): string | null {
  if (!after.length) return null;
  const i = before.indexOf(gone);
  if (i < 0) return after[0];
  for (let j = i + 1; j < before.length; j += 1) if (after.includes(before[j])) return before[j];
  for (let j = i - 1; j >= 0; j -= 1) if (after.includes(before[j])) return before[j];
  return after[0];
}

/** A row's second line: where it came from and its kind. */
export function rowMeta(item: TriageItem): string {
  switch (item.kind) {
    case 'recording': return ['Recording', item.recording.duration].filter(Boolean).join(' · ');
    case 'import': return `Import · ${item.imp.fileCount} note${item.imp.fileCount === 1 ? '' : 's'}`;
    case 'task': return `New task from ${item.task.from}`;
    case 'review': return item.needs.detail || 'Review item';
    case 'ask': return item.needs.kind === 'approval' ? 'Approval' : item.needs.kind === 'canvas' ? 'Canvas waiting for your answer' : 'Question';
    case 'result': return item.needs.state === 'changed' ? 'Result changed since accepted' : 'Result ready for review';
    case 'failure': return 'Failure';
  }
}
