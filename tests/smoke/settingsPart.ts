import { expect, type Page } from '@playwright/test';

/** Bring a lazy Settings part (S2: Keyboard shortcuts, Accounts, Skills,
 *  History & recovery's run list) near the screen and wait until it has
 *  mounted. A section switch scrolls back to the top on its next frame, so
 *  the scroll is retried until the part's placeholder is gone. */
export async function showSettingsPart(page: Page, id: string) {
  const part = page.locator(`#settings-${id}`);
  await expect(async () => {
    await part.scrollIntoViewIfNeeded();
    await expect(part.locator('.settings-part-pending')).toHaveCount(0, { timeout: 1000 });
  }).toPass({ timeout: 10_000 });
}
