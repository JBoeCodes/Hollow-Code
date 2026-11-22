import type { Page } from '@playwright/test';

/** Settings → Usage opens on Limits (Settings PR 8); this opens another tab. */
export async function showUsageTab(page: Page, name: 'Limits' | 'All agents' | 'Budgets' | string) {
  await page.getByRole('tablist', { name: 'Usage view' }).getByRole('tab', { name, exact: true }).click();
}
