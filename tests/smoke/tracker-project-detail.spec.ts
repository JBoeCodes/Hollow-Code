// One project's detail (overhaul B3, board S4): from an issue's Project on
// the Tasks page, and a project record in Library, Records. Against the
// browser mock (`src/mockTracker.ts`: p-1 Receipts rebuild is tracked with
// workspace ws-receipts; ws-fleet is vault only). Written in lane 3 of the
// COO rebuild without running it; the one full browser run on final main
// runs it.
import { expect, test, type Page } from './fixtures/test';
import { choose, openLibrary, openTasks } from './choose';
import { openLibraryView } from './libraryView';

const RECORDS = [
  { path: 'workspaces/ws-fleet/PROJECT.md', basename: 'PROJECT', title: 'Fleet dashboard' },
  { path: 'workspaces/ws-receipts/PROJECT.md', basename: 'PROJECT', title: 'Receipts rebuild' },
  { path: 'library/records/meetings/2026/2026-11-04-receipts-kickoff.md', basename: '2026-11-04-receipts-kickoff', title: 'Receipts kickoff' },
  { path: 'library/records/people/dana-kim.md', basename: 'dana-kim', title: 'Dana Kim' },
];

async function openRecord(page: Page, _path: string) {
  await page.goto('/?mock=1');
  await page.evaluate(notes => localStorage.setItem('mockVaultNotes', JSON.stringify(notes)), RECORDS);
  await page.reload();
  await openLibrary(page);
  // Library opens on All; the folder picker lives in Notes.
  await openLibraryView(page, 'Notes');
  await choose(page.getByRole('combobox', { name: 'Library folder' }), 'records');
}

test('an issue’s Project opens the project with the tracker’s fields and the portfolio', async ({ page }) => {
  await page.goto('/?mock=1');
  await openTasks(page);
  await page.locator('.issue-row[data-issue-id="i-31"] .issue-row-main').click();
  await page.getByRole('button', { name: 'Open project' }).click();
  const detail = page.locator('.project-detail');
  await expect(detail.getByRole('heading', { name: 'Receipts rebuild' })).toBeVisible();
  await expect(detail.getByRole('note')).toContainText('Tracked in Linear');
  const props = detail.getByRole('complementary', { name: 'Project properties' });
  await expect(props).toContainText('In Progress');
  await expect(props).toContainText('Alex Rivera');
  await expect(props).toContainText('Automation');
  await expect(props).toContainText('1,200');
  await expect(detail.getByRole('button', { name: /Open in Linear \(P-ENG-1\)/ })).toBeVisible();
});

// The project filter went in B1; since B4 a project's issues list inside its
// detail through the same issue list, so Tasks has no Show its issues.
test('the project detail lists its own issues', async ({ page }) => {
  await page.goto('/?mock=1');
  await openTasks(page);
  await page.locator('.issue-row[data-issue-id="i-31"] .issue-row-main').click();
  await page.getByRole('button', { name: 'Open project' }).click();
  const issues = page.locator('.project-detail').getByRole('region', { name: 'Issues', exact: true });
  await expect(issues.locator('.issue-row[data-issue-id="i-31"]')).toBeVisible();
  await expect(issues.locator('.issue-row[data-issue-id="i-34"]')).toHaveCount(0);
  await issues.locator('.issue-row[data-issue-id="i-31"] .issue-row-main').click();
  await expect(page.locator('.issue-pane .issue-detail-crumb')).toContainText('ENG-31');
});

test('portfolio fields edit and save in PROJECT.md, and a bad number is refused', async ({ page }) => {
  await page.goto('/?mock=1');
  await openTasks(page);
  await page.locator('.issue-row[data-issue-id="i-31"] .issue-row-main').click();
  await page.getByRole('button', { name: 'Open project' }).click();
  const props = page.getByRole('complementary', { name: 'Project properties' });
  await props.getByRole('button', { name: 'Edit' }).click();
  const form = props.getByRole('form', { name: 'Edit portfolio' });
  await form.getByLabel('Value a year').fill('lots');
  await expect(form.getByText('A number, like 1,200')).toBeVisible();
  await expect(form.getByRole('button', { name: 'Save' })).toBeDisabled();
  await form.getByLabel('Value a year').fill('40,000');
  await form.getByLabel('Stakeholders').fill('Dana Kim\nOps leads');
  await form.getByRole('button', { name: 'Save' }).click();
  await expect(props).toContainText('40,000');
  await expect(props).toContainText('Ops leads');
});

test('Library, Records lists records but not people, and a project record opens as the project', async ({ page }) => {
  await openRecord(page, 'workspaces/ws-fleet/PROJECT.md');
  const list = page.getByRole('complementary', { name: 'Library files' });
  await expect(list).toContainText('Receipts kickoff');
  await expect(list).not.toContainText('Dana Kim');
  await list.getByText('Fleet dashboard').click();
  await expect(page.locator('.project-detail')).toBeVisible();
  await page.getByRole('button', { name: 'Show the file' }).click();
  await expect(page.locator('.project-detail')).toHaveCount(0);
  await page.getByRole('button', { name: 'Show the project' }).click();
  await expect(page.locator('.project-detail')).toBeVisible();
});

test('Track in Linear says what it creates, then tracks the vault-only project', async ({ page }) => {
  await openRecord(page, 'workspaces/ws-fleet/PROJECT.md');
  await page.getByRole('complementary', { name: 'Library files' }).getByText('Fleet dashboard').click();
  const detail = page.locator('.project-detail');
  await expect(detail.getByRole('note')).toContainText('Vault only');
  await detail.getByRole('button', { name: 'Track in Linear' }).click();
  await expect(detail).toContainText('Everyone on the team sees it');
  await detail.getByRole('button', { name: 'Create in Linear' }).click();
  await expect(detail.getByRole('note')).toContainText('Tracked in Linear');
  await expect(detail.getByRole('button', { name: 'Track in Linear' })).toHaveCount(0);
});
