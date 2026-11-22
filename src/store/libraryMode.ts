// Library's views (consolidation lane B, 2026-10-13): All (overnight 11-12
// L3: one searchable list), Notes, Results and
// Recordings (overnight 11-12 L4: views opened from All, off the strip),
// Connected projects, Conversations (S4, 2026-11-03: the old
// Conversation history page), Skills, History (overnight 11-12 L2: Settings'
// skills catalog and History & recovery) and Learnings, and inside Results the All/Files/Canvases
// filter. The canvas gallery used to be a full-screen overlay (`g x`, the
// palette); it is now Library → Results → Canvases, and those entry points
// route here (store/canvasGallery.ts), as do workspace Results' Canvases links.
//
// Kept free of component imports so the palette, `g` keys and a workspace's
// Results can open the view without pulling in a lazy chunk.
import { batch, createSignal } from 'solid-js';
import { activePage, setActivePage, setOpenVaultNoteRequest, requestOpenSkill } from './runtime';
import { goToPage } from './workspaces';
import { flushViewTransition } from '../viewTransitions';
import { libraryMode as mode, setLibraryMode, type LibraryMode } from './libraryModeState';
import { guardNavigation } from './navigationGuard';
import { requestGalleryCanvas, type GalleryCanvasRequest } from '../canvas/galleryRequest';
import type { LibraryListFilter } from '../libraryListModel';
// The view signal lives in libraryModeState.ts, which store/runtime.ts reads.
export { libraryMode, setLibraryMode, isLibraryDetail, type LibraryMode } from './libraryModeState';

// What a view change clears on the Library page while it is mounted (its
// open file, file history and New note form): Library binds it, so the
// palette, links and its own tabs all start a view clean (P1-19).
let clearView: () => void = () => {};
/** Library binds what a view change clears; returns the unbind. */
export function bindLibraryViewReset(fn: () => void) {
  clearView = fn;
  return () => { if (clearView === fn) clearView = () => {}; };
}

/** Change Library's view the one way a tab click does (#1311 re-review):
 *  a page jump still waiting for its view transition (Back, ⌘1) is applied
 *  first, so it can never land after this one and hide the view; the same
 *  unsaved-draft guard; the page's open file, history and New note form
 *  cleared; and leaving Conversations, a Back step of its own (`chat`), is a
 *  step too. `page` `chat` shows Conversations. */
function showLibraryView(commit: () => void, page: 'vault' | 'chat' = 'vault', onOpen?: () => void) {
  flushViewTransition();
  const apply = () => { clearView(); commit(); onOpen?.(); };
  if (activePage() !== 'vault') return goToPage(page, { onNavigate: apply });
  return guardNavigation(() => {
    const leaving = mode() === 'conversations';
    apply();
    if (page === 'chat') setActivePage('chat');
    else if (leaving && mode() !== 'conversations') setActivePage('vault');
  });
}

/** Show Library's tab `mode` from anywhere (overnight 11-12 L2): the palette's
 *  Library: Skills and Library: History, Learnings' links, the Settings
 *  links and deep links that point at a Library tab, and Library's own tab
 *  strip, so all of them behave alike. It remembers the tab as a click does.
 *  Conversations opens through its own page id, `chat`. */
export function openLibraryTab(next: LibraryMode, onOpen?: () => void) {
  if (next === 'conversations') return showLibraryView(() => {}, 'chat', onOpen);
  return showLibraryView(() => setLibraryMode(next, true), 'vault', onOpen);
}

/** Open one vault note in Library → Notes' reader and editor (overnight 11-12
 *  L3): All's Open in Notes, so editing keeps a visible path. All is
 *  remembered, as Notes is its view (L4). */
export function openLibraryNote(path: string) {
  return showLibraryView(() => batch(() => { setLibraryMode('notes', true); setOpenVaultNoteRequest(path); }));
}

/** Open Library, Skills on one skill's detail (the palette's skill rows). The
 *  request is set only once the view change is accepted: a refused one (an
 *  unsaved draft, answered Keep editing) must not leave it for a later,
 *  plain Library, Skills to pick up. */
export function openLibrarySkill(name: string) {
  return showLibraryView(() => { setLibraryMode('skills', true); requestOpenSkill(name); });
}

/** All's filter (overnight 11-12 L4), kept here so Back to All from a view
 *  it opened (Grid from Outputs, Recordings from Meetings) returns to it. */
const [allFilter, setAllFilterSignal] = createSignal<LibraryListFilter>('recent');
export const libraryAllFilter = allFilter;
export const setLibraryAllFilter = (next: LibraryListFilter) => setAllFilterSignal(next);

/** The learning review home obeys the same unsaved-draft guard as Results. */
export const openLibraryLearnings = () => openLibraryTab('learnings');

/** What the single Library Results list includes. */
export type ResultsType = 'all' | 'files' | 'canvases';

const [type, setType] = createSignal<ResultsType>('all');
export const resultsType = type;
export const setResultsType = (next: ResultsType) => setType(next);

const [canvasWorkspace, setCanvasWorkspace] = createSignal('');
const [canvasRequest, setCanvasRequest] = createSignal(0);
/** The workspace filter Canvases opens with (`''`: all workspaces). */
export const canvasStartWorkspace = canvasWorkspace;
/** Bumps on every open, so a link opened while Canvases is shown still
 *  applies its workspace filter. */
export const canvasOpenRequest = canvasRequest;

/** Whether Library → Results → Canvases is on screen. */
export const libraryCanvasesShown = () => activePage() === 'vault' && mode() === 'results' && type() === 'canvases';

/** Show Library → Results → Canvases, optionally filtered to one workspace. */
export function openLibraryCanvases(workspaceId = '', request: GalleryCanvasRequest | null = null) {
  return showLibraryView(() => batch(() => {
    // Publish one accepted destination. Refused or superseded requests never
    // change the view or leave a Compare request for a later ordinary visit.
    requestGalleryCanvas(request);
    setCanvasWorkspace(workspaceId);
    setLibraryMode('results');
    setType('canvases');
    setCanvasRequest(n => n + 1);
  }));
}

/** Clear the canvas type filter, staying in Library → Results. */
export function closeLibraryCanvases() { if (type() === 'canvases') setType('all'); }

/** Leaving Library: Results starts with every type next time, and All
 *  with Recent. */
export function resetResultsType() { setType('all'); setAllFilterSignal('recent'); }
