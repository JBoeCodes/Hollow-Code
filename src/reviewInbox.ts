import { createSignal } from 'solid-js';
import type { SuggestionContext } from './workspaceContent';
import { openWaitingOnYou } from './store/waitingOnYou';

/** Shared invalidation token for every review consumer (the waiting-on-you
 *  surface's cards, the review queue snapshot, Settings, Memory). */
export const [reviewInboxRevision, setReviewInboxRevision] = createSignal(0);
export function refreshReviewInbox() {
  setReviewInboxRevision(value => value + 1);
}

/** Open a page's review items: the one "waiting on you" surface, filtered
 *  to that page's kinds (decision 7 retired the per-page review inboxes).
 *  Notification links, run refusals and the Automations fix buttons land
 *  here. */
export function requestSuggestions(context: SuggestionContext) {
  openWaitingOnYou(context);
}
