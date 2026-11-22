import { readFileSync } from 'node:fs';
import { expect, test } from './fixtures/test';
const kit = () => readFileSync('src/canvas/kit/generated/kit.min.js', 'utf8');
const css = () => readFileSync('src/canvas/kit/generated/kit.min.css', 'utf8');
const content = `<hc-chart type="bar" label="Checks">A: 10\nB: 20</hc-chart>
<hc-gauge label="Capacity" value="125" max="100"></hc-gauge>
<hc-stat label="Week" value="9"><hc-sparkline label="Trend">A: 2\nB: —\nC: 9</hc-sparkline></hc-stat>
<hc-timeline label="Delivery">2026-10-12 | Build | 2026-10-14\n2026-10-15 | Ship</hc-timeline>
<hc-heatmap label="Work" start="2026-10-01" end="2026-10-03">2026-10-01: 0\n2026-10-03: 8</hc-heatmap>`;

test('instruments show values, unknown days and keyboard inspection', async ({ page }) => {
  await page.setContent(`<style>${css()}</style>${content}<script>${kit()}</script>`);
  await expect(page.locator('hc-gauge')).toContainText('125');
  await expect(page.locator('hc-gauge')).toContainText('outside the shown range');
  await expect(page.locator('hc-sparkline path')).toHaveCount(2);
  await expect(page.locator('hc-heatmap [data-level=missing]')).toHaveCount(1);
  await expect(page.locator('hc-heatmap [data-level=zero]')).toHaveCount(1);
  const flow = page.locator('hc-timeline svg');
  await flow.focus(); await page.keyboard.press('End');
  await expect(page.locator('hc-timeline .hc-instrument-tip')).toHaveText('Ship: 2026-10-15');
  await page.locator('hc-heatmap svg').focus(); await page.keyboard.press('ArrowRight');
  await expect(page.locator('hc-heatmap .hc-instrument-tip')).toHaveText('2026-10-02: No data');
});

test('chart entrance is transform-only, finishes on interaction and disposes on removal', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.setContent(`<style>${css()}</style>${content}<script>${kit()}</script>`);
  await expect.poll(() => page.locator('.hc-chart-entrance').count()).toBe(1);
  const effects = await page.evaluate(() => document.getAnimations().map(a => (a.effect as KeyframeEffect).getKeyframes()));
  expect(effects.length).toBeGreaterThan(0);
  for (const frames of effects) for (const frame of frames) expect(Object.keys(frame).filter(k => !['offset', 'computedOffset', 'easing', 'composite', 'transform'].includes(k))).toEqual([]);
  await page.locator('hc-chart canvas').focus(); await page.keyboard.press('ArrowRight');
  await expect(page.locator('.hc-chart-entrance')).toHaveCount(0);
  await expect(page.locator('.hc-chart-tip')).toContainText('A');
  await page.locator('hc-chart').evaluate(e => e.remove());
  expect(await page.evaluate(() => document.getAnimations().length)).toBe(0);
});

test('both reduced-motion gates show final charts without entrances', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.setContent(`<style>${css()}</style>${content}<script>${kit()}</script>`);
  expect(await page.evaluate(() => document.getAnimations().length)).toBe(0);
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.setContent(`<html data-hc-motion="reduce"><head><style>${css()}</style></head><body>${content}<script>${kit()}</script></body></html>`);
  expect(await page.evaluate(() => document.getAnimations().length)).toBe(0);
  await expect(page.locator('.hc-chart-entrance')).toHaveCount(0);
});

test('offscreen charts wait, pause when scrolled away, and settle when motion is reduced', async ({ page }) => {
  await page.evaluate(() => {
    const old = HTMLCanvasElement.prototype.toDataURL;
    (window as unknown as { encodes: number }).encodes = 0;
    HTMLCanvasElement.prototype.toDataURL = function (...args) { (window as unknown as { encodes: number }).encodes++; return old.apply(this, args); };
  });
  await page.setContent(`<style>${css()}hc-chart{margin-top:1400px}</style>${content}<script>${kit()}</script>`);
  await expect(page.locator('hc-chart')).toHaveAttribute('data-hc-ready', '');
  expect(await page.evaluate(() => document.getAnimations().length)).toBe(0);
  expect(await page.evaluate(() => (window as unknown as { encodes: number }).encodes)).toBe(0);
  await page.locator('hc-chart').scrollIntoViewIfNeeded();
  await expect.poll(() => page.evaluate(() => document.getAnimations().length)).toBeGreaterThan(0);
  await page.evaluate(() => window.scrollTo(0, 0));
  await expect.poll(() => page.evaluate(() => document.getAnimations().every(a => a.playState === 'paused'))).toBe(true);
  await page.evaluate(() => { document.documentElement.dataset.hcMotion = 'reduce'; });
  await expect(page.locator('.hc-chart-entrance')).toHaveCount(0);
  expect(await page.evaluate(() => document.getAnimations().length)).toBe(0);
});

for (const kind of ['line', 'area', 'bar', 'donut']) {
  test(`${kind} finishes at exactly its nonanimated pixels`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'no-preference' });
    await page.setContent(`<style>${css()}</style><hc-chart type="${kind}">A: 10\nB: 20\nC: 5</hc-chart><script>${kit()}</script>`);
    const chart = page.locator('hc-chart'), cv = chart.locator('canvas');
    await expect(chart).toHaveAttribute('data-hc-ready', '');
    await expect.poll(() => chart.locator('.hc-chart-entrance').count()).toBe(1);
    await expect(chart.locator('.hc-chart-entrance')).toHaveCount(0);
    const final = await cv.evaluate((el: HTMLCanvasElement) => el.toDataURL());
    await page.evaluate(() => { document.documentElement.dataset.hcMotion = 'reduce'; });
    await chart.evaluate(el => el.setAttribute('label', 'Repaint'));
    expect(await cv.evaluate((el: HTMLCanvasElement) => el.toDataURL())).toBe(final);
  });
}

test('live host motion preference reaches frames and refuses non-parent messages', async ({ page }) => {
  const runtime = readFileSync('src/components/sandbox-doc.js', 'utf8');
  await page.setContent('<iframe sandbox="allow-scripts"></iframe>');
  await page.locator('iframe').evaluate((el: HTMLIFrameElement, script) => {
    el.srcdoc = `<script>${script}</script><p>Motion probe</p>`;
  }, runtime);
  const frame = page.frameLocator('iframe');
  await expect(frame.locator('p')).toHaveText('Motion probe');
  await frame.locator('p').evaluate(() => window.postMessage({ __hc: 1, kind: 'motion', reducedMotion: true }, '*'));
  await expect(frame.locator('html')).not.toHaveAttribute('data-hc-motion', 'reduce');
  await page.locator('iframe').evaluate((el: HTMLIFrameElement) => el.contentWindow!.postMessage({ __hc: 1, kind: 'motion', reducedMotion: true }, '*'));
  await expect(frame.locator('html')).toHaveAttribute('data-hc-motion', 'reduce');
  await page.locator('iframe').evaluate((el: HTMLIFrameElement) => el.contentWindow!.postMessage({ __hc: 1, kind: 'motion', reducedMotion: false }, '*'));
  await expect(frame.locator('html')).not.toHaveAttribute('data-hc-motion', 'reduce');
});
