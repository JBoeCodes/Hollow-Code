import { createComputed, createRoot, createSignal } from 'solid-js';
import { onReviewPage, openReviewView, reviewView, reviewViewShown } from './destinations';
import { api } from '../api';
import { createNeedsYouList } from '../needsYou';
import { discardDraft, listDrafts, openDraftPaths, writeDraft } from '../fileDrafts';
import { flashFooter, flashWithUndo } from './snackbars';
import { reloadReviewQueue } from '../reviewQueue';
import type { SuggestionContext } from '../workspaceContent';
import type { NeedsYouItem } from '../workspaceTypes';
import {
  WAITING_FILTER_PAGE, WAITING_PAGE, countedDrafts, reviewTabCounts, reviewViewCount, reviewViewKinds, waitingBadgeOf, waitingCount, waitingQueryCategories, waitingQueryKinds,
  type WaitingBadge, type WaitingDraft, type WaitingView,
} from '../waitingOnYou';

/**
 * State of the one "waiting on you" surface (decision 7): whether it is
 * open, which page's review kinds it is filtered to, and the two paged
 * lists it reads from `list_needs_you`. The bell's Needs you tab and See all,
 * Home's summary line, the
 * Tasks / Settings / Automations links and every old "open the review
 * panel" deep link (`requestSuggestions`) all open it here.
 */
// Since consolidation D (2026-10-13) the surface is the Review page; it is
// "open" while Review shows one of its waiting views (not Incoming).
export const waitingOpen = () => reviewViewShown('all') || reviewViewShown('decisions') || reviewViewShown('results') || reviewViewShown('failures');
export const [waitingFilter, setWaitingFilter] = createSignal<SuggestionContext | null>(null);
// A Home pip uses the same exact-item opener as Review, once its lazy panel
// mounts. Consumed once; this is navigation, never an approval or answer.
export const [waitingItemRequest, setWaitingItemRequest] = createSignal<NeedsYouItem | null>(null);
let disposeItemRequest: (() => void) | undefined;
let itemRequestSequence = 0;
export async function openWaitingItem(item: NeedsYouItem): Promise<boolean> {
  const sequence = ++itemRequestSequence;
  disposeItemRequest?.();
  setWaitingItemRequest(null);
  setWaitingFilter(null);
  // An editor may hold or cancel navigation. Publish only after the actual
  // destination arrives, including the view transition's delayed update.
  try { await openReviewView('all'); } catch { return false; }
  if (sequence !== itemRequestSequence || !onReviewPage() || reviewView() !== 'all') return false;
  refreshWaitingOnYou(true);
  setWaitingItemRequest(item);
  // Watch navigation now, even if Review's lazy page never mounts. An old
  // click must not fire on an unrelated later visit after a failed chunk.
  disposeItemRequest = createRoot(dispose => {
    createComputed(() => {
      if (!waitingItemRequest()) { dispose(); return; }
      if (!onReviewPage() || reviewView() !== 'all') { setWaitingItemRequest(null); dispose(); }
    });
    return dispose;
  }, null);
  return true;
}

/** Everything waiting, every kind, a page at a time. Also the bell's count. */
export const waitingList = createNeedsYouList((cursor, limit) => api.listNeedsYou(cursor, limit, waitingQueryKinds(null)), WAITING_PAGE);
/** That page's review items only, for the surface filtered to one page's
 *  review kinds. The query filters by category before paging, so its total
 *  and "Show more" count only that page's items. */
export const waitingReviews = createNeedsYouList((cursor, limit) => {
  const context = waitingFilter();
  return api.listNeedsYou(cursor, limit, waitingQueryKinds(context), waitingQueryCategories(context));
}, WAITING_FILTER_PAGE);

/** Decisions, Results and Failures, each its own query of its own kinds
 *  (P0-09), loaded while Review is on screen. */
const viewList = (view: Exclude<WaitingView, 'all'>) =>
  createNeedsYouList((cursor, limit) => api.listNeedsYou(cursor, limit, reviewViewKinds(view, null)), WAITING_PAGE);
const viewLists = { decisions: viewList('decisions'), results: viewList('results'), failures: viewList('failures') };
/** The list a waiting view shows (no page filter). */
export const waitingListFor = (view: WaitingView) => (view === 'all' ? waitingList : viewLists[view]);
/** Review's tab counts, by the one rule (`reviewViewCount`, `reviewTabCounts`). */
export function waitingTabCounts(): Record<WaitingView, number | undefined> {
  const count = (view: Exclude<WaitingView, 'all'>) => {
    const list = viewLists[view];
    return list.loaded() ? reviewViewCount(view, { total: list.total(), items: list.items() }, waitingDrafts()) : undefined;
  };
  return reviewTabCounts(waitingBadgeState().count, { decisions: count('decisions'), results: count('results'), failures: count('failures') }, waitingDrafts().length);
}

/** This window's unsaved file drafts: localStorage, which the query can't
 *  see. Only those no editor has open and idle ten minutes (P1-14, D3);
 *  each refresh re-reads them, so one crosses in within a minute. */
export const [waitingDrafts, setWaitingDrafts] = createSignal<WaitingDraft[]>(readDrafts());
function readDrafts(): WaitingDraft[] {
  try { return countedDrafts(listDrafts().map(d => ({ path: d.path, savedAt: d.savedAt })), openDraftPaths(), Date.now()); } catch { return []; }
}

/** The bell's waiting number: everything that can be acted on (0
 *  while unknown; `waitingBadgeState` says which). Drafts are apart (D3). */
export function waitingBadge(): number {
  return waitingCount(waitingList.total(), waitingList.items());
}

/** The bell's waiting count: unknown before the first answer and after a
 *  failed read, else a number, with drafts named apart (P1-14). */
export function waitingBadgeState(): WaitingBadge {
  return waitingBadgeOf({ loaded: waitingList.loaded(), failed: waitingList.failed(), total: waitingList.total(), items: waitingList.items() }, waitingDrafts());
}

/** Review's Discard on a draft row (P1-14, D3): gone from storage and the
 *  editor's memory, the file unchanged, with an Undo that writes it back as
 *  it was. */
export function discardWaitingDraft(path: string): void {
  const name = path.split('/').pop() || path;
  // Opened since the list was read: the editor owns it now (its own Discard).
  if (openDraftPaths().has(path)) { refreshWaitingOnYou(true); flashFooter(`${name} is open in an editor: discard the draft there.`); return; }
  const draft = discardDraft(path);
  refreshWaitingOnYou(true);
  if (!draft) { flashFooter(`The draft of ${name} was already gone.`); return; }
  flashWithUndo({
    message: `Draft of ${name} discarded. The file is unchanged.`,
    kind: 'delete',
    undo: () => {
      const at = Date.parse(draft.savedAt);
      if (!writeDraft(path, draft.text, draft.baseline, undefined, Number.isNaN(at) ? new Date() : new Date(at))) flashFooter('Couldn’t bring the draft back: local draft storage is full.');
      refreshWaitingOnYou(true);
    },
  });
}

let pending: ReturnType<typeof setTimeout> | undefined;
/** Reload what is shown. Bursts (an approval card and a session change in
 *  the same tick) coalesce into one query. The review queue snapshot the
 *  cards and page links read reloads with it: some writers (a chat learning
 *  proposal, a commitment proposal) bump nothing it watches, so without this
 *  the query could list an item whose card the snapshot doesn't have yet. */
export function refreshWaitingOnYou(now = false): void {
  if (pending) clearTimeout(pending);
  pending = undefined;
  const run = () => {
    pending = undefined;
    void waitingList.load();
    if (onReviewPage()) for (const list of Object.values(viewLists)) void list.load();
    if (waitingFilter()) void waitingReviews.load();
    void reloadReviewQueue();
    setWaitingDrafts(readDrafts());
  };
  if (now) run();
  else pending = setTimeout(run, 150);
}

/** The page whose review items `waitingReviews` holds. */
let reviewsFor: SuggestionContext | null = null;

/** Open the surface, filtered to one page's review kinds or showing
 *  everything (`null`). */
export function openWaitingOnYou(context: SuggestionContext | null = null, view?: WaitingView): void {
  if (context === 'learning') {
    void import('./libraryMode').then(m => m.openLibraryLearnings());
    return;
  }
  // Another page's review items never stand in for this page's, not even
  // until the new answer lands: the filtered list starts over unloaded.
  if (context && context !== reviewsFor) waitingReviews.reset();
  if (context) reviewsFor = context;
  setWaitingFilter(context);
  // A page's own review link shows everything of that kind. Without an
  // explicit view, page links keep the last chosen waiting view.
  // Incoming was cut (overnight 11-08 S3).
  openReviewView(view ?? (context ? 'all' : reviewView()));
  refreshWaitingOnYou(true);
}

/** Kept for callers that used to close the popover before navigating
 *  elsewhere: leaving Review is now the navigation itself. */
export function closeWaitingOnYou(): void { /* a page: nothing to close */ }
