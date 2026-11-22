/**
 * Settings, Usage, Limits: the controls for automatic Claude rotation.
 * The panel passes `api` in so tests can verify their saved values.
 */
import type { ClaudeAccountRouting, ClaudeAccountUpdate, ClaudeAccountsView } from '../types';

/** The slice of `api` the Claude limit controls use. */
export interface ClaudeAccountsCalls {
  setClaudeAccountRouting(routing: ClaudeAccountRouting, pinned: string | null, ceilingPercent: number | null): Promise<ClaudeAccountsView>;
  updateClaudeAccount(input: ClaudeAccountUpdate): Promise<ClaudeAccountsView>;
  refreshClaudeAccounts(): Promise<ClaudeAccountsView>;
}

/** Move on at's choices. */
export const CEILINGS = [70, 80, 90, 95];

/** The choices with the saved ceiling in them, in order. */
export function ceilingChoices(current: number): number[] {
  return CEILINGS.includes(current) ? CEILINGS : [...CEILINGS, current].sort((a, b) => a - b);
}

export const spreadUsage = {
  /** Move on at: the share of either window an account is skipped at. */
  moveOnAt: (calls: ClaudeAccountsCalls, percent: number) => calls.setClaudeAccountRouting('spread', null, percent),
  /** In rotation: whether runs may use the account. */
  inRotation: (calls: ClaudeAccountsCalls, id: string, on: boolean) => calls.updateClaudeAccount({ id, enabled: on }),
  /** Check usage: read every account's 5-hour and weekly windows now. */
  checkUsage: (calls: ClaudeAccountsCalls) => calls.refreshClaudeAccounts(),
};
