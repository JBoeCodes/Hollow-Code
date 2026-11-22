import { expect, test, type Page } from './fixtures/test';
import { showSettingsPart } from './settingsPart';
import { choose, chooseNew } from './choose';

/** Settings, skills, memory and onboarding (next-level P18), on `?mock=1`.
 * Desktop index only: below 860px the same sections are a selector, which
 * app-smoke covers. */
test.skip(({ viewport }) => (viewport?.width ?? 1000) <= 860, 'the desktop settings index');

const index = (page: Page) => page.getByRole('navigation', { name: 'Settings sections' });
async function openSettings(page: Page, section?: string) {
  await page.goto('/?mock=1');
  await page.getByLabel('Main navigation').getByRole('button', { name: /^Settings/ }).first().click();
  if (section) await index(page).getByRole('button', { name: section, exact: true }).click();
}

test('the index is one list of seventeen sections, and a search hit jumps to its control', async ({ page }) => {
  await openSettings(page);
  // PR 3: one flat list with no group caps; thirteen since U5 PR 2 added Updates (2026-11-15).
  await expect(index(page).locator('.settings-index-cap')).toHaveCount(0);
  await expect(index(page).locator('.settings-index-item')).toHaveText(['General', 'Chat', 'Models', 'Accounts', 'Personalization', 'Connectors', 'Sources', 'Background work', 'Devices', 'Usage', 'Keyboard shortcuts', 'Diffs', 'Browser', 'Worktrees', 'Archived', 'Updates', 'Advanced']);
  // One search, one choice: the control's own name is its accessible name.
  // Start locally was cut in Settings PR 6 (C2): the search for it lands on its part, Hollow folder.
  // (Searched by its name since 17a3052cf, 11-13, dropped "new task" from Hollow folder's words.)
  await page.getByLabel('Find a setting').fill('start locally');
  await index(page).getByRole('button', { name: 'Hollow folder', exact: true }).click();
  await expect(page.locator('.settings-found')).toBeVisible();
  // Hollow folder is a part of Sources since PR 3.
  await expect(page.getByRole('heading', { name: 'Hollow folder', exact: true, level: 3 })).toBeVisible();
});

test('first run asks only for the folder; services are labelled optional', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockSetupConfigured', 'false'));
  await page.goto('/?mock=1&setup=1');
  const steps = page.getByRole('list', { name: 'Getting started' });
  await expect(steps.getByRole('listitem')).toHaveCount(3);
  await expect(steps.getByRole('listitem').nth(0)).toContainText('No account or connector');
  await expect(steps.getByRole('listitem').nth(1)).toContainText('a workspace, a note and a task');
  await expect(steps.getByRole('listitem').nth(2)).toContainText('Optional');
});

test('a local note and project need no connector, and Settings no longer repeats the buttons', async ({ page }) => {
  // Start locally was cut in Settings PR 6 (C2): the File menu, the palette and Library make them.
  await openSettings(page, 'Sources');
  await expect(page.getByRole('heading', { name: 'Start locally' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Write a note' })).toHaveCount(0);

  await page.getByLabel('Main navigation').getByRole('button', { name: 'Library', exact: true }).click();
  await page.getByRole('button', { name: 'New note' }).click();
  await page.getByLabel('Note name').fill('First local note');
  await page.getByRole('button', { name: 'Create note' }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__mockApiCalls?.vault_create_note ?? 0)).toBe(1);

  // The task and the project are made where they live now: the Tasks page and the sidebar's New project.
  await page.getByLabel('Main navigation').getByRole('button', { name: /^Tasks/ }).first().click();
  await page.getByRole('button', { name: 'New local task', exact: true }).first().click();
  await page.getByRole('textbox', { name: 'Task name', exact: true }).fill('First local task');
  await page.getByRole('button', { name: 'Create task', exact: true }).click();
  await expect.poll(() => page.evaluate(() => ((window as any).__mockCreatedTasks ?? []).length)).toBe(1);

  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Produce a checked local report.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Local first');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Local first');

  // Nothing above touched a connector or a sign-in.
  const calls = await page.evaluate(() => (window as any).__mockApiCalls ?? {});
  for (const c of ['save_connector', 'import_connector', 'check_connectors', 'm365_sign_in']) expect(calls[c] ?? 0, c).toBe(0);
  expect(await page.evaluate(() => localStorage.getItem('mockConnectors'))).toBeNull();
});

test('keyboard shortcuts no longer list the in-surface keys (Settings PR 6, C7); the ? card still does', async ({ page }) => {
  await openSettings(page, 'Keyboard shortcuts'); // a section again since PR 3
  await expect(page.getByRole('region', { name: 'Keys inside a surface' })).toHaveCount(0);
  await page.getByRole('button', { name: 'Show all keys', exact: true }).click();
  const keys = page.getByRole('dialog', { name: 'Keyboard shortcuts' });
  await expect(keys).toContainText(/make a task from the selection/i);
  await page.keyboard.press('Escape');
  await page.getByRole('switch', { name: 'Use keyboard shortcuts' }).click();
  await expect(page.locator('#settings-shortcuts .settings-section-status')).toHaveText('Off');
});

test('memory shows where each item came from and what superseded it', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('button',{name:'Library',exact:true}).click();
  await page.getByRole('navigation',{name:'Library view'}).getByRole('button',{name:'Learnings',exact:true}).click();
  const review = page.locator('.learning-review');
  await review.getByRole('navigation',{name:'Learning state'}).getByRole('button',{name:/^Saved/}).click();
  await expect(review).toContainText('A conversation (run-chat-0).');
  await expect(review).toContainText('Replaces');
  await choose(review.getByRole('combobox',{name:'Saved learning state'}),'replaced');
  const card = review.locator('.review-card').filter({ hasText: 'Where tasks live' });
  await expect(card).toContainText('outdated');
  await expect(card).toContainText('Replaced by');
  await expect(card.getByRole('button',{name:'learnings/use-vault.md',exact:true})).toBeVisible();
});

test('skills say which copy the selected agent may load, and which are on request', async ({ page }) => {
  await openSettings(page, 'Connectors'); // Skills is a part of Connectors (PR 3)
  await page.getByRole('button', { name: 'Open Library, Skills', exact: true }).click(); // Skills moved to Library in Settings PR 4
  await page.locator('.skills-row').filter({ hasText: 'humanizer' }).click();
  const detail = page.getByRole('region', { name: 'Skill details' });
  await expect(detail.locator('.skill-availability')).toContainText('may load the Claude copy by itself');
  await page.getByRole('group', { name: 'Available to' }).getByRole('button', { name: 'Codex' }).click();
  await expect(detail.locator('.skill-availability')).toContainText('The Use in button asks it to read the Shared copy');
});

test('Search inside project files: the switch shows progress, and off says so', async ({ page }) => {
  // Repo features R14b: src/mockDirectories.ts's mockContentIndexStatus.
  // Advanced → Search & indexes (a closed group) since Settings PR 3.
  await openSettings(page, 'Advanced');
  await page.locator('#settings-indexes > summary').click();
  // Off until switched on (2026-10-19).
  const sw = page.getByRole('switch', { name: 'Search inside project files (local index)' });
  await expect(sw).toHaveAttribute('aria-checked', 'false');
  const progress = page.locator('.settings-content-index');
  await expect(progress).toContainText('Off: agents find project files by name and date only.');
  await sw.click();
  await expect(sw).toHaveAttribute('aria-checked', 'true');
  await expect(progress).toHaveText('2,803 of 2,803 files');
  await sw.click();
  await expect(sw).toHaveAttribute('aria-checked', 'false');
  await expect(progress).toContainText('Off: agents find project files by name and date only.');
});
