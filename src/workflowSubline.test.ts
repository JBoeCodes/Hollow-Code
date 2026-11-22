import { describe, expect, it } from 'vitest';
import type { Workflow } from './types';
import { SCHEDULE_PROBLEM_ROW, outputGlobForScope, promptPlaceholder, workflowSubline } from './pages/workflows/workflowText';

// P1-04 (AUTO-22): choosing another workspace moves the default results
// folder along but keeps one the person typed; the prompt's placeholder names
// the workspace the automation runs in, not the vault.
describe('the editor follows its workspace', () => {
  it('moves the default results glob and keeps a custom one', () => {
    const paths = ['workspaces/a', 'workspaces/b'];
    expect(outputGlobForScope('workspaces/a/outputs/*', paths, 'workspaces/b')).toBe('workspaces/b/outputs/*');
    expect(outputGlobForScope('', paths, 'workspaces/b')).toBe('workspaces/b/outputs/*');
    expect(outputGlobForScope('  ', [], 'workspaces/b')).toBe('workspaces/b/outputs/*');
    expect(outputGlobForScope('briefings/*.md', paths, 'workspaces/b')).toBe('briefings/*.md');
    expect(outputGlobForScope('workspaces/a/outputs/report-*.md', paths, 'workspaces/b')).toBe('workspaces/a/outputs/report-*.md');
    // The working folder was edited by hand in between: A's default still follows.
    expect(outputGlobForScope('workspaces/a/outputs/* ', paths, 'workspaces/b')).toBe('workspaces/b/outputs/*');
  });
  it('names the workspace in the prompt placeholder', () => {
    expect(promptPlaceholder('Weekly ops')).toContain('It runs in Weekly ops');
    expect(promptPlaceholder('Weekly ops')).not.toContain('The vault is the working directory');
    expect(promptPlaceholder(null)).toContain('It runs in the Hollow folder');
    expect(promptPlaceholder(undefined, '/Users/me/code')).toContain('It runs in its working folder');
  });
});

// B04: what an automation's row says under its title (`workflowSubline`,
// moved out of Workflows.tsx's WorkflowRow memo). A schedule the backend
// can't read (`scheduleProblem`) never runs, so the row says so instead of
// "—"; every other case reads as it did. The row and detail pane end to end:
// tests/smoke/automation-bad-schedule.spec.ts.

const NOW = Date.parse('2026-10-16T12:00:00Z');
const HOUR = 3_600_000;
const iso = (offsetMs: number) => new Date(NOW + offsetMs).toISOString();

type Row = Pick<Workflow, 'schedule' | 'nextRunAt' | 'lastRunAt' | 'misfireGraceMin' | 'scheduleProblem'>;
const row = (over: Partial<Row> = {}): Row => ({
  schedule: '0 0 7 * * * *', nextRunAt: null, lastRunAt: null, misfireGraceMin: null, ...over,
});
const broken = { schedule: 'not-cron', scheduleProblem: 'Invalid expression: Invalid cron expression.' };

describe('workflowSubline (B04)', () => {
  it('says an unreadable schedule won’t run, as a warning, instead of "—"', () => {
    expect(workflowSubline(row(broken), null, null, NOW)).toEqual({ text: SCHEDULE_PROBLEM_ROW, warn: true });
    expect(SCHEDULE_PROBLEM_ROW).toBe('Schedule can’t be read, so this won’t run on schedule');
    // A stale next-run time or a last run doesn't hide it.
    expect(workflowSubline(row({ ...broken, nextRunAt: iso(2 * HOUR), lastRunAt: iso(-HOUR) }), null, null, NOW))
      .toEqual({ text: SCHEDULE_PROBLEM_ROW, warn: true });
    // Without the problem the same row read "—", the old silent state.
    expect(workflowSubline(row({ schedule: 'not-cron' }), null, null, NOW)).toEqual({ text: '—', warn: false });
  });

  it('a live queue row still wins, unless a hold refuses the automation', () => {
    expect(workflowSubline(row(broken), null, 'Running', NOW)).toEqual({ text: 'Running', warn: false });
    expect(workflowSubline(row(broken), 'Paused for review', 'Running', NOW)).toEqual({ text: SCHEDULE_PROBLEM_ROW, warn: true });
    expect(workflowSubline(row({ schedule: 'manual' }), null, 'Queued', NOW)).toEqual({ text: 'Queued', warn: false });
  });

  it('manual automations read as before', () => {
    expect(workflowSubline(row({ schedule: 'manual' }), null, null, NOW)).toEqual({ text: 'never run', warn: false });
    expect(workflowSubline(row({ schedule: 'manual', lastRunAt: iso(-3 * HOUR) }), null, null, NOW))
      .toEqual({ text: 'last run 3h ago', warn: false });
  });

  it('scheduled automations name the absolute next time and preserve missed, held and last states', () => {
    expect(workflowSubline(row({ nextRunAt: iso(2 * HOUR) }), null, null, NOW)).toEqual({ text: 'next Fri, 10:00 AM EDT · in 2h', warn: false });
    expect(workflowSubline(row({ nextRunAt: iso(-5 * HOUR) }), null, null, NOW))
      .toEqual({ text: 'missed. Run now to catch up.', warn: true });
    expect(workflowSubline(row({ nextRunAt: iso(2 * HOUR) }), 'Paused for review', 'Running', NOW))
      .toEqual({ text: 'Paused for review', warn: true });
    expect(workflowSubline(row({ schedule: 'window:nightly', nextRunAt: iso(2 * HOUR) }), null, null, NOW))
      .toEqual({ text: 'next Fri, 10:00 AM EDT · in 2h', warn: false });
    expect(workflowSubline(row({ lastRunAt: iso(-2 * 24 * HOUR) }), null, null, NOW)).toEqual({ text: 'last run 2d ago', warn: false });
    expect(workflowSubline(row(), null, null, NOW)).toEqual({ text: '—', warn: false });
  });
});
