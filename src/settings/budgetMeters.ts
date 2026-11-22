/**
 * Settings, Usage, Limits: budgets as meters (Settings PR 8). Daily and
 * weekly budgets first, closest to their limit first; monthly ones below.
 * Each meter draws its warn mark (the budget's warn percent) and, for a hard
 * stop, a mark at the limit. Pure.
 */
import type { BudgetStatus } from '../types';
import { budgetOfLimit } from '../budgets';

export type BudgetLevel = 'ok' | 'warn' | 'over' | 'unknown';

export interface BudgetMeterRow {
  id: string;
  label: string;
  window: BudgetStatus['window'];
  /** The fill, 0 to 1 (clamped; the words keep the real number). */
  fill: number;
  /** Rounded percent used, null when the ledger could not be read. */
  percent: number | null;
  /** Where the warn mark sits, 0 to 1. */
  warnAt: number;
  /** A hard stop: a second mark at the limit. */
  hardStop: boolean;
  level: BudgetLevel;
  /** "$4.10 of $10.00" or "120k of 500k tokens". */
  amount: string;
}

const clamp = (n: number) => Math.min(Math.max(n, 0), 1);

export function budgetMeter(status: BudgetStatus): BudgetMeterRow {
  const percent = status.percent == null || !Number.isFinite(status.percent) ? null : status.percent;
  const warnPercent = Math.min(Math.max(status.warnPercent, 1), 100);
  const level: BudgetLevel = percent == null || status.ledgerError
    ? 'unknown'
    : status.state === 'over' || status.state === 'paused' || percent >= 100
      ? 'over'
      : percent >= warnPercent ? 'warn' : 'ok';
  return {
    id: status.id,
    label: status.scopeLabel,
    window: status.window,
    fill: percent == null ? 0 : clamp(percent / 100),
    percent: percent == null ? null : Math.round(percent),
    warnAt: warnPercent / 100,
    hardStop: status.hardStop,
    level,
    amount: budgetOfLimit(status),
  };
}

/** Day and week budgets, fullest first, then month budgets, fullest first. */
export function limitBudgets(budgets: readonly BudgetStatus[]): { soon: BudgetMeterRow[]; monthly: BudgetMeterRow[] } {
  const byFull = (a: BudgetMeterRow, b: BudgetMeterRow) => (b.percent ?? -1) - (a.percent ?? -1) || a.label.localeCompare(b.label);
  const rows = budgets.map(budgetMeter);
  return {
    soon: rows.filter(r => r.window !== 'month').sort(byFull),
    monthly: rows.filter(r => r.window === 'month').sort(byFull),
  };
}

const PERIOD_WORDS: Record<BudgetStatus['window'], string> = { day: 'Today', week: 'This week', month: 'This month' };

export function budgetPeriodWords(window: BudgetStatus['window']): string {
  return PERIOD_WORDS[window];
}
