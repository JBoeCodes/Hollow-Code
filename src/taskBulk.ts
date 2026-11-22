/**
 * Task helpers shared by the local task detail pane: which workspace a task
 * belongs to (from its plan origin) and the first message of "Work on this".
 * Pure, unit-tested. (The bulk-edit planner that lived here went with the
 * Tasks page's multi-select in the issue-list rewrite.)
 */
import type { Task } from './types';

/** The workspace a task explicitly belongs to, from its plan origin (never by name). */
export function originWorkspace(t: Pick<Task, 'origin'>): string | null {
  const m = /^plan:workspaces\/([^/]+)\/PLAN\.md$/.exec(t.origin ?? '') ?? /^workspace:([^/\s]+)$/.exec(t.origin ?? '');
  return m ? m[1] : null;
}

/** The first message of "Work on this": the task, its file and done-when. */
export function workOnPrompt(t: Pick<Task, 'title' | 'vaultPath' | 'doneWhen' | 'due' | 'description'>): string {
  return [
    `Work on the task “${t.title}”${t.vaultPath ? ` (${t.vaultPath})` : ''}.`,
    t.due ? `It is due ${t.due.slice(0, 10)}.` : '',
    t.doneWhen ? `Done when: ${t.doneWhen.trim()}` : '',
    t.description?.trim() ? `Notes: ${t.description.trim().slice(0, 600)}` : '',
    'Use this workspace’s brief and sources. Say what you changed and what is left.',
  ].filter(Boolean).join('\n');
}
