// Paperclip PR 15 (15b): the silent-run indicator's pure helpers. No store or
// Tauri import (`import type` erases), so the node vitest suite can load it.
//
// A running chat turn or automation reports `activity` every 15 s: when its
// agent last printed anything (`lastEventAt`, its spawn before the first
// line) and the progress phase of its latest tool call. The dock attaches a
// workflow run's activity to its existing running queue row by
// `historyRunId`; the composer shows the chat turn's. Informational only.
import type { ProgressPhase, RunQueueItem, RunQueueSource } from './types';

/** No output for this long before the indicator says "quiet". */
export const QUIET_AFTER_MS = 2 * 60_000;

export interface RunActivityEntry {
  lastEventAt: string | null;
  progress: ProgressPhase | null;
}

/** runId → the run's latest activity. */
export type ActivityMap = ReadonlyMap<string, RunActivityEntry>;

/** The chat turn in flight's activity. */
export interface ChatActivity extends RunActivityEntry {
  runId: string;
}

/** What the reducers read from a `workflow-event` or `chat-event`. */
export type ActivityEventLike =
  | { type: 'activity'; runId: string; lastEventAt: string | null; progress: ProgressPhase | null }
  | { type: 'done'; runId?: string | null }
  | { type: string; runId?: string | null };

const PROGRESS_LABELS: Record<ProgressPhase, string> = {
  researching: 'Researching',
  usingTools: 'Using tools',
  workingWithFiles: 'Working with files',
  coordinating: 'Coordinating',
};

/** "quiet 6m" / "quiet 1h 5m" once nothing was printed for two minutes;
 *  null before that, without a time, or for an unreadable one. */
export function quietLabel(lastEventAt: string | null | undefined, now: number): string | null {
  if (!lastEventAt) return null;
  const at = Date.parse(lastEventAt);
  if (!Number.isFinite(at)) return null;
  const quiet = now - at;
  if (quiet < QUIET_AFTER_MS) return null;
  const minutes = Math.floor(quiet / 60_000);
  if (minutes < 60) return `quiet ${minutes}m`;
  return `quiet ${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

/** A person's words for a progress phase; null for none or an unknown one. */
export function progressLabel(progress: ProgressPhase | string | null | undefined): string | null {
  if (!progress) return null;
  return PROGRESS_LABELS[progress as ProgressPhase] ?? null;
}

function isActivity(evt: ActivityEventLike): evt is Extract<ActivityEventLike, { type: 'activity' }> {
  return evt.type === 'activity' && typeof evt.runId === 'string' && evt.runId !== '';
}

/** Fold one event in (a new map when it changes anything): an `activity`
 *  upserts its run, so a run started before a reload is picked up on its
 *  next tick; a `done` drops its run. */
export function applyActivityEvent(map: ActivityMap, evt: ActivityEventLike): ActivityMap {
  if (isActivity(evt)) {
    const next = new Map(map);
    next.set(evt.runId, { lastEventAt: evt.lastEventAt ?? null, progress: evt.progress ?? null });
    return next;
  }
  if (evt.type === 'done' && evt.runId && map.has(evt.runId)) {
    const next = new Map(map);
    next.delete(evt.runId);
    return next;
  }
  return map;
}

/** Drop runs with no live queue row (a `done` that was missed, a run that
 *  ended while the window was closed). Returns the same map when nothing
 *  was dropped. */
export function pruneActivity(map: ActivityMap, liveRunIds: Iterable<string>): ActivityMap {
  const live = new Set(liveRunIds);
  if ([...map.keys()].every(runId => live.has(runId))) return map;
  return new Map([...map].filter(([runId]) => live.has(runId)));
}

/** The run ids of the queue's running rows. */
export function liveRunIds(items: readonly RunQueueItem[]): string[] {
  return items
    .filter(item => item.status === 'running' && !!item.historyRunId)
    .map(item => item.historyRunId as string);
}

/** A queue row's activity, by its `historyRunId`. */
export function activityFor(map: ActivityMap, runId: string | null | undefined): RunActivityEntry | null {
  if (!runId) return null;
  return map.get(runId) ?? null;
}

/** The chat turn's activity after an `activity` chat event: only the turn in
 *  flight's (`liveRunId`, set at its `runStarted`); a foreign or late run's
 *  event changes nothing. */
export function nextChatActivity(
  current: ChatActivity | null,
  liveRunId: string | null,
  evt: ActivityEventLike,
): ChatActivity | null {
  if (!isActivity(evt) || !liveRunId || evt.runId !== liveRunId) return current;
  return { runId: evt.runId, lastEventAt: evt.lastEventAt ?? null, progress: evt.progress ?? null };
}

const STOPPABLE: RunQueueSource[] = ['cron', 'manual', 'trigger', 'retry', 'restrictedResume', 'chat'];

/** Whether the dock offers Stop on a row: a running workflow (it stops like
 *  the Workflows page's Stop) or chat turn (like the composer's Stop), both
 *  through `cancel_queued_run`. A running heartbeat cannot be stopped. */
export function canStopRunning(item: Pick<RunQueueItem, 'status' | 'source'>): boolean {
  return item.status === 'running' && STOPPABLE.includes(item.source);
}

/** The dock row's one-line activity: progress (or "Running" beside a quiet
 *  time), then quiet time; null when there is nothing to say. */
export function activityLine(entry: RunActivityEntry | null, now: number): string | null {
  if (!entry) return null;
  const quiet = quietLabel(entry.lastEventAt, now);
  const verb = progressLabel(entry.progress) ?? (quiet ? 'Running' : null);
  const parts = [verb, quiet].filter((part): part is string => !!part);
  return parts.length > 0 ? parts.join(' · ') : null;
}
