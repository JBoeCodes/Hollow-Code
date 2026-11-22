import { existsSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { expect, test, type CDPSession, type Page } from '../smoke/fixtures/test';
import { openGlobalSearch } from '../smoke/choose';

/**
 * Run 15 S7: frontend responsiveness probe. One test, five measures, on the
 * mock's scale fixture (`?mock=1&demo=large`: 200 workspaces, 5,000 tasks,
 * 10,000 notes, a 1,000-turn thread) in a production-mode mock build
 * (playwright.perf.config.ts). Prints one `S7_PERF {json}` line:
 *   launch      navigation to the shell, and to interactive (no long task for 1 s)
 *   switch      a top-level page switch, the click event to the next painted
 *               frame (main-thread ms over a window to 400 ms after it)
 *   openLong    opening the 1,000-turn thread, Enter to its newest turn painted
 *               (the session's first open apart: openLongFirst)
 *   typing      keydown to the next frame in that thread's composer, and
 *               main-thread CPU per key, over three bursts
 *   stream      a 24 KB markdown reply streamed in 475 deltas on a fixed
 *               60-a-second schedule: main-thread CPU, task time and its
 *               script, layout and style parts, frames over 50 ms, how long
 *               delivery took, and the lag from the last delta to that text
 *               on screen (which includes ChatMessage's 80 ms render
 *               throttle); then the same as prose alone (streamProse*)
 * The `*Cpu` figures are the main thread's CPU time (Chrome's
 * Performance.getMetrics ThreadTime), which counts script, style and layout
 * whatever triggered them, and which a busy machine inflates far less than
 * wall-clock task time (`*Task`). The stream also reports how many layouts
 * and style recalculations ran.
 * Frontend only (mock IPC, Chromium): compare builds with it, not with the app.
 *   PW_PORT=<port> npx playwright test --config=tests/perf/playwright.perf.config.ts
 * RUNS (default 5) sets the samples for launch, openLong and stream;
 * STREAM_KB (default 24) the reply's size; SECTIONS (default all) which
 * measures run. PROFILE=1 adds the top self-time source files (sourcemaps)
 * of one extra switch of each page, a typing burst and one extra stream of
 * each kind, none of them counted.
 */

const RUNS = Number(process.env.RUNS ?? 5);
/** SECTIONS (default all) picks measures: launch, switch, open, typing, stream. */
const SECTIONS = (process.env.SECTIONS ?? 'launch,switch,open,typing,stream').split(',');
const on = (section: string) => SECTIONS.includes(section);
const STREAM_KB = Number(process.env.STREAM_KB ?? 24);
const PROFILE = process.env.PROFILE === '1';
const ROOT = fileURLToPath(new URL('../..', import.meta.url));
const DIST = `${ROOT}node_modules/.cache/hollow-perf-dist`;

interface CpuProfile {
  nodes: { id: number; callFrame: { url: string; functionName: string; lineNumber: number; columnNumber: number } }[];
  samples: number[];
  timeDeltas: number[];
}

/** Self time per original source file and function of a CPU profile, through
 *  the build's sourcemaps (scripts/perf-switch.mjs). */
function attribute(profile: CpuProfile, base: string) {
  const require = createRequire(import.meta.url);
  const { SourceMapConsumer } = require('source-map-js');
  const maps = new Map<string, { originalPositionFor(p: { line: number; column: number }): { source: string | null; name: string | null } } | null>();
  const mapFor = (url: string) => {
    if (maps.has(url)) return maps.get(url)!;
    const file = url.startsWith(base) ? `${DIST}${new URL(url).pathname}` : null;
    const m = file && existsSync(`${file}.map`) ? new SourceMapConsumer(JSON.parse(readFileSync(`${file}.map`, 'utf8'))) : null;
    maps.set(url, m);
    return m;
  };
  const self = new Map<string, number>();
  const byId = new Map(profile.nodes.map(n => [n.id, n]));
  for (let i = 0; i < profile.samples.length; i++) {
    const f = byId.get(profile.samples[i])!.callFrame;
    let key = f.url ? f.url.replace(base, '') : `(${f.functionName || 'program'})`;
    const m = f.url ? mapFor(f.url) : null;
    if (m) {
      const o = m.originalPositionFor({ line: f.lineNumber + 1, column: f.columnNumber });
      if (o.source) key = `${o.source.replace(/^.*?\/(src|node_modules)\//, '$1/')}${o.name ? ` ${o.name}` : ''}`;
    }
    self.set(key, (self.get(key) ?? 0) + (profile.timeDeltas[i] ?? 0) / 1000);
  }
  return [...self].sort((a, b) => b[1] - a[1]).slice(0, 16).map(([k, ms]) => `${ms.toFixed(1)} ms  ${k}`);
}

/** The `p`th percentile of `a` (nearest rank), one decimal. */
const pct = (a: number[], p: number) => {
  if (!a.length) return null;
  const s = [...a].sort((x, y) => x - y);
  return +s[Math.min(s.length - 1, Math.max(0, Math.ceil(p / 100 * s.length) - 1))].toFixed(1);
};
const summary = (a: number[]) => ({ n: a.length, p50: pct(a, 50), p90: pct(a, 90), max: a.length ? +Math.max(...a).toFixed(1) : null });

/** Tauri event delivery on top of the `?mock=1` IPC (as long-chat.spec.ts,
 *  with the smoke fixture's unlisten shim, so fresh launch pages get it too),
 *  a long-task recorder, and the time of the latest click or key press. */
function instrument() {
  const callbacks = new Map<number, (event: unknown) => void>();
  const listeners = new Map<string, number[]>();
  let next = 1;
  const w = window as unknown as Record<string, unknown>;
  w.__TAURI_EVENT_PLUGIN_INTERNALS__ ??= { unregisterListener() { /* the fake below has no listeners to drop */ } };
  w.__inputAt = 0;
  for (const type of ['click', 'keydown']) document.addEventListener(type, e => { w.__inputAt = e.timeStamp; }, { capture: true });
  w.__TAURI_INTERNALS__ = {
    transformCallback(callback: (event: unknown) => void) { const id = next++; callbacks.set(id, callback); return id; },
    invoke(command: string, args: { event?: string; handler?: number }) {
      if (command === 'plugin:event|listen' && args?.event && args.handler) {
        listeners.set(args.event, [...(listeners.get(args.event) ?? []), args.handler]);
        return Promise.resolve(next++);
      }
      if (command === 'plugin:event|unlisten') return Promise.resolve();
      return Promise.reject(new Error(`not in Tauri: ${command}`));
    },
  };
  w.__emitTauriEvent = (event: string, payload: unknown) => {
    for (const id of listeners.get(event) ?? []) callbacks.get(id)?.({ event, id, payload });
  };
  const long: { start: number; duration: number }[] = [];
  w.__long = long;
  new PerformanceObserver(l => { for (const e of l.getEntries()) long.push({ start: e.startTime, duration: e.duration }); }).observe({ type: 'longtask', buffered: true });
}

/** Resolves after the next two animation frames (the frame that paints what
 *  is in the DOM now has been produced). */
const painted = (page: Page) => page.evaluate(() => new Promise<number>(r => requestAnimationFrame(() => requestAnimationFrame(() => r(performance.now())))));
/** From the latest click or key press event (not Playwright's actionability
 *  checks before it) to the next painted frame. */
const sinceInput = async (page: Page) => {
  const at = await painted(page);
  return at - await page.evaluate(() => (window as unknown as { __inputAt: number }).__inputAt);
};
/** Waits until no long task has run for `quiet` ms after `from` (or the
 *  deadline passes) and returns the long tasks seen. */
const quietLongTasks = (page: Page, from: number, quiet = 1000, deadline = 10_000) => page.evaluate(async ({ from, quiet, deadline }) => {
  const w = window as unknown as { __long: { start: number; duration: number }[] };
  const end = () => w.__long.reduce((a, t) => Math.max(a, t.start + t.duration), from);
  while (performance.now() - end() < quiet && performance.now() - from < deadline) await new Promise(r => setTimeout(r, 100));
  // Judged before the recorder is drained (end() reads it).
  const wasQuiet = performance.now() - end() >= quiet;
  return { long: w.__long.splice(0), quiet: wasQuiet };
}, { from, quiet, deadline });
const takeLong = (page: Page) => page.evaluate(() => {
  const w = window as unknown as { __long: { start: number; duration: number }[] };
  return w.__long.splice(0);
});

/** Chrome's running totals: main-thread task time (wall clock, so it grows
 *  when the machine is busy) and its script, layout and style parts, in ms;
 *  the main thread's CPU time (`ThreadTime`, ms), which load inflates far
 *  less; and how many layouts and style recalculations ran. */
async function threadTotals(cdp: CDPSession) {
  const { metrics } = await cdp.send('Performance.getMetrics');
  const m = Object.fromEntries(metrics.map(x => [x.name, x.value]));
  return {
    task: (m.TaskDuration ?? 0) * 1000, script: (m.ScriptDuration ?? 0) * 1000, layout: (m.LayoutDuration ?? 0) * 1000, style: (m.RecalcStyleDuration ?? 0) * 1000,
    cpu: (m.ThreadTime ?? 0) * 1000, layouts: m.LayoutCount ?? 0, styles: m.RecalcStyleCount ?? 0,
  };
}
/** The main thread's CPU ms so far (the `*Cpu` figures). */
const mainThreadMs = async (cdp: CDPSession) => (await threadTotals(cdp)).cpu;

/** Time to interactive: the end of the last long task that starts before a
 *  quiet window of `quiet` ms with none, never before `from`
 *  (scripts/perf-launch-chat.mjs). */
function interactiveAt(longTasks: { start: number; duration: number }[], from: number, quiet = 1000) {
  let at = from;
  for (const t of [...longTasks].sort((x, y) => x.start - y.start)) {
    if (t.start - at >= quiet) break;
    at = Math.max(at, t.start + t.duration);
  }
  return at;
}

/** A long reply's markdown: headings, prose, lists, a table and (unless
 *  `prose`) a code fence every third section, about `kb` KB, with a unique
 *  last line to wait for. A fence splits the reply's rendering into
 *  segments; prose alone is one segment that grows to the whole answer. */
function longReply(kb: number, prose = false) {
  const parts: string[] = [];
  let i = 0;
  while (parts.join('\n\n').length < kb * 1024) {
    i++;
    parts.push(`## Section ${i}: what changed`);
    parts.push(`The **${i}th** pass over the garage leases found that the *variance* in location ${i * 7} comes from a late invoice, not from a rate change. See \`ledger-${i}.csv\` and [the note](notes/lease-${i}.md) for the detail.`);
    parts.push(['- Rate held at the contract figure', '- Two invoices arrived after month end', `- Location ${i * 7} needs a manual accrual`].join('\n'));
    if (i % 3 === 0 && !prose) parts.push(['```ts', `const accrual${i} = invoices.filter(x => x.late).reduce((a, x) => a + x.amount, 0);`, `console.log(accrual${i});`, '```'].join('\n'));
    if (i % 4 === 0) parts.push(['| Month | Billed | Paid |', '|---|---|---|', `| Aug | $${i}1,200 | $${i}1,150 |`, `| Sep | $${i}1,240 | $${i}1,240 |`].join('\n'));
  }
  parts.push('That is the last line of the streamed answer: stream-end-marker.');
  return parts.join('\n\n');
}

const MAIN_NAV = (page: Page) => page.getByRole('navigation', { name: 'Main', exact: true });

/** Enter in the palette to the thread's newest turn painted. */
async function openLongThread(page: Page) {
  // The palette from the sidebar's search entry (the top bar's Search anything went in U1).
  await openGlobalSearch(page);
  await page.getByPlaceholder(/Find a conversation/).fill('Long design thread');
  await page.locator('[data-palette-idx]', { hasText: 'Long design thread' }).first().waitFor();
  await page.keyboard.press('Enter');
  await page.locator('.turn-content', { hasText: 'Reviewed part 499' }).first().waitFor({ timeout: 15_000 });
  return sinceInput(page);
}

test('S7 frontend responsiveness probe', async ({ page, browser }) => {
  test.setTimeout(900_000);
  const out: Record<string, unknown> = { runs: RUNS };

  // 1. Launch: a fresh context per run (cold cache), navigation to the shell
  //    and to interactive (a full quiet second observed, 10 s at most). These
  //    pages are not the fixture's, so their page errors are checked here.
  const shell: number[] = [], interactive: number[] = [];
  const launchErrors: string[] = [];
  let inconclusive = 0;
  for (let run = 0; run < (on('launch') ? RUNS : 0); run++) {
    const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    const p = await ctx.newPage();
    p.on('pageerror', e => { if (!/ResizeObserver loop/.test(e.message)) launchErrors.push(`${e.name}: ${e.message}`); });
    await p.emulateMedia({ reducedMotion: 'reduce' });
    await p.addInitScript(instrument);
    await p.goto('/?mock=1&demo=large');
    await MAIN_NAV(p).waitFor();
    await p.getByRole('heading', { name: 'Projects', exact: true }).first().waitFor();
    const shellAt = await painted(p);
    const settled = await quietLongTasks(p, shellAt);
    if (!settled.quiet) inconclusive++;
    shell.push(shellAt);
    interactive.push(interactiveAt(settled.long, shellAt));
    await ctx.close();
  }
  expect(launchErrors, 'page errors during launch').toEqual([]);
  out.launchShell = summary(shell);
  out.launchInteractive = summary(interactive);
  out.launchNoQuietSecond = inconclusive;

  // The rest runs in one page, as a person's session does.
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.addInitScript(instrument);
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Performance.enable');
  await page.goto('/?mock=1&demo=large');
  await MAIN_NAV(page).waitFor();
  await page.waitForTimeout(3000);

  // 2. Page switches: Tasks, Review, Home, ten cycles; the click event to the
  //    next frame. The main-thread window runs from before the click to 400 ms
  //    after that frame (what the switch leaves running counts too).
  //    Needs you (was Review) has no rail row since U3: the bell opens first,
  //    outside the measured window, and its See all needs you is the click.
  const bellSeeAll = (p: Page) => p.getByRole('dialog', { name: 'Notifications', exact: true }).getByRole('button', { name: 'See all needs you', exact: true });
  const pages: { name: string; open?: (p: Page) => Promise<unknown>; button: (p: Page) => ReturnType<Page['locator']>; ready: (p: Page) => Promise<unknown> }[] = [
    { name: 'tasks', button: p => MAIN_NAV(p).getByRole('button', { name: /^Tasks/ }).first(), ready: p => p.locator('.issue-row').first().waitFor() },
    { name: 'review', open: async p => { await p.locator('.top-bar').getByRole('button', { name: 'Notifications', exact: true }).click(); await bellSeeAll(p).waitFor(); },
      button: bellSeeAll, ready: p => p.getByRole('heading', { name: 'Needs you', level: 1 }).first().waitFor() },
    { name: 'home', button: p => MAIN_NAV(p).getByRole('button', { name: /^(Home|Work|Projects)/ }).first(), ready: p => p.getByRole('heading', { name: 'Projects', exact: true }).first().waitFor() },
  ];
  const switches: Record<string, number[]> = { tasks: [], review: [], home: [] };
  const switchCpu: Record<string, number[]> = { tasks: [], review: [], home: [] };
  for (let cycle = 0; cycle < (on('switch') ? 11 : 0); cycle++) {
    for (const target of pages) {
      await target.open?.(page);
      const cpu0 = await mainThreadMs(cdp);
      await target.button(page).click();
      await target.ready(page);
      const ms = await sinceInput(page);
      await page.waitForTimeout(400);
      const cpu = (await mainThreadMs(cdp)) - cpu0;
      // The first cycle loads each page's chunk: reported apart.
      if (cycle === 0) out[`switchFirst_${target.name}`] = Math.round(ms);
      else { switches[target.name].push(ms); switchCpu[target.name].push(cpu); }
    }
  }
  for (const k of Object.keys(switches)) { out[`switch_${k}`] = summary(switches[k]); out[`switchWindowCpu_${k}`] = summary(switchCpu[k]); }
  const base = new URL(page.url()).origin;
  if (PROFILE) await cdp.send('Profiler.enable');
  if (PROFILE && on('switch')) {
    for (const target of pages) {
      await cdp.send('Profiler.start');
      await MAIN_NAV(page).getByRole('button', { name: target.button }).first().click();
      await target.ready(page);
      await painted(page);
      await page.waitForTimeout(400);
      out[`profile_switch_${target.name}`] = attribute((await cdp.send('Profiler.stop')).profile as CpuProfile, base);
    }
  }
  await takeLong(page);

  // 3. Opening the 1,000-turn thread from Home, RUNS times. Its main-thread
  //    window runs from before the palette opens to 1.5 s after the thread
  //    painted.
  const open: number[] = [], openCpu: number[] = [], openLong: number[] = [];
  // Typing and the streams need the thread open, at least once.
  const thread = on('open') || on('typing') || on('stream');
  for (let run = 0; run < (on('open') ? RUNS : thread ? 1 : 0); run++) {
    await MAIN_NAV(page).getByRole('button', { name: /^(Home|Work|Projects)/ }).first().click();
    await page.getByRole('heading', { name: 'Projects', exact: true }).first().waitFor();
    await page.waitForTimeout(1000);
    await takeLong(page);
    const cpu0 = await mainThreadMs(cdp);
    open.push(await openLongThread(page));
    await page.waitForTimeout(1500);
    openCpu.push((await mainThreadMs(cdp)) - cpu0);
    openLong.push((await takeLong(page)).reduce((a, x) => a + x.duration, 0));
  }
  if (on('open')) {
    out.openLong = summary(open);
    // The session's first chat open (its chunks may still be loading), apart.
    out.openLongFirst = Math.round(open[0]);
    out.openLongFirstWindowCpu = Math.round(openCpu[0]);
    out.openLongWindowCpu = summary(openCpu);
    out.openLongTaskMs = summary(openLong);
  }

  // 4. Typing in that thread's composer once it has settled: keydown to the
  //    next frame, and main-thread ms per key.
  const composer = page.getByLabel('Chat message');
  if (thread) {
    await composer.click();
    await page.waitForTimeout(2000);
    await page.evaluate(() => {
      const w = window as unknown as { __keys: number[] };
      w.__keys = [];
      document.addEventListener('keydown', e => {
        const t0 = e.timeStamp;
        requestAnimationFrame(() => setTimeout(() => w.__keys.push(performance.now() - t0), 0));
      }, { capture: true });
    });
  }
  const text = 'Checking typing latency in a long conversation with many turns above it.';
  if (on('typing')) {
    // Three bursts; CPU per key is per burst (the 300 ms after it included).
    const perKey: number[] = [];
    for (let burst = 0; burst < 3; burst++) {
      const typingCpu0 = await mainThreadMs(cdp);
      await page.keyboard.type(text, { delay: 40 });
      await page.waitForTimeout(300);
      perKey.push(((await mainThreadMs(cdp)) - typingCpu0) / text.length);
      await composer.fill('');
      await page.waitForTimeout(300);
    }
    const keys = await page.evaluate(() => (window as unknown as { __keys: number[] }).__keys);
    out.typingKeyToFrame = summary(keys);
    out.typingCpuPerKey = summary(perKey);
    if (PROFILE) {
      await cdp.send('Profiler.start');
      await page.keyboard.type(text, { delay: 40 });
      await page.waitForTimeout(300);
      out.profile_typing = attribute((await cdp.send('Profiler.stop')).profile as CpuProfile, base);
      await composer.fill('');
    }
  }
  await takeLong(page);

  // 5. A long streamed reply in that thread, RUNS times (plus one profiled),
  //    twice: `stream*` with a code fence every third section, then
  //    `streamProse*` with none (one segment that grows to the whole answer).
  for (const prose of [false, true]) if (on('stream')) await streamRuns(prose);

  async function streamRuns(prose: boolean) {
  const key = prose ? 'streamProse' : 'stream';
  const reply = longReply(STREAM_KB, prose);
  const chunks: string[] = [];
  for (let at = 0; at < reply.length; at += 52) chunks.push(reply.slice(at, at + 52));
  out[`${key}Chunks`] = chunks.length;
  out[`${key}Bytes`] = reply.length;
  const streamCpu: number[] = [], streamLag: number[] = [], streamGaps: number[] = [], streamLongMs: number[] = [];
  const streamScript: number[] = [], streamLayout: number[] = [], streamStyle: number[] = [], streamDelivery: number[] = [];
  const streamTask: number[] = [], streamLayouts: number[] = [], streamStyles: number[] = [];
  for (let run = 0; run < RUNS + (PROFILE ? 1 : 0); run++) {
    const profiling = PROFILE && run === RUNS;
    const sends = await page.evaluate(() => (window as unknown as { __mockApiCalls?: Record<string, number> }).__mockApiCalls?.chat_send ?? 0);
    await composer.fill(`Stream probe ${run + 1}: summarise the garage lease variances.`);
    await composer.press('Meta+Enter');
    await expect.poll(() => page.evaluate(() => (window as unknown as { __mockApiCalls?: Record<string, number> }).__mockApiCalls?.chat_send ?? 0)).toBe(sends + 1);
    await page.evaluate(runId => (window as unknown as { __emitTauriEvent: (n: string, p: unknown) => void }).__emitTauriEvent('chat-event', { type: 'runStarted', asked: 'probe', runId, recall: [] }), `run-s7-${key}-${run}`);
    await page.waitForTimeout(500);
    await takeLong(page);
    if (profiling) await cdp.send('Profiler.start');
    const cpu0 = await threadTotals(cdp);
    // Deltas at 60 a second from inside the page, each due at a fixed time
    // from the start (a slow build is not handed a slower stream; one that
    // falls behind catches up), with a frame-gap recorder. The lag runs from
    // just before the last delta to it on screen, so it includes where that
    // delta lands in ChatMessage's 80 ms render throttle.
    const r = await page.evaluate(async parts => {
      const w = window as unknown as { __emitTauriEvent: (n: string, p: unknown) => void };
      const gaps: number[] = [];
      let last = performance.now(), on = true;
      const frame = (t: number) => { gaps.push(t - last); last = t; if (on) requestAnimationFrame(frame); };
      requestAnimationFrame(frame);
      const start = performance.now();
      let sent = start;
      for (let i = 0; i < parts.length; i++) {
        const wait = start + i * (1000 / 60) - performance.now();
        if (wait > 0) await new Promise(res => setTimeout(res, wait));
        sent = performance.now();
        w.__emitTauriEvent('chat-event', { type: 'assistantDelta', text: parts[i] });
      }
      const delivered = performance.now() - start;
      // The last line on screen.
      await new Promise<void>(res => {
        const tick = () => {
          const els = document.querySelectorAll('.turn-content');
          const lastEl = els[els.length - 1];
          if (lastEl?.textContent?.includes('stream-end-marker')) res(); else requestAnimationFrame(tick);
        };
        tick();
      });
      const shown = await new Promise<number>(res => requestAnimationFrame(() => requestAnimationFrame(() => res(performance.now()))));
      on = false;
      return { lag: shown - sent, slowFrames: gaps.filter(g => g > 50).length, delivered };
    }, chunks);
    if (profiling) out[`profile_${key}`] = attribute((await cdp.send('Profiler.stop')).profile as CpuProfile, base);
    else {
      const cpu = await threadTotals(cdp);
      streamCpu.push(cpu.cpu - cpu0.cpu);
      streamTask.push(cpu.task - cpu0.task);
      streamLayouts.push(cpu.layouts - cpu0.layouts);
      streamStyles.push(cpu.styles - cpu0.styles);
      streamScript.push(cpu.script - cpu0.script);
      streamLayout.push(cpu.layout - cpu0.layout);
      streamStyle.push(cpu.style - cpu0.style);
      streamLag.push(r.lag);
      streamGaps.push(r.slowFrames);
      streamDelivery.push(r.delivered);
      streamLongMs.push((await takeLong(page)).reduce((a, x) => a + x.duration, 0));
    }
    await page.evaluate(() => (window as unknown as { __emitTauriEvent: (n: string, p: unknown) => void }).__emitTauriEvent('chat-event', { type: 'done', sessionId: null }));
    await page.waitForTimeout(1500);
  }
  out[`${key}Cpu`] = summary(streamCpu);
  out[`${key}Task`] = summary(streamTask);
  out[`${key}Layouts`] = summary(streamLayouts);
  out[`${key}StyleRecalcs`] = summary(streamStyles);
  out[`${key}Script`] = summary(streamScript);
  out[`${key}Layout`] = summary(streamLayout);
  out[`${key}Style`] = summary(streamStyle);
  out[`${key}LastDeltaToScreen`] = summary(streamLag);
  // Ideally (chunks - 1) / 60 s; more means the page fell behind the stream.
  out[`${key}DeliveryMs`] = summary(streamDelivery);
  out[`${key}SlowFrames`] = summary(streamGaps);
  out[`${key}LongTaskMs`] = summary(streamLongMs);
  }

  console.log(`S7_PERF ${JSON.stringify(out)}`);
});
