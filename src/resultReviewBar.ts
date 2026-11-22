// The review bar a Result opens with in the Files tab (COO rebuild L4,
// 2026-11-10; it was the Results section's review, ResultsView.tsx). Which
// controls it offers for a Result's review state. Accepting stays the
// person's own: there is no agent action for it (decision 3).
import type { WorkResult } from './workspaceTypes';

export interface ReviewBarControl { label: string; disabled: boolean; title?: string; primary: boolean }

export interface ReviewBarModel {
  /** Mark ready for review (a draft only). */
  markReady: ReviewBarControl | null;
  /** Accept, or Accept this new version after a change. */
  accept: ReviewBarControl | null;
  /** Compare with accepted (a changed Result with its accepted snapshot). */
  compare: boolean;
  /** More → Move back to draft (anything but a draft). */
  backToDraft: boolean;
}

/** The bar's controls for `r`, while `busy` (a review being recorded). */
export function reviewBarModel(r: Pick<WorkResult, 'review' | 'sha256' | 'hasSnapshot'>, busy = false): ReviewBarModel {
  const exact = !!r.sha256;
  const tooLarge = 'Too large to record an exact version';
  const markReady = r.review === 'draft'
    ? { label: 'Mark ready for review', disabled: busy || !exact, title: exact ? undefined : tooLarge, primary: true }
    : null;
  const accept = r.review === 'ready' || r.review === 'changed' || r.review === 'draft'
    ? { label: r.review === 'changed' ? 'Accept this new version' : 'Accept', disabled: busy || !exact,
      title: exact ? 'Accept exactly this version' : tooLarge, primary: r.review !== 'draft' }
    : null;
  return { markReady, accept, compare: r.hasSnapshot && r.review === 'changed', backToDraft: r.review !== 'draft' };
}

/** The toast after a review was recorded. */
export function reviewedMessage(state: 'draft' | 'ready' | 'accepted', title: string): string {
  return state === 'accepted' ? `Accepted “${title}”. Nothing was sent or published.`
    : state === 'ready' ? `Marked “${title}” ready for review.` : `“${title}” is a draft again.`;
}
