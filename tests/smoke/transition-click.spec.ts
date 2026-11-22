import { showWorkspaceList } from './choose';
import { expect, test } from './fixtures/test';

/**
 * A click during a page transition is never lost (overnight 10-11/12 P4).
 * While a view transition plays, Chromium hit-tests the document element,
 * whatever `::view-transition { pointer-events }` says: a click in the rail
 * 40 ms after opening a workspace did nothing (0 of 6 on main `49f67adf`).
 * viewTransitions.ts ends the motion and hands the click to what is under
 * the pointer. Real mouse clicks: Playwright's own `click()` waits until the
 * target can be hit, which hides exactly this.
 */
test.use({ reducedMotion: 'no-preference' });
test.beforeEach(async ({}, info) => { test.skip(info.project.name.includes('narrow'), 'The rail collapses to icons on narrow screens'); });

async function openWorkspaceByMouse(page: import('@playwright/test').Page) {
  // Launch has its own entrance and reopens the last conversation: go to
  // Work once that has settled.
  await expect(page.locator('html')).not.toHaveClass(/is-booting/);
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: /^Projects/ }).click();
  await expect(page.locator('html')).not.toHaveClass(/is-transitioning/);
  // A workspace card on Work: the person's own open, which animates.
  await showWorkspaceList(page);
  const row = page.locator('.home-project-row .work-card').first();
  await row.scrollIntoViewIfNeeded();
  await expect(row).toBeInViewport();
  const box = (await row.boundingBox())!;
  await page.mouse.click(box.x + 60, box.y + box.height / 2);
  await page.waitForTimeout(40);
  expect(await page.evaluate(() => document.documentElement.classList.contains('is-transitioning')), 'the switch is still animating').toBe(true);
}

test('a click on Library mid-transition opens Library', async ({ page }) => {
  await page.goto('/?mock=1&demo=large');
  const library = (await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true }).boundingBox())!;
  await openWorkspaceByMouse(page);
  await page.mouse.click(library.x + 30, library.y + library.height / 2);
  await expect(page.getByRole('heading', { name: 'Library', exact: true })).toBeVisible({ timeout: 1500 });
  await expect(page.locator('html')).not.toHaveClass(/is-transitioning/);
});

test('a click over the page mid-transition only ends the motion', async ({ page }) => {
  await page.goto('/?mock=1&demo=large');
  await openWorkspaceByMouse(page);
  // Read without waiting, so the click still lands mid-transition.
  const before = await page.evaluate(() => ({
    title: document.querySelector('.work-header h1')?.textContent ?? null,
    box: document.querySelector('.canvas-split > .page')!.getBoundingClientRect().toJSON() as DOMRect,
    moving: document.documentElement.classList.contains('is-transitioning'),
  }));
  expect(before.moving, 'still animating at the click').toBe(true);
  await page.mouse.click(before.box.x + before.box.width / 2, before.box.y + before.box.height / 2);
  // Ended by the click: a cross-fade left to run takes ~300 ms more.
  await expect(page.locator('html')).not.toHaveClass(/is-transitioning/, { timeout: 150 });
  await expect(page.locator('.work-header h1')).toHaveText(before.title ?? '');
});
