// The Preview pane's web mode (2026-09-30 as the side panel's browser; run 15
// P1 made it the app-wide Preview pane's): tabs of real web pages beside any
// page, the way ChatGPT's desktop app shows a site it built. Each tab is a
// native child webview (src-tauri/src/browser_pane.rs) that BrowserPane.tsx
// places over its page area; this module owns the tab list. The pane is one
// for the whole app, so its tabs are too (next-level P15 kept one list per
// workspace while the browser lived in each workspace's side panel).

import { createSignal } from 'solid-js';
import { listen } from '../nativeEvents';
import { api, type BrowserBounds } from '../api';
import { isBrowserMockMode } from '../mockMode';
import { addressFile, isWebUrl, normalizeAddress, tabLabel } from '../browserUrl';
import { errMsg, openExternal } from '../utils';
import { activePage, chatCwd, flashFooter, openFileInSidePane, openInBrowser, showConfirm, confirmDialog, vaultRoot } from './runtime';
import { workspaceDetail } from './workspaces';
import { openPreview } from './preview';
import { linkTarget } from './viewPrefs';
import { CHAT_SCOPE, NO_ANSWER_MS, followsPageUrl, lruVictim, nextActive, noAnswerMessage, stoppedTab, tabAfterEvent, tabsInScope, unanswered } from '../browserReview';
import { FrameOpenGate, type LinkKind } from '../linkGuard';
import { canvasFilePath } from '../canvas/frameFile';

export interface BrowserTab {
  id: string;
  url: string;
  title: string;
  loading: boolean;
  /** The native webview exists (browser_open resolved); only then placed. */
  ready: boolean;
  /** The page answered (a load event arrived) at least once; until then the
   *  native page stays hidden behind "Loading…" (run 15 P2). */
  committed: boolean;
  /** When the current load was asked for, and when the page last answered
   *  (ms): a load with no answer for NO_ANSWER_MS fails with Try again. */
  openedAt: number;
  answeredAt: number;
  /** When the tab was last on show: at the cap the oldest one closes. */
  usedAt: number;
  /** The workspace it was opened in, or `chat` (next-level P15). */
  scope: string;
  /** Why the page could not be opened (the tab stays, with Try again). */
  error?: string | null;
  /** The error is the time limit's, which a late answer clears. */
  timedOut?: boolean;
  /** The newest load asked of this tab (`begin`); only its outcome counts. */
  attempt: number;
  /** That load's command hasn't settled: the old page's events don't count. */
  opening: boolean;
  /** The native load the tab's state follows (browser_pane.rs `NAVIGATED`):
   *  only events sent by it apply. */
  gen: number;
  /** The newest event of a newer load that beat its command's reply (a fast
   *  page): applied when that command settles with the same `gen`. */
  early?: { gen: number; kind: 'loading' | 'loaded'; url?: string; at: number } | null;
  /** The load a Stop ended: its start event, already on its way when Stop
   *  came, must not bring the spinner back (no `loaded` follows a stopped
   *  load). Cleared by the next answer or load (run 15 P2 second pass). */
  stoppedGen?: number | null;
}

/** The tab list's scope: one, app-wide, like the Preview pane (run 15 P1). */
export function browserScope(): string {
  return CHAT_SCOPE;
}

/** The workspace the web mode works for now: the open workspace on its page,
 *  else `chat`. Page reviews, Save to workspace, a workspace's running
 *  services and annotation drafts belong to it (drafts saved before run 15
 *  P1 carry the same ids), while the tabs themselves stay one app-wide list. */
export function browserWorkspaceScope(): string {
  const open = workspaceDetail();
  return activePage() === 'today' && open ? open.workspace.id : CHAT_SCOPE;
}

/** Matches browser_pane.rs MAX_TABS. */
export const MAX_BROWSER_TABS = 8;
/** Every tab, every scope (the native webviews that exist). */
export const [allBrowserTabs, setBrowserTabs] = createSignal<BrowserTab[]>([]);
/** The tabs of the current scope, in order. */
export const browserTabs = () => tabsInScope(allBrowserTabs(), browserScope());
const [activeByScope, setActiveByScope] = createSignal<Record<string, string | null>>({});
/** The tab on show in this scope; null is the new-tab page (address field,
 *  running services). A tab of another scope is never "active" here. */
export function activeBrowserTab(): string | null {
  const id = activeByScope()[browserScope()] ?? null;
  return id && browserTabs().some(t => t.id === id) ? id : null;
}
export function setActiveBrowserTab(id: string | null) {
  const scope = browserScope();
  setActiveByScope(m => ({ ...m, [scope]: id }));
  if (id) patch(id, { usedAt: Date.now() });
}
/** Ask the address field to take focus (⌘L, a new tab). */
export const [browserFocusNonce, setBrowserFocusNonce] = createSignal(0);

let seq = 0;
const newTabId = () => `t${Date.now().toString(36)}${(seq++).toString(36)}`;
const patch = (id: string, change: Partial<BrowserTab>) =>
  setBrowserTabs(list => list.map(t => (t.id === id ? { ...t, ...change } : t)));

/** The page area's last measured place, so a tab created while the pane is
 *  hidden starts where it will show. BrowserPane keeps it current. */
let lastBounds: BrowserBounds = { x: 0, y: 0, width: 1, height: 1 };
export function noteBrowserBounds(b: BrowserBounds) { lastBounds = b; }

let listening = false;
function listenOnce() {
  if (listening || isBrowserMockMode()) return;
  listening = true;
  type Ev = { tabId: string; kind: string; url?: string; title?: string; path?: string; ok?: boolean; gen?: number };
  listen<Ev>('browser-event', ({ payload: e }) => {
    switch (e.kind) {
      case 'loading': case 'loaded': {
        const t = allBrowserTabs().find(x => x.id === e.tabId);
        if (!t) break;
        const gen = e.gen ?? 0;
        // A stopped load's start, sent before Stop reached the page.
        if (e.kind === 'loading' && t.stoppedGen === gen && !t.opening) break;
        // A newer load's own event that beat its command's reply waits for it.
        if (gen > t.gen && t.opening) patch(e.tabId, { early: { gen, kind: e.kind, url: e.url, at: Date.now() } });
        // With no command pending (a Stop dropped its reply), a newer load is
        // what the webview shows: follow it from now on, but as no tracked
        // load, so it never restarts the spinner (a stopped page sends no
        // `loaded`, and no time limit runs for it).
        else if (gen > t.gen) patch(e.tabId, { gen, ...tabAfterEvent(t, e.kind, e.url, Date.now()), loading: false, stoppedGen: null });
        // The page this tab follows; an older load's, or the old page's while
        // a newer load starts, count for nothing. (Limit: a callback of the
        // old page already queued when the new load starts carries the new
        // gen; wry discards WebKit's navigation identity, and WebKit cancels
        // the old load as the new one starts, so only that one can slip.)
        else if (gen === t.gen && !t.opening) patch(e.tabId, { ...tabAfterEvent(t, e.kind, e.url, Date.now()), stoppedGen: null });
        break;
      }
      case 'title': patch(e.tabId, { title: e.title ?? '' }); break;
      case 'new-tab': if (e.url) void openBrowserUrl(e.url, { newTab: true }); break;
      case 'download':
        if (e.ok === true) flashFooter('Download finished: it is in your Downloads folder.');
        else if (e.ok === false) flashFooter('× The download failed.');
        else if (e.path) flashFooter(`Downloading ${e.path.split('/').pop()}…`);
        break;
    }
  }).catch(() => { listening = false; });
}

/** Every load a tab is asked for (open, Try again, Reload) and every Stop
 *  is a new attempt: only the newest attempt's outcome touches the tab, so a
 *  slow refusal or a late success of an older one never overwrites it
 *  (run 15 P2 review; browser_pane.rs keeps the same rule natively). */
let attempts = 0;
const nextAttempt = () => ++attempts;
const tabOf = (id: string) => allBrowserTabs().find(x => x.id === id);

/** A load of tab `id` starts: until the command settles, `opening` makes
 *  the old page's late events count for nothing (`tabAfterEvent`). */
function begin(id: string, change: Partial<BrowserTab> = {}): number {
  const attempt = nextAttempt();
  patch(id, { ...change, attempt, opening: true, openedAt: Date.now(), loading: true, error: null, timedOut: false, stoppedGen: null });
  return attempt;
}

/** The command for `attempt` settled (`error` when it refused). On success
 *  the webview exists; a page that then sends no load event in NO_ANSWER_MS
 *  fails with Try again (WebKit reports no failed load). The mock has no
 *  load events, so its pages answer at once. */
function settle(id: string, attempt: number, failed: boolean, outcome?: unknown) {
  const t = tabOf(id);
  // Closed while it opened: its webview must not outlive it.
  if (!t) { if (!failed) void api.browserClose(id).catch(() => { /* never made */ }); return; }
  if (t.attempt !== attempt) return;
  if (failed) { patch(id, { opening: false, loading: false, error: errMsg(outcome), timedOut: false }); return; }
  if (isBrowserMockMode()) { patch(id, { opening: false, ready: true, ...tabAfterEvent({}, 'loaded', undefined, Date.now()) }); return; }
  // From now on the tab follows this load's events, and one that came early counts.
  const gen = typeof outcome === 'number' ? outcome : t.gen;
  patch(id, { opening: false, ready: true, gen, early: null });
  const early = t.early;
  if (early && early.gen === gen) patch(id, tabAfterEvent({ ...t, opening: false }, early.kind, early.url, early.at));
  window.setTimeout(() => {
    const now = tabOf(id);
    if (now?.attempt === attempt && unanswered(now, Date.now(), NO_ANSWER_MS)) patch(id, { loading: false, error: noAnswerMessage(now.url), timedOut: true });
  }, NO_ANSWER_MS);
}

/** Run browser_open for a load begun as `attempt`. */
async function load(id: string, url: string, attempt: number, first?: Promise<void> | null) {
  try {
    // A tab closed to make room goes first, so Rust's own cap never refuses.
    await first;
    // A newer load, a Stop or a close came meanwhile: this load is over.
    if (tabOf(id)?.attempt !== attempt) return;
    settle(id, attempt, false, await api.browserOpen(id, url, lastBounds));
  } catch (e) {
    // The tab stays with the reason and Try again (next-level P15), instead
    // of vanishing behind a toast.
    settle(id, attempt, true, e);
  }
}

/** Load what was typed or clicked: in the tab on show, or a new tab. A web
 *  link already open in a tab switches to that tab instead. A file address
 *  or path shows that file in the Preview pane (a tab never loads files). */
export async function openBrowserUrl(input: string, opts: { newTab?: boolean } = {}): Promise<void> {
  const file = addressFile(input);
  if (file) { openPreview({ kind: 'file', path: file }); return; }
  const url = normalizeAddress(input);
  if (!url) { flashFooter('× The browser opens web pages only (http or https).'); return; }
  listenOnce();
  // A tab already showing this address is reused (one app-wide list since run 15 P1).
  const open = opts.newTab ? browserTabs().find(t => t.url === url) : undefined;
  if (open) { setActiveBrowserTab(open.id); if (open.error) void retryBrowserTab(open.id); return; }
  const current = opts.newTab ? null : browserTabs().find(t => t.id === activeBrowserTab());
  if (current) { await load(current.id, url, begin(current.id, { url })); return; }
  // At the cap the tab shown longest ago makes room (run 15 P2): refusing
  // left the click looking ignored. The list changes before any await, so a
  // second open meanwhile counts the new tab and never makes a ninth.
  let closing: Promise<void> | null = null;
  if (allBrowserTabs().length >= MAX_BROWSER_TABS) {
    const gone = allBrowserTabs().find(t => t.id === lruVictim(allBrowserTabs(), activeBrowserTab()));
    if (!gone) { flashFooter(`× Close a browser tab first (at most ${MAX_BROWSER_TABS}).`); return; }
    closing = closeBrowserTab(gone.id);
    flashFooter(`Closed ${tabLabel(gone.url, gone.title)} to make room (at most ${MAX_BROWSER_TABS} tabs).`);
  }
  const id = newTabId();
  const now = Date.now();
  setBrowserTabs(list => [...list, { id, url, title: '', loading: true, ready: false, committed: false, opening: true, attempt: 0, gen: 0, openedAt: now, answeredAt: 0, usedAt: now, scope: browserScope(), error: null }]);
  setActiveBrowserTab(id);
  await load(id, url, begin(id), closing);
}

/** Open a failed tab's address again. */
export async function retryBrowserTab(id: string) {
  const t = tabOf(id);
  if (t) await load(id, t.url, begin(id));
}

export async function closeBrowserTab(id: string) {
  const at = browserTabs().findIndex(t => t.id === id);
  if (at < 0) return;
  const wasActive = activeBrowserTab() === id;
  const rest = allBrowserTabs().filter(t => t.id !== id);
  setBrowserTabs(rest);
  if (wasActive) setActiveBrowserTab(nextActive(rest, browserScope(), at));
  await api.browserClose(id).catch(() => { /* already gone */ });
}

/** The new-tab page, address field focused. */
export function newBrowserTab() {
  setActiveBrowserTab(null);
  setBrowserFocusNonce(n => n + 1);
}

/** Set by BrowserPane while it's mounted: whether its live page is on show
 *  (a ready tab, nothing drawn over it). */
let livePage: (() => boolean) | null = null;
/** Returns the unregister, which clears only this pane's predicate. */
export function registerBrowserPage(on: () => boolean): () => void {
  livePage = on;
  return () => { if (livePage === on) livePage = null; };
}
/** Whether Back and Forward mean the browser page's (P1-47): its live page is
 *  on show and has the keys, with focus in the panel or in the native page
 *  itself (then the app's document has lost focus to it). */
export function browserTakesNav(): boolean {
  if (!livePage?.()) return false;
  if (!document.hasFocus()) return true;
  const el = document.activeElement as Element | null;
  return !!el?.closest?.('.browser-pane');
}

export function browserGo(action: 'back' | 'forward' | 'reload' | 'stop') {
  const id = activeBrowserTab();
  if (!id) return;
  const t = tabOf(id);
  // A failed tab, or one whose first open hasn't made its webview yet, has
  // nothing to reload: open its address again.
  if ((t?.error || !t?.ready) && action === 'reload') { void retryBrowserTab(id); return; }
  // Stop ends the load here too (no event follows a stopped load) and is an
  // attempt of its own, so the load it stopped can't come back.
  if (t && action === 'stop') patch(id, { ...stoppedTab(t), attempt: nextAttempt(), opening: false, timedOut: false, stoppedGen: t.gen });
  // Back and Forward are the person's own navigation (run 15 P2 second
  // pass): a refused address's failure goes, and a pending reload's time
  // limit with it (a page from WebKit's cache may send no event). A tab whose
  // load command is still out stays with that load, and one that never showed
  // a page keeps its failure and Try again (there is nothing to go back to).
  // A Stop's `stoppedGen` stays: with no history, Back changes nothing, and
  // the stopped load's late start must still not bring the spinner back.
  if (t?.ready && t.committed && !t.opening && (action === 'back' || action === 'forward')) {
    patch(id, { attempt: nextAttempt(), loading: false, error: null, timedOut: false });
  }
  // A reload gets the same time limit as an open (Back and Forward don't:
  // a page from WebKit's cache may send no event at all), and a reload of a
  // page on this Mac whose server stopped fails like an open.
  const reload = action === 'reload' && !!t && !isBrowserMockMode() ? begin(id) : 0;
  void api.browserGo(id, action).then(
    gen => { if (reload) settle(id, reload, false, gen); },
    e => { if (reload) settle(id, reload, true, e); else flashFooter(`× ${errMsg(e)}`); },
  );
}

/** Follow a single-page app's address changes (no page load fires), never
 *  over an address still loading or one that failed (`followsPageUrl`). */
export async function syncBrowserUrl(id: string) {
  try {
    const url = await api.browserUrl(id);
    const t = allBrowserTabs().find(x => x.id === id);
    if (url && t && t.url !== url && followsPageUrl(t)) patch(id, { url });
  } catch { /* closed meanwhile */ }
}

/** A web link clicked in Hollow (a reply, a service's port, a pull request):
 *  in the Preview pane's web mode, beside whatever page is showing (the pane
 *  loads it, store/preview.ts); in the default browser when ⌘ / Shift is
 *  held or it isn't http(s). */
export function openWebLink(url: string, e?: MouseEvent | KeyboardEvent) {
  // Settings, Browser: the default can be the Mac's browser; ⌘ or Shift reverses either way.
  const reversed = !!(e && (e.metaKey || e.shiftKey));
  const external = !isWebUrl(url) || (linkTarget() === 'browser' ? !reversed : reversed);
  if (external) { openInBrowser(url).catch(() => openExternal(url)); return; }
  openPreview({ kind: 'url', url });
}

/** A web or mail link the main window took over (`linkGuard.ts`): web links
 *  as `openWebLink`, mail links in the default mail app. */
export function followLink(href: string, kind: LinkKind, e?: MouseEvent) {
  if (kind === 'web') openWebLink(href, e);
  else openInBrowser(href).catch(() => openExternal(href));
}

const frameOpens = new FrameOpenGate();

/** Focus/activation limits prompts, not authorization: artifact scripts can
 * forge link messages after a click. Scripted documents must get an explicit
 * host confirmation of the exact destination. Script-free previews keep their
 * ordinary link behavior. Callers pass an `open` message `acceptFrameMessage`
 * (canvasBridge.ts) accepted from that frame, and the frame's mode. */
export function openFrameLink(message: { href: string; link: LinkKind }, frame: HTMLIFrameElement, scripted = true): boolean {
  // Capture the destination now: only what the person sees may open.
  const link = { href: message.href, kind: message.link };
  const focused = document.activeElement === frame;
  const active = (navigator as { userActivation?: { isActive: boolean } }).userActivation?.isActive;
  if (!frame.isConnected || confirmDialog() || !frameOpens.admit(Date.now(), focused, active)) return true;
  if (!scripted) { followLink(link.href, link.kind); return true; }
  const src = frame.src;
  const srcdoc = frame.srcdoc;
  void showConfirm({
    title: 'Open this canvas link?',
    body: `This document runs scripts and wants to open:\n${link.href}`,
    confirmLabel: 'Open', cancelLabel: 'Cancel', danger: true,
  }).then(ok => {
    if (ok && frame.isConnected && frame.src === src && frame.srcdoc === srcdoc) {
      followLink(link.href, link.kind);
    }
  }).catch(() => { /* A failed confirmation opens nothing. */ });
  return true;
}

const frameFiles = new FrameOpenGate();

/** A canvas's `hc-file` link (bridge `navigate`, accepted from `frame` by
 *  `acceptFrameMessage`): open the vault file in the Preview pane, as a
 *  reply's file link does. Only while the person is using the frame
 *  (focused, with user activation), at most three a minute, and only a file
 *  inside `base` (the conversation's folder; default `chatCwd`) or the vault
 *  (`canvasFilePath`). It only shows the file, read only, so scripted canvases
 *  need no confirmation. `open` replaces the opener (the smoke harness). */
export function openFrameFile(target: string, frame: HTMLIFrameElement, base?: string | null, open: (path: string) => void = openFileInSidePane): void {
  const focused = document.activeElement === frame;
  const active = (navigator as { userActivation?: { isActive: boolean } }).userActivation?.isActive;
  if (!frame.isConnected || confirmDialog() || !frameFiles.admit(Date.now(), focused, active)) return;
  void (async () => {
    const root = await vaultRoot();
    const path = canvasFilePath(target, base === undefined ? chatCwd() : base, root);
    if (path && frame.isConnected) open(path);
    else if (!path) flashFooter('That canvas link isn’t a file in this folder or the vault', 2500);
  })();
}

/** A saved page review to open in the web mode's Page reviews layer
 *  (Results → Open in Page reviews). BrowserPane takes it and clears it. */
export const [pageReviewRequest, setPageReviewRequest] = createSignal<string | null>(null);
export async function openPageReview(path: string) {
  // The web mode's start page, which shows the review layer.
  openPreview({ kind: 'url', url: '' });
  setPageReviewRequest(path);
}
