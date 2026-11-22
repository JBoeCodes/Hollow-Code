import { test, expect } from './fixtures/test';
import { choose, chooseNew, openChanges } from './choose';

// Services that outlive a turn (build programme PR B6): the Changes tab lists
// the worktree's services. The mock (src/mockBuild.ts) stands in for the broker.

const TREE = '/Users/fixture/Hollow/Worktrees/hollow/dev-server';

test('the Changes tab lists services with Open, Log and Stop', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.evaluate(tree => localStorage.setItem('mockServices', JSON.stringify([
    { scope: tree, name: 'web', command: 'npm run dev -- --port 5173', cwd: tree, port: 5173, kind: 'dev-server', status: 'running', exitCode: null, pid: 4242, startedAt: '2026-09-27T05:00:00Z', log: '/Users/fixture/Library/Logs/Hollow/services/x/web.log' },
    { scope: tree, name: 'once', command: 'npm run build', cwd: tree, port: null, kind: 'other', status: 'exited', exitCode: 1, pid: 4243, startedAt: '2026-09-27T05:00:00Z', log: '/x/once.log' },
    { scope: '/elsewhere', name: 'other', command: 'x', cwd: '/elsewhere', port: null, kind: 'other', status: 'running', exitCode: null, pid: 1, startedAt: '', log: '' },
  ])), TREE);
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Run the site locally.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Dev server');
  await page.locator('.project-create-options > summary').click();
  await page.getByRole('radio', { name: 'Repository worktree' }).click();
  await choose(page.getByRole('combobox', { name: 'Repository' }), '/Users/fixture/Projects/hollow');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-branch')).toHaveText('hollow/dev-server');
  // Changes in the Preview pane, from the top bar's branch (COO rebuild L4).
  await openChanges(page, 'hollow/dev-server');

  const services = page.locator('.bc-services');
  await expect(services.locator('.bc-service')).toHaveCount(2);
  const web = services.locator('.bc-service', { hasText: 'web' });
  await expect(web).toHaveAttribute('data-status', 'running');
  await expect(web.getByRole('button', { name: 'Open :5173' })).toBeVisible();
  await expect(services.locator('.bc-service', { hasText: 'once' }).getByRole('button', { name: 'Open' })).toHaveCount(0);
  await web.getByRole('button', { name: 'Log' }).click();
  await expect(web.locator('.bc-service-log')).toContainText('ready in 312 ms');

  await web.getByRole('button', { name: 'Stop' }).click();
  await expect(page.locator('.confirm-dialog')).toContainText('Stop web?');
  await page.locator('.confirm-dialog').getByRole('button', { name: 'Stop', exact: true }).click();
  await expect(services.locator('.bc-service')).toHaveCount(1);
  const log = await page.evaluate(() => JSON.parse(localStorage.getItem('mockBuildLog') ?? '[]') as { command: string; args: Record<string, unknown> }[]);
  expect(log.find(e => e.command === 'stop_service')).toMatchObject({ args: { cwd: TREE, name: 'web' } });
  // An exited service is removed without a confirm.
  await services.locator('.bc-service', { hasText: 'once' }).getByRole('button', { name: 'Remove' }).click();
  await expect(services).toContainText('Nothing running.');
});
