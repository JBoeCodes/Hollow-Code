/**
 * Two-finger trackpad swipe for Back and Forward (t3code parity 4.6, A13).
 * A horizontal swipe is a burst of wheel events with a dominant `deltaX`;
 * macOS keeps sending small ones for a while after the fingers lift. One
 * gesture therefore fires once, and the next can start only after a quiet gap.
 * Swiping right (content moves right) goes Back, swiping left goes Forward, as
 * in a browser. The Back and Forward buttons and ⌘[ ⌘] stay the visible routes.
 */
/** Accumulated horizontal distance (px) that counts as a swipe. */
export const SWIPE_DISTANCE = 140;
/** A pause this long ends a gesture. */
export const SWIPE_IDLE_MS = 240;
/** Horizontal must beat vertical by this much, so a diagonal scroll never navigates. */
export const SWIPE_DOMINANCE = 2.5;

export type SwipeStep = 'back' | 'forward' | null;

export function createSwipeDetector() {
  let at = -Infinity, dx = 0, dy = 0, spent = false;
  return {
    /** Feed one wheel event's deltas at time `t` (ms); the step to take, if this event completed a swipe. */
    feed(deltaX: number, deltaY: number, t: number): SwipeStep {
      if (t - at > SWIPE_IDLE_MS) { dx = 0; dy = 0; spent = false; }
      at = t;
      if (spent) return null;
      dx += deltaX; dy += deltaY;
      if (Math.abs(dy) > SWIPE_DISTANCE / 2 && Math.abs(dy) * SWIPE_DOMINANCE > Math.abs(dx)) { spent = true; return null; }
      if (Math.abs(dx) < SWIPE_DISTANCE || Math.abs(dx) < Math.abs(dy) * SWIPE_DOMINANCE) return null;
      spent = true;
      return dx < 0 ? 'back' : 'forward';
    },
  };
}

interface ScrollBox { scrollLeft: number; scrollWidth: number; clientWidth: number; parentElement: ScrollBox | null }
/** Whether something under the pointer would scroll sideways itself (a wide
 *  table, a code block, a carousel) in the swipe's direction: that swipe is its. */
export function scrollsSideways(
  from: ScrollBox | null,
  step: Exclude<SwipeStep, null>,
  overflowX: (el: ScrollBox) => string = el => getComputedStyle(el as unknown as Element).overflowX,
): boolean {
  for (let el = from; el; el = el.parentElement) {
    if (el.scrollWidth <= el.clientWidth + 1) continue;
    if (!/auto|scroll/.test(overflowX(el))) continue;
    // Back is a swipe right: it scrolls the box toward its start.
    const max = el.scrollWidth - el.clientWidth;
    if (step === 'back' ? el.scrollLeft > 0 : el.scrollLeft < max - 1) return true;
  }
  return false;
}

export interface HistoryWheelDeps {
  enabled: () => boolean;
  /** A dialog is open or a shortcut is being recorded: the swipe is not ours. */
  blocked: () => boolean;
  back: () => void;
  forward: () => void;
}

/** The document's wheel listener (App.tsx builds it with the store's steps). */
export function createHistoryWheel(deps: HistoryWheelDeps) {
  const detector = createSwipeDetector();
  return (e: WheelEvent) => {
    if (!deps.enabled() || e.ctrlKey || e.deltaMode !== 0 || e.deltaX === 0) return;
    const step = detector.feed(e.deltaX, e.deltaY, e.timeStamp);
    if (!step || deps.blocked()) return;
    const target = e.target as Element | null;
    if (target?.closest?.('input, textarea, [contenteditable="true"], .cm-editor')) return;
    if (scrollsSideways(target as unknown as ScrollBox | null, step)) return;
    if (step === 'back') deps.back(); else deps.forward();
  };
}
