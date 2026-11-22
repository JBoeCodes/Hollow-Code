// Page transitions through the View Transitions API (overnight 10-11 B1).
//
// A user page jump runs its state change inside `document.startViewTransition`,
// so the page column (`view-transition-name: hollow-page`, styles/motion.css)
// rises in opaque and the rail and top bar stay live. A click on a workspace card
// or rail row can name itself the morph source: it then grows into the page it
// opens, because it takes the page column's transition name for the old
// snapshot.
//
// Never a stall: no transition when motion is reduced (Settings or system),
// the window is away or hidden, the engine lacks the API, or a transition is
// already running; and an update the engine hasn't run within START_LIMIT_MS
// is applied directly. The update runs exactly once either way.
//
// Never a lost click: while a transition plays, Chromium hit-tests the
// document element whatever `::view-transition { pointer-events }` says, so a
// click in the rail 60 ms after opening a workspace did nothing (overnight
// 10-11/12 P4). Such a click skips the motion and goes to what is under the
// pointer outside the page column (`catchOverlayClicks`).

import { motionReduced } from './motion';

/** The page column's transition name (styles/motion.css). */
export const PAGE_NAME = 'hollow-page';
/** The root's class while a transition runs. */
export const TRANSITIONING = 'is-transitioning';
/** How long a transition may wait for the engine to start it. */
export const START_LIMIT_MS = 300;

type Doc = Document & { startViewTransition?: (update: () => Promise<void> | void) => ViewTransition };

let running = false;
let morphSource: HTMLElement | null = null;
let morphTarget: HTMLElement | null = null;
/** The running transition's update while the engine hasn't run it yet:
 *  a navigation in that gap applies it first, so a later jump is never
 *  overwritten by an earlier one (⌘[ twice, or A then B). */
let pending: (() => void) | null = null;
/** The transition whose overlay covers the window now. */
let active: ViewTransition | null = null;
/** Documents whose overlay clicks are caught (`catchOverlayClicks`). */
const catching = new WeakSet<object>();
/** The page column, which the transition animates (beside the Preview pane
 *  in the canvas split since run 15 P1). */
const PAGE_SELECTOR = '.canvas-split > .page';

/** Apply a waiting visual update before a newer immediate choice (task
 *  keyboard selection, Close or New). Its delayed callback then does nothing. */
export function flushViewTransition() { pending?.(); }

/** Whether a page change may animate now. */
export function canTransition(doc: Doc | undefined = pageDocument()): boolean {
  if (!doc || typeof doc.startViewTransition !== 'function') return false;
  if (running) return false;
  const root = doc.documentElement;
  if (motionReduced(doc)) return false;
  // Launch has its own entrance (main.tsx `is-booting`).
  if (root.classList.contains('is-booting')) return false;
  if (doc.visibilityState !== 'visible') return false;
  if (doc.body?.classList.contains('is-away')) return false;
  return true;
}

/** Name `el` (a workspace card, a rail row) as what the next transition grows
 *  into the page. Taken by the next `withViewTransition`; a click handler sets
 *  it right before the navigation it starts. */
export function morphFrom(el: EventTarget | null | undefined, target?: HTMLElement | null) {
  morphSource = typeof HTMLElement !== 'undefined' && el instanceof HTMLElement ? el : null;
  morphTarget = target ?? null;
}

/** The window's document; none under Node (unit tests): no transitions. */
const pageDocument = (): Doc | undefined => (typeof document === 'undefined' ? undefined : document);

const wait = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms));

/** A person's click or right-click that hit the overlay (the document
 *  element) while a transition plays: skip the motion, then hand the click to
 *  the element under the pointer. Only outside the page column: the rail, the
 *  top bar and the panels don't move, but the spot clicked in the page was
 *  the old page's, whose content has already changed, so there the click
 *  only ends the motion. Only a click whose press also hit the overlay, in
 *  place (a drag or a selection ending there is no click on what is under
 *  it), and not a double-click's second click (that would open the row
 *  twice). A text field it lands on takes focus, as a real click gives. */
function catchOverlayClicks(doc: Doc) {
  if (catching.has(doc) || typeof doc.addEventListener !== 'function') return;
  catching.add(doc);
  let press: { x: number; y: number } | null = null;
  doc.addEventListener('pointerdown', (e: PointerEvent) => {
    press = e.isTrusted && e.target === doc.documentElement ? { x: e.clientX, y: e.clientY } : null;
  }, true);
  const onOverlay = (e: MouseEvent) => {
    const t = active;
    if (!t || !e.isTrusted || e.target !== doc.documentElement) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    const inPlace = !!press && Math.hypot(press.x - e.clientX, press.y - e.clientY) < 4;
    press = null;
    const replay = () => {
      // Another transition started meanwhile (⌘[ in the same frame): after it.
      if (active && active !== t) { active.finished.then(replay, replay); return; }
      const el = doc.elementFromPoint(e.clientX, e.clientY);
      const View = doc.defaultView?.MouseEvent;
      if (!el || el === doc.documentElement || !View || el.closest(PAGE_SELECTOR)) return;
      el.closest<HTMLElement>('input, textarea, select, [contenteditable="true"]')?.focus();
      el.dispatchEvent(new View(e.type, {
        bubbles: true, cancelable: true, composed: true, view: doc.defaultView, detail: e.detail,
        clientX: e.clientX, clientY: e.clientY, screenX: e.screenX, screenY: e.screenY,
        button: e.button, buttons: e.buttons, ctrlKey: e.ctrlKey, shiftKey: e.shiftKey, altKey: e.altKey, metaKey: e.metaKey,
      }));
    };
    try { t.skipTransition(); } catch { /* already over */ }
    if (inPlace && e.detail <= 1) t.finished.then(replay, replay);
  };
  doc.addEventListener('click', onOverlay, true);
  doc.addEventListener('contextmenu', onOverlay, true);
}

/** Run `update` (a navigation) inside a view transition when motion allows,
 *  else directly. Returns what `update` returns (its promise, for an async
 *  one). `settleMs`: for an async update, how long the new snapshot may wait
 *  for it, so a fast workspace load morphs into the loaded page. */
export function withViewTransition<T>(update: () => T, settleMs = 0, doc: Doc | undefined = pageDocument()): T {
  const source = morphSource;
  const target = morphTarget;
  morphSource = null;
  morphTarget = null;
  // An earlier navigation still waiting for the engine goes first.
  if (pending) pending();
  if (!doc || !canTransition(doc)) return update();
  let result!: T;
  let ran = false;
  let failed: { error: unknown } | null = null;
  const apply = () => {
    if (ran) return;
    ran = true;
    if (pending === flush) pending = null;
    try { result = update(); } catch (error) { failed = { error }; }
  };
  // Apply now, from outside the engine's callback: the motion is dropped.
  let transition: ViewTransition | null = null;
  const flush = () => {
    apply();
    try { transition?.skipTransition(); } catch { /* already over */ }
    restore();
    settled();
  };
  let settled: () => void = () => undefined;
  const page = doc.querySelector<HTMLElement>(PAGE_SELECTOR);
  const destination = target?.isConnected ? target : page;
  const names = new Map<HTMLElement, string>();
  for (const el of [source, destination, page]) if (el) names.set(el, el.style.viewTransitionName);
  // The morph source takes the page's name for the old snapshot; two
  // elements may never share one, so the page gives it up meanwhile.
  if (source && destination && source.isConnected) {
    if (page) page.style.viewTransitionName = 'none';
    destination.style.viewTransitionName = 'none';
    source.style.viewTransitionName = PAGE_NAME;
  }
  const restore = () => {
    for (const [el, name] of names) el.style.viewTransitionName = name;
  };
  running = true;
  // The page column carries its transition name only while one runs
  // (styles/motion.css), so it has no capture side effects at rest.
  const root = doc.documentElement;
  root.classList.add(TRANSITIONING);
  try {
    transition = doc.startViewTransition!(async () => {
      if (ran) return; // A timeout/newer choice already applied and cleaned up.
      restore();
      // A row-to-detail morph leaves the surrounding page stationary.
      if (destination && destination !== page) {
        if (page) page.style.viewTransitionName = 'none';
        destination.style.viewTransitionName = PAGE_NAME;
      }
      apply();
      if (settleMs > 0 && result instanceof Promise) await Promise.race([result.catch(() => undefined), wait(settleMs)]);
    });
  } catch {
    running = false;
    root.classList.remove(TRANSITIONING);
    restore();
    apply();
    if (failed) throw (failed as { error: unknown }).error;
    return result;
  }
  const started = transition;
  active = started;
  catchOverlayClicks(doc);
  const done = () => { running = false; if (active === started) active = null; if (pending === flush) pending = null; root.classList.remove(TRANSITIONING); restore(); };
  started.finished.then(done, done);
  started.ready.catch(() => { /* skipped: the update still ran */ });
  pending = flush;
  // Callers get the update's own result once it has run: a sync caller (a
  // page jump) ignores it, an async one (opening a workspace) awaits it.
  const out = new Promise<T>((resolve, reject) => {
    settled = () => {
      if (failed) reject((failed as { error: unknown }).error);
      else Promise.resolve(result).then(v => resolve(v as T), reject);
    };
  });
  out.catch(() => { /* the caller's to handle; never an unhandled rejection here */ });
  // The engine runs the update after it captures the old frame. If that
  // never happens (a throttled window), apply it now and drop the motion.
  const guard = setTimeout(() => { if (!ran) flush(); }, START_LIMIT_MS);
  const callbackDone = () => { clearTimeout(guard); settled(); };
  started.updateCallbackDone.then(callbackDone, callbackDone);
  return out as unknown as T;
}
