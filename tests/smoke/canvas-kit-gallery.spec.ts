import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import AxeBuilder from '@axe-core/playwright';
import { expect, test, type FrameLocator, type Locator, type Page, type TestInfo } from './fixtures/test';

// Canvas C2 K3: the kit@1 gallery (src/canvas/kit/gallery.html) in the smoke
// harness. Screenshots in light, dark and glass at both widths are attached
// to the results; there are no pixel baselines (the smoke runner is Linux and
// these were made on macOS), so the checks are geometry and computed styles.
// Then axe on the host and inside the frames, and reduced motion.

const GALLERY = 'Canvas kit gallery';
const FALLBACK = 'Canvas kit gallery without the kit script';
const SOURCE = readFileSync(join(process.cwd(), 'src/canvas/kit/gallery.html'), 'utf8');
/** Every kit element the gallery uses, read from its source. */
const TAGS = [...new Set(SOURCE.match(/<hc-[a-z]+/g)!.map(t => t.slice(1)))].sort();
const CHART_TYPES = ['line', 'bar', 'area', 'donut'];
/** The first chart colour in each scheme (`palette.ts`). */
const SERIES_1 = { light: '#2a78d6', dark: '#3987e5' };

type Theme = 'light' | 'dark';

const galleryFrame = (page: Page): FrameLocator =>
  page.getByRole('region', { name: GALLERY, exact: true }).frameLocator('iframe[title="Rendered HTML artifact"]');
const fallbackFrame = (page: Page): FrameLocator =>
  page.getByRole('region', { name: FALLBACK, exact: true }).frameLocator('iframe');

/** Pixels with any ink in a canvas's backing store. */
const inked = (canvas: Locator) => canvas.evaluate((c: HTMLCanvasElement) => {
  const d = c.getContext('2d')!.getImageData(0, 0, c.width, c.height).data;
  let n = 0;
  for (let i = 3; i < d.length; i += 4) if (d[i]) n++;
  return n;
});

/** A host token resolved to a computed colour. */
const hostColour = (page: Page, token: string) => page.evaluate((t) => {
  const probe = document.createElement('span');
  probe.style.color = `var(${t})`;
  document.body.append(probe);
  const c = getComputedStyle(probe).color;
  probe.remove();
  return c;
}, token);

/** Open the harness on `theme` (optionally glass), expand the gallery and wait
 *  until every element has upgraded and every chart has drawn. */
async function openGallery(page: Page, theme: Theme, { glass = false } = {}): Promise<FrameLocator> {
  // Glass is persisted like the theme; the theme store applies it to <html>
  // when the harness loads it (the theme watcher sees the attribute).
  await page.addInitScript(([t, g]) => {
    localStorage.setItem('theme', t);
    localStorage.setItem('glass', g ? 'on' : 'off');
  }, [theme, glass] as const);
  await page.goto('/?mock=1&window=smoke-artifact');
  const region = page.getByRole('region', { name: GALLERY, exact: true });
  await region.getByRole('button', { name: 'Expand artifact' }).click();
  // Charts draw on an animation frame, which a browser pauses for an
  // offscreen cross-origin frame.
  await region.locator('iframe').scrollIntoViewIfNeeded();
  const frame = galleryFrame(page);
  await expect(frame.locator('html')).toHaveAttribute('data-hc-scheme', theme);
  await expect.poll(() => frame.locator('main').evaluate(m =>
    [...m.querySelectorAll('*')].filter(e => e.localName.startsWith('hc-') && !e.hasAttribute('data-hc-ready')).map(e => e.localName),
  ), 'every kit element upgraded').toEqual([]);
  for (const type of CHART_TYPES) {
    await expect.poll(() => inked(frame.locator(`hc-chart[type="${type}"] canvas`)), `${type} chart drew`).toBeGreaterThan(500);
  }
  return frame;
}

async function attach(info: TestInfo, name: string, target: Locator) {
  await info.attach(`${name}-${info.project.name}.png`, { body: await target.screenshot(), contentType: 'image/png' });
}

/** Layout inside a frame: nothing wider than the document, no element off its
 *  right edge, every kit element and canvas has a box. */
const layout = (frame: FrameLocator) => frame.locator('main').evaluate((main) => {
  const doc = document.documentElement;
  const width = doc.clientWidth;
  const offRight: string[] = [];
  const empty: string[] = [];
  for (const el of main.querySelectorAll('*')) {
    const r = el.getBoundingClientRect();
    const kit = el.localName.startsWith('hc-') || el.localName === 'canvas';
    if (kit && (r.width < 1 || r.height < 1) && !el.closest('[hidden], .hc-sr')) empty.push(el.localName + (el.getAttribute('label') ? `[${el.getAttribute('label')}]` : ''));
    // Contents of a scroll container may overflow it on purpose.
    let scroller = el.parentElement;
    let scrolls = false;
    while (scroller && scroller !== main) {
      if (/(auto|scroll)/.test(getComputedStyle(scroller).overflowX)) { scrolls = true; break; }
      scroller = scroller.parentElement;
    }
    if (!scrolls && r.width && r.right > width + 1 && !el.closest('.hc-sr')) offRight.push(`${el.localName}.${el.className} ${Math.round(r.right)}>${width}`);
  }
  return { scrollWidth: doc.scrollWidth, width, offRight: offRight.slice(0, 8), empty };
});

for (const theme of ['light', 'dark'] as const) {
  test(`gallery in ${theme}: every element upgrades, follows the host tokens and fits its frame`, async ({ page }, info) => {
    const frame = await openGallery(page, theme);
    // Every kit@1 component is on the page (the source is the list).
    const manifest = readFileSync(join(process.cwd(), 'src/canvas/kit/manifest.ts'), 'utf8');
    const components = [...manifest.matchAll(/^\s+'(hc-[a-z]+)',$/gm)].map(m => m[1]).sort();
    expect(TAGS).toEqual(components);
    for (const tag of TAGS) expect(await frame.locator(tag).count(), tag).toBeGreaterThan(0);

    const region = page.getByRole('region', { name: GALLERY, exact: true });
    await attach(info, `kit-gallery-${theme}`, region);

    // Computed styles follow the host theme.
    const ink = await hostColour(page, '--ink');
    const danger = await hostColour(page, '--error');
    expect(await frame.locator('body').evaluate(b => getComputedStyle(b).color)).toBe(ink);
    expect(await frame.locator('hc-badge[tone="danger"]').evaluate(el => getComputedStyle(el, '::before').color)).toBe(danger);
    expect(await frame.locator('hc-chart').first().evaluate(el => getComputedStyle(el).getPropertyValue('--hc-s1').trim())).toBe(SERIES_1[theme]);
    expect(await frame.locator('html').evaluate(el => getComputedStyle(el).colorScheme)).toContain(theme);

    // Geometry: nothing spills out of the frame, every element has a box, the
    // frame sits inside its card, and charts are DPR-sized.
    const box = await layout(frame);
    expect(box.offRight).toEqual([]);
    expect(box.empty).toEqual([]);
    expect(box.scrollWidth).toBeLessThanOrEqual(box.width);
    const iframe = (await region.locator('iframe').boundingBox())!;
    const card = (await region.boundingBox())!;
    expect(iframe.x).toBeGreaterThanOrEqual(card.x);
    expect(iframe.x + iframe.width).toBeLessThanOrEqual(card.x + card.width + 1);
    for (const type of CHART_TYPES) {
      const s = await frame.locator(`hc-chart[type="${type}"] canvas`).evaluate((c: HTMLCanvasElement) =>
        ({ width: c.width, cssWidth: c.clientWidth, dpr: devicePixelRatio, doc: document.documentElement.clientWidth }));
      expect(s.cssWidth, type).toBeGreaterThan(type === 'donut' ? 100 : 200);
      expect(s.cssWidth, type).toBeLessThanOrEqual(s.doc);
      expect(s.width, type).toBe(Math.round(s.cssWidth * s.dpr));
    }
    // Stat tiles wrap rather than overflow; each tile has room for its value.
    const tiles = await frame.locator('hc-stat').evaluateAll(els => els.map(e => e.getBoundingClientRect().width));
    for (const w of tiles) expect(w).toBeGreaterThan(100);

    // The no-script gallery: nothing upgrades, the content is readable text
    // in the same theme.
    const fallback = fallbackFrame(page);
    await page.getByRole('region', { name: FALLBACK, exact: true }).locator('iframe').scrollIntoViewIfNeeded();
    await expect(fallback.getByText('Install waits for the bed.')).toBeVisible();
    await expect(fallback.locator('[data-hc-ready]')).toHaveCount(0);
    expect(await fallback.locator('main').evaluate(() => typeof (window as unknown as { HollowKit?: unknown }).HollowKit)).toBe('undefined');
    for (const text of ['Lane B: 30', 'Mon: 12, 8', 'Rust | 2,077 | 12', 'Second panel text.', 'const name = "Hollow";']) {
      await expect(fallback.getByText(text)).toBeVisible();
    }
    await expect.poll(() => fallback.locator('body').evaluate(b => getComputedStyle(b).color)).toBe(ink);
    const fbox = await layout(fallback);
    expect(fbox.scrollWidth).toBeLessThanOrEqual(fbox.width);
    await attach(info, `kit-gallery-no-script-${theme}`, page.getByRole('region', { name: FALLBACK, exact: true }));
  });
}

test('gallery under glass keeps opaque tokens and stays inside its card', async ({ page }, info) => {
  const frame = await openGallery(page, 'dark', { glass: true });
  await expect(page.locator('html')).toHaveAttribute('data-glass', 'on');
  // Glass is an overlay: frames still get the opaque theme tokens (glass.css
  // never redefines them), so chart gaps and kit surfaces stay solid.
  const tokens = await frame.locator('html').evaluate(el => {
    const cs = getComputedStyle(el);
    return ['--app-surface', '--app-canvas', '--app-ink'].map(n => cs.getPropertyValue(n).trim());
  });
  for (const t of tokens) expect(t, 'opaque token').toMatch(/^#[0-9a-f]{6}$/i);
  expect(await frame.locator('body').evaluate(b => getComputedStyle(b).color)).toBe(await hostColour(page, '--ink'));
  const box = await layout(frame);
  expect(box.offRight).toEqual([]);
  expect(box.scrollWidth).toBeLessThanOrEqual(box.width);
  await attach(info, 'kit-gallery-glass', page.getByRole('region', { name: GALLERY, exact: true }));
});

/** Serious and critical axe violations, readably. */
const serious = (results: Awaited<ReturnType<AxeBuilder['analyze']>>) => results.violations
  .filter(v => v.impact === 'serious' || v.impact === 'critical')
  .map(v => `${v.id} (${v.impact}): ${v.nodes.slice(0, 4).map(n => n.target.join(' >>> ')).join(' | ')}`);

for (const theme of ['light', 'dark'] as const) {
  test(`axe in ${theme}: no serious or critical violations on the host gallery or inside its frames`, async ({ page }) => {
    await openGallery(page, theme);
    await page.getByRole('region', { name: FALLBACK, exact: true }).locator('iframe').scrollIntoViewIfNeeded();
    await expect(fallbackFrame(page).getByText('Install waits for the bed.')).toBeVisible();
    for (const region of [GALLERY, FALLBACK]) {
      const results = await new AxeBuilder({ page }).include(`[aria-label="${region}"]`).analyze();
      expect(serious(results), region).toEqual([]);
      // The frame document was analysed, not just the host: some checked
      // nodes sit inside the iframe (a target path through it).
      const inFrame = [...results.passes, ...results.incomplete].some(r => r.nodes.some(n => n.target.length > 1));
      expect(inFrame, `${region}: axe reached inside the frame`).toBe(true);
    }
  });
}

/** Animations running in a frame after starting everything that could move:
 *  open each closed disclosure and switch tabs. getAnimations() flushes
 *  styles, so a transition that would run shows up here. */
const runningAfter = (frame: FrameLocator) => frame.locator('body').evaluate(() => {
  for (const b of document.querySelectorAll<HTMLElement>('hc-details [aria-expanded="false"]')) b.click();
  document.querySelectorAll<HTMLElement>('[role="tab"]')[1]?.click();
  return document.getAnimations().filter(a => a.playState === 'running').map(a => String((a.effect as KeyframeEffect | null)?.target?.className ?? '?'));
});
/** The kit's only transitions (kit.css), as computed now. */
const transitions = (frame: FrameLocator) => Promise.all(['.hc-details-chevron', '.hc-progress-fill'].map(sel =>
  frame.locator(sel).first().evaluate(el => getComputedStyle(el).transitionDuration)));

test('reduced motion: nothing runs in the host or in either gallery frame', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const frame = await openGallery(page, 'light');
  await frame.locator('hc-flow[label="Release route"]').scrollIntoViewIfNeeded();
  await frame.locator('body').evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  expect(await runningAfter(frame)).toEqual([]);
  expect(await transitions(frame)).toEqual(['0s', '0s']);
  await page.getByRole('region', { name: FALLBACK, exact: true }).locator('iframe').scrollIntoViewIfNeeded();
  await expect(fallbackFrame(page).getByText('Install waits for the bed.')).toBeVisible();
  expect(await fallbackFrame(page).locator('body').evaluate(() => document.getAnimations().filter(a => a.playState === 'running').length)).toBe(0);
  expect(await page.evaluate(() => document.getAnimations().filter(a => a.playState === 'running').length)).toBe(0);
});

test('full motion keeps transitions and only the explicitly active flow particles keep running', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  const frame = await openGallery(page, 'light');
  expect(await transitions(frame)).toEqual(['0.2s', '0s']);
  // Fix visibility before judging the gated entrances and active paths.
  await runningAfter(frame);
  await frame.locator('hc-flow[label="Release route"]').scrollIntoViewIfNeeded();
  const running = () => frame.locator('body').evaluate(() => document.getAnimations()
    .filter(a => a.playState === 'running').map(a => {
      const effect = a.effect as KeyframeEffect | null;
      const target = effect?.target as Element | null;
      return {
        looping: effect?.getTiming().iterations === Infinity,
        particle: target?.classList.contains('hc-flow-particle') ?? false,
        active: target?.closest('.hc-flow-link')?.getAttribute('data-active') === 'true',
        flow: target?.closest('hc-flow')?.getAttribute('label') ?? null,
      };
    }));
  await expect.poll(running).toEqual([
    { looping: true, particle: true, active: true, flow: 'Release route' },
    { looping: true, particle: true, active: true, flow: 'Release route' },
  ]);
});

test('hc-flow draws its nodes, links and groups, names the drawing and says it in words', async ({ page }) => {
  const frame = await openGallery(page, 'light');
  const deploy = frame.locator('hc-flow[label="Deploy decision"]');
  const svg = deploy.locator('svg.hc-flow-svg');
  // Focusable links must remain exposed beneath the labelled SVG group.
  await expect(svg).toHaveAttribute('role', 'group');
  await expect(svg).toHaveAttribute('aria-label', 'Deploy decision');
  await expect(deploy.locator('g.hc-flow-node')).toHaveCount(4);
  await expect(deploy.locator('g.hc-flow-link')).toHaveCount(4);
  await expect(deploy.locator('g.hc-flow-node[data-shape="decision"] polygon:not(.hc-stipple)')).toHaveCount(1);
  // A tone is its colour and its icon: the success node's label starts with ✓.
  await expect(deploy.locator('g.hc-flow-node[data-tone="success"] text')).toHaveText('✓ Deploy');
  await expect(deploy.locator('ul.hc-sr li')).toHaveText([
    'Change merged (start) → CI green?',
    'CI green? (decision) → Deploy (yes), Fix failures (no)',
    'Deploy (Success)',
    'Fix failures (Danger) → CI green?',
  ]);
  const path = frame.locator('hc-flow[label="Request path"]');
  await expect(path.locator('g.hc-flow-group')).toHaveCount(2);
  await expect(path.locator('g.hc-flow-group text')).toHaveText(['Frontend', 'Rust backend']);
  // Laid out across where it fits: every node sits on one row. In a narrow
  // window running across would shrink its text past 80%, so it runs down
  // (10-11 B2): every node sits in one column.
  const ys = await path.locator('g.hc-flow-node rect:not(.hc-stipple)').evaluateAll(rs => rs.map(r => r.getAttribute('y')));
  const xs = await path.locator('g.hc-flow-node rect:not(.hc-stipple)').evaluateAll(rs => rs.map(r => r.getAttribute('x')));
  if (test.info().project.name === 'chromium-narrow') expect(new Set(ys).size).toBe(ys.length);
  else expect(new Set(ys).size).toBe(1);
  expect(xs.length).toBe(4);
});
