/**
 * Approval badges on chat tool rows (Paperclip PR 21). Pure: type imports
 * only, so the node vitest suite can load it.
 *
 * The approval broker binds an `approval-request` to the agent's tool call
 * (same tool, same arguments) and names it by `toolUseId`. The webview then
 * badges exactly that row: "Awaiting approval" while the card is open, and
 * the outcome once `approval-resolved` arrives. An unbound request (null
 * `toolUseId`) badges nothing; its tail card still works.
 */
import type { ApprovalReason, ApprovalResolved } from './types';

/** A row's approval state: waiting, or how it ended. */
/** B4: `allowed_phone` and `denied_phone` are an iPhone's answers. */
export type ApprovalBadge = 'pending' | ApprovalReason | 'allowed_phone' | 'denied_phone';

const LABELS: Record<ApprovalBadge, string> = {
  pending: 'Awaiting approval',
  allowed: 'Allowed',
  denied: 'Denied',
  timed_out: 'Timed out',
  cancelled: 'Cancelled',
  allowed_phone: 'Allowed on iPhone',
  denied_phone: 'Denied on iPhone',
};

/** The badge text for a state. */
export function approvalBadgeLabel(badge: ApprovalBadge): string {
  return LABELS[badge];
}

/** The badge a resolved event leaves. An older backend sends no `reason`:
 *  `approved` means allowed, `cancelled` means the request ended without an
 *  answer, anything else was a denial. */
export function badgeFromResolved(r: Pick<ApprovalResolved, 'approved' | 'cancelled' | 'reason' | 'answeredBy'>): ApprovalBadge {
  if (r.answeredBy?.kind === 'device') {
    if (r.reason === 'allowed') return 'allowed_phone';
    if (r.reason === 'denied') return 'denied_phone';
  }
  if (r.reason === 'allowed' || r.reason === 'denied' || r.reason === 'timed_out' || r.reason === 'cancelled') return r.reason;
  if (r.approved) return 'allowed';
  if (r.cancelled) return 'cancelled';
  return 'denied';
}

/** Where a tool call with this id sits, searching the newest turn first
 *  (the call an approval targets is almost always in the live turn). */
export function findToolById(
  turns: ReadonlyArray<{ tools: ReadonlyArray<{ id?: string }> }>,
  id: string | null | undefined,
): { turnIdx: number; toolIdx: number } | null {
  if (!id) return null;
  for (let t = turns.length - 1; t >= 0; t--) {
    const tools = turns[t].tools;
    for (let i = tools.length - 1; i >= 0; i--) {
      if (tools[i].id === id) return { turnIdx: t, toolIdx: i };
    }
  }
  return null;
}

/** Badges buffered by tool-use id, because the approval events and the
 *  `toolUse` chat event travel on different channels and either may arrive
 *  first. Capped: the oldest entry goes first. */
export const APPROVAL_BADGE_CAP = 64;

export function rememberBadge(map: Map<string, ApprovalBadge>, id: string, badge: ApprovalBadge): void {
  map.delete(id);
  map.set(id, badge);
  while (map.size > APPROVAL_BADGE_CAP) {
    const oldest = map.keys().next().value;
    if (oldest === undefined) break;
    map.delete(oldest);
  }
}
