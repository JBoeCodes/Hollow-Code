import { expect, test, type Page } from './fixtures/test';
import { showSettingsPart } from './settingsPart';

/** Settings → Keyboard shortcuts (2026-10-01): one switch turns every Hollow
 * shortcut off, and each command's key can be recorded, removed or reset.
 * The `?mock=1` IPC keeps the saved settings in memory, as the backend's
 * `shortcuts.json` does; the native menu and system-wide keys are the
 * backend's half and are checked by its own tests. */

test.skip(({ viewport }) => (viewport?.width ?? 1000) <= 860, 'the desktop rail and palette');

async function openShortcuts(page: Page) {
  await page.goto('/?mock=1');
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  // A section of its own since Settings PR 3; its chip says On or Off.
  await page.locator('.settings-index').getByRole('button', { name: 'Keyboard shortcuts', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Keyboard shortcuts', exact: true })).toBeVisible();
  await showSettingsPart(page, 'shortcuts'); // scrolls the section into view
}

const row = (page: Page, id: string) => page.locator(`[data-shortcut="${id}"]`);
const blur = (page: Page) => page.locator('.settings-section-title').click();

test('the switch turns every shortcut off and back on', async ({ page }) => {
  await openShortcuts(page);
  const palette = page.locator('.palette-input');

  await blur(page);
  await page.keyboard.press('Meta+k');
  await expect(palette).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(palette).toBeHidden();

  await page.getByRole('switch', { name: 'Use keyboard shortcuts' }).click();
  await expect(page.locator('#settings-shortcuts .settings-section-status')).toHaveText('Off');
  await blur(page);
  await page.keyboard.press('Meta+k');
  await page.keyboard.press('Meta+b');
  await expect(palette).toBeHidden();
  await expect(page.locator('.work-rail')).not.toHaveClass(/collapsed/);
  // The keymap card's bare ? is off too.
  await page.keyboard.press('Shift+Slash');
  await expect(page.locator('.keymap-card')).toBeHidden();

  await page.getByRole('switch', { name: 'Use keyboard shortcuts' }).click();
  await expect(page.locator('#settings-shortcuts .settings-section-status')).toHaveText('On');
  await blur(page);
  await page.keyboard.press('Meta+k');
  await expect(palette).toBeVisible();
});

test('a recorded key replaces the default, and Reset puts it back', async ({ page }) => {
  await openShortcuts(page);
  const sidebar = row(page, 'toggle-sidebar');
  await expect(sidebar.locator('kbd')).toHaveText('⌘B');

  await sidebar.getByRole('button', { name: 'Change the key for Toggle sidebar' }).click();
  await expect(sidebar).toContainText('Press the new keys');
  // A key without ⌘ is refused; the row keeps listening.
  await page.keyboard.press('j');
  await expect(sidebar).toContainText('Hold ⌘');
  // A key macOS owns is refused.
  await page.keyboard.press('Meta+q');
  await expect(sidebar).toContainText("is macOS's Quit");
  await page.keyboard.press('Meta+Shift+j');
  // macOS's order, ⌃⌥⇧⌘ (P1-49).
  await expect(sidebar.locator('kbd')).toHaveText('⇧⌘J');

  const rail = page.locator('.work-rail');
  await blur(page);
  await page.keyboard.press('Meta+b');
  await expect(rail).not.toHaveClass(/collapsed/);
  await page.keyboard.press('Meta+Shift+j');
  await expect(rail).toHaveClass(/collapsed/);
  await page.keyboard.press('Meta+Shift+j');
  await expect(rail).not.toHaveClass(/collapsed/);

  await sidebar.getByRole('button', { name: /^Reset Toggle sidebar/ }).click();
  await expect(sidebar.locator('kbd')).toHaveText('⌘B');
  await expect(sidebar.getByRole('button', { name: /^Reset Toggle sidebar/ })).toHaveCount(0);
});

test('taking another command’s key leaves that command with none', async ({ page }) => {
  await openShortcuts(page);
  const task = row(page, 'new-task');
  await expect(task).toContainText('None');
  await task.getByRole('button', { name: 'Change the key for New task' }).click();
  await page.keyboard.press('Meta+k');
  await expect(task.locator('kbd')).toHaveText('⌘K');
  await expect(task).toContainText('Taken from Command palette, which now has no key.');
  await expect(row(page, 'search')).toContainText('None');
  await expect(page.locator('.palette-input')).toBeHidden();

  // Esc cancels a recording without changing anything.
  await row(page, 'new-note').getByRole('button', { name: 'Change the key for New note' }).click();
  await page.keyboard.press('Escape');
  await expect(row(page, 'new-note').locator('kbd')).toHaveText('⇧⌘N');

  await page.getByRole('button', { name: 'Reset all to defaults' }).click();
  await page.getByRole('dialog', { name: 'Reset every shortcut to its default?' }).getByRole('button', { name: 'Reset all', exact: true }).click();
  await expect(row(page, 'search').locator('kbd')).toHaveText('⌘K');
  await expect(task).toContainText('None');
});

test('Remove clears one key, and the keymap card shows the keys in effect', async ({ page }) => {
  await openShortcuts(page);
  await row(page, 'new-note').getByRole('button', { name: 'Remove the key for New note' }).click();
  await expect(row(page, 'new-note')).toContainText('None');
  await blur(page);
  await page.keyboard.press('Shift+Slash');
  const card = page.locator('.keymap-card');
  await expect(card).toBeVisible();
  await expect(card).toContainText('toggle sidebar');
  await expect(card).not.toContainText('new note');
  await card.getByRole('button', { name: 'Change shortcuts…' }).click();
  await expect(card).toBeHidden();
  await expect(page.getByRole('heading', { name: 'Keyboard shortcuts', exact: true })).toBeVisible();
});

// P1-49: a ⌥ chord saved for Jump used to be dropped by the window; a
// system-wide key needs all of ⌘⌥⇧; a go-to key can be removed.
test('a ⌥ chord runs, a system-wide key asks for ⌥⇧⌘, and Go to keys can be removed', async ({ page }) => {
  await openShortcuts(page);
  const jump = row(page, 'jump');
  await jump.getByRole('button', { name: 'Change the key for Jump to a workspace or chat' }).click();
  // Selected text's ⌥⌘T is no command's: the window leaves it to the selection.
  await page.keyboard.press('Meta+Alt+t');
  await expect(jump).toContainText('make a task from the selection');
  await page.keyboard.press('Meta+Alt+j');
  await expect(jump.locator('kbd')).toHaveText('⌥⌘J');
  await blur(page);
  await page.keyboard.press('Meta+Alt+j');
  const palette = page.locator('[aria-label="Command palette"]');
  await expect(palette.getByRole('group', { name: 'Search in' }).getByRole('button', { name: 'Workspaces' })).toHaveAttribute('aria-pressed', 'true');
  await page.keyboard.press('Escape');
  await expect(palette).toHaveCount(0);

  const quick = row(page, 'quick-capture');
  await quick.getByRole('button', { name: 'Change the key for Quick capture task' }).click();
  await page.keyboard.press('Meta+Shift+k');
  await expect(quick).toContainText('needs ⌥⇧⌘ and a key');
  await page.keyboard.press('Escape');
  await expect(quick.locator('kbd')).toHaveText('⌥⇧⌘T');

  // Go to keys is its own tab (Settings PR 5).
  await page.getByRole('tab', { name: 'Go to keys', exact: true }).click();
  const tasks = row(page, 'go:tasks');
  await tasks.getByRole('button', { name: 'Remove the key after g for Tasks' }).click();
  await expect(tasks.getByRole('textbox', { name: 'Key after g for Tasks' })).toHaveValue('');
  await expect(tasks.getByRole('button', { name: /^Reset Tasks to g t/ })).toBeVisible();
});
