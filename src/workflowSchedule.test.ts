import { describe, expect, it } from 'vitest';
import {
  DEFAULT_MISFIRE_GRACE_MIN,
  describeSchedule,
  isValidSchedule,
  nextRuns,
  nextRunState,
  parseScheduleModel,
  renderScheduleModel,
  scheduleCatchUp,
  scheduleWindow,
} from './workflowSchedule';

describe('parseScheduleModel / renderScheduleModel', () => {
  it('round-trips every cron shape Hollow writes', () => {
    // Exact strings written by the app (templates, presets, routine seeds).
    for (const s of [
      'manual',
      '0 0 7 * * * *',        // daily 7am
      '0 30 18 * * * *',      // daily 6:30pm
      '0 0 9 * * Mon-Fri *',  // weekdays 9am
      '0 0 8 * * Mon *',      // Mondays 8am
      '0 0 9 * * Sun *',      // Sundays 9am (workflow-suggester template)
    ]) {
      expect(renderScheduleModel(parseScheduleModel(s))).toBe(s);
    }
  });

  it('classifies structured shapes', () => {
    expect(parseScheduleModel('manual').kind).toBe('manual');
    expect(parseScheduleModel('').kind).toBe('manual');
    expect(parseScheduleModel('0 0 7 * * * *')).toMatchObject({ kind: 'daily', hour: 7, minute: 0 });
    expect(parseScheduleModel('0 15 18 * * Mon-Fri *')).toMatchObject({ kind: 'weekday', hour: 18, minute: 15 });
    expect(parseScheduleModel('0 0 8 * * Wed *')).toMatchObject({ kind: 'weekly', day: 'Wed' });
    // Case-insensitive day names still classify.
    expect(parseScheduleModel('0 0 9 * * mon-fri *').kind).toBe('weekday');
    expect(parseScheduleModel('0 0 9 * * fri *')).toMatchObject({ kind: 'weekly', day: 'Fri' });
  });

  it('preserves unrecognized crons verbatim as custom', () => {
    for (const s of [
      '0 */15 * * * * *',      // interval minutes
      '0 0 * * * * *',         // hourly
      '0 0 */4 * * * *',       // every 4h
      '0 0 9 1 * * *',         // day-of-month pinned
      '0 0 9 * * Mon,Wed *',   // multi-day list
      '30 0 9 * * * *',        // non-zero seconds
      'not-cron',
    ]) {
      const m = parseScheduleModel(s);
      expect(m.kind).toBe('custom');
      expect(renderScheduleModel(m)).toBe(s);
    }
  });

  it('describes schedules compactly', () => {
    expect(describeSchedule('manual')).toBe('manual');
    expect(describeSchedule('0 0 7 * * * *')).toBe('Daily 7:00 AM');
    expect(describeSchedule('0 30 18 * * * *')).toBe('Daily 6:30 PM');
    expect(describeSchedule('0 0 9 * * Mon-Fri *')).toBe('Weekdays 9:00 AM');
    // P1-04: whole day names, not "Mons" / "Thus".
    expect(describeSchedule('0 0 8 * * Mon *')).toBe('Mondays 8:00 AM');
    expect(describeSchedule('0 0 8 * * Thu *')).toBe('Thursdays 8:00 AM');
    expect(describeSchedule('0 0 0 * * Sun *')).toBe('Sundays 12:00 AM');
    expect(describeSchedule('0 0 * * * * *')).toBe('Hourly');
    expect(describeSchedule('0 */15 * * * * *')).toBe('Every 15 min');
  });
});

describe('nextRunState', () => {
  const now = Date.parse('2026-08-03T12:28:00-04:00');
  const iso = (msAgo: number) => new Date(now - msAgo).toISOString();

  it('future stays future', () => {
    expect(nextRunState(iso(-60_000), null, now)).toBe('future');
  });

  it('past-due within grace is a catch-up', () => {
    expect(nextRunState(iso(60_000), null, now)).toBe('catchup');
    expect(nextRunState(iso(DEFAULT_MISFIRE_GRACE_MIN * 60_000), null, now)).toBe('catchup');
  });

  it('past grace is missed (will skip + reschedule)', () => {
    expect(nextRunState(iso(DEFAULT_MISFIRE_GRACE_MIN * 60_000 + 1_000), null, now)).toBe('missed');
    // The 2026-08-03 case: 18h stale close-day.
    expect(nextRunState(iso(18 * 3_600_000), null, now)).toBe('missed');
    // Per-workflow override narrows the window.
    expect(nextRunState(iso(10 * 60_000), 5, now)).toBe('missed');
    expect(nextRunState(iso(4 * 60_000), 5, now)).toBe('catchup');
  });

  it('unparseable timestamps fail safe as future', () => {
    expect(nextRunState('garbage', null, now)).toBe('future');
  });
});

describe('window schedules (Scheduler v2)', () => {
  it('reads window:<name> as a valid schedule with a plain-English label', () => {
    expect(scheduleWindow('window:friday-pm')).toBe('friday-pm');
    expect(scheduleWindow(' window: nightly ')).toBe('nightly');
    expect(scheduleWindow('window:')).toBeNull();
    expect(scheduleWindow('0 0 9 * * * *')).toBeNull();
    expect(isValidSchedule('window:friday-pm')).toBe(true);
    expect(isValidSchedule('window:Bad Name')).toBe(false);
    expect(describeSchedule('window:friday-pm')).toBe('Friday afternoon (15:00–18:00)');
    expect(describeSchedule('window:quarter-end')).toBe('Window: quarter-end');
  });

  it('reads the catch-up from the schedule string', () => {
    expect(scheduleWindow('window:nightly/run-all')).toBe('nightly');
    expect(scheduleCatchUp('window:nightly/run-all')).toBe('run-all');
    expect(scheduleCatchUp('window:nightly')).toBe('run-once-on-wake');
    expect(scheduleCatchUp('0 0 9 * * * *')).toBeNull();
    expect(isValidSchedule('window:nightly/often')).toBe(false);
    expect(describeSchedule('window:nightly/skip')).toBe('Nightly (02:00–05:00) · skips missed runs');
  });

  it('round-trips the default catch-up and needs host starts for preview', () => {
    const m = parseScheduleModel('window:tonight');
    expect(m.kind).toBe('window');
    expect(renderScheduleModel(m)).toBe('window:tonight/run-once-on-wake');
    expect(nextRuns('window:tonight')).toBeNull();
  });
});
