import { expect, test } from './fixtures/test';
import AxeBuilder from '@axe-core/playwright';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';

for (const theme of ['light', 'dark', 'glass']) test(`instruments in the real mock frame: ${theme}`, async ({ page }, info) => {
  await page.addInitScript(t => {
    localStorage.setItem('theme', t === 'light' ? 'light' : 'dark');
    localStorage.setItem('glass', t === 'glass' ? 'on' : 'off');
  }, theme);
  await page.goto('/?mock=1&window=smoke-artifact');
  const region = page.getByRole('region', { name: 'Canvas kit gallery', exact: true });
  await region.getByRole('button', { name: 'Expand artifact' }).click();
  const frame = region.frameLocator('iframe');
  const instruments = frame.getByRole('region', { name: 'Instruments' });
  // The canvas frame is cross-origin, and Chromium throttles animation frames in
  // a cross-origin frame that is off screen. Playwright's "stable" check waits
  // on those frames, so it timed out whenever Expand left the frame below the
  // fold (1 in 12 on 8e1962fd, about 1 in 5 later). Bring the frame on screen
  // first, then scroll to the region inside it.
  await region.locator('iframe').scrollIntoViewIfNeeded();
  await instruments.evaluate(el => el.scrollIntoView({ block: 'center' }));
  for (const tag of ['hc-gauge', 'hc-sparkline', 'hc-timeline', 'hc-heatmap']) {
    await expect(instruments.locator(tag)).toHaveAttribute('data-hc-ready', '');
  }
  await expect(frame.locator('html')).toHaveAttribute('data-hc-scheme', theme === 'light' ? 'light' : 'dark');
  expect(await instruments.evaluate(el => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  await instruments.locator('hc-heatmap svg').focus(); await page.keyboard.press('End');
  await expect(instruments.locator('hc-heatmap .hc-instrument-tip')).toContainText('2026-10-18: 14 turns');
  const result = await new AxeBuilder({ page }).include('[aria-label="Canvas kit gallery"] iframe').withTags(['wcag2a', 'wcag2aa']).analyze();
  expect(result.violations.filter(v => v.impact === 'critical' || v.impact === 'serious')).toEqual([]);
  const dir = process.env.HOLLOW_LANE_SHOTS;
  if (dir) {
    mkdirSync(dir, { recursive: true });
    await instruments.screenshot({ path: join(dir, `v1-${theme}-${info.project.name}.png`) });
  }
});
