import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { NeedsYouItem } from '../workspaceTypes';

// Decision 7: every old "open this page's review panel" link opens the one
// "waiting on you" surface instead, filtered to that page's review kinds.
const state = vi.hoisted(() => ({
  calls: [] as Array<{ cursor: string | null; limit: number; kinds: string[] | undefined; categories: readonly string[] | undefined }>,
  // The review queue snapshot the surface's cards read (../reviewQueue),
  // stubbed: the ids it holds, and how often the surface re-read it.
  snapshot: [] as string[],
  queue: ['rev-7'] as string[],
  rereads: 0,
  // A needs-you answer held back until the test releases it.
  hold: null as null | Promise<void>,
}));
vi.mock('../reviewQueue', () => ({
  reloadReviewQueue: async () => { state.rereads += 1; state.snapshot = [...state.queue]; },
}));
vi.mock('../api', () => ({
  api: {
    listNeedsYou: async (cursor: string | null, limit: number, kinds?: string[], categories?: readonly string[]) => {
      state.calls.push({ cursor, limit, kinds, categories });
      if (state.hold) await state.hold;
      const items: NeedsYouItem[] = [{
        id: 'question:q-1', kind: 'question', sourceId: 'q-1', title: 'Which venue?', detail: null, state: 'stale', at: null,
        workspaceId: null, sessionKey: 's-1', path: null, category: 'question', link: null,
      }];
      return { items, total: 3, nextCursor: null, unavailable: ['Notion sync'] };
    },
  },
}));
import { requestSuggestions } from '../reviewInbox';
import { closeWaitingOnYou, openWaitingOnYou, refreshWaitingOnYou, waitingBadge, waitingFilter, waitingList, waitingOpen, waitingReviews } from './waitingOnYou';
import { REVIEW_VIEW_KINDS, reviewCardView, waitingEmptyState } from '../waitingOnYou';

/** Decisions', Results' and Failures' own queries, in load order. */
const VIEW_QUERIES = [REVIEW_VIEW_KINDS.decisions, REVIEW_VIEW_KINDS.results, REVIEW_VIEW_KINDS.failures];

describe('the waiting-on-you surface state', () => {
  beforeEach(() => { state.calls.length = 0; state.rereads = 0; state.hold = null; closeWaitingOnYou(); });

  it("opens filtered to a page's review kinds from the old review-panel deep link", async () => {
    requestSuggestions('budgets');
    expect(waitingOpen()).toBe(true);
    expect(waitingFilter()).toBe('budgets');
    // It reloads everything (the bell's count), each narrower view's own
    // kinds (P0-09; Review is on screen here) and the review items the
    // filter shows.
    expect(state.calls.map(c => c.kinds)).toEqual([undefined, ...VIEW_QUERIES, ['review']]);
    // The query filters to that page's review kinds before paging, so a page
    // of other review items can't hide them (or read as "none waiting").
    expect(state.calls.map(c => c.categories)).toEqual([undefined, undefined, undefined, undefined, ['budget_pause']]);
    requestSuggestions('tasks');
    expect(state.calls.at(-1)?.categories).toEqual(['heartbeat_task', 'meeting_task', 'agent_task', 'commitment', 'companion_conflict', 'companion_old_op']);
  });

  it('opens unfiltered from the bell or Home, and counts what can be acted on', async () => {
    openWaitingOnYou();
    expect(waitingFilter()).toBeNull();
    expect(state.calls.map(c => c.kinds)).toEqual([undefined, ...VIEW_QUERIES]);
    await vi.waitFor(() => expect(waitingList.items()).toHaveLength(1));
    expect(waitingList.unavailable()).toEqual(['Notion sync']);
    // Three waiting, one of them a question that can no longer be answered.
    expect(waitingBadge()).toBe(2);
  });

  it('re-reads the review queue whenever it opens or refreshes, so a card the query lists opens', async () => {
    // A chat turn just proposed a learning: the query lists review rev-7,
    // but nothing bumped the review queue snapshot, which predates it.
    state.snapshot = [];
    const view = (reread: boolean) => reviewCardView({ found: state.snapshot.includes('rev-7'), loading: false, failed: false, reread });
    openWaitingOnYou();
    expect(state.rereads).toBe(1);
    await vi.waitFor(() => expect(state.snapshot).toEqual(['rev-7']));
    expect(view(false)).toBe('card');
    refreshWaitingOnYou(true);
    expect(state.rereads).toBe(2);
  });

  it("starts a page's filtered list over, unloaded, when another page's link opens it", async () => {
    requestSuggestions('budgets');
    await vi.waitFor(() => expect(waitingReviews.loaded()).toBe(true));
    expect(waitingReviews.items()).toHaveLength(1);
    let release!: () => void;
    state.hold = new Promise(resolve => { release = resolve; });
    requestSuggestions('tasks');
    // Until the tasks answer lands: no budgets rows, and not "none waiting".
    expect(waitingReviews.items()).toEqual([]);
    expect(waitingReviews.loaded()).toBe(false);
    expect(waitingEmptyState({ shown: 0, more: waitingReviews.next() != null, loaded: waitingReviews.loaded(), failed: waitingReviews.failed() })).toBe('loading');
    release();
    await vi.waitFor(() => expect(waitingReviews.loaded()).toBe(true));
    // The same page again keeps its list while it reloads.
    requestSuggestions('tasks');
    expect(waitingReviews.loaded()).toBe(true);
  });
});
