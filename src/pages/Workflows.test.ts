import { describe, expect, it } from 'vitest';
import workflowsSource from './Workflows.tsx?raw';

describe('Workflows active workflow refresh comparison', () => {
  it('refreshes active() when hold, error, runner, end-note, or title fields change', () => {
    const start = workflowsSource.indexOf('Restore the previously-active workflow when the list refreshes.');
    const end = workflowsSource.indexOf('setActive(match);', start);
    expect(start).toBeGreaterThan(-1);
    expect(end).toBeGreaterThan(start);
    const block = workflowsSource.slice(start, end);
    for (const field of [
      'slug',
      'lastRunAt',
      'lastStatus',
      'lastError',
      'schedule',
      'nextRunAt',
      'scheduleProblem',
      'permissionHold',
      'budgetHold',
      'runsOn',
      'endedNote',
      'title',
    ]) {
      expect(block, field).toContain(`match.${field} !== current.${field}`);
    }
  });
});
