// Whether a commitment is snoozed or awake (overnight 10-11 A5): its own tiny
// module because Today, Home and the Tasks panel need it at launch, while the
// rest of the ledger's helpers (`commitmentLedger.ts`) load with the ledger.
import type { Commitment } from './types';

/** Snoozed past `today` (`YYYY-MM-DD`): it wakes on its snooze day. */
export function isSnoozed(c: Commitment, today: string): boolean {
  return c.status === 'open' && !!c.snoozedUntil && c.snoozedUntil > today;
}

/** Open and awake: what the open list, Today and the Tasks panel show. */
export function isAwake(c: Commitment, today: string): boolean {
  return c.status === 'open' && !isSnoozed(c, today);
}
