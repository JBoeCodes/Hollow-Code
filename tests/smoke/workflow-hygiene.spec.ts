import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';
import { openReviews, reviewLink, goToReviewPage } from './waiting';

/** Paperclip PR 9 (9a-1). The editor's save seam behaves like the backend's
 * `upsert_input`: it records every `write_workflow` input and refuses one
 * whose `editBase` is not the current baseline. Every accepted save moves the
 * baseline (the mock `read_workflow` derives it from `mockWorkflowSaves`), so
 * an editor that keeps sending the base it loaded before its last save is
 * caught on the second save. */
async function installWorkflowSaveSeam(page: Page) {
  await page.route('**/src/mockApi.ts*', async route => {
    const response = await route.fetch();
    const original = await response.text();
    if (!original.includes('export async function mockInvoke(')) throw new Error('Workflow fixture seam missing');
    await route.fulfill({ response, body: `${original}
{
  const baseWorkflowMock = mockInvoke;
  mockInvoke = async (command, args) => {
    if (command === 'write_workflow') {
      const saves = Number(localStorage.getItem('mockWorkflowSaves') ?? '0');
      const inputs = JSON.parse(localStorage.getItem('mockWorkflowInputs') ?? '[]');
      inputs.push(args.input);
      localStorage.setItem('mockWorkflowInputs', JSON.stringify(inputs));
      if (args.input.editBase !== 'mock-edit-base-' + saves) {
        throw 'WORKFLOW_EDIT_CONFLICT: This automation changed outside the editor since you opened it, so nothing was saved. Your edits are still in the form: copy what you want to keep, then reopen the automation, which loads the current file and discards the unsaved edits.';
      }
      localStorage.setItem('mockWorkflowSaves', String(saves + 1));
      const detail = await baseWorkflowMock('read_workflow', { slug: args.originalSlug });
      return { ...detail.workflow, ...args.input };
    }
    return baseWorkflowMock(command, args);
  };
}
` });
  });
}

function railButton(page: Page, name: string) {
  return page.getByRole('complementary', { name: 'Main navigation' }).getByRole('button', { name, exact: true });
}

async function openEditor(page: Page) {
  await page.goto('/?mock=1');
  await railButton(page, 'Automations').click();
  await page.locator('.workflow-row').filter({ hasText: 'Morning Brief' }).click();
  await page.getByRole('button', { name: 'Automation options' }).click();
  await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
  const description = page.locator('.workflow-description-input');
  await expect(description).toHaveValue('Daily operating brief.');
  return description;
}

const saves = (page: Page) => page.evaluate(() => localStorage.getItem('mockWorkflowSaves'));
const inputs = (page: Page) =>
  page.evaluate(() => JSON.parse(localStorage.getItem('mockWorkflowInputs') ?? '[]') as { editBase?: string; description: string }[]);

test('two consecutive automation saves each send the current edit base', async ({ page }) => {
  await installWorkflowSaveSeam(page);
  const description = await openEditor(page);

  await description.fill('First edit.');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect.poll(() => saves(page)).toBe('1');
  // The in-place save reloads the detail (and with it the baseline); the
  // mock returns the fixture again, so the form shows its description.
  await expect(description).toHaveValue('Daily operating brief.');

  await description.fill('Second edit.');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect.poll(() => saves(page)).toBe('2');

  const sent = await inputs(page);
  expect(sent.map(input => input.editBase)).toEqual(['mock-edit-base-0', 'mock-edit-base-1']);
  expect(sent.map(input => input.description)).toEqual(['First edit.', 'Second edit.']);
  await expect(page.getByText(/changed outside the editor/)).toHaveCount(0);
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});

test('a save refused as a conflict keeps the draft and says to reopen', async ({ page }) => {
  await installWorkflowSaveSeam(page);
  const description = await openEditor(page);
  // The file changes on disk after the editor loaded it.
  await page.evaluate(() => localStorage.setItem('mockWorkflowSaves', '5'));

  await description.fill('My unsaved edit.');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByText(/changed outside the editor since you opened it/).first()).toBeVisible();
  await expect(page.getByText(/WORKFLOW_EDIT_CONFLICT/)).toHaveCount(0);
  await expect(description).toHaveValue('My unsaved edit.');
  expect(await saves(page)).toBe('5');
  expect((await inputs(page)).map(input => input.editBase)).toEqual(['mock-edit-base-0']);
});

/** Paperclip PR 9 (9a-2): the notify select sends `notify` only when the
 * user changed it, so an unrelated save never rewrites the key. */
test('the notifications select is sent only when it changed', async ({ page }) => {
  await installWorkflowSaveSeam(page);
  const description = await openEditor(page);
  const select = page.getByRole('combobox', { name: 'Automation notifications' });
  await expect(select).toHaveAttribute('data-value', 'always');

  await description.fill('An unrelated edit.');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect.poll(() => saves(page)).toBe('1');

  await choose(select, 'failures');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect.poll(() => saves(page)).toBe('2');

  const sent = await page.evaluate(() => JSON.parse(localStorage.getItem('mockWorkflowInputs') ?? '[]') as { notify?: string }[]);
  expect(sent.map(input => input.notify ?? null)).toEqual([null, 'failures']);
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});

async function seedStreak(page: Page, schedule = '0 0 7 * * * *') {
  await page.addInitScript(({ schedule }) => {
    if (localStorage.getItem('mockReviewItems')) return;
    localStorage.setItem('mockReviewItems', JSON.stringify([{
      id: 'streak-morning', kind: 'workflow_failure_streak', source: 'workflow', risk: 'local_write',
      status: 'pending', createdAt: '2026-09-25T12:00:00Z', updatedAt: '2026-09-25T12:00:00Z',
      sourcePath: 'workflows/morning-brief.md', snoozedUntil: null, error: null,
      fingerprint: 'workflow-failure-streak:morning-brief:run-3',
      payload: {
        slug: 'morning-brief', title: 'Morning Brief', count: 3, firstFailedAt: '2026-09-25T07:00:00Z',
        lastError: 'claude exited 1', schedule, triggers: [],
      },
    }]));
  }, { schedule });
}

test('a failing automation offers Pause schedule and Dismiss from Automations', async ({ page }) => {
  await seedStreak(page);
  await page.goto('/?mock=1');
  await goToReviewPage(page, 'automationHealth');
  // Only the kinds that wait get a link (no permission review is pending).
  await expect(reviewLink(page, 'automations')).toHaveCount(0);
  await expect(reviewLink(page, 'automationHealth')).toContainText('Failing automations');
  const panel = await openReviews(page, 'automationHealth', '1 waiting');
  const card = panel.locator('.review-inbox-card').filter({ hasText: 'Morning Brief' });
  await expect(card).toContainText('Failed 3 times in a row · claude exited 1');
  await expect(card.getByRole('button', { name: 'Edit', exact: true })).toHaveCount(0);
  await expect(card.getByRole('button', { name: 'Tomorrow', exact: true })).toHaveCount(1);
  await expect(card.getByRole('button', { name: 'Dismiss', exact: true })).toHaveCount(1);

  await card.getByRole('button', { name: 'Pause schedule', exact: true }).click();
  await expect(card).toHaveCount(0);
  expect(await page.evaluate(() => window.__mockApiCalls?.resolve_review_item ?? 0)).toBe(1);
  const resolved = await page.evaluate(() =>
    (JSON.parse(localStorage.getItem('mockReviewItems') ?? '[]') as { id: string; status: string }[])
      .find(item => item.id === 'streak-morning')?.status);
  expect(resolved).toBe('accepted');
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});

/** Paperclip PR 9 (9a-2): a streak card shows only under Automations →
 * Failing automations, and no other kind shows there. Project briefs have no
 * page link; they wait in the unfiltered surface. */
test('failure-streak cards route only to Failing automations', async ({ page }) => {
  await page.addInitScript(() => {
    if (localStorage.getItem('mockReviewItems')) return;
    const base = {
      source: 'workflow', risk: 'local_write', status: 'pending',
      createdAt: '2026-09-25T12:00:00Z', updatedAt: '2026-09-25T12:00:00Z', snoozedUntil: null, error: null,
    };
    localStorage.setItem('mockReviewItems', JSON.stringify([
      { ...base, id: 'streak-nightly', kind: 'workflow_failure_streak', sourcePath: 'workflows/nightly-report.md',
        fingerprint: 'workflow-failure-streak:nightly-report:run-3',
        payload: { slug: 'nightly-report', title: 'Nightly Report', count: 3, firstFailedAt: '2026-09-25T07:00:00Z', lastError: 'claude exited 1', schedule: '0 0 7 * * * *', triggers: [] } },
      { ...base, id: 'perm-morning', kind: 'workflow_permissions', sourcePath: 'workflows/morning-brief.md',
        fingerprint: 'workflow-permissions:morning-brief:a:b',
        payload: { slug: 'morning-brief', title: 'Morning Brief', summary: 'Permission mode: acceptEdits → bypassPermissions', escalation: true, diff: [] } },
      { ...base, id: 'task-local', kind: 'heartbeat_task', source: 'heartbeat', sourcePath: 'calendar/review.md',
        fingerprint: 'task-local', payload: { title: 'Refresh the project brief', note: 'Check it.' } },
      { ...base, id: 'learn-local', kind: 'learning', source: 'chat', sourcePath: null,
        fingerprint: 'learn-local', payload: { subject: 'Release verification', statement: 'Check the installed application.' } },
    ]));
  });
  await page.goto('/?mock=1');
  const health = await openReviews(page, 'automationHealth');
  await expect(health.locator('.review-inbox-card')).toHaveCount(1);
  await expect(health.locator('.review-inbox-card')).toContainText('Nightly Report');
  await expect(health).toContainText('workflows/nightly-report.md');
  await expect(health).not.toContainText(/Morning Brief|Refresh the project brief|Release verification/);

  const permissions = await openReviews(page, 'automations');
  await expect(permissions.locator('.review-inbox-card')).toHaveCount(1);
  await expect(permissions).not.toContainText('Nightly Report');

  const tasks = await openReviews(page, 'tasks');
  await expect(tasks.locator('.review-inbox-card')).toHaveCount(1);
  await expect(tasks).not.toContainText('Nightly Report');

  // Settings → Memory opens the one Learnings home, Library → Learnings
  // (#1238): the pending learning is its only row, and no streak shows there.
  const learning = await openReviews(page, 'learning');
  const pendingLearnings = learning.locator('.review-card').filter({ has: page.getByRole('group', { name: 'Learning actions' }) });
  await expect(pendingLearnings).toHaveCount(1);
  await expect(pendingLearnings).toContainText('Release verification');
  await expect(learning).not.toContainText('Nightly Report');
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});

/** Merged plan slice 4 (A·D3): a finished run refetches the detail, and that
 * refetch used to re-hydrate every field and clear the run log, wiping an
 * unsaved edit. Events go through a stand-in for Tauri's event bridge (as in
 * run-activity.spec.ts); no agent is involved. */
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

function emit(page: Page, name: string, payload: Record<string, unknown>) {
  return page.evaluate(([event, body]) => {
    (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void })
      .__emitTauriEvent(event as string, body);
  }, [name, payload] as const);
}

const readWorkflowCalls = (page: Page) => page.evaluate(() => window.__mockApiCalls?.read_workflow ?? 0);

async function finishRun(page: Page) {
  const before = await readWorkflowCalls(page);
  await emit(page, 'workflow-event', {
    type: 'started', slug: 'morning-brief', at: '2026-10-02T09:00:00Z', runId: 'r-form', queueId: 21, recall: [],
    source: 'manual', reason: 'run now',
  });
  await emit(page, 'workflow-event', {
    type: 'done', slug: 'morning-brief', runId: 'r-form', queueId: 21, status: 'success', output: '',
    durationSec: 3, error: null, nextRunAt: null, receipt: null, evidenceError: null,
  });
  // The pane refetches its detail when the run ends.
  await expect.poll(() => readWorkflowCalls(page)).toBeGreaterThan(before);
}

test('a run finishing keeps unsaved edits in the automation form', async ({ page }) => {
  await installEventBridge(page);
  await installWorkflowSaveSeam(page);
  const description = await openEditor(page);
  const title = page.locator('.workflow-title-input');

  await description.fill('My unsaved edit.');
  await title.fill('Morning Brief (draft)');
  await finishRun(page);

  await expect(description).toHaveValue('My unsaved edit.');
  await expect(title).toHaveValue('Morning Brief (draft)');
  // The run log is not form state: the finished run's rows stay.
  const runPanel = page.locator('.workflow-run-panel');
  await expect(runPanel.locator('.workflow-run-row.kind-done')).toContainText('Done: 3s');

  // The kept edit saves against the base the form was loaded from.
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect.poll(() => saves(page)).toBe('1');
  expect((await inputs(page)).map(input => input.description)).toEqual(['My unsaved edit.']);
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});

test('a run finishing on a clean automation form keeps its run log', async ({ page }) => {
  await installEventBridge(page);
  const description = await openEditor(page);
  await finishRun(page);

  // A clean form takes the refetched detail; the log of the run stays.
  await expect(description).toHaveValue('Daily operating brief.');
  const runPanel = page.locator('.workflow-run-panel');
  await expect(runPanel.locator('.workflow-run-row.kind-started')).toHaveCount(1);
  await expect(runPanel.locator('.workflow-run-row.kind-done')).toContainText('Done: 3s');
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});
