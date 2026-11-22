import { describe, expect, it } from 'vitest';
import { runDurations } from './runDurations';

describe('loaded run durations', () => {
  it('reads newest-first history in chronological order, preserving zero and missing gaps', () => {
    const result = runDurations([5, Number.NaN, 0, 10].map(durationSec => ({ durationSec })));
    expect(result.points.map(p => p.seconds)).toEqual([10, 0, null, 5]);
    expect(result.path.match(/M/g)).toHaveLength(2);
    expect(result.max).toBe(10);
    expect(result.points[1].y).toBe(44);
  });
  it('bounds the chart to the same 25 most recent records as the reading list', () => {
    expect(runDurations(Array.from({length: 30}, (_, durationSec) => ({ durationSec }))).points.map(p => p.seconds))
      .toEqual(Array.from({length: 25}, (_, i) => 24 - i));
  });
  it('does not manufacture numeric samples from invalid values or empty history', () => {
    expect(runDurations([]).path).toBe('');
    const result = runDurations([-1, Infinity, null, undefined].map(durationSec => ({ durationSec })));
    expect(result.path).toBe('');
    expect(result.max).toBe(0);
    expect(result.points.every(p => p.seconds === null)).toBe(true);
    expect(runDurations([{ durationSec: 0 }]).path).toBe('M150,44');
  });
});
