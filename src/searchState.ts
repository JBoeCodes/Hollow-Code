/**
 * Asynchronous search states (audit H09). A request that has not answered yet
 * is `pending`, never an empty result: the UI keeps the last results (dimmed)
 * or says it is searching, instead of flashing "Nothing found". `unavailable`
 * (the feature/index is off or not ready) and `error` (the request failed)
 * are distinct from a completed search that found nothing. Which request may
 * answer is `latestRequest.ts` (moved from here in X21a).
 */
export type SearchState<T> =
  | { status: 'idle' }
  | { status: 'pending'; query: string; previous: T[] | null }
  | { status: 'done'; query: string; results: T[] }
  | { status: 'unavailable'; query: string; reason: string }
  | { status: 'error'; query: string; message: string };

/** Results to show now: completed results, or the previous ones while a
 *  newer request is pending. Null when there is nothing to show yet. */
export function visibleResults<T>(s: SearchState<T>): T[] | null {
  if (s.status === 'done') return s.results;
  if (s.status === 'pending') return s.previous;
  return null;
}

/** Start a request: keep whatever was visible so the list does not blank. */
export function beginSearch<T>(prev: SearchState<T>, query: string): SearchState<T> {
  return { status: 'pending', query, previous: visibleResults(prev) };
}

/** Headline for an empty or not-yet-complete result list. */
export function searchEmptyText(s: SearchState<unknown>, mode: 'name' | 'meaning'): { title: string; detail: string; role: 'status' | 'alert' | undefined } | null {
  switch (s.status) {
    case 'pending': return s.previous?.length ? null : { title: mode === 'meaning' ? 'Searching by meaning…' : 'Searching…', detail: 'Results appear as soon as the search finishes.', role: 'status' };
    case 'done': return s.results.length ? null : mode === 'meaning'
      ? { title: 'No matching files.', detail: 'Nothing in the index is close to that description. Try other words, or search by name.', role: 'status' }
      : { title: 'Nothing by that name.', detail: 'Try a different phrase, or look across all your files.', role: 'status' };
    case 'unavailable': return { title: 'Search by meaning isn’t available yet.', detail: s.reason, role: 'status' };
    case 'error': return { title: 'Search didn’t finish.', detail: s.message, role: 'alert' };
    default: return null;
  }
}
