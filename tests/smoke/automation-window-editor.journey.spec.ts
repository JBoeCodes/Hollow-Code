import { expect, test } from './fixtures/test';
import { choose } from './choose';

async function editor(page: import('@playwright/test').Page, initialTriggers?: string[]) {
  await page.goto('/?mock=1');
  if (initialTriggers) await page.evaluate(async triggers => {
    const path = '/src/api.ts'; const { api } = await import(/* @vite-ignore */ path);
    const read = api.readWorkflow;
    api.readWorkflow = async slug => {
      const detail = await read(slug);
      return { ...detail, workflow: { ...detail.workflow, triggers } };
    };
  }, initialTriggers);
  await page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Automations', exact: true }).click();
  await page.getByRole('button', { name: 'Open Morning Brief', exact: true }).click();
  await page.getByRole('button', { name: 'Automation options' }).click();
  await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
  await expect(page.getByRole('textbox', { name: 'Automation name', exact: true })).toHaveValue('Morning Brief');
  await page.evaluate(async () => {
    const path = '/src/api.ts'; const { api } = await import(/* @vite-ignore */ path);
    const write = api.writeWorkflow;
    const read = api.readWorkflow;
    let saved: Record<string, unknown> | undefined;
    api.readWorkflow = async slug => {
      const detail = await read(slug);
      return saved ? { ...detail, workflow: { ...detail.workflow, ...saved } } : detail;
    };
    api.writeWorkflow = async (...args: Parameters<typeof write>) => {
      const result = await write(...args);
      saved = args[0] as unknown as Record<string, unknown>;
      (window as any).savedAutomation = saved;
      return result;
    };
  });
}

test('automation editor has a form heading before its section headings', async ({ page }) => {
  await editor(page);
  await expect(page.getByRole('heading', { name: 'Edit automation', exact: true, level: 2 })).toHaveCount(1);
  await expect(page.getByRole('heading', { name: 'Schedule', exact: true, level: 3 })).toHaveCount(1);
});

test('Window and catch-up choices save the stamped schedule without cron typing', async ({ page }) => {
  await editor(page);
  const schedule = page.getByRole('region', { name: 'Schedule', exact: true });
  await schedule.getByRole('button', { name: 'Window', exact: true }).click();
  await choose(schedule.getByRole('combobox', { name: 'Saved window', exact: true }), 'nightly');
  await choose(schedule.getByRole('combobox', { name: 'Catch-up', exact: true }), 'run-all');
  await expect(schedule).toContainText('at most 5');
  await expect(schedule).not.toContainText('never as a backlog');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).savedAutomation?.schedule)).toBe('window:nightly/run-all');
});

test('trigger edits appear in save approval and can be removed by clicking', async ({ page }) => {
  await editor(page);
  const schedule = page.getByRole('region', { name: 'Schedule', exact: true });
  await choose(schedule.getByRole('combobox', { name: 'Trigger kind', exact: true }), 'task-unblocked');
  await schedule.getByRole('button', { name: 'Add trigger', exact: true }).click();
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  const approval = page.getByRole('dialog', { name: /Approve wider access/ });
  await expect(approval).toContainText('Triggers: none → task-unblocked');
  await approval.getByRole('button', { name: 'Cancel', exact: true }).click();
  expect(await page.evaluate(() => (window as any).savedAutomation)).toBeUndefined();
  await expect(schedule.getByRole('button', { name: 'Remove task-unblocked', exact: true })).toHaveCount(1);
  await choose(schedule.getByRole('combobox', { name: 'Trigger kind', exact: true }), 'task-unblocked');
  await schedule.getByRole('button', { name: 'Add trigger', exact: true }).click();
  await expect(schedule.getByRole('button', { name: 'Remove task-unblocked', exact: true })).toHaveCount(1);
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await approval.getByRole('button', { name: 'Save and approve', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).savedAutomation?.triggers)).toContain('task-unblocked');
  await schedule.getByRole('button', { name: 'Remove task-unblocked', exact: true }).click();
  await page.evaluate(() => { (window as any).savedAutomation = undefined; });
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).savedAutomation?.triggers)).toEqual([]);
  await expect(approval).toHaveCount(0);
});

test('a saved window lists its automations and opens a prefilled new automation', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockWorkflowSchedules', JSON.stringify({ 'morning-brief': 'window:friday-pm' })));
  await page.goto('/?mock=1');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) await choose(page.getByRole('combobox', { name: 'Settings section' }), 'background');
  else await page.locator('.settings-index').getByRole('button', { name: 'Background work', exact: true }).click();
  // The windows editor is on the Automations page since Settings PR 4.
  await page.getByRole('button', { name: 'Open windows and run queue', exact: true }).click();
  const friday = page.getByRole('list', { name: 'Automation windows' }).getByRole('group', { name: /^Friday (afternoon|draft)/ });
  await expect(friday.getByRole('button', { name: 'Open Morning Brief', exact: true })).toBeVisible();
  const label = friday.getByRole('textbox', { name: 'Label', exact: true });
  await label.fill('Friday draft');
  await expect(friday.getByRole('button', { name: 'Add automation', exact: true })).toBeDisabled();
  await expect(friday.getByRole('button', { name: 'Open Morning Brief', exact: true })).toBeDisabled();
  await page.getByRole('button', { name: 'Discard changes', exact: true }).click();
  await friday.getByRole('button', { name: 'Add automation', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'New automation', exact: true })).toBeVisible();
  await expect(page.getByRole('combobox', { name: 'Saved window', exact: true })).toContainText('Friday afternoon');
  await page.locator('.workflow-template-card').last().click();
  await expect(page.getByRole('combobox', { name: 'Saved window', exact: true })).toContainText('Friday afternoon');
  await page.getByRole('button', { name: 'Manual', exact: true }).click();
  await expect(page.getByRole('combobox', { name: 'Saved window', exact: true })).toHaveCount(0);
});

test('adding a typed trigger keeps an unknown stored token until it is removed', async ({ page }) => {
  await editor(page, ['future-local-token']);
  const schedule = page.getByRole('region', { name: 'Schedule', exact: true });
  await expect(schedule.getByRole('button', { name: 'Remove future-local-token', exact: true })).toBeVisible();
  await choose(schedule.getByRole('combobox', { name: 'Trigger kind', exact: true }), 'task-unblocked');
  await schedule.getByRole('button', { name: 'Add trigger', exact: true }).click();
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await page.getByRole('dialog', { name: /Approve wider access/ }).getByRole('button', { name: 'Save and approve', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).savedAutomation?.triggers)).toEqual(['future-local-token', 'task-unblocked']);
});

test('window starts refresh while an editor remains open and after focus', async ({ page }) => {
  await page.clock.install({ time: new Date('2026-10-24T12:00:00Z') });
  await page.addInitScript(() => localStorage.setItem('mockWorkflowSchedules', JSON.stringify({ 'morning-brief': 'window:nightly' })));
  await page.goto('/?mock=1');
  await page.evaluate(async () => {
    const path = '/src/api.ts'; const { api } = await import(/* @vite-ignore */ path);
    const read = api.automationWindows;
    (window as any).windowReads = 0;
    api.automationWindows = async () => {
      (window as any).windowReads++;
      const view = await read();
      return { ...view, windows: view.windows.map(w => ({ ...w,
        nextStarts: [30_000, 90_000, 150_000].map(delay => new Date(Date.now() + delay).toISOString()),
      })) };
    };
  });
  await page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Automations', exact: true }).click();
  await page.getByRole('button', { name: 'Open Morning Brief', exact: true }).click();
  await page.getByRole('button', { name: 'Automation options' }).click();
  await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
  const schedule = page.getByRole('region', { name: 'Schedule', exact: true });
  await expect(schedule.locator('.workflow-next-list time')).toHaveCount(3);
  await page.clock.fastForward(60_000);
  await expect.poll(() => page.evaluate(() => (window as any).windowReads)).toBeGreaterThan(1);
  await expect(schedule.locator('.workflow-next-list time')).toHaveCount(3);
  const reads = await page.evaluate(() => (window as any).windowReads);
  await page.evaluate(() => window.dispatchEvent(new Event('focus')));
  await expect.poll(() => page.evaluate(() => (window as any).windowReads)).toBeGreaterThan(reads);
});
