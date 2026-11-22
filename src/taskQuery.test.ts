import { describe, expect, it } from 'vitest';
import { TASK_QUERY_HELP, showsReady, compareTasks, effectivePriority, matchesTaskQuery, rankTasks, toTaskQuery } from './taskQuery';
import querySource from './taskQuery.ts?raw';
import type { Task } from './types';

function task(over: Partial<Task> = {}): Task {
  return {
    url: 'vault:tasks/sample.md',
    title: 'Submit the report',
    status: 'Not started',
    priority: 'P2',
    due: '2026-06-12',
    dueTime: null,
    durationMin: null,
    effort: null,
    projectName: 'Work',
    projectUrl: null,
    section: null,
    labels: ['urgent'],
    description: 'Quarterly numbers',
    daysUntilDue: 0,
    bucket: 'today',
    vaultPath: 'tasks/sample.md',
    recurrence: null,
    subtasks: [],
    contentHash: '',
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

describe('matchesTaskQuery', () => {
  it('matches everything on an empty query', () => {
    expect(matchesTaskQuery(task(), '')).toBe(true);
    expect(matchesTaskQuery(task(), '   ')).toBe(true);
  });

  it('matches bucket clauses', () => {
    expect(matchesTaskQuery(task({ bucket: 'today' }), 'today')).toBe(true);
    expect(matchesTaskQuery(task({ bucket: 'overdue' }), 'overdue')).toBe(true);
    expect(matchesTaskQuery(task({ bucket: 'thisweek' }), 'upcoming')).toBe(true);
    expect(matchesTaskQuery(task({ bucket: 'undated' }), 'nodate')).toBe(true);
    expect(matchesTaskQuery(task({ bucket: 'later' }), 'today')).toBe(false);
  });

  it('matches priority case-insensitively', () => {
    expect(matchesTaskQuery(task({ priority: 'P1' }), 'p1')).toBe(true);
    expect(matchesTaskQuery(task({ priority: 'P2' }), 'p1')).toBe(false);
  });

  it('matches done/active by completion vocabulary', () => {
    expect(matchesTaskQuery(task({ status: 'Delivered' }), 'done')).toBe(true);
    expect(matchesTaskQuery(task({ status: 'Completed' }), 'delivered')).toBe(true);
    expect(matchesTaskQuery(task({ status: 'In progress' }), 'active')).toBe(true);
    expect(matchesTaskQuery(task({ status: 'Done' }), 'active')).toBe(false);
  });

  it('ready matches t.ready, not the word in a title (P1-16)', () => {
    expect(matchesTaskQuery(task({ ready: true }), 'ready')).toBe(true);
    expect(matchesTaskQuery(task({ ready: false, title: 'Get ready for the audit' }), 'ready')).toBe(false);
    expect(matchesTaskQuery(task({ ready: true, priority: 'P1' }), 'ready & p1')).toBe(true);
  });

  it('the filter help names every token matchTaskClause reads (P1-16)', () => {
    const body = querySource.slice(querySource.indexOf('function matchTaskClause'), querySource.indexOf('export function matchesTaskQuery'));
    const words = [...body.matchAll(/c === '([^']+)'/g)].map(m => m[1]);
    expect(words).toEqual(expect.arrayContaining(['today', 'overdue', 'upcoming', 'nodate', 'done', 'active', 'ready']));
    for (const w of words) expect(TASK_QUERY_HELP, w).toContain(w);
    if (body.includes('p[1-4]')) expect(TASK_QUERY_HELP).toContain('p1');
    if (body.includes("startsWith('@')")) expect(TASK_QUERY_HELP).toContain('@label');
    if (body.includes("startsWith('#')")) expect(TASK_QUERY_HELP).toContain('#project');
  });

  it('the Ready chip shows only once a task’s blockers are done (P1-16)', () => {
    expect(showsReady(task({ ready: true, blockedBy: ['vault:tasks/a.md'] }))).toBe(true);
    expect(showsReady(task({ ready: true, blockedBy: [] }))).toBe(false);
    expect(showsReady(task({ ready: false, blockedBy: ['vault:tasks/a.md'] }))).toBe(false);
  });

  it('matches @label and #project clauses', () => {
    expect(matchesTaskQuery(task({ labels: ['urgent'] }), '@urgent')).toBe(true);
    expect(matchesTaskQuery(task({ labels: ['Urgent'] }), '@urgent')).toBe(true);
    expect(matchesTaskQuery(task({ labels: [] }), '@urgent')).toBe(false);
    expect(matchesTaskQuery(task({ projectName: 'Engineering' }), '#eng')).toBe(true);
    expect(matchesTaskQuery(task({ projectName: null }), '#eng')).toBe(false);
  });

  it('falls back to title/description substring', () => {
    expect(matchesTaskQuery(task(), 'report')).toBe(true);
    expect(matchesTaskQuery(task(), 'quarterly')).toBe(true);
    expect(matchesTaskQuery(task(), 'zebra')).toBe(false);
  });

  it('AND-joins clauses with &', () => {
    expect(matchesTaskQuery(task(), 'today & p2 & @urgent')).toBe(true);
    expect(matchesTaskQuery(task(), 'today & p1')).toBe(false);
  });
});

describe('effectivePriority', () => {
  it('boosts work 2+ days past due by one tier', () => {
    expect(effectivePriority(task({ priority: 'P2', bucket: 'overdue', daysUntilDue: -2 }))).toBe(0);
    expect(effectivePriority(task({ priority: 'P2', bucket: 'overdue', daysUntilDue: -1 }))).toBe(1);
    expect(effectivePriority(task({ priority: 'P1', bucket: 'overdue', daysUntilDue: -30 }))).toBe(0);
  });

  it('treats an unset priority as the lowest tier', () => {
    expect(effectivePriority(task({ priority: null, daysUntilDue: 3 }))).toBe(4);
    expect(effectivePriority(task({ priority: null, bucket: 'overdue', daysUntilDue: -9 }))).toBe(3);
  });

  it('never boosts completed work', () => {
    expect(effectivePriority(task({ priority: 'P3', bucket: 'done', daysUntilDue: -34 }))).toBe(2);
  });
});

describe('rankTasks', () => {
  it('leads with overdue, then effective priority, then due date', () => {
    const soonP1 = task({ url: 'a', priority: 'P1', bucket: 'thisweek', due: '2026-06-14', daysUntilDue: 2 });
    const lateP3 = task({ url: 'b', priority: 'P3', bucket: 'overdue', due: '2026-05-01', daysUntilDue: -42 });
    const lateP1 = task({ url: 'c', priority: 'P1', bucket: 'overdue', due: '2026-06-01', daysUntilDue: -11 });
    expect(rankTasks([soonP1, lateP3, lateP1]).map(t => t.url)).toEqual(['c', 'b', 'a']);
  });

  it('sinks completed work below everything open', () => {
    const done = task({ url: 'done', priority: 'P1', bucket: 'done', due: '2026-05-01', daysUntilDue: -42 });
    const open = task({ url: 'open', priority: 'P4', bucket: 'later', due: '2026-12-01', daysUntilDue: 180 });
    expect(rankTasks([done, open]).map(t => t.url)).toEqual(['open', 'done']);
  });

  it('lifts avoided work above a same-bucket task one tier higher', () => {
    const fresh = task({ url: 'fresh', priority: 'P1', bucket: 'overdue', due: '2026-06-11', daysUntilDue: -1 });
    const avoided = task({ url: 'avoided', priority: 'P2', bucket: 'overdue', due: '2026-05-20', daysUntilDue: -23 });
    expect(rankTasks([fresh, avoided]).map(t => t.url)).toEqual(['avoided', 'fresh']);
  });

  it('does not mutate the input', () => {
    const pool = [
      task({ url: 'a', priority: 'P4', bucket: 'later', daysUntilDue: 20 }),
      task({ url: 'b', priority: 'P1', bucket: 'overdue', daysUntilDue: -5 }),
    ];
    rankTasks(pool);
    expect(pool.map(t => t.url)).toEqual(['a', 'b']);
  });

  it('exposes the same order through compareTasks', () => {
    const a = task({ url: 'a', priority: 'P1', bucket: 'overdue', daysUntilDue: -5 });
    const b = task({ url: 'b', priority: 'P4', bucket: 'later', daysUntilDue: 20 });
    expect(compareTasks(a, b)).toBeLessThan(0);
    expect(compareTasks(b, a)).toBeGreaterThan(0);
    expect(compareTasks(a, a)).toBe(0);
  });
});

describe('toTaskQuery', () => {
  it('passes explicit & queries through', () => {
    expect(toTaskQuery('overdue & p1')).toBe('overdue & p1');
  });

  it('converts spoken queries, dropping filler', () => {
    expect(toTaskQuery('show me overdue p1 tasks')).toBe('overdue & p1');
    expect(toTaskQuery('today @urgent')).toBe('today & @urgent');
    expect(toTaskQuery('overdue budget')).toBe('overdue & budget');
  });

  it('returns null for plain fuzzy text', () => {
    expect(toTaskQuery('quarterly report')).toBeNull();
    expect(toTaskQuery('settings')).toBeNull();
    expect(toTaskQuery('')).toBeNull();
  });

  it('treats labels/projects/priorities as structural', () => {
    expect(toTaskQuery('@errand')).toBe('@errand');
    expect(toTaskQuery('#hollow active')).toBe('#hollow & active');
    expect(toTaskQuery('p2')).toBe('p2');
  });
});
