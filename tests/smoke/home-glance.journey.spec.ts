import { expect, test } from './fixtures/test';

// Today and Weekly review were cut (overnight 11-08 S3). Work is Home and
// Running. The workspace map was cut (overnight 11-08 S4). Tasks opens from Home's own button.

test('daily destinations remain visible beside chat-first Home', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  const views = page.getByRole('navigation', { name: 'Projects views' });
  for (const name of ['Home', 'Running']) {
    await expect(views.getByRole('button', { name, exact: true })).toBeVisible();
  }
  await expect(views.getByRole('button', { name: 'Today', exact: true })).toHaveCount(0);
  await expect(page.getByRole('group', { name: 'Today range' })).toHaveCount(0);
  await expect(page.getByRole('textbox', { name: 'Ask anything' })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Projects' }).getByRole('button', { name: 'Map', exact: true })).toHaveCount(0);
  await views.getByRole('button', { name: 'Home', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Projects' })).toBeVisible();
  await page.locator('.home-view-actions').getByRole('button', { name: 'Tasks', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Tasks', exact: true })).toBeVisible();
});

test('Monday has no Weekly review screen', async ({ page }) => {
  await page.clock.setFixedTime(new Date('2026-10-19T10:00:00-04:00'));
  await page.goto('/?mock=1&demo=1');
  await expect(page.getByRole('navigation', { name: 'Projects views' }).getByRole('button', { name: 'Today', exact: true })).toHaveCount(0);
  await expect(page.getByRole('region', { name: 'Weekly review content' })).toHaveCount(0);
  await expect(page.locator('.wr-kicker')).toHaveCount(0);
  await expect(page.getByRole('textbox', { name: 'Ask anything' })).toBeVisible();
});

test('Friday keeps Running visible and has no weekly tab count', async ({ page }) => {
  await page.clock.install({ time: new Date('2026-10-16T14:59:30-04:00') });
  await page.goto('/?mock=1&demo=1');
  const running = page.getByRole('navigation', { name: 'Projects views' }).getByRole('button', { name: 'Running', exact: true });
  await expect(running).toBeVisible();
  await page.clock.fastForward(61_000);
  await expect(page.getByRole('navigation', { name: 'Projects views' }).getByRole('button', { name: 'Today', exact: true })).toHaveCount(0);
  await expect(running).toBeVisible();
  await expect(running.locator('.ui-tab-count')).toHaveCount(0);
  await running.click();
  await expect(page.getByRole('region', { name: 'Running', exact: true })).toBeVisible();
});
