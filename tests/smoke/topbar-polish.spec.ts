import { expect, test } from './fixtures/test';
import { seedSubagents } from './subagentsFixture';

const capture = process.env.POLISH_CAPTURE_DIR;
const before = process.env.POLISH_CAPTURE_REV === 'before';

for (const width of [1440, 800]) for (const theme of ['light', 'dark']) {
  test(`chat card and pane at ${width} in ${theme}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.addInitScript(t => localStorage.setItem('theme', t), theme);
    await seedSubagents(page);
    await page.goto('/?mock=1&demo=1');
    await page.getByRole('button', { name: 'Open Q3 regional operations review', exact: true }).click();
    const composer = page.getByRole('textbox', { name: 'Chat message', exact: true });
    await composer.fill('Keep this draft');
    if (before) await page.getByRole('button', { name: 'Side panel', exact: true }).click();
    const card = page.locator('#side-panel-card');
    await expect(card).toBeVisible();
    await expect(card).toContainText('1 running, 1 queued, 3 done');
    if (!before) {
      await expect(composer).toBeFocused();
      await expect(page.locator('.work-conversation .chat-navigation')).toHaveCount(0);
      await expect(page.locator('.work-card-toggle')).toHaveCount(0);
      await expect(page.getByRole('button', { name: 'Actions for this conversation', exact: true })).toBeVisible();
    }
    if (capture) await page.screenshot({ path: `${process.env.POLISH_CAPTURE_DIR}/${before ? 'before' : 'after'}-${width}-${theme}-card.png` });
    await page.getByRole('button', { name: 'Project files', exact: true }).click();
    const pane = page.getByRole('complementary', { name: 'Preview', exact: true });
    await expect(pane).toBeVisible();
    await expect(card).toHaveCount(0);
    if (capture) await page.screenshot({ path: `${process.env.POLISH_CAPTURE_DIR}/${before ? 'before' : 'after'}-${width}-${theme}-pane.png` });
    await pane.getByRole('button', { name: /Close.*[Pp]review/ }).click();
    await expect(composer).toHaveValue('Keep this draft');
    if (!before) {
      await expect(card).toBeVisible();
      await page.emulateMedia({ reducedMotion: 'reduce' });
      await expect.poll(() => card.evaluate(el => getComputedStyle(el).animationName)).toBe('none');
      await page.emulateMedia({ reducedMotion: 'no-preference' });
      await page.evaluate(() => { document.documentElement.dataset.motion = 'reduced'; });
      await expect.poll(() => card.evaluate(el => getComputedStyle(el).animationName)).toBe('none');
    }
  });
}

test('the summary follows live counts and file arrivals without moving focus', async ({ page }) => {
  test.skip(before, 'behavior introduced by this patch');
  await seedSubagents(page);
  await page.goto('/?mock=1&demo=1');
  await page.getByRole('button', { name: 'Open Q3 regional operations review', exact: true }).click();
  const card = page.locator('#side-panel-card');
  const composer = page.getByRole('textbox', { name: 'Chat message', exact: true });
  await card.getByRole('button', { name: /^Outputs \d/ }).click();
  await card.getByRole('button', { name: /^Subagents / }).click();
  await composer.fill('Still writing');
  await expect(card).toContainText('1 running, 1 queued, 3 done');
  await page.evaluate(async () => {
    const views = JSON.parse(localStorage.getItem('mockSubagents')!);
    for (const view of Object.values(views) as any[]) {
      view.done = [...view.active.map((row: any) => ({ ...row, status: 'done', agent: 'future-agent' })), ...view.done];
      view.active = []; view.running = 0; view.queued = 0; view.doneCount = 5;
    }
    localStorage.setItem('mockSubagents', JSON.stringify(views));
    const queuePath = '/src/store/runQueue.ts';
    const queue = await import(/* @vite-ignore */ queuePath);
    queue.setRunQueue([]);
    const workspacePath = '/src/store/workspaces.ts';
    const work = await import(/* @vite-ignore */ workspacePath);
    const detail = work.workspaceDetail();
    work.setWorkspaceDetail({ ...detail, files: [...detail.files, {
      path: `${detail.workspace.path}/outputs/new-analysis.md`, name: 'new-analysis.md', kind: 'output', modified: '2099-01-01T00:00:00Z', size: 20,
    }] });
  });
  await expect(card).toContainText('5 done');
  await expect(card.locator('[data-agent="future-agent"]').first()).toBeVisible();
  await expect(card.getByRole('button', { name: /new-analysis/ })).toBeVisible();
  await expect(composer).toBeFocused();
  await expect(composer).toHaveValue('Still writing');
});

test('an empty quick chat keeps its card and composer inside the chat', async ({ page }, info) => {
  test.skip(before, 'behavior introduced by this patch');
  // As quick-chats.spec.ts: the narrow rail is icons only, with no Quick chats +.
  test.skip(info.project.name.includes('narrow'), 'The rail tree collapses to icons on narrow screens');
  await page.goto('/?mock=1&demo=1');
  await page.getByRole('button', { name: 'New quick chat', exact: true }).click();
  const card = page.locator('#side-panel-card');
  const composer = page.getByRole('textbox', { name: 'Chat message', exact: true });
  await expect(card).toBeVisible();
  await expect(card).toContainText('None yet');
  await expect(composer).toBeFocused();
  await expect(composer).toBeInViewport();
  const a = (await card.boundingBox())!, b = (await composer.boundingBox())!;
  expect(a.y + a.height <= b.y || a.x >= b.x + b.width).toBe(true);
});

for (const width of [1440, 900, 800, 600]) test(`summary shrinks without covering text at ${width}`, async ({ page }) => {
  test.skip(before, 'responsive summary introduced by this patch');
  await page.setViewportSize({ width, height: 900 });
  await seedSubagents(page);
  await page.goto('/?mock=1&demo=1');
  await page.getByRole('button', { name: 'Open Q3 regional operations review', exact: true }).click();
  const card = page.locator('#side-panel-card');
  await expect(card).toContainText('1 running, 1 queued, 3 done');
  const size = await card.evaluate(el => {
    const card = el.getBoundingClientRect();
    const wrap = el.closest('.chat-scroll-wrap')!.getBoundingClientRect();
    const textRight = Math.max(...Array.from(el.parentElement!.querySelectorAll('.chat-scroll .turn p, .chat-scroll pre, .chat-scroll .chat-canvas'), node => node.getBoundingClientRect().right));
    return { card: card.width, wrap: wrap.width, textRight, cardLeft: card.left };
  });
  if (width === 1440) expect(size.card).toBeCloseTo(280, 0);
  else {
    expect(size.card).toBeLessThanOrEqual(201);
    expect(size.card).toBeLessThanOrEqual(size.wrap * .3 + 1);
  }
  expect(size.textRight).toBeLessThanOrEqual(size.cardLeft);
  await expect(page.getByRole('textbox', { name: 'Chat message', exact: true })).toBeInViewport();
});
