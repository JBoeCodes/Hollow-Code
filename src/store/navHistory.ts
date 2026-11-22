// Back/Forward history (P1-42): the stack runtime.ts steps through, its pure
// moves, and the hook through which store/workspaces.ts (which runtime.ts
// can't import) names the workspace on screen and reopens one.
import { createSignal } from 'solid-js';
import type { PageId } from './runtime';
import type { LibraryMode } from './libraryModeState';

/** One step. `page` is the route; `notePath` the vault note open there (so
 *  Back reopens it); on Your work's page (`today`), `workspaceId` and
 *  `sessionKey` name the workspace and conversation on screen: no key is a
 *  new conversation not sent yet, and no workspace is Your work's overview.
 *  `libraryView`: on a Library step (`vault`), the tab it showed when it was
 *  left, so Back reopens that tab (S4; Conversations is its own `chat` step). */
export interface NavEntry { page: PageId; notePath?: string; workspaceId?: string; sessionKey?: string; libraryView?: LibraryMode }
export interface NavStack { entries: NavEntry[]; cursor: number }

const NAV_LIMIT = 50;

/** `s` after visiting `e`: unchanged when `e` is the entry on screen (only
 *  identical entries coalesce), else `e` pushed in place of any forward
 *  entries, bounded to the newest 50. */
export function visit(s: NavStack, e: NavEntry): NavStack {
  const cur = s.entries[s.cursor];
  if (cur && cur.page === e.page && cur.workspaceId === e.workspaceId && cur.sessionKey === e.sessionKey && cur.notePath === e.notePath) return s;
  const entries = [...s.entries.slice(0, s.cursor + 1), e].slice(-NAV_LIMIT);
  return { entries, cursor: entries.length - 1 };
}

/** One step back (`-1`) or forward (`1`), or null at that end. */
export function step(s: NavStack, by: -1 | 1): { stack: NavStack; entry: NavEntry } | null {
  const entry = s.entries[s.cursor + by];
  return entry ? { stack: { entries: s.entries, cursor: s.cursor + by }, entry } : null;
}

/** `s` with the entry on screen changed by `edit`, when `edit` returns one. */
export function editCurrent(s: NavStack, edit: (e: NavEntry) => NavEntry | null): NavStack {
  const next = s.entries[s.cursor] && edit(s.entries[s.cursor]);
  return next ? { entries: s.entries.map((e, i) => i === s.cursor ? next : e), cursor: s.cursor } : s;
}

/** `s` with the Library step on screen marked as showing tab `view`, as it
 *  is left (S4): unchanged for any other step or for Conversations. */
export function keepLibraryView(s: NavStack, view: LibraryMode): NavStack {
  return view === 'conversations' ? s : editCurrent(s, e => e.page === 'vault' && e.libraryView !== view ? { ...e, libraryView: view } : null);
}

export const [nav, setNav] = createSignal<NavStack>({ entries: [{ page: 'today' }], cursor: 0 });

/** A new conversation's first send gave it key `sessionKey`: the newest entry
 *  of that workspace up to the one on screen learns it, if it showed the
 *  conversation unsent, so Back returns to it (the person may have left for
 *  another page before the key came). Called on every refresh too, so an
 *  entry that has a key, or a key some entry already has, stops it. */
export function learnConversation(workspaceId: string, sessionKey: string) {
  setNav(s => {
    if (s.entries.some(e => e.sessionKey === sessionKey)) return s;
    for (let i = s.cursor; i >= 0; i--) {
      const e = s.entries[i];
      if (e.page !== 'today' || e.workspaceId !== workspaceId) continue;
      return e.sessionKey ? s : { entries: s.entries.map((x, j) => j === i ? { ...e, sessionKey } : x), cursor: s.cursor };
    }
    return s;
  });
}

/** Launch reopened workspace `id`'s last conversation, a step nobody took: it
 *  replaces the starting overview entry, unless the person has moved on. */
export function launchLanded(id: string) {
  setNav(s => s.entries.length === 2 && !s.entries[0].workspaceId && s.entries[0].page === 'today' && s.entries[1].workspaceId === id ? { entries: [s.entries[1]], cursor: 0 } : s);
}

/** Bound by store/workspaces.ts at load. `current`: the workspace and
 *  conversation on screen; `restore`: called with every entry Back or
 *  Forward lands on, after its page is shown, to reopen a workspace entry
 *  (and to drop an open still loading for another). */
let hook: { current(): Pick<NavEntry, 'workspaceId' | 'sessionKey'>; restore(e: NavEntry): void } = { current: () => ({}), restore: () => {} };
export function bindWorkspaceNav(next: typeof hook) { hook = next; }
export const workspaceNav = () => hook;

/** While `fn` runs, page changes record no step: replaying an entry, or an
 *  open's first page change before its workspace has loaded. */
let quiet = 0;
export function quietly<T>(fn: () => T): T {
  quiet++;
  try { return fn(); } finally { quiet--; }
}
export const navQuiet = () => quiet > 0;
