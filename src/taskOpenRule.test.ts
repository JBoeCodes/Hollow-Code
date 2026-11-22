import { describe, expect, it } from 'vitest';
import { isOpenTask } from './taskStatus';
import { layoutDay } from './agenda';
import { dueBuckets } from './todayBrief';
import type { Task } from './types';

// P1-12: one rule for finished tasks. Call sites (Agenda,
// Today's facts) each kept their own list and disagreed on
// Deployed, Closed and Cancelled; every one now asks `isOpenTask`.
const STATUSES: [string, boolean][] = [
  ['Not started', true], ['In progress', true], ['Blocked', true], ['todo', true], ['', true],
  ['Done', false], [' done ', false], ['Delivered', false], ['Deployed', false], ['Complete', false],
  ['Completed', false], ['Closed', false], ['Cancelled', false], ['Canceled', false], ['Won’t do', false],
];
const DAY = '2026-10-24';
const task = (status: string) => ({
  url: `u-${status}`, title: `T ${status}`, status, due: DAY, dueTime: null, durationMin: null, bucket: 'today', priority: null,
}) as unknown as Task;

describe('one open or closed answer per status at every call site (P1-12)', () => {
  it.each(STATUSES)('%j: open = %s', (status, open) => {
    expect(isOpenTask(status), 'the rule').toBe(open);
    const t = task(status);
    expect(layoutDay(DAY, [], [], [t]).dueNoTime.length === 1, 'Agenda').toBe(open);
    expect(dueBuckets([t], DAY).today.length === 1, 'Today’s facts').toBe(open);
  });
});
