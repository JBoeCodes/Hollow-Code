import { showWorkspaceList } from './choose';
import { test, expect } from './fixtures/test';
import { choose, chooseNew } from './choose';

// Build workspaces (build programme PR B1): a workspace with its own git
// worktree and branch. The mock stands in for `git worktree add`.

test.beforeEach(async ({ page }) => { await page.goto('/?mock=1'); });

test('a build workspace gets a branch and runs its conversations in the worktree', async ({ page }) => {
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Fix the About page layout.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('About page');
  await page.locator('.project-create-options > summary').click();
  await page.getByRole('radio', { name: 'Repository worktree' }).click();
  const repo = page.getByRole('combobox', { name: 'Repository' });
  // Nothing is chosen for the user (audit H05).
  await expect(repo).toHaveAttribute('data-value', '');
  await expect(page.getByRole('button', { name: 'Create project', exact: true })).toBeDisabled();
  await choose(repo, '/Users/fixture/Projects/site');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('About page');
  const chip = page.locator('.work-branch');
  await expect(chip).toHaveText('hollow/about-page');
  await expect(chip).toHaveAttribute('title', /Worktree: \/Users\/fixture\/Hollow\/Worktrees\/site\/about-page/);
  // The workspace conversation is the one bound to the worktree folder.
  await expect(page.locator('.work-conversation .chat-composer, .work-conversation textarea').first()).toBeVisible();
  const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]'));
  expect(stored.at(-1).cwd).toBe('/Users/fixture/Hollow/Worktrees/site/about-page');
});

test('removing a worktree with unsaved work asks twice, then falls back', async ({ page }, info) => {
  // Narrow windows hide the header's All work button; the flow is width-independent.
  test.skip(info.project.name === 'chromium-narrow', 'uses the wide header');
  await page.evaluate(() => localStorage.setItem('mockWorktreeUnsaved', '1'));
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Try a change.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Spike');
  await page.locator('.project-create-options > summary').click();
  await page.getByRole('radio', { name: 'Repository worktree' }).click();
  await choose(page.getByRole('combobox', { name: 'Repository' }), '/Users/fixture/Projects/hollow');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-branch')).toHaveText('hollow/spike');
  await page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true }).click();
  await showWorkspaceList(page);
  const card = page.locator('.work-card', { hasText: 'Spike' });
  await expect(card).toHaveAttribute('title', /Build · hollow\/spike/);
  await card.click({ button: 'right' });
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Remove worktree…' }).click();
  await page.getByRole('button', { name: 'Remove worktree', exact: true }).click();
  await expect(page.getByText('It has 2 uncommitted changes and 1 unpushed commit.', { exact: false })).toBeVisible();
  await page.getByRole('button', { name: 'Remove anyway', exact: true }).click();
  await card.click();
  await expect(page.locator('.work-branch')).toHaveText('hollow/spike · worktree removed');
});

// The flake above (2026-09-29, T1): the context menu closed on ANY scroll
// event, and scroll anchoring fires them on the list while rows above the
// right-clicked card animate in. A scroll that leaves the card where it was
// must keep the menu; a wheel gesture outside it still closes it.
test('the workspace menu survives a scroll that does not move its row', async ({ page }, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'uses the wide header');
  await page.evaluate(() => localStorage.setItem('mockWorkspaces', JSON.stringify([
    { id: 'alpha-1', title: 'Alpha', status: 'active', path: 'workspaces/alpha-1', cwd: '/Users/fixture/vault/workspaces/alpha-1', modified: '2026-09-26T12:00:00Z' },
  ])));
  await page.reload();
  await showWorkspaceList(page);
  const card = page.locator('.work-card', { hasText: 'Alpha' });
  await card.click({ button: 'right' });
  const menu = page.locator('.ctx-menu');
  await expect(menu).toBeVisible();
  await page.evaluate(() => {
    for (const el of [document.querySelector('.work-list'), document.documentElement]) el?.dispatchEvent(new Event('scroll'));
  });
  await page.waitForTimeout(100);
  await expect(menu).toBeVisible();
  // A wheel outside the menu closes it (one over the menu itself never does).
  // Work's layout decides where the card, and so the menu, sits; aim
  // just above the menu rather than at a fixed point.
  const box = await menu.boundingBox();
  await page.mouse.move((box?.x ?? 0) + (box?.width ?? 0) / 2, Math.max(60, (box?.y ?? 100) - 30));
  await page.mouse.wheel(0, 40);
  await expect(menu).toHaveCount(0);
});
