import { expect, test, type Page } from './fixtures/test';
import { chooseNew } from './choose';

/**
 * 2026-11-10 (Alex: "having the file link be the full path is ridiculous";
 * and the COO's `outputs/location-manager/email-intake/IT-REQUEST.md`, relative
 * to another project, said "couldn't find"): a reply's file links show as
 * chips (the name and a type icon, the path as tooltip, accessible name and
 * copy), and a relative path no folder of the chat holds is looked for across
 * every project and the vault (`chat_find_file`; the mock's `mockFindFiles`
 * says what the vault holds): one match opens, several offer a picker.
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
  await page.getByRole('textbox', { name: 'Project name' }).fill('Chips');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Chips');
  return page.evaluate(() =>
    (JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]') as { title: string; cwd: string }[]).find(w => w.title === 'Chips')!.cwd);
}

async function reply(page: Page, events: Record<string, unknown>[]) {
  const composer = page.getByLabel('Chat message');
  await composer.fill('Where is it?');
  await composer.press('Meta+Enter');
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBeGreaterThan(0);
  await emit(page, { type: 'runStarted', asked: 'Where is it?', runId: 'run-chips', recall: [] });
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

const findFiles = (page: Page, found: Record<string, string[]>) => page.evaluate(f => localStorage.setItem('mockFindFiles', JSON.stringify(f)), found);
const chips = (page: Page) => lastReply(page).locator('.file-chip');

test('a reply\'s file links show their names and type icons, never their paths', async ({ page }) => {
  await openProject(page);
  await reply(page, [{ type: 'assistantText', text: [
    'Saved `workspaces/notion-390b/outputs/location-manager/email-intake/IT-REQUEST.md`,',
    'the rows in `outputs/q3/rows.csv`, the deck `outputs/q3/review.pptx`, the PDF outputs/q3/review.pdf,',
    'code at `src/app.ts:42`, an older `outputs/phone/IT-REQUEST.md` and [the plan](outputs/plan.md).',
  ].join(' ') }]);
  await expect(chips(page).locator('.file-chip-name')).toHaveText(['IT-REQUEST.md', 'rows.csv', 'review.pptx', 'review.pdf', 'app.ts', 'IT-REQUEST.md', 'the plan']);
  // Each has a type icon; the visible text never holds a folder path.
  await expect(chips(page).locator('svg')).toHaveCount(7);
  for (const text of await chips(page).allInnerTexts()) expect(text).not.toContain('outputs/');
  await expect(lastReply(page).locator('.file-chip.tone-sheet')).toHaveCount(1);
  await expect(lastReply(page).locator('.file-chip.tone-slides')).toHaveCount(1);
  await expect(lastReply(page).locator('.file-chip.tone-pdf')).toHaveCount(1);
  // The cited line is muted beside the name; same-name files show their folders.
  await expect(chips(page).nth(4).locator('.file-chip-lines')).toHaveText(':42');
  await expect(chips(page).nth(0).locator('.file-chip-folder')).toHaveText('email-intake');
  await expect(chips(page).nth(5).locator('.file-chip-folder')).toHaveText('phone');
  await expect(chips(page).nth(1).locator('.file-chip-folder')).toBeHidden();
  // The path is the tooltip and part of the accessible name.
  const first = chips(page).first();
  await expect(first).toHaveAttribute('title', 'workspaces/notion-390b/outputs/location-manager/email-intake/IT-REQUEST.md');
  await expect(first).toHaveAccessibleName('Open IT-REQUEST.md, workspaces/notion-390b/outputs/location-manager/email-intake/IT-REQUEST.md');
  // A Markdown link keeps its own words when they aren't a path (ReplyMarkdown.tsx).
  await expect(lastReply(page).getByRole('link', { name: 'Open the plan, outputs/plan.md' })).toBeVisible();
  // Keyboard focus reaches a chip.
  await first.focus();
  await expect(first).toBeFocused();
});

test('copying a reply copies a chip\'s path, not its name', async ({ page }) => {
  await openProject(page);
  await reply(page, [{ type: 'assistantText', text: 'See `outputs/q3/rows.csv` for the rows.' }]);
  const copied = await lastReply(page).locator('.md-view').evaluate(el => {
    const range = document.createRange();
    range.selectNodeContents(el);
    const sel = window.getSelection()!;
    sel.removeAllRanges(); sel.addRange(range);
    const data = new DataTransfer();
    el.dispatchEvent(new ClipboardEvent('copy', { clipboardData: data, bubbles: true, cancelable: true }));
    return data.getData('text/plain');
  });
  expect(copied).toContain('See outputs/q3/rows.csv for the rows.');
});

test('a path relative to another project opens from the vault, and several matches offer a picker', async ({ page }, info) => {
  test.skip(info.project.name !== 'chromium-desktop', 'the Preview pane split is desktop-only');
  await openProject(page);
  const tail = 'outputs/location-manager/email-intake/IT-REQUEST.md';
  const portal = `/mock/vault/workspaces/notion-390b/${tail}`;
  await reply(page, [{ type: 'assistantText', text: `The request is in \`${tail}\`.` }]);
  // No folder of this chat holds it; one project's folder does.
  await missing(page, ['IT-REQUEST.md']);
  await findFiles(page, { [tail]: [portal] });
  await chips(page).first().click();
  await expect(shown(page)).toHaveText('IT-REQUEST.md');
  await expect(shown(page)).toHaveAttribute('title', endsWithPath(portal));
  await expect(page.locator('.snackbar').filter({ hasText: "couldn't find" })).toHaveCount(0);
  // Two places hold it: a picker, and the pick opens.
  await pane(page).getByRole('button', { name: 'Close preview', exact: true }).click();
  const other = `/mock/vault/workspaces/ops/${tail}`;
  await findFiles(page, { [tail]: [portal, other] });
  await chips(page).first().click();
  const dialog = page.getByRole('dialog', { name: 'Which IT-REQUEST.md?' });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('button', { name: /^Open / })).toHaveCount(2);
  await expect(dialog.getByRole('button', { name: 'Confirm' })).toHaveCount(0);
  await dialog.getByRole('button', { name: /^Open .*workspaces\/ops\// }).click();
  await expect(dialog).toBeHidden();
  await expect(shown(page)).toHaveAttribute('title', endsWithPath(other));
});

test('an absolute path that is gone says so and never opens another file of its name', async ({ page }) => {
  await openProject(page);
  await reply(page, [{ type: 'assistantText', text: 'See `/Users/me/vault/workspaces/a/outputs/gone.md`.' }]);
  await missing(page, ['gone.md']);
  await findFiles(page, { 'outputs/gone.md': ['/mock/vault/workspaces/b/outputs/gone.md'] });
  await chips(page).first().click();
  await expect(page.locator('.snackbar').filter({ hasText: "Hollow couldn't find gone.md" })).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as { __findCalls?: unknown[] }).__findCalls?.length ?? 0)).toBe(0);
});
