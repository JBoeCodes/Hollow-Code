import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from './fixtures/test';
import { choose, openAdvancedGroup } from './choose';
import { settled } from './settle';

/**
 * Overnight H20: Settings → Advanced → This Mac → Disk space's two cleanups, the person's
 * click for build.prune_install_targets and build.clean_worktrees. Each
 * previews, shows every path in the confirm dialog, deletes nothing on
 * Cancel or Esc, and on confirm deletes exactly the previewed list (its hash)
 * and measures again. Mock only (src/mockDiskCleanup.ts): nothing on disk is
 * touched; deletes are recorded in localStorage `mockDiskCleanups`.
 */

const narrow = (page: Page) => (page.viewportSize()?.width ?? 1000) <= 860;
const calls = (page: Page, command: string) => page.evaluate(c => window.__mockApiCalls?.[c] ?? 0, command);
const cleanups = (page: Page) => page.evaluate(() => JSON.parse(localStorage.getItem('mockDiskCleanups') ?? '[]'));
const folders = (page: Page) => page.getByRole('list', { name: "Hollow's largest folders" });
const PRUNE = 'Delete old install builds…';
const CLEAN = 'Remove finished worktrees…';

async function openDiskSpace(page: Page) {
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).waitFor();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  if (narrow(page)) await choose(page.getByRole('combobox', { name: 'Settings section' }), 'advanced');
  else await page.locator('.settings-index').getByRole('button', { name: 'Advanced', exact: true }).click();
  await openAdvancedGroup(page, 'this-mac'); // Disk space sits in This Mac (Settings PR 3)
  await expect(page.getByRole('button', { name: PRUNE })).toBeVisible();
  await expect(page.getByRole('button', { name: CLEAN })).toBeVisible();
}

test('Disk space: Delete old install builds… previews, Cancel deletes nothing, Delete deletes the list and measures again', async ({ page }) => {
  await openDiskSpace(page);
  await page.getByRole('button', { name: 'Check disk space' }).click();
  await expect(folders(page)).toContainText('hollow-install-target-old-lane');
  await expect(page.getByText('120 GB free · Hollow warns under 50 GB')).toBeVisible();

  const button = page.getByRole('button', { name: PRUNE });
  await button.click();
  const dialog = page.getByRole('dialog', { name: 'Delete 2 old install build folders?' });
  await expect(dialog).toBeVisible();
  // Every folder that goes, with its measured size; every one that stays, and why.
  await expect(dialog).toContainText('hollow-install-target-old-lane, last used 2026-10-03 · 12 GB');
  await expect(dialog).toContainText('hollow-install-target-spike, last used 2026-09-28 · 8.0 GB');
  await expect(dialog).toContainText('hollow-install-target-main (built the installed app');
  await expect(dialog).toContainText('Deletes 2 install build folders in ~/Library/Caches, about 20 GB as last measured.');
  await expect(dialog).toContainText('The apps in /Applications are not touched.');
  await expect(dialog).not.toContainText('Sizes unknown');
  // A delete: the dialog starts on Cancel, so a stray Enter deletes nothing.
  await expect(dialog.getByRole('button', { name: 'Cancel' })).toBeFocused();
  // Scan the dialog once its open fade ends: mid-fade, the red title reads 4.42:1.
  await settled(dialog);
  expect((await new AxeBuilder({ page }).include('.confirm-dialog').analyze()).violations).toEqual([]);

  await dialog.getByRole('button', { name: 'Cancel' }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByText('Nothing was deleted.', { exact: true })).toBeVisible();
  expect(await calls(page, 'prune_install_targets')).toBe(0);
  await expect(folders(page)).toContainText('hollow-install-target-old-lane');

  const measuredBefore = await calls(page, 'disk_guard');
  await button.click();
  await page.getByRole('dialog', { name: 'Delete 2 old install build folders?' }).getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(page.getByText('Deleted 2 install build folders.', { exact: true })).toBeVisible();
  // Exactly the previewed list: the mock got the preview's hash.
  expect(await cleanups(page)).toEqual([{ command: 'prune_install_targets', expectedHash: 'mock-targets-2' }]);
  // Measured again: the folders are gone and more space is free.
  await expect(page.getByText('140 GB free · Hollow warns under 50 GB')).toBeVisible();
  await expect(folders(page)).not.toContainText('hollow-install-target-old-lane');
  expect(await calls(page, 'disk_guard')).toBeGreaterThan(measuredBefore);
});

test('Disk space: Remove finished worktrees… by keyboard, Esc removes nothing, Remove removes the list and measures', async ({ page }) => {
  await openDiskSpace(page);
  const button = page.getByRole('button', { name: CLEAN });
  await button.focus();
  await page.keyboard.press('Enter');
  const dialog = page.getByRole('dialog', { name: 'Remove 1 finished worktree and 1 "~" folder?' });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText('/Users/mock/Hollow/Worktrees/Hollow/finished-lane [n15/finished-lane] (workspace build-finished-lane) — also deletes node_modules, dist');
  await expect(dialog).toContainText('Deletes 1 "~" folder of stray build output (1.5 GB):');
  await expect(dialog).toContainText('Kept: 1 worktree');
  await expect(dialog).toContainText('Not touched: 1 branch with commits no remote has.');
  // Not measured yet: no worktree size, and it says how to get one.
  await expect(dialog).toContainText('Sizes unknown: press Check disk space first.');

  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  await expect(page.getByText('Nothing was removed.', { exact: true })).toBeVisible();
  expect(await calls(page, 'clean_worktrees')).toBe(0);
  // Focus came back to the button that opened it.
  await expect(button).toBeFocused();

  await page.keyboard.press('Enter');
  await expect(dialog).toBeVisible();
  const remove = dialog.getByRole('button', { name: 'Remove', exact: true });
  await expect(dialog.getByRole('button', { name: 'Cancel' })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(remove).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(dialog).toBeHidden();
  await expect(page.getByText('Removed 2 worktrees and folders.', { exact: true })).toBeVisible();
  expect(await cleanups(page)).toEqual([{ command: 'clean_worktrees', expectedHash: 'mock-worktrees-2' }]);
  // Not measured before: the cleanup measures, and the worktree is gone.
  await expect(page.getByText('126 GB free · Hollow warns under 50 GB')).toBeVisible();
  await expect(folders(page)).not.toContainText('finished-lane');
});

test('Disk space: Settings left and reopened while a removal runs measures again when it ends (EH10)', async ({ page }) => {
  await openDiskSpace(page);
  // The removal is held until the test lets it go (no fixed delay).
  await page.evaluate(() => {
    let release: () => void = () => {};
    window.__mockHoldOnce = { command: 'clean_worktrees', release: new Promise<void>(r => { release = () => r(); }) };
    (window as unknown as { __releaseClean: () => void }).__releaseClean = () => release();
  });
  await page.getByRole('button', { name: CLEAN }).click();
  const dialog = page.getByRole('dialog', { name: 'Remove 1 finished worktree and 1 "~" folder?' });
  await dialog.getByRole('button', { name: 'Remove', exact: true }).click();
  await expect(dialog).toBeHidden();
  // Away and back while it runs: a new panel, buttons still waiting.
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: /^Projects/ }).click();
  await expect(page.getByRole('button', { name: CLEAN })).toHaveCount(0);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  if (narrow(page)) await choose(page.getByRole('combobox', { name: 'Settings section' }), 'advanced');
  else await page.locator('.settings-index').getByRole('button', { name: 'Advanced', exact: true }).click();
  await openAdvancedGroup(page, 'this-mac'); // Disk space sits in This Mac (Settings PR 3)
  await expect(page.getByRole('button', { name: CLEAN })).toHaveAttribute('aria-disabled', 'true');
  const measured = await calls(page, 'disk_guard');
  await page.evaluate(() => (window as unknown as { __releaseClean: () => void }).__releaseClean());
  // When it ends, this panel (not the one that started it) measures, once.
  await expect(page.getByText('Removed 2 worktrees and folders.', { exact: true })).toBeVisible();
  await expect(page.getByText('126 GB free · Hollow warns under 50 GB')).toBeVisible();
  await expect(folders(page)).not.toContainText('finished-lane');
  expect(await calls(page, 'disk_guard')).toBe(measured + 1);
});

test('Disk space: with nothing to delete, no dialog opens and nothing runs', async ({ page }) => {
  await page.addInitScript(() => { localStorage.setItem('mockDiskPruned', '1'); localStorage.setItem('mockDiskCleaned', '1'); });
  await openDiskSpace(page);
  await page.getByRole('button', { name: PRUNE }).click();
  await expect(page.getByText('Nothing to delete: every install build folder is kept. Nothing changed.')).toBeVisible();
  await page.getByRole('button', { name: CLEAN }).click();
  await expect(page.getByText('Nothing to remove: no worktree is merged, unchanged and pushed, and there is no "~" folder. Nothing changed.')).toBeVisible();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(await calls(page, 'prune_install_targets')).toBe(0);
  expect(await calls(page, 'clean_worktrees')).toBe(0);
});

test('Disk space: a list that changed since the dialog deletes nothing and says so in plain words', async ({ page }) => {
  await openDiskSpace(page);
  await page.getByRole('button', { name: PRUNE }).click();
  const dialog = page.getByRole('dialog', { name: 'Delete 2 old install build folders?' });
  await expect(dialog).toBeVisible();
  // Something else deletes them while the dialog is open: the preview's hash
  // no longer matches, and the backend refuses (mockDiskCleanup.ts).
  await page.evaluate(() => localStorage.setItem('mockDiskPruned', '1'));
  await dialog.getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByRole('alert').filter({
    hasText: 'The list changed since you saw it. Nothing was deleted; press the button again to see the new list.',
  })).toBeVisible();
  // It sent the previewed list's hash; nothing reports a delete.
  expect(await cleanups(page)).toEqual([{ command: 'prune_install_targets', expectedHash: 'mock-targets-2' }]);
  await expect(page.getByText(/^Deleted \d/)).toHaveCount(0);
  // It measures again after any delete attempt (the buttons wait for it).
  await expect(page.getByText('140 GB free · Hollow warns under 50 GB')).toBeVisible();
  // Pressing again shows the new list: nothing left, and the alert clears.
  await page.getByRole('button', { name: PRUNE }).click();
  await expect(page.getByText('Nothing to delete: every install build folder is kept. Nothing changed.')).toBeVisible();
  await expect(page.getByRole('alert').filter({ hasText: 'The list changed since you saw it.' })).toHaveCount(0);
});

test("Disk space: when running programs can't be read, Remove finished worktrees… says so and opens nothing", async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockDiskInUseUnknown', '1'));
  await openDiskSpace(page);
  await page.getByRole('button', { name: CLEAN }).click();
  await expect(page.getByText("Hollow couldn't read which programs are running, so no worktree is removed. Nothing changed.", { exact: true })).toBeVisible();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(await calls(page, 'clean_worktrees')).toBe(0);
});

// Repo features R7b: Hollow's databases (read-only) and Disk space's third
// cleanup, Remove old indexes… (mockDiskCleanup.ts `mockStaleIndexes`).
const INDEXES = 'Remove old indexes…';
const databases = (page: Page) => page.getByRole('list', { name: "Hollow's databases" });
// Hollow's databases sit in Advanced's closed Diagnostics group (Settings PR 3).
const openAdvanced = (page: Page) => openAdvancedGroup(page, 'diagnostics');

test('Hollow’s databases: Measure lists the four kinds with owners, Check integrity counts rows and changes nothing', async ({ page }) => {
  await openDiskSpace(page);
  await openAdvanced(page);
  await expect(page.getByRole('heading', { name: "Hollow's databases" })).toBeVisible();
  await page.getByRole('button', { name: 'Measure', exact: true }).click();
  const list = databases(page);
  await expect(list).toContainText('Conversations · this vault · 439 MB');
  await expect(list).toContainText("Conversations · Second-brain: another vault's conversations · kept");
  await expect(list).toContainText('Search index · this vault · 86 MB + 16 MB WAL · version 2');
  await expect(list).toContainText("Meaning-search index · Second-brain: another vault's");
  await expect(list).toContainText('Companion ledger · Hollow');
  // Measuring runs no check and no counts.
  await expect(list).not.toContainText('Integrity');
  // No delete button here: the last line points to Disk space.
  await expect(page.getByText('3 old indexes of other vaults use 270 MB: Disk space → Remove old indexes… removes them.')).toBeVisible();

  await page.getByRole('button', { name: 'Check integrity' }).click();
  await expect(page.getByText(/Every file checked ok\.$/)).toBeVisible();
  await expect(list).toContainText('Integrity ok · sessions 3,120 rows · turns 41,000 rows');
  expect(await calls(page, 'remove_stale_indexes')).toBe(0);
  expect(await cleanups(page)).toEqual([]);
});

test('Disk space: Remove old indexes… lists other vaults’ indexes, Cancel removes nothing, Remove removes them and measures again', async ({ page }) => {
  await openDiskSpace(page);
  const button = page.getByRole('button', { name: INDEXES });
  await button.click();
  const dialog = page.getByRole('dialog', { name: 'Remove 3 old indexes?' });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText('Removes 3 indexes of other vaults from /Users/mock/Library/Application Support/Hollow, 270 MB in all.');
  await expect(dialog).toContainText('• Second-brain-20f08a1b2c3d4e5f.db (Second-brain, search index, last changed 2026-09-30) · 120 MB');
  await expect(dialog).toContainText('• Second-brain-20f08a1b2c3d4e5f.db (Second-brain, meaning-search index, last changed 2026-09-30) · 90 MB');
  await expect(dialog).toContainText("(Second-brain, conversations): another vault's conversations: never removed");
  await expect(dialog).toContainText("This vault's files and every Sessions database are never removed.");
  await expect(dialog.getByRole('button', { name: 'Cancel' })).toBeFocused();

  await dialog.getByRole('button', { name: 'Cancel' }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByText('Nothing was removed.', { exact: true })).toBeVisible();
  expect(await calls(page, 'remove_stale_indexes')).toBe(0);

  const measuredBefore = await calls(page, 'disk_guard');
  await button.click();
  await page.getByRole('dialog', { name: 'Remove 3 old indexes?' }).getByRole('button', { name: 'Remove', exact: true }).click();
  await expect(page.getByText('Removed 3 old indexes, 270 MB.', { exact: true })).toBeVisible();
  expect(await cleanups(page)).toEqual([{ command: 'remove_stale_indexes', expectedHash: 'mock-indexes-3' }]);
  // Measured again, and Hollow's databases no longer lists them.
  await expect.poll(() => calls(page, 'disk_guard')).toBeGreaterThan(measuredBefore);
  // Hollow's databases, including Measure, sit in the closed Diagnostics group.
  await openAdvanced(page);
  await page.getByRole('button', { name: 'Measure', exact: true }).click();
  await expect(databases(page)).not.toContainText('Vault-3a14c0ffee123456');
  await expect(page.getByText(/Disk space → Remove old indexes…/)).toHaveCount(0);
  // Pressing again: nothing left, no dialog.
  await button.click();
  await expect(page.getByText("Nothing to remove: no other vault's index has gone unused for an hour. Nothing changed.")).toBeVisible();
  await expect(page.getByRole('dialog')).toHaveCount(0);
});
