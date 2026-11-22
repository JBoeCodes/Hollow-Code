/**
 * Browser and annotation review (next-level P15, 2026-10-01): the pure half.
 * Capture labels (a snapshot is a capture at a time and viewport, never a
 * live view), annotation drafts that survive navigation and reload, tab
 * scoping by workspace, and the before/after verdicts of a saved review
 * (Rust `workspace_reviews.rs`). No Solid, no Tauri: vitest loads it.
 */
import type { Mark } from './annotate';
import type { AnnotationSet, ReviewLink, ReviewVerdict } from './types';

// ── Captures ─────────────────────────────────────────────────────────────
export interface Size { width: number; height: number }

/** "14:02" today, else "Oct 1, 14:02" (24-hour, the user's locale order). */
export function captureTime(iso: string, now = new Date()): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  const time = d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', hour12: false });
  const sameDay = d.toDateString() === now.toDateString();
  return sameDay ? time : `${d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}, ${time}`;
}

export const viewportLabel = (v: Size) => `${Math.round(v.width)}×${Math.round(v.height)}`;

/** The label every snapshot carries: "Capture · 14:02 · 1280×720". */
export function captureLabel(capturedAt: string, view: Size, now = new Date()): string {
  return ['Capture', captureTime(capturedAt, now), viewportLabel(view)].filter(Boolean).join(' · ');
}

/** The sentence the chat draft gets, so the agent knows it is a capture. */
export function captureSentence(capturedAt: string, view: Size, now = new Date()): string {
  return `Captured ${captureTime(capturedAt, now)} at a ${viewportLabel(view)} viewport (a snapshot, not the live page).`;
}

// ── Tabs by workspace ────────────────────────────────────────────────────
/** The scope a tab belongs to: the workspace it was opened in, or `chat`
 *  (Conversation history, no workspace open). */
export const CHAT_SCOPE = 'chat';

export function tabsInScope<T extends { scope: string }>(tabs: readonly T[], scope: string): T[] {
  return tabs.filter(t => t.scope === scope);
}

/** The tab to show next in `scope` after `closed` (at `index`) went away. */
export function nextActive<T extends { id: string; scope: string }>(rest: readonly T[], scope: string, index: number): string | null {
  const mine = tabsInScope(rest, scope);
  return mine[Math.min(index, mine.length - 1)]?.id ?? null;
}

// ── Tabs that work (run 15 P2) ───────────────────────────────────────────
/** The tab to close for room at the cap: the one shown longest ago (the
 *  first such on a tie), never the one on show. Null when none can go. */
export function lruVictim<T extends { id: string; usedAt: number }>(tabs: readonly T[], active: string | null): string | null {
  let pick: T | null = null;
  for (const t of tabs) if (t.id !== active && (!pick || t.usedAt < pick.usedAt)) pick = t;
  return pick?.id ?? null;
}

/** A tab's load as the pane tracks it. WebKit sends a load event only once
 *  the server answered (browser_pane.rs): until one arrives, `committed` is
 *  false and the native page stays hidden behind "Loading…". */
export interface TabLoad {
  url: string; loading: boolean; committed: boolean; openedAt: number; answeredAt: number; error?: string | null;
  /** The error is the time limit's (the page may still come); a refusal or a
   *  Stop is not, and stays until the next open. */
  timedOut?: boolean;
  /** A new load's command hasn't settled yet: events are the old page's. */
  opening?: boolean;
}

/** A page that hasn't answered this long after it was opened fails, with
 *  Try again, instead of loading forever. */
export const NO_ANSWER_MS = 20_000;

/** A `loading` / `loaded` event: the page answered. An error the time limit
 *  set clears, since the page came after all. Any other error stays, with
 *  the address that failed: the event is then the old page's, which never
 *  left the webview (a refused address is never navigated to). */
export function tabAfterEvent(tab: Pick<TabLoad, 'error' | 'timedOut' | 'opening'>, kind: 'loading' | 'loaded', url: string | undefined, now: number): Partial<TabLoad> {
  // The command for a newer load hasn't settled: the old page sent this.
  if (tab.opening) return {};
  if (tab.error && !tab.timedOut) return { committed: true, loading: false };
  return { loading: kind === 'loading', committed: true, answeredAt: now, error: null, timedOut: false, ...(url ? { url } : {}) };
}

/** Whether the load opened at `openedAt` still has no answer after `limit`. */
export function unanswered(tab: Pick<TabLoad, 'loading' | 'openedAt' | 'answeredAt'>, now: number, limit: number): boolean {
  return tab.loading && tab.answeredAt < tab.openedAt && now - tab.openedAt >= limit;
}

/** Whether the address the webview reports may replace the tab's: not while
 *  a load is waiting for its answer (the webview still reports the old page)
 *  or after it failed, so the failure names, and Try again opens, what was
 *  asked for. */
export function followsPageUrl(tab: Pick<TabLoad, 'loading' | 'openedAt' | 'answeredAt' | 'error'>): boolean {
  return !tab.error && !(tab.loading && tab.answeredAt < tab.openedAt);
}

const hostOf = (url: string) => { try { return new URL(url).host || url; } catch { return url; } };

export function noAnswerMessage(url: string): string {
  return `${hostOf(url)} didn’t answer in ${Math.round(NO_ANSWER_MS / 1000)} seconds. Check the address, or open it in your browser to see why.`;
}

/** Stop ends the load. A page that already showed stays as it was; one that
 *  never answered says so, with Try again. */
export function stoppedTab(tab: Pick<TabLoad, 'committed' | 'url'>): Partial<TabLoad> {
  return tab.committed ? { loading: false } : { loading: false, error: `Stopped before ${hostOf(tab.url)} answered.` };
}

export interface Rect { left: number; top: number; right: number; bottom: number }

/** Whether a floating layer of the app's own (a menu, tooltip, popover or
 *  dialog) overlaps the page area. The native page draws above the DOM, so
 *  it steps aside for any such layer, however small; one only touching the
 *  page's edge, or with no area, doesn't count. */
export function layerCovers(page: Rect, layers: readonly Rect[]): boolean {
  return layers.some(l => l.right > l.left && l.bottom > l.top
    && l.right > page.left && l.left < page.right && l.bottom > page.top && l.top < page.bottom);
}

// ── Drafts ───────────────────────────────────────────────────────────────
/** An unsent annotation: the capture and its marks, kept until it is added
 *  to the chat, saved to the workspace, or discarded. */
export interface AnnotationDraft {
  scope: string;
  tabId: string;
  url: string;
  title: string;
  capturedAt: string;
  view: Size;
  /** The snapshot's own size: marks are in these pixels, even when the
   *  stored image had to be scaled down to fit. */
  imageSize: Size | null;
  image: string;
  marks: Mark[];
  savedAt: string;
}

export const DRAFTS_KEY = 'hollow.annotateDrafts';
/** Per scope; the oldest goes first. */
export const MAX_DRAFTS = 4;

type Store = Pick<Storage, 'getItem' | 'setItem'>;

function readAll(store: Store): AnnotationDraft[] {
  try {
    const v = JSON.parse(store.getItem(DRAFTS_KEY) ?? '[]');
    return Array.isArray(v) ? v.filter(d => d && typeof d.image === 'string' && Array.isArray(d.marks)) : [];
  } catch { return []; }
}

export function loadDrafts(store: Store, scope: string): AnnotationDraft[] {
  return readAll(store).filter(d => d.scope === scope).sort((a, b) => b.savedAt.localeCompare(a.savedAt));
}

export const sameDraft = (a: Pick<AnnotationDraft, 'scope' | 'tabId' | 'capturedAt'>, b: Pick<AnnotationDraft, 'scope' | 'tabId' | 'capturedAt'>) =>
  a.scope === b.scope && a.tabId === b.tabId && a.capturedAt === b.capturedAt;

/** Keep `draft` (replacing the same capture's earlier save). Returns false
 *  when storage refused it even after dropping older drafts. */
export function saveDraft(store: Store, draft: AnnotationDraft): boolean {
  const others = readAll(store).filter(d => !sameDraft(d, draft));
  const mine = others.filter(d => d.scope === draft.scope).sort((a, b) => b.savedAt.localeCompare(a.savedAt)).slice(0, MAX_DRAFTS - 1);
  let rest = [...others.filter(d => d.scope !== draft.scope), ...mine];
  for (;;) {
    try { store.setItem(DRAFTS_KEY, JSON.stringify([...rest, draft])); return true; }
    catch {
      if (!rest.length) return false;
      // Out of room: drop the oldest other draft and try again.
      rest = [...rest].sort((a, b) => a.savedAt.localeCompare(b.savedAt)).slice(1);
    }
  }
}

export function removeDraft(store: Store, draft: Pick<AnnotationDraft, 'scope' | 'tabId' | 'capturedAt'>) {
  try { store.setItem(DRAFTS_KEY, JSON.stringify(readAll(store).filter(d => !sameDraft(d, draft)))); } catch { /* nothing to free */ }
}

/** "3 marks" / "1 mark" / "no marks yet". */
export const marksLabel = (n: number) => (n ? `${n} mark${n === 1 ? '' : 's'}` : 'no marks yet');

// ── Saved reviews: before / after ────────────────────────────────────────
export const VERDICT_LABEL: Record<ReviewVerdict, string> = { done: 'Done', notDone: 'Not yet', unclear: 'Unclear' };

/** The newest verdict on mark `mark` of capture `capture`. */
export function latestLink(set: Pick<AnnotationSet, 'links'>, capture: string, mark: number): ReviewLink | null {
  for (let i = set.links.length - 1; i >= 0; i--) if (set.links[i].capture === capture && set.links[i].mark === mark) return set.links[i];
  return null;
}

/** Each requested change (a mark on the first capture) with the capture
 *  that shows its result, if the user has compared one. */
export function requestedChanges(set: Pick<AnnotationSet, 'captures' | 'links'>): { index: number; mark: Mark; link: ReviewLink | null }[] {
  const first = set.captures[0];
  if (!first) return [];
  return (first.marks as Mark[]).map((mark, index) => ({ index, mark, link: latestLink(set, first.id, index) }));
}

export function reviewProgress(set: Pick<AnnotationSet, 'captures' | 'links'>): { requested: number; done: number; open: number } {
  const all = requestedChanges(set);
  const done = all.filter(c => c.link?.status === 'done').length;
  return { requested: all.length, done, open: all.length - done };
}

/** "2 of 3 changes done", "No changes requested". */
export function progressLabel(p: { requested: number; done: number }): string {
  if (!p.requested) return 'No changes requested';
  return `${p.done} of ${p.requested} change${p.requested === 1 ? '' : 's'} done`;
}

/** The base64 body of a `data:` URL. */
export const dataUrlBody = (url: string) => url.replace(/^data:[^,]*,/, '');
