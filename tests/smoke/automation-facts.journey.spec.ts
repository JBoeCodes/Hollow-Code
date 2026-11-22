import { expect, test } from './fixtures/test';

async function automations(page: import('@playwright/test').Page) {
  await page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Automations', exact: true }).click();
}

test('Largest text keeps automation state and routine words whole', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1100 });
  await page.addInitScript(() => localStorage.setItem('hollow.studio.textScale', '125'));
  await page.goto('/?mock=1'); await automations(page);
  const row = page.locator('.workflow-row').first();
  await expect(row).toBeVisible();
  for (const label of [row.locator('.meta > span').first(), row.locator('.workflow-routine-chip')]) {
    const dimensions = await label.evaluate(el => ({ height: el.getBoundingClientRect().height, line: parseFloat(getComputedStyle(el).lineHeight) }));
    expect(dimensions.height).toBeLessThan(dimensions.line * 1.6);
  }
});

test('trigger-only automations belong to Scheduled or triggered', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.evaluate(async () => {
    const path = '/src/api.ts'; const { api } = await import(/* @vite-ignore */ path);
    const rows = await api.listWorkflows();
    api.listWorkflows = async () => [{ ...rows[0], slug: 'trigger-only', title: 'Synthetic trigger', schedule: 'manual', triggers: ['task-overdue'], permissionHold: null, budgetHold: null, scheduleProblem: null, lastStatus: 'ok', lastError: null }];
  });
  await automations(page);
  await expect(page.locator('.workflow-group-label')).toHaveText(['Scheduled or triggered 1']);
  await expect(page.getByRole('button', { name: 'Open Synthetic trigger', exact: true })).toBeVisible();
});
