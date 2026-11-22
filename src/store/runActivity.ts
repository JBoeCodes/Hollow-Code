/** Running automations' activity (Paperclip PR 15b): runId → when the agent
 * last printed anything and its progress phase, from the backend's
 * `workflow-event` `activity` ticks (every 15 s).
 *
 * A plain signal, never a resource, like `runQueue`. The dock reads it by a
 * running queue row's `historyRunId`, so it adds no second list: an activity
 * with no live row is pruned whenever the queue list changes, and a run
 * started before a reload (no `started` seen) is picked up on its next tick.
 * The chat turn's activity lives in `store/chatRuntime.ts` (`chatActivity`). */
import { createEffect, createRoot, createSignal } from 'solid-js';
import { listen } from '../nativeEvents';
import type { WorkflowEvent } from '../types';
import { applyActivityEvent, liveRunIds, pruneActivity, type ActivityMap } from '../runActivity';
import { runQueue } from './runQueue';

export const [workflowActivity, setWorkflowActivity] = createSignal<ActivityMap>(new Map());

listen<WorkflowEvent>('workflow-event', e => {
  setWorkflowActivity(map => applyActivityEvent(map, e.payload));
}).catch(() => { /* not in tauri */ });

createRoot(() => {
  createEffect(() => {
    const live = liveRunIds(runQueue());
    setWorkflowActivity(map => pruneActivity(map, live));
  });
});
