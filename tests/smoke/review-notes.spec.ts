import { expect, test, type Page } from './fixtures/test';
import { chooseNew } from './choose';

// Review notes on a file in the Preview pane (row OC9): notes on lines of a
// Markdown file, sent as one draft message, kept across an edit that moves
// their line. Written with the preview-pane spec's patterns; the file is a
// `mockPreviewFiles` fixture, opened from a Read tool row's chip.

const PLAN = '/mock/vault/notes/plan.md';
const TEXT = ['# Plan', 'Ship the notes view', 'Keep it small', 'Stale lines', 'Done'].join('\n');
const EDITED = ['# Plan', 'Added intro one', 'Added intro two', 'Ship the notes view', 'Keep it small', 'Stale lines', 'Done'].join('\n');

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

test('notes on a plan go to the chat as one message and follow their line across an edit', async ({ page }) => {
  test.skip(page.viewportSize()!.width <= 1100, 'the split is desktop-only');
  await installEventBridge(page);
  await page.addInitScript(([path, text]) => localStorage.setItem('mockPreviewFiles', JSON.stringify({ [path]: { text } })), [PLAN, TEXT]);
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Produce a checked release report.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Release');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Release');

  const composer = page.getByLabel('Chat message');
  await composer.fill('Check the plan');
  await composer.press('Meta+Enter');
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBeGreaterThan(0);
  await emit(page, { type: 'runStarted', asked: 'Check the plan', runId: 'run-review-notes', recall: [] });
  await emit(page, { type: 'toolUse', id: 'r1', name: 'Read', input: { file_path: PLAN } });
  await emit(page, { type: 'toolResult', id: 'r1', content: '     1→# Plan', isError: false });
  await emit(page, { type: 'assistantText', text: 'Read the plan.' });
  await emit(page, { type: 'done', sessionId: null });

  await page.locator('.work-conversation .chat-run-fold-head').click();
  await page.locator('.work-conversation .toolcall').filter({ hasText: 'plan.md' }).first()
    .getByRole('button', { name: 'Preview plan.md', exact: true }).click();
  const pane = page.getByRole('complementary', { name: 'Preview' });

  // The toggle is a visible button; the numbered view has one add button per line.
  await pane.getByRole('button', { name: 'Review notes', exact: true }).click();
  await expect(pane.getByRole('button', { name: 'Send notes to chat' })).toBeDisabled();
  await expect(pane.getByRole('button', { name: /^Add a note on line \d+$/ })).toHaveCount(5);

  // Keyboard: focus one line's button, save with Ctrl+Enter; Escape cancels another.
  await pane.getByRole('button', { name: 'Add a note on line 2' }).focus();
  await page.keyboard.press('Enter');
  const editor = pane.getByRole('textbox', { name: 'Note on line 2' });
  await expect(editor).toBeFocused();
  await editor.fill('Say what the view shows');
  await page.keyboard.press('Control+Enter');
  await expect(pane.getByRole('button', { name: 'Add a note on line 2' })).toBeFocused();
  await pane.getByRole('button', { name: 'Add a note on line 4' }).click();
  await pane.getByRole('textbox', { name: 'Note on line 4' }).fill('Remove this section');
  await pane.getByRole('button', { name: 'Save note', exact: true }).click();
  await pane.getByRole('button', { name: 'Add a note on line 5' }).click();
  await page.keyboard.press('Escape');
  await expect(pane.getByRole('textbox', { name: /Note on line/ })).toHaveCount(0);
  await expect(pane.getByRole('button', { name: 'Send 2 notes to chat' })).toBeEnabled();

  // The agent edits the file: two lines land above, and both notes follow.
  await page.evaluate(([path, text]) => localStorage.setItem('mockPreviewFiles', JSON.stringify({ [path]: { text } })), [PLAN, EDITED]);
  await pane.getByRole('button', { name: 'Refresh preview' }).click();
  await expect(pane.locator('[data-row-line="4"] .rn-note')).toContainText('Say what the view shows');
  await expect(pane.locator('[data-row-line="6"] .rn-note')).toContainText('Remove this section');

  // Resolve one, then send: one message, only the open note, a draft not a turn.
  await pane.getByRole('button', { name: 'Resolve the note on line 4' }).click();
  await pane.getByRole('button', { name: 'Send 1 note to chat' }).click();
  await expect(composer).toHaveValue(/Line 6: "Stale lines"\n\s+Remove this section/);
  await expect(composer).not.toHaveValue(/Say what the view shows/);
  expect(await page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBe(1);
});
