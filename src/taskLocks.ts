// Which task fields a linked tracker issue owns (overnight P3: split from
// taskEdit.ts so the launch path carries only this).
import type { Task } from './types';

/// The `HH:MM` a full ISO `due` carries, or '' when it has no time part.
export function dueTimePart(raw: string): string {
  return /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}/.test(raw) ? raw.slice(11, 16) : '';
}

export type LockableField = 'title' | 'status' | 'priority' | 'due' | 'dueTime' | 'project' | 'description';

const LOCKABLE: readonly LockableField[] = ['title', 'status', 'priority', 'due', 'dueTime', 'project', 'description'];

/// The TaskModal fields to lock for `t`: its tracker lock's fields, plus the
/// Time field when the issue-owned due date itself carries a time (a local
/// `due_time` would otherwise override the issue's time and never sync).
export function lockedTaskFields(t: Task | null): LockableField[] {
  const held = new Set(t?.tracker?.lock.fields ?? []);
  if (held.has('due') && dueTimePart((t?.due ?? '').trim())) held.add('dueTime');
  return LOCKABLE.filter(f => held.has(f));
}
