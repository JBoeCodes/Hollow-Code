// Evening in a zone west of Greenwich, as on the CI smoke runner: UTC has
// already rolled over to tomorrow while the local calendar is still today.
(globalThis as unknown as { process: { env: Record<string, string> } }).process.env.TZ = 'America/New_York';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { dateKey } from './agenda';
import { demoTasks } from './mockDemo';

afterEach(() => { vi.useRealTimers(); });

describe('demo task fixtures', () => {
  it('date due-today tasks by the local calendar the Agenda uses, not UTC', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-10-03T00:10:00Z')); // 20:10 on 2 Oct in New York
    const today = dateKey(new Date());
    expect(today).toBe('2026-10-02');
    const due = (title: string) => demoTasks(8).find(t => t.title === title)?.due;
    expect(due('Weekly safety walkthrough notes')).toBe(today);
    expect(due('Confirm float supervisor budget')).toBe('2026-10-03');
    expect(due('Collect vendor security questionnaires')).toBe('2026-09-30');
  });
});
