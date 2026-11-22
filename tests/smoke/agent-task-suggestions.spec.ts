import { expect, test, type Page } from './fixtures/test';
import { openReviews, railButton, reviewLink, waitingPanel } from './waiting';

// Paperclip PR 12: a task an agent proposed through hollow_tasks
// (`propose_tasks`) is an `agent_task` review item. It lives with the other
// task suggestions in Tasks → Task suggestions, edits like them, and
// accepting it creates the task (the backend's shared idempotent create) and
// refreshes the task list. The older workspace-only suggestion tests are in
// the smoke failure baseline, so this one stands alone.

const AGENT_TASK = {
  id: 'agent-rollout',
  kind: 'agent_task',
  source: 'chat',
  risk: 'local_write',
  status: 'pending',
  createdAt: '2026-09-26T12:00:00Z',
  updatedAt: '2026-09-26T12:00:00Z',
  fingerprint: 'agent-task:0123456789abcdef01234567',
  sourcePath: null,
  snoozedUntil: null,
  error: null,
  payload: {
    title: 'Draft the rollout checklist',
    due: null,
    priority: 'P2',
    note: 'You asked for this twice in the launch chat.',
    projectUrl: null,
    labels: [],
    evidenceExcerpt: 'notes/rollout.md',
    runId: 'run-20260926-1',
    surface: 'chat',
    scope: 's-20260926-0000abcd',
  },
};

async function seed(page: Page) {
  await page.addInitScript((item) => {
    if (localStorage.getItem('mockReviewItems')) return;
    localStorage.setItem('mockReviewItems', JSON.stringify([
      item,
      {
        ...item, id: 'hb-task', kind: 'heartbeat_task', source: 'heartbeat', fingerprint: 'hb-task',
        sourcePath: 'calendar/review.md', payload: { title: 'Refresh the project brief', note: 'Check the meeting decisions.' },
      },
    ]));
  }, AGENT_TASK);
}

const calls = (page: Page, command: string) =>
  page.evaluate((name) => window.__mockApiCalls?.[name] ?? 0, command);

test('an agent-proposed task renders, edits and accepts from Task suggestions', async ({ page }) => {
  await seed(page);
  await page.goto('/?mock=1');

  const panel = await openReviews(page, 'tasks');
  const card = panel.locator('.review-inbox-card').filter({ hasText: 'Draft the rollout checklist' });
  await expect(card).toHaveCount(1);
  await expect(card).toContainText('Agent-proposed task');
  await expect(card).toContainText('Chat');
  await expect(card).toContainText('You asked for this twice in the launch chat.');
  await expect(card.locator('.review-evidence')).toContainText('notes/rollout.md');
  // The heartbeat suggestion shares the filter.
  await expect(panel.locator('.review-inbox-card').filter({ hasText: 'Refresh the project brief' })).toHaveCount(1);

  // The same editor as the other task suggestions.
  await card.getByRole('button', { name: 'Edit', exact: true }).click();
  await expect(card.getByLabel('Task', { exact: true })).toHaveValue('Draft the rollout checklist');
  await expect(card.getByLabel('Priority', { exact: true })).toHaveValue('P2');
  await expect(card.getByLabel('Context', { exact: true })).toBeVisible();
  await card.getByLabel('Due date', { exact: true }).fill('2026-10-02');

  const resolvesBefore = await calls(page, 'resolve_review_item');
  const tasksBefore = await calls(page, 'tasks');
  await card.getByRole('button', { name: 'Accept', exact: true }).click();
  await expect.poll(() => calls(page, 'resolve_review_item')).toBe(resolvesBefore + 1);
  // Accepting creates a task, so the task list refetches.
  await expect.poll(() => calls(page, 'tasks')).toBeGreaterThan(tasksBefore);
  const stored = await page.evaluate(() => {
    const items = JSON.parse(localStorage.getItem('mockReviewItems') ?? '[]') as Array<{ id: string; status: string; payload: Record<string, unknown> }>;
    return items.find(item => item.id === 'agent-rollout');
  });
  expect(stored?.status).toBe('accepted');
  expect(stored?.payload.due).toBe('2026-10-02');
  await expect(card).toHaveCount(0);

  // It never appears under another page's filter.
  await railButton(page, 'Automations').click();
  await expect(page.getByText('Draft the rollout checklist')).toHaveCount(0);
});

test('a pending agent-proposed task is not offered in Automations or Settings', async ({ page }) => {
  await seed(page);
  await page.goto('/?mock=1');

  await railButton(page, 'Automations').click();
  await expect(page.getByText('Draft the rollout checklist')).toHaveCount(0);

  // Settings → Personalization → Memory links to the learning suggestions.
  const learning = await openReviews(page, 'learning');
  await expect(learning.getByText('Draft the rollout checklist')).toHaveCount(0);
  await railButton(page, 'Projects').click();
  await expect(waitingPanel(page)).toHaveCount(0);

  // Control: the item is still pending under Tasks.
  const panel = await openReviews(page, 'tasks', '2 waiting');
  await expect(panel.locator('.review-inbox-card').filter({ hasText: 'Draft the rollout checklist' })).toHaveCount(1);
});
