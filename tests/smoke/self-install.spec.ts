import { test, expect, type Page } from './fixtures/test';
import { choose, chooseNew, openChanges, openSettingsSection } from './choose';

// Build app from Hollow (build programme PR B5) and, since row U10, the
// finished build in the Update button. Everything is mocked
// (src/mockBuild.ts, src/mockApi.ts): no build or install ever runs.

async function openHollowChanges(page: Page, repo = '/Users/fixture/Projects/hollow') {
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Ship a fix to Hollow.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Fix Hollow');
  await page.locator('.project-create-options > summary').click();
  await page.getByRole('radio', { name: 'Repository worktree' }).click();
  await choose(page.getByRole('combobox', { name: 'Repository' }), repo);
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-branch')).toHaveText('hollow/fix-hollow');
  // Changes in the Preview pane, from the top bar's branch (COO rebuild L4).
  await openChanges(page, 'hollow/fix-hollow');
  await expect(page.locator('.build-changes .bc-files')).toBeVisible();
}

const log = (page: Page) => page.evaluate(() => (JSON.parse(localStorage.getItem('mockBuildLog') ?? '[]') as { command: string }[]).map(e => e.command));

test('quit offers keep running and Cancel without stopping a run', async ({ page }) => {
  await page.goto('/?mock=1');
  await expect(page.getByRole('navigation', { name: 'Shared tools' })).toBeVisible();
  // SelfInstallNotice is a lazy chunk, so `quit-requested` can fire before it
  // subscribes (browser fix b17: the 11-17 suite on dce8428ec lost the event;
  // the app itself recovers through its `pending_quit_request` read on mount).
  // The mock bus returns how many listeners got it: emit until one did.
  await expect.poll(() => page.evaluate(() => (window as unknown as { __mockEmit: (name: string, payload: unknown) => number }).__mockEmit('quit-requested', { id: 'quit-1', names: ['Polish lane B', 'Daily recap'] }))).toBeGreaterThan(0);
  const dialog = page.getByRole('dialog', { name: 'Quit and stop running work?' });
  await expect(dialog).toContainText('Polish lane B');
  await expect(dialog.getByRole('button', { name: 'Cancel', exact: true })).toBeFocused();
  await dialog.getByRole('button', { name: 'Keep running in the menu bar' }).focus();
  await page.keyboard.press('Enter');
  await expect(dialog).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem('mockQuitChoice'))).toBe('tray');
});

test('a lost quit dialog returns on reload and Cancel leaves work running', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.evaluate(() => localStorage.setItem('mockPendingQuit', JSON.stringify({ id: 'quit-reload', names: ['Polish lane B'] })));
  await page.reload();
  const dialog = page.getByRole('dialog', { name: 'Quit and stop running work?' });
  await expect(dialog).toContainText('Polish lane B');
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem('mockQuitChoice'))).toBe('cancel');
});

test('new work causes another quit confirmation before quitting', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.evaluate(() => {
    localStorage.setItem('mockPendingQuit', JSON.stringify({ id: 'quit-recheck', names: ['Polish lane B'] }));
    localStorage.setItem('mockQuitNewWork', '1');
  });
  await page.reload();
  const dialog = page.getByRole('dialog', { name: 'Quit and stop running work?' });
  await dialog.getByRole('button', { name: 'Quit and stop 1 run', exact: true }).click();
  await expect(dialog).toContainText('New automation');
  expect(await page.evaluate(() => localStorage.getItem('mockQuitChoice'))).toBeNull();
  await dialog.getByRole('button', { name: 'Quit and stop 2 runs', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem('mockQuitChoice'))).toBe('quit');
});

test('unknown running work has no invented count and permits explicit Quit anyway', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.evaluate(() => localStorage.setItem('mockPendingQuit', JSON.stringify({ id: 'quit-unknown', names: ['Running work could not be checked'], uncertain: true })));
  await page.reload();
  const dialog = page.getByRole('dialog', { name: 'Quit and stop running work?' });
  await expect(dialog).toContainText('Running work could not be checked');
  await expect(dialog).not.toContainText('1 run');
  await dialog.getByRole('button', { name: 'Quit anyway', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem('mockQuitChoice'))).toBe('quit');
});

test('only a Hollow worktree offers Build app', async ({ page }) => {
  await page.goto('/?mock=1');
  await openHollowChanges(page, '/Users/fixture/Projects/site');
  await expect(page.locator('.bc-app')).toHaveCount(0);
});

test('build, stop, then build: the finished build waits in the Update button', async ({ page }) => {
  await page.goto('/?mock=1');
  await openHollowChanges(page);
  const app = page.locator('.bc-app');
  await app.getByRole('button', { name: 'Build app' }).click();
  await expect(page.locator('.confirm-dialog')).toContainText('two build slots');
  await expect(page.locator('.confirm-dialog')).toContainText('Update button');
  await page.locator('.confirm-dialog').getByRole('button', { name: 'Build app' }).click();
  await expect(app).toContainText('Building the app from hollow/fix-hollow');
  await expect(app.locator('.bc-log pre')).toContainText('npm run tauri:bundle');
  await app.getByRole('button', { name: 'Stop build' }).click();
  await expect(app).toContainText('The build failed: Stopped');

  await page.evaluate(() => localStorage.setItem('mockAppBuildInstant', '1'));
  await app.getByRole('button', { name: 'Build again' }).click();
  await page.locator('.confirm-dialog').getByRole('button', { name: 'Build app' }).click();
  await expect(app).toContainText('Built from hollow/fix-hollow (b5b5b5b5b5b5).');
  await app.locator('.bc-brings summary').click();
  await expect(app.locator('.bc-brings')).toContainText('#771 Name the installed release');
  await expect(app.locator('.bc-brings')).toContainText('Not checked by hand in the native app.');
  await expect(app).toContainText('This build drops 2 commits');
  // Row U10: no install card here. The build is offered in the top bar's
  // Update button ("Local build of <branch>"), which installs it.
  await expect(app.getByRole('button', { name: /Install/ })).toHaveCount(0);
  await app.getByRole('button', { name: 'Show in Update button' }).click();
  await expect(app).toContainText('It waits in the Update button in the top bar as Local build of hollow/fix-hollow');
  await page.getByRole('button', { name: 'Update', exact: true }).click();
  const popover = page.getByRole('dialog', { name: /Local build of hollow\/fix-hollow/ });
  await expect(popover).toContainText('Version 0.6.100, commit b5b5b5b5b5b5');
  await expect(popover.getByRole('button', { name: 'Restart to update' })).toBeVisible();
  expect(await log(page)).toEqual(['start_app_build', 'cancel_app_build', 'start_app_build', 'update_offer_local_build']);
});

test('after a relaunch the install result shows until dismissed', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.evaluate(() => localStorage.setItem('mockSelfInstallResult', JSON.stringify({ id: 'x', ok: false, startedAt: '', finishedAt: '', commit: 'abc', branch: 'hollow/fix', log: '/Users/fixture/Library/Logs/Hollow/self-install/x.log', message: 'The installer exited 1', state: 'unknown' })));
  await page.reload();
  const notice = page.locator('.snackbar.is-sticky');
  await expect(notice).toContainText('Install failed');
  // A bare exit code is not evidence the previous app is still there (slice 6).
  await expect(notice).toContainText('could not confirm which app is installed');
  await expect(notice).not.toContainText('previous app is still installed');
  await expect(notice).toContainText('x.log');
  await notice.getByRole('button', { name: 'Dismiss' }).click();
  await expect(notice).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem('mockSelfInstallAcked'))).toBe('1');
  await page.evaluate(() => localStorage.setItem('mockSelfInstallResult', JSON.stringify({ id: 'y', ok: true, startedAt: '', finishedAt: '', commit: 'def456', branch: 'hollow/fix', log: '', message: '' })));
  await page.reload();
  await expect(page.locator('.snackbar.is-sticky')).toContainText('Installed the build from hollow/fix (def456).');
});
