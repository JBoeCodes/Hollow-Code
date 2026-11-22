import { expect, it } from 'vitest';
import { formatRun, missedRunSentences, nextRuns, parseScheduleModel, renderScheduleModel, scheduleCatchUp, scheduleWindow } from './workflowSchedule';

it('offers a named window as its own editable schedule kind', () => {
  expect(parseScheduleModel('window:nightly/run-all').kind).toBe('window');
});

it('describes the window run-all limit without claiming no backlog', () => {
  const text = missedRunSentences({ schedule: 'window:nightly/run-all', misfireGraceMin: null }).join(' ');
  expect(text).toMatch(/at most 5\b/);
  expect(text).not.toContain('never as a backlog');
});

it.each(['window:nightly', 'window:nightly/skip', 'window:nightly/run-once-on-wake', 'window:nightly/run-all'])('retains window and catch-up through %s', schedule => {
  const rendered = renderScheduleModel(parseScheduleModel(schedule));
  expect(parseScheduleModel(schedule).kind).toBe('window');
  expect(scheduleWindow(rendered)).toBe('nightly');
  expect(scheduleCatchUp(rendered)).toBe(scheduleCatchUp(schedule));
});

it('distinguishes skipped window occurrences from one catch-up on wake', () => {
  const skip = missedRunSentences({ schedule: 'window:nightly/skip', misfireGraceMin: null }).join(' ');
  const once = missedRunSentences({ schedule: 'window:nightly/run-once-on-wake', misfireGraceMin: null }).join(' ');
  expect(skip).toMatch(/skip.*missed/i);
  expect(once).toMatch(/once.*wake/i);
  expect(skip).not.toContain('within 2 hours');
  expect(once).not.toContain('within 2 hours');
});

it('uses host-provided starts across DST and formats the host zone', () => {
  const times = ['2026-10-24T00:00:00Z', '2026-10-25T00:00:00Z', '2026-10-26T01:00:00Z'];
  const windows = { timeZone: 'Europe/Paris', overnightCapUsd: 40, windows: [{ name: 'nightly', label: 'Nightly', days: ['sat', 'sun', 'mon', 'tue'], start: '02:00', end: '05:00', period: 'day', openNow: false, nextStart: times[0], nextStarts: times }] };
  const next = nextRuns('window:nightly/run-all', new Date('2026-10-23T12:00:00Z'), 3, 1500, windows);
  expect(next?.runs.map(run => run.at.toISOString())).toEqual(times.map(value => new Date(value).toISOString()));
  expect(next?.timeZone).toBe('Europe/Paris');
  for (const run of next!.runs) expect(formatRun(run, 'en-US')).toContain('2:00 AM');
});

it('keeps a missing saved window and treats missing host coverage as unavailable', () => {
  const model = parseScheduleModel('window:removed-window/skip');
  expect(renderScheduleModel(model)).toBe('window:removed-window/skip');
  expect(nextRuns('window:removed-window/skip', Date.now(), 3, 1500, {
    windows: [], timeZone: 'America/New_York', overnightCapUsd: 40,
  })).toBeNull();
  expect(nextRuns('window:nightly', Date.now(), 3, 1500, {
    windows: [], timeZone: 'Not/AZone', overnightCapUsd: 40,
  })).toBeNull();
});
