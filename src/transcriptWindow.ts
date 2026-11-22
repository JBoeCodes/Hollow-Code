// The transcript's window of drawn turns, both ends (parity 0.3, profiling night
// 11-20). The latest TURN_WINDOW turns draw first; "Show earlier" (or the scroll
// reaching it) moves the start up; once the person is reading back, the end of
// the window gives way ("Show later", or the scroll reaching it), so the drawn turns
// stay within TURN_CAP however far they read. Measured before this: loading a
// 1,000-turn thread back to its start drew 21,132 DOM nodes, held +59 MB of heap and
// made 123 key presses stall (`profile.md`); the budget it is held to is
// `scripts/perf-budgets.mjs`. Pure arithmetic, so it is tested without a DOM.

/** The most turns drawn while reading back: three steps of the window, and a little more for the
 *  turns on screen. */
export const TURN_CAP = 200;
/** How many turns one press of Show earlier or Show later adds. */
export const LOAD_STEP = 80;
/** How many turns one load adds when the scroll reaches an end by itself. Drawing 80 turns takes about
 *  75 ms (profile.md), a visible stall mid-scroll; 20 take about a frame, and the loader runs again
 *  as the scroll goes on. */
export const AUTO_STEP = 20;

export interface TurnWindow {
  /** First drawn turn. */
  from: number;
  /** One past the last drawn turn; `total` when the window reaches the end. */
  to: number;
}

/** The window the end of the thread gives way to, or `null` when it need not: after the start moved
 *  up, the end is cut so no more than TURN_CAP turns draw, keeping every turn up to `lastVisible`
 *  (and half a step past it) and never cutting while the person is at the end. */
export function trimEnd(w: TurnWindow, total: number, lastVisible: number | null, atEnd: boolean): number | null {
  if (atEnd || w.to - w.from <= TURN_CAP) return null;
  const keep = lastVisible === null ? 0 : lastVisible + LOAD_STEP / 2;
  const to = Math.min(total, Math.max(w.from + TURN_CAP, keep));
  return to < w.to ? to : null;
}

/** The start the window moves to when its end grew past TURN_CAP: the oldest turns are dropped to
 *  bring it back, but never one on screen (nor half a step above it). `null` when nothing can go. */
export function trimStart(w: TurnWindow, firstVisible: number | null): number | null {
  if (firstVisible === null || w.to - w.from <= TURN_CAP) return null;
  const from = Math.min(w.to - TURN_CAP, firstVisible - LOAD_STEP / 2);
  return from > w.from ? from : null;
}

/** "Show later": the end moves down one step. */
export function laterEnd(to: number, total: number, step = LOAD_STEP): number {
  return Math.min(total, to + step);
}

/** The window that holds turn `idx` (and, if given, `need`, the first turn a fold needs): a jump to a
 *  turn outside it recenters instead of loading everything between. Kept to TURN_CAP by dropping the
 *  end farther from `idx`. `from` never passes `idx - 4`, as `windowStart` did. */
export function windowFor(w: TurnWindow, total: number, idx: number, need = idx): TurnWindow {
  let from = w.from;
  let to = w.to;
  const first = Math.min(idx, need);
  if (first < from) from = Math.max(0, first - 4);
  if (idx >= to) to = Math.min(total, idx + LOAD_STEP / 2);
  if (to - from > TURN_CAP) {
    if (idx - from > to - 1 - idx) from = Math.max(0, Math.min(first - 4, to - TURN_CAP));
    else to = Math.min(total, from + TURN_CAP);
  }
  return { from, to };
}
