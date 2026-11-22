import { openTasks } from './choose';
import { expect, test, type Locator, type Page } from './fixtures/test';

/** Open the editor's More details section; it starts open for a task that
 *  already has steps, labels or blockers, so a bare click would close it. */
async function openMore(sheet: Locator) {
  const more = sheet.locator('details.task-more');
  if (!(await more.evaluate(el => (el as HTMLDetailsElement).open))) await more.locator('summary').click();
}


/**
 * Paperclip PR 11b: task blockers.
 *
 * - (The row's `Blocked · n` chip and the Blocked scope went with the
 *   issue-list rewrite, 11-10.)
 * - The Tasks pane lists Blocked by with the never-resolve warnings
 *   (a missing link, a recurring blocker) and Blocks, beside the read-only
 *   Done when / Activity disclosures.
 * - The pane's picker and TaskModal's picker send `blockedBy` through
 *   `update_task` with the loaded contentHash; TaskModal's candidates leave
 *   out a task that would close a loop and mark a recurring one.
 * - A backend refusal (`TASK_CYCLE: …`, thrown as a plain STRING like Tauri)
 *   shows without its prefix.
 *
 * Like task-edit.spec.ts, the served mock module is extended at the end so
 * `mockInvoke` (captured by `src/api.ts`) records every `update_task` input.
 */
/** A local task's title button in the Tasks list (issue-list rewrite, 11-10). */
const openButton = (page: Page, title: string) =>
  page.locator('.issue-group[data-kind="local"] .issue-row.task-row').filter({ hasText: title }).locator('.issue-row-main');

async function harness(page: Page, options: { cycle?: boolean; bridge?: boolean } = {}) {
  await page.route('**/src/mockApi.ts*', async route => {
    const response = await route.fetch();
    const original = await response.text();
    if (!original.includes('export async function mockInvoke(')) {
      throw new Error('task-blockers harness: mockInvoke declaration not found in the served module');
    }
    const body = `${original}
// ---- injected by tests/smoke/task-blockers.spec.ts ----
{
  const options = ${JSON.stringify(options)};
  const weekly = 'vault://tasks/weekly-review.md';
  const gone = 'vault:tasks/gone.md';
  const extra = (url, title, hash, over) => ({
    ...tasks[1], url, title, vaultPath: 'tasks/' + title + '.md', contentHash: hash, subtasks: [],
    doneWhen: null, activity: null, blockedBy: [], blocks: [], ready: true, unresolvedBlockers: [], blockerWarnings: [], ...over,
  });
  tasks.push(extra(weekly, 'Weekly review', 'mock-sha-weekly', { recurrence: 'every week', blocks: [tasks[0].url] }));
  tasks.push(extra('vault://tasks/draft-roadmap.md', 'Draft the roadmap', 'mock-sha-roadmap', {}));
  tasks[0].blockedBy = [tasks[1].url, weekly, gone];
  tasks[0].unresolvedBlockers = [tasks[1].url, weekly, gone];
  tasks[0].blockerWarnings = [{ url: weekly, kind: 'recurring' }, { url: gone, kind: 'missing' }];
  tasks[0].ready = false;
  tasks[1].blocks = [tasks[0].url];
  window.__taskUpdates = [];
  // An edit made outside Hollow (Obsidian) while the picker is open.
  window.__externalBlockerEdit = () => {
    tasks[0].blockedBy = [...tasks[0].blockedBy, 'vault://tasks/draft-roadmap.md'];
    tasks[0].contentHash = 'mock-sha-external-edit';
  };
  const raw = mockInvoke;
  mockInvoke = async (command, args) => {
    if (command === 'update_task') {
      window.__taskUpdates.push(structuredClone(args.input));
      if (options.cycle) throw 'TASK_CYCLE: Finish artifact safety defaults → Draft the roadmap → Finish artifact safety defaults. Nothing was saved; a task can’t wait on work that waits on it.';
    }
    return raw(command, args);
  };
}
`;
    await route.fulfill({ response, body });
  });
  if (options.bridge) await installEventBridge(page);
  await page.goto('/?mock=1');
  await openTasks(page);
  await expect(page.getByRole('heading', { name: 'Tasks', exact: true })).toBeVisible();
  await expect(openButton(page, 'Finish artifact safety defaults')).toBeVisible();
}

/** Stand-in for Tauri's event bridge (as in workspace-only.spec.ts), so a
 *  test can deliver `vault-changed` to the app's own listener. */
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

const updates = (page: Page) => page.evaluate(() => (window as any).__taskUpdates as Record<string, unknown>[]);
const FIRST = 'vault://tasks/finish-artifact-safety.md';

test('blockers show never-resolve warnings and the pickers send blockedBy', async ({ page }) => {
  await harness(page);
  await openButton(page, 'Finish artifact safety defaults').click();
  const pane = page.locator('.task-detail-blockers');
  await expect(pane.getByRole('heading', { name: 'Blocked by' })).toBeVisible();
  await expect(pane.locator('.task-blocker-list li')).toHaveCount(3);
  await expect(pane).toContainText('Sketch the 3.0 daily cockpit');
  await expect(pane).toContainText('Weekly review repeats');
  await expect(pane).toContainText('No task at tasks/gone.md');
  // The read-only reserved sections sit in the same pane.
  await expect(page.locator('.task-detail-reserved summary', { hasText: 'Done when' })).toBeVisible();
  await expect(page.locator('.task-detail-reserved summary', { hasText: /^Log$/ })).toBeVisible();

  await pane.getByRole('button', { name: 'Edit blockers' }).click();
  await pane.getByRole('searchbox', { name: 'Find a task this waits on' }).fill('roadmap');
  await pane.getByRole('button', { name: /Draft the roadmap/ }).click();
  await expect(pane.locator('.blocked-by-chip')).toHaveCount(4);
  await pane.getByRole('button', { name: 'Save blockers' }).click();
  await expect.poll(async () => (await updates(page)).length).toBe(1);
  expect((await updates(page))[0]).toEqual({
    url: FIRST,
    expected: 'mock-sha-finish-artifact-safety',
    blockedBy: ['vault://tasks/rebuild-today.md', 'vault://tasks/weekly-review.md', 'vault:tasks/gone.md', 'vault://tasks/draft-roadmap.md'],
  });

  // The blocker lists what it blocks. (Close first: a narrow window shows
  // either the list or the pane.)
  await page.getByRole('button', { name: 'Close task details' }).click();
  await openButton(page, 'Sketch the 3.0 daily cockpit').click();
  await expect(page.locator('.task-detail-blockers').getByRole('heading', { name: 'Blocks' })).toBeVisible();
  await expect(page.locator('.task-detail-blockers')).toContainText('Finish artifact safety defaults');
});

test('TaskModal picker leaves out a loop, marks a recurring task and sends blockedBy', async ({ page }) => {
  await harness(page);
  await page.keyboard.press('ControlOrMeta+k');
  await page.locator('.palette-input').fill('Sketch the 3.0 daily cockpit');
  await page.locator('.palette-row', { hasText: 'Sketch the 3.0 daily cockpit' }).first().click();
  const sheet = page.locator('.task-editor');
  await expect(sheet).toBeVisible();
  await openMore(sheet);
  const search = sheet.getByRole('searchbox', { name: 'Find a task this waits on' });
  // Finish artifact safety defaults waits on this task: offering it would close a loop.
  await search.fill('finish');
  await expect(search).toHaveValue('finish');
  await expect(sheet.locator('.blocked-by-options')).toContainText('would create a loop');
  await search.fill('weekly');
  const option = sheet.getByRole('button', { name: /Weekly review/ });
  await expect(option).toContainText('repeats');
  await option.click();
  await expect(sheet.locator('.blocked-by-chip')).toHaveText(['Weekly review×']);
  await sheet.getByRole('button', { name: 'Save changes', exact: true }).click();
  await expect(page.locator('.task-editor')).toHaveCount(0);
  expect(await updates(page)).toEqual([
    { url: 'vault://tasks/rebuild-today.md', expected: 'mock-sha-rebuild-today', blockedBy: ['vault://tasks/weekly-review.md'] },
  ]);
});

test('the editor’s first frame never pulls focus out of a field already in use', async ({ page }) => {
  // C6 P3, the root of this file's flake under load: TaskEditor focuses its
  // title one frame after it mounts. On a loaded machine that frame can land
  // between Playwright focusing the blocker search and typing into it, so
  // "finish" went into the title and the search's options never showed. Here
  // the editor's frames are held until the search has focus, then run.
  await page.addInitScript(() => {
    const w = window as unknown as { __holdFrames: boolean; __heldFrames: FrameRequestCallback[]; __flushFrames: () => void };
    const raf = window.requestAnimationFrame.bind(window);
    w.__holdFrames = false;
    w.__heldFrames = [];
    window.requestAnimationFrame = cb => { if (!w.__holdFrames) return raf(cb); w.__heldFrames.push(cb); return 0; };
    w.__flushFrames = () => { w.__holdFrames = false; for (const cb of w.__heldFrames.splice(0)) cb(performance.now()); };
  });
  await harness(page);
  await page.keyboard.press('ControlOrMeta+k');
  await page.locator('.palette-input').fill('Sketch the 3.0 daily cockpit');
  await page.evaluate(() => { (window as unknown as { __holdFrames: boolean }).__holdFrames = true; });
  await page.locator('.palette-row', { hasText: 'Sketch the 3.0 daily cockpit' }).first().click();
  const sheet = page.locator('.task-editor');
  await expect(sheet).toBeVisible();
  await openMore(sheet);
  const search = sheet.getByRole('searchbox', { name: 'Find a task this waits on' });
  const title = sheet.locator('textarea').first();
  const titleBefore = await title.inputValue();
  await search.focus();
  await page.evaluate(() => (window as unknown as { __flushFrames: () => void }).__flushFrames());
  await expect(search).toBeFocused();
  await page.keyboard.type('finish');
  await expect(search).toHaveValue('finish');
  await expect(title).toHaveValue(titleBefore);
  await expect(sheet.locator('.blocked-by-options')).toContainText('would create a loop');
});

test('a refused blocker edit keeps the picker open and shows the reason without its code', async ({ page }) => {
  await harness(page, { cycle: true });
  await openButton(page, 'Finish artifact safety defaults').click();
  const pane = page.locator('.task-detail-blockers');
  await pane.getByRole('button', { name: 'Edit blockers' }).click();
  await pane.getByRole('searchbox', { name: 'Find a task this waits on' }).fill('roadmap');
  await pane.getByRole('button', { name: /Draft the roadmap/ }).click();
  await pane.getByRole('button', { name: 'Save blockers' }).click();
  const alert = pane.getByRole('alert');
  await expect(alert).toContainText('Finish artifact safety defaults → Draft the roadmap');
  await expect(alert).not.toContainText('TASK_CYCLE');
  await expect(pane.getByRole('button', { name: 'Save blockers' })).toBeVisible();
});

test('the pane picker compares against the hash from when editing began', async ({ page }) => {
  await harness(page, { bridge: true });
  await openButton(page, 'Finish artifact safety defaults').click();
  const pane = page.locator('.task-detail-blockers');
  await pane.getByRole('button', { name: 'Edit blockers' }).click();
  await expect(pane.locator('.blocked-by-chip')).toHaveCount(3);
  // Obsidian edits the task while the picker is open, and the watcher refetches.
  const before = await page.evaluate(() => window.__mockApiCalls?.tasks ?? 0);
  await page.evaluate(() => {
    const w = window as unknown as { __externalBlockerEdit: () => void; __emitTauriEvent: (name: string, payload: unknown) => void };
    w.__externalBlockerEdit();
    w.__emitTauriEvent('vault-changed', { folders: ['tasks'], paths: ['tasks/finish-artifact-safety.md'] });
  });
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.tasks ?? 0)).toBeGreaterThan(before);
  await pane.getByRole('button', { name: 'Save blockers' }).click();
  await expect.poll(async () => (await updates(page)).length).toBe(1);
  // The save carried the baseline it was opened with, so the external edit wins a conflict...
  expect((await updates(page))[0].expected).toBe('mock-sha-finish-artifact-safety');
  const alert = pane.getByRole('alert');
  await expect(alert).toContainText('changed on disk');
  await expect(alert).not.toContainText('TASK_CONFLICT');
  // ...and the draft stays open.
  await expect(pane.getByRole('button', { name: 'Save blockers' })).toBeVisible();
  await expect(pane.locator('.blocked-by-chip')).toHaveCount(3);
});
