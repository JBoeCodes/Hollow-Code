/** Chart fills in an ordered dither where only alpha varies. One series is one
 *  theme colour, so light and dark need no second shade. Off cells keep 40% of
 *  the on alpha instead of leaving holes. Fills are dense at the baseline and
 *  thin toward the value, under a soft edge (one row near full alpha, one half
 *  row). Original code; the ideas come from Boring-Software-Inc/dither-kit
 *  (1e7faee), whose code was not copied. Variants by series slot: gradient,
 *  hatched, dotted, solid; all four thin toward the value (solid never has
 *  off cells). */
const BAYER = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
const CELL = 2;
const STEPS = 12;

/** Alpha of one cell: `d` is 0 at the value end and 1 at the base, `j` counts
 *  cells in from the value end (the first two form the soft edge). */
function cellAlpha(kind: number, cx: number, cy: number, d: number, j: number): number {
  const lit = kind === 0 ? d > (BAYER[(cy & 3) * 4 + (cx & 3)] + 0.5) / 16 : kind === 1 ? ((cx + cy) & 3) < 2 : kind === 2 ? !((cx | cy) & 1) : true;
  const on = kind === 3 ? 0.5 + 0.4 * d : 0.3 + 0.7 * d;
  return j < 2 ? 0.72 / (1 + j) : lit ? on : on * 0.4;
}

type Bins = Path2D[];
const newBins = (): Bins => Array.from({ length: STEPS + 1 }, () => new Path2D());
const addCell = (bins: Bins, a: number, gain: number, cx: number, cy: number) => bins[Math.round(a * gain * STEPS)].rect(cx * CELL, cy * CELL, CELL, CELL);
function flush(ctx: CanvasRenderingContext2D, colour: string, bins: Bins): void {
  ctx.save();
  ctx.clip();
  ctx.fillStyle = colour;
  bins.forEach((p, i) => { ctx.globalAlpha = i / STEPS; ctx.fill(p); });
  ctx.restore();
}

/** Fills the current path (clipped to it) with cells on a 2px lattice, from the
 *  value line `top(x)` to `floor`, bucketed by alpha so each bucket is one fill. */
export function ditherFill(ctx: CanvasRenderingContext2D, colour: string, slot: number, x0: number, x1: number, top: (x: number) => number, floor: number, gain = 1): void {
  const kind = slot % 4;
  const bins = newBins();
  for (let cx = Math.floor(x0 / CELL); cx * CELL < x1; cx++) {
    const t = top(cx * CELL + 1);
    if (t !== t) continue;
    const dir = t < floor ? 1 : -1;
    const depth = Math.abs(floor - t) || 1;
    const first = dir > 0 ? Math.floor(t / CELL) : Math.ceil(t / CELL) - 1;
    for (let j = 0; j < 999; j++) {
      const cy = first + dir * j;
      const mid = cy * CELL + 1;
      if ((mid - floor) * dir >= 1) break;
      addCell(bins, cellAlpha(kind, cx, cy, Math.min(1, Math.abs(mid - t) / depth), j), gain, cx, cy);
    }
  }
  flush(ctx, colour, bins);
}

/** The same fill for a donut sector (the current path): dense at the inner
 *  radius, thin toward the outer edge, which carries the soft edge rows. */
export function ditherRing(ctx: CanvasRenderingContext2D, colour: string, slot: number, cx: number, cy: number, r0: number, r1: number, gain = 1): void {
  const kind = slot % 4;
  const bins = newBins();
  const span = r1 - r0 || 1;
  for (let i = Math.floor((cx - r1) / CELL); i * CELL < cx + r1; i++) {
    for (let j = Math.floor((cy - r1) / CELL); j * CELL < cy + r1; j++) {
      const r = Math.hypot(i * CELL + 1 - cx, j * CELL + 1 - cy);
      if (r < r0 - 1 || r > r1 + 1) continue;
      addCell(bins, cellAlpha(kind, i, j, Math.min(1, Math.max(0, (r1 - r) / span)), Math.max(0, Math.floor((r1 - r) / CELL))), gain, i, j);
    }
  }
  flush(ctx, colour, bins);
}

/** The value line of a series, interpolated at `px`; NaN across a gap (a null
 *  value turns `y` into NaN). `px` only ever increases between calls. */
export function lineTop(vals: Array<number | null>, x: (i: number) => number, y: (v: number) => number): (px: number) => number {
  let i = 0;
  return px => {
    while (i < vals.length - 2 && x(i + 1) < px) i++;
    const a = y(vals[i] ?? NaN);
    return a + (y(vals[i + 1] ?? NaN) - a) * Math.min(1, Math.max(0, (px - x(i)) / (x(i + 1) - x(i))));
  };
}
