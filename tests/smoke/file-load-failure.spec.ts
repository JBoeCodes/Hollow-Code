import { expect, test, type Page } from './fixtures/test';
import { chooseNew } from './choose';

// A file that can't be read (missing, a folder, an image over the preview
// limit) fails in the side panel's Files view, never in the page's recovery
// boundary. Clicking `src/App.tsx:331` in a workspace reply resolves it
// against the workspace folder, where it isn't: the read rejects, and an
// unguarded `file()` read threw "Work could not load" (10-23).
async function open(page: Page) {
  await page.goto('/?mock=1'); await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Check file failures');
  await page.getByRole('textbox', { name: 'Project name' }).fill('File failures');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('File failures');
  // On a narrow window the panel replaces the conversation: opening the file opens it.
}
const refuse = (page: Page, refusals: Record<string, string>) =>
  page.evaluate(r => localStorage.setItem('mockPreviewRefusal', JSON.stringify(r)), refusals);

test('a file that can’t be read fails in the Files view, with Retry, and the page stays', async ({ page }) => {
  await open(page);
  await refuse(page, { '/src/App.tsx': 'read /mock/vault/src/App.tsx: No such file or directory (os error 2)' });
  await page.evaluate(async () => {
    const path = '/src/store/chatRuntime.ts'; const s = await import(/* @vite-ignore */ path);
    s.setChatTurns([{ role: 'assistant', text: 'The screen is `src/App.tsx:331`, mounted per page.', tools: [] }]);
  });
  const reply = page.locator('.work-conversation .chat-msg').last();
  await reply.locator('code.file-ref').click();
  const pane = page.getByRole('complementary', { name: 'Preview', exact: true });
  const failed = pane.getByRole('alert').filter({ hasText: 'Couldn’t open App.tsx' });
  await expect(failed).toBeVisible();
  await expect(failed).toContainText('isn’t at this path');
  await expect(page.getByText('Work could not load')).toHaveCount(0);
  // The rest of the page and the panel's own controls stay.
  await expect(page.locator('.work-header h1')).toHaveText('File failures');
  // The Preview pane (run 15 P1) keeps its own boundary and controls.
  await expect(page.getByText('Preview could not load')).toHaveCount(0);
  await expect(pane.getByRole('tab', { selected: true })).toHaveText('App.tsx');
  await expect(pane.getByRole('button', { name: 'Close preview', exact: true })).toBeVisible();
  // Retry reads it again: once the file is there, it shows.
  await refuse(page, {});
  await failed.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(failed).toHaveCount(0);
  await expect(pane.locator('.chat-preview-source, .chat-preview-body .note-renderer, .chat-preview-body pre').first()).toBeVisible();
});

test('a folder and an oversized picture say so, with the way to open them', async ({ page }) => {
  await open(page);
  await refuse(page, {
    '/outputs/canvases': 'not a file: /mock/vault/outputs/canvases',
    '/huge.png': 'image too large to preview (14 MB)',
  });
  const pane = page.getByRole('complementary', { name: 'Preview', exact: true });
  const show = (path: string) => page.evaluate(async p => {
    const path = '/src/store/chatRuntime.ts'; const s = await import(/* @vite-ignore */ path);
    s.openFileInSidePane(p);
  }, path);
  await show('/mock/vault/outputs/canvases');
  await expect(pane.getByRole('alert').filter({ hasText: 'Couldn’t open canvases' })).toContainText('folder');
  await expect(pane.getByRole('button', { name: 'Show in Finder', exact: true })).toBeVisible();
  await show('/mock/vault/huge.png');
  const big = pane.getByRole('alert').filter({ hasText: 'Couldn’t open huge.png' });
  await expect(big).toContainText('too large to preview');
  await expect(big.getByRole('button', { name: 'Open in default app', exact: true })).toBeVisible();
  await expect(page.getByText('Work could not load')).toHaveCount(0);
  await expect(page.getByText('Preview could not load')).toHaveCount(0);
});
