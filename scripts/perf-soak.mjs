#!/usr/bin/env node
// Overnight 10-11/12 P6: a memory soak in the browser mock on the scale
// fixture (`?mock=1&demo=large`). Each cycle opens the 60-turn chat with 10
// canvases and scrolls it end to end (canvas frames mount and unmount), visits
// two workspaces, Tasks and Work, and opens and closes the palette. After
// every cycle it collects garbage and samples the page's JS heap, its DOM
// counters (documents, nodes, event listeners) and the frames Playwright sees
// (canvas frames are out of process, so the heap doesn't hold them). Needs a
// `build:mock` served by one `vite preview`:
//   BASE=http://127.0.0.1:1459 SOAK_MIN=30 node scripts/perf-soak.mjs [--out f.json]
// Prints one JSON summary: the samples, and the growth from the warm-up's end
// to the last cycles (median of three each) plus a least-squares slope per
// sample. Frontend only: mock IPC, Chromium.
import { writeFileSync } from 'node:fs';
import { realpathSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';

const BASE = process.env.BASE ?? 'http://127.0.0.1:1459';
const MINUTES = Number(process.env.SOAK_MIN ?? 30);
/** Cycles before the baseline: caches, lazy chunks and pools fill first. */
const WARMUP = Number(process.env.SOAK_WARMUP ?? 3);

/** Median of the middle of `a`. */
export const median = a => {
  if (!a.length) return null;
  const s = [...a].sort((x, y) => x - y);
  return s[Math.floor(s.length / 2)];
};

/** Least-squares slope of `ys` over their index. */
export function slope(ys) {
  const n = ys.length;
  if (n < 2) return 0;
  const mx = (n - 1) / 2;
  const my = ys.reduce((a, b) => a + b, 0) / n;
  let num = 0, den = 0;
  ys.forEach((y, x) => { num += (x - mx) * (y - my); den += (x - mx) ** 2; });
  return num / den;
}

/** Growth of each counter from the warm-up's end to the run's end. */
export function growth(samples, warmup = WARMUP) {
  const after = samples.slice(warmup);
  const keys = after.length ? Object.keys(after[0]).filter(k => typeof after[0][k] === 'number' && k !== 'cycle' && k !== 'atMin') : [];
  const out = after.length < 6 ? { note: `only ${after.length} samples after the warm-up: the two ends share samples` } : {};
  for (const k of keys) {
    const series = after.map(s => s[k]);
    const first = median(series.slice(0, 3));
    const last = median(series.slice(-3));
    out[k] = { first, last, delta: +(last - first).toFixed(2), perCycle: +slope(series).toFixed(4) };
  }
  return out;
}

async function main() {
  const outFile = process.argv.includes('--out') ? process.argv[process.argv.indexOf('--out') + 1] : null;
  const browser = await chromium.launch();
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await ctx.newPage();
  const cdp = await ctx.newCDPSession(page);
  await cdp.send('Memory.enable').catch(() => undefined);
  await page.goto(`${BASE}/?mock=1&demo=large`);
  await page.getByRole('heading', { name: 'Projects', exact: true }).waitFor();
  await page.waitForTimeout(2000);
  const rail = page.getByRole('navigation', { name: 'Workspace navigation' });
  const main = page.getByRole('navigation', { name: 'Main' });
  const head = n => rail.getByRole('group', { name: `${n} conversations` }).locator('.work-tree-head');
  const sample = async (cycle, t0) => {
    await cdp.send('HeapProfiler.collectGarbage');
    await page.waitForTimeout(200);
    await cdp.send('HeapProfiler.collectGarbage');
    const heap = (await cdp.send('Runtime.getHeapUsage')).usedSize / 1048576;
    const dom = await cdp.send('Memory.getDOMCounters').catch(() => ({}));
    return {
      cycle, atMin: +((Date.now() - t0) / 60000).toFixed(2), heapMB: +heap.toFixed(2),
      documents: dom.documents ?? -1, nodes: dom.nodes ?? -1, listeners: dom.jsEventListeners ?? -1,
      frames: page.frames().length, iframes: await page.evaluate(() => document.querySelectorAll('iframe').length),
    };
  };
  const cycle = async () => {
    await page.getByRole('button', { name: /Search anything/ }).first().click();
    await page.getByPlaceholder(/Find a conversation/).fill('Canvas review thread');
    await page.locator('[data-palette-idx]', { hasText: 'Canvas review thread' }).first().waitFor();
    await page.keyboard.press('Enter');
    await page.locator('.turn-content', { hasText: "That's round 30." }).first().waitFor({ timeout: 15000 });
    await page.evaluate(async () => {
      const s = document.querySelector('.chat-scroll');
      if (!s) return;
      const frame = () => new Promise(r => requestAnimationFrame(r));
      s.scrollTop = 0;
      await new Promise(r => setTimeout(r, 300));
      // Capped: a transcript that stops moving fails the soak instead of hanging it.
      for (let i = 0; s.scrollTop + s.clientHeight < s.scrollHeight - 2; i++) { if (i > 500) throw new Error('scroll down stuck'); s.scrollTop += 400; await frame(); await frame(); }
      await new Promise(r => setTimeout(r, 300));
      for (let i = 0; s.scrollTop > 0; i++) { if (i > 500) throw new Error('scroll up stuck'); s.scrollTop -= 600; await frame(); }
    });
    for (const n of ['Q3 regional operations review', 'Q4 hiring plan']) {
      await head(n).click();
      await page.locator('.work-header h1', { hasText: n }).waitFor({ state: 'attached' });
      await page.waitForTimeout(150);
    }
    // Automations: its run listeners mount and unmount (P6's listener fix).
    await main.getByRole('button', { name: /^Automations/ }).click();
    await page.locator('.workflows-page, .page').first().waitFor();
    await page.waitForTimeout(300);
    await main.getByRole('button', { name: /^Tasks/ }).click();
    await page.locator('.task-line').first().waitFor();
    await page.waitForTimeout(150);
    await main.getByRole('button', { name: /^Projects/ }).click();
    await page.getByRole('heading', { name: 'Projects', exact: true }).waitFor();
    await page.getByRole('button', { name: /Search anything/ }).first().click();
    await page.getByPlaceholder(/Find a conversation/).fill('plan');
    await page.waitForTimeout(150);
    await page.keyboard.press('Escape');
    await page.waitForTimeout(150);
  };
  const t0 = Date.now();
  const samples = [await sample(0, t0)];
  for (let i = 1; Date.now() - t0 < MINUTES * 60_000; i++) {
    await cycle();
    samples.push(await sample(i, t0));
  }
  await browser.close();
  const summary = { base: BASE, minutes: MINUTES, cycles: samples.length - 1, warmup: WARMUP, growth: growth(samples), samples };
  const json = JSON.stringify(summary, null, 1);
  if (outFile) writeFileSync(outFile, json);
  console.log(JSON.stringify({ minutes: MINUTES, cycles: summary.cycles, growth: summary.growth }, null, 1));
}

const invoked = process.argv[1] && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));
if (invoked) await main();
