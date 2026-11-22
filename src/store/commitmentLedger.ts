// The commitments ledger (overnight 10-11 A5): whether the view is shown and
// the filter it opens with. Since consolidation lane B (2026-10-13) it is
// Tasks → Commitments (removed in the issue-list rewrite), not an overlay; these functions
// are the aliases the palette, `g k`, Today and the Tasks panel call.
import { createSignal } from 'solid-js';

const [query, setQuery] = createSignal('');
const [request, setRequest] = createSignal(0);
/** The Commitments screen was cut (overnight 11-08 S3). The ledger data stays. */
export const ledgerOpen = () => false;
/** The person filter the view starts with (`''` for none). */
export const ledgerStartQuery = query;
/** Bumps on every open, so an open while the view is already shown still
 *  applies its filter. */
export const ledgerRequest = request;
const [focus, setFocus] = createSignal<string | null>(null);
/** The promise (its note's vault path) an open asked to show: a task's
 *  "Open the promise" lands on that exact row (plan completion item 4). */
export const ledgerFocusPath = focus;
export const clearLedgerFocus = () => setFocus(null);
export function openCommitmentLedger(startQuery = '', focusPath: string | null = null): void { setQuery(startQuery); setFocus(focusPath); setRequest(n => n + 1); }
export function closeCommitmentLedger(): void { /* no screen to close */ }
