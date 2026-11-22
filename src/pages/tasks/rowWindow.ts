// The Tasks list draws a group's rows in steps (profiling night 11-20: 5,000
// tasks drew 4,550 rows at once, a 283 ms switch with long tasks on every
// cycle). A group shows ROW_STEP rows first and "Show more" adds ROW_STEP, or
// the scroll reaching the button does. The selected row and the one the keys
// move to are always inside the window, so j and k never land on a row that is
// not drawn.

/** Rows a group draws first, and how many each step adds. */
export const ROW_STEP = 60;

/** How many of a group's `total` rows draw, given the person's `limit` for it
 *  (null: the first step) and the index of the selected row in it (-1: none). */
export function rowsDrawn(total: number, limit: number | null, selectedIndex: number): number {
  const wanted = Math.max(limit ?? ROW_STEP, selectedIndex >= 0 ? selectedIndex + 1 : 0);
  return Math.max(0, Math.min(total, wanted));
}

/** The limit after one more step, from what is drawn now. */
export function nextLimit(drawn: number, total: number): number {
  return Math.min(total, drawn + ROW_STEP);
}
