import { expect, test, type Page } from './fixtures/test';
import { backToAll, openLibraryView } from './libraryView';

const DIR = 'workspaces/local-q3-review/outputs';
const calls = (page: Page) => page.evaluate(() => ({ ...window.__mockApiCalls }));
const reads = (counts: Record<string, number>, before: Record<string, number>) =>
  ['chat_preview_file', 'media_poster', 'read_canvas'].reduce((n, key) => n + (counts[key] ?? 0) - (before[key] ?? 0), 0);

async function seed(page: Page, canvases = false) {
  await page.goto('/?mock=1&demo=1');
  await expect(page.locator('.home-project-row').first()).toBeVisible();
  await page.evaluate(async ({ dir, canvases }) => {
    const notes = JSON.parse(localStorage.getItem('mockWorkspaceNotes') ?? '{}');
    for (let i = 0; i < 40; i++) notes[`${dir}/photo-${String(i).padStart(2, '0')}.png`] = 'image';
    notes[`${dir}/canvases/bookkeeping.canvas.json`] = '{}';
    localStorage.setItem('mockWorkspaceNotes', JSON.stringify(notes));
    if (canvases) {
      const path = '/src/api.ts', { api } = await import(/* @vite-ignore */ path);
      for (let i = 0; i < 36; i++) await api.recordCanvas(
        { workspaceId: 'local-q3-review', sessionKey: 'grid-fixture' },
        `<hc-canvas id="grid-${i}" title="Gallery ${String(i).padStart(2, '0')}" texture="dither"><hc-stats><hc-stat label="Checks" value="${i}"></hc-stat></hc-stats><hc-chart type="bar" label="Checks">Work: 8\nLibrary: 6</hc-chart><hc-table label="Pages">Page | State\nWork | Ready</hc-table><hc-timeline label="Delivery">2026-11-04 | Review\n2026-11-05 | Release</hc-timeline></hc-canvas>`, null);
    }
  }, { dir: DIR, canvases });
  await page.goto('/?mock=1');
  await expect(page.locator('.home-project-row').first()).toBeVisible();
}
async function open(page: Page, canvases = false) {
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true }).click();
  await openLibraryView(page, 'Results');
  if (canvases) await page.getByRole('group', { name: 'Result type' }).getByRole('button', { name: 'Canvases', exact: true }).click();
}

test('Library Results is a responsive grid and arrow focus does not open a result', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await seed(page);
  // Library opens on All (overnight 11-12 L3); Results is All → Outputs → Grid (L4),
  // with All still selected in the strip and Back to All above it.
  await open(page);
  await expect(page.getByRole('navigation', { name: 'Library view' }).getByRole('button', { name: 'All', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(backToAll(page)).toBeVisible();
  const cards = page.locator('.library-result-card');
  await expect(cards).toHaveCount(42);
  const first = await cards.nth(0).boundingBox(), second = await cards.nth(1).boundingBox();
  expect(second!.x).toBeGreaterThan(first!.x);
  expect(Math.abs(second!.y - first!.y)).toBeLessThan(1);
  const buttons = page.locator('[data-result-button]');
  await buttons.first().focus();
  await page.keyboard.press('ArrowDown');
  await expect(buttons.nth(1)).toBeFocused();
  await expect(page.locator('.library-results-detail')).toHaveCount(0);
});

test('canvas sketches fit the grid and detail returns to the same result by button and Escape', async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 900 });
  await seed(page, true);
  await open(page);
  const list = page.locator('.library-results-list');
  const sketch = list.locator('.res-thumb-canvas svg').first();
  await expect(sketch).toBeVisible();
  const fits = await sketch.evaluate(el => {
    const sketch = el.getBoundingClientRect(), tile = el.closest('.res-thumb')!.getBoundingClientRect();
    return sketch.top >= tile.top - 1 && sketch.bottom <= tile.bottom + 1;
  });
  expect(fits, 'all four sketch panels fit inside the thumbnail').toBe(true);
  const row = list.getByRole('button', { name: 'Gallery 35', exact: true });
  await row.click();
  await page.getByRole('button', { name: 'Back to all results', exact: true }).click();
  await expect(page.locator('.library-results-detail')).toHaveCount(0);
  await expect(row).toBeFocused();
  await row.press('Enter');
  await expect(page.locator('.library-results-detail')).toBeVisible();
  await row.press('Escape');
  await expect(page.locator('.library-results-detail')).toHaveCount(0);
  await expect(row).toBeFocused();
  await page.getByRole('group', { name: 'Result type' }).getByRole('button', { name: 'Canvases', exact: true }).click();
  await expect(page.locator('.library-results-detail')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Back to all results', exact: true })).toHaveCount(0);
  await row.press('Escape');
  await expect(page.locator('.library-results-detail')).toBeVisible();
});

test('Library reads nearby thumbnails within its quota, keeps semantics and opens the owning result by keyboard', async ({ page }) => {
  await page.setViewportSize({ width: 720, height: 600 });
  await seed(page);
  const before = await calls(page);
  await open(page);
  const grid = page.locator('.library-results-list');
  await expect(grid.getByRole('listitem')).toHaveCount(42);
  await expect(grid.locator('.res-thumb-image img').first()).toBeVisible();
  const firstReads = await calls(page);
  expect(reads(firstReads, before)).toBeGreaterThan(0);
  expect(reads(firstReads, before)).toBeLessThanOrEqual(24);
  expect(await grid.locator('.res-thumb-image img').count()).toBeLessThan(24);
  await expect(grid).not.toContainText('bookkeeping');
  await expect(grid.locator('.res-thumb-image').first()).toHaveAttribute('aria-hidden', 'true');
  await expect(grid.locator('img').first()).toHaveAttribute('alt', '');
  await grid.locator('.library-result-row').last().scrollIntoViewIfNeeded();
  expect(reads(await calls(page), before)).toBeLessThanOrEqual(24);
  const row = grid.getByRole('button', { name: 'Q3 regional operations review', exact: true });
  await expect(row).toHaveAttribute('aria-description', /Q3 regional operations review/);
  await row.focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('complementary', { name: 'Preview', exact: true })).toBeVisible();
  // At this width Preview fills the available page; closing returns to the
  // same Library result rather than navigating to its workspace.
  await page.getByRole('button', { name: 'Close preview', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Library', exact: true })).toBeVisible();
  await expect(row).toBeVisible();
  const after = await calls(page);
  expect(after.save_canvas_state ?? 0).toBe(before.save_canvas_state ?? 0);
  expect(after.record_canvas ?? 0).toBe(before.record_canvas ?? 0);
});

test('canvas previews keep the list usable when reads hang, with selection and roving keys intact', async ({ page }) => {
  await seed(page, true);
  await page.evaluate(async () => {
    const path = '/src/api.ts', { api } = await import(/* @vite-ignore */ path);
    api.readCanvas = () => new Promise(() => {});
  });
  await open(page, true);
  const list = page.locator('.library-results-list');
  await expect(list.getByRole('button')).toHaveCount(36);
  await expect(page.getByRole('searchbox', { name: 'Find a result' })).toBeVisible();
  const selected = list.locator('[aria-pressed="true"]');
  await selected.focus();
  const key = await selected.getAttribute('data-result-button');
  await page.keyboard.press('ArrowDown');
  await expect(list.locator('button:focus')).not.toHaveAttribute('data-result-button', key!);
  // Focus movement alone never opens another body while reads are pending.
  await expect(selected).toHaveAttribute('data-result-button', key!);
  await page.keyboard.press('Enter');
  await page.getByRole('searchbox', { name: 'Find a result' }).fill('Gallery 05');
  await expect(list.getByRole('button')).toHaveCount(1);
  await expect(list.getByRole('button', { name: 'Gallery 05', exact: true })).toBeVisible();
});

test('canvas first paint reserves detail reads, scrolls without writes and exposes host sketches', async ({ page }) => {
  await seed(page, true);
  // Open Canvases directly so Files' previews do not share this measurement.
  await page.evaluate(async () => {
    const modePath = '/src/store/libraryModeState.ts', { setLibraryMode } = await import(/* @vite-ignore */ modePath);
    setLibraryMode('notes', true);
    const path = '/src/store/canvasGallery.ts', { openCanvasGallery } = await import(/* @vite-ignore */ path);
    openCanvasGallery();
  });
  const list = page.locator('.library-results-list');
  await expect(list.getByRole('button')).toHaveCount(36);
  await expect(page.locator('.library-results-detail iframe')).toHaveCount(1);
  // Notes is All's view (overnight 11-12 L4), so All is what was remembered.
  expect(await page.evaluate(() => localStorage.getItem('hollow.library.mode'))).toBe('all');
  await expect(list.locator('.res-thumb-canvas').first()).toBeVisible();
  expect(await list.locator('.res-thumb-canvas').count()).toBeLessThan(22);
  await expect(list.getByRole('button', { name: 'Gallery 35', exact: true })).toHaveAttribute('aria-description', /Q3 regional operations review/);
  expect((await calls(page)).read_canvas).toBeLessThanOrEqual(24);
  const before = await calls(page);
  await list.getByRole('button').last().scrollIntoViewIfNeeded();
  expect((await calls(page)).read_canvas).toBeLessThanOrEqual(24);
  await expect(list.locator('iframe')).toHaveCount(0);
  await expect(list.locator('.res-thumb-canvas').first()).toHaveAttribute('role', 'img');
  const after = await calls(page);
  expect(after.save_canvas_state ?? 0).toBe(before.save_canvas_state ?? 0);
  expect(after.record_canvas ?? 0).toBe(before.record_canvas ?? 0);
});

for (const width of [390, 720]) test(`both galleries fit ${width}px and Largest text`, async ({ page }) => {
  await page.setViewportSize({ width, height: 800 });
  await seed(page, true);
  await open(page);
  await page.evaluate(() => { document.documentElement.dataset.textScale = '125'; });
  for (const type of ['All', 'Canvases']) {
    const selector = '.library-results-list';
    if (type === 'Canvases') await page.getByRole('group', { name: 'Result type' }).getByRole('button', { name: 'Canvases', exact: true }).click();
    await expect(page.locator(`${selector} > *`).first()).toBeVisible();
    const geometry = await page.locator(selector).evaluate(el => {
      const list = el.getBoundingClientRect();
      const left = list.left + el.clientLeft, right = left + el.clientWidth;
      const cards = [...el.children].map(child => child.getBoundingClientRect());
      return {
        stacked: cards.every((card, i) => Math.abs(card.left - left) <= 1
          && Math.abs(card.right - right) <= 1
          && (i === 0 || card.top >= cards[i - 1].bottom - 1)),
        overflow: el.scrollWidth - el.clientWidth,
        pageOverflow: document.documentElement.scrollWidth - innerWidth,
      };
    });
    expect(geometry.overflow).toBeLessThanOrEqual(1);
    expect(geometry.pageOverflow).toBeLessThanOrEqual(1);
    expect(geometry.stacked, 'full-width rows stack without horizontal overlap').toBe(true);
    if (type === 'Canvases') {
      const tileFits = await page.locator('.library-result-row').first().evaluate(el => {
        const tile = el.getBoundingClientRect(), list = el.closest('.library-results-list')!.getBoundingClientRect();
        return tile.top >= list.top - 1 && tile.bottom <= list.bottom + 1;
      });
      expect(tileFits).toBe(true);
      const detail = page.getByRole('region', { name: 'Selected canvas' });
      await detail.scrollIntoViewIfNeeded();
      expect((await detail.boundingBox())!.height).toBeGreaterThanOrEqual(200);
      await expect(detail.getByRole('button', { name: 'Open workspace', exact: true })).toBeVisible();
    }
  }
});
