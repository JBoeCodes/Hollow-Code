import { expect, test, type Page } from './fixtures/test';
import { clickAskSend } from './askSend';
import { bellWaitingCount, navigateWaiting } from './waiting';

/**
 * Canvas C4 A3: an unanswered ask-canvas is listed in the one "waiting on
 * you" surface like a question card. Its row opens that conversation by its
 * exact key, scrolled to that canvas (not to the bottom, where the
 * conversation would otherwise open); once the person has sent the answer
 * it leaves the list and the bell's count.
 *
 * `mockAskCanvasFixture` puts an ask-canvas reply in "Wire the session
 * store" (a conversation in no workspace, so it opens on the Chat page);
 * `mockAskCanvasFiller` adds exchanges after it, so the canvas sits far
 * above the bottom. The mock's needs-you query lists a saved ask-canvas
 * until the last sent prompt answers it (`mockAskCanvasNeedsYou`).
 */

const ROW_ID = 'canvas:s-20260811-fixture1/release-choices@1';
const TITLE = 'Release choices';
const calls = (page: Page, command: string) => page.evaluate(c => window.__mockApiCalls?.[c] ?? 0, command);
// U3: the full view opens from the top bar's bell, then See all needs you
// (navigateWaiting; the rail's Review row went).
const panel = (page: Page) => page.getByRole('region', { name: 'Waiting on you' });
const row = (page: Page) => panel(page).locator(`.waiting-item[data-id="${ROW_ID}"]`);
const canvasOf = (page: Page) => page.locator('.turn .chat-canvas[data-canvas-id="release-choices"]');
// The bell's description names the count ("Needs you, 3 waiting. …").
const badge = async (page: Page) => (await bellWaitingCount(page)) ?? 0;

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name.includes('narrow'), 'The ask fixture conversation opens on the desktop Chat page');
  await page.addInitScript(() => {
    localStorage.setItem('mockAskCanvasFixture', '1');
    localStorage.setItem('mockAskCanvasFiller', '24');
  });
  await page.goto('/?mock=1');
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
});

test('an unanswered ask-canvas waits on you; its row opens the conversation at that canvas; it leaves once the answer is sent', async ({ page }) => {
  await navigateWaiting(page);
  await expect(row(page)).toBeVisible();
  const group = panel(page).locator('section.waiting-group[data-group="asks"]');
  await expect(group.locator(`.waiting-item[data-id="${ROW_ID}"]`)).toHaveCount(1);
  await expect(row(page)).toContainText(TITLE);
  await expect(row(page)).toContainText('Canvas waiting for your answer');
  const before = await badge(page);
  expect(before).toBeGreaterThan(0);

  // Exact-id reopening: the conversation opens scrolled to the canvas.
  await row(page).getByRole('button').click();
  await expect(panel(page)).toHaveCount(0);
  await expect(canvasOf(page)).toHaveAttribute('data-saved', 'true');
  await expect(canvasOf(page)).toBeInViewport();
  await expect(canvasOf(page).locator('[data-ask-state="waiting"]')).toHaveText('Waiting on your answer');
  // Not where the conversation opens by itself: its last exchange is out of view.
  await expect(page.locator('.turn').last()).not.toBeInViewport();
  await expect(page.locator('.chat-scroll:visible')).not.toHaveJSProperty('scrollTop', 0);

  // The person answers in the canvas and sends with Hollow's Send. After the
  // reveal, frames above still grow during its 700 ms reading hold and its
  // second scrollIntoView at 500 ms (chatScrollController `revealCanvas`,
  // `holdReadingForNavigation`); night ST9 (2026-10-14) saw a click in that
  // window land beside the radio and waited out 800 ms of stillness. A click
  // inside the frame now counts as the person's own (H10, 2026-10-15): focus
  // moving into it ends the hold and the second look (`onFrameFocus`), and the
  // reading anchor keeps the canvas still from there. So the first click goes
  // to the form's title, which does nothing itself, and is made again until
  // the frame has focus; then the answer.
  const frame = canvasOf(page).locator(`iframe[title="Canvas: ${TITLE}"]`);
  const form = canvasOf(page).frameLocator(`iframe[title="Canvas: ${TITLE}"]`).getByRole('form', { name: TITLE });
  await expect(async () => {
    await form.getByRole('heading', { name: TITLE, exact: true }).click();
    await expect(frame).toBeFocused({ timeout: 500 });
  }).toPass({ timeout: 5000 });
  await form.getByRole('radio', { name: /Friday, after the bed passes/ }).check();
  // Hollow's Send under the canvas sends the answers straight away (2026-10-14).
  await clickAskSend(canvasOf(page).getByRole('group', { name: 'Send this canvas’s answers' }).getByRole('button', { name: 'Send answers', exact: true }));
  await expect.poll(() => calls(page, 'chat_send')).toBe(1);
  await expect(canvasOf(page).locator('[data-ask-state="answered"]')).toBeVisible();

  // Answered and sent: no longer waiting, and the bell counts one fewer.
  await expect.poll(() => badge(page), { timeout: 5000 }).toBe(before - 1);
  await navigateWaiting(page);
  await expect(panel(page)).toBeVisible();
  await expect(panel(page).locator('.waiting-loading')).toHaveCount(0);
  await expect(row(page)).toHaveCount(0);
});

test('a reload keeps it waiting until it is answered; Enter on its row opens it at the canvas', async ({ page }) => {
  await navigateWaiting(page);
  await expect(row(page)).toBeVisible();
  await page.keyboard.press('Escape');
  await page.reload();
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await navigateWaiting(page);
  await expect(row(page)).toBeVisible();
  // The keys work too: the first row has focus; Enter on the canvas row opens it.
  await row(page).getByRole('button').focus();
  await page.keyboard.press('Enter');
  await expect(canvasOf(page)).toBeInViewport();
  await expect(canvasOf(page).locator('[data-ask-state="waiting"]')).toBeVisible();
});
