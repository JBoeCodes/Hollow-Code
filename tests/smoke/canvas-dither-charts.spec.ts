import { test, expect, type Page } from './fixtures/test';
import { readFileSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { execFileSync } from 'node:child_process';

// Chart fills in dither: opacity-only, faint off cells, fade toward the value,
// per-series variants. DITHER_PHASE=before reads the kit from DITHER_BASE
// (a commit) for the before captures; DITHER_CAPTURES is the output folder.
const base = process.env.DITHER_BASE || 'origin/main';
const read = (p: string) => process.env.DITHER_PHASE === 'before'
  ? execFileSync('git', ['show', `${base}:${p}`], { encoding: 'utf8' }) : readFileSync(resolve(p), 'utf8');
const rows = (names: string[], data: number[][]) => data.map((r, i) => `${names[i]}: ${r.join(', ')}`).join('\n');
const sample = `<h1>Charts</h1><div class="grid">
<hc-chart id="bar" type="bar" height="190" label="Items by team" series="Design, Build, Review, Ship">${rows(['Avery', 'Morgan', 'Riley', 'Jordan'], [[12, 8, 5, 3], [3, 9, 7, 2], [7, 4, 9, 6], [10, 6, 3, 8]])}</hc-chart>
<hc-chart id="area" type="area" height="190" label="Queue" series="Open, Review, Done">${rows(['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'], [[3, 2, 1], [7, 4, 3], [4, 6, 5], [9, 5, 8], [6, 7, 9], [8, 4, 10]])}</hc-chart>
<hc-chart id="same" type="bar" height="190" label="Same" series="A, B, C, D">Now: 10, 10, 10, 10</hc-chart>
<hc-chart id="line" type="line" height="190" label="Completed" series="Claude, Codex">${rows(['Mon', 'Tue', 'Wed', 'Thu', 'Fri'], [[12, 8], [15, 10], [11, 13], [18, 12], [16, 15]])}</hc-chart>
<hc-chart id="one" type="area" height="190" label="One series">Mon: 3\nTue: 7\nWed: 4\nThu: 9\nFri: 6</hc-chart>
<hc-chart id="donut" type="donut" height="190" label="Time">Design: 12\nBuild: 30\nReview: 7</hc-chart>
<div><hc-bars id="bars" label="Hours" unit="h">Design: 12\nBuild: 30\nReview: 7</hc-bars><hc-sparkline id="spark" label="Trend">Mon: 12\nTue: 15\nWed: 11\nThu: 18\nFri: 14</hc-sparkline></div></div>`;

/** Mounts the kit with a chart fixture the way a canvas does. */
async function open(page: Page, theme: 'light' | 'dark', content = sample): Promise<void> {
  await page.setViewportSize({ width: 1200, height: 900 });
  await page.emulateMedia({ reducedMotion: 'no-preference', colorScheme: theme });
  const styles = read('src/styles/tokens.css');
  const aliases = [...read('src/components/htmlSandbox.ts').matchAll(/\['(--app-[\w-]+)', '(--[\w-]+)'/g)].map(m => `${m[1]}:var(${m[2]})`).join(';');
  const kit = read('src/canvas/kit/generated/kit.min.js'), css = read('src/canvas/kit/generated/kit.min.css');
  const html = `<html data-theme="${theme}"><head><base href="http://127.0.0.1:1420/"><style>${styles}\n:root{${aliases}}body{margin:0;padding:24px;font:15px/1.55 system-ui;background:var(--surface-card);color:var(--ink)}h1{font-size:26px;margin:0 0 16px}.grid{display:grid;grid-template-columns:1fr 1fr;gap:20px}.grid>*{min-width:0}</style><style>${css}</style>
  <script>window.__allowed=true;window.__level='full';window.hollowCanvas=Object.freeze({init:{},get textureAllowed(){return window.__allowed},get textureLevel(){return window.__level},get(){},set(){return true}});</script>
  <script>${kit}</script></head><body>${content}</body></html>`;
  await page.setContent(html);
  await expect(page.locator('hc-chart').first()).toHaveAttribute('data-hc-ready', '');
  await page.evaluate(() => {
    document.documentElement.dataset.hcMotion = 'reduce';
    dispatchEvent(new Event('hc-reflow')); dispatchEvent(new Event('resize'));
    return new Promise<void>(r => requestAnimationFrame(() => requestAnimationFrame(() => r())));
  });
}

for (const theme of ['light', 'dark'] as const) {
  test(`chart fills in ${theme}`, async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', e => errors.push(e.message));
    await open(page, theme);
    await expect(page.locator('.hc-chart-entrance')).toHaveCount(0);
    const folder = process.env.DITHER_CAPTURES;
    if (folder) {
      mkdirSync(folder, { recursive: true });
      const shots: Record<string, string> = { bar: '#bar', area: '#area', line: '#line', donut: '#donut', bars: '#bars', spark: '#spark', legend: '#bar .hc-chart-legend', 'donut-legend': '#donut .hc-chart-legend' };
      const shoot = async (suffix: string, ids: string[]) => { for (const id of ids) await page.locator(shots[id]).screenshot({ path: resolve(folder, `${process.env.DITHER_PHASE || 'after'}-${id}-${theme}${suffix}.png`) }); };
      await shoot('', Object.keys(shots));
      // Quiet captures of the same surfaces.
      await page.evaluate(() => { (window as unknown as { __level: string }).__level = 'quiet'; dispatchEvent(new Event('hollow-texture')); });
      await page.waitForFunction(() => new Promise<boolean>(r => requestAnimationFrame(() => requestAnimationFrame(() => r(true)))));
      await shoot('-quiet', ['donut', 'bars', 'spark', 'legend', 'donut-legend']);
      await page.evaluate(() => { (window as unknown as { __level: string }).__level = 'full'; dispatchEvent(new Event('hollow-texture')); });
      await page.waitForFunction(() => new Promise<boolean>(r => requestAnimationFrame(() => requestAnimationFrame(() => r(true)))));
    }
    if (process.env.DITHER_PHASE === 'before') return;

    /** RGBA pixels of a chart canvas, by CSS column, for one vertical slice. */
    const column = (id: string, xFrac: number) => page.evaluate(({ id, xFrac }) => {
      const cv = document.querySelector<HTMLCanvasElement>(`#${id} canvas`)!;
      const ctx = cv.getContext('2d')!;
      const x = Math.round(cv.width * xFrac);
      return { height: cv.height, data: Array.from(ctx.getImageData(x, 0, 1, cv.height).data), dpr: cv.width / cv.clientWidth };
    }, { id, xFrac });
    const tone = (id: string) => page.evaluate(id => {
      const cv = document.querySelector<HTMLCanvasElement>(`#${id} canvas`)!;
      const d = cv.getContext('2d')!.getImageData(0, 0, cv.width, cv.height).data;
      let a = 0, solid = 0;
      for (let i = 3; i < d.length; i += 4) { a += d[i]; if (d[i] === 255) solid++; }
      return { alpha: a / (d.length / 4), solid };
    }, id);

    // 1 + 2: a series is one colour, off cells stay tinted: scanning an area's
    // fill from just under its value line to just above the baseline never
    // meets a transparent hole.
    // The single-series area, at its middle: rows with series colour (chroma,
    // which the grey grid and zero line lack) run from its value line to the floor.
    const area = await column('one', 0.5);
    const tinted = (y: number) => { const [r, g, b, a] = area.data.slice(y * 4, y * 4 + 4); return a > 0 && Math.max(r, g, b) - Math.min(r, g, b) > 40; };
    let first = 0;
    while (first < area.height && !tinted(first)) first++;
    let floorRow = area.height - 1;
    while (floorRow > 0 && !tinted(floorRow)) floorRow--;
    const holes: number[] = [];
    for (let y = first + 12; y < floorRow - 6; y++) if (!tinted(y) && area.data[y * 4 + 3] === 0) holes.push(y);
    expect(floorRow - first, 'the fill spans the plot').toBeGreaterThan(40);
    expect(holes, 'no transparent holes between the value line and the baseline').toEqual([]);

    // 3: dense at the floor, thin toward the value.
    const mean = (rowsFrom: number, rowsTo: number) => { let s = 0; for (let y = rowsFrom; y < rowsTo; y++) s += area.data[y * 4 + 3]; return s / Math.max(1, rowsTo - rowsFrom); };
    const span = floorRow - first;
    expect(mean(first + Math.round(span * 0.7), floorRow)).toBeGreaterThan(mean(first + 8, first + Math.round(span * 0.3)));

    // 4: four equal bars (one per series slot) carry four different fills: each
    // bar's interior alpha mean and share of near-solid cells differ pairwise.
    const looks = await page.evaluate(() => {
      const cv = document.querySelector<HTMLCanvasElement>('#same canvas')!;
      const w = cv.width, h = cv.height, d = cv.getContext('2d')!.getImageData(0, 0, w, h).data;
      const dpr = w / cv.clientWidth;
      const at = (x: number, y: number) => d.slice((y * w + x) * 4, (y * w + x) * 4 + 4);
      const tinted = (x: number, y: number) => { const [r, g, b, a] = at(x, y); return a > 0 && Math.max(r, g, b) - Math.min(r, g, b) > 40; };
      const probeY = Math.round(h - 34 * dpr);
      const runs: Array<[number, number]> = [];
      for (let x = 0, start = -1; x <= w; x++) {
        const on = x < w && tinted(x, probeY);
        if (on && start < 0) start = x;
        if (!on && start >= 0) { if (x - start > 6 * dpr) runs.push([start, x]); start = -1; }
      }
      let top = 0;
      while (top < h && !tinted(Math.round((runs[0][0] + runs[0][1]) / 2), top)) top++;
      return runs.slice(0, 4).map(([x0, x1]) => {
        let sum = 0, strong = 0, n = 0;
        for (let y = top + Math.round(10 * dpr); y < probeY + 10; y++) for (let x = x0 + Math.round(3 * dpr); x < x1 - Math.round(3 * dpr); x++) {
          const a = at(x, y)[3]; sum += a; n++; if (a > 150) strong++;
        }
        return { mean: sum / n / 255, strong: strong / n };
      });
    });
    expect(looks).toHaveLength(4);
    for (let i = 0; i < 4; i++) for (let j = i + 1; j < 4; j++) {
      expect(Math.abs(looks[i].mean - looks[j].mean) + Math.abs(looks[i].strong - looks[j].strong), `bars ${i} and ${j} differ`).toBeGreaterThan(0.03);
    }
    for (const l of looks) expect(l.mean, 'every bar cell is tinted, never a hole').toBeGreaterThan(0.1);
    const multi = await tone('bar');
    expect(multi.alpha).toBeGreaterThan(0);

    // Texture off: the same chart repaints with the plain solid fill.
    const dither = await tone('bar');
    await page.evaluate(() => { (window as unknown as { __allowed: boolean }).__allowed = false; dispatchEvent(new Event('hollow-texture')); });
    await expect(page.locator('#bar')).toHaveAttribute('data-hc-ready', '');
    await page.waitForFunction(() => new Promise<boolean>(r => requestAnimationFrame(() => requestAnimationFrame(() => r(true)))));
    const plain = await tone('bar');
    expect(plain.solid, 'plain bars are solid fills').toBeGreaterThan(dither.solid);
    expect(await page.locator('#spark .hc-fade-area').evaluate(el => getComputedStyle(el).display)).toBe('none');
    await page.evaluate(() => { (window as unknown as { __allowed: boolean }).__allowed = true; dispatchEvent(new Event('hollow-texture')); });
    await page.waitForFunction(() => new Promise<boolean>(r => requestAnimationFrame(() => requestAnimationFrame(() => r(true)))));

    // Quiet lowers every chart's alpha; full restores it.
    const full = await tone('area');
    await page.evaluate(() => { (window as unknown as { __level: string }).__level = 'quiet'; dispatchEvent(new Event('hollow-texture')); });
    await page.waitForFunction(() => new Promise<boolean>(r => requestAnimationFrame(() => requestAnimationFrame(() => r(true)))));
    const quiet = await tone('area');
    expect(quiet.alpha).toBeLessThan(full.alpha);
    await page.evaluate(() => { (window as unknown as { __level: string }).__level = 'full'; dispatchEvent(new Event('hollow-texture')); });

    // Sparkline and bars fade by opacity: a gradient fill on the sparkline,
    // gradient background (no surface-coloured holes) on the bars.
    expect(await page.locator('#spark .hc-fade-area').evaluate(el => getComputedStyle(el).display)).toBe('block');
    expect(await page.locator('#bars .hc-bar-fill').first().evaluate(el => getComputedStyle(el).backgroundImage)).toContain('linear-gradient');
    // The stipple over the bar is the series colour, never the surface (no white specks).
    expect(await page.locator('#bars .hc-bar-fill').first().evaluate(el => {
      const probe = document.createElement('span'); probe.style.color = 'var(--hc-s1)'; el.append(probe);
      const series = getComputedStyle(probe).color; probe.remove();
      return getComputedStyle(el, '::after').backgroundColor === series;
    })).toBe(true);
    expect(errors).toEqual([]);
  });
}

/** A wide chart with many points: the worst plausible hover target. */
const wide = (type: string) => `<div style="width:1400px"><hc-chart id="w" type="${type}" height="400" label="Wide" series="A, B, C, D">${Array.from({ length: 60 }, (_, i) => `D${i + 1}: ${[0, 1, 2, 3].map(k => 20 + ((i * (k + 3) * 7) % 40) + k * 9).join(', ')}`).join('\n')}</hc-chart></div>`;
/** Moves the pointer across the plot, one category per event; returns each event's cost in ms. */
const hoverCosts = (page: Page, events = 120) => page.evaluate(n => {
  const cv = document.querySelector<HTMLCanvasElement>('#w canvas')!;
  const r = cv.getBoundingClientRect();
  const costs: number[] = [];
  for (let i = 0; i < n; i++) {
    const t = performance.now();
    cv.dispatchEvent(new MouseEvent('mousemove', { clientX: r.left + 40 + ((i * 13) % 60) * ((r.width - 80) / 60), clientY: r.top + 100, bubbles: true }));
    costs.push(performance.now() - t);
  }
  return costs;
}, events);
const median = (a: number[]) => [...a].sort((x, y) => x - y)[Math.floor(a.length / 2)];

test('hover redraw stays cheap on the worst plausible chart', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 });
  const report: Record<string, number> = {};
  for (const type of ['bar', 'area', 'line']) {
    await open(page, 'light', wide(type));
    await page.setViewportSize({ width: 1500, height: 900 });
    await page.evaluate(() => { dispatchEvent(new Event('resize')); return new Promise<void>(r => requestAnimationFrame(() => requestAnimationFrame(() => r()))); });
    expect(await page.locator('#w canvas').evaluate(cv => (cv as HTMLCanvasElement).clientWidth)).toBeGreaterThan(1300);
    const per = Number(process.env.DITHER_HOVER_N || 60);
    await hoverCosts(page, Math.min(20, per));
    // Timers are coarse (WebKit rounds to a millisecond), so time batches of 60 events and divide.
    const batches: number[] = [];
    for (let b = 0; b < (per === 60 ? 9 : 3); b++) batches.push((await hoverCosts(page, per)).reduce((a, c) => a + c, 0) / per);
    report[type] = Math.round(median(batches) * 1000) / 1000;
  }
  console.log(`HOVER_MS ${process.env.DITHER_PHASE || 'after'} ${JSON.stringify(report)}`);
  if (process.env.DITHER_PHASE !== 'before') for (const [type, ms] of Object.entries(report)) expect(ms, `${type} hover median`).toBeLessThan(4);
});

const settle = (page: Page) => page.waitForFunction(() => new Promise<boolean>(r => requestAnimationFrame(() => requestAnimationFrame(() => r(true)))));
const setLevel = async (page: Page, level: 'full' | 'quiet') => {
  await page.evaluate(l => { (window as unknown as { __level: string }).__level = l; dispatchEvent(new Event('hollow-texture')); }, level);
  await settle(page);
};

test('a hover repaints without rebuilding the dithered fills, and a changed colour or Quiet still rebuilds them', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 900 });
  for (const type of ['bar', 'area', 'line', 'donut']) {
    await open(page, 'light', wide(type));
    await page.evaluate(() => {
      const w = window as unknown as { __made: number };
      w.__made = 0;
      const Native = Path2D;
      // Every fill rebuild constructs Path2D bins; a cached hover constructs none (a donut redraws only its one grown sector).
      (window as unknown as { Path2D: unknown }).Path2D = class extends Native { constructor(d?: string | Path2D) { super(d as string); w.__made++; } };
    });
    const made = () => page.evaluate(() => (window as unknown as { __made: number }).__made);
    await hoverCosts(page, 40);
    if (type === 'donut') expect(await made(), 'donut hover rebuilds at most the active sector').toBeLessThanOrEqual(40 * 13);
    else expect(await made(), `${type} hover rebuilds no fills`).toBe(0);

    // A series colour change reaches the cached layer on the next hover (the key holds the colours).
    if (type === 'donut') continue;
    const red = () => page.evaluate(() => {
      const cv = document.querySelector<HTMLCanvasElement>('#w canvas')!;
      const d = cv.getContext('2d')!.getImageData(0, 0, cv.width, cv.height).data;
      let n = 0;
      for (let i = 0; i < d.length; i += 4) if (d[i] > 2 * d[i + 1] + 20 && d[i] > 2 * d[i + 2] + 20 && d[i + 3] > 30) n++;
      return n;
    });
    const before = await red();
    await page.locator('#w').evaluate(el => (el as HTMLElement).style.setProperty('--hc-s1', '#ff0000'));
    await hoverCosts(page, 2);
    expect(await red(), `${type} hover shows the new series colour`).toBeGreaterThan(before + 500);
    // Quiet rebuilds too: it lowers the alpha of what is cached.
    const alpha = () => page.evaluate(() => {
      const cv = document.querySelector<HTMLCanvasElement>('#w canvas')!;
      const d = cv.getContext('2d')!.getImageData(0, 0, cv.width, cv.height).data;
      let a = 0; for (let i = 3; i < d.length; i += 4) a += d[i];
      return a;
    });
    const full = await alpha();
    await setLevel(page, 'quiet');
    await hoverCosts(page, 2);
    expect(await alpha(), `${type} Quiet lowers the cached fills`).toBeLessThan(full);
  }
});

for (const theme of ['light', 'dark'] as const) {
  test(`donut sectors use the opacity-only fill in ${theme}`, async ({ page }) => {
    await open(page, theme, `<div style="width:420px"><hc-chart id="d" type="donut" height="240" label="Share">A: 25\nB: 25\nC: 25\nD: 25</hc-chart></div>`);
    await page.waitForFunction(() => !document.querySelector('.hc-chart-entrance'));
    const sample = () => page.evaluate(() => {
      const cv = document.querySelector<HTMLCanvasElement>('#d canvas')!;
      const dpr = cv.width / cv.clientWidth, cx = cv.width / 2, cy = cv.height / 2;
      const r1 = (Math.min(cv.clientWidth, cv.clientHeight) / 2 - 6) * dpr, r0 = r1 * 0.62;
      const d = cv.getContext('2d')!.getImageData(0, 0, cv.width, cv.height).data;
      const quads = [0, 0, 0, 0], cells = [0, 0, 0, 0], tiers: Set<number>[] = [new Set(), new Set(), new Set(), new Set()];
      let holes = 0, ring = 0;
      for (let y = 0; y < cv.height; y++) for (let x = 0; x < cv.width; x++) {
        const dx = x - cx, dy = y - cy, r = Math.hypot(dx, dy);
        if (r < r0 + 6 * dpr || r > r1 - 6 * dpr) continue;
        // Away from the 2px gaps between sectors.
        if (Math.abs(dx) < 4 * dpr || Math.abs(dy) < 4 * dpr) continue;
        const q = dx > 0 ? (dy < 0 ? 0 : 1) : (dy > 0 ? 2 : 3);
        const a = d[(y * cv.width + x) * 4 + 3];
        ring++; if (a === 0) holes++;
        quads[q] += a; cells[q]++; tiers[q].add(Math.round(a / 20));
      }
      return { means: quads.map((v, i) => Math.round(v / cells[i])), tiers: tiers.map(t => t.size), holes, ring };
    });
    const full = await sample();
    expect(full.holes, 'faint off cells leave no transparent hole').toBe(0);
    expect(new Set(full.means).size, 'four distinct looks').toBe(4);
    for (const t of full.tiers.slice(0, 3)) expect(t, 'several opacity tiers, not one dot colour').toBeGreaterThan(2);
    await setLevel(page, 'quiet');
    const quiet = await sample();
    expect(quiet.means.reduce((a, b) => a + b), 'Quiet lowers the donut alpha').toBeLessThan(full.means.reduce((a, b) => a + b));
  });

  test(`a hovered donut sector is drawn once, with no ink ring left from the cached sector in ${theme}`, async ({ page }) => {
    await open(page, theme, `<div style="width:420px"><hc-chart id="d" type="donut" height="240" label="Share">A: 25\nB: 25\nC: 25\nD: 25</hc-chart></div>`);
    await page.waitForFunction(() => !document.querySelector('.hc-chart-entrance'));
    // Mean distance from the ink colour of a thin band just inside a sector's resting outer edge, where its 2px ink outline sits.
    const band = (lo: number, hi: number) => page.evaluate(([lo, hi]) => {
      const cv = document.querySelector<HTMLCanvasElement>('#d canvas')!;
      const dpr = cv.width / cv.clientWidth, cx = cv.width / 2, cy = cv.height / 2;
      const r1 = (Math.min(cv.clientWidth, cv.clientHeight) / 2 - 6) * dpr;
      const d = cv.getContext('2d')!.getImageData(0, 0, cv.width, cv.height).data;
      const probe = document.createElement('i'); probe.style.color = 'var(--hc-ink)'; document.querySelector('#d')!.append(probe);
      const [ir, ig, ib] = getComputedStyle(probe).color.match(/\d+/g)!.map(Number); probe.remove();
      let n = 0, sum = 0;
      for (let a = 20; a <= 70; a += 0.5) for (let r = r1 - hi * dpr; r <= r1 - lo * dpr; r += 0.5) {
        const x = Math.round(cx + Math.sin(a * Math.PI / 180) * r), y = Math.round(cy - Math.cos(a * Math.PI / 180) * r);
        const i = (y * cv.width + x) * 4;
        n++; sum += (Math.abs(d[i] - ir) + Math.abs(d[i + 1] - ig) + Math.abs(d[i + 2] - ib));
      }
      return sum / n;
    }, [lo, hi]);
    const edge = () => band(0.5, 2);
    const rest = await edge();
    await page.locator('#d canvas').evaluate(cv => {
      const r = cv.getBoundingClientRect(), cx = r.width / 2, cy = r.height / 2;
      cv.dispatchEvent(new MouseEvent('mousemove', { clientX: r.left + cx + 90 * Math.SQRT1_2, clientY: r.top + cy - 90 * Math.SQRT1_2, bubbles: true }));
    });
    // The hovered sector grew 4px: its old edge radius is now interior, so ink there is a ghost of the cached sector.
    expect(await edge(), 'no ghost ink ring inside the grown sector').toBeGreaterThan(rest + 60);
  });

  test(`legend swatches show each series' look in ${theme}`, async ({ page }) => {
    await open(page, theme);
    const swatches = (sel: string) => page.locator(`${sel} .hc-chart-legend .hc-sw`).evaluateAll(els => els.map(el => {
      const b = getComputedStyle(el, '::before'), s = getComputedStyle(el);
      const probe = (c: string) => { const e = document.createElement('i'); e.style.color = c; document.body.append(e); const v = getComputedStyle(e).color; e.remove(); return v; };
      const lum = (rgb: string) => { const m = rgb.match(/[\d.]+/g)!.slice(0, 3).map(Number).map(v => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }); return 0.2126 * m[0] + 0.7152 * m[1] + 0.0722 * m[2]; };
      const ink = probe('var(--hc-ink)'), surface = probe('var(--hc-surface)');
      const [hi, lo] = [lum(ink), lum(surface)].sort((a, b) => b - a);
      return { look: (el as HTMLElement).dataset.look, content: b.content, width: b.width, height: b.height, image: b.backgroundImage, fontSize: s.fontSize, edge: (hi + 0.05) / (lo + 0.05) };
    }));
    const bar = await swatches('#bar');
    expect(bar.map(s => s.look)).toEqual(['0', '1', '2', '3']);
    expect(bar.map(s => s.image.replace(/\(.*$/s, ''))).toEqual(['linear-gradient', 'repeating-linear-gradient', 'radial-gradient', 'none']);
    for (const s of bar) {
      expect(s.content).toBe('""');
      expect(parseFloat(s.width), 'readable size').toBeGreaterThanOrEqual(12);
      expect(parseFloat(s.height)).toBeGreaterThanOrEqual(8);
      expect(parseFloat(s.height)).toBeLessThanOrEqual(12);
      expect(s.fontSize, 'a fill chart shows the look alone').toBe('0px');
      expect(s.edge, 'swatch outline against the surface, icon contrast').toBeGreaterThanOrEqual(3);
    }
    const area = await swatches('#area');
    expect(area.map(s => s.look)).toEqual(['0', '1', '2']);
    // A line keeps its marker glyph beside the look.
    for (const s of await swatches('#line')) { expect(s.fontSize).not.toBe('0px'); expect(s.content).toBe('""'); }
    const donut = await swatches('#donut');
    expect(donut.map(s => s.look)).toEqual(['0', '1', '2']);
    // Tooltip rows use the same swatch.
    await page.locator('#bar canvas').hover({ position: { x: 120, y: 80 } });
    expect(await page.locator('#bar .hc-tip-row .hc-sw[data-look]').count()).toBe(4);
    // A single graphic opted out keeps the glyph, with no pattern box.
    await page.locator('#bar').evaluate(el => el.setAttribute('texture', 'plain'));
    await settle(page);
    const plain = await swatches('#bar');
    expect(plain.every(s => s.content === 'none' || s.content === 'normal')).toBe(true);
    expect(plain.every(s => s.fontSize !== '0px')).toBe(true);
  });
}

test('the bars and sparkline fades follow Quiet', async ({ page }) => {
  await open(page, 'light');
  const read = () => page.evaluate(() => ({
    bar: getComputedStyle(document.querySelector('#bars .hc-bar-fill')!).backgroundImage,
    shadow: getComputedStyle(document.querySelector('#bars .hc-bar-fill')!).boxShadow,
    spark: Number(getComputedStyle(document.querySelector('#spark .hc-fade-area')!).opacity),
  }));
  const full = await read();
  expect(full.spark).toBe(1);
  await setLevel(page, 'quiet');
  const quiet = await read();
  expect(quiet.spark).toBeCloseTo(0.6, 2);
  expect(quiet.bar).not.toBe(full.bar);
  expect(quiet.shadow).not.toBe(full.shadow);
  await setLevel(page, 'full');
  expect(await read()).toEqual(full);
});
