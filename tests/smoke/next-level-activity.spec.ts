import { expect, test } from './fixtures/test';
import { choose, chooseNew, openChanges } from './choose';
import { bellDialog, bellHistoryRows, navigateWaiting, openBellHistory, openWaiting, waitingBell } from './waiting';

/** Next-level P19/P20 (browser mock), now the one "waiting on you" surface
 *  (decision 7): it groups by kind, opens the exact item, pages, and names
 *  sources it couldn't read; a build's delivery record keeps committed,
 *  published, merged and installed apart. */
test.beforeEach(({}, info) => { test.skip(info.project.name === 'chromium-narrow', 'desktop panels'); });

test('an unsaved draft waits in Waiting on you and opens its file', async ({ page }) => {
  // Idle eleven minutes: Review lists a draft only after ten (P1-14, D3).
  await page.addInitScript(() => localStorage.setItem('hollow.fileDraft.workspaces/local-q3-review/outputs/decision-slides.md', JSON.stringify({ text: '# Decisions\n\nchanged', baseline: '# Decisions for October 15\n', savedAt: new Date(Date.now() - 11 * 60_000).toISOString() })));
  await page.goto('/?mock=1&demo=1');
  const panel = await openWaiting(page);
  const drafts = panel.getByRole('region', { name: 'Unsaved drafts' });
  await expect(drafts).toContainText('Unsaved draft: decision-slides.md');
  // The row itself, not its Discard (P1-14).
  await drafts.getByRole('button', { name: /^Unsaved draft/ }).click();
  await expect(panel).toHaveCount(0);
  await expect(page.locator('.work-file-restored, .conflict-resolver').first()).toBeVisible();
});

test('the bell groups everything waiting by kind and opens a failure by its notification', async ({ page }) => {
  await page.addInitScript(() => {
    if (localStorage.getItem('mockNeedsYouExtra')) return;
    localStorage.setItem('mockNeedsYouExtra', JSON.stringify([
      { id: 'question:q-1', kind: 'question', sourceId: 'q-1', title: 'Which venue should we book?', detail: null, state: 'pending', at: '2026-10-03T09:00:00Z', workspaceId: null, sessionKey: 's-mock-question', path: null, category: 'question', link: null },
      { id: 'question:q-2', kind: 'question', sourceId: 'q-2', title: 'Keep the old heading?', detail: 'The file changed', state: 'stale', at: '2026-10-03T08:00:00Z', workspaceId: null, sessionKey: 's-mock-question', path: null, category: 'question', link: null },
    ]));
  });
  await page.goto('/?mock=1');
  const panel = await openWaiting(page);
  const asks = panel.getByRole('region', { name: 'Approvals and questions' });
  await expect(asks).toContainText('Which venue should we book?');
  // A question that can no longer be answered is listed, says why, and opens nothing.
  const stale = asks.getByRole('button', { name: /Keep the old heading/ });
  await expect(stale).toHaveAttribute('aria-disabled', 'true');
  await expect(stale).toContainText('can no longer be answered');
  await expect(panel.getByRole('region', { name: 'Review items and proposals' })).toContainText('Release verification');
  // The keys walk the rows; Escape closes and gives focus back to the bell.
  await page.keyboard.press('j');
  await expect(panel.locator('[data-waiting-row]:focus')).toHaveCount(1);
  const failures = panel.getByRole('region', { name: 'Failures' });
  await expect(failures).toContainText('Close my day failed');
  // Opening a failure follows its own link and leaves it waiting (the plan's
  // P1-13 call, run 13 FIX-MR): in Review, Mark handled, x or resolving marks it read.
  await failures.locator('[data-waiting-row]').filter({ hasText: 'Close my day failed' }).click();
  await expect(panel).toHaveCount(0);
  await openWaiting(page);
  const still = page.getByRole('region', { name: 'Failures' });
  await expect(still).toContainText('Close my day failed');
  expect(await page.evaluate(() => window.__mockApiCalls?.mark_notification_read ?? 0)).toBe(0);
  await still.getByRole('button', { name: 'Mark handled: Close my day failed', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.mark_notification_read ?? 0)).toBe(1);
  await expect(page.getByRole('region', { name: 'Failures' })).toHaveCount(0);
});

test('Needs you has no Incoming; x on a Failures row marks it read', async ({ page }) => {
  await page.goto('/?mock=1');
  await navigateWaiting(page);
  await expect(page.getByRole('navigation', { name: 'Review views' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Incoming', exact: true })).toHaveCount(0);
  await page.getByRole('navigation', { name: 'Waiting filters' }).getByRole('button', { name: /^Failures/ }).click();
  const panel = await openWaiting(page);
  const failures = panel.getByRole('region', { name: 'Failures' });
  await expect(failures).toContainText('Close my day failed');
  expect(await page.evaluate(() => window.__mockApiCalls?.mark_notification_read ?? 0)).toBe(0);
  await failures.locator('[data-waiting-row]').filter({ hasText: 'Close my day failed' }).focus();
  await page.keyboard.press('x');
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.mark_notification_read ?? 0)).toBe(1);
  await expect(panel.getByRole('region', { name: 'Failures' })).toHaveCount(0);
});

test('a failure with no link stays waiting when opened and says what clears it', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockNotificationsExtra', JSON.stringify([{
    id: 'no-link-failure', title: 'Nightly export failed', body: 'disk full', kind: 'error', source: 'export',
    actionLabel: null, actionUrl: null, created: new Date().toISOString(), read: false,
  }])));
  await page.goto('/?mock=1');
  const panel = await openWaiting(page);
  const failures = panel.getByRole('region', { name: 'Failures' });
  await failures.locator('[data-waiting-row]').filter({ hasText: 'Nightly export failed' }).click();
  await expect(page.getByText('Nothing to open for this failure: Mark handled clears it.').first()).toBeVisible();
  await expect(failures).toContainText('Nightly export failed');
  expect(await page.evaluate(() => window.__mockApiCalls?.mark_notification_read ?? 0)).toBe(0);
});

test('Clear listed in the bell history removes what it lists and leaves the failures waiting (P0-08)', async ({ page }) => {
  await page.goto('/?mock=1');
  const panel = await openWaiting(page);
  const failures = panel.getByRole('region', { name: 'Failures' });
  await expect(failures).toContainText('Close my day failed');
  const before = await failures.getByRole('button').count();
  // The history lives once, in Bell → Notifications (U3b), not in this view.
  await expect(panel.locator('.waiting-recent')).toHaveCount(0);
  await openBellHistory(page);
  // The unread failure is listed on its own row, with its own Mark read.
  await expect(bellHistoryRows(page).filter({ hasText: 'Close my day failed' }).getByRole('button', { name: 'Mark read', exact: true })).toBeVisible();
  await bellDialog(page).getByRole('button', { name: 'Clear listed' }).click();
  // It asks first, with the count, and says the failures stay.
  const dialog = page.getByRole('dialog').filter({ hasText: /Clear \d+ notifications?\?/ });
  await expect(dialog).toContainText('Unread failures stay in Needs you.');
  await dialog.getByRole('button', { name: 'Clear', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.delete_notifications ?? 0)).toBe(1);
  expect(await page.evaluate(() => window.__mockApiCalls?.clear_notifications ?? 0)).toBe(0);
  // What is left listed is the failure alone, still waiting.
  await openBellHistory(page);
  await expect(bellHistoryRows(page)).toHaveCount(1);
  await expect(bellHistoryRows(page)).toContainText('Close my day failed');
  // The bell's own Close: the waiting view also listens for Escape.
  await bellDialog(page).getByRole('button', { name: 'Close notifications', exact: true }).click();
  await expect(bellDialog(page)).toHaveCount(0);
  await expect(failures).toContainText('Close my day failed');
  await expect(failures.getByRole('button')).toHaveCount(before);
});

test('a failure whose link is a review opens the filtered surface, not a closed one', async ({ page }) => {
  await page.addInitScript(() => {
    if (localStorage.getItem('mockReviewItems')) return;
    localStorage.setItem('mockNotificationsExtra', JSON.stringify([{
      id: 'streak-notice', title: 'Morning Brief failed 3 times in a row', body: 'claude exited 1', kind: 'error', source: 'morning-brief',
      actionLabel: 'Review', actionUrl: 'workflow-failures:morning-brief', created: new Date().toISOString(), read: false,
    }]));
    localStorage.setItem('mockReviewItems', JSON.stringify([{
      id: 'streak-morning', kind: 'workflow_failure_streak', source: 'workflow', risk: 'local_write',
      status: 'pending', createdAt: '2026-09-25T12:00:00Z', updatedAt: '2026-09-25T12:00:00Z',
      sourcePath: 'workflows/morning-brief.md', snoozedUntil: null, error: null,
      fingerprint: 'workflow-failure-streak:morning-brief:run-3',
      payload: { slug: 'morning-brief', title: 'Morning Brief', count: 3, firstFailedAt: '2026-09-25T07:00:00Z', lastError: 'claude exited 1', schedule: '0 0 7 * * * *', triggers: [] },
    }]));
  });
  await page.goto('/?mock=1');
  const panel = await openWaiting(page);
  // The row itself, not its Mark handled button (P1-13), which names the same failure.
  await panel.getByRole('region', { name: 'Failures' }).locator('[data-waiting-row]').filter({ hasText: 'Morning Brief failed 3 times' }).click();
  // The link reopens the surface filtered to failing automations: it stays open, card shown.
  await expect(panel).toBeVisible();
  await expect(panel).toHaveAttribute('data-filter', 'automationHealth');
  await expect(panel.locator('.review-inbox-card').filter({ hasText: 'Morning Brief' })).toBeVisible();
  // Opening it leaves it unread (run 13 FIX-MR).
  expect(await page.evaluate(() => window.__mockApiCalls?.mark_notification_read ?? 0)).toBe(0);
});

test('a bell history notice whose link is a review opens the filtered surface', async ({ page }) => {
  await page.addInitScript(() => {
    if (localStorage.getItem('mockReviewItems')) return;
    localStorage.setItem('mockNotificationsExtra', JSON.stringify([{
      id: 'paused-notice', title: 'Morning Brief is paused for a permission review', body: '', kind: 'warning', source: 'morning-brief',
      actionLabel: 'Review', actionUrl: 'workflow-permissions:morning-brief', created: new Date().toISOString(), read: false,
    }]));
    localStorage.setItem('mockReviewItems', JSON.stringify([{
      id: 'perm-morning', kind: 'workflow_permissions', source: 'workflow', risk: 'local_write',
      status: 'pending', createdAt: '2026-09-25T12:00:00Z', updatedAt: '2026-09-25T12:00:00Z',
      sourcePath: 'workflows/morning-brief.md', snoozedUntil: null, error: null,
      fingerprint: 'workflow-permissions:morning-brief:a:b',
      payload: { slug: 'morning-brief', title: 'Morning Brief', summary: 'Permission mode: acceptEdits → bypassPermissions', escalation: true, diff: [] },
    }]));
  });
  await page.goto('/?mock=1');
  // Bell → Notifications (U3b): the row closes the bell first, then opens Review filtered.
  await openBellHistory(page);
  await bellHistoryRows(page).filter({ hasText: 'paused for a permission review' }).locator('.notif-row-body').click();
  await expect(bellDialog(page)).toHaveCount(0);
  const panel = page.getByRole('region', { name: 'Waiting on you' });
  await expect(panel).toBeVisible();
  await expect(panel).toHaveAttribute('data-filter', 'automations');
  await expect(panel.locator('.review-inbox-card').filter({ hasText: 'Morning Brief' })).toBeVisible();
});

test('a review row opens its card in place; the surface says which sources it could not read', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockNeedsYouUnavailable', JSON.stringify(['questions', 'Notion sync'])));
  await page.goto('/?mock=1');
  const panel = await openWaiting(page);
  await expect(panel.getByRole('status').filter({ hasText: 'Couldn’t read questions and Notion sync' })).toBeVisible();
  const reviews = panel.getByRole('region', { name: 'Review items and proposals' });
  // The fixture's budget pause opens its card in place.
  const row = reviews.getByRole('button', { name: /Budget pause/ });
  await expect(row).toHaveAttribute('aria-expanded', 'false');
  await row.click();
  await expect(row).toHaveAttribute('aria-expanded', 'true');
  const card = panel.locator('.review-inbox-card[data-review-id="review-budget-pause-1"]');
  await expect(card).toBeVisible();
  await expect(card.getByRole('button', { name: 'Raise and resume', exact: true })).toBeVisible();
  // A pending learning is listed and counted, but decided in Library
  // Learnings (#1238): its row is a link there, never a card in place.
  const learning = reviews.getByRole('button', { name: /Release verification/ });
  await expect(learning).toContainText('Decide in Library Learnings');
  await expect(learning).not.toHaveAttribute('aria-expanded');
  await expect(panel.locator('.review-inbox-card').filter({ hasText: 'Release verification' })).toHaveCount(0);
  await learning.click();
  await expect(page.getByRole('region', { name: 'Learnings', exact: true })).toBeVisible();
});

test('the bell says how many things wait and opens the same surface from Home', async ({ page }) => {
  // The bell is the shared waiting-count route since U3 (the rail's Review
  // row went with it, after the Home cleanup).
  await page.goto('/?mock=1&demo=1');
  await expect(waitingBell(page)).toHaveAttribute('aria-description', /^Needs you, \d+ waiting\. \d+ unread notifications$/);
  await expect(page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: /^(Review|Needs you)\b/ })).toHaveCount(0);
  await navigateWaiting(page);
  await expect(page.getByRole('region', { name: 'Waiting on you' })).toBeVisible();
  // The old Home list is gone: one surface, not a second list.
  await expect(page.locator('.home-needs')).toHaveCount(0);
});

test('Show more pages past the first page', async ({ page }) => {
  await page.addInitScript(() => {
    if (localStorage.getItem('mockNeedsYouExtra')) return;
    const at = (i: number) => new Date(Date.UTC(2026, 9, 3, 8, 0, 0) - i * 60_000).toISOString();
    localStorage.setItem('mockNeedsYouExtra', JSON.stringify(Array.from({ length: 40 }, (_, i) => ({
      id: `conflict:page-${i}:status`, kind: 'conflict', sourceId: `page-${i}`, title: `Conflict ${String(i).padStart(2, '0')}`,
      detail: 'status changed in both Hollow and Notion', state: 'conflict', at: at(i), workspaceId: null, sessionKey: null,
      path: `tasks/conflict-${i}.md`, category: 'task', link: null,
    }))));
  });
  await page.goto('/?mock=1');
  const panel = await openWaiting(page);
  const conflicts = panel.getByRole('region', { name: 'Sync conflicts' });
  await expect(conflicts).toContainText('Conflict 00');
  await expect(conflicts).not.toContainText('Conflict 39');
  const more = panel.getByRole('button', { name: /^Show \d+ more$/ });
  for (let i = 0; i < 5 && await more.count(); i++) await more.click();
  await expect(conflicts).toContainText('Conflict 39');
  await expect(more).toHaveCount(0);
});

test('a build workspace shows its delivery record, step by step', async ({ page }) => {
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Fix the About page layout.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('About page');
  await page.locator('.project-create-options > summary').click();
  await page.getByRole('radio', { name: 'Repository worktree' }).click();
  await choose(page.getByRole('combobox', { name: 'Repository' }), '/Users/fixture/Projects/hollow');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-branch')).toHaveText('hollow/about-page');
  await openChanges(page, 'hollow/about-page');
  const record = page.getByRole('list', { name: 'Delivery record' });
  await expect(record).toBeVisible();
  await expect(record.locator('li[data-state="done"]').filter({ hasText: 'Committed' })).toBeVisible();
  await expect(record.locator('li').filter({ hasText: 'Pull request open' })).toHaveAttribute('data-state', 'pending');
  await expect(record.locator('li').filter({ hasText: 'Merged' })).toHaveAttribute('data-state', 'pending');
});
