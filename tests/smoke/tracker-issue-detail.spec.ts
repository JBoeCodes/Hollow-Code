// The issue detail (overhaul B1, board S3) against the browser mock: every
// field, edits that write through, read-only fields that say so, the
// activity feed and Agent activity, Open in Linear and Archive. Written in
// lane 3 of the COO rebuild without running it. The full page, relation rows
// and the tracker's change history (row T1, 2026-11-12) were also written
// without running them.
import { expect, test } from './fixtures/test';
import { choose, openTasks, showTaskList } from './choose';

test.beforeEach(async ({ page }) => {
  await page.goto('/?mock=1');
  await openTasks(page);
  await page.locator('.issue-row[data-issue-id="i-31"] .issue-row-main').click();
});

test('the detail shows every field of the issue', async ({ page }) => {
  const pane = page.locator('.issue-pane');
  await expect(pane.locator('.issue-detail-crumb')).toContainText('ENG-31');
  await expect(pane.getByRole('textbox', { name: 'Title' })).toHaveValue('Match bank deposits to receipts');
  const props = pane.getByRole('complementary', { name: 'Properties' });
  await expect(props.getByRole('combobox', { name: 'Status' })).toContainText('In Progress');
  await expect(props.getByRole('combobox', { name: 'Priority' })).toContainText('High');
  await expect(props.getByRole('combobox', { name: 'Assignee' })).toContainText('Alex Rivera');
  await expect(props.getByRole('combobox', { name: 'Project' })).toContainText('Receipts rebuild');
  await expect(props.getByRole('combobox', { name: 'Cycle' })).toContainText('Cycle 4');
  await expect(props.getByRole('combobox', { name: 'Milestone' })).toContainText('Receipts live');
  await expect(props.getByRole('spinbutton', { name: 'Estimate' })).toHaveValue('3');
  await expect(props).toContainText('Created by');
  await expect(pane.getByRole('region', { name: 'Sub-issues' })).toContainText('ENG-36');
  await expect(pane.getByRole('region', { name: 'Relations' })).toContainText('Blocked by');
  await expect(pane.getByRole('region', { name: 'Links and attachments' })).toContainText('Bank file format');
  await expect(pane.getByRole('region', { name: 'Activity', exact: true })).toContainText('Bank file format attached.');
  await expect(pane.getByRole('region', { name: 'Agent activity' })).toBeVisible();
  await expect(pane.getByRole('button', { name: 'Open in Linear (ENG-31)' })).toBeVisible();
});

test('a narrow side pane stacks the properties instead of overlapping the title', async ({ page }) => {
  // Overnight C15: at 1440x900 the side pane is about 432px wide; the body
  // splits into two columns only when the pane itself is wider than 45em.
  await page.setViewportSize({ width: 1440, height: 900 });
  const pane = page.locator('.issue-pane');
  const title = await pane.getByRole('textbox', { name: 'Title' }).boundingBox();
  const props = await pane.getByRole('complementary', { name: 'Properties' }).boundingBox();
  expect(title && props).toBeTruthy();
  const overlapX = Math.min(title!.x + title!.width, props!.x + props!.width) - Math.max(title!.x, props!.x);
  const overlapY = Math.min(title!.y + title!.height, props!.y + props!.height) - Math.max(title!.y, props!.y);
  expect(overlapX <= 0 || overlapY <= 0).toBe(true);
});

test('status, priority and assignee edit in place and reach the list', async ({ page }) => {
  const props = page.getByRole('complementary', { name: 'Properties' });
  await choose(props.getByRole('combobox', { name: 'Status' }), 'st-review');
  await expect(props.getByRole('combobox', { name: 'Status' })).toContainText('In Review');
  await choose(props.getByRole('combobox', { name: 'Assignee' }), { label: 'Sam Lee' });
  await choose(props.getByRole('combobox', { name: 'Priority' }), '1');
  // A narrow window shows the pane in the list's place: close it to look.
  await showTaskList(page);
  await expect(page.locator('.issue-row[data-issue-id="i-31"] .issue-assignee-name')).toHaveText('Sam Lee');
  await expect(page.locator('.issue-row[data-issue-id="i-31"]').getByRole('img', { name: 'Urgent' })).toBeVisible();
});

test('the title and description edit and save', async ({ page }) => {
  const title = page.getByRole('textbox', { name: 'Title' });
  await title.fill('Match deposits to receipts');
  await title.press('Enter');
  await expect(page.locator('.issue-row[data-issue-id="i-31"] .issue-title')).toHaveText('Match deposits to receipts');
  await page.getByRole('button', { name: 'Edit description' }).click();
  await page.getByRole('textbox', { name: 'Description' }).fill('Match **every** deposit.');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.locator('.issue-description strong')).toHaveText('every');
});

test('read-only fields say so on hover', async ({ page }) => {
  const created = page.locator('.issue-property', { hasText: 'Created by' });
  await expect(created).toHaveAttribute('title', 'Set by the tracker');
});

test('a sub-issue opens from the detail', async ({ page }) => {
  await page.getByRole('region', { name: 'Sub-issues' }).getByRole('button', { name: 'ENG-36' }).click();
  await expect(page.locator('.issue-pane .issue-detail-crumb')).toContainText('ENG-36');
});

test('Archive asks first and archives in the tracker', async ({ page }) => {
  await page.getByRole('button', { name: 'Archive in Linear' }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toContainText('Archive ENG-31 in Linear?');
  await dialog.getByRole('button', { name: 'Archive' }).click();
  await expect(page.locator('.issue-detail-banner')).toContainText('Archived in Linear');
});

test('an unassigned issue says Unassigned in the column', async ({ page }) => {
  await showTaskList(page);
  await page.locator('.issue-row[data-issue-id="i-34"] .issue-row-main').click();
  await expect(page.getByRole('complementary', { name: 'Properties' }).getByRole('combobox', { name: 'Assignee' })).toContainText('Unassigned');
});

test('Open full page shows the issue as the page, and Back to list returns', async ({ page }) => {
  // A narrow window shows the pane instead of the list, so Back to list
  // closes the pane there rather than returning to it beside the list.
  const narrow = await page.locator('.task-collection').isHidden();
  await page.getByRole('button', { name: 'Open full page' }).click();
  await expect(page.locator('.task-desk')).toHaveClass(/issue-page/);
  await expect(page.locator('.task-collection')).toBeHidden();
  await expect(page.getByRole('textbox', { name: 'Title' })).toHaveValue('Match bank deposits to receipts');
  const props = page.getByRole('complementary', { name: 'Properties' });
  await expect(props.getByRole('combobox', { name: 'Status' })).toBeVisible();
  await expect(props.getByRole('button', { name: 'Open in Linear (ENG-31)' })).toBeVisible();
  await page.getByRole('button', { name: 'Back to list' }).click();
  await expect(page.locator('.task-collection')).toBeVisible();
  await expect(page.locator('.issue-row[data-issue-id="i-31"]')).toBeVisible();
  if (narrow) {
    await expect(page.locator('.task-desk')).not.toHaveClass(/has-selection/);
    await page.locator('.issue-row[data-issue-id="i-31"] .issue-row-main').click();
  }
  await expect(page.locator('.issue-pane .issue-detail-crumb')).toContainText('ENG-31');

  // Escape in the full page goes back too, but not while typing in the title.
  await page.getByRole('button', { name: 'Open full page' }).click();
  await page.getByRole('textbox', { name: 'Title' }).focus();
  await page.keyboard.press('Escape');
  await expect(page.locator('.task-collection')).toBeHidden();
  await page.locator('.issue-detail-crumb').click();
  await page.keyboard.press('Escape');
  await expect(page.locator('.task-collection')).toBeVisible();
});

test('a relation row shows the other issue’s title and opens it', async ({ page }) => {
  const relations = page.getByRole('region', { name: 'Relations' });
  const blocked = relations.getByRole('button', { name: /^ENG-33 Read the bank file format/ });
  await expect(blocked).toContainText('Read the bank file format');
  await expect(page.getByRole('region', { name: 'Sub-issues' })).toContainText('Parse deposit lines');
  await blocked.click();
  await expect(page.locator('.issue-pane .issue-detail-crumb')).toContainText('ENG-33');
});

test('the activity shows the tracker’s state change for ENG-31', async ({ page }) => {
  const activity = page.getByRole('region', { name: 'Activity', exact: true });
  await expect(activity.locator('.issue-feed-change[data-kind="state"]')).toContainText('Alex Rivera moved from Todo to In Progress');
  await expect(activity.locator('.issue-feed-change[data-kind="state"] time')).toHaveText('2h ago');
  await expect(activity).toContainText('Bank file format attached.');
  await expect(activity).not.toContainText('Last updated');
});

test('an issue with no history read falls back to the feed the mirror gives', async ({ page }) => {
  await page.evaluate(() => localStorage.setItem('mockTracker', JSON.stringify({ historyFails: true })));
  await showTaskList(page);
  await page.locator('.issue-row[data-issue-id="i-32"] .issue-row-main').click();
  await showTaskList(page);
  await page.locator('.issue-row[data-issue-id="i-31"] .issue-row-main').click();
  const activity = page.getByRole('region', { name: 'Activity', exact: true });
  await expect(activity).toContainText('Last updated');
  await expect(activity.locator('.issue-feed-change')).toHaveCount(0);
});
