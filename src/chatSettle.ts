/**
 * Settling an opened conversation (10-12): opening a conversation, or coming
 * back to it, scrolls once (to its end, or to where the person was). Its
 * canvases and HTML frames draw as short placeholders first and reach their
 * real height a moment later. WebKit has no scroll anchoring, so that growth
 * above the view left the scroll position where it was, a canvas or two back
 * from the end ("it throws me up to an older canvas").
 *
 * While a settle is active, each size change puts the view back on its
 * target: the end, or the restored position. It ends once the content has
 * been quiet for `SETTLE_QUIET_MS` after it first grew (a cold frame may take a
 * while to report), after `SETTLE_MAX_MS` at most, or as soon as the person
 * does anything in the transcript or moves the view (ChatBody cancels it). The
 * rules are here so the node Vitest suite covers them; ChatBody supplies the
 * geometry and the size changes.
 */

/** A settle ends this long after the content last grew or shrank, in ms. */
export const SETTLE_QUIET_MS = 3000;
/** And this long after it started, whatever happens, in ms. */
export const SETTLE_MAX_MS = 10_000;

/** Where the view belongs: the end, or a restored scroll position. */
export type SettleTarget = { kind: 'bottom' } | { kind: 'top'; top: number };

/** What a settle reads and moves: the scroller. */
export interface SettleView {
  scrollTop(): number;
  setScrollTop(top: number): void;
  clientHeight(): number;
  scrollHeight(): number;
}

export function createSettle(view: SettleView, now: () => number = () => performance.now()) {
  let target: SettleTarget | null = null;
  let started = 0;
  let last = 0;
  // The content height last seen, and whether it has changed since the start.
  let height = 0;
  let changed = false;
  /** The scroll position for the target, within the scroll range. */
  function place(t: SettleTarget): number {
    const max = Math.max(0, view.scrollHeight() - view.clientHeight());
    return t.kind === 'bottom' ? max : Math.min(max, Math.max(0, t.top));
  }
  function apply() {
    if (!target) return;
    const top = place(target);
    if (Math.abs(top - view.scrollTop()) >= 1) view.setScrollTop(top);
  }
  function expired(at: number) { return at - started > SETTLE_MAX_MS || (changed && at - last > SETTLE_QUIET_MS); }
  return {
    /** A settle is on (and hasn't timed out). */
    active(): boolean {
      if (target && expired(now())) target = null;
      return target !== null;
    },
    /** Start (or restart) settling on `t`, and go there now. */
    start(t: SettleTarget) {
      target = t;
      started = last = now();
      height = view.scrollHeight();
      changed = false;
      apply();
    },
    /** Something changed size: back onto the target. False once the settle
     *  is over (timed out or cancelled); the caller stops watching. */
    resized(): boolean {
      if (!target) return false;
      const at = now();
      if (expired(at)) { target = null; return false; }
      const h = view.scrollHeight();
      if (h !== height) { height = h; changed = true; last = at; }
      apply();
      return true;
    },
    /** The person did something, or something else took the scroll. */
    cancel() { target = null; },
  };
}

export type Settle = ReturnType<typeof createSettle>;
