#!/usr/bin/env node
// Overnight 10-11/12 P1: the launch and long-chat half of `npm run perf`, in
// the browser mock on the scale fixture (`?mock=1&demo=large`: 200
// workspaces, 5,000 tasks, 10,000 notes, a 1,000-turn thread and a 60-turn
// thread with 10 canvases). Needs a `build:mock` served by one `vite preview`
// (`npm run perf` starts it); prints one JSON summary:
//   BASE=http://127.0.0.1:1459 node scripts/perf-launch-chat.mjs
// Frontend only: mock IPC, Chromium, no Rust and no WKWebView, so compare
// builds with it, not with the app's own numbers.
import { realpathSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { RAIL_ROWS, openChatBySearch } from './perf-nav.mjs';

const BASE = process.env.BASE ?? 'http://127.0.0.1:1459';
const RUNS = Number(process.env.PERF_RUNS ?? 3);
/** The `p`th percentile of `a` (nearest rank), one decimal. */
export const pct = (a, p) => {
  if (!a.length) return null;
  const s = [...a].sort((x, y) => x - y);
  return +s[Math.min(s.length - 1, Math.max(0, Math.ceil(p / 100 * s.length) - 1))].toFixed(1);
};
const median = a => pct(a, 50);

/** Time to interactive: the end of the last long task that starts before a
 *  quiet window of `quiet` ms with none, and never before `from`. */
export function interactiveAt(longTasks, from, quiet = 1000) {
  let at = from;
  for (const t of [...longTasks].sort((x, y) => x.start - y.start)) {
    if (t.start - at >= quiet) break;
    at = Math.max(at, t.start + t.duration);
  }
  return at;
}

async function main() {
  const browser = await chromium.launch();
  const out = { base: BASE, runs: RUNS };
  const instrument = railSel => {
    window.__long = [];
    new PerformanceObserver(l => { for (const e of l.getEntries()) window.__long.push({ start: e.startTime, duration: e.duration }); }).observe({ type: 'longtask', buffered: true });
    // The rail draws a subset of the workspaces: note when its row count
    // last changed (first 10 s), which is when it finished drawing.
    window.__railAt = null; window.__railRows = 0; 
    const tick = () => {
      const n = document.querySelectorAll(railSel).length;
      if (n !== window.__railRows) { window.__railRows = n; window.__railAt = performance.now(); }
      if (performance.now() < 10_000) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  };

  // 1. Cold launch: first paint, the shell, interactive, IPC at interactive.
  const launch = { ipcByCommand: null, fcp: [], shell: [], sidebar: [], interactive: [], ipcAtInteractive: [], ipcCommands: [], railRows: 0 };
  for (let run = 0; run < RUNS; run++) {
    const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    const page = await ctx.newPage();
    await page.addInitScript(instrument, RAIL_ROWS);
    await page.goto(`${BASE}/?mock=1&demo=large`);
    await page.getByRole('navigation', { name: 'Main' }).waitFor();
    await page.getByRole('heading', { name: 'Projects', exact: true }).waitFor();
    const shell = await page.evaluate(() => performance.now());
    // Wait until a quiet second has been seen after the shell (at most 10 s),
    // so interactive isn't cut off by a chain still running.
    await page.waitForFunction(from => {
      const long = window.__long.filter(t => t.start + t.duration >= from);
      const end = long.reduce((at, t) => Math.max(at, t.start + t.duration), from);
      return performance.now() - end >= 1000;
    }, shell, { timeout: 10_000, polling: 100 }).catch(() => {});
    const sidebar = await page.evaluate(() => window.__railAt);
    launch.railRows = await page.evaluate(() => window.__railRows);
    const m = await page.evaluate(() => ({
      fcp: performance.getEntriesByName('first-contentful-paint')[0]?.startTime ?? null,
      long: window.__long,
      calls: window.__mockApiCalls ?? {},
    }));
    const tti = interactiveAt(m.long, shell);
    launch.fcp.push(m.fcp); launch.shell.push(shell); launch.sidebar.push(sidebar); launch.interactive.push(tti);
    // IPC by interactive is not separable after the fact; the tally after the
    // quiet window is the launch's (the mock counts every invoke).
    const total = Object.values(m.calls).reduce((a, b) => a + b, 0);
    launch.ipcAtInteractive.push(total);
    launch.ipcCommands.push(Object.keys(m.calls).length);
    launch.ipcByCommand = m.calls;
    await ctx.close();
  }
  out.launch = {
    firstContentfulPaintMs: median(launch.fcp.filter(x => x !== null)),
    shellVisibleMs: median(launch.shell),
    sidebarDrawnMs: median(launch.sidebar.filter(x => x !== null)),
    sidebarRows: launch.railRows,
    interactiveMs: median(launch.interactive),
    ipcCalls: median(launch.ipcAtInteractive),
    ipcDistinctCommands: median(launch.ipcCommands),
    // The last run's tally by command (what the launch asks for, with repeats).
    ipcByCommand: Object.fromEntries(Object.entries(launch.ipcByCommand ?? {}).sort((a, b) => b[1] - a[1])),
  };

  // 2. Rail show: hide and show the sidebar. Collapsing is a class change
  // (the rows stay in the DOM), so this times showing it until its rows are
  // laid out again, not a re-render of the workspace list.
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await ctx.newPage();
  await page.addInitScript(instrument, RAIL_ROWS);
  const cdp = await ctx.newCDPSession(page);
  await page.goto(`${BASE}/?mock=1&demo=large`);
  await page.getByRole('heading', { name: 'Projects', exact: true }).waitFor();
  await page.waitForTimeout(1500);
  const rows = await page.evaluate(sel => document.querySelectorAll(sel).length, RAIL_ROWS);
  const toggles = [];
  for (let i = 0; i < 5; i++) {
    await page.getByRole('button', { name: 'Hide sidebar' }).click();
    await page.getByRole('button', { name: 'Show sidebar' }).waitFor();
    const t = await page.evaluate(() => performance.now());
    await page.getByRole('button', { name: 'Show sidebar' }).click();
    await page.waitForFunction(({ n, sel }) => {
      const heads = document.querySelectorAll(sel);
      return heads.length >= n && heads[heads.length - 1].getBoundingClientRect().width > 0;
    }, { n: rows, sel: RAIL_ROWS }, { timeout: 10000, polling: 'raf' });
    toggles.push(await page.evaluate(t0 => performance.now() - t0, t));
  }
  out.sidebarShowMs = { p50: median(toggles), max: Math.max(...toggles) };

  // 3. A 60-turn chat with 10 canvases: open, scroll top to bottom, memory.
  const heap = async () => { await cdp.send('HeapProfiler.collectGarbage'); return +((await cdp.send('Runtime.getHeapUsage')).usedSize / 1048576).toFixed(1); };
  const heapBefore = await heap();
  await page.evaluate(() => { window.__long = []; });
  const row = await openChatBySearch(page, 'Canvas review thread', 'Canvas review thread');
  const tOpen = await page.evaluate(() => performance.now());
  await row.click();
  await page.locator('.turn-content', { hasText: "That's round 30." }).first().waitFor({ timeout: 15000 });
  const openMs = Math.round(await page.evaluate(t0 => performance.now() - t0, tOpen));
  await page.waitForTimeout(1500);
  const openLong = await page.evaluate(() => window.__long.filter(t => t.duration > 50).length);
  // Scroll the transcript from its top to its bottom in 300 px steps, one
  // per frame, and record each frame's length.
  const scroll = await page.evaluate(async () => {
    const s = document.querySelector('.chat-scroll');
    if (!s) return null;
    window.__long = [];
    s.scrollTop = 0;
    await new Promise(r => setTimeout(r, 500));
    const frames = [];
    let last = performance.now();
    while (s.scrollTop + s.clientHeight < s.scrollHeight - 2) {
      s.scrollTop += 300;
      await new Promise(r => requestAnimationFrame(r));
      const now = performance.now();
      frames.push(now - last);
      last = now;
      if (frames.length > 2000) break;
    }
    return { frames, height: s.scrollHeight, long: window.__long.filter(t => t.duration > 50).length, iframes: document.querySelectorAll('.chat-scroll iframe').length, canvases: document.querySelectorAll('.chat-scroll .chat-canvas, .chat-scroll .chat-canvas-skeleton:not(.chat-canvas .chat-canvas-skeleton)').length };
  });
  const heapAfter = await heap();
  out.canvasChat = scroll ? {
    openMs, openLongOver50: openLong,
    scrollFrames: scroll.frames.length, frameP50Ms: pct(scroll.frames, 50), frameP95Ms: pct(scroll.frames, 95), frameMaxMs: scroll.frames.length ? +Math.max(...scroll.frames).toFixed(1) : null,
    framesOver50Ms: scroll.frames.filter(f => f > 50).length, scrollLongOver50: scroll.long,
    transcriptHeightPx: scroll.height, iframesMounted: scroll.iframes, canvasesRendered: scroll.canvases,
    // The page's own isolate: out-of-process canvas frames aren't in it.
    heapMB: { before: heapBefore, afterScroll: heapAfter },
  } : 'not opened';
  // 4. Memory per chat (profiling night 11-20): back to Home, open the 1,000-turn
  // thread and the 60-turn canvas thread in turn; the JS heap (after a forced GC)
  // and DOM nodes with each chat open, and Home's heap after each pair, so a
  // chat that is not released shows as a slope. The page's own isolate only.
  const chats = [['Long design thread', 'Reviewed part 499'], ['Canvas review thread', "That's round 30."]];
  const goHome = async () => { await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: /^Projects/ }).click(); await page.locator('section[aria-label="Start a conversation"]').waitFor(); await page.waitForTimeout(400); };
  const perChat = { homeHeapMB: [await (async () => { await goHome(); return heap(); })()], open: [] };
  for (let round = 0; round < 4; round++) {
    for (const [query, text] of chats) {
      const r = await openChatBySearch(page, query, query);
      await r.click();
      await page.locator('.turn-content', { hasText: text }).first().waitFor({ timeout: 15000 });
      await page.waitForTimeout(600);
      perChat.open.push({ chat: query, round, heapMB: await heap(), domNodes: await page.evaluate(() => document.querySelectorAll('*').length) });
      await goHome();
    }
    perChat.homeHeapMB.push(await heap());
  }
  const firstOpen = Object.fromEntries(chats.map(([q]) => [q, perChat.open.find(o => o.chat === q)]));
  out.memoryPerChat = {
    homeHeapMB: perChat.homeHeapMB,
    openHeapMB: Object.fromEntries(chats.map(([q]) => [q, perChat.open.filter(o => o.chat === q).map(o => o.heapMB)])),
    openDomNodes: Object.fromEntries(chats.map(([q]) => [q, firstOpen[q].domNodes])),
    heapOverHomeMB: Object.fromEntries(chats.map(([q]) => [q, +(firstOpen[q].heapMB - perChat.homeHeapMB[0]).toFixed(1)])),
    retainedPerRoundMB: +((perChat.homeHeapMB[perChat.homeHeapMB.length - 1] - perChat.homeHeapMB[1]) / (perChat.homeHeapMB.length - 2)).toFixed(2),
  };
  await ctx.close();
  await browser.close();
  console.log(JSON.stringify(out, null, 2));
}

/** Run as a script (real paths: /tmp is /private/tmp, and symlinks). */
const real = p => { try { return realpathSync(p); } catch { return p; } };
if (process.argv[1] && real(fileURLToPath(import.meta.url)) === real(process.argv[1])) await main();
