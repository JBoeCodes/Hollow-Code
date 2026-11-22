/** The spending caps Hollow keeps for you (P1-07), listed in Settings, Usage
 *  → Budgets above your own budgets: the Main agent's daily hard stop and the
 *  overnight cap (both edited there). The bake-off monthly cap went with the
 *  bake-off runner (11-10, S6), and the last night's spend with the night queue. */

export type CapState = 'ok' | 'warn' | 'stopped' | 'unknown';

export interface BuiltInCap {
  id: 'main-agent' | 'overnight';
  label: string;
  /** What the cap covers, in a sentence. */
  covers: string;
  spentUsd: number | null;
  limitUsd: number | null;
  /** Measured fill for the spend's own period; absent for incomplete or zero-cap data. */
  meterPercent: number | null;
  costIncomplete: boolean;
  spentWords: string;
  state: CapState;
  /** Edited inline here; false while its limit is unread. */
  editable: boolean;
}

export interface BuiltInCapsInput {
  main: { spentUsd: number | null; capUsd: number } | null;
  overnightCapUsd: number | null;
}

/** The highest cap typed here, as the Main agent setting allows. */
export const MAX_CAP_USD = 1000;

const money = (n: number) => `$${n.toFixed(2)}`;
const cap = (n: number) => `$${n.toFixed(n % 1 ? 2 : 0)}`;
const measuredPercent = (spent: number | null, limit: number | null): number | null =>
  spent == null || limit == null || !Number.isFinite(spent) || !Number.isFinite(limit) || spent < 0 || limit <= 0
    ? null : Math.min(100, spent / limit * 100);

/** Warn from 75%, stopped at the cap; a $0 cap stops everything. */
export function capState(spent: number | null, limit: number | null): CapState {
  if (spent == null || limit == null) return 'unknown';
  if (limit <= 0 || spent >= limit) return 'stopped';
  return spent / limit >= 0.75 ? 'warn' : 'ok';
}

export function builtInCaps(input: BuiltInCapsInput): BuiltInCap[] {
  const main = input.main;
  const mainSpent = main?.spentUsd ?? null;
  const mainLimit = main?.capUsd ?? null;
  const nightLimit = input.overnightCapUsd;
  return [
    {
      id: 'main-agent', label: 'COO daily hard stop',
      covers: 'Its own turns and the subagents it started. Chats you ask it for are never capped.',
      spentUsd: mainSpent, limitUsd: mainLimit,
      meterPercent: measuredPercent(mainSpent, mainLimit), costIncomplete: false,
      spentWords: mainSpent != null && mainLimit != null ? `${money(mainSpent)} of ${cap(mainLimit)} today` : 'Spend today not read yet',
      state: capState(mainSpent, mainLimit), editable: mainLimit != null,
    },
    {
      id: 'overnight', label: 'Overnight cap',
      covers: 'One night of every unattended run.',
      spentUsd: null, limitUsd: nightLimit,
      meterPercent: null, costIncomplete: false,
      spentWords: 'Spend per night isn’t shown here',
      state: nightLimit != null && nightLimit <= 0 ? 'stopped' : 'ok',
      editable: nightLimit != null,
    },
  ];
}

/** A typed cap in dollars ("$12.50", "25"), or null when it isn't one. */
export function parseCapInput(raw: string): number | null {
  const t = raw.trim().replace(/^\$/, '').trim();
  if (!/^\d+(\.\d{1,2})?$/.test(t)) return null;
  const n = Number(t);
  return n <= MAX_CAP_USD ? n : null;
}
