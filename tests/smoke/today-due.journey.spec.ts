import { expect, test } from './fixtures/test';

// The Today Due card (Mark done, Snooze, coverage alerts) was cut with Today
// (overnight 11-08 S3); the Due today scope went with the issue-list rewrite
// (11-10). This journey checks the card is gone and Tasks is the list.

test('Work has no Today Due card, and Tasks is the list', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await expect(page.getByRole('navigation', { name: 'Projects views' }).getByRole('button', { name: 'Today', exact: true })).toHaveCount(0);
  await expect(page.getByRole('region', { name: 'Due', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: /Mark “Local due” done/ })).toHaveCount(0);
  await page.locator('.home-view-actions').getByRole('button', { name: 'Tasks', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Tasks', level: 1 })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Task list', exact: true })).toBeVisible();
});

test('a tasks read failure does not bring the Today Due card back', async ({ page }) => {
  await page.route('**/src/mockApi.ts*', async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: `${await response.text()}
    {
      const raw = mockInvoke;
      mockInvoke = async (command, args) => {
        if (command === 'tasks') throw Error('Synthetic due read failed');
        return raw(command, args);
      };
    }` });
  });
  await page.goto('/?mock=1&demo=1');
  await expect(page.getByRole('region', { name: 'Due', exact: true })).toHaveCount(0);
  await expect(page.getByRole('region', { name: 'Today content' })).toHaveCount(0);
  await page.locator('.home-view-actions').getByRole('button', { name: 'Tasks', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Tasks', level: 1 })).toBeVisible();
});
