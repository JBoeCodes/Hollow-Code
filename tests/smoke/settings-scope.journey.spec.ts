import { expect, test, type Page } from './fixtures/test';
import { SETTINGS_SECTION_BY_ID, saveNote } from '../../src/settingsSections';

// Below 860 px the index keeps only its search (P1-21), so the section
// picker decides the path.
async function chooseSection(page: Page, label: string) {
  const picker = page.getByRole('combobox', { name: 'Settings section', exact: true });
  if (!(await picker.isVisible())) await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: label, exact: true }).click();
  else {
    await picker.click();
    await page.getByRole('option', { name: label, exact: true }).click();
  }
}

test('save note stays visible and fits at desktop, minimum and Largest text', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await page.getByRole('navigation', { name: 'Shared tools' }).getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page.locator('.settings-section').first()).toBeVisible();
  for (const id of ['models', 'general', 'background', 'advanced', 'personalization', 'sources', 'devices']) {
    const meta = SETTINGS_SECTION_BY_ID[id];
    await chooseSection(page, meta.label);
    const scope = page.locator(`#settings-${id} .settings-section-scope`);
    await expect(scope).toHaveAccessibleName(`What ${meta.label} changes`);
    await expect(scope).toHaveText(saveNote(meta.scope));
    await expect(scope).toBeVisible();
    await expect(scope.locator('details')).toHaveCount(0);
    const bounds = await scope.evaluate(root => {
      const viewport = root.closest('.settings-body')!.getBoundingClientRect();
      const r = root.getBoundingClientRect();
      return {
        overflow: document.scrollingElement!.scrollWidth > innerWidth,
        fits: r.left >= viewport.left - 1 && r.right <= viewport.right + 1,
      };
    });
    expect(bounds).toEqual({ overflow: false, fits: true });
  }
  await chooseSection(page, 'General');
  await expect(page.locator('#settings-general .settings-section-scope')).toHaveText('Saves at once. Forms need Save.');
});
