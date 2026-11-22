import { For, Show, createEffect, createResource, createSignal, on, onCleanup, onMount } from 'solid-js';
import { listen, onNative, type UnlistenFn } from '../nativeEvents';
import { inspectionForTab, showPreviewBrowser, type PreviewInspection } from '../previewCursor';
import { observeKitMotion } from '../motionKit';
import { Icon } from '../icons';
import { api, type BrowserBounds } from '../api';
import { activePage, chatAgentSessionKey, chatCwd, chatSessionLoading, chatTurns, conversationHistoryShown, flashFooter, openInBrowser, showConfirm, trackTask } from '../store';
import { chatName, grantOn, type TabGrant } from '../tabGrants';
import { workspaceDetail, workspaceLoading } from '../store/workspaces';
import {
  activeBrowserTab, browserFocusNonce, browserGo, browserTabs, browserWorkspaceScope, closeBrowserTab, newBrowserTab,
  noteBrowserBounds, openBrowserUrl, pageReviewRequest, registerBrowserPage, retryBrowserTab, setPageReviewRequest, setActiveBrowserTab, setBrowserFocusNonce, syncBrowserUrl,
} from '../store/browser';
import { tabLabel } from '../browserUrl';
import { CHAT_SCOPE, captureLabel, layerCovers, loadDrafts, marksLabel, removeDraft, type AnnotationDraft, type Rect } from '../browserReview';
import type { ServiceView } from '../workspaceTypes';
import { BrowserAnnotate, type AnnotateTarget } from './BrowserAnnotate';
import { BrowserReviews, type Shot } from './BrowserReviews';
import { Button, StatePanel, StatusPill } from './ui';
import { errMsg, openExternal } from '../utils';
import { chordFromEvent } from '../shortcuts';
import '../styles/browser-pane.css';

/** The pane's own chords (localKeys.ts, Browser panel). */
const PANE_KEYS: Record<string, 'back' | 'forward' | 'reload'> = { 'CmdOrCtrl+[': 'back', 'CmdOrCtrl+]': 'forward', 'CmdOrCtrl+R': 'reload' };

/** Sample grid for "is anything drawn over the page area?" (see covered),
 *  inset so chrome that only hugs an edge (the panel's resize handle
 *  straddles the left one) doesn't count as covering the page. */
const GRID_X = 6, GRID_Y = 8, INSET = 8;
/** The app's floating layers, any of which hides the page however small it
 *  is (the grid misses a tooltip between its points; run 15 P2). */
const LAYERS = '[role="menu"], [role="listbox"], [role="tooltip"], [role="dialog"], [role="alertdialog"], [popover]';
/** Width a notice needs left of the page to move there (the stack's 380px
 *  and its margins, styles/notifications-people.css). */
const TOAST_ROOM = 420;
/** A load slower than this offers Stop / Open in your browser. */
const SLOW_MS = 12_000;

const frames = (n: number) => new Promise<void>(resolve => { const step = (k: number) => (k <= 0 ? resolve() : requestAnimationFrame(() => step(k - 1))); step(n); });

/**
 * The Preview pane's web mode: a tab strip, a toolbar and the page area.
 * The page itself is a native webview laid over `.browser-page` (native views
 * draw above the DOM), so this measures the area every frame and hides the
 * page whenever DOM is drawn over it (a menu, a dialog, a tooltip) or the
 * view isn't showing, and until the page first answers (run 15 P2: WebKit
 * reports no failed load, so "Loading…", then "Couldn't open", show instead
 * of a blank page). Notices move left of a live page. The page is kept alive
 * while hidden.
 *
 * Next-level P15 (tabs per workspace until run 15 P1 made them one app-wide
 * list; reviews, drafts and services still follow the open workspace); a dev
 * preview strip for this workspace's running dev servers; failed and slow
 * loads have their own states; ⌘[ / ⌘] / ⌘R while focus is in the pane;
 * Annotate captures are labelled as captures and kept as drafts; reviews
 * saved to the workspace open over the page with a before/after compare.
 */
export function BrowserPane(props: { unifiedTabs?: boolean } = {}) {
  let page!: HTMLDivElement;
  let field!: HTMLInputElement;
  const tab = () => browserTabs().find(t => t.id === activeBrowserTab());
  const [address, setAddress] = createSignal('');
  const [editing, setEditing] = createSignal(false);
  const [covered, setCovered] = createSignal(false);
  // Annotate: the tab frozen as a snapshot to mark up (BrowserAnnotate).
  const [annotating, setAnnotating] = createSignal<AnnotateTarget | null>(null);
  const [capturing, setCapturing] = createSignal(false);
  // Page reviews (BrowserReviews): open, and which one first.
  const [reviews, setReviews] = createSignal<{ path: string | null } | null>(null);
  // While a comparison captures the page, the review layer steps aside
  // (stays mounted, invisible) so WebKit has the page on show to snapshot.
  const [suspended, setSuspended] = createSignal(false);
  const [slow, setSlow] = createSignal(false);
  const [draftsTick, setDraftsTick] = createSignal(0);
  /** Where the page was last shown, for mapping marks back to it. */
  let placed: BrowserBounds | null = null;
  // The tabs are one app-wide list; reviews, drafts and services belong to
  // the workspace open on its page (run 15 P1 review).
  const workspace = () => (browserWorkspaceScope() !== CHAT_SCOPE ? workspaceDetail()?.workspace ?? null : null);
  const overlay = () => !!annotating() || (!!reviews() && !suspended());
  createEffect(() => { if (!editing()) setAddress(tab()?.url === 'about:blank' ? '' : tab()?.url ?? ''); });
  // A pane that has just loaded (it is lazy, next-level P21) may not be in
  // the document yet at the microtask; try again on the next frame.
  const focusField = () => {
    const expand = page?.closest('.preview-mini')?.querySelector<HTMLButtonElement>('.preview-player-controls button[aria-expanded="false"]');
    if (expand) {
      expand.click();
      requestAnimationFrame(() => { field?.focus(); field?.select(); });
      return;
    }
    if (!field?.isConnected) { requestAnimationFrame(() => { field?.focus(); field?.select(); }); return; }
    field.focus(); field.select();
  };
  createEffect(on(browserFocusNonce, n => { if (n) queueMicrotask(focusField); }, { defer: true }));

  // Running dev servers here: the new-tab page's buttons and the dev
  // preview strip. A workspace's services run in its folder.
  const serviceCwd = () => workspace()?.cwd ?? (chatCwd() || 'all');
  const [services, { refetch: refetchServices }] = createResource(serviceCwd, key =>
    api.listServices(key === 'all' ? undefined : key).then(list => list.filter(s => s.status === 'running' && s.port)).catch(() => [] as ServiceView[]));
  let unlistenServices: UnlistenFn | undefined;
  let disposed = false;
  listen('services-changed', () => void refetchServices()).then(fn => { if (disposed) fn(); else unlistenServices = fn; }).catch(() => { /* browser mock */ });
  const servicesTimer = setInterval(() => void refetchServices(), 10_000);
  onCleanup(() => { disposed = true; unlistenServices?.(); clearInterval(servicesTimer); });
  const serviceUrl = (s: ServiceView) => `http://localhost:${s.port}`;
  const showing = (s: ServiceView) => !!tab()?.url && tab()!.url.replace(/\/$/, '').startsWith(serviceUrl(s));
  async function stopService(s: ServiceView) {
    const ok = await showConfirm({ title: `Stop ${s.name}?`, body: `Ends ${s.command} and everything it started. Its page stops answering.`, confirmLabel: 'Stop' });
    if (!ok) return;
    try { await trackTask(`Stop ${s.name}`, () => api.stopService(s.scope, s.name)); } catch (e) { flashFooter(`× ${errMsg(e)}`); }
    void refetchServices();
  }

  // Unsent annotations of this scope (the one being edited excluded).
  const drafts = () => {
    void draftsTick();
    const open = annotating();
    return loadDrafts(localStorage, browserWorkspaceScope()).filter(d => !open || d.capturedAt !== open.capturedAt || d.tabId !== open.tabId);
  };
  const restore = (d: AnnotationDraft) => {
    setReviews(null);
    setAnnotating({ tabId: d.tabId, image: d.image, view: d.view, url: d.url, title: d.title, capturedAt: d.capturedAt, scope: d.scope, size: d.imageSize, marks: d.marks });
  };
  const discardDraft = (d: AnnotationDraft) => { removeDraft(localStorage, d); setDraftsTick(n => n + 1); };

  // Anything of the app's own DOM on top of the page area: at any grid
  // point, or a floating layer overlapping it anywhere.
  const coveredNow = (r: DOMRect) => {
    for (let i = 0; i < GRID_X; i++) for (let j = 0; j < GRID_Y; j++) {
      const x = r.left + INSET + (r.width - 2 * INSET) * (i / (GRID_X - 1));
      const y = r.top + INSET + (r.height - 2 * INSET) * (j / (GRID_Y - 1));
      const hit = document.elementFromPoint(x, y);
      if (!hit || !page.contains(hit)) return true;
    }
    const layers: Rect[] = [];
    for (const el of document.querySelectorAll(LAYERS)) {
      if (page.contains(el) || el.contains(page)) continue;
      // Closed details can retain a body's layout rectangle even though it
      // paints nothing. Its summary remains visible and may own a tooltip.
      let hidden = false;
      for (let parent = el.parentElement; parent; parent = parent.parentElement) {
        if (parent.tagName === 'DETAILS' && !parent.hasAttribute('open') && !parent.querySelector(':scope > summary')?.contains(el)) { hidden = true; break; }
      }
      if (hidden) continue;
      const style = getComputedStyle(el);
      if (style.display !== 'none' && style.visibility === 'visible') layers.push(el.getBoundingClientRect());
    }
    return layerCovers(r, layers);
  };
  // While a live page shows, notices stack left of it (styles/browser-pane.css):
  // over it, every toast hid the whole page for its nine seconds.
  const root = document.documentElement;
  let asideAt = '';
  const toastsAside = (left: number | null) => {
    const at = left == null ? '' : `${left}px`;
    if (at === asideAt) return;
    asideAt = at;
    if (at) { root.style.setProperty('--native-page-left', at); root.setAttribute('data-native-page', ''); }
    else { root.removeAttribute('data-native-page'); root.style.removeProperty('--native-page-left'); }
  };

  onMount(() => {
    let raf = 0, dirty = true, shown = '', rectKey = '';
    const mark = () => { dirty = true; };
    // Cursor notifications repaint only this tab and navigation. The native
    // browser retains the coordinates and rechecks the original share.
    const invalidateCursor = () => { shown = ''; mark(); };
    const stopCursor = onNative<PreviewInspection>('browser-inspection', event => {
      if (inspectionForTab(event, tab())) invalidateCursor();
    });
    const stopMotion = observeKitMotion(page, invalidateCursor);
    const observer = new MutationObserver(mark);
    // Page transitions cover the native view with a root-level pseudo-layer;
    // its removal changes <html>, not body. Recheck when that layer clears.
    observer.observe(document.documentElement, { subtree: true, childList: true, attributes: true, attributeFilter: ['class', 'style', 'open', 'hidden', 'data-resizing'] });
    window.addEventListener('resize', mark);
    document.addEventListener('visibilitychange', mark);
    const frame = () => {
      raf = requestAnimationFrame(frame);
      const r = page.getBoundingClientRect();
      const key = `${Math.round(r.left)},${Math.round(r.top)},${Math.round(r.width)},${Math.round(r.height)}`;
      if (key !== rectKey) { rectKey = key; dirty = true; }
      if (!dirty) return;
      dirty = false;
      // The pane's resize handle lies over the page area's left edge; the
      // native page would sit above it and swallow the drag, so start after it.
      const grip = page.closest('.preview-pane')?.querySelector<HTMLElement>(':scope > .resize-handle')?.getBoundingClientRect();
      const left = grip && grip.width && grip.right > r.left && grip.left <= r.left + 1 ? Math.ceil(grip.right) : Math.round(r.left);
      const bounds: BrowserBounds = { x: left, y: Math.round(r.top), width: Math.round(r.right) - left, height: Math.round(r.height) };
      if (bounds.width > 20 && bounds.height > 20) noteBrowserBounds(bounds);
      const t = tab();
      // A page that hasn't answered stays hidden: WebKit would show a blank
      // white page over "Loading…" (run 15 P2).
      const live = !!t?.ready && !!t.committed && !t.error && bounds.width > 20 && bounds.height > 20 && document.visibilityState === 'visible';
      // Only with room for a notice on the left; else they stay on screen
      // and the page steps aside while one shows (a compact window).
      toastsAside(live && bounds.x >= TOAST_ROOM ? bounds.x : null);
      const hide = !live || !!document.body.dataset.resizing || overlay() || coveredNow(r);
      setCovered(live && hide);
      const next = hide ? 'hidden' : `${t!.id}|${key}`;
      if (next === shown) return;
      shown = next;
      if (!hide) placed = bounds;
      void showPreviewBrowser(hide ? null : t!.id, hide ? null : bounds, page).catch(() => { shown = ''; });
    };
    raf = requestAnimationFrame(frame);
    // Tab or scope changes and a tab becoming ready or answering re-place the page.
    createEffect(() => { void tab()?.id; void tab()?.ready; void tab()?.committed; void tab()?.error; void overlay(); void browserWorkspaceScope(); mark(); });
    // Switching to another tab (or workspace) ends an annotation; its marks
    // stay as a draft and the restore notice offers them back.
    createEffect(on(() => `${browserWorkspaceScope()}|${tab()?.id ?? ''}`, () => { if (annotating()) setAnnotating(null); setReviews(null); setDraftsTick(n => n + 1); }, { defer: true }));
    // Results → Open in Page reviews: open that review over the page.
    createEffect(() => { const path = pageReviewRequest(); if (!path || !workspace()) return; setAnnotating(null); setReviews({ path }); setPageReviewRequest(null); });
    // A load slower than SLOW_MS says so.
    let slowTimer: number | undefined;
    createEffect(() => {
      const loading = !!tab()?.loading; void tab()?.id;
      window.clearTimeout(slowTimer); setSlow(false);
      if (loading) slowTimer = window.setTimeout(() => setSlow(true), SLOW_MS);
    });
    // Single-page apps change the address without a page load.
    const poll = setInterval(() => { const t = tab(); if (t?.ready && shown !== 'hidden') void syncBrowserUrl(t.id); }, 1500);
    // The menu's Back and Forward step the page while it's on show (P1-47).
    const unregister = registerBrowserPage(() => !!tab()?.ready && !!shown && shown !== 'hidden');
    // ⌘L focuses the address field while the view shows.
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey && !e.shiftKey && !e.altKey && e.key.toLowerCase() === 'l') { e.preventDefault(); setBrowserFocusNonce(n => n + 1); }
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => {
      unregister();
      stopCursor(); stopMotion();
      cancelAnimationFrame(raf);
      clearInterval(poll);
      window.clearTimeout(slowTimer);
      observer.disconnect();
      window.removeEventListener('resize', mark);
      window.removeEventListener('keydown', onKey);
      document.removeEventListener('visibilitychange', mark);
      toastsAside(null);
      void api.browserShow(null, null).catch(() => { /* no tabs */ });
    });
  });
  if (!activeBrowserTab() && !browserTabs().length) queueMicrotask(() => setBrowserFocusNonce(n => n + 1));

  /** In-pane shortcuts (local-command policy: only while focus is in the
   *  pane, never global): ⌘[ back, ⌘] forward, ⌘R reload. */
  const onPaneKey = (e: KeyboardEvent) => {
    if (!tab() || overlay()) return;
    // By the physical key, as App's keymap matches it, so a non-US layout's
    // ⌘[ still lands here and not in Hollow's Back (P1-47).
    const action = PANE_KEYS[chordFromEvent(e) ?? ''];
    if (action) { e.preventDefault(); browserGo(action); }
  };

  const submit = (e: Event) => {
    e.preventDefault();
    // Read before leaving edit mode: that resets the field to the tab's URL.
    const typed = address();
    const newTab = !tab();
    setEditing(false);
    field.blur();
    void openBrowserUrl(typed, { newTab });
  };
  /** Freeze the tab as a snapshot and mark it up. Captured while the page
   *  is still on show (WebKit snapshots what it draws). */
  const annotate = async () => {
    const t = tab();
    if (!t?.ready || capturing()) return;
    if (annotating()) { setAnnotating(null); return; }
    setReviews(null);
    setCapturing(true);
    // Whose capture it is, fixed now: the tab list is app-wide, so the same
    // tab can still be on show after a switch to another workspace while the
    // snapshot is taken (run 15 P1 review).
    const scope = browserWorkspaceScope();
    try {
      const capturedAt = new Date().toISOString();
      const png = await api.browserSnapshot(t.id);
      if (disposed || browserWorkspaceScope() !== scope) return;
      const view = placed ?? { width: page.clientWidth, height: page.clientHeight };
      if (tab()?.id === t.id) setAnnotating({ tabId: t.id, image: `data:image/png;base64,${png}`, view: { width: view.width, height: view.height }, url: t.url, title: t.title, capturedAt, scope });
    } catch (e) { flashFooter(`× ${errMsg(e)}`); }
    finally { setCapturing(false); }
  };
  /** A capture for "Compare with a newer capture": the review layer steps
   *  aside so the page shows, then WebKit snapshots it. */
  const captureForReview = async (): Promise<Shot | null> => {
    const t = tab();
    if (!t?.ready) { flashFooter('× Open the page in a tab first.'); return null; }
    setSuspended(true);
    try {
      await frames(3);
      await new Promise(r => setTimeout(r, 120));
      const capturedAt = new Date().toISOString();
      const png = await api.browserSnapshot(t.id);
      const image = `data:image/png;base64,${png}`;
      const view = placed ?? { width: page.clientWidth, height: page.clientHeight };
      const imageSize = await new Promise<{ width: number; height: number }>(resolve => {
        const img = new Image();
        img.onload = () => resolve({ width: img.naturalWidth, height: img.naturalHeight });
        img.onerror = () => resolve({ width: view.width, height: view.height });
        img.src = image;
      });
      return { image, view: { width: view.width, height: view.height }, imageSize, url: t.url, title: t.title, capturedAt };
    } catch (e) { flashFooter(`× ${errMsg(e)}`); return null; }
    finally { setSuspended(false); }
  };
  const external = () => { const url = tab()?.url; if (url) openInBrowser(url).catch(() => openExternal(url)); };
  // Repo ideas 2026-10-27 #5 (read only): Share with chat lets the
  // conversation on screen read this tab's text (`ui.read_tab`) during one
  // turn. While anything is shared, look again every 3 s: the grant ends
  // with that turn, in Rust. A failed look keeps the last list (and its
  // Stop), says so and tries again; a look that started before a share or a
  // Stop is dropped (review of #1064).
  const [grants, setGrants] = createSignal<TabGrant[]>([]);
  const [grantsStale, setGrantsStale] = createSignal(false);
  let grantLook = 0;
  const refetchGrants = async () => {
    const look = ++grantLook;
    try {
      const list = await api.browserTabGrants();
      if (look === grantLook) { setGrants(list); setGrantsStale(false); }
    } catch { if (look === grantLook) setGrantsStale(true); }
  };
  void refetchGrants();
  const grantTimer = setInterval(() => { if (grants().length || grantsStale()) void refetchGrants(); }, 3000);
  onCleanup(() => clearInterval(grantTimer));
  const sharedHere = () => grantOn(grants(), tab()?.id);
  // Only the conversation on screen, once it has settled: Conversation
  // history (Library → Conversations), or an open workspace whose centre
  // shows its own conversation. A key kept from a chat left for Home or a
  // workspace's documents is never shared with.
  const chatOnScreen = () => {
    if (chatSessionLoading()) return null;
    const d = workspaceDetail();
    const inWorkspace = activePage() === 'today' && !!d && !workspaceLoading() && chatCwd() === d.workspace.cwd;
    return conversationHistoryShown() || inWorkspace ? chatAgentSessionKey() : null;
  };
  const share = async () => {
    const t = tab(), key = chatOnScreen();
    if (!t?.ready || !key) return;
    try {
      const g = await api.browserGrantTab(t.id, key, chatName(chatTurns.find(turn => turn.role === 'user')?.text));
      grantLook++;
      setGrants(list => [...list.filter(x => x.tabId !== g.tabId && x.sessionKey !== g.sessionKey), g]);
      flashFooter(`Shared with “${g.title}” for its turn (read only)`, 3000);
    } catch (e) { flashFooter(`× ${errMsg(e)}`); }
  };
  const stopSharing = async (tabId: string) => {
    try { await api.browserRevokeTab(tabId); } catch (e) { flashFooter(`× ${errMsg(e)}`); return; }
    grantLook++;
    setGrants(list => list.filter(g => g.tabId !== tabId));
  };

  return (
    // A listener on the pane itself (not Solid's delegated one on document),
    // so it runs before App's keymap, which then skips the key (P1-47).
    <div class="browser-pane" on:keydown={onPaneKey}>
      <Show when={!props.unifiedTabs}><div class="browser-tabs" role="tablist" aria-label="Browser tabs">
        <For each={browserTabs()}>{t =>
          <div class="browser-tab" role="presentation" data-active={t.id === activeBrowserTab()}>
            <button type="button" role="tab" class="browser-tab-main" aria-selected={t.id === activeBrowserTab()} title={t.url} onClick={() => setActiveBrowserTab(t.id)}>
              <Show when={t.loading} fallback={<Icon name={t.error ? 'alert' : 'globe'} size={12} />}><span class="browser-tab-spin" aria-hidden="true" /></Show>
              <span class="browser-tab-label">{tabLabel(t.url, t.title)}</span>
            </button>
            <button type="button" class="browser-tab-close" aria-label={`Close ${tabLabel(t.url, t.title)}`} title="Close tab" onClick={() => void closeBrowserTab(t.id)}>×</button>
          </div>
        }</For>
        <button type="button" class="browser-icon-btn browser-new-tab" aria-label="New tab" title="New tab" onClick={newBrowserTab}><Icon name="plus" size={13} /></button>
      </div></Show>
      <form class="browser-toolbar" onSubmit={submit}>
        <button type="button" class="browser-icon-btn" aria-label="Back" title="Back (⌘[)" disabled={!tab()} onClick={() => browserGo('back')}><Icon name="arrowLeft" size={14} /></button>
        <button type="button" class="browser-icon-btn" aria-label="Forward" title="Forward (⌘])" disabled={!tab()} onClick={() => browserGo('forward')}><Icon name="arrowRight" size={14} /></button>
        <Show when={tab()?.loading} fallback={
          <button type="button" class="browser-icon-btn" aria-label="Reload" title="Reload (⌘R)" disabled={!tab()} onClick={() => browserGo('reload')}><Icon name="refresh" size={14} /></button>
        }>
          <button type="button" class="browser-icon-btn" aria-label="Stop loading" title="Stop loading" onClick={() => browserGo('stop')}><Icon name="stop" size={14} /></button>
        </Show>
        <input ref={field} class="browser-address" type="text" inputmode="url" spellcheck={false} autocomplete="off"
          aria-label="Address" placeholder="Search or enter an address" value={address()}
          onFocus={() => { setEditing(true); queueMicrotask(() => field.select()); }}
          onBlur={() => setEditing(false)}
          onInput={e => setAddress(e.currentTarget.value)}
          onKeyDown={e => { if (e.key === 'Escape') { setEditing(false); setAddress(tab()?.url ?? ''); field.blur(); } }} />
        <button type="button" class="browser-icon-btn browser-annotate-btn" aria-label="Annotate" aria-pressed={!!annotating()} disabled={!tab()?.ready || capturing()}
          title="Annotate: capture this page, mark it up and add it to the chat" onClick={() => void annotate()}><Icon name="squarePen" size={14} /><span>Annotate</span></button>
        <Show when={workspace()}>
          <button type="button" class="browser-icon-btn" aria-label="Page reviews" aria-pressed={!!reviews()} title="Page reviews saved to this workspace"
            onClick={() => { setAnnotating(null); setReviews(reviews() ? null : { path: null }); }}><Icon name="compare" size={14} /></button>
        </Show>
        <button type="button" class="browser-icon-btn browser-share-btn" aria-label="Share with chat" aria-pressed={!!sharedHere()} disabled={!tab()?.ready || (!sharedHere() && !chatOnScreen())}
          title={sharedHere() ? 'Stop sharing this tab' : chatOnScreen() ? "Let the conversation on screen read this tab's text (read only) during its turn: the one running, else its next" : 'Open the conversation that should read this tab (its first message sent) to share it'}
          onClick={() => void (sharedHere() ? stopSharing(sharedHere()!.tabId) : share())}><Icon name="eye" size={14} /><span>Share with chat</span></button>
        <button type="button" class="browser-icon-btn" aria-label="Open in your browser" title="Open in your browser" disabled={!tab()} onClick={external}><Icon name="externalLink" size={14} /></button>
      </form>
      <Show when={sharedHere()}>{g => (
        <div class="browser-grant" role="status">
          <Icon name="eye" size={13} />
          <span>In use by <strong title={g().title}>{g().title}</strong> · it can read this tab's text, nothing more, until its turn ends<Show when={grantsStale()}> · couldn't check just now, trying again</Show></span>
          <Button size="sm" variant="ghost" onClick={() => void stopSharing(g().tabId)}>Stop</Button>
        </div>
      )}</Show>
      <Show when={tab()?.loading}><div class="browser-loading-bar" role="progressbar" aria-label={`Loading ${tabLabel(tab()!.url, '')}`} /></Show>
      <Show when={workspace() && services()?.length}>
        <div class="browser-dev" role="group" aria-label="Dev preview">
          <span class="browser-dev-label">Dev preview</span>
          <For each={services()}>{s =>
            <span class="browser-dev-service">
              <StatusPill quiet label="Running" tone="ok" hint={`${s.command} in ${s.cwd}`} />
              <strong>{s.name}</strong><span class="browser-dev-port">:{s.port}</span>
              <Show when={showing(s)} fallback={<Button size="sm" variant="ghost" onClick={() => void openBrowserUrl(serviceUrl(s), { newTab: true })}>Open</Button>}>
                <span class="browser-dev-showing">Showing</span>
              </Show>
              <Button size="sm" variant="ghost" onClick={() => void stopService(s)}>Stop</Button>
            </span>
          }</For>
        </div>
      </Show>
      <Show when={!annotating() && drafts().length}>
        <div class="browser-drafts" role="status" aria-label="Unsent annotations">
          <For each={drafts()}>{d =>
            <span class="browser-draft">
              <Icon name="squarePen" size={12} />
              <span class="browser-draft-text">Unsent annotation of {tabLabel(d.url, d.title)} · {captureLabel(d.capturedAt, d.view)} · {marksLabel(d.marks.length)}</span>
              <Button size="sm" variant="ghost" onClick={() => restore(d)}>Restore</Button>
              <Button size="sm" variant="ghost" onClick={() => discardDraft(d)}>Discard</Button>
            </span>
          }</For>
        </div>
      </Show>
      <div class="browser-page" ref={page} data-tab={tab()?.id ?? ''}>
        <Show when={annotating()} keyed fallback={
          <Show when={tab()} fallback={
            <div class="browser-start">
              <Icon name="globe" size={22} />
              <h2>Open a page beside the conversation.</h2>
              <p>Type an address or a search above. Links in replies open here too; hold ⌘ to open one in your browser instead.</p>
              <Show when={services()?.length}>
                <div class="browser-services" aria-label="Running here">
                  <For each={services()}>{s =>
                    <button type="button" class="btn small ghost" onClick={() => void openBrowserUrl(serviceUrl(s), { newTab: true })}>
                      <Icon name="globe" size={12} /> {s.name} :{s.port}
                    </button>
                  }</For>
                </div>
              </Show>
              <p class="browser-auth-note">Signing in to a site here is ordinary browsing. It doesn't connect that service to Hollow or let agents use it: agents reach services only through connectors you share in Settings.</p>
            </div>
          }>
            <Show when={!tab()!.error} fallback={
              <StatePanel kind="failed" class="browser-failed" title={`Couldn't open ${tabLabel(tab()!.url, '')}`}
                actions={<>
                  <Button size="sm" variant="primary" onClick={() => void retryBrowserTab(tab()!.id)}>Try again</Button>
                  <Button size="sm" variant="ghost" onClick={external}>Open in your browser</Button>
                  <Button size="sm" variant="ghost" onClick={() => void closeBrowserTab(tab()!.id)}>Close tab</Button>
                </>} error={tab()!.error ?? ''} />
            }>
              <div class="browser-placeholder" aria-live="polite">
                <Show when={covered()} fallback={
                  <Show when={tab()?.loading}>
                    <span class="browser-loading-text">
                      <span class="browser-tab-spin" aria-hidden="true" />
                      {slow() ? `Still loading ${tabLabel(tab()!.url, '')}…` : `Loading ${tabLabel(tab()!.url, '')}…`}
                      <Show when={slow()}>
                        <Button size="sm" variant="ghost" onClick={() => browserGo('stop')}>Stop</Button>
                        <Button size="sm" variant="ghost" onClick={external}>Open in your browser</Button>
                      </Show>
                    </span>
                  </Show>
                }>
                  <span>{tabLabel(tab()!.url, tab()!.title)}</span>
                </Show>
              </div>
            </Show>
          </Show>
        }>{target => <BrowserAnnotate target={target} workspaceId={workspace()?.id ?? null}
          onClose={() => { setAnnotating(null); setDraftsTick(n => n + 1); }}
          onSaved={set => { setAnnotating(null); setReviews({ path: set.path }); }} />}</Show>
        <Show when={reviews() && workspace()}>
          <div class="browser-reviews-layer" data-suspended={suspended() ? '' : undefined}>
            <BrowserReviews workspaceId={workspace()!.id} initialPath={reviews()!.path} canCapture={!!tab()?.ready && !tab()?.error}
              capture={captureForReview} onOpenUrl={url => { setReviews(null); void openBrowserUrl(url, { newTab: true }); }} onClose={() => setReviews(null)} />
          </div>
        </Show>
      </div>
    </div>
  );
}
