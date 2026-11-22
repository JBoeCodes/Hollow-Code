import { afterEach, expect, it, vi } from 'vitest';

afterEach(() => vi.unstubAllGlobals());

async function paint(kind: string, texture = 'dither', allowed = true, values = [2, 5, null, 3] as Array<number | null>) {
  vi.stubGlobal('HTMLElement', class {});
  vi.stubGlobal('window', { hollowCanvas: { textureAllowed: allowed } });
  vi.stubGlobal('matchMedia', () => ({ matches: false }));
  vi.stubGlobal('getComputedStyle', () => ({ getPropertyValue: () => '', fontFamily: '' }));
  vi.stubGlobal('devicePixelRatio', 1);
  vi.stubGlobal('DOMMatrix', class { scale() { return this; } });
  const pattern = { setTransform() {} };
  const tile = { fillStyle: '', clearRect() {}, fillRect() {} };
  vi.stubGlobal('document', { documentElement: {}, createElement: () => ({ getContext: () => tile, toDataURL: () => 'data:image/png;base64,tile' }) });
  vi.stubGlobal('Path2D', class { cells = 0; rect() { this.cells++; } });
  type Op = { name: string; pattern?: boolean; cells?: number; path: unknown[][] };
  const ops: Op[] = [];
  let path: unknown[][] = [];
  const context: Record<string, unknown> = {
    measureText: (s: string) => ({ width: s.length * 5 }),
    createPattern: vi.fn(() => pattern),
    beginPath: () => { path = []; },
    fill: (cells?: { cells: number }) => ops.push({ name: 'fill', pattern: context.fillStyle === pattern, cells: cells?.cells, path: [...path] }),
    stroke: () => ops.push({ name: 'stroke', path: [...path] }),
    clip: () => ops.push({ name: 'clip', path: [...path] }),
    save() {}, restore() {}, fillText: vi.fn(), fillRect() {},
  };
  for (const name of ['moveTo', 'lineTo', 'arc', 'arcTo', 'closePath']) context[name] = (...args: unknown[]) => path.push([name, ...args]);
  const { HcChart } = await import('./kit/components/chart');
  const chart = new HcChart();
  Object.assign(chart, {
    _kind: kind, _active: 0,
    closest: () => ({ getAttribute: () => texture, matches: () => false }),
    _data: { categories: ['A', 'B', 'C', 'D'], series: [
      { name: 'One', values, cells: values.map(v => v === null ? '' : String(v)) },
      { name: 'Two', values: [4, 1, null, 6], cells: ['4', '1', '', '6'] },
    ] },
  });
  (chart as unknown as { _paint(c: unknown, w: number, h: number, layer: string): void })._paint(context, 500, 220, 'data');
  return { ops, context };
}

it.each(['line', 'area'])('%s paints all dither fills before any data stroke or marker', async kind => {
  const { ops } = await paint(kind);
  const firstStroke = ops.findIndex(op => op.name === 'stroke');
  const cells = ops.filter(op => op.cells);
  expect(cells.length).toBeGreaterThan(0);
  expect(ops.slice(firstStroke).some(op => op.cells)).toBe(false);
  // Opacity only: no pattern is ever the fill of a chart series.
  expect(ops.some(op => op.pattern)).toBe(false);
  // Each series is clipped to its exact closed area, one subpath per run.
  const clips = ops.filter(op => op.name === 'clip');
  expect(clips).toHaveLength(2);
  for (const op of clips) expect(op.path.filter(p => p[0] === 'closePath')).toHaveLength(2);
});

it('plain and dither charts paint no cells in plain, ascii or vetoed charts', async () => {
  for (const [kind, texture, allowed] of [['area', 'plain', true], ['line', 'ascii', true], ['bar', 'dither', false]] as const) {
    expect((await paint(kind, texture, allowed)).ops.some(op => op.cells !== undefined)).toBe(false);
  }
});

it('plain, ascii and host-vetoed charts allocate no pattern', async () => {
  for (const [kind, texture, allowed] of [['line', 'plain', true], ['area', 'ascii', true], ['bar', 'dither', false], ['donut', 'dither', false]] as const) {
    expect((await paint(kind, texture, allowed)).context.createPattern).not.toHaveBeenCalled();
  }
});

it('donut sectors are opacity-only cell fills clipped to the exact sector, with solid boundaries', async () => {
  const plain = (await paint('donut', 'plain')).ops.filter(op => op.name === 'fill');
  const textured = (await paint('donut')).ops;
  expect(textured.some(op => op.pattern)).toBe(false);
  // Per sector: the cell fill clips to the sector first, then the ink outline clips again.
  const sectors = textured.filter(op => op.name === 'clip').filter((_, i) => i % 2 === 0);
  expect(sectors.map(op => op.path)).toEqual(plain.map(op => op.path));
  expect(textured.filter(op => op.cells).length).toBeGreaterThan(plain.length);
  expect(textured.at(-1)?.name).toBe('stroke');
});

it.each([[null, null], [0, 0], [-2, 0]])('donut with %j keeps unknown and nonpositive data explicit', async (...values) => {
  const { context } = await paint('donut', 'dither', true, values);
  expect(context.fillText).toHaveBeenCalledWith(values.every(v => v === null) ? 'No data' : 'No positive values', 8, 20);
  expect(context.createPattern).not.toHaveBeenCalled();
});

it('bar dither fills are clipped to the exact solid column paths', async () => {
  const plain = (await paint('bar', 'plain')).ops.filter(op => op.name === 'fill');
  const { ops, context } = await paint('bar');
  // Per bar: one clip for the cells, one for the inner ink outline, both on the column path.
  const clips = ops.filter(op => op.name === 'clip');
  expect(clips).toHaveLength(plain.length * 2);
  expect(clips.filter((_, i) => i % 2 === 0).map(op => op.path)).toEqual(plain.map(op => op.path));
  expect(clips.filter((_, i) => i % 2 === 1).map(op => op.path)).toEqual(plain.map(op => op.path));
  expect(ops.filter(op => op.cells).length).toBeGreaterThanOrEqual(plain.length);
  expect(context.createPattern).not.toHaveBeenCalled();
});
