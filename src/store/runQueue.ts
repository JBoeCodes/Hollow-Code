/** The durable automation queue's client state (Paperclip PR 7).
 *
 * A plain signal, never a `createResource`: `run-queue-event` fires on every
 * enqueue, wait and finish, and a resource read under the page Suspense would
 * blank the page on each refetch (PR 5 shipped exactly that bug in the review
 * panels). Every event schedules one debounced refetch; responses that arrive
 * out of order are dropped. */
import { createSignal } from 'solid-js';
import { listen } from '../nativeEvents';
import { api } from '../api';
import type { RunQueueEvent, RunQueueItem } from '../types';
import { chatQueuedTurn, isPreparingTurn, reconcileQueuedChatRows } from './runtime';
import { latestRequest } from '../latestRequest';

/** Finished rows shown after the live ones. */
export const RUN_QUEUE_RECENT = 20;
/** Events arrive in bursts (enqueue, waiting, running); refetch once. */
export const RUN_QUEUE_DEBOUNCE_MS = 250;

export const [runQueue, setRunQueue] = createSignal<RunQueueItem[]>([]);

export const [runQueueRead, setRunQueueRead] = createSignal({loaded:false, error:false});

let timer: ReturnType<typeof setTimeout> | null = null;
const reads = latestRequest();

async function fetchRunQueue() {
  const seq = reads.start();
  try {
    const items = await api.listRunQueue(RUN_QUEUE_RECENT);
    if (reads.isLatest(seq)) {
      setRunQueue(items);
      setRunQueueRead({loaded:true, error:false});
      // Paperclip PR 7b: adopt a chat turn queued before a reload, release
      // one that ended unseen, and put a turn the previous Hollow process
      // queued but never started back into the composer (once).
      reconcileQueuedChatRows(items);
    }
  } catch {
    if (reads.isLatest(seq)) setRunQueueRead(s => ({...s, error:true}));
    // Keep the last list; the next event or mount retries.
  }
}

/** Refetch the queue after `delayMs` (debounced: a burst of calls fetches
 *  once). `0` fetches on the next tick. */
export function refreshRunQueue(delayMs = RUN_QUEUE_DEBOUNCE_MS) {
  if (timer !== null) clearTimeout(timer);
  timer = setTimeout(() => {
    timer = null;
    void fetchRunQueue();
  }, delayMs);
}

listen<RunQueueEvent>('run-queue-event', () => refreshRunQueue()).catch(() => { /* not in tauri */ });

/** While a sent turn is Preparing (accepted, nothing holds it, not started),
 *  look again every few seconds: a hold that begins after the send (the
 *  memory gate, another chat taking the last slot) emits no queue event, so
 *  without this the badge would keep saying Preparing for a row that now
 *  truly waits (2026-11-16, Working, not Queued). */
export const PREPARING_RECHECK_MS = 3000;
const preparingCheck = setInterval(() => {
  try {
    if (isPreparingTurn(chatQueuedTurn())) refreshRunQueue(0);
  } catch { /* the chat store isn't loaded (a test that mocks it): nothing to recheck */ }
}, PREPARING_RECHECK_MS);
(preparingCheck as { unref?: () => void }).unref?.();
