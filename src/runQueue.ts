// Pure helpers for the durable automation queue (Paperclip PR 7). No store or
// Tauri import, so the node vitest suite can load it.
import type { RunQueueItem, RunQueueSource } from './types';
import { permissionHoldText } from './workflowPermissions';
import { isLive, readsWorking } from './runQueueLive';
import { queuedBadgeText } from './chatQueue';

/** Mirrors Rust `vault_workflows::PERMISSION_REFUSED_REASON`: the held row's
 *  note while its settings change awaits review (pinned by the Rust test
 *  `the_frontend_mirrors_the_hold_and_stale_holder_strings`). */
export const PERMISSION_REFUSED_NOTE =
  'automation settings changed; review them in Automations → Permission changes';
/** Mirrors Rust `vault_workflows::PERMISSION_RECORDS_UNAVAILABLE_REASON`. */
export const PERMISSION_UNAVAILABLE_NOTE =
  'automation permission records could not be read; repair or move them, then restart Hollow';
/** Mirrors Rust `LeaseHolder::Stale.describe()`. */
export const STALE_HOLDER = 'a stale lease left by an earlier run';

const WORKFLOW_SOURCES: RunQueueSource[] = ['cron', 'manual', 'trigger', 'retry', 'restrictedResume'];

// The dock's live-row helpers live in runQueueLive.ts so the top bar (in the
// App chunk) doesn't pull this module's hold/permission text along; they are
// re-exported here unchanged (ST1, 2026-10-14).
export { isLive, queuedCount, readsWorking, runningAutomations, dockVisible } from './runQueueLive';

/** The live queue row for a workflow slug, running first, then claimed, then
 *  the earliest queued; null when the workflow has none. */
export function liveItemForSlug(items: readonly RunQueueItem[], slug: string): RunQueueItem | null {
  const rank = (item: RunQueueItem) => (item.status === 'running' ? 0 : item.status === 'claimed' ? 1 : 2);
  const matches = items
    .filter(item => isLive(item) && item.sourceId === slug && WORKFLOW_SOURCES.includes(item.source))
    .sort((a, b) => rank(a) - rank(b) || a.id - b.id);
  return matches[0] ?? null;
}

/** Why a row is paused (a permission hold), in the same words as the
 *  workflow's own next-run text; null when it is not held. */
export function heldText(heldReason: string | null): string | null {
  if (!heldReason) return null;
  if (heldReason === PERMISSION_REFUSED_NOTE) return permissionHoldText('review');
  if (heldReason === PERMISSION_UNAVAILABLE_NOTE) return permissionHoldText('unavailable');
  return `paused: ${heldReason}`;
}

/** `heldText` as a line of its own (capitalised), as the dock shows it. */
export function heldLine(heldReason: string | null): string | null {
  const held = heldText(heldReason);
  return held ? held[0].toUpperCase() + held.slice(1) : null;
}

/** The label on a chat bubble that truly can't start: the dock's own paused
 *  line for a permission hold, so the two agree (2026-11-17), else Queued
 *  with its wait. */
export function queuedTurnLabel(q: { waitingFor: string | null; heldReason?: string | null }): string {
  return heldLine(q.heldReason ?? null) ?? queuedBadgeText(q);
}

/** One line on where a row stands: running, starting, paused for review,
 *  waiting for a named writer (with the restart hint behind a stale lease),
 *  or its place in line. A chat row nothing truly holds back reads
 *  "Preparing", never "Queued". */
export function waitingText(item: RunQueueItem): string {
  if (item.status === 'running') return 'Running';
  if (item.status === 'claimed') return item.cancelRequested ? 'Stopping' : 'Preparing';
  if (item.status === 'paused') return 'Paused for requested work';
  if (!isLive(item)) return item.status;
  const held = heldLine(item.heldReason);
  if (held) return held;
  if (item.cancelRequested) return 'Skipping this occurrence';
  // A chat nothing truly holds back is working, not queued (2026-11-16).
  if (readsWorking(item)) return 'Preparing';
  if (item.waitingHolder) {
    if (item.waitingHolder === STALE_HOLDER) {
      return `Waiting for ${STALE_HOLDER} (restart Hollow to reconcile it)`;
    }
    return `Waiting for ${item.waitingHolder}`;
  }
  if (item.waitingReason) return `Waiting for ${item.waitingReason}`;
  return item.position ? `Queued #${item.position}` : 'Queued';
}

/** The chip on a workflow row with a live queue row. A permission hold is
 *  the workflow's own next-run text and outranks it, so callers skip the
 *  chip for a held workflow. */
export function queueChipText(item: RunQueueItem): string {
  if (item.status === 'running') return 'Running';
  const text = waitingText(item);
  return text.startsWith('Queued') ? text : `Queued · ${text}`;
}

const SOURCE_LABELS: Record<RunQueueSource, string> = {
  cron: 'Scheduled',
  manual: 'Run now',
  trigger: 'Triggered',
  retry: 'Retry',
  restrictedResume: 'Continue after requested work',
  heartbeat: 'Heartbeat',
  chat: 'Chat',
  ingest: 'Import',
};

export function sourceLabel(source: RunQueueSource): string {
  return SOURCE_LABELS[source] ?? source;
}

/** The action a row offers, or null: Stop a running workflow, Skip a queued
 *  cron occurrence (it is still recorded), Cancel anything else waiting. A
 *  running heartbeat cannot be stopped. */
export function cancelLabel(item: RunQueueItem): 'Stop' | 'Skip' | 'Cancel' | null {
  if (item.status === 'running') return WORKFLOW_SOURCES.includes(item.source) ? 'Stop' : null;
  if (item.cancelRequested) return null;
  if (item.status === 'claimed') return 'Stop';
  if (item.status === 'queued') return item.source === 'cron' ? 'Skip' : 'Cancel';
  return null;
}

/** What a stop did: told the row to stop (or took it out of the queue),
 *  found it had already ended, the person said no, or there was no queue
 *  row to stop by, so nothing was stopped (never a stop by key). */
export type RowStop = 'stopped' | 'ended' | 'declined' | 'noRow';

/** The notice for a turn with no queue row (`RowStop` `noRow`). */
export const NO_ROW_NOTICE = 'That turn has no queue row any more, so nothing was stopped. Open its chat to stop it.';

/** The note for a stop that stopped nothing (`noRow`, `ended`), else null.
 *  Here, not in Mission Control, to keep that lazy chunk in its budget. */
export const rowStopNote = (done: RowStop, title: string) =>
  done === 'noRow' ? NO_ROW_NOTICE : done === 'ended' ? `“${title}” had already ended, so nothing was stopped.` : null;

/** Stop one run-queue row once the person confirms, by its row id, never by
 *  its conversation key: while a confirm is open the turn can end and a held
 *  message or an agent's send_to_session can start the next one on the same
 *  key, and a stop by key would end that one. `cancel_queued_run` checks the
 *  row (a running chat turn under its stop slot's lock), so a row that ended
 *  meanwhile stops nothing and reads `ended` (overnight 11-13). */
export async function stopRowAfterConfirm(id: number, confirm: () => Promise<boolean>, cancel: (id: number) => Promise<RunQueueItem>): Promise<RowStop> {
  if (!(await confirm())) return 'declined';
  const item = await cancel(id);
  return isLive(item) || item.status === 'cancelled' || item.status === 'skipped' ? 'stopped' : 'ended';
}
