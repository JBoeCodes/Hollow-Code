import { expect, test, type Page } from './fixtures/test';
import { goToReviewPage, openReviews, openWaiting, railButton, reviewLink, storedStatus, waitingPanel, leaveReview } from './waiting';

async function seedSuggestions(page: Page) {
  await page.addInitScript(() => {
    if (localStorage.getItem('mockReviewItems')) return;
    const base = {
      kind: 'heartbeat_task', source: 'heartbeat', risk: 'local_write', status: 'pending',
      createdAt: '2026-09-06T12:00:00Z', updatedAt: '2026-09-06T12:00:00Z',
      sourcePath: 'calendar/review.md', snoozedUntil: null, error: null,
    };
    localStorage.setItem('mockReviewItems', JSON.stringify([
      { ...base, id: 'local-task', fingerprint: 'local-task', payload: { title: 'Refresh the project brief', note: 'Check the meeting decisions.' } },
      { ...base, id: 'local-meeting', fingerprint: 'local-meeting', kind: 'meeting_task', source: 'meeting', payload: { title: 'Confirm the meeting checklist', note: 'Recorded next action.', sourcePath: 'calendar/review.md' } },
      { ...base, id: 'local-commitment', fingerprint: 'local-commitment', kind: 'commitment', source: 'chat', payload: { title: 'Confirm the project owner', counterparty: 'Avery', direction: 'owed_by_me', context: 'Confirm ownership at the next review.' } },
      { ...base, id: 'local-project', fingerprint: 'local-project', kind: 'project_activation', source: 'activation', payload: { projectName: 'Payroll reconciliation', currentState: 'Pilot reviewed.', desiredState: 'Approve real export evidence.' } },
      { ...base, id: 'local-goal', fingerprint: 'local-goal', kind: 'goal', source: 'activation', payload: { title: 'Audit readiness', why: 'Close evidence gaps.', successEvidence: 'Signed review.' } },
      { ...base, id: 'local-learning', fingerprint: 'local-learning', kind: 'learning', source: 'chat', payload: { subject: 'Release verification', statement: 'Check the installed application.' } },
      { ...base, id: 'local-permission', fingerprint: 'workflow-permissions:morning-brief:a:b', kind: 'workflow_permissions', source: 'workflow', sourcePath: 'workflows/morning-brief.md', payload: { slug: 'morning-brief', title: 'Morning Brief', summary: 'Permission mode: acceptEdits → bypassPermissions', escalation: true, diff: [] } },
      { ...base, id: 'local-streak', fingerprint: 'workflow-failure-streak:nightly-report:run-3', kind: 'workflow_failure_streak', source: 'workflow', sourcePath: 'workflows/nightly-report.md', payload: { slug: 'nightly-report', title: 'Nightly Report', count: 3, firstFailedAt: '2026-09-06T07:00:00Z', lastError: 'claude exited 1', schedule: '0 0 7 * * * *', triggers: [] } },
      { ...base, id: 'old-send', fingerprint: 'old-send', kind: 'heartbeat_send', risk: 'external_send', payload: { subject: 'Old send' } },
      { ...base, id: 'old-reply', fingerprint: 'old-reply', kind: 'reply_draft', source: 'inbox', payload: { subject: 'Old reply' } },
      { ...base, id: 'old-meeting', fingerprint: 'old-meeting', kind: 'meeting_followup', status: 'accepted', payload: { subject: 'Old meeting follow-up' } },
      { ...base, id: 'old-source', fingerprint: 'old-source', sourcePath: 'emails/retired.md', payload: { title: 'Retired message task' } },
    ]));
  });
}

test('each page links into Waiting on you filtered to its own review items, never legacy messaging', async ({ page }) => {
  await seedSuggestions(page);
  await page.goto('/?mock=1');
  // No page keeps a review list of its own (decision 7).
  await expect(page.locator('.review-inbox')).toHaveCount(0);
  await expect(page.getByLabel('Main navigation').getByRole('button', { name: /Approvals/ })).toHaveCount(0);

  const contexts = [
    { context: 'tasks', titles: ['Refresh the project brief', 'Confirm the meeting checklist', 'Confirm the project owner'] },
    { context: 'automations', titles: ['Morning Brief'] },
    // Paperclip PR 9 (9a-2): failure-streak cards have their own link.
    { context: 'automationHealth', titles: ['Nightly Report'] },
  ] as const;
  for (const entry of contexts) {
    const panel = await openReviews(page, entry.context, `${entry.titles.length} waiting`);
    await expect(panel.locator('.review-inbox-card')).toHaveCount(entry.titles.length);
    for (const title of entry.titles) await expect(panel.locator('.review-inbox-title')).toContainText([title]);
    for (const other of [...contexts.filter(candidate => candidate.context !== entry.context).flatMap(candidate => candidate.titles), 'Release verification']) {
      await expect(panel).not.toContainText(other);
    }
    await expect(panel).not.toContainText(/Old send|Old reply|Old meeting|Retired message task/);
    await leaveReview(page);
  }
  // Settings → Memory's link opens the one Learnings home, Library →
  // Learnings (#1238), listing only the pending learning.
  // Its link counts the profile suggestions that moved into Learnings too (S PR 4).
  const learnings = await openReviews(page, 'learning', '1 learning');
  const pendingLearnings = learnings.locator('.review-card').filter({ has: page.getByRole('group', { name: 'Learning actions' }) });
  await expect(pendingLearnings).toHaveCount(1);
  await expect(pendingLearnings).toContainText('Release verification');
  for (const title of contexts.flatMap(entry => entry.titles)) await expect(learnings).not.toContainText(title);
  await expect(learnings).not.toContainText(/Old send|Old reply|Old meeting|Retired message task/);
  await leaveReview(page);

  // Unfiltered, the bell lists every pending review item, project briefs and
  // goals included (they have no page of their own), and still no legacy
  // messaging.
  const all = await openWaiting(page);
  const reviews = all.getByRole('region', { name: 'Review items and proposals' });
  for (const title of ['Payroll reconciliation', 'Audit readiness', 'Refresh the project brief', 'Release verification', 'Nightly Report']) {
    await expect(reviews).toContainText(title);
  }
  // The learning is listed and counted, as a link to where it is decided.
  await expect(reviews.getByRole('button', { name: /Release verification/ })).toContainText('Decide in Library Learnings');
  await expect(all).not.toContainText(/Old send|Old reply|Old meeting|Retired message task/);
});

test('contextual edits, acceptance, and dismissal persist through the shared review API', async ({ page }) => {
  await seedSuggestions(page);
  await page.goto('/?mock=1');
  let panel = await openReviews(page, 'tasks');
  const task = panel.locator('.review-inbox-card').filter({ hasText: 'Refresh the project brief' });
  await task.getByRole('button', { name: 'Edit', exact: true }).click();
  await task.getByLabel('Task', { exact: true }).fill('Refresh the approved project brief');
  await task.getByLabel('Context', { exact: true }).fill('Use the confirmed meeting decisions.');
  await task.getByRole('button', { name: 'Accept', exact: true }).click();
  await panel.locator('.review-inbox-card').filter({ hasText: 'Confirm the meeting checklist' }).getByRole('button', { name: 'Reject', exact: true }).click();
  // A resolved item leaves the surface.
  await expect(panel.locator('.review-inbox-card')).toHaveCount(1);
  await goToReviewPage(page, 'tasks');
  await expect(reviewLink(page, 'tasks')).toContainText('1 waiting');

  // A learning is decided in Library Learnings (#1238): an edited Save is a
  // one-learning batch through the learning review API.
  panel = await openReviews(page, 'learning');
  const learning = panel.locator('.review-card').filter({ hasText: 'Release verification' }).filter({ has: page.getByRole('group', { name: 'Learning actions' }) });
  await learning.getByText('Edit and source', { exact: true }).click();
  await learning.getByRole('textbox', { name: 'Learning', exact: true }).fill('Verify the installed app against the approved behavior.');
  await learning.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(panel.getByRole('group', { name: 'Learning actions' })).toHaveCount(0);
  // Profile suggestions share Learnings since S PR 4, so the empty line stays
  // hidden while they wait: the decided learning is what must be gone.
  await expect(panel.locator('.review-card').filter({ hasText: 'Verify the installed app' }).filter({ has: page.getByRole('group', { name: 'Learning actions' }) })).toHaveCount(0);
  expect(await page.evaluate(() => window.__mockApiCalls?.resolve_review_item ?? 0)).toBe(2);
  expect(await page.evaluate(() => window.__mockApiCalls?.resolve_learning_proposals_batch ?? 0)).toBe(1);

  await page.reload();
  const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('mockReviewItems') ?? '[]') as Array<{ id: string; status: string; payload: Record<string, unknown> }>);
  const byId = new Map(stored.map(item => [item.id, item]));
  expect(byId.get('local-task')?.status).toBe('accepted');
  expect(byId.get('local-task')?.payload).toMatchObject({ title: 'Refresh the approved project brief', note: 'Use the confirmed meeting decisions.' });
  expect(byId.get('local-meeting')?.status).toBe('rejected');
  expect(byId.get('local-learning')?.status).toBe('accepted');
  expect(byId.get('local-learning')?.payload).toMatchObject({ statement: 'Verify the installed app against the approved behavior.' });
  await goToReviewPage(page, 'tasks');
  await expect(reviewLink(page, 'tasks')).toContainText('1 waiting');
  expect(stored.filter(item => item.id.startsWith('old-')).map(item => item.status)).toEqual(['pending', 'pending', 'accepted', 'pending']);
});

async function seedPermissionReviews(page: Page, refused: string[] = []) {
  await page.addInitScript(({ refused }) => {
    localStorage.setItem('mockRefusedWorkflows', JSON.stringify(refused));
    if (localStorage.getItem('mockReviewItems')) return;
    const base = {
      kind: 'workflow_permissions', source: 'workflow', risk: 'local_write',
      createdAt: '2026-09-25T12:00:00Z', updatedAt: '2026-09-25T12:00:00Z',
      sourcePath: 'workflows/morning-brief.md', snoozedUntil: null,
    };
    const payload = {
      slug: 'morning-brief', title: 'Morning Brief', path: 'workflows/morning-brief.md',
      observedDigest: 'b', approvedDigest: 'a', changedFields: ['permission mode', 'allowed tools'], escalation: true,
      summary: 'Permission mode: acceptEdits → bypassPermissions · Allowed tools +Bash(*)',
      diff: [
        { field: 'permission mode', kind: 'scalar', before: 'acceptEdits', after: 'bypassPermissions' },
        { field: 'allowed tools', kind: 'list', added: ['Read', 'Bash(*)'], removed: [], reordered: false },
      ],
    };
    // One suggestion of another context each, so routing is proven both
    // ways: none of these may appear under Permission changes, and no
    // permission review may appear under Task suggestions.
    const other = { ...base, status: 'pending', error: null, sourcePath: 'calendar/review.md' };
    localStorage.setItem('mockReviewItems', JSON.stringify([
      { ...base, id: 'perm-live', fingerprint: 'workflow-permissions:morning-brief:a:b', status: 'pending', error: null, payload },
      { ...base, id: 'perm-old', fingerprint: 'workflow-permissions:morning-brief:a:c', status: 'rejected', error: 'Superseded: the automation changed again.', payload: { ...payload, summary: 'Prompt changed', diff: [] } },
      { ...other, id: 'other-task', fingerprint: 'other-task', kind: 'heartbeat_task', source: 'heartbeat', payload: { title: 'Refresh the project brief', note: 'Check the meeting decisions.' } },
      { ...other, id: 'other-learning', fingerprint: 'other-learning', kind: 'learning', source: 'chat', sourcePath: null, payload: { subject: 'Release verification', statement: 'Check the installed application.' } },
    ]));
  }, { refused });
}

test('permission changes open from Automations at every width and require confirmation', async ({ page }) => {
  await seedPermissionReviews(page);
  await page.goto('/?mock=1');
  const panel = await openReviews(page, 'automations', '1 waiting');
  // Only permission reviews are shown.
  await expect(panel.locator('.review-inbox-card')).toHaveCount(1);
  await expect(panel).not.toContainText(/Refresh the project brief|Release verification/);
  const card = panel.locator('.review-inbox-card').filter({ hasText: 'Morning Brief' });
  await expect(card).toHaveCount(1);
  await expect(card).toContainText('bypassPermissions');
  await expect(card).toContainText('+Bash(*)');
  await expect(card.getByRole('button', { name: 'Edit', exact: true })).toHaveCount(0);
  await expect(card.getByRole('button', { name: 'Tomorrow', exact: true })).toHaveCount(0);

  // Approval always asks first; an escalation is a danger confirmation.
  await card.getByRole('button', { name: 'Approve…', exact: true }).click();
  let confirm = page.getByRole('dialog', { name: /Approve changes to/ });
  await expect(confirm).toBeVisible();
  await expect(confirm).toHaveClass(/danger/);
  await expect(confirm).toContainText('+Bash(*)');
  await confirm.getByRole('button', { name: 'Keep paused', exact: true }).click();
  await expect(confirm).toHaveCount(0);
  // Answering the confirmation leaves the surface open.
  await expect(panel).toBeVisible();
  expect(await page.evaluate(() => window.__mockApiCalls?.resolve_review_item ?? 0)).toBe(0);
  await expect(card).toHaveCount(1);

  // The `a` key, with the card focused, goes through the same confirmation.
  await card.focus();
  await page.keyboard.press('a');
  confirm = page.getByRole('dialog', { name: /Approve changes to/ });
  await expect(confirm).toBeVisible();
  await confirm.getByRole('button', { name: 'Approve and allow', exact: true }).click();
  await expect(panel.locator('.review-inbox-card')).toHaveCount(0);
  expect(await page.evaluate(() => window.__mockApiCalls?.resolve_review_item ?? 0)).toBe(1);
  expect(await storedStatus(page, 'perm-live')).toBe('accepted');
  // The superseded request stays out of the surface (it waits on no one).
  await expect(panel).not.toContainText('Prompt changed');

  // And no permission review leaks into another page's filter.
  const tasks = await openReviews(page, 'tasks');
  await expect(tasks.locator('.review-inbox-card')).toHaveCount(1);
  await expect(tasks).toContainText('Refresh the project brief');
  await expect(tasks).not.toContainText(/Morning Brief|bypassPermissions/);
});

/** Stand-in for Tauri's event bridge (as in chat-workspace.spec.ts), so
 *  `listen()` registers for real and a test can deliver `vault-changed` to
 *  the app's own listener. Commands still go through the `?mock=1` IPC. */
async function installEventBridge(page: Page) {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (event: unknown) => void>();
    const listeners = new Map<string, number[]>();
    let next = 1;
    const w = window as unknown as Record<string, unknown>;
    w.__TAURI_INTERNALS__ = {
      transformCallback(callback: (event: unknown) => void) {
        const id = next++;
        callbacks.set(id, callback);
        return id;
      },
      invoke(command: string, args: { event?: string; handler?: number }) {
        if (command === 'plugin:event|listen' && args?.event && args.handler) {
          listeners.set(args.event, [...(listeners.get(args.event) ?? []), args.handler]);
          return Promise.resolve(next++);
        }
        if (command === 'plugin:event|unlisten') return Promise.resolve();
        return Promise.reject(new Error(`not in Tauri: ${command}`));
      },
    };
    w.__emitTauriEvent = (event: string, payload: unknown) => {
      for (const id of listeners.get(event) ?? []) callbacks.get(id)?.({ event, id, payload });
    };
  });
}

/** Hold `list_review_inbox` responses for 300ms while `__slowReviewInbox` is
 *  set, so a refetch that re-suspends the page would visibly show the
 *  loading fallback (the plain mock resolves at once). */
async function slowReviewInbox(page: Page) {
  await page.route('**/src/mockApi.ts*', async route => {
    const response = await route.fetch();
    const original = await response.text();
    if (!original.includes('export async function mockInvoke(')) {
      throw new Error('slowReviewInbox: mockInvoke declaration not found in the served module');
    }
    const body = `${original}
// ---- injected by tests/smoke/workspace-only.spec.ts ----
{
  const __rawMockInvoke = mockInvoke;
  mockInvoke = async (command, args) => {
    if (command === 'list_review_inbox' && window.__slowReviewInbox) await new Promise(resolve => setTimeout(resolve, 300));
    return __rawMockInvoke(command, args);
  };
}
`;
    await route.fulfill({ response, body });
  });
}

test('a workflows/ vault change refreshes the review links without blanking the page', async ({ page }) => {
  await installEventBridge(page);
  await slowReviewInbox(page);
  await seedPermissionReviews(page);
  await page.goto('/?mock=1');
  for (const [destination, context] of [['Automations', 'automations'], ['Tasks', 'tasks']] as const) {
    await railButton(page, destination).click();
    await expect(reviewLink(page, context)).toBeAttached();
    await expect(page.locator('.page-loading')).toHaveCount(0);
    const before = await page.evaluate(() => window.__mockApiCalls?.list_review_inbox ?? 0);
    // Record every insertion of the page-level loading fallback from here on.
    await page.evaluate(() => {
      const w = window as unknown as { __sawPageLoading: boolean; __pageLoadingObserver: MutationObserver; __slowReviewInbox: boolean };
      w.__sawPageLoading = false;
      w.__pageLoadingObserver = new MutationObserver(records => {
        for (const record of records) {
          for (const node of Array.from(record.addedNodes)) {
            if (node instanceof Element && (node.matches('.page-loading') || node.querySelector('.page-loading'))) w.__sawPageLoading = true;
          }
        }
      });
      w.__pageLoadingObserver.observe(document.body, { childList: true, subtree: true });
      w.__slowReviewInbox = true;
      (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void })
        .__emitTauriEvent('vault-changed', { folders: ['workflows'], paths: ['workflows/morning-brief.md'] });
    });
    // The burst refreshed the review link's count (an automation run's own status
    // writes land here twice per run)...
    await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.list_review_inbox ?? 0)).toBeGreaterThan(before);
    await page.waitForTimeout(500);
    // ...while the page kept showing its content.
    expect(await page.evaluate(() => (window as unknown as { __sawPageLoading: boolean }).__sawPageLoading)).toBe(false);
    await expect(reviewLink(page, context)).toBeAttached();
    await page.evaluate(() => {
      const w = window as unknown as { __pageLoadingObserver: MutationObserver; __slowReviewInbox: boolean };
      w.__pageLoadingObserver.disconnect();
      w.__slowReviewInbox = false;
    });
  }
});

test('a refused Run now opens Waiting on you on the permission changes', async ({ page }) => {
  await seedPermissionReviews(page, ['morning-brief']);
  await page.goto('/?mock=1');
  await railButton(page, 'Automations').click();
  await expect(reviewLink(page, 'automations')).toBeVisible();
  await expect(waitingPanel(page)).toHaveCount(0);
  const row = page.locator('.workflow-row').filter({ hasText: 'Morning Brief' });
  // A pending review pauses it, so the list never promises a catch-up run.
  await expect(row).toContainText('paused until its settings change is approved');
  await expect(row).not.toContainText('Run now to catch up');
  await row.hover();
  await row.getByRole('button', { name: 'Run Morning Brief now', exact: true }).click();
  // The row's Run now asks first (P0-13).
  await page.getByRole('dialog', { name: 'Run “Morning Brief” now?' }).getByRole('button', { name: 'Run now', exact: true }).click();
  await expect(waitingPanel(page)).toHaveAttribute('data-filter', 'automations');
  await expect(waitingPanel(page).locator('.review-inbox-card').filter({ hasText: 'Morning Brief' })).toHaveCount(1);
  expect(await page.evaluate(() => window.__mockApiCalls?.run_workflow ?? 0)).toBe(1);
});
