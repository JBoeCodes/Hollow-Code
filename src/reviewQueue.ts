import { createResource, createSignal } from 'solid-js';
import { api } from './api';
import { reviewInboxRevision } from './reviewInbox';
import { isWorkspaceReviewItem } from './workspaceContent';
import type { ReviewInboxItem } from './types';

// ── Local suggestions: one snapshot of the vault-canonical
// .review-queue.json v2. The "waiting on you" surface opens its items as
// cards, the page links (WaitingLink) count them, and contextual chat reads
// them. It stays cold until a surface enables it. Tracking
// `reviewInboxRevision` refreshes every reader after any surface resolves an
// item; `reloadReviewQueue` re-reads it without bumping the revision, for
// writers that bump nothing (a chat learning proposal, a commitment
// proposal). Its own module, re-exported by the store, so the surface's
// state (store/waitingOnYou.ts) can reload it without importing the store.
const [approvalsEnabled, setApprovalsEnabled] = createSignal(false);
const [failed, setFailed] = createSignal(false);
export const [reviewQueueItems, { refetch: refetchApprovals }] =
  createResource<ReviewInboxItem[], number>(
    () => (approvalsEnabled() ? reviewInboxRevision() + 1 : false),
    async (_revision, info) => {
      try {
        const items = (await api.listReviewInbox()).filter(isWorkspaceReviewItem);
        setFailed(false);
        return items;
      } catch {
        // Keep the last answer rather than reading as "nothing waiting", and
        // say the read failed (`reviewQueueFailed`).
        setFailed(true);
        return (info.value as ReviewInboxItem[] | undefined) ?? [];
      }
    },
  );
export function enableApprovalsResource() { setApprovalsEnabled(true); }

/** Whether the last read of the queue failed: what is shown is the last
 *  good answer (or none), not a fresh one. */
export const reviewQueueFailed = failed;

let reloading: Promise<unknown> | null = null;
/** Re-read the snapshot now, without bumping `reviewInboxRevision` (the bell
 *  refreshes the surface on every bump, and the surface reloads the queue on
 *  every refresh, so a bump here would loop). Calls made while a re-read is
 *  in flight share it. Does nothing until a surface enabled the snapshot. */
export function reloadReviewQueue(): Promise<unknown> {
  if (!approvalsEnabled()) return Promise.resolve();
  if (!reloading) {
    reloading = Promise.resolve(refetchApprovals()).catch(() => undefined).finally(() => { reloading = null; });
  }
  return reloading;
}

/// Pending decisions across every local suggestion kind.
/// `applying` counts as pending (same semantics as the review links' count):
/// it's the transient claim state while an accept is being applied.
export function approvalsPendingCount(): number {
  return (reviewQueueItems() ?? [])
    .filter(i => i.status === 'pending' || i.status === 'applying').length;
}
