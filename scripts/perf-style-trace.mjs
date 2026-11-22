#!/usr/bin/env node
// A13 (overnight 10-15/16): a style-invalidation trace of a warm Home -> Tasks
// switch, to find which selectors or root class flips cause its style recalc
// (docs/verification-2026-10-14/MEASURES.md, "Recheck 2026-10-16"). Browser
// mock on the scale fixture, Chromium, frontend only. Navigation and "shown"
// detection are perf-switch.mjs's. Needs a `build:mock` served by one
// `vite preview` (no other dev or preview server up):
//   npm run build:mock && node node_modules/vite/bin/vite.js preview --host 127.0.0.1 --port 1459 --strictPort
//   BASE=http://127.0.0.1:1459 node scripts/perf-style-trace.mjs > report.json
// Env: CYCLES (default 6); SETTLE, ms after Home shows before the measured
// click (default 150, perf-switch's pause; Home's row entrance runs ~300 ms,
// so 950 keeps it out); TRACE=0 times the switch without tracing (tracing
// roughly doubles style-recalc time); VARIANT_CSS (a style tag) and VARIANT_JS
// (evaluated once) try a change without a build; LABEL is echoed. The trace
// covers the click until Tasks shows; per cycle the report lists every style
// recalc on the page's main thread (ms after the click, duration, elements).
// The last raw trace goes to $TRACE_OUT (default <tmpdir>/perf-style-trace.json).
import { mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { chromium } from 'playwright';
import { pct } from './perf-launch-chat.mjs';

const BASE = process.env.BASE ?? 'http://127.0.0.1:1459';
const CYCLES = Number(process.env.CYCLES ?? 6);
const VARIANT_CSS = process.env.VARIANT_CSS ?? '';
const SETTLE = Number(process.env.SETTLE ?? 150);
const TRACE = process.env.TRACE !== '0';
const VARIANT_JS = process.env.VARIANT_JS ?? '';
const LABEL = process.env.LABEL ?? (VARIANT_CSS || VARIANT_JS ? 'variant' : 'control');
const TRACE_OUT = process.env.TRACE_OUT ?? join(tmpdir(), 'perf-style-trace.json');
const CATEGORIES = ['devtools.timeline', 'disabled-by-default-devtools.timeline',
  'disabled-by-default-devtools.timeline.invalidationTracking', 'blink.debug'];
const INVALIDATION = new Set(['StyleInvalidatorInvalidationTracking', 'StyleRecalcInvalidationTracking', 'ScheduleStyleInvalidationTracking']);
const RECALC = new Set(['UpdateLayoutTree', 'RecalculateStyles']);

// Steps as in perf-switch.mjs: 'home' returns via sidebar Work (lands on Workspaces), 'tasks' from Home.
const HOME = { name: 'home', click: ['nav[aria-label="Main"] button', 'Projects'], ready: ['[aria-label="Pick up where you left off"]:not([aria-busy="true"])', ''] };
const TASKS = { name: 'tasks', click: ['nav[aria-label="Main"] button', 'Tasks'], ready: ['h1', 'Tasks'] };

// Verbatim from perf-switch.mjs run(), minus the Enter branch.
const run = (page, s) => page.evaluate(async ({ click, ready }) => {
  const find = ([sel, text], exact) => [...document.querySelectorAll(sel)].find(e => {
    const t = (e.textContent ?? '').trim();
    return !text || (exact ? t === text : t.includes(text));
  });
  const shown = () => { const e = find(ready, ready[0] === 'h1'); return !!e && e.getClientRects().length > 0; };
  const frame = () => new Promise(r => requestAnimationFrame(r));
  const t0 = performance.now();
  const target = find(click, false); if (!target) return null; target.click();
  while (!shown()) { await frame(); if (performance.now() - t0 > 15000) return null; }
  await frame(); await frame();
  const ms = +(performance.now() - t0).toFixed(1);
  await frame();
  return ms;
}, { click: s.click, ready: s.ready });

async function step(page, s, wait = 150) {
  if (wait > 0) await page.waitForTimeout(wait);
  const ms = await run(page, s);
  if (ms === null) throw new Error(`perf-style-trace: ${s.name} never showed`);
  return ms;
}

async function readStream(cdp, handle) {
  const chunks = [];
  for (;;) {
    const { data, eof, base64Encoded } = await cdp.send('IO.read', { handle, size: 1 << 20 });
    if (data) chunks.push(base64Encoded ? Buffer.from(data, 'base64') : Buffer.from(data, 'utf8'));
    if (eof) break;
  }
  await cdp.send('IO.close', { handle }).catch(() => {});
  return Buffer.concat(chunks).toString('utf8');
}

async function traced(cdp, fn) {
  await cdp.send('Tracing.start', { transferMode: 'ReturnAsStream', traceConfig: { includedCategories: CATEGORIES } });
  let result;
  let stream;
  try { result = await fn(); } finally {
    const done = new Promise(r => cdp.once('Tracing.tracingComplete', r));
    await cdp.send('Tracing.end');
    ({ stream } = await done);
  }
  if (!stream) throw new Error('perf-style-trace: tracingComplete had no stream');
  return { result, text: await readStream(cdp, stream) };
}

// Compact one invalidation set / list entry into "{.a .b #id tag [attr] !flags}".
const compactSet = s => {
  // A bare pointer ("0x12c0004df80") names no set and differs per run.
  if (!s || typeof s !== 'object') return s == null || /^0x[0-9a-f]+$/i.test(String(s)) ? '' : String(s);
  const parts = [...(s.classes ?? []).map(c => '.' + c), ...(s.ids ?? []).map(i => '#' + i),
    ...(s.tagNames ?? []), ...(s.attributes ?? []).map(a => `[${a}]`)];
  for (const f of ['allDescendantsMightBeInvalid', 'customPseudoInvalid', 'treeBoundaryCrossing', 'insertionPointCrossing', 'invalidatesSlotted', 'invalidatesParts', 'subtree'])
    if (s[f]) parts.push('!' + f);
  return `{${parts.join(' ')}}`;
};
const asList = v => (Array.isArray(v) ? v : v ? [v] : []);

function invalidationKey(e) {
  const d = e.args?.data ?? {};
  const changed = [d.changedClass && '.' + d.changedClass, d.changedId && '#' + d.changedId,
    d.changedAttribute && `[${d.changedAttribute}]`, d.changedPseudo && ':' + d.changedPseudo].filter(Boolean).join(' ');
  const sets = [...asList(d.invalidationList), ...asList(d.invalidationSet)].map(compactSet).filter(x => x && x !== '{}');
  const sels = asList(d.selectors).map(s => (typeof s === 'string' ? s : s?.selector)).filter(Boolean);
  const extra = typeof d.extraData === 'string' ? d.extraData : '';
  return [e.name.replace('InvalidationTracking', ''), d.reason ?? '', changed && `changed=${changed}`,
    sets.length && `sets=${[...new Set(sets)].join(',')}`, sels.length && `selectors=${[...new Set(sels)].join(' | ')}`,
    d.selectorPart && `part=${d.selectorPart}`, extra && `extra=${extra}`].filter(Boolean).join(' | ');
}

function analyse(text, inv) {
  const parsed = JSON.parse(text);
  const events = Array.isArray(parsed) ? parsed : parsed.traceEvents ?? [];
  // Renderer main thread: the CrRendererMain with the most style-recalc events.
  const mains = new Set(events.filter(e => e.ph === 'M' && e.name === 'thread_name' && e.args?.name === 'CrRendererMain').map(e => `${e.pid}:${e.tid}`));
  const recalc = new Map();
  const open = new Map();
  for (const e of events) {
    if (!RECALC.has(e.name)) continue;
    const tk = `${e.pid}:${e.tid}`;
    if (mains.size && !mains.has(tk)) continue;
    const add = (ts, dur, args) => {
      const r = recalc.get(tk) ?? { tk, ms: 0, elements: 0, n: 0, list: [] };
      const elements = Number(args?.elementCount ?? args?.data?.elementCount ?? args?.endData?.elementCount ?? 0);
      r.ms += dur / 1000; r.n++; r.elements += elements;
      r.list.push({ ts, ms: +(dur / 1000).toFixed(2), elements });
      recalc.set(tk, r);
    };
    if (e.ph === 'X') add(e.ts, e.dur ?? 0, e.args);
    else if (e.ph === 'B') open.set(tk + e.name, e);
    else if (e.ph === 'E' && open.has(tk + e.name)) { const b = open.get(tk + e.name); open.delete(tk + e.name); add(b.ts, e.ts - b.ts, { ...b.args, ...e.args }); }
  }
  const main = [...recalc.values()].sort((a, b) => b.n - a.n)[0] ?? { tk: '', ms: 0, elements: 0, n: 0, list: [] };
  // Times relative to the measured click (its dispatch on the same thread).
  const click = events.find(e => e.name === 'EventDispatch' && e.args?.data?.type === 'click' && `${e.pid}:${e.tid}` === main.tk);
  const zero = click?.ts ?? main.list[0]?.ts ?? 0;
  const recalcs = main.list.sort((a, b) => a.ts - b.ts).map(r => ({ atMs: +((r.ts - zero) / 1000).toFixed(1), ms: r.ms, elements: r.elements }));
  let invCount = 0;
  for (const e of events) {
    if (!INVALIDATION.has(e.name)) continue;
    invCount++;
    const k = invalidationKey(e);
    const g = inv.get(k) ?? { count: 0, nodes: new Map() };
    g.count++;
    const node = e.args?.data?.nodeName ?? '(none)';
    g.nodes.set(node, (g.nodes.get(node) ?? 0) + 1);
    inv.set(k, g);
  }
  return { ms: +main.ms.toFixed(2), elements: main.elements, recalcEvents: main.n, invalidationEvents: invCount, totalEvents: events.length, recalcs };
}

async function main() {
  mkdirSync(dirname(TRACE_OUT), { recursive: true });
  let browser;
  try {
    browser = await chromium.launch({ headless: true });
    const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    const page = await ctx.newPage();
    await page.goto(`${BASE}/?mock=1&demo=large`);
    await page.getByRole('region', { name: 'Pick up where you left off' }).waitFor({ timeout: 20_000 });
    if (VARIANT_CSS) await page.addStyleTag({ content: VARIANT_CSS });
    if (VARIANT_JS) await page.evaluate(VARIANT_JS);
    await page.waitForTimeout(2000);
    for (let i = 0; i < 2; i++) { await step(page, TASKS); await step(page, HOME); }
    const cdp = await page.context().newCDPSession(page);
    const inv = new Map();
    const perCycle = [];
    let lastText = '';
    for (let c = 0; c < CYCLES; c++) {
      await page.waitForTimeout(SETTLE);
      if (!TRACE) { perCycle.push({ switchMs: await step(page, TASKS, 0), ms: null, elements: null }); await step(page, HOME); continue; }
      const { result: switchMs, text } = await traced(cdp, () => step(page, TASKS, 0));
      lastText = text;
      perCycle.push({ switchMs, ...analyse(text, inv) });
      await step(page, HOME);
    }
    if (lastText) writeFileSync(TRACE_OUT, lastText);
    const ms = perCycle.map(p => p.ms);
    const report = {
      label: LABEL, settle: SETTLE, trace: TRACE, base: BASE, cycles: CYCLES, variantCss: VARIANT_CSS || null, variantJs: VARIANT_JS || null,
      switchMs: { median: pct(perCycle.map(p => p.switchMs), 50), p90: pct(perCycle.map(p => p.switchMs), 90) },
      styleRecalcMs: TRACE ? { median: pct(ms, 50), p90: pct(ms, 90), values: ms } : null,
      elementCount: TRACE ? { median: pct(perCycle.map(p => p.elements), 50) } : null,
      perCycle,
      topInvalidations: [...inv].sort((a, b) => b[1].count - a[1].count).slice(0, 25).map(([key, g]) => ({
        key, count: g.count, perCycle: +(g.count / CYCLES).toFixed(1),
        nodes: [...g.nodes].sort((a, b) => b[1] - a[1]).slice(0, 5).map(([n, k]) => `${n} x${k}`),
      })),
    };
    console.log(JSON.stringify(report, null, 2));
  } finally {
    await browser?.close().catch(() => {});
  }
}

main().catch(err => { console.error(err?.stack ?? err); process.exitCode = 1; });
