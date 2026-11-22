import { describe, expect, it } from 'vitest';
import { canonicalTaskStatus, priorityLabel, projectKey, taskFormProblem, taskProjectChoices, taskStatusOptions } from './taskEdit';

describe('shared task vocabulary (audit H02)', () => {
  it('names priorities the same way everywhere', () => {
    expect(priorityLabel('P1')).toBe('High (P1)');
    expect(priorityLabel('p4')).toBe('Lowest (P4)');
    expect(priorityLabel('')).toBe('None');
    expect(priorityLabel(null)).toBe('None');
    expect(priorityLabel('Urgent')).toBe('Urgent');
  });
  it('keeps a task’s own status selectable', () => {
    expect(taskStatusOptions('In progress')).toEqual(['Not started', 'In progress', 'Delivered']);
    expect(taskStatusOptions('Waiting on client')).toEqual(['Not started', 'In progress', 'Delivered', 'Waiting on client']);
  });
  it('groups status synonyms under one canonical status (the Board todo column)', () => {
    expect(canonicalTaskStatus('todo')).toBe('Not started');
    expect(canonicalTaskStatus('To Do')).toBe('Not started');
    expect(canonicalTaskStatus('')).toBe('Not started');
    expect(canonicalTaskStatus(null)).toBe('Not started');
    expect(canonicalTaskStatus('doing')).toBe('In progress');
    expect(canonicalTaskStatus('Done')).toBe('Delivered');
    expect(canonicalTaskStatus('Waiting on client')).toBe('Waiting on client');
  });
  it('dedupes projects by name, prefers linked ones and keeps the current project', () => {
    const c = taskProjectChoices([{ name: 'Beta' }, { name: 'alpha', url: 'vault:projects/alpha.md' }, { name: 'Alpha' }], { name: 'Gone project' });
    expect(c.map(p => p.name)).toEqual(['alpha', 'Beta', 'Gone project']);
    expect(c[0].url).toBe('vault:projects/alpha.md');
    expect(projectKey(c[1])).toBe('name:Beta');
  });
  it('validates identically on every surface', () => {
    expect(taskFormProblem({ title: '  ', dueTime: '' }, null)).toBe('Add a task name.');
    expect(taskFormProblem({ title: 'x', dueTime: 'soonish' }, null)).toMatch(/Couldn’t read the time/);
    expect(taskFormProblem({ title: 'x', dueTime: '4pm' }, '16:00')).toBeNull();
    expect(taskFormProblem({ title: 'x', dueTime: '' }, null)).toBeNull();
  });
});
