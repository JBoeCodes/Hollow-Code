import { describe, expect, it } from 'vitest';
import { applyTaskEditChanges, assistProjectUrl, linkedLockNotice, lockedTaskFields, taskEditChanges, taskEditValues } from './taskEdit';
import type { TrackerLockKind } from './generated/ipc';

/** A tracker-linked task's meta with this lock. */
const linkedTo = (kind: TrackerLockKind, fields: string[]) => ({ provider: 'linear', id: 'i1', key: 'ENG-1', url: null, statusType: null, priority: 0, lock: { kind, fields } });
import type { Task } from './types';

function task(over: Partial<Task> = {}): Task {
  return {
    url: 'vault:tasks/sample.md',
    title: 'Submit the report',
    status: 'Not started',
    priority: 'P2',
    due: '2026-06-12',
    dueTime: '14:00',
    durationMin: 45,
    effort: null,
    projectName: 'Work',
    projectUrl: 'vault:projects/active/Work.md',
    section: 'Q3',
    labels: ['ui'],
    description: 'Quarterly numbers',
    daysUntilDue: 0,
    bucket: 'today',
    vaultPath: 'tasks/sample.md',
    recurrence: null,
    subtasks: [{ text: 'Draft', done: false }],
    contentHash: 'hash-1',
    fingerprint: null,
    origin: null,
    doneWhen: null,
    activity: null,
    blockedBy: [],
    blocks: [],
    ready: true,
    unresolvedBlockers: [],
    blockerWarnings: [],
    ...over,
  };
}

describe('taskEditChanges sends only changed fields', () => {
  const open = { locked: [] };

  it('returns nothing for an untouched form', () => {
    const initial = taskEditValues(task());
    expect(taskEditChanges(initial, taskEditValues(task()), open)).toEqual({});
  });

  it('keeps an untouched ISO due out of the payload and carries its time when the date moves', () => {
    const iso = task({ due: '2026-07-23T17:00:00.000-04:00', dueTime: null });
    const initial = taskEditValues(iso);
    expect(initial.due).toBe('2026-07-23');
    expect(initial.dueTime).toBe('17:00');
    expect(taskEditChanges(initial, { ...initial }, open)).toEqual({});
    expect(taskEditChanges(initial, { ...initial, due: '2026-07-24' }, open))
      .toEqual({ due: '2026-07-24', dueTime: '17:00' });
  });

  it('never sends issue-owned fields for a linked task', () => {
    const linked = task({ tracker: linkedTo('linked', ['title', 'status', 'priority', 'due', 'description', 'project']) });
    const initial = taskEditValues(linked);
    const current = {
      ...initial,
      title: 'Renamed',
      project: 'Elsewhere',
      description: 'Rewritten summary',
      due: '2026-06-20',
      labels: ['ui', 'customer'],
    };
    expect(taskEditChanges(initial, current, { locked: lockedTaskFields(linked) })).toEqual({ labels: ['ui', 'customer'] });
  });

  it('sends a cleared duration as 0', () => {
    const initial = taskEditValues(task());
    expect(taskEditChanges(initial, { ...initial, durationMin: '' }, open))
      .toEqual({ durationMin: 0 });
  });

  it('compares labels the way the backend stores them', () => {
    const initial = taskEditValues(task({ labels: ['ui'] }));
    const current = { ...initial, labels: '@ui, ui'.split(',') };
    expect(taskEditChanges(initial, current, open)).toEqual({});
  });

  it('sends changed sub-tasks and a changed time alone', () => {
    const initial = taskEditValues(task());
    const subtasks = [{ text: 'Draft', done: true }];
    expect(taskEditChanges(initial, { ...initial, subtasks, dueTime: '09:30' }, open))
      .toEqual({ subtasks, dueTime: '09:30' });
  });
});

describe('lockedTaskFields', () => {
  const all = ['title', 'status', 'priority', 'due', 'description', 'project'];

  it('locks nothing for a local task', () => {
    expect(lockedTaskFields(task())).toEqual([]);
    expect(lockedTaskFields(null)).toEqual([]);
  });

  it('adds the time when the issue owns a due date that carries one', () => {
    const iso = task({ due: '2026-07-23T17:00:00.000-04:00', dueTime: null, tracker: linkedTo('linked', all) });
    expect(lockedTaskFields(iso)).toEqual(['title', 'status', 'priority', 'due', 'dueTime', 'project', 'description']);
    const initial = taskEditValues(iso);
    expect(taskEditChanges(initial, { ...initial, dueTime: '09:00' }, { locked: lockedTaskFields(iso) })).toEqual({});
    // A date-only issue due leaves the Hollow-only time editable.
    const dated = task({ due: '2026-07-23', dueTime: null, tracker: linkedTo('linked', all) });
    expect(lockedTaskFields(dated)).not.toContain('dueTime');
  });

  it('holds only the project when the lock says so', () => {
    for (const kind of ['remoteMissing', 'moved', 'staleRelations'] as const) {
      const held = task({ tracker: linkedTo(kind, ['project']) });
      expect(lockedTaskFields(held)).toEqual(['project']);
      const initial = taskEditValues(held);
      expect(taskEditChanges(initial, { ...initial, title: 'Retitled', project: 'Other' }, { locked: lockedTaskFields(held) }))
        .toEqual({ title: 'Retitled' });
    }
  });
});

describe('linkedLockNotice', () => {
  it('lists exactly the locked fields', () => {
    expect(linkedLockNotice(['title', 'status']))
      .toBe('Linked to an issue. Edit the title and status on the issue in Tasks; everything else saves here.');
    expect(linkedLockNotice(['title', 'due', 'dueTime', 'description']))
      .toContain('title, due date, time and summary on the issue');
  });
});

describe('applyTaskEditChanges', () => {
  it('patches only the changed keys and records the new hash', () => {
    const before = task();
    const after = applyTaskEditChanges(before, { priority: 'P1', project: 'Other' }, 'hash-2');
    expect(after).toEqual({ ...before, priority: 'P1', projectName: 'Other', projectUrl: null, contentHash: 'hash-2' });
  });
});

describe('assistProjectUrl', () => {
  it("sends only a linked task's stored project url", () => {
    const linked = task();
    // Unchanged field: the stored link.
    expect(assistProjectUrl(linked, 'Work', 'Work')).toBe('vault:projects/active/Work.md');
    expect(assistProjectUrl(linked, 'Work', '  Work ')).toBe('vault:projects/active/Work.md');
    // Cleared field.
    expect(assistProjectUrl(linked, 'Work', '')).toBeUndefined();
    expect(assistProjectUrl(linked, 'Work', '   ')).toBeUndefined();
    // Changed field, even to another existing project's name.
    expect(assistProjectUrl(linked, 'Work', 'Hollow 3.0')).toBeUndefined();
    // No stored link: never matched by name.
    expect(assistProjectUrl(task({ projectUrl: null }), 'Work', 'Work')).toBeUndefined();
    // A new task.
    expect(assistProjectUrl(null, '', 'Work')).toBeUndefined();
  });
});

describe('blockedBy (Paperclip PR 11b)', () => {
  const open = { locked: [] };
  const blocked = task({ blockedBy: ['vault:tasks/a.md', 'vault:tasks/b.md'] });

  it('is sent only when the list changes', () => {
    const initial = taskEditValues(blocked);
    expect(initial.blockedBy).toEqual(['vault:tasks/a.md', 'vault:tasks/b.md']);
    expect(taskEditChanges(initial, { ...initial, blockedBy: [...initial.blockedBy] }, open)).toEqual({});
    // Case and surrounding space are not a change.
    expect(taskEditChanges(initial, { ...initial, blockedBy: [' vault:tasks/A.md', 'vault:tasks/b.md'] }, open)).toEqual({});
    expect(taskEditChanges(initial, { ...initial, blockedBy: ['vault:tasks/a.md'] }, open))
      .toEqual({ blockedBy: ['vault:tasks/a.md'] });
    expect(taskEditChanges(initial, { ...initial, blockedBy: [] }, open)).toEqual({ blockedBy: [] });
    expect(taskEditChanges(initial, { ...initial, blockedBy: ['vault:tasks/b.md', 'vault:tasks/a.md'] }, open))
      .toEqual({ blockedBy: ['vault:tasks/b.md', 'vault:tasks/a.md'] });
  });

  it('is never held by a tracker lock', () => {
    const linked = task({ blockedBy: [], tracker: linkedTo('linked', ['title', 'status', 'priority', 'due', 'description', 'project']) });
    const initial = taskEditValues(linked);
    expect(taskEditChanges(initial, { ...initial, blockedBy: ['vault:tasks/a.md'] }, { locked: lockedTaskFields(linked) }))
      .toEqual({ blockedBy: ['vault:tasks/a.md'] });
  });

  it('patches the list optimistically', () => {
    expect(applyTaskEditChanges(blocked, { blockedBy: [] }, 'hash-3').blockedBy).toEqual([]);
  });
});
