/**
 * Journey J12 (plan completion L5, UI review journey 2 "Turn a meeting into
 * accountable work", matrix V7j): with visible controls only, find a
 * recording, read its summary, link it to a workspace, create the chosen
 * tasks from its action items, survive a partial failure (the made task is
 * reported, the failed one stays chosen, and creating again makes no twin),
 * and see the missing-sign-in state with its route to fix it. A required
 * spec: desktop, native minimum window and Largest text; no skips
 * (scripts/check-required-specs.mjs).
 */
import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';
import { openLibraryView } from './libraryView';

async function rail(page: Page, name: string) {
  const target = page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name, exact: true });
  const shown = await target.waitFor({ state: 'visible', timeout: 2_000 }).then(() => true, () => false);
  if (!shown) await page.getByRole('button', { name: 'Show sidebar', exact: true }).click();
  await target.click();
}

async function recordings(page: Page) {
  await rail(page, 'Library');
  await openLibraryView(page, 'Recordings');
}

test('click-only: recording → summary → linked workspace → tasks, through a partial failure and a retry', async ({ page }) => {
  test.setTimeout(60_000);
  await page.goto('/?mock=1&demo=1');
  await recordings(page);
  await page.getByRole('option', { name: /Weekly ops sync/ }).click();
  const reader = page.getByRole('region', { name: 'Recording' });
  await expect(reader.getByRole('region', { name: 'Summary' })).toContainText('Action items');

  // File it to a workspace first, so the tasks carry that project.
  await choose(reader.getByRole('combobox', { name: 'Link to a workspace' }), { label: 'Q4 hiring plan' });
  await expect(reader.getByRole('combobox', { name: 'Link to a workspace' })).toContainText('Q4 hiring plan');

  const side = page.getByRole('complementary', { name: 'Proposed tasks' });
  await expect(side).toContainText('Nothing is created until you choose.');
  const boxes = side.getByRole('checkbox');
  await expect(boxes).toHaveCount(2);
  const second = (await side.locator('li strong').nth(1).innerText()).trim();
  await boxes.first().check();
  await boxes.nth(1).check();
  // The second create fails once (a busy vault).
  await page.evaluate(title => { (window as unknown as { __mockFailCreateTaskOnce?: string }).__mockFailCreateTaskOnce = title; }, second);
  await side.getByRole('button', { name: /Create 2 tasks/ }).click();
  // The one that was made is reported; the failed one is named and stays chosen.
  await expect(side.getByRole('status')).toContainText('1 created');
  await expect(page.getByText(`“${second}” wasn’t created`)).toBeVisible();
  await expect(boxes.first()).not.toBeChecked();
  await expect(boxes.nth(1)).toBeChecked();
  await side.getByRole('button', { name: /Create 1 task/ }).click();
  await expect(side.getByRole('status')).toContainText('1 created');

  // Choosing the first again makes no twin.
  await boxes.first().check();
  await side.getByRole('button', { name: /Create 1 task/ }).click();
  await expect(side.getByRole('status')).toContainText('0 created, 1 already existed (not duplicated)');

  // Provenance and filing: each create names the recording and the linked project, once.
  const made = await page.evaluate(() => ((window as unknown as { __mockCreatedTasks?: { title?: string; description?: string; project?: string; idempotencyKey?: string }[] }).__mockCreatedTasks ?? []));
  const keys = new Set(made.map(t => t.idempotencyKey));
  expect(keys.size, 'two distinct tasks').toBe(2);
  for (const t of made) {
    expect(t.description).toContain('From the recording “Weekly ops sync”');
    expect(t.project).toBe('Q4 hiring plan');
  }
});

test('click-only: without a Plaud sign-in, Recordings says so and opens Settings', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockPlaudOff', '1'));
  await page.goto('/?mock=1&demo=1');
  await recordings(page);
  const state = page.getByText('Plaud isn’t connected');
  await expect(state).toBeVisible();
  await expect(page.getByText(/Sign in to Plaud for Recordings in Settings → Connectors/)).toBeVisible();
  // The button goes where the sentence says (it opened Settings' first section before),
  // and is reachable in a 720 × 480 window (the reader covered it before). Since P1-20 it
  // lands on the Recordings in Hollow sign-in's Connect.
  await page.getByRole('button', { name: 'Sign in to Plaud for Recordings', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Connectors', level: 2 }).first()).toBeVisible();
  // Scoped to the row: other rows in Connectors have a Connect of their own.
  const plaudRow = page.locator('.settings-row').filter({ has: page.locator('.settings-row-k', { hasText: /^Recordings in Hollow$/ }) });
  await expect(plaudRow.getByRole('button', { name: 'Connect', exact: true })).toBeFocused();
});
