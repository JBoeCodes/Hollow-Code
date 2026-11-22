import { describe, expect, it } from 'vitest';
import { defaultTicked, gardenActionLabel, gardenLines, gardenSelection } from './gardener';

const payload = {
  title: '3 things can be tidied',
  items: [
    { id: 'idle-workspace:old', kind: 'idle-workspace', target: 'old', title: 'Archive “Old”', reason: 'Idle 45 days.', action: 'archive', undo: 'Set it active.' },
    { id: 'test-task:tasks/notion-1.md', kind: 'test-task', target: 'tasks/notion-1.md', title: 'Delete in Notion', reason: 'Throwaway.', action: 'manual', undo: 'Notion trash.' },
    { id: 'test-task:tasks/t1.md', kind: 'test-task', target: 'tasks/t1.md', title: 'Trash t1', reason: 'Throwaway.', action: 'trash', undo: 'Restore it.' },
    { id: '', action: 'trash' },
    { id: 'x', action: 'delete' },
    'junk',
  ],
};

describe('gardener card', () => {
  it('reads well-formed lines and drops the rest', () => {
    expect(gardenLines(payload).map(l => l.id)).toEqual([
      'idle-workspace:old', 'test-task:tasks/notion-1.md', 'test-task:tasks/t1.md',
    ]);
    expect(gardenLines({})).toEqual([]);
  });

  it('ticks every applicable line by default and never a manual one', () => {
    expect(defaultTicked(gardenLines(payload))).toEqual(['idle-workspace:old', 'test-task:tasks/t1.md']);
  });

  it('sends only ticked, applicable ids', () => {
    const lines = gardenLines(payload);
    expect(gardenSelection(lines, ['test-task:tasks/t1.md', 'test-task:tasks/notion-1.md', 'made-up'])).toEqual({ ticked: ['test-task:tasks/t1.md'] });
    expect(gardenSelection(lines, [])).toEqual({ ticked: [] });
  });

  it('names actions in plain words', () => {
    expect(gardenActionLabel('remove-worktree')).toBe('Remove worktree');
    expect(gardenActionLabel('manual')).toBe('By hand in Notion');
    expect(gardenActionLabel('resolve')).toBe('Resolve in Needs you');
  });
});

describe('conflict copies (overnight A10)', () => {
  it('lists a resolve line but never ticks or sends it', () => {
    const lines = gardenLines({ items: [
      { id: 'conflict-copy:notes/Plan 2.md', kind: 'conflict-copy', target: 'notes/Plan 2.md', title: 'notes/Plan 2.md', reason: 'A conflict copy.', action: 'resolve', undo: 'Resolve it in Waiting on you.' },
      { id: 'idle-workspace:w1', kind: 'idle-workspace', target: 'w1', title: 'W1', reason: 'Idle.', action: 'archive', undo: 'Unarchive.' },
    ] });
    expect(lines.map(l => l.action)).toEqual(['resolve', 'archive']);
    expect(defaultTicked(lines)).toEqual(['idle-workspace:w1']);
    expect(gardenSelection(lines, lines.map(l => l.id))).toEqual({ ticked: ['idle-workspace:w1'] });
  });
});
