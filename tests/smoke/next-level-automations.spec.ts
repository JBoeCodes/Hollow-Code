import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

/** next-level P17: outcome-first automations. The mock's Morning Brief is a
 * daily 7am Claude automation whose latest run failed with a 401 (its stderr
 * tail), whose receipt wrote `briefings/2026-07-07.md`, and whose daily token
 * budget is paused. */

function railButton(page: Page, name: string) {
  return page.getByRole('complementary', { name: 'Main navigation' }).getByRole('button', { name, exact: true });
}

const calls = (page: Page, command: string) => page.evaluate(cmd => window.__mockApiCalls?.[cmd] ?? 0, command);
const misses = (page: Page) => page.evaluate(() => window.__mockApiMisses ?? []);

async function openBrief(page: Page) {
  await page.goto('/?mock=1');
  await railButton(page, 'Automations').click();
  await page.locator('.workflow-row').filter({ hasText: 'Morning Brief' }).click();
  const review = page.locator('.workflow-review');
  await expect(review).toBeVisible();
  return review;
}

/** True when every locator's element comes after the previous one in the DOM. */
async function inOrder(page: Page, selectors: string[]) {
  return page.evaluate(list => {
    const nodes = list.map(s => document.querySelector(s));
    if (nodes.some(n => !n)) return `missing: ${list[nodes.findIndex(n => !n)]}`;
    for (let i = 1; i < nodes.length; i++) {
      if (!(nodes[i - 1]!.compareDocumentPosition(nodes[i]!) & Node.DOCUMENT_POSITION_FOLLOWING)) return `${list[i]} before ${list[i - 1]}`;
    }
    return 'ok';
  }, selectors);
}

test('the run review leads with status, the fix and the latest result; logs wait behind a disclosure', async ({ page }) => {
  const review = await openBrief(page);

  // One status line in the shared vocabulary, one primary action.
  await expect(review.locator('.workflow-status-line .ui-pill').first()).toContainText('Failed');
  await expect(review.locator('.workflow-status-line')).toContainText('Checks failed');
  await expect(review.locator('.btn.primary')).toHaveCount(1);
  await expect(review.locator('.workflow-output-actions').getByRole('button', { name: 'Run now', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Retry now' })).toHaveCount(0);
  await expect(review.locator('.workflow-run-note')).toContainText('Run now is a real run');
  await expect(review.locator('.workflow-run-note')).toContainText('Today: 3 runs');

  // The actionable reason, with its one fix.
  const fix = review.locator('.workflow-fix');
  await expect(fix).toContainText('could not sign in');
  await expect(fix.getByRole('button', { name: 'Open Accounts' })).toBeVisible();

  // The latest useful result, named by path, before history and logs.
  const latest = review.locator('.workflow-latest');
  await expect(latest.getByRole('heading', { name: 'Latest result' })).toBeVisible();
  await expect(latest.locator('.workflow-latest-caption')).toContainText('briefings/2026-07-07.md');
  await expect(latest.locator('.viewer')).toContainText('Fixture vault note');
  expect(await inOrder(page, ['.workflow-status-line', '.workflow-fix', '.workflow-latest', '.workflow-history-section', '.workflow-logs'])).toBe('ok');

  // Logs on demand: stderr is not shown until the disclosure opens.
  const logs = review.locator('details.workflow-logs');
  expect(await logs.evaluate(el => (el as HTMLDetailsElement).open)).toBe(false);
  await expect(logs.locator('.workflow-stderr-tail').first()).toBeHidden();
  await logs.locator('summary').first().click();
  await expect(logs.locator('.workflow-stderr-tail').first()).toContainText('401 Invalid authentication');
  expect(await misses(page)).toEqual([]);
});

test('Preview configuration describes the run and calls no run path; Run now is the real run', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockConnectors', JSON.stringify([{
    id: 'linear', name: 'Linear', description: '', transport: 'http', command: '', args: [], url: 'https://mcp.linear.app/mcp',
    serverId: '', iconUrl: '', envKeys: [], headerKeys: [], agents: ['claude'], chat: true, automations: true, mode: 'always',
    autoApprove: false, enabled: true, origin: 'hollow', updatedAt: '2026-10-01T00:00:00Z',
  }])));
  const review = await openBrief(page);
  await review.getByRole('button', { name: 'Automation options' }).click();
  await page.getByRole('menuitem', { name: 'Preview configuration' }).click();
  const preview = page.locator('.workflow-preview');
  await expect(preview).toBeVisible();
  await expect(preview.getByRole('heading', { name: 'Preview · nothing runs' })).toBeVisible();
  await expect(preview.locator('[data-group="who"]')).toContainText('Claude');
  await expect(preview.locator('[data-group="where"]')).toContainText('briefs/*.html');
  await expect(preview.locator('[data-group="effects"]')).toContainText('Linear');
  await expect(preview.locator('[data-group="limits"]')).toContainText('the “morning-brief” automation');
  await expect(preview.locator('[data-group="when"] dt')).toHaveText(['Next run', 'Then', 'Then']);
  await expect(preview.locator('[data-group="when"]')).toContainText('this Mac’s time zone');
  await expect.poll(() => calls(page, 'list_connectors')).toBeGreaterThan(0);
  // The preview read data; it started nothing.
  expect(await calls(page, 'run_workflow')).toBe(0);
  expect(await calls(page, 'cancel_workflow')).toBe(0);

  await preview.getByRole('button', { name: 'Close preview' }).click();
  await expect(preview).toHaveCount(0);
  expect(await calls(page, 'run_workflow')).toBe(0);

  // The real run is a different, explicit action.
  await review.locator('.workflow-output-actions').getByRole('button', { name: 'Run now', exact: true }).click();
  await expect.poll(() => calls(page, 'run_workflow')).toBe(1);
  expect(await misses(page)).toEqual([]);
});

test('the editor is outcome-first, shows the next three runs and re-reviews widened access', async ({ page }) => {
  const review = await openBrief(page);
  await review.getByRole('button', { name: 'Automation options' }).click();
  await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
  const editor = page.locator('.workflow-detail');
  for (const name of ['Purpose', 'Inputs & scope', 'Schedule', 'Destination', 'Allowed effects', 'Budget']) {
    await expect(editor.getByRole('heading', { name, exact: true })).toBeVisible();
  }
  // Existing hooks survive the reorganisation.
  await expect(page.locator('.workflow-description-input')).toHaveValue('Daily operating brief.');
  await expect(page.getByRole('combobox', { name: 'Automation notifications' })).toHaveAttribute('data-value', 'always');

  // Next three runs with the zone and the missed-run rule.
  const next = editor.locator('.workflow-next-runs');
  await expect(next.locator('.workflow-next-list li')).toHaveCount(3);
  await expect(next).toContainText('this Mac’s time zone');
  await expect(next).toContainText('within 2 hours of the time');

  // Budget: the automation's own paused budget applies.
  await expect(editor.locator('.workflow-budget-list')).toContainText('the “morning-brief” automation');

  // A narrowing edit is listed; Run now waits for a save.
  const approval = editor.locator('.workflow-approval');
  await expect(approval).toContainText('No unsaved changes');
  // Tool lists sit under a labelled Advanced fold (consolidation H).
  await editor.getByText('Advanced: exactly which tools it may use').click();
  await editor.getByRole('textbox', { name: 'Disallowed tools' }).fill('Bash');
  await expect(approval).toContainText('Saving approves these changes:');
  await expect(approval).toContainText('Disallowed tools: +Bash');
  await expect(editor.locator('.workflow-detail-actions').getByRole('button', { name: 'Run now', exact: true })).toBeDisabled();

  // A widening edit (a new working folder) is confirmed at save; cancel writes nothing.
  await editor.getByRole('textbox', { name: 'Working folder' }).fill('/Users/fixture/elsewhere');
  await expect(approval).toContainText('Saving approves wider access:');
  await expect(approval).toContainText('Working folder: the vault → /Users/fixture/elsewhere');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  const dialog = page.locator('.confirm-dialog');
  await expect(dialog).toContainText('Approve wider access');
  await expect(dialog).toContainText('Working folder: the vault → /Users/fixture/elsewhere');
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  expect(await calls(page, 'write_workflow')).toBe(0);
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await page.locator('.confirm-dialog').getByRole('button', { name: 'Save and approve' }).click();
  await expect.poll(() => calls(page, 'write_workflow')).toBe(1);

  // Switching to Manual: nothing scheduled, nothing to miss.
  await editor.getByRole('button', { name: 'Manual', exact: true }).click();
  await expect(next.locator('.workflow-next-list li')).toHaveCount(0);
  await expect(next).toContainText('Runs only when you start it');
  expect(await calls(page, 'run_workflow')).toBe(0);
  expect(await misses(page)).toEqual([]);
});

test('the editor preview reflects unsaved changes and still runs nothing', async ({ page }) => {
  const review = await openBrief(page);
  await review.getByRole('button', { name: 'Automation options' }).click();
  await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
  const editor = page.locator('.workflow-detail');
  await choose(editor.getByRole('combobox', { name: 'Permission mode' }), 'plan');
  await editor.getByRole('button', { name: 'Preview configuration' }).click();
  const preview = editor.locator('.workflow-preview');
  await expect(preview.locator('[data-group="effects"]')).toContainText('tasks (read only)');
  await expect(preview.locator('[data-group="gate"]')).toContainText('Unsaved changes');
  await expect(preview.locator('[data-group="gate"]')).toContainText('Permission mode: default → plan');
  expect(await calls(page, 'run_workflow')).toBe(0);
  expect(await misses(page)).toEqual([]);
});

test('a paused automation names the review it waits for as its one fix', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('mockRefusedWorkflows', JSON.stringify(['morning-brief']));
    localStorage.setItem('mockReviewItems', JSON.stringify([{
      id: 'perm-morning', kind: 'workflow_permissions', source: 'workflow', risk: 'local_write', status: 'pending',
      createdAt: '2026-09-25T12:00:00Z', updatedAt: '2026-09-25T12:00:00Z', sourcePath: 'workflows/morning-brief.md',
      snoozedUntil: null, error: null, fingerprint: 'workflow-permissions:morning-brief:a:b',
      payload: {
        slug: 'morning-brief', title: 'Morning Brief', path: 'workflows/morning-brief.md', observedDigest: 'b', approvedDigest: 'a',
        changedFields: ['permission mode'], escalation: true, summary: 'Permission mode: acceptEdits → bypassPermissions',
        diff: [{ field: 'permission mode', kind: 'scalar', before: 'acceptEdits', after: 'bypassPermissions' }],
      },
    }]));
  });
  const review = await openBrief(page);
  const fix = review.locator('.workflow-fix');
  await expect(fix).toContainText('Paused');
  await expect(fix).toContainText('settings changed outside the automation editor');
  await fix.getByRole('button', { name: 'Review permission changes' }).click();
  // The fix opens Waiting on you on the permission changes (decision 7).
  const panel = page.getByRole('region', { name: 'Waiting on you' });
  await expect(panel).toHaveAttribute('data-filter', 'automations');
  await expect(panel.locator('.review-inbox-card').filter({ hasText: 'Morning Brief' })).toHaveCount(1);
  expect(await calls(page, 'run_workflow')).toBe(0);
});
