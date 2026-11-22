// The activity dock: in-flight and recent slow operations (startTask,
// finishTask, failTask, trackTask). No store imports.
// Moved verbatim out of store/runtime.ts (plan L3 X19, 2026-10-14); runtime.ts
// re-exports every symbol, so `store/runtime` imports keep working.
import { createSignal } from 'solid-js';
import { isExpectedCancellation } from '../toastPolicy';
import { errMsg } from '../utils';
import { openWorkView } from './destinations';

// ─────────────────────────────────────────────────────────────
// ACTIVITY DOCK — append-only log of slow / remote operations
// (Claude calls, MCP writes, vault writes, calendar fetches).
// The dock surfaces in-flight work and a rolling audit trail so
// nothing silently spins or silently fails. Session-scoped; not
// persisted across reloads.
// ─────────────────────────────────────────────────────────────

/// `warning` is a soft-failure status: the task technically completed
/// (didn't throw), but the result indicates partial failure — e.g. an
/// Outlook bulk archive where some messages failed on Graph or some
/// local vault writes failed. The UI surfaces these in amber rather
/// than the green checkmark used for clean success. `cancelled` (Paperclip
/// PR 22) is an expected stop, shown neutrally as "Stopped".
export type ActivityStatus = 'pending' | 'success' | 'warning' | 'error' | 'cancelled';

export interface ActivityTask {
  id: string;
  name: string;
  status: ActivityStatus;
  startedAt: number;
  finishedAt?: number;
  /** Short context shown beneath the name (e.g. "via M365 MCP"). */
  detail?: string;
  /** Result on success (1-line summary) or error message on failure. */
  result?: string;
}

const ACTIVITY_MAX = 100;
export const [activityTasks, setActivityTasks] = createSignal<ActivityTask[]>([]);
export const [activityDockOpen, setActivityDockOpen] = createSignal(false);

/** Manage live work in its visible home, through the existing navigation guard. */
export function openRunningActivity() {
  return openWorkView('activity', () => setActivityDockOpen(false));
}

export function startTask(name: string, detail?: string): string {
  const id = `t_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`;
  const task: ActivityTask = { id, name, detail, status: 'pending', startedAt: Date.now() };
  setActivityTasks(prev => [task, ...prev].slice(0, ACTIVITY_MAX));
  return id;
}

export function finishTask(id: string, result?: string, status: 'success' | 'warning' | 'cancelled' = 'success') {
  setActivityTasks(prev => prev.map(t =>
    t.id === id ? { ...t, status, finishedAt: Date.now(), result } : t
  ));
}

export function failTask(id: string, error: string) {
  setActivityTasks(prev => prev.map(t =>
    t.id === id ? { ...t, status: 'error', finishedAt: Date.now(), result: error } : t
  ));
}

/** Wrap an async call so it shows up in the activity dock with
 *  automatic pending → success / warning / error transitions. Re-throws
 *  so callers can still react to a thrown failure.
 *
 *  `onSuccessStatus` lets the caller flag a non-throwing result as a
 *  partial failure (e.g. bulk archive where 1 of 5 messages failed on
 *  Outlook), or as an expected stop (`cancelled`). Default is always
 *  `success`. A thrown expected cancellation (a queued run cancelled before
 *  it started, a stop while waiting for an agent slot) ends the row as a
 *  neutral "Stopped" instead of an error, and is still re-thrown. */
export async function trackTask<T>(
  name: string,
  fn: () => Promise<T>,
  opts?: {
    detail?: string;
    onSuccessResult?: (value: T) => string | undefined;
    onSuccessStatus?: (value: T) => 'success' | 'warning' | 'cancelled';
  },
): Promise<T> {
  const id = startTask(name, opts?.detail);
  try {
    const value = await fn();
    const status = opts?.onSuccessStatus?.(value) ?? 'success';
    if (status === 'cancelled') finishTask(id, 'Stopped', 'cancelled');
    else finishTask(id, opts?.onSuccessResult?.(value), status);
    return value;
  } catch (e) {
    if (isExpectedCancellation(e)) finishTask(id, 'Stopped', 'cancelled');
    else failTask(id, errMsg(e));
    throw e;
  }
}

export function clearCompletedTasks() {
  setActivityTasks(prev => prev.filter(t => t.status === 'pending'));
}
