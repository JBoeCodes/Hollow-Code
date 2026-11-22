#!/usr/bin/env node
// Next-level P21: the verification contract's interaction budgets, measured in
// the browser mock on the scale fixture (`?mock=1&demo=large`: 5,000 tasks,
// 10,000 library notes, 200 workspaces, a 1,000-turn thread). Run against a
// production-mode build that keeps the mock (`build:mock`; a plain
// `npm run build` drops it, see src/mockMode.ts), one server at a time:
//   npm run build:mock && npx vite preview --host 127.0.0.1 --port 1459 --strictPort
//   BASE=http://127.0.0.1:1459 node scripts/perf-mock-budgets.mjs
// This measures the frontend only (mock IPC, no Rust, no WKWebView); native
// timings need the app itself. Prints one JSON summary.
import { chromium } from 'playwright';
import { PAGE_TITLE, TASK_ROW, openChatBySearch, openPalette, railProject } from './perf-nav.mjs';
const BASE = process.env.BASE ?? 'http://127.0.0.1:1459';
const pct = (a, p) => { const s = [...a].sort((x, y) => x - y); return +s[Math.min(s.length - 1, Math.ceil(p / 100 * s.length) - 1)].toFixed(1); };
const out = {};
const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
const page = await ctx.newPage();
const cdp = await ctx.newCDPSession(page);
await page.addInitScript(() => {
  window.__long = [];
  new PerformanceObserver(l => { for (const e of l.getEntries()) window.__long.push(e.duration); }).observe({ type: 'longtask', buffered: true });
  window.__events = [];
  try { new PerformanceObserver(l => { for (const e of l.getEntries()) if (e.name === 'keydown' || e.name === 'keypress' || e.name === 'input') window.__events.push(e.duration); }).observe({ type: 'event', durationThreshold: 16, buffered: true }); } catch {}
});
const longs = async () => page.evaluate(() => { const l = window.__long; window.__long = []; return l; });

// 1. First usable shell (cold browser cache, mock data seeded in-page).
for (let run = 0; run < 3; run++) {
  const t0 = Date.now();
  await page.goto(`${BASE}/?mock=1&demo=large`);
  await page.getByRole('navigation', { name: 'Main' }).waitFor();
  await page.getByRole('heading', { name: 'Projects', exact: true }).waitFor();
  (out.shell ??= []).push(Date.now() - t0);
}
out.shellLong = (await longs()).filter(d => d > 50).length;
// Five asks B4: how much DOM the shell builds (the rail lists 200 workspaces).
out.shellDomNodes = await page.evaluate(() => document.querySelectorAll('*').length);
// Overnight Y7: IPC calls 3 s after the shell's last load, from the mock's own
// tally (src/launchIpc.ts budgets repeats in the app over 15 s; this total is
// for comparing builds, not with the app's).
await page.waitForTimeout(3000);
out.shellIpcCalls = await page.evaluate(() => Object.values(window.__mockApiCalls ?? {}).reduce((a, b) => a + b, 0));

// 2. Warm navigation between two loaded workspaces.
const head = t => railProject(page, t);
const names = ['Q3 regional operations review', 'Q4 hiring plan'];
for (const n of names) { await head(n).click(); await page.locator(PAGE_TITLE, { hasText: n }).waitFor({ state: 'attached' }); }
await longs();
for (let i = 0; i < 20; i++) {
  const n = names[i % 2];
  const t = await page.evaluate(() => performance.now());
  await head(n).click();
  await page.locator(PAGE_TITLE, { hasText: n }).waitFor({ state: 'attached' });
  await page.locator('.chat-composer textarea').waitFor();
  (out.warmNav ??= []).push(await page.evaluate(t0 => performance.now() - t0, t));
}
out.warmNavLong = (await longs()).filter(d => d > 50);

// 3. Composer typing in a 1,000-turn thread (opened from the palette).
// Five asks B4: open time of the 1,000-turn thread and the DOM it builds.
{
  // Open it from the sidebar's chat search (the thread is not among the recent projects).
  const row = await openChatBySearch(page, 'Long design thread', 'Long design thread');
  const t = await page.evaluate(() => performance.now());
  await row.click();
  await page.locator('.turn-content', { hasText: 'Reviewed part 499' }).first().waitFor({ timeout: 10000 }).catch(() => { out.longThreadMissing = true; });
  out.longThreadOpenMs = Math.round(await page.evaluate(t0 => performance.now() - t0, t));
  out.longThreadDomNodes = await page.evaluate(() => document.querySelectorAll('*').length);
  out.longThreadOpenLong = (await longs()).filter(d => d > 50);
}
await page.locator('.chat-composer textarea').click();
await page.evaluate(() => { window.__events = []; });
await page.keyboard.type('Checking typing latency in a long conversation with many turns.', { delay: 15 });
out.typingSlowEvents = await page.evaluate(() => window.__events);
out.typingLong = (await longs()).filter(d => d > 50);
await page.locator('.chat-composer textarea').fill('');
// The burst above starts the moment the thread shows, while its turns are
// still drawn in steps. Five asks B4: type again once it has settled.
await page.waitForTimeout(3000);
await page.evaluate(() => { window.__events = []; });
await page.keyboard.type('Checking typing latency once the thread has settled.', { delay: 15 });
out.typingSettledSlowEvents = await page.evaluate(() => window.__events);
await page.locator('.chat-composer textarea').fill('');

// 4. Search: palette over 10,000 library notes and 200 workspaces.
const qs = ['needle', 'location 150', 'garage lease', 'hiring', 'board pack', 'variance', 'invoice', 'operating plan', 'q3', 'pricing'];
for (const q of qs) {
  const tOpen = await page.evaluate(() => performance.now());
  const palette = await openPalette(page);
  const input = palette.getByRole('combobox').or(palette.locator('input')).first();
  await input.waitFor();
  (out.paletteOpen ??= []).push(await page.evaluate(t0 => performance.now() - t0, tOpen));
  const before = await page.evaluate(() => [...document.querySelectorAll('[data-palette-idx]')].map(r => r.textContent).join('|'));
  const t = await page.evaluate(() => performance.now());
  await input.fill(q);
  const ok = await page.waitForFunction(b => { const now = [...document.querySelectorAll('[data-palette-idx]')].map(r => r.textContent).join('|'); return now !== b; }, before, { timeout: 3000, polling: 'raf' }).then(() => true).catch(() => false);
  // Settled: the rows stop changing for 400 ms (debounced note/project search
  // included); report when they last changed.
  const settled = await page.evaluate(async t0 => {
    let last = performance.now(), sig = '';
    const read = () => [...document.querySelectorAll('[data-palette-idx]')].map(r => r.textContent).join('|');
    sig = read();
    while (performance.now() - last < 400 && performance.now() - t0 < 4000) {
      await new Promise(r => requestAnimationFrame(r));
      const now = read(); if (now !== sig) { sig = now; last = performance.now(); }
    }
    return last - t0;
  }, t);
  const ms = settled;
  (out.search ??= []).push(ms); (out.searchDetail ??= []).push(`${q}:${ok ? Math.round(ms) : 'unchanged'}:${await page.locator('[data-palette-idx]').count()}`);
  await page.keyboard.press('Escape');
  await page.waitForTimeout(200);
}
out.searchLong = (await longs()).filter(d => d > 50);

// 5. Tasks: 5,000 tasks, scroll the list to the end.
await longs();
await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: /^Tasks/ }).click();
const t5 = Date.now();
await page.locator(TASK_ROW).first().waitFor();
out.tasksFirstRow = Date.now() - t5;
out.tasksRowsInDom = await page.locator(TASK_ROW).count();
await page.waitForTimeout(300);
out.tasksOpenLong = (await longs()).filter(d => d > 50);
const scroller = await page.evaluateHandle(() => { let el = document.querySelector('.issue-row'); while (el && !(el.scrollHeight > el.clientHeight + 10 && /auto|scroll/.test(getComputedStyle(el).overflowY))) el = el.parentElement; return el; });
out.tasksRowsAfterScroll = 0;
for (let i = 0; i < 40; i++) { await scroller.evaluate(el => el && (el.scrollTop += 1200)); await page.waitForTimeout(50); }
out.tasksScrollLong = (await longs()).filter(d => d > 50);
out.tasksRowsAfterScroll = await page.locator(TASK_ROW).count();
out.tasksTabCounts = await page.getByRole('tablist', { name: 'Task views' }).innerText().catch(() => '');

// 6. Memory across 20 palette + side-panel cycles.
const heap = async () => { await cdp.send('HeapProfiler.collectGarbage'); return (await cdp.send('Runtime.getHeapUsage')).usedSize / 1048576; };
await head(names[0]).click(); await page.locator(PAGE_TITLE, { hasText: names[0] }).waitFor({ state: 'attached' });
const h0 = await heap();
for (let i = 0; i < 20; i++) {
  const pal = await openPalette(page);
  await pal.locator('input').first().fill('plan');
  await page.keyboard.press('Escape');
  await head(names[(i + 1) % 2]).click();
  await page.waitForTimeout(80);
}
const h1 = await heap();
for (let i = 0; i < 20; i++) {
  const pal = await openPalette(page);
  await pal.locator('input').first().fill('plan');
  await page.keyboard.press('Escape');
  await head(names[(i + 1) % 2]).click();
  await page.waitForTimeout(80);
}
const h2 = await heap();
out.heapMB = [h0, h1, h2].map(x => +x.toFixed(1));

const summary = {
  shellMs: out.shell, shellLongTasksOver50: out.shellLong, shellDomNodes: out.shellDomNodes, shellIpcCalls: out.shellIpcCalls,
  longThreadOpenMs: out.longThreadMissing ? 'not opened' : out.longThreadOpenMs, longThreadDomNodes: out.longThreadDomNodes, longThreadLongOver50: out.longThreadOpenLong.length,
  paletteOpenP50: pct(out.paletteOpen, 50), paletteOpenP95: pct(out.paletteOpen, 95),
  warmNavP50: pct(out.warmNav, 50), warmNavP95: pct(out.warmNav, 95), warmNavLongOver50: out.warmNavLong.length,
  typingEventsOver16ms: out.typingSlowEvents.length, typingMaxEventMs: Math.max(0, ...out.typingSlowEvents), typingLongOver50: out.typingLong.length,
  typingSettledEventsOver16ms: out.typingSettledSlowEvents.length, typingSettledMaxEventMs: Math.max(0, ...out.typingSettledSlowEvents),
  searchP50: pct(out.search, 50), searchP95: pct(out.search, 95), searchLongOver50: out.searchLong.length, searchMaxLong: Math.max(0, ...out.searchLong),
  tasksFirstRowMs: out.tasksFirstRow, tasksOpenLongOver50: out.tasksOpenLong, tasksRowsInDom: out.tasksRowsInDom, tasksRowsAfterScroll: out.tasksRowsAfterScroll, tasksScrollLongOver50: out.tasksScrollLong.length, tasksScrollMaxLong: Math.max(0, ...out.tasksScrollLong),
  searchDetail: out.searchDetail,
  tasksTabs: out.tasksTabCounts.replace(/\s+/g, ' ').slice(0, 160),
  heapMB_start_after20_after40: out.heapMB,
};
console.log(JSON.stringify(summary, null, 2));
await browser.close();
