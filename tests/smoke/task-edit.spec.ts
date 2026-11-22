import { openTasks } from './choose';
import { expect, test, type Locator, type Page } from './fixtures/test';
import { choose } from './choose';

/** Open the editor's More details section; it starts open for a task that
 *  already has steps, labels or blockers, so a bare click would close it. */
async function openMore(sheet: Locator) {
  const more = sheet.locator('details.task-more');
  if (!(await more.evaluate(el => (el as HTMLDetailsElement).open))) await more.locator('summary').click();
}


/**
 * Paperclip PR 3: task edits carry a compare-and-swap baseline and only the
 * fields the user changed.
 *
 * - The shared task editor (sheet and Tasks pane) sends `expected` (the loaded contentHash)
 *   plus exactly the changed fields; an untouched sheet writes nothing.
 * - A tracker-linked task locks its issue-owned fields in TaskModal; a lock
 *   that holds only the project leaves the rest editable. (The older lock
 *   these once covered went with two-way sync, 11-10.)
 * - A status toggle hands back the written hash, so an editor opened before
 *   the list refetch lands does not conflict with the user's own toggle.
 * - Paperclip PR 10: Break down on a linked task sends its stored project
 *   URL (so the backend can add the project's goals), and stops sending it
 *   once the Project field changes.
 * - A stale save keeps the draft and explains the conflict. The conflict is
 *   thrown as a plain STRING because Tauri rejects invoke() that way; an
 *   `e?.message` reader shows "error: failed" there.
 *
 * `mockInvoke` is an exported function declaration, so reassigning it at the
 * end of its own module makes `src/api.ts` (which captures it at module-eval
 * time) use the wrapper. The fixture `tasks` array is in scope there too.
 */
type HarnessOptions = {
  linkSecond?: boolean;
  staleFirst?: boolean;
  isoDueSecond?: boolean;
  conflict?: boolean;
  /** After the first status toggle, hold every `tasks` refetch until
   *  `window.__releaseTasks()`, so only the returned hash can refresh the list. */
  holdTasksAfterToggle?: boolean;
  holdWrite?: boolean;
};

/** A local task's row in the Tasks list (issue-list rewrite, 11-10). */
const localRow = (page: Page, title: string) => page.locator('.issue-group[data-kind="local"] .issue-row.task-row').filter({ hasText: title });

async function harness(page: Page, options: HarnessOptions = {}) {
  await page.route('**/src/mockApi.ts*', async route => {
    const response = await route.fetch();
    const original = await response.text();
    if (!original.includes('export async function mockInvoke(')) {
      throw new Error('task-edit harness: mockInvoke declaration not found in the served module');
    }
    const body = `${original}
// ---- injected by tests/smoke/task-edit.spec.ts ----
{
  const options = ${JSON.stringify(options)};
  if (options.isoDueSecond) { tasks[1].due = '2026-07-23T17:00:00.000-04:00'; tasks[1].dueTime = null; }
  const linkedTo = (kind, fields) => ({provider:'linear',id:'i1',key:'ENG-1',url:null,statusType:null,priority:0,lock:{kind,fields}});
  if (options.linkSecond) tasks[1].tracker = linkedTo('linked',['title','status','priority','due','description','project']);
  if (options.staleFirst) tasks[0].tracker = linkedTo('staleRelations',['project']);
  window.__taskUpdates = [];
  window.__assistInputs = [];
  let holding = false;
  const held = [];
  window.__releaseTasks = () => { holding = false; held.splice(0).forEach(release => release()); };
  window.__releaseWrite = () => {};
  const raw = mockInvoke;
  mockInvoke = async (command, args) => {
    if (options.holdWrite && command === 'set_task_status') await new Promise(resolve => { window.__releaseWrite = resolve; });
    if (holding && command === 'tasks') return new Promise(resolve => held.push(() => resolve(raw(command, args))));
    if (command === 'task_assist') window.__assistInputs.push(structuredClone(args.input));
    if (command === 'update_task') {
      window.__taskUpdates.push(structuredClone(args.input));
      if (options.conflict) throw 'TASK_CONFLICT: tasks/x.md changed on disk after it was opened. Nothing was saved; reopen the task to see the latest version, then reapply your edit.';
    }
    const result = await raw(command, args);
    if (options.holdTasksAfterToggle && command === 'set_task_status') holding = true;
    return result;
  };
}
`;
    await route.fulfill({ response, body });
  });
  await page.goto('/?mock=1');
  await openTasks(page);
  await expect(page.getByRole('heading', { name: 'Tasks', exact: true })).toBeVisible();
  // The palette lists tasks from the same resource the page just loaded.
  // A tracker-linked task (staleFirst) is not a Local row in the list; the
  // palette still opens it, which is what that test uses.
  if (!options.staleFirst) await expect(localRow(page, 'Finish artifact safety defaults')).toBeVisible();
}

async function openFromPalette(page: Page, title: string) {
  await page.keyboard.press('ControlOrMeta+k');
  await page.locator('.palette-input').fill(title);
  await page.locator('.palette-row', { hasText: title }).first().click();
  await expect(page.locator('.task-editor')).toBeVisible();
  return page.locator('.task-editor');
}

const updates = (page: Page) => page.evaluate(() => (window as any).__taskUpdates as Record<string, unknown>[]);

test('P0-22 a local tick succeeds', async ({ page }) => {
  await harness(page);
  await page.getByRole('button', { name: 'Mark Finish artifact safety defaults done', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__mockApiCalls?.set_task_status ?? 0)).toBe(1);
});

test('P0-22 a pending tick leaves other tasks enabled', async ({ page }) => {
  await harness(page, { holdWrite: true });
  await page.getByRole('button', { name: 'Mark Finish artifact safety defaults done', exact: true }).click();
  await expect(page.locator('[data-task-url="vault://tasks/finish-artifact-safety.md"] .issue-row-check')).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Mark Sketch the 3.0 daily cockpit done', exact: true })).toBeEnabled();
  await page.evaluate(() => (window as any).__releaseWrite());
});

test('task editor saves only changed fields with the loaded content hash', async ({ page }) => {
  await harness(page);
  const sheet = await openFromPalette(page, 'Finish artifact safety defaults');
  await sheet.getByRole('radio', { name: 'Medium priority (P2)' }).click();
  await sheet.getByRole('button', { name: 'Save changes', exact: true }).click();
  await expect(page.locator('.task-editor')).toHaveCount(0);
  expect(await updates(page)).toEqual([
    { url: 'vault://tasks/finish-artifact-safety.md', expected: 'mock-sha-finish-artifact-safety', priority: 'P2' },
  ]);
});

test('the Tasks pane edit sends its baseline too', async ({ page }) => {
  await harness(page);
  await localRow(page, 'Finish artifact safety defaults').locator('.issue-row-main').click();
  await page.getByRole('button', { name: 'Edit task', exact: true }).click();
  await page.getByRole('textbox', { name: 'Task name', exact: true }).fill('Finish artifact safety, reviewed');
  await page.getByRole('button', { name: 'Save changes', exact: true }).click();
  await expect.poll(async () => (await updates(page)).length).toBe(1);
  expect((await updates(page))[0]).toEqual({
    url: 'vault://tasks/finish-artifact-safety.md',
    expected: 'mock-sha-finish-artifact-safety',
    title: 'Finish artifact safety, reviewed',
  });
});

test('a TaskModal save that changes nothing sends no update', async ({ page }) => {
  await harness(page, { isoDueSecond: true });
  const sheet = await openFromPalette(page, 'Sketch the 3.0 daily cockpit');
  // The ISO due shows as a date plus a time; saving untouched must not
  // rewrite `due` or insert `due_time`.
  await expect(sheet.getByLabel('Due date', { exact: true })).toHaveValue('2026-07-23');
  await sheet.getByRole('button', { name: 'Save changes', exact: true }).click();
  await expect(page.locator('.task-editor')).toHaveCount(0);
  const calls = await page.evaluate(() => (window as any).__mockApiCalls ?? {});
  expect(calls.update_task ?? 0).toBe(0);
  expect(await updates(page)).toEqual([]);
});

test('linked task editor locks synced fields', async ({ page }) => {
  await harness(page, { linkSecond: true });
  const sheet = await openFromPalette(page, 'Sketch the 3.0 daily cockpit');
  await expect(sheet.locator('.task-linked-note')).toContainText('Linked to an issue');
  await expect(sheet.getByLabel('Task name', { exact: true })).toBeDisabled();
  await expect(sheet.getByRole('combobox', { name: 'Project', exact: true })).toBeDisabled();
  await expect(sheet.getByLabel('Due date', { exact: true })).toBeDisabled();
  await expect(sheet.getByRole('textbox', { name: /^Notes/ })).toBeDisabled();
  await expect(sheet.getByRole('combobox', { name: 'Status', exact: true })).toBeDisabled();
  await expect(sheet.getByRole('radio', { name: 'High priority (P1)' })).toBeDisabled();
  await expect(sheet.getByRole('button', { name: 'Suggest a clearer name', exact: true })).toBeDisabled();
  await openMore(sheet);
  await sheet.getByLabel(/^Labels/).fill('product, cockpit');
  await sheet.getByRole('button', { name: 'Save changes', exact: true }).click();
  await expect(page.locator('.task-editor')).toHaveCount(0);
  expect(await updates(page)).toEqual([
    { url: 'vault://tasks/rebuild-today.md', expected: 'mock-sha-rebuild-today', labels: ['product', 'cockpit'] },
  ]);
});

test('stale task edit keeps the editor open and explains the conflict', async ({ page }) => {
  await harness(page, { conflict: true });
  const sheet = await openFromPalette(page, 'Finish artifact safety defaults');
  await sheet.getByLabel('Task name', { exact: true }).fill('Retitled draft');
  await sheet.getByRole('button', { name: 'Save changes', exact: true }).click();
  const status = sheet.locator('.task-detail-error');
  await expect(status).toContainText('changed on disk');
  await expect(status).not.toContainText('TASK_CONFLICT');
  await expect(sheet).toBeVisible();
  await expect(sheet.getByLabel('Task name', { exact: true })).toHaveValue('Retitled draft');
});

test('a lock that holds only the project leaves the rest editable', async ({ page }) => {
  await harness(page, { staleFirst: true });
  const sheet = await openFromPalette(page, 'Finish artifact safety defaults');
  await expect(sheet.locator('.task-linked-note')).toContainText('Edit the project on the issue');
  // No redirect: only a linked lock offers the synced-fields jump.
  await expect(sheet.getByRole('button', { name: 'Edit synced fields in Tasks', exact: true })).toHaveCount(0);
  await expect(sheet.getByRole('combobox', { name: 'Project', exact: true })).toBeDisabled();
  await expect(sheet.getByLabel('Task name', { exact: true })).toBeEnabled();
  await expect(sheet.getByRole('button', { name: 'Suggest a clearer name', exact: true })).toBeEnabled();
  await sheet.getByLabel('Task name', { exact: true }).fill('Finish artifact safety, relink later');
  await sheet.getByRole('button', { name: 'Save changes', exact: true }).click();
  await expect(page.locator('.task-editor')).toHaveCount(0);
  expect(await updates(page)).toEqual([
    { url: 'vault://tasks/finish-artifact-safety.md', expected: 'mock-sha-finish-artifact-safety', title: 'Finish artifact safety, relink later' },
  ]);
});

test('a status toggle refreshes the baseline, so the next edit does not conflict with it', async ({ page }) => {
  await harness(page, { holdTasksAfterToggle: true });
  // Done tasks stay in the list only while a search is typed.
  await page.getByRole('textbox', { name: 'Find a task' }).fill('Finish artifact safety defaults');
  const tick = page.getByRole('button', { name: 'Mark Finish artifact safety defaults done', exact: true });
  await expect(tick).toBeEnabled();
  await tick.click();
  // The optimistic flip lands; the list refetch is held, so the only thing
  // that can refresh the row's contentHash is the hash the toggle returned.
  await expect(page.getByRole('button', { name: 'Reopen Finish artifact safety defaults', exact: true })).toBeVisible();
  await expect.poll(() => page.evaluate(() => (window as any).__mockApiCalls?.set_task_status ?? 0)).toBe(1);
  await localRow(page, 'Finish artifact safety defaults').locator('.issue-row-main').click();
  // Local tasks edit in the same shared editor, inline in the Tasks pane (H02).
  await page.getByRole('button', { name: 'Edit task', exact: true }).click();
  const sheet = page.locator('.task-detail .task-editor-form');
  await expect(sheet).toBeVisible();
  await sheet.getByRole('radio', { name: 'Medium priority (P2)' }).click();
  await sheet.getByRole('button', { name: 'Save changes', exact: true }).click();
  await expect(page.locator('.task-detail .task-editor-form')).toHaveCount(0);
  expect(await updates(page)).toEqual([
    { url: 'vault://tasks/finish-artifact-safety.md', expected: 'mock-sha-finish-artifact-safety+', priority: 'P2' },
  ]);
  await page.evaluate(() => (window as any).__releaseTasks());
});

test("task assist sends the linked task's project URL and omits it once the project changes", async ({ page }) => {
  await harness(page);
  const sheet = await openFromPalette(page, 'Finish artifact safety defaults');
  const inputs = () => page.evaluate(() => (window as any).__assistInputs as Record<string, unknown>[]);
  await openMore(sheet);
  await sheet.getByRole('button', { name: 'Suggest steps', exact: true }).click();
  await expect(sheet.locator('.assist-staged-row')).toHaveCount(2);
  expect((await inputs()).at(-1)).toMatchObject({
    action: 'breakdown',
    title: 'Finish artifact safety defaults',
    projectUrl: 'vault://projects/hollow-3-0.md',
  });

  await choose(sheet.getByRole('combobox', { name: 'Project', exact: true }), '');
  await sheet.getByRole('button', { name: 'Suggest steps', exact: true }).click();
  await expect(sheet.locator('.assist-staged-row')).toHaveCount(4);
  const sent = await inputs();
  expect(sent).toHaveLength(2);
  expect(sent[1].action).toBe('breakdown');
  expect('projectUrl' in sent[1]).toBe(false);
});
