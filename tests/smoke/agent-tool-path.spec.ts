import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from './fixtures/test';
import { choose, openAdvancedGroup } from './choose';

/**
 * A12: Settings → Advanced → Diagnostics → Folders agents can run tools from. The read-only
 * disclosure lists the PATH every agent gets (`agent_tool_path`), each folder
 * with its source, missing folders muted and marked, and says so when the
 * login shell's PATH couldn't be read. Mock only (src/mockApi.ts): localStorage
 * `mockAgentToolPath` = 'failed' | 'pending' stands in for the shell's read.
 */

const narrow = (page: Page) => (page.viewportSize()?.width ?? 1000) <= 860;
const SUMMARY = 'Folders agents can run tools from';
const disclosure = (page: Page) => page.locator('details.agent-tool-path');
const folders = (page: Page) => page.getByRole('list', { name: 'Folders agents can run tools from, in order' });

async function openSystem(page: Page, mode?: 'failed' | 'pending') {
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).waitFor();
  // Read when the disclosure opens, so setting it now is in time.
  if (mode) await page.evaluate(m => localStorage.setItem('mockAgentToolPath', m), mode);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  if (narrow(page)) await choose(page.getByRole('combobox', { name: 'Settings section' }), 'advanced');
  else await page.locator('.settings-index').getByRole('button', { name: 'Advanced', exact: true }).click();
  // The row sits in Advanced's closed Diagnostics group (Settings PR 3).
  await openAdvancedGroup(page, 'diagnostics');
  await expect(disclosure(page).getByText(SUMMARY)).toBeVisible();
}

/** Opens the disclosure from the keyboard: its summary is a tab stop. */
async function expand(page: Page) {
  const summary = disclosure(page).locator('summary');
  await summary.focus();
  await expect(summary).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(disclosure(page)).toHaveAttribute('open', '');
  await expect(folders(page)).toBeVisible();
}

test('System lists the folders agents get, each with its source, and passes axe', async ({ page }) => {
  await openSystem(page);
  await expand(page);
  const shellRow = folders(page).locator('li[data-from="shell"]').first();
  await expect(shellRow).toBeVisible();
  await expect(shellRow).toContainText('/Users/you/google-cloud-sdk/bin');
  await expect(shellRow).toContainText('shell');
  // A missing folder says so in words, not only in its muted colour.
  await expect(folders(page).locator('li[data-missing]').first()).toContainText('missing');
  // A folder whose check timed out says so too (overnight Spare 1).
  await expect(folders(page).locator('li[data-unknown]')).toContainText("/Volumes/share/bin");
  await expect(folders(page).locator('li[data-unknown]')).toContainText("didn't answer");
  // The shell was read: no note.
  await expect(disclosure(page).getByText("Hollow couldn't read your shell's PATH", { exact: false })).toHaveCount(0);
  await expect(disclosure(page).getByRole('button', { name: 'Copy the PATH agents get' })).toBeVisible();
  expect((await new AxeBuilder({ page }).include('details.agent-tool-path').withTags(['wcag2a', 'wcag2aa']).analyze()).violations).toEqual([]);
});

test("System says when the shell's PATH couldn't be read, and keeps Hollow's folders", async ({ page }) => {
  await openSystem(page, 'failed');
  await expand(page);
  await expect(disclosure(page).getByText("Hollow couldn't read your shell's PATH (timed out after 5000 ms). Agents get Hollow's default folders.")).toBeVisible();
  await expect(folders(page).locator('li[data-from="shell"]')).toHaveCount(0);
  await expect(folders(page).locator('li[data-from="hollow"]').first()).toBeVisible();
  expect((await new AxeBuilder({ page }).include('details.agent-tool-path').withTags(['wcag2a', 'wcag2aa']).analyze()).violations).toEqual([]);
});

// A12 review: a Settings search hit on this row lands with its disclosure
// focused (the row has no other control on screen), so Enter opens it.
test('a Settings search for gcloud lands on the row with its disclosure focused', async ({ page }) => {
  test.skip(narrow(page), 'the search field is the desktop index; the narrow picker is covered by required.minimum');
  await page.goto('/?mock=1');
  await page.getByLabel('Main navigation').getByRole('button', { name: /^Settings/ }).first().click();
  const summary = disclosure(page).locator('summary');
  const jump = async () => {
    await page.getByLabel('Find a setting').fill('gcloud');
    await page.locator('.settings-index').getByRole('button', { name: SUMMARY, exact: true }).click();
    await expect(summary).toBeFocused();
  };
  await jump();
  // Opened once (its Copy button mounts) and closed again: the hidden Copy
  // isn't what the jump focuses.
  await page.keyboard.press('Enter');
  await expect(page.getByRole('button', { name: 'Copy the PATH agents get' })).toBeVisible();
  await page.keyboard.press('Enter');
  await expect(disclosure(page)).not.toHaveAttribute('open', '');
  await page.getByLabel('Find a setting').fill('');
  await jump();
  await page.keyboard.press('Enter');
  await expect(disclosure(page)).toHaveAttribute('open', '');
  await expect(folders(page)).toBeVisible();
});
