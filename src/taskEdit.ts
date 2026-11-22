// Pure TaskModal edit model: the prefill snapshot taken when the sheet opens
// and the changed-field diff sent on save. Dependency-free apart from utils,
// so the node vitest suite can load it (TaskModal.tsx itself imports the store).

import { fmtClockHHMM, parseClockTime } from './utils';
import type { SubTask, Task, UpdateTaskFields } from './types';
import { dueTimePart, lockedTaskFields, type LockableField } from './taskLocks';

// Re-exported from a small module so Home's due-tasks card doesn't pull the
// task editor's helpers into the App chunk (overnight P3).
export { dueTimePart, lockedTaskFields, type LockableField };

/// The date part of a stored `due`, or '' when it isn't a calendar date.
/// Imported and tracker tasks can carry full ISO stamps ("2026-07-23T17:00:00.000-04:00")
/// and `<input type="date">` renders anything but `YYYY-MM-DD` as empty — so
/// without this the sheet showed no due date and saved the blank back over it.
export function dueDatePart(raw: string): string {
  return /^\d{4}-\d{2}-\d{2}/.test(raw) ? raw.slice(0, 10) : '';
}

/** The editable TaskModal fields, as the form holds them. */
export interface TaskEditValues {
  title: string;
  status: string;
  priority: string;
  /** `YYYY-MM-DD` date part, or ''. */
  due: string;
  /** Canonical `HH:MM`, or '' for no time. */
  dueTime: string;
  /** Duration input text; compared as an integer. */
  durationMin: string;
  recurrence: string;
  section: string;
  project: string;
  labels: string[];
  description: string;
  subtasks: SubTask[];
  /** `vault:tasks/…` blocker URLs (Paperclip PR 11b), in order. */
  blockedBy: string[];
}

export type TaskEditChanges = Omit<UpdateTaskFields, 'url' | 'expected'>;

const FIELD_LABELS: Record<LockableField, string> = {
  title: 'title', status: 'status', priority: 'priority', due: 'due date',
  dueTime: 'time', project: 'project', description: 'summary',
};

function listPhrase(items: string[]): string {
  return items.length < 2 ? items.join('') : `${items.slice(0, -1).join(', ')} and ${items[items.length - 1]}`;
}

/// The notice TaskModal shows above a tracker-linked task's form, naming the
/// fields the issue owns.
export function linkedLockNotice(locked: readonly LockableField[]): string {
  return `Linked to an issue. Edit the ${listPhrase(locked.map(f => FIELD_LABELS[f]))} on the issue in Tasks; everything else saves here.`;
}

/// The prefill snapshot for a task (or a blank new task).
export function taskEditValues(t: Task | null): TaskEditValues {
  const rawDue = (t?.due ?? '').trim();
  return {
    title: t?.title ?? '',
    status: t?.status || 'Not started',
    priority: t?.priority ?? '',
    due: dueDatePart(rawDue),
    // A full-ISO `due` holds the time the date input can't; surface it
    // unless the task already carries its own `due_time`.
    dueTime: parseClockTime(fmtClockHHMM(t?.dueTime || dueTimePart(rawDue))) ?? '',
    durationMin: t?.durationMin != null ? String(t.durationMin) : '',
    recurrence: t?.recurrence ?? '',
    section: t?.section ?? '',
    project: t?.projectName ?? '',
    labels: t ? [...t.labels] : [],
    description: t?.description ?? '',
    subtasks: t ? t.subtasks.map(s => ({ ...s })) : [],
    blockedBy: t ? [...(t.blockedBy ?? [])] : [],
  };
}

/// Labels as the backend stores them: trimmed, leading `@` dropped, empties
/// removed, de-duplicated case-insensitively (first spelling wins).
export function normalizeLabels(labels: string[]): string[] {
  const out: string[] = [];
  for (const raw of labels) {
    const label = raw.trim().replace(/^@/, '').trim();
    if (label && !out.some(x => x.toLowerCase() === label.toLowerCase())) out.push(label);
  }
  return out;
}

function minutes(raw: string): number | null {
  const n = parseInt(raw.trim(), 10);
  return Number.isFinite(n) && n >= 0 ? n : null;
}

/// Only the fields whose value differs from the prefill snapshot. An
/// unchanged form yields `{}` (no write). Changing `due` also sends the shown
/// time, so an ISO due's time survives as `due_time`. A locked field (see
/// `lockedTaskFields`) is never sent.
export function taskEditChanges(
  initial: TaskEditValues,
  current: TaskEditValues,
  opts: { locked: readonly LockableField[] },
): TaskEditChanges {
  const locked = new Set(opts.locked);
  const out: TaskEditChanges = {};
  const changed = (a: string, b: string) => a.trim() !== b.trim();
  if (changed(initial.title, current.title)) out.title = current.title.trim();
  if (changed(initial.status, current.status)) out.status = current.status.trim();
  if (changed(initial.priority, current.priority)) out.priority = current.priority.trim();
  const dueChanged = changed(initial.due, current.due) && !locked.has('due');
  if (dueChanged) out.due = current.due.trim();
  if (initial.dueTime !== current.dueTime || dueChanged) out.dueTime = current.dueTime;
  const before = minutes(initial.durationMin);
  const after = minutes(current.durationMin);
  if (before !== after) out.durationMin = after ?? 0;
  if (changed(initial.recurrence, current.recurrence)) out.recurrence = current.recurrence.trim();
  if (changed(initial.section, current.section)) out.section = current.section.trim();
  if (changed(initial.project, current.project)) out.project = current.project.trim();
  const labels = normalizeLabels(current.labels);
  if (normalizeLabels(initial.labels).join('\n') !== labels.join('\n')) out.labels = labels;
  if (changed(initial.description, current.description)) out.description = current.description.trim();
  if (JSON.stringify(initial.subtasks) !== JSON.stringify(current.subtasks)) out.subtasks = current.subtasks;
  // Blockers are Hollow-only (never tracker-locked); sent only when the list
  // itself changed, compared as URLs case-insensitively in order.
  const blockers = (list: string[]) => list.map(u => u.trim()).filter(Boolean);
  const blockersKey = (list: string[]) => blockers(list).map(u => u.toLowerCase()).join('\n');
  if (blockersKey(initial.blockedBy) !== blockersKey(current.blockedBy)) out.blockedBy = blockers(current.blockedBy);
  for (const key of locked) delete out[key];
  return out;
}

/// The project URL task assist may send (Paperclip PR 10): the task's stored
/// `projectUrl`, only while the Project field still shows the project the
/// sheet opened with. A changed or cleared field, a task with no stored link
/// and a new task send nothing. No matching by display name: the backend
/// never links a task by name either.
export function assistProjectUrl(
  task: Pick<Task, 'projectUrl'> | null,
  initialProject: string,
  currentProject: string,
): string | undefined {
  const url = task?.projectUrl?.trim();
  if (!url) return undefined;
  const current = currentProject.trim();
  if (!current || current !== initialProject.trim()) return undefined;
  return url;
}

/// The optimistic list patch for a saved edit: only the changed keys, plus
/// the hash of the bytes just written so the next edit's baseline is current.
export function applyTaskEditChanges(task: Task, changes: TaskEditChanges, contentHash: string): Task {
  const next: Task = { ...task, contentHash };
  if (changes.title !== undefined) next.title = changes.title;
  if (changes.status !== undefined) next.status = changes.status;
  if (changes.priority !== undefined) next.priority = (changes.priority || null) as Task['priority'];
  if (changes.due !== undefined) next.due = changes.due || null;
  if (changes.dueTime !== undefined) next.dueTime = changes.dueTime || null;
  if (changes.durationMin !== undefined) next.durationMin = changes.durationMin || null;
  if (changes.recurrence !== undefined) next.recurrence = changes.recurrence || null;
  if (changes.section !== undefined) next.section = changes.section || null;
  if (changes.project !== undefined) {
    next.projectName = changes.project || null;
    // Only a fresh read can resolve the new name to a project url.
    next.projectUrl = null;
  }
  if (changes.labels !== undefined) next.labels = changes.labels;
  if (changes.description !== undefined) next.description = changes.description;
  if (changes.subtasks !== undefined) next.subtasks = changes.subtasks;
  // Readiness and warnings are recomputed by the next list read.
  if (changes.blockedBy !== undefined) next.blockedBy = changes.blockedBy;
  return next;
}

// ── One task vocabulary (audit H02) ────────────────────────────────────────
// Every task surface (the shared TaskEditor, the list, details, filters and
// the issue detail) names priorities and statuses from here, so the
// palette's editor and the Tasks page can no longer drift apart.

/** Stored priority codes with their plain names. */
export const TASK_PRIORITIES: readonly { value: string; label: string }[] = [
  { value: 'P1', label: 'High' }, { value: 'P2', label: 'Medium' },
  { value: 'P3', label: 'Low' }, { value: 'P4', label: 'Lowest' },
];

/** "High (P1)"; an unknown stored value is shown as stored; none → "None". */
export function priorityLabel(p: string | null | undefined): string {
  const raw = (p ?? '').trim();
  if (!raw) return 'None';
  const hit = TASK_PRIORITIES.find(x => x.value.toLowerCase() === raw.toLowerCase());
  return hit ? `${hit.label} (${hit.value})` : raw;
}

/** Local task statuses; a task's own current status (e.g. one an agent or an
 *  older note wrote) stays selectable instead of being silently replaced. */
export const TASK_STATUSES: readonly string[] = ['Not started', 'In progress', 'Delivered'];
export function taskStatusOptions(current?: string | null): string[] {
  const c = (current ?? '').trim();
  return c && !TASK_STATUSES.some(s => s.toLowerCase() === c.toLowerCase()) ? [...TASK_STATUSES, c] : [...TASK_STATUSES];
}

/** The status a task is shown and grouped under. Agents and imports write
 *  the same state several ways (`todo`, `To do`, `open`, `doing`, `Done`), and
 *  grouping by the raw string put a `todo` column next to "Not started" on the
 *  Board. Known synonyms map onto the canonical three; anything else (an imported
 *  status such as "Waiting on client") keeps its own name. */
const STATUS_SYNONYMS: Record<string, string> = {
  '': 'Not started', 'not started': 'Not started', 'todo': 'Not started', 'to do': 'Not started', 'to-do': 'Not started',
  'open': 'Not started', 'new': 'Not started', 'backlog': 'Not started', 'planned': 'Not started',
  'in progress': 'In progress', 'in-progress': 'In progress', 'doing': 'In progress', 'started': 'In progress',
  'active': 'In progress', 'wip': 'In progress', 'working': 'In progress',
  'delivered': 'Delivered', 'done': 'Delivered', 'complete': 'Delivered', 'completed': 'Delivered', 'deployed': 'Delivered',
};
export function canonicalTaskStatus(status?: string | null): string {
  const raw = (status ?? '').trim();
  return STATUS_SYNONYMS[raw.toLowerCase().replace(/\s+/g, ' ')] ?? raw;
}

/** A project the editor can file a task under. `url` links it; without one
 *  the name alone is stored (as the backend does for free-text projects). */
export interface TaskProjectChoice { name: string; url?: string }
export const projectKey = (p: TaskProjectChoice) => p.url ?? `name:${p.name}`;

/** Deduplicated, sorted choices: linked projects win over a bare name, and a
 *  task's current project is always present so editing never drops it. */
export function taskProjectChoices(sources: TaskProjectChoice[], current?: TaskProjectChoice | null): TaskProjectChoice[] {
  const byName = new Map<string, TaskProjectChoice>();
  for (const p of [...sources, ...(current?.name ? [current] : [])]) {
    const name = p.name.trim(); if (!name) continue;
    const key = name.toLowerCase();
    const prev = byName.get(key);
    if (!prev || (!prev.url && p.url)) byName.set(key, { name, ...(p.url ? { url: p.url } : {}) });
  }
  return [...byName.values()].sort((a, b) => a.name.localeCompare(b.name));
}

/** Why the editor cannot save yet, or null. Same rules on every surface. */
export function taskFormProblem(v: { title: string; dueTime: string }, timeParsed: string | null): string | null {
  if (!v.title.trim()) return 'Add a task name.';
  if (v.dueTime.trim() && !timeParsed) return `Couldn’t read the time “${v.dueTime.trim()}”. Try 4pm or 16:00.`;
  return null;
}
