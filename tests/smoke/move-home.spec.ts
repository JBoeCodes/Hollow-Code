import { expect, test, type Page } from './fixtures/test';

/**
 * Move Hollow home (2026-11-12): Settings → Sources → Hollow folder → Move
 * Hollow home, the person's click path for system.move_hollow_home_plan,
 * system.move_hollow_home and system.rollback_move_home. Mock only
 * (src/mockMoveHome.ts): nothing on disk moves and the app never quits;
 * localStorage `mockMoveHomeChatRunning`, `mockMoveHomeState` and
 * `mockMoveHomeRefuse` pick the variant, and starts and rollbacks are logged
 * in `mockMoveHomeCalls`.
 */

// N53 (ENG-29, 2026-11-15): 390 px is below the 720 px minimum window, so the
// narrow project skips this panel's click path; the desktop and minimum
// projects keep running it.
test.beforeEach(({}, info) => {
  test.skip(info.project.name.includes('narrow'), 'Below the 720 px minimum window; the Move Hollow home panel is checked at desktop width');
});

const calls = (page: Page, command: string) => page.evaluate(c => window.__mockApiCalls?.[c] ?? 0, command);
const list = (page: Page, name: string) => page.getByRole('list', { name, exact: true });

async function openHollowFolder(page: Page, seed: Record<string, string> = {}) {
  await page.addInitScript(s => { for (const [k, v] of Object.entries(s)) localStorage.setItem(k, v); }, seed);
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).waitFor();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  // Below 860 px the index keeps only its search, so the section picker decides the path.
  const picker = page.getByRole('combobox', { name: 'Settings section', exact: true });
  if (await picker.isVisible()) {
    await picker.click();
    await page.getByRole('option', { name: 'Sources', exact: true }).click();
  } else {
    await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Sources', exact: true }).click();
  }
  await expect(page.getByRole('heading', { name: 'Move Hollow home' })).toBeVisible();
  // The panel sits in a closed group (FIX-4, 11-13): its heading opens it.
  await page.getByRole('heading', { name: 'Move Hollow home' }).click();
  await expect(page.getByRole('button', { name: 'Check the move' })).toBeVisible();
}

test('Move Hollow home: Check the move shows the report, Quit and move waits for a running chat', async ({ page }) => {
  await openHollowFolder(page, { mockMoveHomeChatRunning: '1' });
  await page.getByRole('button', { name: 'Check the move' }).click();

  await expect(page.getByText(/^Moves 4 folders \(47 GB, 122,712 files\) into \/Users\/mock\/Hollow\./)).toBeVisible();
  await expect(list(page, 'What moves')).toContainText('Vault: /Users/mock/Documents/projects/hollow-vault → /Users/mock/Hollow/Vault');
  await expect(list(page, 'What moves')).toContainText('Personal projects: /Users/mock/Hollow/Projects/personal → /Users/mock/Hollow/Projects/Personal');
  await expect(list(page, 'Links left at the old places')).toContainText('/Users/mock/Documents/projects/hollow-vault links to /Users/mock/Hollow/Vault');
  await expect(list(page, 'Records updated to the new paths')).toContainText('Conversations sessions.cwd: 412 rows');
  await expect(list(page, 'Name clashes')).toContainText('It goes to Archive.');
  await expect(list(page, 'Files in iCloud only')).toContainText('1,200 files are in iCloud only');
  await expect(list(page, 'What must stop first')).toContainText('Quarterly review draft');
  await expect(list(page, 'Git repositories to repair')).toContainText('2 worktrees to repair');
  await expect(list(page, 'Backup')).toContainText('couldn’t tell');

  // A running chat: the button stays, disabled, with the reason.
  const quit = page.getByRole('button', { name: 'Quit and move…' });
  await expect(quit).toBeDisabled();
  await expect(page.getByText('Wait for 1 chat to finish or stop it, then check again.')).toBeVisible();
  await quit.click({ force: true });
  await expect(page.getByRole('group', { name: 'Confirm the move' })).toHaveCount(0);

  // The option checks again with the folders beside the vault.
  await page.getByRole('checkbox', { name: 'Also move the folders beside the vault' }).check();
  await expect(list(page, 'What moves')).toContainText('/Users/mock/Hollow/Projects/morning-paper');
  await expect(list(page, 'Name clashes')).toContainText('Kept in Projects: /Users/mock/Documents/Old Mac/projects/pricing-study');
  expect(await calls(page, 'move_home_start')).toBe(0);
});

test('Move Hollow home: Quit and move asks again, then shows each step until done', async ({ page }) => {
  await openHollowFolder(page);
  await page.getByRole('button', { name: 'Check the move' }).click();
  await page.getByRole('button', { name: 'Quit and move…' }).click();
  const confirm = page.getByRole('group', { name: 'Confirm the move' });
  await expect(confirm).toContainText('Hollow quits, moves 4 folders into /Users/mock/Hollow and opens again.');
  await confirm.getByRole('button', { name: 'Cancel' }).click();
  expect(await calls(page, 'move_home_start')).toBe(0);

  await page.getByRole('button', { name: 'Quit and move…' }).click();
  await page.getByRole('button', { name: 'Yes, quit and move' }).click();
  await expect(page.getByText(/Hollow quits in a moment/)).toBeVisible();
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('mockMoveHomeCalls') ?? '[]'))).toEqual([{ command: 'move_home_start', hash: 'mock-move-home-base' }]);
  await expect(page.getByRole('status', { name: 'Move progress' })).toContainText('Moved.', { timeout: 15_000 });
  await expect(list(page, 'Move steps')).toContainText('Checking the result');
  await expect(page.getByRole('button', { name: 'Roll back…' })).toBeEnabled();
});

test('Move Hollow home: a finished move shows its checks and Roll back…', async ({ page }) => {
  await openHollowFolder(page, { mockMoveHomeState: 'done' });
  await expect(page.getByRole('status', { name: 'Move progress' })).toContainText('Moved. All 3 checks passed.');
  await expect(list(page, 'Checks after the move')).toContainText('File counts: 122,712 files before and after');
  const rollBack = page.getByRole('button', { name: 'Roll back…' });
  await expect(rollBack).toBeEnabled();
  await rollBack.click();
  const confirm = page.getByRole('group', { name: 'Confirm the rollback' });
  await expect(confirm).toBeVisible();
  await confirm.getByRole('button', { name: 'Cancel' }).click();
  expect(await calls(page, 'move_home_rollback')).toBe(0);
  await rollBack.click();
  await page.getByRole('button', { name: 'Yes, roll back' }).click();
  await expect(page.getByRole('status', { name: 'Move progress' })).toContainText('Rolled back.');
  await expect(rollBack).toHaveCount(0);
});

test('Move Hollow home: a rollback that would be refused says why', async ({ page }) => {
  await openHollowFolder(page, { mockMoveHomeState: 'done', mockMoveHomeRefuse: '1' });
  await expect(page.getByRole('button', { name: 'Roll back…' })).toBeDisabled();
  await expect(page.getByText(/Copy them out of \/Users\/mock\/Hollow\/Vault first, then roll back\./)).toBeVisible();
});
