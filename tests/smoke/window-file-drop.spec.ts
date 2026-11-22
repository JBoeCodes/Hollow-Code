import { expect, test, type Page } from './fixtures/test';
import { chooseNew } from './choose';

/**
 * 2026-11-04 (Alex: "drag and drop any file into hollow and have it show
 * in chat and send to the agent"): a file dropped anywhere on the window,
 * not only on the message box, attaches to the chat. A chat on screen takes
 * it; a project's chat behind another page comes back for it; on Home a new
 * quick chat opens holding it (2026-11-14). Targets that take file drops themselves (the
 * message box) keep them.
 */

/** Drag a PDF over `selector` (the veil then says where it goes, from its
 *  lazy chunk), and later drop it there: whether frames let the drag
 *  through, and whether the drop's default (the webview loading the file)
 *  was cancelled. */
async function dragOver(page: Page, selector: string, name = 'report.pdf') {
  await page.locator(selector).first().evaluate((el, name) => {
    const dt = new DataTransfer();
    dt.items.add(new File([new TextEncoder().encode('%PDF-1.4')], name, { type: 'application/pdf' }));
    (window as unknown as { __dt: DataTransfer }).__dt = dt;
    el.dispatchEvent(new DragEvent('dragenter', { dataTransfer: dt, bubbles: true, cancelable: true }));
    el.dispatchEvent(new DragEvent('dragover', { dataTransfer: dt, bubbles: true, cancelable: true }));
  }, name);
}
async function dropNow(page: Page, selector: string) {
  return page.locator(selector).first().evaluate(el => {
    const dt = (window as unknown as { __dt: DataTransfer }).__dt;
    el.dispatchEvent(new DragEvent('dragover', { dataTransfer: dt, bubbles: true, cancelable: true }));
    const framesPass = document.documentElement.hasAttribute('data-file-drag');
    const drop = new DragEvent('drop', { dataTransfer: dt, bubbles: true, cancelable: true });
    el.dispatchEvent(drop);
    return { framesPass, prevented: drop.defaultPrevented, after: document.querySelector('.window-file-drop') !== null };
  });
}
const veil = (page: Page) => page.locator('.window-file-drop');

async function newProject(page: Page, name: string) {
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Read what I drop.');
  await page.getByRole('textbox', { name: 'Project name' }).fill(name);
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText(name);
  await expect(page.locator('.chat-composer').first()).toBeVisible();
}

const chip = (page: Page) => page.locator('.chat-attachments .chat-attach-file');

test('a file dropped on the conversation, not the message box, attaches to the chat', async ({ page }) => {
  await newProject(page, 'Drop anywhere');
  await dragOver(page, '.chat-main .chat-scroll');
  await expect(veil(page)).toHaveText('Drop to attach to this chat');
  expect(await dropNow(page, '.chat-main .chat-scroll')).toEqual({ framesPass: true, prevented: true, after: false });
  await expect(chip(page)).toContainText('report.pdf');
  await expect(page.getByRole('textbox', { name: 'Chat message' }).first()).toBeFocused();
  await expect.poll(() => page.evaluate(() => document.documentElement.hasAttribute('data-file-drag'))).toBe(false);
});

test('the message box keeps its own drop, without the window veil', async ({ page }) => {
  await newProject(page, 'Drop on the box');
  await dragOver(page, '.chat-composer');
  await expect(page.locator('.chat-composer.drag-over')).toHaveCount(1);
  await expect(veil(page)).toHaveCount(0);
  expect((await dropNow(page, '.chat-composer')).prevented).toBe(true);
  await expect(chip(page)).toHaveCount(1);
});

test('a file dropped while another page shows comes back to the project’s chat', async ({ page }) => {
  await newProject(page, 'Come back');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page.locator('.chat-composer').first()).toBeHidden();
  await dragOver(page, '.canvas');
  await expect(veil(page)).toHaveText('Drop to attach to the chat in Come back');
  expect((await dropNow(page, '.canvas')).prevented).toBe(true);
  await expect(page.locator('.work-header h1')).toHaveText('Come back');
  await expect(chip(page)).toContainText('report.pdf');
});

test('a file dropped on Home opens a new quick chat holding it', async ({ page }) => {
  await page.goto('/?mock=1');
  await expect(page.locator('.chat-composer')).toHaveCount(0);
  await dragOver(page, '.canvas');
  await expect(veil(page)).toHaveText('Drop to attach to a new quick chat');
  expect((await dropNow(page, '.canvas')).prevented).toBe(true);
  await expect(page.locator('.chat-composer').first()).toBeVisible();
  // The destination is a new quick chat, not the COO's chat.
  await expect(page.locator('.work-header h1')).toHaveText('Quick chats');
  await expect(chip(page)).toContainText('report.pdf');
});
