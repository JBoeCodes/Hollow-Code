import { createSignal } from 'solid-js';
import { api } from './api';
import {
  flashFooter, flashWithUndo, refetchTasks, refetchProjects,
  tasksData, mutateTasks, projectsData, mutateProjects, trackTask,
} from './store';
import type { Priority } from './types';
import { isTaskConflict, taskErrorText } from './utils';

// The conversation drag contract (MIME type, payload, drop-handler registry).
export * from './sessionDrag';

/// Record the hash of the bytes a task write just produced, so an editor
/// opened before the (unawaited) refetch lands holds the current baseline
/// instead of conflicting with the user's own write.
function patchTaskHash(url: string, contentHash: string) {
  const snap = tasksData();
  if (!snap) return;
  mutateTasks({ ...snap, items: snap.items.map(t => (t.url === url ? { ...t, contentHash } : t)) });
}

/// Status strings that mark the work as "done": the single source of truth the Tasks views use to pull
/// completed tasks into the Completed group / Delivered column. Includes both
// `isCompletionStatus` lives in taskStatus.ts (pure, so modules that must not
// pull in the store can use it); re-exported here for existing callers.
export { isCompletionStatus } from './taskStatus';
import { isCompletionStatus } from './taskStatus';

export interface DragPayload {
  kind: 'task' | 'project';
  url: string;
  originStatus?: string | null;
  originPriority?: string | null;
  /** Due date (`YYYY-MM-DD`) at drag start — the Tasks Calendar view's
   *  reschedule undo target. */
  originDue?: string | null;
}

export const [dragPayload, setDragPayload] = createSignal<DragPayload | null>(null);

// Helper for drop zones — call this on drop after preventDefault.
// Applies the change to local cached data immediately (optimistic),
// then fires the backend mutation. On failure the cache is restored
// from the pre-drop snapshot.
export async function applyDrop(args: {
  kind: 'task' | 'project';
  url: string;
  dimension: 'status' | 'priority' | 'due';
  value: string | null;
  originValue: string | null | undefined;
}) {
  if (args.value === args.originValue) return;
  const labelTxt = args.value === null || args.value === ''
    ? (args.dimension === 'due' ? 'no date' : 'unprioritized')
    : args.value;

  // Snapshot + optimistic mutation.
  const taskSnap = args.kind === 'task' ? tasksData() : null;
  const projectSnap = args.kind === 'project' ? projectsData() : null;
  if (args.kind === 'task' && taskSnap) {
    mutateTasks({
      ...taskSnap,
      items: taskSnap.items.map(t => {
        if (t.url !== args.url) return t;
        if (args.dimension === 'status') return { ...t, status: args.value ?? '' };
        if (args.dimension === 'due') return { ...t, due: args.value || null };
        return { ...t, priority: (args.value as Priority | null) };
      }),
    });
  } else if (args.kind === 'project' && projectSnap) {
    mutateProjects({
      ...projectSnap,
      items: projectSnap.items.map(p =>
        p.url === args.url ? { ...p, status: args.value ?? '' } : p,
      ),
    });
  }
  flashFooter(`✓ ${labelTxt}`, 900);

  const taskName = `${args.kind === 'task' ? 'Task' : 'Project'} ${args.dimension} → ${labelTxt}`;
  try {
    const written = await trackTask(
      taskName,
      async (): Promise<string | undefined> => {
        // Every task write is compare-and-swap against the row the person
        // dragged (review F01): a newer edit elsewhere is refused, not lost.
        const expected = taskSnap?.items.find(t => t.url === args.url)?.contentHash || undefined;
        if (args.kind === 'task' && args.dimension === 'status') {
          return await api.setTaskStatus(args.url, args.value!, expected);
        } else if (args.kind === 'task' && args.dimension === 'priority') {
          return await api.setTaskPriority(args.url, args.value || null, expected);
        } else if (args.kind === 'task' && args.dimension === 'due') {
          return (await api.updateTask({ url: args.url, expected, due: args.value ?? '' })).contentHash;
        } else if (args.kind === 'project' && args.dimension === 'status') {
          await api.setProjectStatus(args.url, args.value!);
        }
        return undefined;
      },
      { detail: 'Vault update' },
    );
    if (args.kind === 'task' && written) patchTaskHash(args.url, written);
    // Reconcile against the vault — it's canonical, the optimistic
    // UI update is what the user sees first.
    if (args.kind === 'task') { Promise.resolve(refetchTasks()).catch(() => {}); }
    else { Promise.resolve(refetchProjects()).catch(() => {}); }

    // Completing a repeating task rolled it to its next date; putting the
    // status back would not restore that date, so there is no Undo to offer
    // (as the bulk plan's notUndoable says).
    const dropped = taskSnap?.items.find(t => t.url === args.url);
    if (args.kind === 'task' && args.dimension === 'status' && dropped?.recurrence && isCompletionStatus(args.value ?? '')) {
      flashFooter('✓ Completed; it repeats, so it moved to its next date', 2500);
      return;
    }
    // Offer an undo: send the SAME mutation back with the original value.
    // Skip when there's no original (creating from nothing), and skip the
    // ambient "✓ <label>" footer flash from earlier — the snackbar replaces it.
    const originLabel = args.originValue === null || args.originValue === undefined || args.originValue === ''
      ? (args.dimension === 'due' ? 'no date' : 'unprioritized')
      : args.originValue;
    flashWithUndo({
      message: `${args.kind === 'task' ? 'Task' : 'Project'} ${args.dimension} → ${labelTxt}`,
      kind: args.dimension === 'status' ? 'status' : args.dimension === 'due' ? 'move' : 'priority',
      undo: async () => {
        try {
          const undone = await trackTask(
            `Undo: ${args.kind === 'task' ? 'task' : 'project'} ${args.dimension} → ${originLabel}`,
            async (): Promise<string | undefined> => {
              // Undo only what this drop wrote: a later edit wins (TASK_CONFLICT).
              if (args.kind === 'task' && args.dimension === 'status') {
                return await api.setTaskStatus(args.url, (args.originValue ?? '') as string, written);
              } else if (args.kind === 'task' && args.dimension === 'priority') {
                return await api.setTaskPriority(args.url, (args.originValue ?? null) as Priority | null, written);
              } else if (args.kind === 'task' && args.dimension === 'due') {
                return (await api.updateTask({ url: args.url, expected: written, due: (args.originValue ?? '') as string })).contentHash;
              } else if (args.kind === 'project' && args.dimension === 'status') {
                await api.setProjectStatus(args.url, (args.originValue ?? '') as string);
              }
              return undefined;
            },
            { detail: 'Vault undo' },
          );
          if (args.kind === 'task' && undone) patchTaskHash(args.url, undone);
          if (args.kind === 'task') { refetchTasks(); } else { refetchProjects(); }
        } catch (e) {
          if (args.kind === 'task') Promise.resolve(refetchTasks()).catch(() => {});
          flashFooter(isTaskConflict(e)
            ? '× Not undone: the task changed after this edit, so it was left as it is.'
            : `× undo failed: ${taskErrorText(e)}`, 3500);
        }
      },
    });
  } catch (e) {
    // Independent task ticks may finish out of order. Undo only this field's
    // optimistic value, never another task's result or a newer file version.
    if (args.kind === 'task' && taskSnap) {
      const current = tasksData();
      const before = taskSnap.items.find(t => t.url === args.url);
      const optimistic = args.dimension === 'status' ? args.value ?? '' : args.dimension === 'due' ? args.value || null : args.value;
      if (current && before) mutateTasks({ ...current, items: current.items.map(t =>
        t.url === args.url && t.contentHash === before.contentHash && t[args.dimension] === optimistic
          ? { ...t, [args.dimension]: before[args.dimension] } : t) });
    }
    else if (args.kind === 'project' && projectSnap) mutateProjects(projectSnap);
    flashFooter(`× ${taskErrorText(e) || 'save failed'}`, 3000);
  }
}

// The conversation-row drag handlers (sessionDragSource, sessionDropTarget)
// live in sessionDragHandlers.ts so the rail can use them without this
// module's task drop; re-exported here for everyone else.
export { SESSION_DROP_CLASS, sessionDragSource, sessionDropTarget } from './sessionDragHandlers';
