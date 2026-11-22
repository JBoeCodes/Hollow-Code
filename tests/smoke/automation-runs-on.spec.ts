import { expect, test, type Page } from './fixtures/test';

// Owner Mac (M4 follow-up, docs/reference/automation.md, Owner Mac): the
// editor's Runs on choice and the list's note. The mock (`src/mockApi.ts`
// `mockWorkflowList`) adds automations from `mockWorkflowsExtra`, owners from
// `mockWorkflowRunsOn`, and the Macs from `mockPeers` (`src/mockPeers.ts`).

const THIS_MAC = 'm-3f9c1a07e2b45d18';
const MINI = 'm-8d21c4be90f17a3e';

async function start(page: Page, seed: { peers: string | null; extra?: unknown[]; owners?: Record<string, string> }) {
  await page.addInitScript(s => {
    if (s.peers == null) localStorage.removeItem('mockPeers'); else localStorage.setItem('mockPeers', s.peers);
    localStorage.setItem('mockWorkflowsExtra', JSON.stringify(s.extra ?? []));
    localStorage.setItem('mockWorkflowRunsOn', JSON.stringify(s.owners ?? {}));
  }, seed);
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Automations', exact: true }).click();
}

async function edit(page: Page, title: string) {
  await page.getByRole('button', { name: `Open ${title}`, exact: true }).click();
  await page.locator('.workflow-output-actions').getByRole('button', { name: 'Automation options' }).click();
  await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
  const field = page.locator('.workflow-runs-on');
  await expect(field).toBeVisible();
  return field;
}

const calls = (page: Page, command: string) => page.evaluate(c => window.__mockApiCalls?.[c] ?? 0, command);

test('Runs on pins an automation to this Mac, approved at save', async ({ page }) => {
  await start(page, { peers: null });
  const field = await edit(page, 'Morning Brief');
  const select = field.getByRole('combobox', { name: 'Runs on' });
  await expect(select).toHaveAttribute('data-value', '');
  await expect(select).toContainText('Any Mac that holds it (until you pair another Mac)');
  await expect(field).toContainText('No owner: it runs on this Mac until another Mac also has this automation.');
  await expect(select).toHaveAccessibleDescription(/^No owner: it runs on this Mac/);
  await field.scrollIntoViewIfNeeded();
  await page.screenshot({ path: test.info().outputPath('runs-on-editor-alone.png') });

  await select.click();
  await page.getByRole('option', { name: /^This Mac/ }).click();
  await expect(select).toHaveAttribute('data-value', 'this');
  await expect(field).toContainText('Runs only on this Mac.');
  await expect(field).toContainText('Moving it to a Mac is a permission change: Save asks you to approve it');
  await expect(select).toHaveAccessibleDescription(/Runs only on this Mac\..*Moving it to a Mac is a permission change/);
  const approval = page.locator('.workflow-approval');
  await expect(approval).toContainText('Saving approves wider access:');
  await expect(approval).toContainText('Runs on: no owner → this Mac');
  await field.scrollIntoViewIfNeeded();
  await page.screenshot({ path: test.info().outputPath('runs-on-editor-picked.png') });

  await page.getByRole('button', { name: 'Save', exact: true }).click();
  const dialog = page.locator('.confirm-dialog');
  await expect(dialog).toContainText('Approve wider access');
  await expect(dialog).toContainText('Runs on: no owner → this Mac');
  await dialog.getByRole('button', { name: 'Save and approve' }).click();
  await expect.poll(() => calls(page, 'write_workflow')).toBe(1);
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('mockWorkflowRunsOn') ?? '{}'))).toEqual({ 'morning-brief': THIS_MAC });
  // Saved: the choice reads This Mac and nothing is left to approve.
  await expect(select).toHaveAttribute('data-value', 'this');
  await expect(approval).toContainText('No unsaved changes');
});

test('the list notes automations that run on another Mac or on none, and the editor says why', async ({ page }) => {
  await start(page, {
    peers: 'demo',
    owners: { 'morning-brief': THIS_MAC, 'nightly-gate': MINI, 'old-report': 'old-laptop' },
    extra: [
      { slug: 'daily-paper', title: 'Daily Paper', path: 'workflows/daily-paper.md' },
      { slug: 'nightly-gate', title: 'Nightly Gate', path: 'workflows/nightly-gate.md' },
      { slug: 'old-report', title: 'Old Report', path: 'workflows/old-report.md' },
    ],
  });
  const row = (title: string) => page.locator('.workflow-row').filter({ has: page.getByRole('button', { name: `Open ${title}`, exact: true }) });
  await expect(row('Nightly Gate').locator('.workflow-row-owner')).toHaveText('Runs on Mac mini');
  await expect(row('Daily Paper').locator('.workflow-row-owner')).toHaveText('Runs on no Mac: it has no owner and another Mac also has this automation. Open Edit and set Runs on.');
  await expect(row('Daily Paper').locator('.workflow-row-owner')).toHaveClass(/workflow-next-warn/);
  await expect(row('Old Report').locator('.workflow-row-owner')).toContainText('Runs on no Mac: “old-laptop” isn’t this Mac or a paired one.');
  await expect(row('Morning Brief').locator('.workflow-row-owner')).toHaveCount(0);
  await page.locator('.split-list').screenshot({ path: test.info().outputPath('runs-on-list.png') });

  const field = await edit(page, 'Daily Paper');
  await expect(field.getByRole('status')).toHaveText('Won’t run on any Mac until it has an owner: another Mac also has this automation. Pick This Mac or a paired Mac.');
  await field.scrollIntoViewIfNeeded();
  await field.screenshot({ path: test.info().outputPath('runs-on-editor-warn.png') });
  const select = field.getByRole('combobox', { name: 'Runs on' });
  await select.click();
  await expect(page.getByRole('option')).toHaveText([
    'Any Mac that holds it (until you pair another Mac)', 'This Mac (Alex’s MacBook Pro)', 'Mac mini',
  ]);
  await page.getByRole('option', { name: 'Mac mini', exact: true }).click();
  await expect(select).toHaveAttribute('data-value', MINI);
  await expect(field).toContainText('Runs only on Mac mini: this Mac’s schedule leaves it alone. Run now still runs it here.');
  await expect(page.locator('.workflow-approval')).toContainText('Runs on: no owner → Mac mini');
  await field.scrollIntoViewIfNeeded();
  await page.screenshot({ path: test.info().outputPath('runs-on-editor-shared.png') });
});

test('saving an owned automation’s other edits sends no runsOn', async ({ page }) => {
  await start(page, { peers: 'demo', owners: { 'morning-brief': MINI } });
  const field = await edit(page, 'Morning Brief');
  await expect(field.getByRole('combobox', { name: 'Runs on' })).toHaveAttribute('data-value', MINI);
  await expect(field.getByRole('status')).toHaveText('Runs only on Mac mini: this Mac’s schedule leaves it alone. Run now still runs it here.');
  await page.getByRole('textbox', { name: 'Automation name' }).fill('Morning Brief, renamed');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect.poll(() => calls(page, 'write_workflow')).toBe(1);
  const sent = await page.evaluate(() => (window as unknown as { __mockLastWorkflowInput?: Record<string, unknown> }).__mockLastWorkflowInput);
  expect(sent?.title).toBe('Morning Brief, renamed');
  expect(sent && 'runsOn' in sent).toBe(false);
});
