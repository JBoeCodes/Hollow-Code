// Settings' jump to a control (palette, links; overnight G3, morning P3).
// A section's lazy panel (Usage, Accounts…) isn't on the page yet the first
// time it opens: its chunk is still loading behind a skeleton. So the jump
// looks again until the control appears or a short limit passes, and stops
// when the person has moved on. Pure scheduling, so the tests drive it with
// fake timers.

/** Try `find` now and then every `every` ms until it returns something,
 *  `stillWanted()` turns false, or `limit` ms pass; `found` gets the result
 *  once. Returns a cancel. */
export function retryFind<T>(
  find: () => T | null | undefined,
  found: (t: T) => void,
  stillWanted: () => boolean,
  opts: { every?: number; limit?: number; now?: () => number; schedule?: (f: () => void, ms: number) => unknown } = {},
): () => void {
  const every = opts.every ?? 100;
  const limit = opts.limit ?? JUMP_LIMIT_MS;
  const now = opts.now ?? (() => Date.now());
  const schedule = opts.schedule ?? ((f: () => void, ms: number) => setTimeout(f, ms));
  const started = now();
  let cancelled = false;
  const attempt = () => {
    if (cancelled || !stillWanted()) return;
    const t = find();
    if (t != null) { found(t); return; }
    if (now() - started < limit) schedule(attempt, every);
  };
  attempt();
  return () => { cancelled = true; };
}

/** How long a jump looks for its control by default. */
export const JUMP_LIMIT_MS = 3000;
/** How long the control's own row is waited for before an earlier match. */
export const EXACT_GRACE_MS = 400;

/** A `find` for `retryFind` that prefers `exact` (the control's own row) and
 *  takes `any` (an earlier card or heading with the same name) only once it
 *  has been the best match for `grace` ms: on a cold Settings open a lazy
 *  section's rows can render after its explainer card, and the first look
 *  would otherwise land on the card (overnight 10-15/16 A07 review). */
export function preferExact<T>(exact: () => T | null | undefined, any: () => T | null | undefined, grace = EXACT_GRACE_MS, now: () => number = () => Date.now()): () => T | null {
  let since: number | null = null;
  return () => {
    const hit = exact();
    if (hit != null) return hit;
    const other = any();
    if (other == null) return null;
    since ??= now();
    return now() - since >= grace ? other : null;
  };
}

/** How long after a jump a target is kept in view while the section fills in. */
export const KEEP_IN_VIEW_MS = 2000;

/** After a jump's scroll, keep its target in view while content above it is
 *  still filling in (S2 review): a lazy part that mounts above the target
 *  (Accounts' panels on a cold jump to Skills) grows after the scroll, and
 *  WKWebView has no scroll anchoring to hold the target in place. Each size
 *  change `onResize` reports re-scrolls (`rescroll`), until `limit` ms pass
 *  or the person scrolls or moves focus (`onInterrupt`); the first of those
 *  stops it for good. Both subscribers return their unsubscribe. Returns a
 *  stop. Pure scheduling, so the tests drive it with fake timers. */
export function keepInView(opts: {
  onResize: (changed: () => void) => () => void;
  onInterrupt: (stop: () => void) => () => void;
  rescroll: () => void;
  limit?: number;
  now?: () => number;
  schedule?: (f: () => void, ms: number) => unknown;
}): () => void {
  const limit = opts.limit ?? KEEP_IN_VIEW_MS;
  const now = opts.now ?? (() => Date.now());
  const started = now();
  let done = false;
  const offs: (() => void)[] = [];
  const stop = () => { if (done) return; done = true; for (const off of offs.splice(0)) off(); };
  offs.push(opts.onInterrupt(stop));
  offs.push(opts.onResize(() => {
    if (done) return;
    if (now() - started >= limit) { stop(); return; }
    opts.rescroll();
  }));
  if (done) for (const off of offs.splice(0)) off();
  else (opts.schedule ?? ((f: () => void, ms: number) => setTimeout(f, ms)))(stop, limit);
  return stop;
}

/** What `openFoldsAround` needs of an element: DOM's `parentElement` and `closest`. */
export interface FoldLike { open?: boolean; parentElement: FoldLike | null; closest(selector: string): FoldLike | null }

/** Open every closed disclosure a row sits inside (System's Advanced fold and
 *  the groups in it), innermost first, so a search hit lands on a visible row
 *  its control can take focus in. The row itself, if it is a disclosure, stays
 *  as it was. Returns how many it opened. */
export function openFoldsAround(row: FoldLike): number {
  let opened = 0;
  for (let fold = row.parentElement?.closest('details') ?? null; fold; fold = fold.parentElement?.closest('details') ?? null) {
    if (!fold.open) opened++;
    fold.open = true;
  }
  return opened;
}

/** What `controlNamed` reads of an element (a DOM element, or a test's fake). */
export interface NamedLike {
  dataset: { settingsControl?: string };
  getAttribute(name: string): string | null;
  textContent: string | null;
  matches(selector: string): boolean;
  closest(selector: string): unknown;
}

/** Where a link's control is looked for on another page: named elements and headings. */
export const PAGE_CONTROL_SELECTOR = '[data-settings-control], [aria-label], h2, h3, h4, .settings-row-k, summary';

/** The element a moved control's label names on its new page (overnight
 *  11-13 S1: an old link into Library keeps its control). An element named
 *  exactly (`data-settings-control` or `aria-label`) first, then a heading
 *  or disclosure whose text starts with it; never one in a navigation list
 *  or tab strip, which name pages, not controls. */
export function controlNamed<T extends NamedLike>(candidates: readonly T[], label: string): T | undefined {
  const own = candidates.filter(el => !el.closest('nav, [role="tablist"]'));
  return own.find(el => el.dataset.settingsControl === label || el.getAttribute('aria-label') === label)
    ?? own.find(el => el.matches('h2, h3, h4, .settings-row-k, summary') && (el.textContent ?? '').replace(/\s+/g, ' ').trim().startsWith(label));
}

/** The page just opened, where a link's control is looked for: never the
 *  sidebar, top bar or a page kept mounted but hidden (Work). */
function openedPageControls(): HTMLElement[] {
  const root = document.querySelector('.canvas-split > .page') ?? document;
  return [...root.querySelectorAll<HTMLElement>(PAGE_CONTROL_SELECTOR)].filter(el => el.getClientRects().length > 0);
}

// The one landing looking right now: a newer link replaces it, and the
// person pointing, typing or leaving the page stops it (11-13 S1 review).
let stopLanding: (() => void) | undefined;

/** Stop the landing still looking, if any. */
export function cancelPageLanding() { stopLanding?.(); }

/** Bring the control `label` names into view on a page just opened (an old
 *  link into Library, overnight 11-13 S1), and focus it or its first
 *  control. Looks again while the tab's lazy view loads, up to the jump
 *  limit, only while `wanted()` holds (the page is still the one opened) and
 *  until the person points or presses a key; a newer landing cancels it. A
 *  control that never shows leaves the tab as it opened. Loaded on use
 *  (`settingsPageTarget`), so the shell chunk stays as it was. */
export function landOnPageControl<T extends NamedLike & { scrollIntoView(o: ScrollIntoViewOptions): void; focus(o: FocusOptions): void; querySelector(s: string): unknown } & FoldLike>(
  label: string,
  wanted: () => boolean = () => true,
  query: () => T[] = openedPageControls as unknown as () => T[],
  opts: Parameters<typeof retryFind>[3] & { onInterrupt?: (stop: () => void) => () => void } = {},
): () => void {
  stopLanding?.();
  const focusable = 'button, input, select, summary, [tabindex], [role="listbox"]';
  let stopped = false;
  let unlisten: () => void = () => {};
  const stop = () => { stopped = true; unlisten(); if (stopLanding === stop) stopLanding = undefined; };
  stopLanding = stop;
  unlisten = (opts.onInterrupt ?? interruptOnInput)(stop);
  const limit = opts.limit ?? JUMP_LIMIT_MS;
  const now = opts.now ?? (() => Date.now());
  const started = now();
  // The last look that finds nothing stops it, so its input listeners go.
  const find = () => {
    const hit = controlNamed(query(), label);
    if (!hit && now() - started >= limit) stop();
    return hit;
  };
  const cancel = retryFind(find, el => {
    stop();
    openFoldsAround(el);
    el.scrollIntoView({ block: 'center' });
    ((el.matches(focusable) ? el : el.querySelector(focusable)) as { focus?(o: FocusOptions): void } | null)?.focus?.({ preventScroll: true });
  }, () => {
    const still = !stopped && wanted() && now() - started <= limit;
    if (!still) stop();
    return still;
  }, { limit, ...opts });
  return () => { cancel(); stop(); };
}

/** Pointing, typing or scrolling ends a landing (DOM listeners). */
function interruptOnInput(stop: () => void): () => void {
  if (typeof window === 'undefined') return () => {};
  const events = ['pointerdown', 'keydown', 'wheel', 'touchstart'] as const;
  for (const e of events) window.addEventListener(e, stop, { capture: true, passive: true });
  return () => { for (const e of events) window.removeEventListener(e, stop, { capture: true }); };
}
