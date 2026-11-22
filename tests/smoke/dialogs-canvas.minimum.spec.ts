import { expect, test, type Locator, type Page } from './fixtures/test';
import { chooseNew, openChat } from './choose';
import { openWaiting } from './waiting';

/**
 * Review F07's required small-window set, the two flows required.minimum.spec
 * didn't cover yet (plan completion L5, matrix V7i): dialogs, and a canvas's
 * host controls in a conversation. Only the chromium-minimum (720 × 480) and
 * chromium-minimum-large-text (720 × 600, Largest text) projects run it, and
 * nothing in it may skip (scripts/check-required-specs.mjs). Visible controls
 * only, the way a person with a mouse would.
 */

async function noHorizontalScroll(page: Page) {
  const [scroll, client] = await page.evaluate(() => [document.scrollingElement!.scrollWidth, document.scrollingElement!.clientWidth]);
  expect(scroll, 'no horizontal page scroll').toBeLessThanOrEqual(client);
}
/** The control is on screen, not clipped by the window. */
async function onScreen(page: Page, target: Locator) {
  await target.scrollIntoViewIfNeeded();
  const box = (await target.boundingBox())!;
  const vp = page.viewportSize()!;
  expect(box.x, 'left edge on screen').toBeGreaterThanOrEqual(0);
  expect(box.y, 'top edge on screen').toBeGreaterThanOrEqual(0);
  expect(box.x + box.width, 'right edge on screen').toBeLessThanOrEqual(vp.width + 0.5);
  expect(box.y + box.height, 'bottom edge on screen').toBeLessThanOrEqual(vp.height + 0.5);
}

test('dialogs: the Add sheet and a confirm keep every choice on screen, and Cancel/Close work', async ({ page }) => {
  await page.goto('/?mock=1&demo=1&scenario=interrupted');
  // The Add sheet, from Library actions (the top bar's New menu went in U1);
  // at this size Library is reached through Show sidebar.
  await chooseNew(page, 'Add a task, note, link, file or recording…');
  const add = page.getByRole('dialog', { name: 'Add' });
  await expect(add).toBeVisible();
  await add.getByRole('textbox', { name: 'What to add' }).fill('Call the Garage 14 landlord about the renewal');
  await add.getByRole('group', { name: 'Add as' }).getByRole('button', { name: 'Task' }).click();
  for (const name of ['Add task', 'Cancel']) await onScreen(page, add.getByRole('button', { name, exact: true }));
  await noHorizontalScroll(page);
  await add.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(add).toHaveCount(0);

  // A confirm (the interrupted-chat resume): its text and both buttons fit.
  await openWaiting(page);
  await page.getByRole('region', { name: 'Waiting on you' }).getByRole('button', { name: /Resume 1 interrupted chat/ }).click();
  const confirm = page.getByRole('dialog');
  await expect(confirm).toContainText('Revise the labour section');
  const buttons = confirm.getByRole('button');
  const count = await buttons.count();
  expect(count, 'a choice and a way out').toBeGreaterThanOrEqual(2);
  for (let i = 0; i < count; i++) await onScreen(page, buttons.nth(i));
  await noHorizontalScroll(page);
  await confirm.getByRole('button', { name: /Cancel|Not now/ }).first().click();
  await expect(confirm).toHaveCount(0);
});

test('canvas host controls: a canvas in a conversation keeps Expand on screen and opens it in the panel', async ({ page }) => {
  test.setTimeout(60_000);
  await page.goto('/?mock=1&demo=large');
  await expect(page.locator('.home-project-row').first()).toBeVisible();
  // The sidebar lists no conversations since L8 (11-10): Search chats opens
  // the thread (in a window this narrow it shows the hidden sidebar for the
  // search and hides it again).
  await openChat(page, 'Canvas review thread');
  const canvas = page.locator('.chat-canvas').last();
  await expect(canvas).toBeVisible({ timeout: 15_000 });
  const expand = canvas.getByRole('button', { name: 'Expand', exact: true });
  await onScreen(page, expand);
  await noHorizontalScroll(page);
  await expand.click();
  // The full-height copy opens in the Preview pane (run 15 P1; it takes the
  // page's place in a small window).
  const pane = page.getByRole('complementary', { name: 'Preview' });
  await expect(pane).toBeVisible();
  await expect(pane.locator('.chat-canvas').first()).toBeVisible();
  await noHorizontalScroll(page);
});
