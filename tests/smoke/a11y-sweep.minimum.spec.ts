/**
 * Overnight 2026-10-15/16 S02: the main views at the native window's minimum
 * (720 × 480) and at Largest text (720 × 600, 125 %), checked two ways.
 *
 * - axe (WCAG 2.1 A and AA) over the whole window, light and dark, once the view has landed
 *   and nothing in it is still moving (a mid-fade colour reads as a contrast
 *   failure, A01).
 * - Keyboard only: Tab from the top of the window through the first controls;
 *   every stop is a visible element inside the window (or scrolled into it),
 *   never the page body, and the walk never sticks on one element.
 *
 * Each view is reached by its route ledger walk (src/routeLedger.ts), so a
 * renamed control fails here as it does in route-ledger.journey.spec.ts.
 * A required spec: it runs only in the minimum projects and may not skip
 * (scripts/check-required-specs.mjs).
 */
import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from './fixtures/test';
import { ROUTES } from '../../src/routeLedger';
import { home, landmark, walk } from './routeWalk';

/** The views, by route id: [label, route id or null for the demo Home]. */
const VIEWS: [string, string | null][] = [
  ['Home', null],
  ['Review', 'review.waiting'],
  ['Running', 'work.activity'],
  ['Tasks', 'tasks.list'],
  ['Projects (Connected projects)', 'library.connected'],
  // The Workspace view's sections went in the COO rebuild (L4, 11-10): a
  // project's notes open as files in the Preview pane, from Project files.
  ['A project’s files in the Preview pane', 'workspace.files'],
  ['Settings', 'settings.page'],
  ['Settings · Agents', 'settings.agents'],
  ['Settings · System', 'settings.system'],
  ['Settings · Interface', 'settings.appearance'],
  ['COO', 'ask.main-agent'],
  ['Results', 'library.results'],
  ['Library', 'library.notes'],
  // Ask's scopes are Work's Ask field (Home, above) since the top bar's
  // Ask ⌄ panel was removed (2026-10-21).
];

/** The view has landed: no page change, loading skeleton or finite animation
 *  still running (ambient infinite loops never settle and are ignored). */
async function still(page: Page) {
  await expect(page.locator('html')).not.toHaveClass(/\bis-transitioning\b/);
  await expect(page.locator('.canvas-split > .page :is(.page-loading, .ui-skeleton)')).toHaveCount(0);
  // Long timers drawn as animations (a snackbar's 6 s countdown bar) aren't
  // entrances and would outlast the poll: only those under 2 s count.
  await expect.poll(() => page.evaluate(() => document.getAnimations().filter(a => {
    const timing = (a.effect as KeyframeEffect | null)?.getComputedTiming();
    return a.playState === 'running' && !!timing && timing.endTime !== Infinity && Number(timing.activeDuration) <= 2000;
  }).length), { message: 'still animating' }).toBe(0);
}

async function open(page: Page, id: string | null) {
  if (!id) {
    await page.goto('/?mock=1&demo=1');
    await expect(page.getByRole('region', { name: 'Projects' })).toBeVisible();
  } else {
    const r = ROUTES.find(x => x.id === id);
    if (!r) throw new Error(`no route ${id}`);
    await home(page, r);
    await walk(page, r.walk);
    await expect(landmark(page, r.lands)).toBeVisible();
  }
  await still(page);
}

async function axe(page: Page) {
  const { violations } = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa']).analyze();
  return violations.flatMap(v => v.nodes.map(n => `${v.id}: ${n.target.join(' ')} · ${n.failureSummary?.split('\n').slice(1, 2).join(' ').trim() ?? ''}`));
}

/** Tab through the view (at most `stops` stops); what's wrong with any stop.
 *  Past the last control focus leaves the document (the body is active) and
 *  the next Tab wraps to the first: that is the end of one cycle, not a lost
 *  focus. A second arrival there ends the walk; reaching it before any
 *  control is a problem. */
async function tabWalk(page: Page, stops = 60) {
  const problems: string[] = [];
  // Start from the top of the document: blur() alone leaves Chromium's
  // sequential focus starting point on the control the walk last clicked.
  await page.evaluate(() => {
    const top = document.createElement('div');
    top.tabIndex = -1;
    document.body.prepend(top);
    top.focus();
    top.remove();
  });
  const seen = new Set<number>();
  let previous = 0;
  let same = 0;
  let wraps = 0;
  for (let i = 0; i < stops; i++) {
    await page.keyboard.press('Tab');
    const stop = await page.evaluate(async () => {
      const el = document.activeElement as HTMLElement | null;
      if (!el || el === document.body || el === document.documentElement) return { id: -1, where: 'body', ok: true, why: '' };
      // Each element keeps one number for the walk, so two alike controls
      // in a row aren't read as focus sticking on one.
      const w = window as unknown as { __s02ids?: WeakMap<Element, number>; __s02next?: number };
      w.__s02ids ??= new WeakMap();
      if (!w.__s02ids.has(el)) w.__s02ids.set(el, (w.__s02next = (w.__s02next ?? 0) + 1));
      const id = w.__s02ids.get(el)!;
      // A focus style that fades in (e.g. Tasks' .task-pick:focus-visible) is
      // read once it has.
      // (Running ones only, and at most 1 s: a paused one never finishes.)
      const fading = el.getAnimations().filter(a => a.playState === 'running' && a.effect?.getComputedTiming().endTime !== Infinity);
      await Promise.race([Promise.all(fading.map(a => a.finished.catch(() => undefined))), new Promise(r => setTimeout(r, 1000))]);
      el.scrollIntoView({ block: 'nearest', inline: 'nearest' });
      const r = el.getBoundingClientRect();
      const style = getComputedStyle(el);
      const cls = typeof el.className === 'string' && el.className.trim() ? '.' + el.className.trim().split(/\s+/).slice(0, 2).join('.') : '';
      const name = `${el.tagName.toLowerCase()}${cls} "${(el.getAttribute('aria-label') ?? el.textContent ?? '').trim().slice(0, 40)}"`;
      if (r.width < 1 || r.height < 1) return { id, where: name, ok: false, why: 'zero-size stop' };
      if (style.visibility === 'hidden' || Number(style.opacity) === 0) return { id, where: name, ok: false, why: 'invisible stop' };
      const inside = r.right > 0 && r.bottom > 0 && r.left < innerWidth && r.top < innerHeight;
      if (!inside) return { id, where: name, ok: false, why: `outside the window (${Math.round(r.left)},${Math.round(r.top)})` };
      return { id, where: name, ok: true, why: '' };
    });
    if (process.env.S02_DEBUG) console.log(`    ${i + 1}: ${stop.where} ${stop.why}`);
    if (stop.where === 'body') {
      if (!seen.size) { problems.push(`stop ${i + 1}: focus left the document before reaching any control`); break; }
      if (++wraps === 2) break;
      previous = 0;
      continue;
    }
    seen.add(stop.id);
    if (!stop.ok) problems.push(`stop ${i + 1}: ${stop.where}: ${stop.why}`);
    same = stop.id === previous ? same + 1 : 0;
    if (same >= 2) { problems.push(`stuck on ${stop.where}`); break; }
    previous = stop.id;
  }
  // A walk that saw almost nothing proves nothing (every view has a rail and a top bar).
  if (seen.size < 10) problems.push(`only ${seen.size} controls reached by Tab`);
  return problems;
}

for (const [label, id] of VIEWS) {
  test(`${label}: axe AA and a keyboard-only walk`, async ({ page }) => {
    test.setTimeout(60_000);
    await open(page, id);
    const found = await axe(page);
    const keys = await tabWalk(page);
    console.log(`${label}\n  axe: ${found.length ? found.join('\n       ') : 'clean'}\n  keys: ${keys.length ? keys.join('\n        ') : 'clean'}`);
    expect.soft(found, `${label}: axe`).toEqual([]);
    expect.soft(keys, `${label}: keyboard`).toEqual([]);
  });
  // The dark theme has its own tokens; the keyboard walk doesn't change with it.
  test(`${label}: axe AA in the dark theme`, async ({ page }) => {
    test.setTimeout(60_000);
    await page.addInitScript(() => { localStorage.setItem('theme', 'dark'); localStorage.setItem('glass', 'off'); });
    await open(page, id);
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
    const found = await axe(page);
    console.log(`${label} (dark)\n  axe: ${found.length ? found.join('\n       ') : 'clean'}`);
    expect(found, `${label}: axe, dark`).toEqual([]);
  });
}
