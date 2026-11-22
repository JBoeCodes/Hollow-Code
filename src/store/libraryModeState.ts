// Library's view signal, on its own (S4, 2026-11-03) so store/runtime.ts can
// open Library → Conversations for the old `chat` page id without importing
// libraryMode.ts, which imports the workspace store (and runtime itself).
// libraryMode.ts re-exports all of it; import it from there elsewhere.
import { createSignal } from 'solid-js';
import { currentPage } from '../navigation';
import type { PageId } from './runtime';

export type LibraryMode = 'all' | 'notes' | 'results' | 'recordings' | 'connected' | 'conversations' | 'learnings' | 'skills' | 'history';

/** The views remembered for the next launch (savedMode reads them back). */
export const REMEMBERED_LIBRARY_MODES: readonly LibraryMode[] = ['all', 'learnings', 'skills', 'history'];

/** Views opened from All rather than the tab strip (overnight 11-12 L4):
 *  Notes (Browse folders), Results (Outputs → Grid) and Recordings
 *  (Meetings → Recordings). The strip shows All selected while one is open,
 *  and remembering one remembers All. */
export const LIBRARY_DETAIL_MODES: readonly LibraryMode[] = ['notes', 'results', 'recordings'];
export const isLibraryDetail = (m: LibraryMode) => LIBRARY_DETAIL_MODES.includes(m);

const MODE_KEY = 'hollow.library.mode';
/** The view Library opens on at launch: a remembered one, else All (one
 *  searchable list, overnight 11-12 L3; it was Results). A remembered Notes,
 *  Results or Recordings (from before L4) opens All too. */
export const savedMode = (): LibraryMode => {
  try { const v = localStorage.getItem(MODE_KEY); return REMEMBERED_LIBRARY_MODES.includes(v as LibraryMode) ? v as LibraryMode : 'all'; } catch { return 'all'; }
};

const [mode, setMode] = createSignal<LibraryMode>(savedMode());
export const libraryMode = mode;

/** Switch Library's view. A person's pick of All, Learnings, Skills or
 *  History is remembered for the next launch, and Notes, Results or
 *  Recordings remember All, whose views they are; Connected projects is not
 *  (it needs a project chosen), nor Conversations (Library would reopen on a
 *  chat). */
export function setLibraryMode(next: LibraryMode, remember = false) {
  setMode(next);
  const kept: LibraryMode = isLibraryDetail(next) ? 'all' : next;
  if (remember && REMEMBERED_LIBRARY_MODES.includes(kept)) { try { localStorage.setItem(MODE_KEY, kept); } catch { /* session only */ } }
}

// The view Conversations was opened from, which any other way into Library
// returns to: the rail's Library, ⌘3 or Back still open Library as before.
let before: LibraryMode = mode();

/** Library's view for a page change to `page` (before currentPage maps it):
 *  `chat`, the old Conversation history page, shows Conversations; a Back or
 *  Forward step that kept its tab (`view`, navHistory.ts) shows that tab; any
 *  other way into Library leaves Conversations for the view it came from. */
export function placeLibraryView(page: PageId, view?: LibraryMode) {
  if (page === 'chat') {
    if (mode() !== 'conversations') before = mode();
    setMode('conversations');
  } else if (currentPage(page) !== 'vault') {
    return;
  } else if (view && view !== 'conversations') {
    setMode(view);
  } else if (mode() === 'conversations') {
    setMode(before);
  }
}
