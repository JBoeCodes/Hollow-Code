// The run queue's live-row helpers: whether a row is live, how many wait, and
// whether the top bar mounts the activity dock. Split out of runQueue.ts (ST1,
// 2026-10-14) so the launch path imports only these few lines; runQueue.ts
// re-exports them for everything else.
import type { RunQueueItem } from './types';

export function isLive(item: Pick<RunQueueItem, 'status'>): boolean {
  return item.status === 'queued' || item.status === 'claimed' || item.status === 'running';
}

/** A chat row that reads Working, never Queued (2026-11-16): claimed or
 *  running, or queued with nothing that truly holds it back (`working`: the
 *  pump is about to claim it, or only the app's own bookkeeping, a tracker
 *  sync write phase, delays it). Admission, recall, retrieval and the
 *  agent's boot all count as working ("Preparing"). Queued is for a row that
 *  truly can't start: a chat-count limit, memory, another run's folder or
 *  lane, a retry time. */
export function readsWorking(item: Pick<RunQueueItem, 'source' | 'status' | 'working'>): boolean {
  if (item.source !== 'chat') return false;
  return item.status === 'claimed' || item.status === 'running' || (item.status === 'queued' && !!item.working);
}

/** The rows that truly wait (queued, or a workflow claimed and about to
 *  start); a chat row that reads Working and running rows are not
 *  "queued". Drives the dock's mount. */
export function queuedCount(items: readonly RunQueueItem[]): number {
  return items.filter(item => (item.status === 'queued' || item.status === 'claimed') && !readsWorking(item)).length;
}

/** Running automation rows (Paperclip PR 15b): the dock shows their quiet
 *  time and a Stop. A running chat turn is left out; the composer already
 *  shows its quiet time and Stop. */
export function runningAutomations(items: readonly RunQueueItem[]): RunQueueItem[] {
  return items.filter(item => item.status === 'running' && item.source !== 'chat');
}

/** Whether the top bar mounts the activity dock: in-process tasks, rows
 *  waiting to start, or a running automation (Paperclip PR 15b; before it the
 *  dock hid while an automation ran alone, so its Stop was unreachable). */
export function dockVisible(items: readonly RunQueueItem[], taskCount: number): boolean {
  return taskCount > 0 || queuedCount(items) > 0 || runningAutomations(items).length > 0;
}
