/**
 * Where a subagent's approval card shows and how it is labelled (SA1). Kept
 * out of `chatApproval.ts` so the shell chunk, which loads that file for the
 * badges, does not carry card code only the chat view uses.
 */
import type { ApprovalRequest } from './types';

/** Whether a chat whose conversation chain is `chainKeys` shows this card
 *  (SA1). An unbound request shows everywhere; a bound one shows in the
 *  conversation it belongs to, and, when that is a subagent, in the one
 *  that started it. */
export function approvalShownIn(
  approval: Pick<ApprovalRequest, 'sessionKey' | 'subagent'>,
  chainKeys: ReadonlySet<string>,
): boolean {
  if (!approval.sessionKey) return true;
  if (chainKeys.has(approval.sessionKey)) return true;
  const parent = approval.subagent?.parentKey;
  return !!parent && chainKeys.has(parent);
}

/** The line that names the subagent a card came from, or null. */
export function approvalOriginLabel(approval: Pick<ApprovalRequest, 'subagent'>): string | null {
  const sub = approval.subagent;
  if (!sub) return null;
  const title = sub.title?.trim();
  return title ? `From subagent: ${title}` : 'From a subagent';
}

/** What a chat draws for a card (2026-11-20): `card` (its Allow/Deny),
 *  `waiting` (a one-line note that it waits in the COO chat, with a link),
 *  or `hidden`. A card raised to the COO shows as a card only in the COO
 *  chat and as a note only in the chat it came from (or its parent), so
 *  there is never a second copy to answer; one id resolves both. */
export function approvalPlacement(
  approval: Pick<ApprovalRequest, 'sessionKey' | 'subagent' | 'routedTo'>,
  chainKeys: ReadonlySet<string>,
  inCoo: boolean,
): 'card' | 'waiting' | 'hidden' {
  if (!approval.routedTo) return approvalShownIn(approval, chainKeys) ? 'card' : 'hidden';
  if (inCoo) return 'card';
  return approvalShownIn(approval, chainKeys) ? 'waiting' : 'hidden';
}

/** The note a routed card leaves in the chat it came from. */
export function approvalWaitingLabel(approval: Pick<ApprovalRequest, 'routedTo'>): string {
  return approval.routedTo ? 'Waiting for approval in the COO chat' : '';
}
