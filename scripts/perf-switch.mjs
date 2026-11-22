#!/usr/bin/env node
// ST2 (plan completion night, 2026-10-14): cold start to an interactive Home,
// and the switch time between Home, Work → Today, Tasks, Library, Automations and a
// 1,000-turn conversation, as median and p90 with the sample size. Browser
// mock on the scale fixture (`?mock=1&demo=large`), Chromium: frontend only
// (mock IPC, no Rust, no WKWebView), so compare builds with it, not with the
// app's own numbers. `npm run perf` runs it; alone, it needs a `build:mock`
// served by one `vite preview` (no other dev or preview server up):
//   npm run build:mock && node node_modules/vite/bin/vite.js preview --host 127.0.0.1 --port 1459 --strictPort
//   BASE=http://127.0.0.1:1459 node scripts/perf-switch.mjs
// COLD_RUNS (default 10) and CYCLES (default 10) set the samples. PROFILE=1
// adds the top self-time source files per switch from a CPU profile (build
// with sourcemaps for readable names: `npx vite build --mode mock --sourcemap`).
import { existsSync, readFileSync, realpathSync } from 'node:fs';
import { createRequire } from 'node:module';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { HOME_READY, openChatBySearch } from './perf-nav.mjs';
import { interactiveAt, pct } from './perf-launch-chat.mjs';

const BASE = process.env.BASE ?? 'http://127.0.0.1:1459';
const COLD_RUNS = Number(process.env.COLD_RUNS ?? 10);
const CYCLES = Number(process.env.CYCLES ?? 10);
const PROFILE = process.env.PROFILE === '1';
const root = fileURLToPath(new URL('..', import.meta.url));

/** `{ n, p50, p90, max }` of a sample, one decimal. */
export function summary(a) {
  const xs = a.filter(x => typeof x === 'number');
  return { n: xs.length, p50: pct(xs, 50), p90: pct(xs, 90), max: xs.length ? +Math.max(...xs).toFixed(1) : null };
}

const instrument = () => {
  window.__long = [];
  new PerformanceObserver(l => { for (const e of l.getEntries()) window.__long.push({ start: e.startTime, duration: e.duration }); }).observe({ type: 'longtask', buffered: true });
};
const painted = page => page.evaluate(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(() => r(performance.now())))));

/** Self time per original source file from a CPU profile (sourcemaps when the build has them). */
function attribute(profile) {
  const require = createRequire(join(root, 'package.json'));
  const { SourceMapConsumer } = require('source-map-js');
  const maps = new Map();
  const mapFor = url => {
    if (maps.has(url)) return maps.get(url);
    let m = null;
    const file = url.startsWith(BASE) ? join(root, 'dist', new URL(url).pathname) : null;
    if (file && existsSync(file + '.map')) m = new SourceMapConsumer(JSON.parse(readFileSync(file + '.map', 'utf8')));
    maps.set(url, m);
    return m;
  };
  const self = new Map();
  const byId = new Map(profile.nodes.map(n => [n.id, n]));
  const dt = profile.timeDeltas;
  for (let i = 0; i < profile.samples.length; i++) {
    const n = byId.get(profile.samples[i]);
    const f = n.callFrame;
    let key = f.url ? f.url.replace(BASE, '') : `(${f.functionName || 'program'})`;
    const m = f.url && mapFor(f.url);
    if (m) {
      const o = m.originalPositionFor({ line: f.lineNumber + 1, column: f.columnNumber });
      if (o.source) key = `${o.source.replace(/^.*?\/(src|node_modules)\//, '$1/')}${o.name ? ` ${o.name}` : ''}`;
    }
    self.set(key, (self.get(key) ?? 0) + (dt[i] ?? 0) / 1000);
  }
  return [...self].sort((a, b) => b[1] - a[1]).slice(0, 12).map(([k, ms]) => `${ms.toFixed(1)} ms  ${k}`);
}

async function main() {
  const browser = await chromium.launch();
  const out = { base: BASE, browser: 'Chromium (Playwright), browser mock, demo=large', coldRuns: COLD_RUNS, cycles: CYCLES };

  // 1. Cold start: a fresh context each run, to Home's resume region on
  // screen, then interactive (the end of the last long task before a quiet second).
  const cold = { homeVisible: [], interactive: [], timedOut: 0 };
  for (let run = 0; run < COLD_RUNS; run++) {
    const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    const page = await ctx.newPage();
    await page.addInitScript(instrument);
    await page.goto(`${BASE}/?mock=1&demo=large`);
    await page.locator(HOME_READY).waitFor({ timeout: 20_000 });
    const home = await painted(page);
    await page.waitForFunction(from => {
      const end = window.__long.filter(t => t.start + t.duration >= from).reduce((at, t) => Math.max(at, t.start + t.duration), from);
      return performance.now() - end >= 1000;
    }, home, { timeout: 15_000, polling: 100 }).then(() => true, () => false).then(quiet => { if (!quiet) cold.timedOut++; });
    const long = await page.evaluate(() => window.__long);
    cold.homeVisible.push(+home.toFixed(1));
    cold.interactive.push(+interactiveAt(long, home).toFixed(1));
    await ctx.close();
  }
  // Home visible is read after Playwright's wait, so it runs a little late;
  // fine for comparing builds. A run that never went quiet is counted.
  out.coldStart = { homeVisibleMs: summary(cold.homeVisible), homeInteractiveMs: summary(cold.interactive), neverQuiet: cold.timedOut };

  // 2. Switches, in one warm context: each destination from the one before it.
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await ctx.newPage();
  await page.addInitScript(instrument);
  const cdp = await ctx.newCDPSession(page);
  await page.goto(`${BASE}/?mock=1&demo=large`);
  await page.locator(HOME_READY).waitFor({ timeout: 20_000 });
  await page.waitForTimeout(2000);
  // Each switch is one click, timed inside the page (no Playwright round
  // trips or actionability checks): from the click to the destination's
  // landmark on screen, plus two frames for its paint. The rail's Work
  // lands on Workspaces (P1-05); the cycle also times the tab switch back.
  const steps = [
    { name: 'home', from: 'the large chat (sidebar Work)', click: ['nav[aria-label="Main"] button', 'Projects'], ready: ['section[aria-label="Start a conversation"]', ''] },
    { name: 'running', from: 'Home', click: ['nav[aria-label="Projects views"] button', 'Running'], ready: ['[aria-label="Running"]', ''] },
    { name: 'homeView', from: 'Running', click: ['nav[aria-label="Projects views"] button', 'Home'], ready: ['[aria-label="Start a conversation"]', ''] },
    { name: 'tasks', from: 'Home', click: ['nav[aria-label="Main"] button', 'Tasks'], ready: ['h1', 'Tasks'] },
    { name: 'library', from: 'Tasks', click: ['nav[aria-label="Main"] button', 'Library'], ready: ['h1', 'Library'] },
    { name: 'automations', from: 'Library', click: ['nav[aria-label="Main"] button', 'Automations'], ready: ['h1', 'Automations'] },
    { name: 'largeChat', from: 'Automations (sidebar chat search, click)', pre: async () => {
        await openChatBySearch(page, 'Long design thread', 'Long design thread');
      },
      click: ['.work-rail-search-chat', 'Long design thread'], ready: ['.turn-content', 'Reviewed part 499'] },
  ];
  const run = s => page.evaluate(async ({ click, ready, enter }) => {
    const find = ([sel, text], exact) => [...document.querySelectorAll(sel)].find(e => {
      const t = (e.textContent ?? '').trim();
      return !text || (exact ? t === text : t.includes(text));
    });
    const shown = () => { const e = find(ready, ready[0] === 'h1'); return !!e && e.getClientRects().length > 0; };
    const frame = () => new Promise(r => requestAnimationFrame(r));
    const t0 = performance.now();
    if (enter) document.activeElement?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    else { const target = find(click, false); if (!target) return null; target.click(); }
    while (!shown()) { await frame(); if (performance.now() - t0 > 15000) return null; }
    await frame(); await frame();
    const ms = +(performance.now() - t0).toFixed(1);
    await frame(); // long-task entries arrive a little after the task ends
    return ms;
  }, { click: s.click, ready: s.ready, enter: !!s.enter });
  const times = Object.fromEntries(steps.map(s => [s.name, []]));
  const longs = Object.fromEntries(steps.map(s => [s.name, 0]));
  const profiles = {};
  // One unmeasured pass loads every lazy chunk, so the cycles time switches, not first loads.
  // Start from the large chat, so the first step's 'from' holds.
  for (const s of [steps[steps.length - 1], ...steps]) { await s.pre?.(); if ((await run(s)) === null) throw new Error(`perf-switch: ${s.name} never showed`); }
  if (PROFILE) await cdp.send('Profiler.enable');
  // With PROFILE=1 one extra cycle is profiled and not counted.
  for (let c = 0; c < CYCLES + (PROFILE ? 1 : 0); c++) {
    for (const s of steps) {
      await s.pre?.();
      await page.waitForTimeout(150);
      await page.evaluate(() => { window.__long = []; });
      const profiling = PROFILE && c === CYCLES;
      if (profiling) await cdp.send('Profiler.start');
      const ms = await run(s);
      if (profiling) profiles[s.name] = attribute((await cdp.send('Profiler.stop')).profile);
      if (profiling) continue;
      times[s.name].push(ms);
      longs[s.name] += await page.evaluate(() => window.__long.filter(t => t.duration > 50).length);
    }
  }
  out.switchMs = Object.fromEntries(steps.map(s => [s.name, { from: s.from, ...summary(times[s.name]), longTasksOver50: longs[s.name] }]));
  if (PROFILE) out.profileTop = profiles;
  await ctx.close();
  await browser.close();
  console.log(JSON.stringify(out, null, 2));
}

const real = p => { try { return realpathSync(p); } catch { return p; } };
if (process.argv[1] && real(fileURLToPath(import.meta.url)) === real(process.argv[1])) await main();
