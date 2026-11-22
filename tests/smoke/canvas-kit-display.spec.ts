import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type FrameLocator, type Locator, type Page } from './fixtures/test';
import { KIT_DISPLAY_FIXTURE } from '../../src/canvas/kit/fixture';

// Canvas C2 K2: the kit's display components in the default Offline srcdoc
// document (kit allowed by its hash, artifact scripts blocked).

const REGION = 'Canvas kit display fixture';

const displayFrame = (page: Page) =>
  page.getByRole('region', { name: REGION }).frameLocator('iframe[title="Rendered HTML artifact"]');

async function openFixture(page: Page): Promise<FrameLocator> {
  await page.goto('/?mock=1&window=smoke-artifact');
  const frame = displayFrame(page);
  await expect(frame.locator('hc-chart[type="line"]')).toHaveAttribute('data-hc-ready', '');
  return frame;
}

/** Pixels with any ink in a canvas's backing store. */
const inked = (canvas: Locator) => canvas.evaluate((c: HTMLCanvasElement) => {
  const d = c.getContext('2d')!.getImageData(0, 0, c.width, c.height).data;
  let n = 0;
  for (let i = 3; i < d.length; i += 4) if (d[i]) n++;
  return n;
});

/** Backing-store size vs CSS size × devicePixelRatio. */
const sizing = (canvas: Locator) => canvas.evaluate((c: HTMLCanvasElement) => ({
  width: c.width, height: c.height, cssWidth: c.clientWidth, cssHeight: c.clientHeight, dpr: devicePixelRatio,
}));

test.describe('at devicePixelRatio 2', () => {
  test.use({ deviceScaleFactor: 2 });

  test('charts draw on a canvas sized by devicePixelRatio, each with a hidden data table', async ({ page }) => {
    const frame = await openFixture(page);
    await expect(frame.locator('body')).not.toHaveAttribute('data-script-ran', '1');
    for (const [type, name] of [['line', 'Sessions per day'], ['bar', 'Checks by suite'], ['donut', 'Spend by provider']]) {
      const chart = frame.locator(`hc-chart[type="${type}"]`);
      const canvas = chart.locator('canvas');
      await expect(canvas).toHaveAttribute('role', 'img');
      await expect(canvas).toHaveAttribute('aria-label', new RegExp(`^${name}: `));
      await expect.poll(() => inked(canvas), type).toBeGreaterThan(500);
      const s = await sizing(canvas);
      expect(s.dpr, type).toBe(2);
      expect(s.cssWidth, type).toBeGreaterThan(100);
      expect(s.width, type).toBe(Math.round(s.cssWidth * 2));
      expect(s.height, type).toBe(Math.round(s.cssHeight * 2));
      // The data table VoiceOver reads: present, named, and visually hidden.
      await expect(frame.getByRole('table', { name })).toHaveCount(1);
      const hidden = chart.locator('.hc-chart-data');
      await expect(hidden).toHaveClass(/hc-sr/);
      const box = await hidden.boundingBox();
      expect(box!.width * box!.height, type).toBeLessThanOrEqual(1);
    }
    const line = frame.getByRole('table', { name: 'Sessions per day' });
    await expect(line.getByRole('row')).toHaveCount(6);
    await expect(line.getByRole('columnheader')).toHaveText(['', 'Claude', 'Codex']);
    await expect(line.getByRole('row', { name: /Wed/ }).getByRole('cell')).toHaveText(['11', '14']);
    await expect(frame.locator('hc-chart[type="line"] canvas')).toHaveAttribute('aria-label', /line chart, 2 series over 5 categories/);
    await expect(frame.locator('hc-chart[type="line"]').getByRole('list', { name: 'Legend' })).toHaveText(/Claude.*Codex/);
    await expect(frame.getByRole('table', { name: 'Checks by suite' }).getByRole('row', { name: /Rust/ })).toContainText('2,077');
    // The donut read its author <table>; parts list value and share.
    await expect(frame.locator('hc-chart[type="donut"]').getByRole('list', { name: 'Legend' })).toContainText('Claude $42 (64%)');
    await expect(frame.getByRole('table', { name: 'Spend by provider' }).getByRole('row', { name: /Grok/ })).toContainText('$6');

    // The Table toggle shows the same table.
    const toggle = frame.locator('hc-chart[type="line"]').getByRole('button', { name: 'Table' });
    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-pressed', 'true');
    await expect(frame.locator('hc-chart[type="line"] .hc-chart-data')).not.toHaveClass(/hc-sr/);
    expect((await line.boundingBox())!.height).toBeGreaterThan(100);
  });
});

test('under prefers-contrast: more, donut texture stays inside its own slice', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('theme', 'light'));
  await page.emulateMedia({ contrast: 'more' });
  const frame = await openFixture(page);
  const chart = frame.locator('hc-chart[type="donut"]');
  const canvas = chart.locator('canvas');
  await expect.poll(() => inked(canvas)).toBeGreaterThan(500);
  // Samples a 7×7 CSS-pixel patch at mid-ring, `turn` of the way clockwise
  // from the top, and counts pixels that differ from the series colour.
  const offColour = (turn: number) => canvas.evaluate((c: HTMLCanvasElement, t: number) => {
    const hex = getComputedStyle(c.closest('hc-chart')!).getPropertyValue('--hc-s' + (t < 0.64 ? 1 : 2)).trim();
    const want = [1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16));
    const dpr = devicePixelRatio;
    const w = c.width / dpr;
    const h = c.height / dpr;
    const r1 = Math.min(w, h) / 2 - 6;
    const r = (r1 + r1 * 0.62) / 2;
    const a = t * Math.PI * 2 - Math.PI / 2;
    const x = Math.round((w / 2 + r * Math.cos(a) - 3) * dpr);
    const y = Math.round((h / 2 + r * Math.sin(a) - 3) * dpr);
    const d = c.getContext('2d')!.getImageData(x, y, Math.round(7 * dpr), Math.round(7 * dpr)).data;
    let off = 0;
    for (let i = 0; i < d.length; i += 4) {
      if (Math.abs(d[i] - want[0]) + Math.abs(d[i + 1] - want[1]) + Math.abs(d[i + 2] - want[2]) > 6) off++;
    }
    return off;
  }, turn);
  // Claude is 42 of 66 (slice 0, 0–64% of the turn, never hatched); Codex is
  // slice 1 (64–91%, hatched). Slice 0 keeps its plain colour: no texture
  // from a later slice is stroked across it.
  expect(await offColour(0.32)).toBe(0);
  // The texture itself is on: slice 1 shows hatch lines.
  expect(await offColour(0.775)).toBeGreaterThan(0);
});

test('a chart redraws on a theme switch and on a resize, without ResizeObserver', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('theme', 'light'));
  const frame = await openFixture(page);
  const canvas = frame.locator('hc-chart[type="line"] canvas');
  await expect.poll(() => inked(canvas)).toBeGreaterThan(500);
  // Redraws wait for an animation frame, and a browser pauses those in an
  // offscreen cross-origin iframe, so bring the artifact into view first.
  await page.getByRole('region', { name: REGION }).locator('iframe').scrollIntoViewIfNeeded();
  const pixels = () => canvas.evaluate((c: HTMLCanvasElement) => c.toDataURL());
  const before = await pixels();
  await expect(frame.locator('html')).toHaveAttribute('data-hc-scheme', 'light');

  await page.evaluate(() => { document.documentElement.dataset.theme = 'dark'; });
  await expect(frame.locator('html')).toHaveAttribute('data-hc-scheme', 'dark');
  await expect.poll(pixels).not.toBe(before);
  // The line now wears the dark palette's first step.
  expect(await frame.locator('hc-chart').first().evaluate(el => getComputedStyle(el).getPropertyValue('--hc-s1').trim())).toBe('#3987e5');

  const wide = (await sizing(canvas)).width;
  const viewport = page.viewportSize()!;
  await page.setViewportSize({ width: Math.max(360, Math.round(viewport.width * 0.6)), height: viewport.height });
  await expect.poll(async () => (await sizing(canvas)).width).toBeLessThan(wide);
  const s = await sizing(canvas);
  expect(s.width).toBe(Math.round(s.cssWidth * s.dpr));
});

test('arrow keys and hover read chart values', async ({ page }) => {
  const frame = await openFixture(page);
  const chart = frame.locator('hc-chart[type="line"]');
  const canvas = chart.locator('canvas');
  const tip = chart.locator('.hc-chart-tip');
  await expect(tip).toBeHidden();
  await canvas.focus();
  await page.keyboard.press('ArrowRight');
  await expect(tip).toBeVisible();
  await expect(tip).toContainText('Mon');
  await expect(tip).toContainText('Claude: 12');
  await expect(tip).toContainText('Codex: 8');
  await page.keyboard.press('End');
  await expect(tip).toContainText('Fri');
  await expect(tip).toContainText('Claude: 21');
  await page.keyboard.press('Escape');
  await expect(tip).toBeHidden();

  const box = (await canvas.boundingBox())!;
  await canvas.hover({ position: { x: box.width - 60, y: box.height / 2 } });
  await expect(tip).toBeVisible();
  await expect(tip).toContainText(/Thu|Fri/);
});

test('hc-table sorts by keyboard and reports aria-sort', async ({ page }) => {
  const frame = await openFixture(page);
  const table = frame.getByRole('table', { name: 'Lanes' });
  const firstColumn = () => table.locator('tbody tr').evaluateAll(rows => rows.map(r => (r as HTMLTableRowElement).cells[0].textContent));
  const hours = table.getByRole('columnheader', { name: /Hours/ });
  await expect(hours).toHaveAttribute('aria-sort', 'none');
  const sortHours = hours.getByRole('button');
  await sortHours.focus();
  await page.keyboard.press('Enter');
  await expect(hours).toHaveAttribute('aria-sort', 'ascending');
  expect(await firstColumn()).toEqual(['Lane C', 'Lane A', 'Lane B', 'Lane D']);
  await page.keyboard.press('Space');
  await expect(hours).toHaveAttribute('aria-sort', 'descending');
  expect(await firstColumn()).toEqual(['Lane B', 'Lane A', 'Lane C', 'Lane D']);

  const status = table.getByRole('columnheader', { name: /Status/ });
  await page.keyboard.press('Tab');
  await expect(status.getByRole('button')).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(status).toHaveAttribute('aria-sort', 'ascending');
  await expect(hours).toHaveAttribute('aria-sort', 'none');
  expect(await firstColumn()).toEqual(['Lane D', 'Lane A', 'Lane C', 'Lane B']);
});

test('hc-tabs use a roving tabindex and arrow keys, and a chart in a revealed panel draws', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const frame = await openFixture(page);
  const tabs = frame.getByRole('tablist', { name: 'Views' });
  const tab = (name: string) => tabs.getByRole('tab', { name });
  await expect(tab('Summary')).toHaveAttribute('aria-selected', 'true');
  await expect(tab('Summary')).toHaveAttribute('tabindex', '0');
  await expect(tab('Queue')).toHaveAttribute('tabindex', '-1');
  await expect(frame.getByText('First panel text.')).toBeVisible();
  const queueCanvas = frame.locator('hc-chart[type="area"] canvas');
  await expect(queueCanvas).toBeHidden();

  await tab('Summary').focus();
  await page.keyboard.press('ArrowRight');
  await expect(tab('Queue')).toBeFocused();
  await expect(tab('Queue')).toHaveAttribute('aria-selected', 'true');
  await expect(tab('Queue')).toHaveAttribute('tabindex', '0');
  await expect(tab('Summary')).toHaveAttribute('tabindex', '-1');
  await expect(frame.getByText('First panel text.')).toBeHidden();
  await expect(frame.getByRole('tabpanel', { name: 'Queue' })).toBeVisible();
  // Hidden at first render (zero width), drawn once its panel is shown.
  await expect.poll(() => inked(queueCanvas)).toBeGreaterThan(300);
  expect((await sizing(queueCanvas)).cssWidth).toBeGreaterThan(100);

  await page.keyboard.press('End');
  await expect(tab('Notes')).toBeFocused();
  await page.keyboard.press('ArrowRight');
  await expect(tab('Summary')).toBeFocused();
  await page.keyboard.press('ArrowLeft');
  await expect(tab('Notes')).toBeFocused();
  await page.keyboard.press('Home');
  await expect(tab('Summary')).toHaveAttribute('aria-selected', 'true');
  // Nothing in the kit animates under reduced motion.
  expect(await frame.locator('body').evaluate(() => document.getAnimations().length)).toBe(0);
});

test('hc-file asks the host to navigate and posts nothing else; a web path is refused', async ({ page }) => {
  const frame = await openFixture(page);
  await page.evaluate((region) => {
    const iframe = document.querySelector(`[aria-label="${region}"] iframe`) as HTMLIFrameElement;
    const seen: unknown[] = [];
    (window as unknown as { kitMessages: unknown[] }).kitMessages = seen;
    addEventListener('message', e => {
      const d = e.data as { kind?: string } | null;
      if (e.source === iframe.contentWindow && d && d.kind !== 'size' && d.kind !== 'settled') seen.push(d);
    });
  }, REGION);
  const messages = () => page.evaluate(() => (window as unknown as { kitMessages: unknown[] }).kitMessages);

  const report = frame.getByRole('link', { name: /Report/ });
  await expect(report).toHaveAttribute('data-vault', 'outputs/report.md');
  await expect(report).toContainText('outputs/report.md');
  await report.click();
  await expect.poll(messages).toEqual([{ __hc: 1, kind: 'navigate', target: 'outputs/report.md' }]);
  await report.focus();
  await page.keyboard.press('Enter');
  await expect.poll(messages).toHaveLength(2);
  expect(await messages()).toEqual([
    { __hc: 1, kind: 'navigate', target: 'outputs/report.md' },
    { __hc: 1, kind: 'navigate', target: 'outputs/report.md' },
  ]);

  // The web path renders as text, not a link, and clicking it sends nothing.
  const web = frame.locator('hc-file[path^="https:"]');
  await expect(web).toHaveAttribute('data-hc-ready', '');
  await expect(web.locator('a')).toHaveCount(0);
  await expect(web).toContainText('Web');
  await web.click();
  await page.waitForTimeout(200);
  expect(await messages()).toHaveLength(2);
  expect(JSON.stringify(await messages())).not.toContain('"open"');
});

test('steps, bars, compare and code render with words and structure, not colour alone', async ({ page }) => {
  const frame = await openFixture(page);
  const steps = frame.getByRole('list', { name: 'Release' });
  await expect(steps.getByRole('listitem')).toHaveCount(4);
  await expect(steps.getByRole('listitem').nth(0)).toContainText('Done: Spike');
  await expect(steps.getByRole('listitem').nth(1)).toHaveAttribute('aria-current', 'step');
  await expect(steps.getByRole('listitem').nth(1)).toContainText('Current: Sandbox');
  await expect(steps.getByRole('listitem').nth(2)).toContainText('Upcoming: Kit');
  await expect(steps.getByRole('listitem').nth(3)).toContainText('Blocked: Install');
  await expect(steps.locator('[aria-current="step"]')).toHaveCount(1);

  const bars = frame.getByRole('list', { name: 'Hours by lane' });
  await expect(bars.getByRole('listitem')).toHaveCount(3);
  await expect(bars.getByRole('listitem').nth(1)).toHaveText(/Lane B\s*30 h/);
  const fill = (i: number) => bars.locator('.hc-bar-fill').nth(i).evaluate(el => el.getBoundingClientRect().width);
  expect(await fill(1)).toBeGreaterThan(await fill(0));

  const fly = frame.getByRole('group', { name: 'Fly' });
  await expect(fly).toContainText('Recommended');
  await expect(frame.getByRole('group', { name: 'Render' })).not.toContainText('Recommended');

  const code = frame.locator('pre[aria-label="Code: example.ts"]');
  await expect(code).toHaveText('// greet the vault\nconst name = "Hollow";\nreturn 42;');
  await expect(code.locator('.hc-syn-keyword').first()).toHaveText('const');
  await expect(code.locator('.hc-syn-comment')).toHaveText('// greet the vault');
  await expect(code).toHaveAttribute('tabindex', '0');
});

test('without the kit script, display markup stays readable', async ({ page }) => {
  const css = readFileSync(join(process.cwd(), 'src/canvas/kit/generated/kit.min.css'), 'utf8');
  await page.setContent(`<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="script-src 'none'">
<style>${css}</style></head><body>${KIT_DISPLAY_FIXTURE}</body></html>`);
  await expect(page.locator('[data-hc-ready]')).toHaveCount(0);
  for (const text of ['Lane B: 30', 'Mon: 12, 8', 'Rust | 2,077 | 12', 'First panel text.', 'Third panel text.', 'Closer to users.', 'const name = "Hollow";']) {
    await expect(page.getByText(text)).toBeVisible();
  }
  // A table source is the fallback table itself.
  await expect(page.getByRole('cell', { name: '$42' })).toBeVisible();
  const before = (selector: string) => page.locator(selector).first().evaluate(el => getComputedStyle(el, '::before').content);
  expect(await before('hc-bars')).toContain('Hours by lane');
  expect(await before('hc-steps > [label="Spike"]')).toContain('Spike');
  expect(await before('hc-tabs > [label="Queue"]')).toContain('Queue');
  expect(await page.locator('hc-file').first().evaluate(el => getComputedStyle(el, '::after').content)).toContain('outputs/report.md');
});
