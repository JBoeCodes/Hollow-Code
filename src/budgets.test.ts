import { describe, expect, it } from 'vitest';
import {
  allowedMetrics, budgetHoldText, budgetNoticeText, budgetOfLimit, budgetStateLabel, canResume, enforceabilityLabel,
  formatBudgetAmount, isBudgetRefusal, isBudgetStop, isBudgetStopError, latestRunIsBudgetStop, suggestionPrefill,
} from './budgets';

describe('budget formatting and resume validation (Paperclip PR 8a)', () => {
  it('formats amounts and never shows $0 for an unknown figure', () => {
    expect(formatBudgetAmount('tokens', 1_200_000)).toBe('1.2M tokens');
    expect(formatBudgetAmount('costUsd', 1.234, true)).toBe('$1.23 (estimated)');
    expect(formatBudgetAmount('costUsd', 0.001)).toBe('<$0.01');
    for (const missing of [null, undefined, Number.NaN, -1]) {
      expect(formatBudgetAmount('costUsd', missing)).toBe('—');
      expect(formatBudgetAmount('tokens', missing)).toBe('—');
    }
    expect(budgetOfLimit({ metric: 'tokens', observed: 1200, limit: 1000, estimated: false }))
      .toBe('1.2k tokens of 1k tokens');
  });

  it('allows a resume only strictly above the observed usage', () => {
    expect(canResume(1200, 1200)).toBe(false);
    expect(canResume(1201, 1200)).toBe(true);
    expect(canResume(Number.NaN, 1200)).toBe(false);
    expect(canResume(0, -5)).toBe(false);
    expect(canResume(5000, null)).toBe(false);
  });

  it('prefills a raise above usage', () => {
    expect(suggestionPrefill(1200, 1800)).toBe(1800);
    expect(suggestionPrefill(1200, 1000)).toBe(1800);
    expect(suggestionPrefill(null, 1800)).toBeNull();
  });

  it('labels states, enforceability, metrics and chat notices', () => {
    expect(budgetStateLabel('paused')).toBe('Paused');
    expect(budgetStateLabel('paused', true)).toBe('Kept paused');
    expect(budgetStateLabel('unknown')).toBe('Usage unavailable');
    expect(enforceabilityLabel('ok')).toBeNull();
    expect(enforceabilityLabel('none')).toBe('Cannot be enforced');
    expect(allowedMetrics('agent:codex')).toEqual(['tokens']);
    expect(allowedMetrics('agent:antigravity')).toEqual(['tokens']);
    expect(allowedMetrics('global')).toEqual(['tokens', 'costUsd']);
    expect(budgetNoticeText({ message: 'All usage has used 85% of today\'s token limit.' }))
      .toBe('Budget: All usage has used 85% of today\'s token limit.');
  });

  it('recognizes budget holds and refusals', () => {
    expect(budgetHoldText('Budget paused: x')).toBe('paused by a usage budget');
    expect(budgetHoldText(null)).toBeNull();
    expect(isBudgetRefusal('Budget paused: all usage reached today\'s token limit')).toBe(true);
    expect(isBudgetRefusal(new Error('Budget paused: x'))).toBe(true);
    expect(isBudgetRefusal('Automation paused for review')).toBe(false);
  });

  it('recognizes runs a budget stopped (Paperclip PR 8b)', () => {
    const row = (at: string, exitKind: string | null) => ({ at, exitKind });
    expect(isBudgetStop(row('2026-09-25T10:00:00Z', 'budget'))).toBe(true);
    expect(isBudgetStop(row('2026-09-25T10:00:00Z', null))).toBe(false);
    expect(isBudgetStop(null)).toBe(false);
    const log = [row('2026-09-25T09:00:00Z', null), row('2026-09-25T10:00:00Z', 'budget')];
    expect(latestRunIsBudgetStop(log)).toBe(true);
    expect(latestRunIsBudgetStop([...log, row('2026-09-25T11:00:00Z', null)])).toBe(false);
    expect(latestRunIsBudgetStop([])).toBe(false);
    expect(isBudgetStopError('Budget limit reached: Workflow brief reached its daily token limit')).toBe(true);
    expect(isBudgetStopError('claude exited exit status: 1')).toBe(false);
    expect(isBudgetStopError(null)).toBe(false);
  });
});
