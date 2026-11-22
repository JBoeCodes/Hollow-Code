import { test, expect } from './fixtures/test';

// Parity 7.3: Projects → Pull requests lists a project's GitHub pull requests
// and issues (the mock stands in for gh), opens one with its checks and
// comments, drafts "Ask about this" in the project's chat, and asks in a
// dialog that names the repository before any triage move reaches GitHub.
// Clicks only.
test('GitHub pages: list, open, ask and triage behind a confirm', async ({ page }) => {
  await page.addInitScript(() => {
    if (sessionStorage.getItem('githubPagesSeeded')) return;
    sessionStorage.setItem('githubPagesSeeded', '1');
    localStorage.removeItem('hollow.workspace');
    localStorage.removeItem('mockGithubTriage');
    localStorage.setItem('mockWorkspaces', JSON.stringify([
      { id: 'app-1', title: 'App', status: 'active', path: 'workspaces/app-1', cwd: '/Users/fixture/Projects/app', modified: '2026-11-20T12:00:00Z' },
    ]));
  });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/?mock=1');
  await page.getByRole('complementary', { name: 'Main navigation' }).getByRole('button', { name: 'Projects', exact: true }).click();
  await page.getByRole('navigation', { name: 'Projects views' }).getByRole('button', { name: 'Pull requests', exact: true }).click();
  const region = page.getByRole('region', { name: 'Pull requests and issues' });
  await expect(region).toBeVisible();

  // The list, then the issue kind.
  const row = region.getByRole('button', { name: /#7 Add the thing/ });
  await expect(row).toBeVisible();
  await region.getByRole('group', { name: 'Show' }).getByRole('button', { name: 'Issues', exact: true }).click();
  await expect(region.getByRole('button', { name: /#12 Crash on launch/ })).toBeVisible();
  await region.getByRole('group', { name: 'Show' }).getByRole('button', { name: 'Pull requests', exact: true }).click();

  // The detail: state, checks, comments, and Ask about this drafts in the chat.
  await row.click();
  const detail = region.getByRole('region', { name: 'pull request details' });
  await expect(detail.getByRole('region', { name: 'Checks' })).toContainText('1 of 2 passed');
  await expect(detail.getByRole('region', { name: 'Comments' })).toContainText('Looks good to me.');
  await detail.getByRole('button', { name: 'Ask about this', exact: true }).first().click();
  await expect(page.getByRole('textbox', { name: 'Message' }).first()).toHaveValue(/About pull request #7 in o\/r: Add the thing/);

  // Back on the page, a triage move asks first, naming the repository; Cancel leaves GitHub alone.
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true }).click();
  await page.getByRole('navigation', { name: 'Projects views' }).getByRole('button', { name: 'Pull requests', exact: true }).click();
  await region.getByRole('button', { name: /#7 Add the thing/ }).click();
  await detail.getByRole('button', { name: 'Triage', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Close pull request' }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toContainText('o/r on GitHub');
  await expect(dialog).toContainText('pull request #7 (Add the thing)');
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  expect(await page.evaluate(() => localStorage.getItem('mockGithubTriage'))).toBeNull();

  // Confirming runs it, once.
  await detail.getByRole('button', { name: 'Triage', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Close pull request' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Close', exact: true }).click();
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('mockGithubTriage') ?? 'null')?.args?.triage?.action)).toBe('close');
});
