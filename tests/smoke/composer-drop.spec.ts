import { expect, test, type Page } from './fixtures/test';
import { chooseNew, openChat, seedChatsChat } from './choose';

/**
 * P0-07 and C5 (2026-11-04): any file dropped on the chat composer attaches
 * (an image as a thumbnail, anything else as a chip with its name and size)
 * and is sent with its saved path; a folder is refused in words with a way
 * to add it to Sources. The webview's default action for the drop (opening
 * the file) is cancelled.
 */

const PNG = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';

/** Stand-in for Tauri's bridge: the file picker records that it opened and is cancelled. */
async function installPicker(page: Page) {
  await page.addInitScript(() => {
    const w = window as unknown as Record<string, unknown>;
    w.__pickerOpens = 0;
    let next = 1;
    w.__TAURI_INTERNALS__ = {
      transformCallback() { return next++; },
      invoke(command: string) {
        if (command === 'plugin:dialog|open') { (w.__pickerOpens as number)++; return Promise.resolve(null); }
        if (command === 'plugin:event|listen') return Promise.resolve(next++);
        if (command === 'plugin:event|unlisten') return Promise.resolve();
        return Promise.reject(new Error(`not in Tauri: ${command}`));
      },
    };
  });
}

/** Drag `files` over the composer and drop them; what the target said while dragged, and whether the drop's default was cancelled. */
async function dropOn(page: Page, files: { name: string; type: string; base64?: string; folder?: boolean }[]) {
  return page.locator('.chat-composer').first().evaluate((el, files) => {
    const dt = new DataTransfer();
    for (const f of files) {
      // A folder reads as a file with no type and no bytes.
      const bytes = f.folder ? new Uint8Array() : f.base64 ? Uint8Array.from(atob(f.base64), c => c.charCodeAt(0)) : new TextEncoder().encode('%PDF-1.4');
      dt.items.add(new File([bytes], f.name, { type: f.type }));
    }
    el.dispatchEvent(new DragEvent('dragover', { dataTransfer: dt, bubbles: true, cancelable: true }));
    const label = el.getAttribute('data-drop-label');
    const drop = new DragEvent('drop', { dataTransfer: dt, bubbles: true, cancelable: true });
    el.dispatchEvent(drop);
    return { label, prevented: drop.defaultPrevented };
  }, files);
}

async function newWorkspace(page: Page) {
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Read the dropped report.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Drop check');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Drop check');
  await expect(page.locator('.chat-composer').first()).toBeVisible();
}

test('a dropped PDF attaches as a chip and is sent with its name and saved path', async ({ page }) => {
  await installPicker(page);
  await newWorkspace(page);
  const result = await dropOn(page, [{ name: 'report.pdf', type: 'application/pdf' }]);
  expect(result).toEqual({ label: 'Drop files to attach', prevented: true });
  const chip = page.locator('.chat-attachments .chat-attach-file');
  await expect(chip).toContainText('report.pdf');
  await expect(chip).toContainText('8 B');
  await expect(chip.getByRole('button', { name: 'Remove report.pdf' })).toBeVisible();
  await expect(page.locator('.snackbar').filter({ hasText: "don't attach" })).toHaveCount(0);
  await page.getByRole('textbox', { name: 'Chat message' }).first().fill('Summarize the report');
  await page.getByRole('button', { name: 'Send', exact: true }).first().click();
  await expect(page.locator('.chat-user-chip').getByRole('button', { name: 'Open report.pdf' })).toBeVisible();
  await expect.poll(() => page.evaluate(() => localStorage.getItem('mockLastChatPrompt') ?? ''))
    .toContain('1. report.pdf at /mock/vault/.hollow-attachments/report-1.pdf\n</attached-files>');
  await expect(page.locator('.chat-attachments')).toHaveCount(0);
});

test('a dropped image still attaches', async ({ page }) => {
  await installPicker(page);
  await newWorkspace(page);
  const result = await dropOn(page, [{ name: 'shot.png', type: 'image/png', base64: PNG }]);
  expect(result).toEqual({ label: 'Drop files to attach', prevented: true });
  await expect(page.locator('.chat-attachments').getByRole('img', { name: 'shot.png' })).toBeVisible();
  await expect(page.locator('.chat-attachments .chat-attach-file')).toHaveCount(0);
});

test('a dropped folder is refused in words, with Add to Sources… opening the picker', async ({ page }) => {
  await installPicker(page);
  await newWorkspace(page);
  const result = await dropOn(page, [{ name: 'Designs', type: '', folder: true }]);
  expect(result.prevented).toBe(true);
  const notice = page.locator('.snackbar').filter({ hasText: "Folders don't attach to a message. Attach the files inside it, or add the folder in Sources." });
  await expect(notice).toBeVisible({ timeout: 1000 });
  await notice.getByRole('button', { name: 'Add to Sources…' }).click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { __pickerOpens: number }).__pickerOpens)).toBe(1);
  await expect(page.getByRole('textbox', { name: 'Chat message' }).first()).toHaveValue('');
  await expect(page.locator('.chat-attachments')).toHaveCount(0);
});

test('a workspace chat keeps Add to Sources… in the + menu', async ({ page }) => {
  await installPicker(page);
  await newWorkspace(page);
  await page.getByRole('button', { name: 'Attach and commands', exact: true }).first().click();
  const menu = page.getByRole('menu');
  await expect(menu.getByRole('menuitem').nth(0)).toHaveText('Attach files…');
  // #1670 (n20/t2-composer-actions, with the file-mention, skill and Markdown rows) put
  // other rows between the two, so the check is that Add to Sources… is there, after Attach.
  const labels = await menu.getByRole('menuitem').allTextContents();
  expect(labels.indexOf('Add to Sources…')).toBeGreaterThan(0);
  await menu.getByRole('menuitem', { name: 'Add to Sources…' }).click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { __pickerOpens: number }).__pickerOpens)).toBe(1);
});

test('in a chat in Quick chats the folder refusal and the + menu have no Sources item', async ({ page }) => {
  await installPicker(page);
  // A quick chat (the Quick chats workspace, back 2026-11-14).
  await seedChatsChat(page, 'Cat names');
  await page.goto('/?mock=1');
  await openChat(page, 'Cat names');
  await expect(page.locator('.work-header h1')).toHaveText('Quick chats');
  await expect(page.locator('.chat-composer').first()).toBeVisible();
  const result = await dropOn(page, [{ name: 'Old drafts', type: '', folder: true }]);
  expect(result.prevented).toBe(true);
  const notice = page.locator('.snackbar').filter({ hasText: "Folders don't attach to a message." });
  await expect(notice).toBeVisible({ timeout: 1000 });
  await expect(notice.getByRole('button', { name: 'Add to Sources…' })).toHaveCount(0);
  await page.getByRole('button', { name: 'Attach and commands', exact: true }).first().click();
  await expect(page.getByRole('menu').getByRole('menuitem', { name: 'Attach files…' })).toBeVisible();
  await expect(page.getByRole('menu').getByRole('menuitem', { name: 'Add to Sources…' })).toHaveCount(0);
});
