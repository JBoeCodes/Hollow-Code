import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

/**
 * Event triggers (COO rebuild F2). An automation's When is a Schedule or an
 * Event (a new issue in team AI, a merged PR in a connected repo, a
 * recording or meeting ending), saved as one `triggers:` token; saving turns
 * that trigger on through the person's own `set_event_trigger`. A tracked
 * project's menu has "Wake the lead for new issues", off until clicked. The
 * `?mock=1` IPC keeps the triggers in localStorage `mockEventTriggers`.
 */

const cwd = (id: string) => `/Users/fixture/vault/workspaces/${id}`;
const workspace = (id: string, title: string) =>
  ({ id, title, status: 'active', path: `workspaces/${id}`, cwd: cwd(id), modified: '2026-11-10T12:00:00Z' });
const saved = (page: Page) => page.evaluate(() => JSON.parse(localStorage.getItem('mockEventTriggers') ?? 'null'));
const nav = (page: Page, name: string) => page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: new RegExp(`^${name}`) });

async function seed(page: Page) {
  await page.addInitScript(([list]) => {
    if (sessionStorage.getItem('triggersSeeded')) return;
    sessionStorage.setItem('triggersSeeded', '1');
    localStorage.setItem('mockWorkspaces', JSON.stringify(list));
    localStorage.setItem('hollow.railWorkspaces', JSON.stringify(['alpha-1', 'beta-2']));
  }, [[workspace('alpha-1', 'Alpha'), workspace('beta-2', 'Beta')]] as const);
  await page.goto('/?mock=1');
}

test('a new automation on an event saves the event token and turns that trigger on', async ({ page }) => {
  await seed(page);
  await nav(page, 'Automations').click();
  await page.getByRole('button', { name: 'New automation', exact: true }).click();
  await choose(page.getByRole('combobox', { name: 'Automation workspace' }), 'beta-2');
  expect(await saved(page)).toBeNull();

  const when = page.getByRole('group', { name: 'When' });
  await expect(when.getByRole('button', { name: 'Schedule', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await when.getByRole('button', { name: 'Event', exact: true }).click();
  await expect(when.getByRole('button', { name: 'Event', exact: true })).toHaveAttribute('aria-pressed', 'true');
  // The schedule picker gives way to the event's own choices.
  await expect(page.getByRole('combobox', { name: 'Event', exact: true })).toBeVisible();
  await expect(page.getByRole('combobox', { name: 'Event project' })).toBeVisible();
  await expect(page.getByRole('status').filter({ hasText: 'events from before never start it' })).toBeVisible();

  await page.getByPlaceholder('e.g. Weekly evidence review').fill('Triage new issues');
  await page.locator('.workflow-new textarea').fill('Read {{trigger_reason}} and sort it.');
  await page.evaluate(async () => {
    const path = '/src/api.ts'; const { api } = await import(/* @vite-ignore */ path);
    const original = api.writeWorkflow;
    api.writeWorkflow = async (input: { schedule: string; triggers?: string[] }, slug: string | null) => {
      localStorage.setItem('triggersTestInput', JSON.stringify({ schedule: input.schedule, triggers: input.triggers }));
      return original(input, slug);
    };
  });
  await page.getByRole('button', { name: 'Create automation', exact: true }).click();
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('triggersTestInput') ?? 'null')))
    .toEqual({ schedule: 'manual', triggers: ['tracker-issue:beta-2'] });
  await expect.poll(async () => (await saved(page))?.kinds?.['tracker-issue']?.on).toBe(true);
  const s = await saved(page);
  expect(Object.keys(s.workspaces['tracker-issue'])).toEqual(['beta-2']);
  expect(s.leadWakes).toEqual({});
  expect((await page.evaluate(() => window.__mockApiMisses ?? [])).filter(c => /event_trigger/.test(c))).toEqual([]);
});

test('switching back to Schedule drops the event; a recording event names no project', async ({ page }) => {
  await seed(page);
  await nav(page, 'Automations').click();
  await page.getByRole('button', { name: 'New automation', exact: true }).click();
  await choose(page.getByRole('combobox', { name: 'Automation workspace' }), 'alpha-1');
  const when = page.getByRole('group', { name: 'When' });
  await when.getByRole('button', { name: 'Event', exact: true }).click();
  await choose(page.getByRole('combobox', { name: 'Event', exact: true }), 'recording-ended');
  await expect(page.getByRole('combobox', { name: 'Event project' })).toHaveCount(0);
  await when.getByRole('button', { name: 'Schedule', exact: true }).click();
  await expect(page.getByRole('combobox', { name: 'Event', exact: true })).toHaveCount(0);
  await expect(page.getByRole('combobox', { name: 'Trigger kind' })).toBeVisible();
  expect(await saved(page)).toBeNull();
});

test('a tracked project\'s menu turns the lead wake on and off', async ({ page }, info) => {
  test.skip(info.project.name.includes('narrow'), 'The rail tree collapses to icons on narrow screens');
  await seed(page);
  const rail = page.getByRole('navigation', { name: 'Project navigation' });
  const head = rail.locator('.work-tree').filter({ has: page.locator('.work-workspace-title', { hasText: /^Beta$/ }) }).locator('.work-tree-head');
  const menu = page.locator('.ctx-menu');
  // The menu asks for the state as it opens; the next one shows it.
  await head.click({ button: 'right' });
  await page.keyboard.press('Escape');
  await head.click({ button: 'right' });
  await menu.getByRole('menuitem', { name: 'Wake the lead for new issues (off)', exact: true }).click();
  await expect(page.getByText('The lead wakes for new issues in team AI from now on.')).toBeVisible();
  const s = await saved(page);
  expect(Object.keys(s.leadWakes)).toEqual(['beta-2']);
  expect(s.kinds['tracker-issue'].on).toBe(true);
  await head.click({ button: 'right' });
  await menu.getByRole('menuitem', { name: 'Wake the lead for new issues (on)', exact: true }).click();
  await expect(page.getByText('The lead no longer wakes for new issues.')).toBeVisible();
  expect((await saved(page)).leadWakes).toEqual({});
});
