import { expect, test, type Page } from './fixtures/test';
import { chooseNew } from './choose';

const preview = (page: Page) => page.getByRole('complementary', { name: 'Preview', exact: true });

/** Click the reply's link to the page: it opens in the Preview pane's web
 *  mode (run 15 P1; it was the side panel's Browser tab), and clicked again
 *  after the pane was closed it brings the same tab back. Once the reply has
 *  rendered: its placeholder's links do nothing. */
async function openPageLink(page: Page) {
  const conversation = page.locator('.work-conversation');
  const link = conversation.getByRole('link', { name: 'the page', exact: true });
  await expect(link).toBeVisible();
  await expect(conversation.locator('.reply-loading')).toHaveCount(0);
  await link.click();
  await expect(preview(page).getByRole('tab', { name: 'localhost:5173' })).toHaveAttribute('aria-selected', 'true');
}

async function openAnnotation(page: Page) {
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Review the page');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Keyboard review');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Keyboard review');
  // A reply linking the page under review. No agent runs here, so it goes
  // into the transcript directly, once the new conversation is ready.
  await expect(page.getByRole('heading', { name: /^What should we build in .+\?$/ })).toBeVisible();
  await page.evaluate(async () => {
    const path = '/src/store/chatRuntime.ts';
    (await import(/* @vite-ignore */ path)).setChatTurns([{ role: 'assistant', text: 'It is up: [the page](http://localhost:5173).', tools: [] }]);
  });
  await openPageLink(page);
  const pane = preview(page);
  await pane.getByRole('button', { name: 'Annotate', exact: true }).click();
  const region = pane.getByRole('region', { name: 'Annotate page' });
  await expect(region.getByRole('img', { name: /^Snapshot/ })).toBeVisible();
  return region;
}

const drafts = (page: Page) => page.evaluate(() => JSON.parse(localStorage.getItem('hollow.annotateDrafts') ?? '[]'));

test('keyboard cursor remains inside the scroll viewport from centre to capture edges', async ({ page }) => {
  const region = await openAnnotation(page);
  const stage = region.getByRole('application', { name: 'Annotation stage' });
  await region.getByRole('button', { name: 'Comment on the whole page' }).focus();
  await page.keyboard.press('Tab');
  const visibleCursor = async () => region.evaluate(el => {
    const stage = el.querySelector('.annotate-canvas')!.getBoundingClientRect();
    const view = el.querySelector('.annotate-stage')!.getBoundingClientRect();
    const svg = el.querySelector('svg[aria-label="Marks"]') as SVGSVGElement;
    const point = el.querySelector('.annotate-position')!.textContent!.match(/Position (\d+), (\d+)/)!;
    const x = stage.left + Number(point[1]) / svg.viewBox.baseVal.width * stage.width;
    const y = stage.top + Number(point[2]) / svg.viewBox.baseVal.height * stage.height;
    return x >= view.left && x <= view.right && y >= view.top && y <= view.bottom;
  });
  await expect.poll(visibleCursor).toBe(true);
  for (const key of ['ArrowDown', 'Shift+ArrowDown', 'Shift+ArrowUp', 'Shift+ArrowRight', 'Shift+ArrowLeft']) {
    await stage.press(key);
    await expect.poll(visibleCursor).toBe(true);
    await expect(stage).toBeFocused();
  }
  await expect(region.getByRole('button', { name: 'Add to chat' })).toBeInViewport();
  expect(await page.evaluate(() => window.scrollY)).toBe(0);
});

test('pointer drawing leaves keyboard crosshair hidden until a stage key is used', async ({ page }) => {
  const region = await openAnnotation(page);
  await region.getByRole('group', { name: 'Annotation tool' }).getByRole('button', { name: 'Draw', exact: true }).click();
  const ink = region.getByLabel('Marks');
  await ink.scrollIntoViewIfNeeded();
  const box = (await ink.boundingBox())!;
  await page.mouse.move(box.x + box.width * 0.3, box.y + box.height * 0.3);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.5, box.y + box.height * 0.5, { steps: 3 });
  await page.mouse.up();
  await expect(region.locator('.annotate-position')).toHaveText('');
  const stage = region.getByRole('application', { name: 'Annotation stage' });
  await stage.press('ArrowRight');
  await expect(region.locator('.annotate-position')).toContainText('Position');
});

test('keyboard places a snapshot-pixel comment and Add to chat only drafts', async ({ page }) => {
  const region = await openAnnotation(page);
  const stage = region.getByRole('application', { name: 'Annotation stage' });
  await region.getByRole('button', { name: 'Comment on the whole page' }).focus();
  await page.keyboard.press('Tab');
  await expect(stage).toBeFocused();
  await page.keyboard.press('ArrowRight');
  await page.keyboard.press('Enter');
  const note = region.getByRole('textbox', { name: 'Comment for mark 1' });
  await expect(note).toBeFocused();
  await note.fill('Move this button');
  await note.press('Enter');
  await expect(region.getByRole('button', { name: 'Mark 1: Move this button', exact: true })).toBeFocused();
  await expect.poll(async () => (await drafts(page))[0]?.marks[0]).toEqual({ kind: 'pin', x: 28, y: 15, note: 'Move this button' });
  const stored = (await drafts(page))[0];
  await region.getByRole('button', { name: 'Add to chat' }).click();
  await expect(region).toHaveCount(0);
  await expect(page.getByLabel('Chat message')).toHaveValue(/1\. Move this button — on <button\.cta>/);
  await expect(page.locator('.chat-attachments .chat-attach')).toHaveCount(1);
  const points = await page.evaluate(() => JSON.parse(localStorage.getItem('mockBrowserLog') ?? '[]').find((c: { command: string }) => c.command === 'browser_inspect').args.points);
  expect(points).toEqual([[Math.round(28 * stored.view.width / 40), Math.round(15 * stored.view.height / 30)]]);
  expect(await page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBe(0);
});

test('whole-page comments survive restore and save without coordinate badges', async ({ page }) => {
  const region = await openAnnotation(page);
  await region.getByRole('button', { name: 'Comment on the whole page' }).click();
  await region.getByRole('textbox', { name: 'Comment for mark 1' }).fill('Use a calmer page');
  await page.keyboard.press('Enter');
  await region.getByRole('button', { name: 'Close', exact: true }).click();
  const pane = preview(page);
  await pane.getByRole('status', { name: 'Unsent annotations' }).getByRole('button', { name: 'Restore' }).click();
  await expect(region.getByRole('button', { name: 'Mark 1: Use a calmer page', exact: true })).toBeVisible();
  await expect(region.locator('.annotate-canvas .annotate-label')).toHaveCount(0);
  expect((await drafts(page))[0].marks).toEqual([{ kind: 'page', note: 'Use a calmer page' }]);
  await region.getByRole('button', { name: 'Save to workspace' }).click();
  const review = pane.getByRole('region', { name: 'Page reviews' });
  await expect(review).toContainText('Use a calmer page');
  await expect(review.locator('.review-capture-frame .review-mark-num')).toHaveCount(0);
});

test('restored captures use original image pixels and map inspection to the viewport', async ({ page }) => {
  const region = await openAnnotation(page);
  await region.getByRole('button', { name: 'Comment on the whole page' }).click();
  await region.getByRole('textbox', { name: 'Comment for mark 1' }).fill('Page context');
  await page.keyboard.press('Enter');
  await region.getByRole('button', { name: 'Close', exact: true }).click();
  await page.evaluate(() => {
    const list = JSON.parse(localStorage.getItem('hollow.annotateDrafts')!);
    list[0].imageSize = { width: 400, height: 300 };
    list[0].view = { width: 200, height: 150 };
    localStorage.setItem('hollow.annotateDrafts', JSON.stringify(list));
  });
  const pane = preview(page);
  // Close the pane and click the link again, so the persisted draft is read afresh.
  await pane.getByRole('button', { name: 'Close preview', exact: true }).click();
  await expect(pane).toHaveCount(0);
  await openPageLink(page);
  await pane.getByRole('status', { name: 'Unsent annotations' }).getByRole('button', { name: 'Restore' }).click();
  const stage = region.getByRole('application', { name: 'Annotation stage' });
  await stage.focus(); await stage.press('ArrowRight'); await stage.press('Enter');
  await region.getByRole('textbox', { name: 'Comment for mark 2' }).fill('Scaled point');
  await page.keyboard.press('Enter');
  await expect.poll(async () => (await drafts(page))[0]?.marks[1]).toEqual({ kind: 'pin', x: 208, y: 150, note: 'Scaled point' });
  await region.getByRole('button', { name: 'Add to chat' }).click();
  await expect(region).toHaveCount(0);
  const points = await page.evaluate(() => JSON.parse(localStorage.getItem('mockBrowserLog') ?? '[]').find((c: { command: string }) => c.command === 'browser_inspect').args.points);
  expect(points).toEqual([[104, 75]]);
});

test('box corners use arrows, clamp to bounds and Escape cancels just the pending box', async ({ page }) => {
  const region = await openAnnotation(page);
  const tools = region.getByRole('group', { name: 'Annotation tool' });
  await tools.getByRole('button', { name: 'Box', exact: true }).click();
  await expect(tools.getByRole('button', { name: 'Box', exact: true })).toHaveAttribute('aria-pressed', 'true');
  const stage = region.getByRole('application', { name: 'Annotation stage' });
  await stage.focus();
  await stage.press('Enter');
  await stage.press('ArrowLeft');
  await stage.press('Escape');
  await expect(region).toBeVisible();
  await expect(region.getByRole('button', { name: /^Mark \d/ })).toHaveCount(0);
  await stage.press('Enter');
  await stage.press('Shift+ArrowRight');
  await stage.press('Shift+ArrowDown');
  await stage.press('Enter');
  await region.getByRole('textbox', { name: 'Comment for mark 1' }).fill('This region');
  await page.keyboard.press('Enter');
  await expect.poll(async () => (await drafts(page))[0]?.marks[0]).toEqual({ kind: 'box', x: 12, y: 15, w: 28, h: 15, note: 'This region' });
});

test('numbered entries edit and delete with focus recovery and retained redo', async ({ page }) => {
  const region = await openAnnotation(page);
  for (const n of [1, 2]) {
    await region.getByRole('button', { name: 'Comment on the whole page' }).click();
    const note = region.getByRole('textbox', { name: `Comment for mark ${n}` });
    await note.fill(`Note ${n}`);
    await note.press('Delete');
    await expect(region.getByRole('button', { name: /^Mark \d/ })).toHaveCount(n);
    await note.press('Escape');
  }
  const first = region.getByRole('button', { name: 'Mark 1: Note 1', exact: true });
  await first.focus();
  await first.press('Delete');
  const survivor = region.getByRole('button', { name: 'Mark 1: Note 2', exact: true });
  await expect(survivor).toBeFocused();
  await survivor.press('Enter');
  await region.getByRole('textbox', { name: 'Comment for mark 1' }).fill('Edited note');
  await page.keyboard.press('Enter');
  await region.getByRole('button', { name: 'Mark 1: Edited note', exact: true }).press('Backspace');
  await expect(region.getByRole('application', { name: 'Annotation stage' })).toBeFocused();
  await page.keyboard.press('Shift+Meta+z');
  await expect(region.getByRole('button', { name: 'Mark 1: Edited note', exact: true })).toBeVisible();
});

test('whole-page-only Add to chat skips inspection and Draw remains pointer-only', async ({ page }) => {
  const region = await openAnnotation(page);
  const tools = region.getByRole('group', { name: 'Annotation tool' });
  await tools.getByRole('button', { name: 'Draw', exact: true }).click();
  await expect(tools.locator('[aria-pressed="true"]')).toHaveCount(1);
  const stage = region.getByRole('application', { name: 'Annotation stage' });
  await stage.focus();
  await stage.press('Enter');
  await expect(region.getByRole('button', { name: /^Mark \d/ })).toHaveCount(0);
  await expect(region).toContainText('Draw uses a pointer');
  await region.getByRole('button', { name: 'Comment on the whole page' }).click();
  await region.getByRole('textbox', { name: 'Comment for mark 1' }).fill('Page only');
  await page.keyboard.press('Enter');
  await region.getByRole('button', { name: 'Add to chat' }).click();
  await expect(region).toHaveCount(0);
  await expect(page.getByLabel('Chat message')).toHaveValue(/1\. Whole page: Page only/);
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('mockBrowserLog') ?? '[]').filter((c: { command: string }) => c.command === 'browser_inspect'))).toEqual([]);
});

test('mixed page and point marks inspect only the point and keep original numbering', async ({ page }) => {
  const region = await openAnnotation(page);
  await region.getByRole('button', { name: 'Comment on the whole page' }).click();
  await region.getByRole('textbox', { name: 'Comment for mark 1' }).fill('Whole page');
  await page.keyboard.press('Enter');
  const stage = region.getByRole('application', { name: 'Annotation stage' });
  await stage.focus();
  await stage.press('Enter');
  await region.getByRole('textbox', { name: 'Comment for mark 2' }).fill('Specific button');
  await page.keyboard.press('Enter');
  await region.getByRole('button', { name: 'Add to chat' }).click();
  await expect(page.getByLabel('Chat message')).toHaveValue(/1\. Whole page: Whole page\n2\. Specific button — on <button\.cta>/);
  const points = await page.evaluate(() => JSON.parse(localStorage.getItem('mockBrowserLog') ?? '[]').find((c: { command: string }) => c.command === 'browser_inspect').args.points);
  expect(points).toHaveLength(1);
});
