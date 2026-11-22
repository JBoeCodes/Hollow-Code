#!/usr/bin/env node
// Plan completion L5 (review roadmap "Measures", matrix V7f): before/after
// timings of the navigation the plan changed, with sample sizes and tails.
// Frontend only: a `build:mock` served by `vite preview`, the browser mock's
// scale fixture (`?mock=1&demo=large`), Chromium. Never native evidence and
// never comparable with the app's own numbers; compare two builds measured
// the same way, on the same machine, back to back.
//   BASE=http://127.0.0.1:1495 RUNS=12 node scripts/perf-plan-measures.mjs > head.json
// Prints JSON: per measure n, median, p90, max and the raw samples (ms).
//   launch.shell      navigation start → the Work heading
//   launch.useful     navigation start → the first "Pick up where you left off" card
//   switch.<page>     a rail click → that page's heading (Tasks, Review, Library, Automations, Work)
//   longChat          a rail click on a 1,000-turn conversation → its last turn drawn
import { chromium } from 'playwright';

const BASE = process.env.BASE ?? 'http://127.0.0.1:1495';
const RUNS = Number(process.env.RUNS ?? 12);

export const pct = (a, p) => {
  if (!a.length) return null;
  const s = [...a].sort((x, y) => x - y);
  return +s[Math.min(s.length - 1, Math.max(0, Math.ceil(p / 100 * s.length) - 1))].toFixed(1);
};
export const summary = a => ({ n: a.length, median: pct(a, 50), p90: pct(a, 90), max: a.length ? +Math.max(...a).toFixed(1) : null, samples: a.map(x => +x.toFixed(1)) });

/** In the page: click `selector`'s button named `name`, then time until `ready()` holds (checked every frame). */
async function timedClick(page, scope, name, readySrc) {
  return page.evaluate(async ({ scope, name, readySrc }) => {
    const ready = new Function(`return (${readySrc})()`);
    const root = document.querySelector(scope);
    const btn = [...(root?.querySelectorAll('button') ?? [])].find(b => (b.getAttribute('aria-label') ?? b.textContent ?? '').trim().includes(name));
    if (!btn) throw new Error(`no button ${name} in ${scope}`);
    const t0 = performance.now();
    btn.click();
    for (;;) {
      await new Promise(r => requestAnimationFrame(r));
      if (ready()) return performance.now() - t0;
      if (performance.now() - t0 > 15_000) throw new Error(`${name}: not ready after 15 s`);
    }
  }, { scope, name, readySrc: readySrc.toString() });
}

const heading = text => `() => [...document.querySelectorAll('main h1, main [role=heading][aria-level="1"]')].some(h => h.textContent.trim().startsWith(${JSON.stringify(text)}) && h.getBoundingClientRect().height > 0)`;
const PAGES = [
  ['Tasks', heading('Tasks')],
  ['Review', `() => !!document.querySelector('[role=region][aria-label="Waiting on you"]')`],
  ['Library', heading('Library')],
  ['Automations', heading('Automations')],
  ['Work', `() => !!document.querySelector('.home-project-row')`],
];

async function main() {
  const browser = await chromium.launch();
  const out = { base: BASE, runs: RUNS, fixture: '?mock=1&demo=large', browser: `chromium ${browser.version()}`, at: new Date().toISOString() };
  const shell = [], useful = [], switches = Object.fromEntries(PAGES.map(([p]) => [p, []])), longChat = [];
  for (let run = 0; run < RUNS; run++) {
    const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    const page = await ctx.newPage();
    await page.goto(`${BASE}/?mock=1&demo=large`);
    await page.getByRole('heading', { name: 'Projects', exact: true }).first().waitFor();
    shell.push(await page.evaluate(() => performance.now()));
    await page.locator('.home-project-row').first().waitFor({ timeout: 15_000 });
    useful.push(await page.evaluate(() => performance.now()));
    await page.waitForTimeout(800);
    // Each rail destination in turn, twice (the first visit loads its chunk).
    for (let lap = 0; lap < 2; lap++) {
      for (const [name, ready] of PAGES) {
        const ms = await timedClick(page, '[aria-label="Main"]', name, ready);
        if (lap === 1) switches[name].push(ms);
        await page.waitForTimeout(150);
      }
    }
    // The 1,000-turn conversation, from the sidebar.
    longChat.push(await timedClick(page, '[aria-label="Workspace navigation"]', 'Long design thread',
      `() => [...document.querySelectorAll('.turn-content')].some(t => t.textContent.includes('Reviewed part 499'))`));
    await ctx.close();
  }
  out.launch = { shell: summary(shell), useful: summary(useful) };
  out.switch = Object.fromEntries(Object.entries(switches).map(([k, v]) => [k, summary(v)]));
  out.longChat = summary(longChat);
  await browser.close();
  console.log(JSON.stringify(out, null, 2));
}

if (process.argv[1] && import.meta.url.endsWith(process.argv[1].split('/').pop())) {
  main().catch(e => { console.error(e); process.exit(1); });
}
