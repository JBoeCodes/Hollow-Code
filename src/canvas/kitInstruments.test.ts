import { describe, expect, it } from 'vitest';
import { gaugeValue, sparkPaths, calendarDay, timelineRows, heatmapDays } from './kit/instruments';

describe('canvas instruments preserve the meaning of their data', () => {
  it('clamps only gauge geometry, keeps out-of-range values and unknowns explicit', () => {
    expect(gaugeValue('125', '0', '100')).toEqual({ value: 125, min: 0, max: 100, fraction: 1 });
    expect(gaugeValue('-5', '0', '100').fraction).toBe(0);
    expect(gaugeValue('', '', '').value).toBeNull();
    expect(gaugeValue('20', '20', '20').fraction).toBeNull();
    expect(gaugeValue('20', '50', '10').fraction).toBeNull();
  });
  it('never joins a sparkline across missing values, handles single and constant values', () => {
    const paths = sparkPaths([1, null, 3, 3], 100, 40);
    expect(paths).toHaveLength(2);
    expect(paths[0]).not.toContain('L');
    expect(paths[1]).toContain('L');
    expect(sparkPaths([0, 0], 100, 40).join('')).not.toMatch(/NaN|Infinity/);
    expect(sparkPaths([], 100, 40)).toEqual([]);
    expect(sparkPaths([null], 100, 40)).toEqual([]);
  });
  it('parses calendar dates strictly without rolling over invalid days or DST', () => {
    expect(calendarDay('2026-02-30')).toBeNull();
    expect(calendarDay('2024-02-29')).not.toBeNull();
    expect(calendarDay('2026-03-09')! - calendarDay('2026-03-08')!).toBe(1);
    expect(calendarDay('10/11/2026')).toBeNull();
  });
  it('rejects reversed spans and retains a count of invalid/omitted timeline rows', () => {
    const rows = timelineRows([['Date', 'Label', 'End'], ['2026-10-01', 'Start'], ['2026-10-02', 'Build', '2026-10-06'], ['2026-10-09', 'Bad', '2026-10-08']]);
    expect(rows.items.map(x => x.label)).toEqual(['Start', 'Build']);
    expect(rows.invalid).toBe(1);
    expect(timelineRows(Array.from({ length: 65 }, () => ['2026-10-01', 'Item'])).omitted).toBe(5);
  });
  it('distinguishes an explicit zero from missing days and missing values', () => {
    const m = heatmapDays([['2026-10-01', '0'], ['2026-10-03', '—']], '', '');
    expect(m.days.map(x => x.value)).toEqual([0, null, null]);
    expect(m.days.map(x => x.date)).toEqual(['2026-10-01', '2026-10-02', '2026-10-03']);
  });
  it('respects explicit table headers with arbitrary names', () => {
    const timeline = timelineRows([['When', 'What'], ['2026-10-01', 'Start']], true);
    expect(timeline.invalid).toBe(0);
    expect(timeline.rows).toEqual([['2026-10-01', 'Start']]);
    const map = heatmapDays([['Day', 'Count'], ['2026-10-01', '3']], '', '', true);
    expect(map.invalid).toBe(0);
    expect(map.rows).toEqual([['2026-10-01', '3']]);
  });
  it('does not invent heatmap data, sum duplicate dates, or allocate unbounded ranges', () => {
    expect(heatmapDays([], '', '').days).toEqual([]);
    const m = heatmapDays([['2026-10-01', '1'], ['2026-10-01', '4'], ['bad', '2']], '2026-01-01', '2030-01-01');
    expect(m.days).toHaveLength(366);
    expect(m.duplicates).toBe(1);
    expect(m.invalid).toBe(1);
    expect(m.truncated).toBe(true);
    expect(m.days.find(x => x.date === '2026-10-01')?.value).toBe(4);
    expect(heatmapDays([], '2026-10-03', '2026-10-01').invalidRange).toBe(true);
  });
});
