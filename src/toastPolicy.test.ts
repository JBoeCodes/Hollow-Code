import { describe, expect, it } from 'vitest';
import { EXPECTED_CANCELLATIONS, isExpectedCancellation, suppressToast } from './toastPolicy';

describe('suppressToast', () => {
  it('keeps ordinary notices in the bell regardless of their destination', () => {
    expect(suppressToast({ actionUrl: 'workflow:morning-brief', source: 'morning-brief' })).toBe(true);
    expect(suppressToast({ actionUrl: 'workflow:close-my-day', source: 'close-my-day' })).toBe(true);
    expect(suppressToast({ actionUrl: null, source: null })).toBe(true);
    // PER-36: run-guard's bell-only notices (shared-doc notes alone).
    expect(suppressToast({ kind: 'warning', actionUrl: 'settings:history', source: 'run-guard-docs' })).toBe(true);
  });

  it('still interrupts for failures and safety or permission notices', () => {
    expect(suppressToast({ kind: 'error', actionUrl: 'workflow:morning-brief' })).toBe(false);
    // Safety and attention notices always toast.
    expect(suppressToast({ actionUrl: 'workflow:morning-brief', source: 'run-guard' })).toBe(false);
    expect(suppressToast({ actionUrl: 'workflow-permissions:morning-brief', source: 'morning-brief' })).toBe(false);
    expect(suppressToast({ actionUrl: 'workflow-failures:morning-brief', source: 'morning-brief' })).toBe(false);
    expect(suppressToast({ actionUrl: 'budget-pauses:', source: 'budgets' })).toBe(false);
    expect(suppressToast({ actionUrl: 'settings:usage', source: 'budgets' })).toBe(false);
    expect(suppressToast({ kind: 'warning', actionUrl: 'settings:system', source: 'disk-guard' })).toBe(false);
    expect(suppressToast({ kind: 'warning', actionUrl: 'settings:connectors', source: 'claude-accounts' })).toBe(false);
  });
});

describe('isExpectedCancellation', () => {
  it('recognizes both backend strings, as strings or errors', () => {
    expect(EXPECTED_CANCELLATIONS).toHaveLength(2);
    expect(isExpectedCancellation('cancelled before it started')).toBe(true);
    expect(isExpectedCancellation('Morning brief was stopped while waiting for a background agent slot')).toBe(true);
    expect(isExpectedCancellation(new Error('cancelled before it started'))).toBe(true);
    expect(isExpectedCancellation({ message: 'cancelled before it started' })).toBe(true);
  });

  it('never treats a failure as expected', () => {
    expect(isExpectedCancellation('claude exited 1')).toBe(false);
    expect(isExpectedCancellation(null)).toBe(false);
    expect(isExpectedCancellation(undefined)).toBe(false);
  });
});
