// The Friday status (overhaul A18) and a canvas copy on an issue (A19),
// against the browser mock (`src/mockTracker.ts`: p-1 Receipts rebuild is
// tracked with workspace ws-receipts). Written in lane 4 of the COO rebuild
// without running it; the one full browser run on final main runs it.
import { expect, test } from './fixtures/test';
import { ROUTES } from '../../src/routeLedger';
import { home, walk } from './routeWalk';
import { choose, openTasks } from './choose';

test('Write status now writes the project’s status and says so', async ({ page }) => {
  await page.goto('/?mock=1');
  await openTasks(page);
  await page.locator('.issue-row[data-issue-id="i-31"] .issue-row-main').click();
  await page.getByRole('button', { name: 'Open project' }).click();
  const write = page.locator('.project-detail').getByRole('button', { name: 'Write status now' });
  await expect(write).toBeVisible();
  await write.click();
  await expect(page.getByText(/^Friday status for Receipts rebuild: \d+ done this week/).first()).toBeVisible();
});

test('the Friday status switches show in Settings and save', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), 'sources');
  } else {
    await page.locator('.settings-index').getByRole('button', { name: 'Sources', exact: true }).click();
  }
  const row = page.locator('.task-tracker-row');
  await row.locator('.task-tracker-friday-fold > summary').click();
  const friday = row.getByRole('switch', { name: 'Write each tracked project’s status at 3 PM on Fridays' });
  const copy = row.getByRole('switch', { name: 'Also post it to Linear as a project update' });
  await expect(friday).toHaveAttribute('aria-checked', 'false');
  await expect(copy).toHaveAttribute('aria-checked', 'false');
  await friday.click();
  await expect(friday).toHaveAttribute('aria-checked', 'true');
  await copy.click();
  await expect(copy).toHaveAttribute('aria-checked', 'true');
});

const attach = ROUTES.find(r => r.id === 'canvas.attach-to-issue')!;
test('a saved canvas attaches a copy to an issue once', async ({ page }) => {
  await home(page, attach);
  await walk(page, attach.walk);
  const dialog = page.getByRole('dialog', { name: 'Attach “Release status” to an issue', exact: true });
  await expect(dialog).toBeVisible();
  await dialog.getByRole('searchbox').fill('ENG-31');
  await dialog.getByRole('button', { name: /^ENG-31/ }).click();
  await dialog.getByRole('button', { name: 'Attach to ENG-31…' }).click();
  // Hollow's confirm card names the outward write first.
  await page.getByRole('button', { name: 'Attach to ENG-31', exact: true }).click();
  await expect(dialog.getByRole('status').last()).toContainText('as a copy; the canvas stays in Hollow');
  await dialog.getByRole('button', { name: 'Attach to ENG-31…' }).click();
  await page.getByRole('button', { name: 'Attach to ENG-31', exact: true }).click();
  await expect(dialog.getByRole('status').last()).toContainText('already on ENG-31');
});
