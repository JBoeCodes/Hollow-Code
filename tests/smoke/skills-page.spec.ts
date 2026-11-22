import { expect, test, type Page } from './fixtures/test';
import { choose, chooseNew } from './choose';

/** P1-57: Settings → Connectors → Skills names the workspace a skill goes to, edits your
 * own skills, and keeps create, edit and delete errors in place instead of a
 * 3-second footer flash. Both widths. */

async function openSkills(page: Page) {
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) await choose(page.getByRole('combobox', { name: 'Settings section' }), 'connectors');
  else await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Connectors', exact: true }).click();
  await page.getByRole('button', { name: 'Open Library, Skills', exact: true }).click(); // Skills moved to Library in Settings PR 4
}

async function createSkill(page: Page, name: string, description: string, body: string) {
  await page.getByRole('button', { name: 'New skill', exact: true }).click();
  await page.getByPlaceholder('my-skill').fill(name);
  await page.getByPlaceholder('When should an agent use this skill?').fill(description);
  await page.getByRole('textbox', { name: 'Instructions (Markdown)' }).fill(body);
  await page.getByRole('button', { name: 'Create skill', exact: true }).click();
}

test('Use in names the open workspace, or a new one when none is open', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await openSkills(page);
  const detail = page.getByRole('region', { name: 'Skill details' });
  await page.getByRole('list', { name: 'Skills' }).getByRole('button', { name: /^release-notes / }).click();
  await expect(detail.getByRole('button', { name: 'Use in a new workspace…', exact: true })).toBeVisible();
  await expect(detail.getByRole('button', { name: 'Use in workspace', exact: true })).toHaveCount(0);

  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Ship the landing page.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Alpha');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Alpha');
  await openSkills(page);
  await page.getByRole('list', { name: 'Skills' }).getByRole('button', { name: /^release-notes / }).click();
  await expect(detail.getByRole('button', { name: 'Use in Alpha', exact: true })).toBeVisible();
  await page.getByRole('list', { name: 'Skills' }).getByRole('button', { name: /^release-notes / }).click({ button: 'right' });
  await expect(page.getByRole('menuitem', { name: 'Use in Alpha', exact: true })).toBeVisible();
});

test('your own skill can be edited in place; a plugin skill shows no Edit', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await openSkills(page);
  await createSkill(page, 'tidy-notes', 'Tidy meeting notes', 'Fix the typo here.');
  const detail = page.getByRole('region', { name: 'Skill details' });
  await expect(detail.getByRole('heading', { name: '/tidy-notes' })).toBeVisible();

  await detail.getByRole('button', { name: 'Edit skill', exact: true }).click();
  await expect(detail.getByRole('heading', { name: 'Edit skill' })).toBeVisible();
  await expect(page.getByPlaceholder('my-skill')).toHaveValue('tidy-notes');
  await expect(page.getByPlaceholder('my-skill')).toHaveAttribute('readonly', '');
  await expect(page.getByPlaceholder('When should an agent use this skill?')).toHaveValue('Tidy meeting notes');
  await expect(page.getByRole('textbox', { name: 'Instructions (Markdown)' })).toHaveValue('Fix the typo here.');
  await page.getByRole('textbox', { name: 'Instructions (Markdown)' }).fill('The typo is fixed.');
  const calls = await page.evaluate(() => window.__mockApiCalls?.write_user_skill ?? 0);
  await page.getByRole('button', { name: 'Save changes', exact: true }).click();
  await expect(detail.getByRole('heading', { name: '/tidy-notes' })).toBeVisible();
  await detail.locator('summary', { hasText: 'Instructions' }).click();
  await expect(detail.locator('.skill-md')).toContainText('The typo is fixed.');
  expect(await page.evaluate(() => window.__mockApiCalls?.write_user_skill ?? 0)).toBe(calls + 1);

  // A plugin's skill is read-only: no Edit in the pane or the menu.
  const providerRow = page.getByRole('list', { name: 'Skills' }).getByRole('button', { name: /^pdf / });
  await providerRow.click();
  await expect(detail.getByRole('button', { name: 'Edit skill', exact: true })).toHaveCount(0);
  await providerRow.click({ button: 'right' });
  await expect(page.getByRole('menuitem', { name: 'Use in a new workspace…', exact: true })).toBeVisible();
  await expect(page.getByRole('menuitem', { name: 'Edit', exact: true })).toHaveCount(0);
});

test('a create refused for a name clash keeps the form open with the error in place', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await openSkills(page);
  await createSkill(page, 'release-notes', 'Clash', 'Body');
  const choice = page.getByRole('dialog', { name: 'Replace the skill /release-notes?' });
  await choice.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(choice).toBeHidden();
  const form = page.locator('.skill-form');
  await page.waitForTimeout(5000);
  await expect(form.getByRole('alert')).toContainText('/release-notes already exists');
  await expect(page.getByPlaceholder('my-skill')).toHaveValue('release-notes');

  await form.getByRole('button', { name: 'Dismiss', exact: true }).click();
  await expect(form.getByRole('alert')).toHaveCount(0);
  await expect(form).toBeVisible();

  // An error the save itself returns stays too.
  await page.evaluate(() => {
    (window as unknown as { __mockScenario?: unknown }).__mockScenario = { names: [], delayMs: {}, unavailable: { write_user_skill: 'write: Permission denied (os error 13)' } };
  });
  await page.getByPlaceholder('my-skill').fill('fresh-name');
  await page.getByRole('button', { name: 'Create skill', exact: true }).click();
  await expect(form.getByRole('alert')).toContainText('Couldn’t save this skill');
  await expect(form).toBeVisible();
});
