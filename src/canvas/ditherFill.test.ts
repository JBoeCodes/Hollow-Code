import { expect, it, vi } from 'vitest';
import { ditherFill, ditherRing, lineTop } from './kit/ditherFill';

/** A recording context: every Path2D remembers its cells by [col, row]. */
type Cells = Array<[number, number]>;
function run(slot: number, top: (x: number) => number, floor: number, x0 = 0, x1 = 40, gain = 1) {
  vi.stubGlobal('Path2D', class { cells: Cells = []; rect(x: number, y: number) { this.cells.push([x / 2, y / 2]); } });
  const fills: Array<{ alpha: number; cells: Cells }> = [];
  const ctx = { alpha: 1, fillStyle: '', save() {}, restore() {}, clip: vi.fn(), set globalAlpha(v: number) { this.alpha = v; }, fill(p: { cells: Cells }) { fills.push({ alpha: this.alpha, cells: p.cells }); } };
  ditherFill(ctx as unknown as CanvasRenderingContext2D, '#2a78d6', slot, x0, x1, top, floor, gain);
  vi.unstubAllGlobals();
  const alphaAt = new Map<string, number>();
  for (const f of fills) for (const [c, r] of f.cells) alphaAt.set(`${c},${r}`, f.alpha);
  return { fills, ctx, alphaAt };
}

it('covers every cell between the value line and the floor, never leaving a hole', () => {
  const { alphaAt } = run(0, () => 20, 60);
  // 20 columns, rows 10..29 (cell rows from y=20 to y=60).
  expect(alphaAt.size).toBe(20 * 20);
  for (const a of alphaAt.values()) expect(a).toBeGreaterThan(0.1);
});

it('clips once and is dense at the floor, thin at the value line, under a soft edge', () => {
  const { alphaAt, ctx } = run(0, () => 20, 60);
  expect(ctx.clip).toHaveBeenCalledTimes(1);
  const col = (c: number) => Array.from({ length: 20 }, (_, i) => alphaAt.get(`${c},${10 + i}`)!);
  const mean = (a: number[]) => a.reduce((x, y) => x + y, 0) / a.length;
  for (let c = 0; c < 4; c++) {
    const rows = col(c);
    expect(rows[0]).toBeCloseTo(0.72 * 1, 1);
    expect(rows[1]).toBeCloseTo(0.36, 1);
  }
  const all = Array.from({ length: 20 }, (_, c) => col(c));
  const top = mean(all.flatMap(c => c.slice(2, 7)));
  const bottom = mean(all.flatMap(c => c.slice(15)));
  expect(bottom).toBeGreaterThan(top + 0.2);
  // Lit cells carry the full tier, off cells 40% of it.
  const tiers = new Set(all.flatMap(c => c.slice(15)).map(a => Math.round(a * 12)));
  expect(tiers.size).toBeGreaterThan(1);
});

it('gives the four slots four different looks and wraps after four', () => {
  const sig = (slot: number) => JSON.stringify([...run(slot, () => 20, 60).alphaAt.entries()].sort());
  const looks = [0, 1, 2, 3].map(sig);
  expect(new Set(looks).size).toBe(4);
  expect(sig(4)).toBe(looks[0]);
});

it('solid has no off cells and every look thins toward the value line', () => {
  const rowMean = (slot: number, rows: number[]) => {
    const { alphaAt } = run(slot, () => 20, 60);
    const v = rows.flatMap(r => Array.from({ length: 20 }, (_, c) => alphaAt.get(`${c},${r}`)!));
    return v.reduce((a, b) => a + b, 0) / v.length;
  };
  const solid = run(3, () => 20, 60).alphaAt;
  expect(Math.min(...[...solid].filter(([k]) => Number(k.split(',')[1]) >= 12).map(([, a]) => a))).toBeGreaterThanOrEqual(0.5);
  for (const slot of [0, 1, 2, 3]) expect(rowMean(slot, [25, 26, 27, 28, 29])).toBeGreaterThan(rowMean(slot, [12, 13, 14, 15, 16]));
});

it('draws bars that hang below the baseline and thin toward their value', () => {
  // Floor at y=20 (cell row 10), value at y=60 (cell row 30): dense near row 10.
  const { alphaAt } = run(0, () => 60, 20);
  expect(alphaAt.size).toBe(20 * 20);
  const rowMean = (rows: number[]) => {
    const v = rows.flatMap(r => Array.from({ length: 20 }, (_, c) => alphaAt.get(`${c},${r}`)!));
    return v.reduce((a, b) => a + b, 0) / v.length;
  };
  expect(rowMean([10, 11, 12, 13, 14])).toBeGreaterThan(rowMean([24, 25, 26, 27, 28]));
});

it('a fractional floor still reaches the baseline with no gap row', () => {
  const { alphaAt } = run(0, () => 20, 60.9);
  for (let c = 0; c < 20; c++) expect(alphaAt.has(`${c},30`), `column ${c} reaches the row holding the floor`).toBe(true);
});

it('quiet gain scales alpha down and skips gaps instead of looping', () => {
  const full = run(1, () => 20, 60).alphaAt;
  const quiet = run(1, () => 20, 60, 0, 40, 0.6).alphaAt;
  expect(Math.max(...quiet.values())).toBeLessThan(Math.max(...full.values()));
  expect(run(0, () => NaN, 60).alphaAt.size).toBe(0);
  expect(run(0, () => 60, 60).alphaAt.size).toBeLessThan(40);
});

it('lineTop interpolates the value line and reports gaps as NaN', () => {
  const y = (v: number) => 100 - v;
  const top = lineTop([10, 30, null, 20], i => i * 20, y);
  expect(top(1)).toBeCloseTo(89, 5);
  expect(top(20)).toBe(70);
  expect(top(30)).toBeNaN();
  expect(top(50)).toBeNaN();
});

it('a shallow fill on a fractional baseline stays inside the alpha tiers', () => {
  for (const slot of [0, 1, 2, 3]) for (const floor of [60.05, 60.5, 60.95, 59.7]) {
    const { alphaAt } = run(slot, () => floor - 5, floor);
    expect(alphaAt.size).toBeGreaterThan(0);
    for (const a of alphaAt.values()) { expect(a).toBeGreaterThan(0); expect(a).toBeLessThanOrEqual(1); }
  }
  for (const floor of [60.05, 60.95]) expect(() => run(0, () => floor + 5, floor)).not.toThrow();
});

function ring(slot: number, gain = 1) {
  vi.stubGlobal('Path2D', class { cells: Cells = []; rect(x: number, y: number) { this.cells.push([x / 2, y / 2]); } });
  const fills: Array<{ alpha: number; cells: Cells }> = [];
  const ctx = { alpha: 1, save() {}, restore() {}, clip: vi.fn(), set globalAlpha(v: number) { this.alpha = v; }, fill(p: { cells: Cells }) { fills.push({ alpha: this.alpha, cells: p.cells }); } };
  ditherRing(ctx as unknown as CanvasRenderingContext2D, '#2a78d6', slot, 50, 50, 31, 50, gain);
  vi.unstubAllGlobals();
  const at = new Map<string, number>();
  for (const f of fills) for (const [c, r] of f.cells) at.set(`${c},${r}`, f.alpha);
  return { at, ctx };
}

it('fills a ring with no hole, dense at the inner radius and thin toward a soft outer edge', () => {
  const { at, ctx } = ring(0);
  expect(ctx.clip).toHaveBeenCalledTimes(1);
  for (const a of at.values()) expect(a).toBeGreaterThan(0.1);
  const radial = (lo: number, hi: number) => {
    const v = [...at].filter(([k]) => { const [c, r] = k.split(',').map(Number); const d = Math.hypot(c * 2 + 1 - 50, r * 2 + 1 - 50); return d >= lo && d < hi; }).map(([, a]) => a);
    return v.reduce((x, y) => x + y, 0) / v.length;
  };
  expect(radial(31, 38)).toBeGreaterThan(radial(41, 46) + 0.1);
  expect(radial(49, 51)).toBeLessThan(0.8);
});

it('gives the four slots four distinct ring looks and Quiet lowers them', () => {
  const sums = [0, 1, 2, 3].map(k => [...ring(k).at.values()].reduce((a, b) => a + b, 0));
  expect(new Set(sums.map(v => Math.round(v))).size).toBe(4);
  expect([...ring(0, 0.6).at.values()].reduce((a, b) => a + b, 0)).toBeLessThan(sums[0]);
  expect([...ring(3).at.values()].every(a => a > 0.3)).toBe(true);
});
