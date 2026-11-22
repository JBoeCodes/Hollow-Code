// Where focus goes back to when Search chats closes (overnight 11-12 re-review of #1310).
// The place Search was asked from is kept while it is open: a repeat request (the shortcut
// pressed again with the search showing) must not replace it with the search box. On close
// the place is used if it is still on the page, else the top bar's search icon; it is
// forgotten either way so the next open starts clean.
export type FocusTarget = { isConnected: boolean };

export function createSearchFocusReturn<T extends FocusTarget>() {
  let returnTo: T | null = null;
  return {
    /** `searchShown`: the search is already open, so a repeat keeps the first place. */
    open(from: T | null, searchShown: boolean) { if (!searchShown) returnTo = from; },
    /** The element to focus now (`fallback` when the original place left the page). */
    close(fallback: () => T | null): T | null {
      const back = returnTo?.isConnected ? returnTo : fallback();
      returnTo = null;
      return back;
    },
    /** Drop the remembered place (hiding the sidebar ends the search; #1332 review). */
    forget() { returnTo = null; },
  };
}
