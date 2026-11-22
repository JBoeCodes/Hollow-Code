import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from './fixtures/test';
import { chooseNew } from './choose';

/**
 * 2026-11-04 (Alex: "any file I open that's created by the agent [should]
 * open in the side pane if I click on it. sometimes when I click on a file it
 * says cannot open file cause file has moved"): a file a chat names is tried
 * in each of the chat's folders, and the first that holds it opens in the
 * Preview pane (`chat_locate_file`; the mock's `mockMissingFiles` removes
 * places). A path in the prose is clickable too; a Mac mini path maps back to
 * this Mac; a file no folder holds is explained in words.
 */

async function installEventBridge(page: Page) {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (event: unknown) => void>();
    const listeners = new Map<string, number[]>();
    let next = 1;
    const w = window as unknown as Record<string, unknown>;
    w.__TAURI_INTERNALS__ = {
      transformCallback(callback: (event: unknown) => void) { const id = next++; callbacks.set(id, callback); return id; },
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

const emit = (page: Page, payload: Record<string, unknown>) => page.evaluate(event => {
  (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void }).__emitTauriEvent('chat-event', event);
}, payload);

async function openProject(page: Page) {
  await installEventBridge(page);
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Write the Q3 review.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Locate');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Locate');
  return page.evaluate(() =>
    (JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]') as { title: string; cwd: string }[]).find(w => w.title === 'Locate')!.cwd);
}

async function reply(page: Page, events: Record<string, unknown>[]) {
  const composer = page.getByLabel('Chat message');
  await composer.fill('Where is it?');
  await composer.press('Meta+Enter');
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBeGreaterThan(0);
  await emit(page, { type: 'runStarted', asked: 'Where is it?', runId: 'run-locate', recall: [] });
  for (const e of events) await emit(page, e);
  await emit(page, { type: 'done', sessionId: null });
}

const missing = (page: Page, ends: string[]) => page.evaluate(e => localStorage.setItem('mockMissingFiles', JSON.stringify(e)), ends);
const pane = (page: Page) => page.getByRole('complementary', { name: 'Preview' });
/** The tab on show: it names the file, and its tooltip ends with the full
 *  path (UI clutter pass, 2026-11-06: no path line under the tabs). */
const shown = (page: Page) => pane(page).getByRole('tab', { selected: true });
const endsWithPath = (path: string) => new RegExp(`\\n${path.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`);
const lastReply = (page: Page) => page.locator('.work-conversation .chat-msg').last();

test('a vault-relative path opens from the vault when the chat’s folder doesn’t hold it', async ({ page }, info) => {
  test.skip(info.project.name !== 'chromium-desktop', 'the Preview pane split is desktop-only');
  const cwd = await openProject(page);
  await reply(page, [{ type: 'assistantText', text: 'The plan is in `workspaces/main/outputs/plan.md`.' }]);
  // Neither the chat's folder nor the workspace folder holds it (the mock's
  // chat folder and vault are different roots); the vault does.
  await missing(page, [`/${cwd.split('/').pop()}/workspaces/main/outputs/plan.md`]);
  await lastReply(page).locator('code.file-ref').click();
  await expect(shown(page)).toHaveText('plan.md');
  await expect(shown(page)).toHaveAttribute('title', /\n\/\S*\/vault\/workspaces\/main\/outputs\/plan\.md$/);
  const tried = await page.evaluate(() => (window as unknown as { __locateCalls: string[][] }).__locateCalls.at(-1)!);
  expect(tried[0]).toBe(`${cwd}/workspaces/main/outputs/plan.md`);
  expect(tried.at(-1)).toMatch(/\/vault\/workspaces\/main\/outputs\/plan\.md$/);
});

test('a path written in the prose opens in the Preview pane', async ({ page }, info) => {
  test.skip(info.project.name !== 'chromium-desktop', 'the Preview pane split is desktop-only');
  const cwd = await openProject(page);
  await reply(page, [{ type: 'assistantText', text: 'I saved the review to outputs/q3-review.md. Node.js stays a word.' }]);
  const prose = lastReply(page).locator('.prose-file-ref');
  await expect(prose).toHaveText(['q3-review.md']);
  await prose.click();
  await expect(shown(page)).toHaveAttribute('title', endsWithPath(`${cwd}/outputs/q3-review.md`));
  // The keyboard reaches it too.
  await pane(page).getByRole('button', { name: 'Close preview', exact: true }).click();
  await prose.focus();
  await page.keyboard.press('Enter');
  await expect(shown(page)).toHaveText('q3-review.md');
});

test('a Mac mini path maps back to this chat’s folder', async ({ page }, info) => {
  test.skip(info.project.name !== 'chromium-desktop', 'the Preview pane split is desktop-only');
  const cwd = await openProject(page);
  const remote = '/Users/runner/hollow-remote/folders/w1-ab12/outputs/runner.md';
  await reply(page, [
    { type: 'toolUse', id: 'w1', name: 'Write', input: { file_path: remote, content: '# from the mini' } },
    { type: 'toolResult', id: 'w1', content: 'written', isError: false },
    { type: 'assistantText', text: 'Done on the runner.' },
  ]);
  await missing(page, ['/hollow-remote/folders/w1-ab12/outputs/runner.md']);
  await page.locator('.work-conversation .chat-run-fold-head').last().click();
  await page.locator('.work-conversation').getByRole('button', { name: 'Preview runner.md', exact: true }).first().click();
  await expect(shown(page)).toHaveAttribute('title', endsWithPath(`${cwd}/outputs/runner.md`));
});

test('a tool row’s file chip is a button beside the row’s toggle, not inside it', async ({ page }, info) => {
  test.skip(info.project.name !== 'chromium-desktop', 'the Preview pane split is desktop-only');
  await openProject(page);
  await reply(page, [
    { type: 'toolUse', id: 'r1', name: 'Read', input: { file_path: 'notes/brief.md' } },
    { type: 'toolResult', id: 'r1', content: '     1→# Brief', isError: false },
    // Three edits fold into one group row (FOLD_MIN).
    { type: 'toolUse', id: 'e1', name: 'Edit', input: { file_path: 'src/a.ts', old_string: 'x', new_string: 'y' } },
    { type: 'toolUse', id: 'e2', name: 'Edit', input: { file_path: 'src/a.ts', old_string: 'p', new_string: 'q' } },
    { type: 'toolUse', id: 'e3', name: 'Edit', input: { file_path: 'docs/b.md', old_string: 'old', new_string: 'new' } },
    { type: 'assistantText', text: 'Read and edited.' },
  ]);
  const convo = page.locator('.work-conversation');
  await convo.locator('.chat-run-fold-head').last().click();
  // Relative paths open since this change, so every chip is a button: none
  // may sit inside the row's toggle (a nested control).
  expect((await new AxeBuilder({ page }).include('.work-conversation').withRules(['nested-interactive']).analyze()).violations).toEqual([]);

  const read = convo.locator('.toolcall').filter({ hasText: 'brief.md' }).first();
  const toggle = read.locator('.toolcall-toggle');
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  // The toggle's name is the row's, not its verb alone.
  await expect(toggle).toHaveAccessibleName('Read 1 line brief.md');
  // The keyboard goes from the toggle to the chip; Enter opens the file.
  await toggle.focus();
  await page.keyboard.press('Tab');
  await expect(read.getByRole('button', { name: 'Preview brief.md', exact: true })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(shown(page)).toHaveText('brief.md');
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  // A click anywhere else on the row still expands it (the toggle's
  // stretched layer), here at the row's far end.
  const head = (await read.locator('.toolcall-head').boundingBox())!;
  await page.mouse.click(head.x + head.width - 24, head.y + head.height / 2);
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');

  // A folded run of edits: each file in its header opens on its own.
  const group = convo.locator('.tool-group').first();
  await expect(group.locator('.toolcall-head .toolcall-file')).toHaveText(['a.ts', 'b.md']);
  const groupToggle = group.locator('.toolcall-head').first().locator('.toolcall-toggle');
  await expect(groupToggle).toHaveAccessibleName(/^Edited 2 files a\.ts b\.md \+3 −3/);
  const wasOpen = await groupToggle.getAttribute('aria-expanded');
  await group.locator('.toolcall-head').first().getByRole('button', { name: 'Preview b.md', exact: true }).click();
  await expect(shown(page)).toHaveText('b.md');
  await expect(groupToggle).toHaveAttribute('aria-expanded', wasOpen!);
});

test('a file no folder holds is explained in words, a scratch file as cleared', async ({ page }) => {
  await openProject(page);
  await reply(page, [{ type: 'assistantText', text: 'See `outputs/gone.md` and `/private/tmp/hollow-run-501/ab/chart.png`.' }]);
  await missing(page, ['gone.md', 'chart.png']);
  const refs = lastReply(page).locator('code.file-ref');
  await refs.first().click();
  await expect(page.locator('.snackbar').filter({ hasText: "Hollow couldn't find gone.md in this chat's folder, its project or the vault." })).toBeVisible();
  await refs.last().click();
  await expect(page.locator('.snackbar').filter({ hasText: "chart.png was in the turn's scratch folder, which Hollow clears when a turn ends." })).toBeVisible();
  await expect(pane(page)).toHaveCount(0);
});
