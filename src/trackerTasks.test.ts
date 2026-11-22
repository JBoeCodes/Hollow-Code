import { describe, expect, it } from 'vitest';
import type { TrackerIssue, TrackerIssueDetail, TrackerState, TrackerViewRow } from './generated/ipc';
import { agentRunsOf, dueText, fieldEdit, fieldLock, groupByStatus, initials, matchesSearch, priorityLabel, step } from './trackerTasks';

const STATES: TrackerState[] = [
  { id: 'b', name: 'Backlog', kind: 'backlog', position: 0, color: null },
  { id: 't', name: 'Todo', kind: 'unstarted', position: 1, color: null },
  { id: 'p', name: 'In Progress', kind: 'started', position: 2, color: null },
  { id: 'r', name: 'In Review', kind: 'started', position: 3, color: null },
  { id: 'd', name: 'Done', kind: 'completed', position: 4, color: null },
];
const row = (key: string, statusName: string, over: Partial<TrackerViewRow> = {}): TrackerViewRow => ({
  id: key.toLowerCase(), key, title: `Title ${key}`, statusName, statusKind: STATES.find(s => s.name === statusName)?.kind ?? 'unstarted',
  priority: 3, labels: [], project: null, projectId: null, due: null, assignee: null, assigneeId: null, estimate: null, cycle: null,
  milestone: null, updatedAt: '2026-11-10T00:00:00Z', url: '', path: null, lock: null, pending: false, conflict: false, archived: false, teamKey: 'ENG', ...over,
});

describe('tasks laid out like a tracker', () => {
  it('groups by status: in progress first, then to do, backlog, done; empty states left out', () => {
    const groups = groupByStatus([row('ENG-1', 'Todo'), row('ENG-2', 'In Review'), row('ENG-3', 'In Progress'), row('ENG-4', 'Done'), row('ENG-5', 'Backlog')], STATES);
    expect(groups.map(g => g.name)).toEqual(['In Progress', 'In Review', 'Todo', 'Backlog', 'Done']);
    expect(groupByStatus([row('ENG-1', 'Todo')], STATES).map(g => g.name)).toEqual(['Todo']);
  });

  it('keeps a row whose state the team no longer lists', () => {
    const groups = groupByStatus([row('ENG-9', 'Triage', { statusKind: 'unstarted' })], STATES);
    expect(groups).toHaveLength(1);
    expect(groups[0]).toMatchObject({ name: 'Triage', kind: 'unstarted' });
  });

  it('orders a group by priority (urgent first, none last), then the latest update', () => {
    const g = groupByStatus([
      row('ENG-1', 'Todo', { priority: 0 }), row('ENG-2', 'Todo', { priority: 4 }), row('ENG-3', 'Todo', { priority: 1 }),
      row('ENG-4', 'Todo', { priority: 4, updatedAt: '2026-11-11T00:00:00Z' }),
    ], STATES)[0];
    expect(g.rows.map(r => r.key)).toEqual(['ENG-3', 'ENG-4', 'ENG-2', 'ENG-1']);
  });

  it('searches the key, title, labels, project, assignee and status', () => {
    const r = row('ENG-31', 'Todo', { title: 'Match bank deposits', labels: ['Accounting'], project: 'Receipts', assignee: 'Alex Rivera' });
    expect(matchesSearch(r, 'eng-31')).toBe(true);
    expect(matchesSearch(r, 'bank receipts')).toBe(true);
    expect(matchesSearch(r, 'alex accounting')).toBe(true);
    expect(matchesSearch(r, 'warehouse')).toBe(false);
    expect(matchesSearch(row('ENG-2', 'Todo'), 'unassigned')).toBe(true);
    expect(matchesSearch(r, '  ')).toBe(true);
  });

  it('words: priorities, initials and due dates', () => {
    expect(priorityLabel(1)).toBe('Urgent');
    expect(priorityLabel(0)).toBe('No priority');
    expect(priorityLabel(9)).toBe('No priority');
    expect(initials('Alex Rivera')).toBe('AR');
    expect(initials('Sam')).toBe('S');
    expect(initials(null)).toBe('?');
    const now = new Date(2026, 10, 10, 12);
    expect(dueText('2026-11-10', now)).toEqual({ text: 'Today', overdue: false });
    expect(dueText('2026-11-11', now)).toEqual({ text: 'Tomorrow', overdue: false });
    expect(dueText('2026-11-09', now)).toEqual({ text: 'Yesterday', overdue: true });
    expect(dueText('2026-11-20', now)?.text).toBe('Nov 20');
    expect(dueText(null, now)).toBeNull();
  });

  it('edits one field against the value the detail read', () => {
    const issue = { id: 'i-1', title: 'Old', state: STATES[1], assignee: null, priority: 2, labels: ['A'], projectId: 'p-1', due: null, cycle: null, milestone: null, estimate: 3, parentId: null, description: '' } as unknown as TrackerIssue;
    expect(fieldEdit(issue, 'status', 'p')).toEqual({ id: 'i-1', values: { status: 'p' }, expected: { status: 't' } });
    expect(fieldEdit(issue, 'assignee', 'u-1')).toEqual({ id: 'i-1', values: { assignee: 'u-1' }, expected: { assignee: '' } });
    expect(fieldEdit(issue, 'labels', ['A', 'B']).expected).toEqual({ labels: ['A'] });
    expect(fieldEdit(issue, 'estimate', null).expected).toEqual({ estimate: 3 });
  });

  it('says why a field is locked', () => {
    const detail = { issue: { archivedAt: null, trashed: false }, lock: { kind: 'linked', fields: [] }, writableFields: ['title', 'status'], pending: ['status'] } as unknown as TrackerIssueDetail;
    expect(fieldLock(detail, 'title')).toBeNull();
    expect(fieldLock(detail, 'status')).toMatch(/still waiting to send/);
    expect(fieldLock(detail, 'due')).toMatch(/owns this field/);
    expect(fieldLock({ ...detail, lock: { kind: 'outOfScope', fields: [] } }, 'title')).toMatch(/another team/);
    expect(fieldLock({ ...detail, issue: { ...detail.issue, archivedAt: '2026-11-01T00:00:00Z' } } as TrackerIssueDetail, 'title')).toMatch(/Archived/);
  });

  it('moves with the keys, clamped', () => {
    expect(step(-1, 1, 3)).toBe(0);
    expect(step(-1, -1, 3)).toBe(2);
    expect(step(2, 1, 3)).toBe(2);
    expect(step(0, -1, 3)).toBe(0);
    expect(step(0, 1, 0)).toBe(-1);
  });

  it('reads agent runs once the backend sends them, newest first', () => {
    const detail = { agentRuns: [{ who: 'COO', model: null, startedAt: '2026-11-10T09:00:00Z', endedAt: null, outcome: 'done', sessionKey: null, summary: null },
      { who: 'Lead', model: 'Claude', startedAt: '2026-11-10T10:00:00Z', endedAt: null, outcome: 'running', sessionKey: 's-1', summary: null }] } as unknown as TrackerIssueDetail;
    expect(agentRunsOf(detail).map(r => r.who)).toEqual(['Lead', 'COO']);
    expect(agentRunsOf({} as TrackerIssueDetail)).toEqual([]);
  });
});
