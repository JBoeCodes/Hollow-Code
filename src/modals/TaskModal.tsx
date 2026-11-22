import { Show, createSignal, lazy, onCleanup, onMount } from 'solid-js';
import { createExitPhase } from '../components/exit';
// The editor loads on first open (next-level P21); Tasks shares the chunk.
const TaskEditor = lazy(() => import('../components/TaskEditor').then(m => ({ default: m.TaskEditor })));
export const preloadTaskEditor = () => import('../components/TaskEditor');
import { enableTasksResource, enableProjectsResource } from '../store';
import type { Task } from '../types';

const [open, setOpen] = createSignal(false);
const [editing, setEditing] = createSignal<Task | null>(null);
/** A new task's starting values (next-level P08: a selection becomes a task). */
export type TaskPrefill = { title?: string; description?: string; project?: string };
const [prefill, setPrefill] = createSignal<TaskPrefill | undefined>(undefined);
// Bumped per open so the editor remounts with a fresh prefill.
const [generation, setGeneration] = createSignal(0);

/// Open the task sheet (command palette, global surfaces). Pass a Task to
/// edit it; omit to create one. Renders the same TaskEditor as the Tasks
/// page's side panel (audit H02).
export function openModal(task?: Task, initial?: TaskPrefill) {
  setEditing(task ?? null);
  setPrefill(task ? undefined : initial);
  // Opened from surfaces that never mount Tasks: load projects (Project
  // choices) and tasks (Blocked-by candidates) on demand.
  enableProjectsResource();
  enableTasksResource();
  setGeneration(g => g + 1);
  setOpen(true);
}

export function TaskModal() {
  let restore: HTMLElement | null = null;
  // Close only flips `open`; the editor survives the 150ms exit hold so the
  // sheet doesn't blank mid-fade. Focus returns to where it was opened from.
  function close() { setOpen(false); queueMicrotask(() => restore?.focus?.()); }
  onMount(() => {
    const onKeydown = (e: KeyboardEvent) => { if (open() && e.key === 'Escape' && !e.defaultPrevented) { e.preventDefault(); close(); } };
    document.addEventListener('keydown', onKeydown);
    onCleanup(() => document.removeEventListener('keydown', onKeydown));
  });
  const phase = createExitPhase(open, 150);
  return (
    <Show when={phase() !== 'closed'}>
      <div class="modal" classList={{ 'is-closing': phase() === 'closing' }} ref={() => { if (open()) restore = document.activeElement as HTMLElement | null; }}
        onClick={e => { if ((e.target as HTMLElement).classList.contains('modal-backdrop')) close(); }}>
        <div class="modal-backdrop" />
        <div class="modal-content task-editor" role="dialog" aria-modal="true" aria-labelledby="task-sheet-title">
          <div class="modal-head">
            <h2 id="task-sheet-title">{editing() ? 'Edit task' : 'New task'}</h2>
            <button class="close-btn" type="button" aria-label="Close task editor" title="Close (Escape)" onClick={close}>×</button>
          </div>
          <Show when={generation()} keyed>{_ =>
            <TaskEditor surface="sheet" task={editing()} prefill={prefill()} headingId="task-sheet-title" onClose={close} />
          }</Show>
        </div>
      </div>
    </Show>
  );
}
