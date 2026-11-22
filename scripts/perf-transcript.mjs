#!/usr/bin/env node
// Profiling night 11-20: what a long transcript costs, and the budget the
// virtualized transcript (parity 0.3) is held to. Browser mock on the scale
// fixture's 1,000-turn thread (`?mock=1&demo=large`), Chromium, frontend only
// (no Rust, no WKWebView): compare builds with it, not with the app's numbers.
//   BASE=http://127.0.0.1:1459 node scripts/perf-transcript.mjs
// `npm run perf` runs it. It opens the thread, loads every earlier turn the
// way a person does (Show earlier, or scrolling to the top where the transcript
// loads them itself), then scrolls the whole transcript in 300 px steps, one per
// frame, and types in the composer. Per stage: DOM nodes, JS heap after a forced
// GC, turns in the DOM, frame times.
import { realpathSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { openChatBySearch } from './perf-nav.mjs';
import { pct } from './perf-launch-chat.mjs';
import { TRANSCRIPT_BUDGET } from './perf-budgets.mjs';

export { TRANSCRIPT_BUDGET };

const BASE = process.env.BASE ?? 'http://127.0.0.1:1459';

export function overBudget(r, b = TRANSCRIPT_BUDGET) {
  const over = [];
  if (r.maxTurnsInDom > b.maxTurnsInDom) over.push(`turns in DOM ${r.maxTurnsInDom} > ${b.maxTurnsInDom}`);
  if (r.domNodesAtTop > b.maxDomNodes) over.push(`DOM nodes ${r.domNodesAtTop} > ${b.maxDomNodes}`);
  if (r.frameP95Ms > b.frameP95Ms) over.push(`frame p95 ${r.frameP95Ms} > ${b.frameP95Ms} ms`);
  if (r.frameMaxMs > b.frameMaxMs) over.push(`worst frame ${r.frameMaxMs} > ${b.frameMaxMs} ms`);
  if (r.heapOverHomeMB > b.maxHeapOverHomeMB) over.push(`heap over Home ${r.heapOverHomeMB} > ${b.maxHeapOverHomeMB} MB`);
  if (r.openMs > b.openMs) over.push(`open ${r.openMs} > ${b.openMs} ms`);
  return over;
}

async function main() {
  const browser = await chromium.launch();
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const page = await ctx.newPage();
  const cdp = await ctx.newCDPSession(page);
  await page.addInitScript(() => {
    window.__long = [];
    new PerformanceObserver(l => { for (const e of l.getEntries()) window.__long.push(e.duration); }).observe({ type: 'longtask', buffered: true });
    window.__events = [];
    try { new PerformanceObserver(l => { for (const e of l.getEntries()) if (e.name === 'keydown' || e.name === 'keypress' || e.name === 'input') window.__events.push(e.duration); }).observe({ type: 'event', durationThreshold: 16, buffered: true }); } catch { /* not supported */ }
  });
  const heap = async () => { await cdp.send('HeapProfiler.collectGarbage'); return +((await cdp.send('Runtime.getHeapUsage')).usedSize / 1048576).toFixed(1); };
  const state = () => page.evaluate(() => {
    const s = document.querySelector('.chat-scroll');
    return { turns: document.querySelectorAll('.chat-scroll .turn[data-turn-index]').length, nodes: document.querySelectorAll('*').length, first: document.querySelector('.chat-scroll .turn[data-turn-index]')?.getAttribute('data-turn-index') ?? null, height: s?.scrollHeight ?? 0 };
  });
  await page.goto(`${BASE}/?mock=1&demo=large`);
  await page.locator('section[aria-label="Start a conversation"]').waitFor();
  await page.waitForTimeout(1500);
  const homeHeap = await heap();

  const row = await openChatBySearch(page, 'Long design thread', 'Long design thread');
  const t0 = await page.evaluate(() => performance.now());
  await row.click();
  await page.locator('.turn-content', { hasText: 'Reviewed part 499' }).first().waitFor({ timeout: 15000 });
  const openMs = Math.round(await page.evaluate(t => performance.now() - t, t0));
  await page.waitForTimeout(1500);
  const atOpen = { ...(await state()), heapMB: await heap() };

  // Read the thread the way a person does, by wheel: back to turn 0, then forward to the end. Each
  // step is one wheel event of 600 px (the page's reader-input gate only loads turns for a reader),
  // one frame; frame times, the turns drawn and the window's start are sampled every step.
  const sweep = dir => page.evaluate(async d => {
    const s = document.querySelector('.chat-scroll');
    const frames = [];
    let maxTurns = 0;
    let loads = 0;
    let lastFirst = null;
    let last = performance.now();
    window.__long = [];
    for (let i = 0; i < 800; i++) {
      const before = s.scrollTop;
      s.dispatchEvent(new WheelEvent('wheel', { deltaY: d * 600, bubbles: true, cancelable: true }));
      s.scrollTop += d * 600;
      await new Promise(r => requestAnimationFrame(r));
      const now = performance.now(); frames.push(now - last); last = now;
      const turns = s.querySelectorAll('.turn[data-turn-index]');
      maxTurns = Math.max(maxTurns, turns.length);
      const first = turns[0]?.getAttribute('data-turn-index') ?? null;
      if (first !== lastFirst) { loads++; lastFirst = first; }
      const stuck = s.scrollTop === before;
      if (stuck) {
        // At the edge of what is drawn: give the loader (an intersection callback) frames to run.
        let idle = 0;
        while (idle < 12 && s.scrollTop === before) { await new Promise(r => requestAnimationFrame(r)); idle++; s.dispatchEvent(new WheelEvent('wheel', { deltaY: d * 600, bubbles: true, cancelable: true })); s.scrollTop += d * 600; }
        if (s.scrollTop === before) break;
      }
    }
    return { frames, maxTurns, loads, long: window.__long.filter(x => x > 50).length };
  }, dir);
  const back = await sweep(-1);
  await page.waitForTimeout(800);
  const atTop = { ...(await state()), heapMB: await heap() };
  await page.locator('.chat-composer textarea').click();
  await page.evaluate(() => { window.__events = []; });
  await page.keyboard.type('Typing after reading a thousand turns back.', { delay: 15 });
  const typing = await page.evaluate(() => window.__events);
  const forward = await sweep(1);
  await page.waitForTimeout(800);
  const afterScroll = { ...(await state()), heapMB: await heap() };
  const frames = { frames: [...back.frames, ...forward.frames], long: back.long + forward.long };
  const maxTurnsInDom = Math.max(atOpen.turns, back.maxTurns, forward.maxTurns);
  const stages = [{ sweep: 'back to turn 0', steps: back.frames.length, windowMoves: back.loads, maxTurnsDrawn: back.maxTurns }, { sweep: 'forward to the end', steps: forward.frames.length, windowMoves: forward.loads, maxTurnsDrawn: forward.maxTurns }];

  const result = {
    openMs, atOpen, atTop, afterScroll, homeHeapMB: homeHeap, stages,
    maxTurnsInDom, domNodesAtTop: atTop.nodes, heapOverHomeMB: +(Math.max(atTop.heapMB, afterScroll.heapMB) - homeHeap).toFixed(1),
    scrollFrames: frames.frames.length, frameP50Ms: pct(frames.frames, 50), frameP95Ms: pct(frames.frames, 95), frameMaxMs: frames.frames.length ? +Math.max(...frames.frames).toFixed(1) : null,
    framesOver32Ms: frames.frames.filter(f => f > 32).length, scrollLongOver50: frames.long,
    typingEventsOver16ms: typing.length, typingMaxEventMs: Math.max(0, ...typing),
  };
  result.overBudget = overBudget(result);
  await browser.close();
  console.log(JSON.stringify(result, null, 2));
}

const real = p => { try { return realpathSync(p); } catch { return p; } };
if (process.argv[1] && real(fileURLToPath(import.meta.url)) === real(process.argv[1])) await main();
