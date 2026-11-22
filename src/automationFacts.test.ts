import { afterAll, expect, it } from 'vitest';
import { nextRunLabel, workflowState } from './pages/workflows/workflowText';
import type { Workflow } from './generated/ipc';
import { BACKGROUND_SYSTEMS } from './backgroundWork';

const env = (globalThis as unknown as { process: { env: Record<string, string | undefined> } }).process.env;
const previousZone = env.TZ;
env.TZ = 'America/New_York';
afterAll(() => { if (previousZone === undefined) delete env.TZ; else env.TZ = previousZone; });

it('names the next run day, time and zone alongside relative time', () => {
  const text = nextRunLabel('2026-10-24T06:00:00Z', null, null, null, Date.parse('2026-10-23T21:00:00Z')).text;
  expect(text).toMatch(/Sat/);
  expect(text).toMatch(/2:00/);
  expect(text).toMatch(/EDT|GMT-4/);
  expect(text).toMatch(/· in \d/);
});

it('explains host window jobs as their own background mechanism', () => {
  expect(BACKGROUND_SYSTEMS.map(s => s.name)).toContain('Window jobs');
});

it('never calls an unfamiliar recorded run state not run yet', () => {
  expect(workflowState({ lastStatus: 'interrupted' } as Workflow, false)).toBe('Last run: interrupted');
});
