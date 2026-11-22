// Tasks with two teams (the shared Engineering and the personal team Personal,
// PER), against the browser mock seeded with `personalTeamKey`
// (`src/mockTracker.ts`). Written without a local run; the gate runs it.
import { expect, test, type Page } from './fixtures/test';
import { openTasks } from './choose';

const seedTwo = (page: Page) => page.addInitScript(() =>
  localStorage.setItem('mockTracker', JSON.stringify({ personalTeamId: '22222222-2222-4222-8222-222222222222', personalTeamKey: 'PER' })));
const list = (page: Page) => page.getByLabel('Tasks', { exact: true });
const teamGroup = (page: Page) => page.getByRole('group', { name: 'Team', exact: true });

test('one team: no team filter and no team chips', async ({ page }) => {
  await page.goto('/?mock=1');
  await openTasks(page);
  await expect(page.locator('.tracker-team-chip')).toBeVisible();
  await expect(teamGroup(page)).toHaveCount(0);
  await expect(list(page).locator('.issue-team')).toHaveCount(0);
});

test('two teams: the filter narrows the list to one team and back, and rows carry a team chip', async ({ page }) => {
  await seedTwo(page);
  await page.goto('/?mock=1');
  await openTasks(page);
  await expect(teamGroup(page)).toBeVisible();
  await expect(teamGroup(page).getByRole('button', { name: 'All teams' })).toHaveAttribute('aria-pressed', 'true');
  await expect(list(page).locator('.issue-row[data-issue-id="i-31"] .issue-team')).toHaveText('ENG');
  await expect(list(page).locator('.issue-row[data-issue-id="i-per-1"] .issue-team')).toHaveText('PER');
  await teamGroup(page).getByRole('button', { name: 'PER' }).click();
  await expect(teamGroup(page).getByRole('button', { name: 'PER' })).toHaveAttribute('aria-pressed', 'true');
  await expect(list(page).locator('.issue-row[data-issue-id="i-per-1"]')).toBeVisible();
  await expect(list(page).locator('.issue-row[data-issue-id="i-31"]')).toHaveCount(0);
  await teamGroup(page).getByRole('button', { name: 'ENG', exact: true }).click();
  await expect(list(page).locator('.issue-row[data-issue-id="i-31"]')).toBeVisible();
  await expect(list(page).locator('.issue-row[data-issue-id="i-per-1"]')).toHaveCount(0);
  await teamGroup(page).getByRole('button', { name: 'All teams' }).click();
  await expect(list(page).locator('.issue-row[data-issue-id="i-per-1"]')).toBeVisible();
});

test('two teams: the Projects tab shows each project with its team and filters too', async ({ page }) => {
  await seedTwo(page);
  await page.goto('/?mock=1');
  await openTasks(page);
  await page.getByRole('group', { name: 'Views' }).getByRole('button', { name: 'Projects' }).click();
  const projects = page.getByRole('list', { name: 'Projects' });
  await expect(projects.locator('.project-row[data-project-id="p-per-1"] .issue-team')).toHaveText('PER');
  await expect(projects.locator('.project-row[data-project-id="p-1"] .issue-team')).toHaveText('ENG');
  await teamGroup(page).getByRole('button', { name: 'PER' }).click();
  await expect(projects.locator('.project-row[data-project-id="p-1"]')).toHaveCount(0);
  await expect(projects.locator('.project-row[data-project-id="p-per-1"]')).toBeVisible();
});

test('two teams: New issue asks which team (ENG by default) and creates in PER', async ({ page }) => {
  await seedTwo(page);
  await page.goto('/?mock=1');
  await openTasks(page);
  await page.getByRole('button', { name: 'New issue' }).click();
  const picker = page.getByRole('group', { name: 'Create in team' });
  await expect(picker.getByRole('button', { name: /Engineering/ })).toHaveAttribute('aria-pressed', 'true');
  await picker.getByRole('button', { name: /Personal/ }).click();
  await page.getByRole('textbox', { name: 'Issue title' }).fill('Note to self');
  await page.getByRole('button', { name: /^Create in/ }).click();
  await expect(list(page).locator('.issue-row', { hasText: 'Note to self' }).locator('.issue-team')).toHaveText('PER');
});

test('one team: New issue offers no team choice', async ({ page }) => {
  await page.goto('/?mock=1');
  await openTasks(page);
  await page.getByRole('button', { name: 'New issue' }).click();
  await expect(page.getByRole('group', { name: 'Create in team' })).toHaveCount(0);
});
