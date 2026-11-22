import { describe, expect, it } from 'vitest';
import { baselineSetOn, costLine, hoursShown, savedCell, savedLine, type ValueTotals } from './valueLedger';

const t = (over: Partial<ValueTotals> = {}): ValueTotals => ({
  runs: 4, accepted: 2, costUsd: 3.648, unknownCostRuns: 0, savedMinutes: 120, savedHoursEstimate: 2, unbaselinedAccepted: 0, ...over,
});

describe('value ledger wording', () => {
  it('never shows an unknown cost as $0', () => {
    expect(costLine(t())).toBe('$3.65');
    expect(costLine(t({ unknownCostRuns: 1 }))).toBe('$3.65 + 1 run with unknown cost');
    expect(costLine(t({ costUsd: 0, unknownCostRuns: 2 }))).toBe('2 runs with unknown cost');
    expect(costLine(t({ costUsd: 0.004, unknownCostRuns: 1 }))).toBe('1 run with unknown cost');
  });

  it('labels hours saved as an estimate, and asks for a baseline when none is set', () => {
    expect(savedLine(t(), true)).toBe('~2.0 h saved from 2 accepted runs (estimate from your baselines)');
    expect(savedLine(t({ accepted: 1 }), true)).toBe('~2.0 h saved from 1 accepted run (estimate from your baselines)');
    expect(savedLine(t(), false)).toBe('Set a baseline below to estimate hours saved (2 accepted runs)');
  });

  it('hides hours saved until a run is accepted (G4)', () => {
    const none = t({ accepted: 0, savedMinutes: 0, savedHoursEstimate: 0 });
    expect(hoursShown(none)).toBe(false);
    expect(savedLine(none, true)).toBeNull();
    expect(savedLine(none, false)).toBeNull();
    expect(hoursShown(t())).toBe(true);
  });

  it('says "baseline not set" instead of 0 hours for accepted work with none (overnight A12)', () => {
    expect(savedLine(t({ savedMinutes: 0, savedHoursEstimate: 0, unbaselinedAccepted: 2 }), true)).toBe('Hours saved: baseline not set for this work (2 accepted runs)');
    expect(savedLine(t({ unbaselinedAccepted: 1 }), true)).toBe('~2.0 h saved from 2 accepted runs (estimate from your baselines; 1 without a baseline)');
    expect(savedCell(t({ savedMinutes: 0, savedHoursEstimate: 0, unbaselinedAccepted: 2 }))).toBe('baseline not set');
    expect(savedCell(t({ unbaselinedAccepted: 1 }))).toBe('~2.0 h + 1 not set');
    expect(savedCell(t())).toBe('~2.0 h');
    expect(savedCell(t({ accepted: 0, savedMinutes: 0, savedHoursEstimate: 0 }))).toBe('~0.0 h');
    expect(savedCell(t({ savedMinutes: 0, savedHoursEstimate: 0, unknownKindAccepted: 2 }))).toBe('work kind unknown');
    expect(savedLine(t({ unknownKindAccepted: 1 }), true)).toBe('~2.0 h saved from 2 accepted runs (estimate from your baselines; 1 of unknown kind)');
  });

  it('shows when a baseline was set, or that it is not', () => {
    expect(baselineSetOn({ setAt: '2026-10-09T12:00:00Z' })).toBe('set 2026-10-09');
    expect(baselineSetOn(undefined)).toBe('not set');
  });
});
