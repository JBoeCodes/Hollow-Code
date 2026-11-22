// The views inside each primary destination (consolidation slice D,
// 2026-10-13). Today, Weekly review, the workspace map and Mission Control
// used to be full-screen overlays on top of the whole app; they are now views
// of Your work, reached from its visible tab strip. Their old open/close
// functions (palette, `g` keys, Home links, agents' navigate) route here, so
// every entry point lands in the same place with the same state. Since S3
// (2026-11-03) Weekly review is Today's Week range, not a tab of its own.
//
// Kept free of component imports: the shell and the views' small open-state
// modules read it without pulling in a lazy chunk.
import { batch, createSignal } from 'solid-js';

/** How this module asks "is Your work's overview on screen?" and goes there.
 *  store/workspaces.ts binds it at load, so this module imports no store and
 *  the views' small open-state modules stay light (and unit-testable). */
let nav: { onHome: () => boolean; goHome: (arrived: () => void) => void | Promise<void> } = { onHome: () => true, goHome: () => {} };
export function bindDestinationNav(next: typeof nav) { nav = next; }

/** Your work's views. `home` is the overview (Ask, recent projects).
 *  Today and Weekly review were cut (overnight 11-08 S3). The workspace map
 *  was cut (overnight 11-08 S4). */
export type WorkView = 'home' | 'activity' | 'pulls';

export const WORK_VIEWS: readonly { id: WorkView; label: string; hint: string }[] = [
  // "Home", not "Overview": a workspace's side panel has its own Overview (P1-05).
  { id: 'home', label: 'Home', hint: 'Start a conversation or open a recent project' },
  // "Running", not "Activity": that word named five places (P1-03).
  { id: 'activity', label: 'Running', hint: 'What is running now and what just finished, with evidence' },
  // Parity 7.3: a project's GitHub pull requests and issues through gh; Linear stays the tracker.
  { id: 'pulls', label: 'Pull requests', hint: 'A project\u2019s GitHub pull requests and issues, with checks, comments and triage' },
];

const [workViewSignal, setWorkViewSignal] = createSignal<WorkView>('home');
export const workView = workViewSignal;

/** Your work is on screen (its overview area, not an open workspace). */
export const onYourWork = () => nav.onHome();

/** Whether `view` is the one on screen now. */
export const workViewShown = (view: WorkView) => onYourWork() && workViewSignal() === view;

/** Show one of Your work's views, leaving any open workspace.
 *  `arrived` runs only once the jump is accepted, in the same batch as the
 *  view. */
export function openWorkView(view: WorkView, arrived?: () => void) {
  const select = () => batch(() => { setWorkViewSignal(view); arrived?.(); });
  // Commit with the accepted jump, including delayed editor confirmations.
  // Refused or superseded requests must not change the hidden destination.
  if (!onYourWork()) return nav.goHome(select);
  select();
}

/** Projects → Pull requests (parity 7.3). */
export const openPullRequests = () => openWorkView('pulls');

/** Back to the overview, staying on Your work. */
export function closeWorkView(view: WorkView) {
  if (workViewSignal() === view) setWorkViewSignal('home');
}

/** The rail's Your work row always lands on the overview. */
export function resetWorkView() { setWorkViewSignal('home'); }

/** Review's views (consolidation D): everything waiting on the person, and
 *  three focused slices of it. Incoming was cut (overnight 11-08 S3).
 *  Granting permission and accepting a Result stay different actions in
 *  every view; nothing here approves in bulk. */
export type ReviewView = 'all' | 'decisions' | 'results' | 'failures';

export const REVIEW_VIEWS: readonly { id: ReviewView; label: string; hint: string }[] = [
  { id: 'all', label: 'All', hint: 'Failures first; newest first within groups; running chats lead within approvals and questions' },
  { id: 'decisions', label: 'Decisions', hint: 'Approvals, questions, proposals and conflicts only you can settle' },
  { id: 'results', label: 'Results', hint: 'Outputs waiting for you to accept or ask for a revision' },
  { id: 'failures', label: 'Failures', hint: 'Runs and syncs that failed, with how to recover' },
];

let reviewNav: { onReview: () => boolean; goReview: () => void | Promise<void> } = { onReview: () => true, goReview: () => {} };
export function bindReviewNav(next: typeof reviewNav) { reviewNav = next; }

const [reviewViewSignal, setReviewViewSignal] = createSignal<ReviewView>('all');
export const reviewView = reviewViewSignal;
export const reviewViewShown = (view: ReviewView) => reviewNav.onReview() && reviewViewSignal() === view;
/** Whether the Review page is on screen, on any view. */
export const onReviewPage = () => reviewNav.onReview();
/** Open Review on one view. */
export function openReviewView(view: ReviewView) {
  setReviewViewSignal(view);
  if (!reviewNav.onReview()) return reviewNav.goReview();
}
