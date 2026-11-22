import { createSignal } from 'solid-js';
import type { NeedsYouItem, NeedsYouPage } from './workspaceTypes';
import { latestRequest } from './latestRequest';

/**
 * Exact-id helpers for the "needs you" query (merged plan slice 4, B·F15).
 * Pure, unit-tested in needsYou.test.ts.
 */

/** The result Results shows. The one asked for by path when it is there.
 *  When a specific result was opened (`exact`) and it is gone, none: the
 *  view says so rather than showing a different result in its place.
 *  Otherwise (a remembered selection that went away, or none) the newest. */
export function shownResultPath(all: readonly { path: string }[], want: string | undefined, exact: boolean): string {
  if (want && all.some(r => r.path === want)) return want;
  if (exact && want) return '';
  return all[0]?.path ?? '';
}

/** The most `list_needs_you` returns in one page (Rust `PAGE_MAX`). */
const NEEDS_PAGE_MAX = 200;

/** One paged list from the "needs you" query on screen (the "waiting on
 *  you" surface, `store/waitingOnYou.ts`): never capped, "Show more" pages on.
 *  `load()` reloads as many rows as are shown (at least a page, at most one
 *  query page); `load(true)` appends the page after the last row shown. A
 *  response overtaken by a newer load is dropped; a failed load keeps the
 *  last list and sets `failed` until a load succeeds. `unavailable` names the
 *  sources the last answer couldn't read. `loaded` turns true with the first
 *  answer (and back to false on `reset()`): until then an empty list means
 *  "not known yet", never "nothing waiting". `reset()` drops the list and
 *  any load in flight, for a list that now asks a different question.
 *  Show more never overtakes a reload (S01d): while one is in flight
 *  (`reloading()`) it does nothing, since its page would follow a list the
 *  reload is about to replace; it takes the current ticket rather than a new
 *  one, so a reload started after it still drops its page. A second Show more
 *  while one is in flight does nothing either (same cursor, same page). */
export function createNeedsYouList(fetchPage: (cursor: string | null, limit: number) => Promise<NeedsYouPage>, pageSize: number) {
  const [items, setItems] = createSignal<NeedsYouItem[]>([]);
  const [total, setTotal] = createSignal(0);
  const [next, setNext] = createSignal<string | null>(null);
  const [unavailable, setUnavailable] = createSignal<string[]>([]);
  const [failed, setFailed] = createSignal(false);
  const [loaded, setLoaded] = createSignal(false);
  const loads = latestRequest();
  // Full loads in flight: a count, since a reload can overtake a reload and
  // the first to settle must not clear the flag for the one still out.
  const [fullLoads, setFullLoads] = createSignal(0);
  const reloading = () => fullLoads() > 0;
  // The Show more in flight, by its ticket: one a reload or reset dropped
  // no longer blocks the next click.
  let paging: number | null = null;
  const load = async (more = false) => {
    if (more && (next() == null || reloading() || (paging != null && loads.isLatest(paging)))) return;
    const mine = more ? loads.peek() : loads.start();
    const size = more ? pageSize : Math.min(NEEDS_PAGE_MAX, Math.max(pageSize, items().length));
    if (more) paging = mine; else setFullLoads(n => n + 1);
    try {
      const page = await fetchPage(more ? next() : null, size);
      if (!loads.isLatest(mine)) return;
      setItems(prev => more ? [...prev, ...page.items] : page.items);
      setTotal(page.total); setNext(page.nextCursor); setUnavailable(page.unavailable); setFailed(false); setLoaded(true);
    } catch {
      // Keep the last list, but never let it pass for a fresh one.
      if (loads.isLatest(mine)) setFailed(true);
    } finally {
      if (!more) setFullLoads(n => n - 1);
      else if (paging === mine) paging = null;
    }
  };
  const reset = () => {
    loads.cancel();
    setItems([]); setTotal(0); setNext(null); setUnavailable([]); setFailed(false); setLoaded(false);
  };
  return { items, total, next, unavailable, failed, loaded, reloading, load, reset };
}
