import { expect, test, type Page } from './fixtures/test';
import { runningList } from './choose';

async function openBrief(page: Page) {
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Automations', exact: true }).click();
  await page.getByRole('button', { name: 'Open Morning Brief', exact: true }).click();
  await expect(page.locator('.workflow-review')).toBeVisible();
}

test('the run strip keeps its explanation and keyboard routes to preview and edit', async ({ page }) => {
  await openBrief(page);
  const strip = page.locator('.workflow-output-actions');
  const run = strip.getByRole('button', { name: 'Run now', exact: true });
  await run.focus();
  await expect(run).toBeFocused();
  await expect(run).toHaveAccessibleDescription(/Claude works in the vault and uses model allowance.*Today: 3 runs.*Budget for the “morning-brief” automation/s);
  await expect(page.locator('#workflow-run-note')).toBeVisible();
  const stats = page.locator('.workflow-cover-stats');
  await expect(stats).toContainText('Schedule');
  await expect(stats).toContainText('Next run');
  await expect(stats).toContainText('Last run');
  const note = page.locator('#workflow-run-note');
  expect(await note.evaluate(el => el.previousElementSibling?.classList.contains('workflow-output-actions'))).toBe(true);
  expect(await note.evaluate(el => el.nextElementSibling?.classList.contains('workflow-cover-stats'))).toBe(true);
  const options = strip.getByRole('button', { name: 'Automation options' });
  await options.focus();
  await page.keyboard.press('ArrowDown');
  const preview = page.getByRole('menuitem', { name: 'Preview configuration' });
  await expect(preview).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('region', { name: 'Configuration preview' })).toBeVisible();
  await expect(options).toBeFocused();
  await options.press('ArrowDown');
  await expect(preview).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('region', { name: 'Configuration preview' })).toBeVisible();
  expect(await page.evaluate(() => window.__mockApiCalls?.run_workflow ?? 0)).toBe(0);
  await page.getByRole('button', { name: 'Close preview' }).click();
  await options.focus();
  await page.keyboard.press('ArrowDown');
  await expect(preview).toBeFocused();
  await page.keyboard.press('End');
  await expect(page.getByRole('menuitem', { name: 'Edit', exact: true })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('textbox', { name: 'Working folder' })).toBeVisible();
});

test('failure recovery and the full result explanation remain reachable at narrow Largest text', async ({ page }) => {
  await page.setViewportSize({ width: 720, height: 600 });
  await page.addInitScript(() => localStorage.setItem('hollow.studio.textScale', '125'));
  await openBrief(page);
  await expect(page.locator('html')).toHaveAttribute('data-text-scale', '125');
  const options = page.getByRole('button', { name: 'Automation options' });
  await options.click();
  const menu = page.getByRole('menu', { name: 'Automation options' });
  await expect(menu).toBeInViewport();
  const bounds = await menu.boundingBox();
  expect(bounds!.x).toBeGreaterThanOrEqual(0);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(720);
  await page.keyboard.press('End');
  const edit = page.getByRole('menuitem', { name: 'Edit', exact: true });
  await expect(edit).toBeFocused();
  const editBounds = await edit.boundingBox();
  expect(editBounds!.y).toBeGreaterThanOrEqual(0);
  // Fractional text metrics may put the final border a fraction of a pixel
  // beyond the scroll viewport; the full item must remain within one pixel.
  expect(editBounds!.y + editBounds!.height).toBeLessThanOrEqual(601);
  await page.keyboard.press('Escape');
  const fix = page.getByRole('status', { name: 'What to fix' });
  await fix.scrollIntoViewIfNeeded();
  await expect(fix).toBeInViewport();
  await expect(fix.getByRole('button', { name: 'Open Accounts' })).toBeVisible();
  const about = page.locator('details.workflow-derived-note');
  await expect(about.locator('p')).toBeHidden();
  await about.locator('summary').focus();
  await page.keyboard.press('Enter');
  await expect(about.locator('p')).toContainText('Name them under Edit → Destination → Files it produces.');
  await expect(about.locator('p')).toBeVisible();
  await expect.poll(() => page.locator('.workflow-review').evaluate(el => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(1);
  await fix.getByRole('button', { name: 'Open Accounts' }).click();
  await expect(page.getByRole('heading', { name: 'Accounts', exact: true })).toBeVisible();
});

test('a live automation still has a reachable Stop from its Running card', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockRunQueue', JSON.stringify([{
    id: 11, source: 'cron', sourceId: 'morning-brief', label: 'Morning brief',
    detail: null, reason: 'scheduled', status: 'running', priority: 3, position: null,
    attempt: 0, coalescedCount: 0, requestedAt: '2026-10-20T03:00:00Z',
    claimedAt: '2026-10-20T03:01:00Z', finishedAt: null, historyRunId: 'r1',
    error: null, waitingSince: null, waitingHolder: null, heldReason: null, cancelRequested: false,
  }])));
  await openBrief(page);
  // The top bar dock's queue rows went in overnight 11-04 U1: Running's card
  // for the run keeps the Stop, behind its confirm, through the same cancel.
  const card = (await runningList(page)).getByRole('article', { name: /^Morning brief,/ });
  await card.getByRole('button', { name: 'Stop', exact: true }).click();
  await page.getByRole('dialog', { name: 'Stop “Morning brief”?' }).getByRole('button', { name: 'Stop', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.cancel_queued_run ?? 0)).toBe(1);
});
