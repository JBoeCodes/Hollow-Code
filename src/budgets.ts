/**
 * Usage-budget labels and checks (Paperclip PR 8a, gate D2). Pure: no store
 * or Tauri import, so the node vitest suite loads it.
 *
 * Rules every label keeps:
 * - An unknown amount reads "—", never "$0" or "0 tokens": no figure is not
 *   no usage.
 * - An estimated (subscription, list-price) cost says so.
 * - A resume is valid only strictly above this window's observed usage, the
 *   same rule the backend enforces.
 */
import type { BudgetEnforceability, BudgetMetric, BudgetState, BudgetStatus, BudgetWindow } from './types';
import { formatTokens } from './usageFormat';

/** What Settings says about enforcement (mirrors Rust `ENFORCEMENT_COPY`). */
export const ENFORCEMENT_COPY =
  'Hard stop applies to automations: every run is checked before it starts, and a Claude run under these policies that crosses a token limit is stopped mid-run. A spending limit is a backstop Claude applies after each message, so a run can go over by up to one message. Codex, Antigravity and Grok runs are checked only before they start. Ordinary chat only warns.';

/** 1200 tokens → "1.2k tokens"; 1.23 dollars → "$1.23" ("$1.23 (estimated)").
 *  null, NaN and negatives → "—". */
export function formatBudgetAmount(
  metric: BudgetMetric,
  value: number | null | undefined,
  estimated = false,
): string {
  if (value == null || !Number.isFinite(value) || value < 0) return '—';
  if (metric === 'tokens') return `${formatTokens(value)} tokens`;
  const dollars = value > 0 && value < 0.005 ? '<$0.01' : `$${value.toFixed(2)}`;
  return estimated ? `${dollars} (estimated)` : dollars;
}

/** "1.2k tokens of 1k tokens" (observed of limit). */
export function budgetOfLimit(status: Pick<BudgetStatus, 'metric' | 'observed' | 'limit' | 'estimated'>): string {
  return `${formatBudgetAmount(status.metric, status.observed, status.estimated)} of ${formatBudgetAmount(status.metric, status.limit)}`;
}

export function budgetStateLabel(state: BudgetState, held = false): string {
  switch (state) {
    case 'paused': return held ? 'Kept paused' : 'Paused';
    case 'over': return 'Over limit';
    case 'warning': return 'Near limit';
    case 'unknown': return 'Usage unavailable';
    default: return 'OK';
  }
}

export function windowLabel(window: BudgetWindow): string {
  return window === 'day' ? 'Today' : window === 'week' ? 'This week' : 'This month';
}

export function enforceabilityLabel(level: BudgetEnforceability): string | null {
  if (level === 'partial') return 'Partly enforced';
  if (level === 'none') return 'Cannot be enforced';
  return null;
}

/** A resume (raise) is valid only strictly above the observed usage. */
export function canResume(newLimit: number, observed: number | null | undefined): boolean {
  if (!Number.isFinite(newLimit) || newLimit <= 0) return false;
  if (observed == null || !Number.isFinite(observed)) return false;
  return newLimit > observed;
}

/** The raise field's starting value: the suggestion, else 1.5× usage, never
 *  at or below it. */
export function suggestionPrefill(observed: number | null | undefined, suggested: number | null | undefined): number | null {
  if (suggested != null && Number.isFinite(suggested) && canResume(suggested, observed)) return suggested;
  if (observed == null || !Number.isFinite(observed)) return null;
  const raised = Math.ceil(Math.max(observed, 1) * 1.5);
  return raised > observed ? raised : null;
}

/** The chat system line for a `budgetWarning` event. */
export function budgetNoticeText(notice: { message: string }): string {
  return `Budget: ${notice.message}`;
}

/** Metrics a scope can be budgeted on (mirrors the backend's refusals for
 *  agent scopes; workflow and heartbeat scopes are checked by the backend). */
export function allowedMetrics(scope: string): BudgetMetric[] {
  if (scope === 'agent:antigravity' || scope === 'agent:codex') return ['tokens'];
  return ['tokens', 'costUsd'];
}

/** The next-run text of an automation a usage budget holds
 *  (`Workflow.budgetHold`); null when none does. A permission hold
 *  outranks it, and it outranks a queued row and a catch-up. */
export function budgetHoldText(hold: string | null | undefined): string | null {
  return hold ? 'paused by a usage budget' : null;
}

/** Every message that blocks a run starts with this (Rust
 *  `budgets::BUDGET_PAUSED_PREFIX`). */
export const BUDGET_PAUSED_PREFIX = 'Budget paused:';

/** A run a usage budget stopped starts its error with this (Rust
 *  `budgets::BUDGET_STOP_PREFIX`, Paperclip PR 8b). */
export const BUDGET_STOP_PREFIX = 'Budget limit reached:';

/** The run-log exit kind of a run a usage budget ended (Rust
 *  `vault_workflows::EXIT_KIND_BUDGET`). */
export const EXIT_KIND_BUDGET = 'budget';

/** A run-log row a usage budget ended (Paperclip PR 8b): it keeps status
 *  `error` but is not a failure of the workflow, so it stays out of the
 *  failure debugger and Recent failures. */
export function isBudgetStop(entry: { exitKind: string | null } | null | undefined): boolean {
  return entry?.exitKind === EXIT_KIND_BUDGET;
}

/** Whether the newest row of a run log is a budget stop. */
export function latestRunIsBudgetStop(log: readonly { at: string; exitKind: string | null }[] | null | undefined): boolean {
  if (!log || log.length === 0) return false;
  const newest = log.reduce((a, b) => (b.at > a.at ? b : a));
  return isBudgetStop(newest);
}

/** A workflow's `lastError` from a budget stop (the list has no run log;
 *  the message is Hollow's own, never agent text). */
export function isBudgetStopError(error: string | null | undefined): boolean {
  return !!error && error.startsWith(BUDGET_STOP_PREFIX);
}

/** A Run now refused by a usage budget. Tauri rejects with a plain string,
 *  so both strings and Error objects are read. */
export function isBudgetRefusal(error: unknown): boolean {
  const message = typeof error === 'string' ? error : error instanceof Error ? error.message : '';
  return message.startsWith(BUDGET_PAUSED_PREFIX);
}
