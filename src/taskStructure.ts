// Pure task-structure helpers (Paperclip PR 11). Dependency-free so the node
// vitest suite can load them.

// ── Blockers (Paperclip PR 11b) ───────────────────────────────────────────

/** The task fields the blocker helpers read (a `Task` satisfies it). */
export interface BlockerTask {
  url: string;
  title: string;
  status: string;
  recurrence: string | null;
  blockedBy: string[];
}

/** One `blocker_warnings` entry (Rust `BlockerWarning`). */
export interface BlockerWarningLike {
  url: string;
  kind: 'missing' | 'cancelled' | 'recurring' | 'cycle';
}

const fold = (url: string) => url.trim().toLowerCase();

/** Mirrors Rust `is_terminal_status`. */
export function isFinishedStatus(status: string): boolean {
  return ['delivered', 'done', 'complete', 'completed'].includes(status.trim().toLowerCase());
}

/** Mirrors Rust `is_cancelled_status`: cancelled / canceled / won't do /
 *  wont do, any case, straight or curly apostrophe. Lives in taskStatus.ts
 *  beside `isOpenTask` (P1-12). */
import { isCancelledStatus } from './taskStatus';
export { isCancelledStatus };

/**
 * Would making `candidateUrl` a blocker of `selfUrl` close a loop? True when
 * the candidate is the task itself or waits, through `blockedBy` links
 * (compared case-insensitively, like the backend's resolver), on it.
 */
export function wouldCycle(tasks: readonly BlockerTask[], selfUrl: string, candidateUrl: string): boolean {
  const byUrl = new Map<string, BlockerTask>();
  for (const t of tasks) if (!byUrl.has(fold(t.url))) byUrl.set(fold(t.url), t);
  const target = fold(selfUrl);
  const seen = new Set<string>([fold(candidateUrl)]);
  const stack = [fold(candidateUrl)];
  while (stack.length) {
    const url = stack.pop()!;
    if (url === target) return true;
    for (const next of byUrl.get(url)?.blockedBy ?? []) {
      const key = fold(next);
      if (!seen.has(key)) {
        seen.add(key);
        stack.push(key);
      }
    }
  }
  return false;
}

/** Why a picker candidate is sorted last, or null. */
export function blockerCandidateNote(t: BlockerTask): string | null {
  if (isCancelledStatus(t.status)) return 'cancelled: would never resolve';
  if (t.recurrence) return 'repeats: Hollow rolls it forward, so it never finishes';
  return null;
}

export interface BlockerCandidate<T extends BlockerTask = BlockerTask> {
  task: T;
  note: string | null;
}

/**
 * The tasks a "Blocked by" picker offers, best first. Excludes the task
 * itself, the tasks already chosen, finished tasks (nothing to wait on) and
 * any task that would close a loop (`selfUrl` null for a new task, which
 * nothing can wait on yet). A non-empty `query` matches titles
 * case-insensitively. Open tasks come first; recurring and cancelled ones
 * sort last with a note, since they never resolve on their own.
 */
export function blockerCandidates<T extends BlockerTask>(
  tasks: readonly T[],
  selfUrl: string | null,
  chosen: readonly string[],
  query = '',
  limit = 8,
): BlockerCandidate<T>[] {
  const picked = new Set(chosen.map(fold));
  const q = query.trim().toLowerCase();
  const out = tasks
    .filter(t => !selfUrl || fold(t.url) !== fold(selfUrl))
    .filter(t => !picked.has(fold(t.url)))
    .filter(t => !isFinishedStatus(t.status))
    .filter(t => !q || t.title.toLowerCase().includes(q))
    .filter(t => !selfUrl || !wouldCycle(tasks, selfUrl, t.url))
    .map(task => ({ task, note: blockerCandidateNote(task) }));
  out.sort((a, b) => Number(a.note !== null) - Number(b.note !== null) || a.task.title.localeCompare(b.task.title));
  return out.slice(0, limit);
}

/** The display name of a blocker URL: its task's title, else the file name. */
export function blockerLabel(tasks: readonly BlockerTask[], url: string): string {
  const hit = tasks.find(t => fold(t.url) === fold(url));
  if (hit) return hit.title;
  const name = url.replace(/^vault:/, '').split('/').pop() ?? url;
  return name.replace(/\.md$/i, '');
}

/** The sentence a never-resolving blocker (or a loop) shows. */
export function blockerWarningText(warning: BlockerWarningLike, name: string): string {
  switch (warning.kind) {
    case 'missing':
      return `No task at ${warning.url.replace(/^vault:/, '')} (renamed, moved or deleted), so this blocker will never resolve.`;
    case 'cancelled':
      return `${name} is cancelled, so this blocker will never resolve.`;
    case 'recurring':
      return `${name} repeats: Hollow rolls it forward instead of finishing it, so this blocker never resolves on its own.`;
    case 'cycle':
      return `${name} waits on this task, directly or through others, so neither can start.`;
  }
}
