import { expect, test, type FrameLocator, type Locator, type Page } from './fixtures/test';
import { clickAskSend } from './askSend';

/**
 * P1-33: ask-canvas answers survive, queue while busy, and never look sent
 * when they weren't.
 *
 * - What the person chose in an ask-canvas is kept in the canvas's saved
 *   state as they answer, so a frame that unmounts (scrolled far away, Read
 *   as text) shows the same answers when it comes back.
 * - While the agent is answering, Hollow's Send reads "Queue answers" and
 *   holds the answers for after the turn (Queue for after, never Steer),
 *   once: a second press, or the side panel's copy, sends nothing more.
 * - A Send that didn't go keeps the answers out of the message box: a callout
 *   under every copy of the canvas offers Send now and Take them back.
 * - An earlier revision has no Send: it offers Go to the current one.
 *
 * `mockAskCanvasFixture` puts an ask-canvas reply in "Wire the session
 * store"; `mockAskCanvasFiller` adds exchanges after it;
 * `mockAskCanvasRevised` adds a later reply with v2 of the same canvas.
 */

const TITLE = 'Release choices';
const DRAFT = [
  'Answered canvas "release-choices":',
  '- When should the release ship?: Monday, with a second look',
  '- Which checks must pass first?: Bed rows',
  '',
  '```hollow-canvas-answer',
  '{"canvasId":"release-choices","revision":1,"answers":{"ship-day":"Monday, with a second look","checks":["Bed rows"]}}',
  '```',
].join('\n');

const calls = (page: Page, command: string) => page.evaluate(c => window.__mockApiCalls?.[c] ?? 0, command);
const lastArgs = (page: Page) => page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}') as { prompt?: string; after?: string | null });
const composer = (page: Page) => page.getByRole('textbox', { name: 'Chat message' });
const canvasOf = (page: Page) => page.locator('.turn .chat-canvas[data-canvas-id="release-choices"]').first();
/** The canvas's second copy: Expand shows it in the Preview pane (run 15 P1;
 *  it was the side panel's Visuals tab, which opened by itself). */
const panelOf = (page: Page) => page.getByRole('complementary', { name: 'Preview' }).locator('.chat-canvas[data-canvas-id="release-choices"]');
async function expandToPane(page: Page) {
  await canvasOf(page).getByRole('button', { name: 'Expand', exact: true }).click();
  return panelOf(page);
}
const frameIn = (canvas: Locator): FrameLocator => canvas.frameLocator(`iframe[title="Canvas: ${TITLE}"]`);
const formIn = (canvas: Locator): Locator => frameIn(canvas).getByRole('form', { name: TITLE });
const groupOf = (canvas: Locator) => canvas.getByRole('group', { name: 'Send this canvas’s answers' });
const sendOf = (canvas: Locator, name = 'Send answers') => groupOf(canvas).getByRole('button', { name, exact: true });
const statusOf = (canvas: Locator) => canvas.locator('.chat-ask-send-status');
/** The not-sent callout: the Send group in its `data-unsent` state. */
const unsentOf = (canvas: Locator) => canvas.locator('.chat-ask-send[data-unsent]');

async function open(page: Page, flags: Record<string, string> = {}) {
  await page.addInitScript((f) => {
    localStorage.setItem('mockAskCanvasFixture', '1');
    for (const [k, v] of Object.entries(f)) localStorage.setItem(k, v);
  }, flags);
  await page.goto('/?mock=1');
  await openChat(page);
}

/** Open the fixture conversation and bring its (first) canvas into view. */
async function openChat(page: Page) {
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.locator('.settings-index').getByRole('button', { name: 'Advanced', exact: true }).click();
  await page.locator('#settings-group-this-mac > summary').click(); // History & recovery: Advanced → This Mac
  await page.getByRole('button', { name: 'Open Library, History', exact: true }).click();
  await page.locator('.library-tabs').getByRole('button', { name: 'Conversations', exact: true }).click(); // Settings PR 4 cut the Conversation history button
  const search = page.getByRole('textbox', { name: 'Search conversations' });
  const show = page.getByRole('button', { name: 'Show conversations', exact: true });
  await expect(search.or(show).first()).toBeVisible();
  if (!await search.isVisible()) await show.click();
  await page.locator('.chat-aside').getByRole('button', { name: 'Open Wire the session store', exact: true }).click();
  await expect(canvasOf(page)).toHaveAttribute('data-saved', 'true');
  await canvasOf(page).scrollIntoViewIfNeeded();
  await expect(formIn(canvasOf(page)).locator('.hc-ask-foot')).toBeAttached();
}

/** The person's answers: not the recommended day, and the first check. */
async function fill(canvas: Locator) {
  const form = formIn(canvas);
  await form.getByRole('radio', { name: /Monday, with a second look/ }).check();
  await form.getByRole('checkbox', { name: /Bed rows/ }).check();
}

async function expectFilled(canvas: Locator) {
  const form = formIn(canvas);
  await expect(form.getByRole('radio', { name: /Monday, with a second look/ })).toBeChecked();
  await expect(form.getByRole('checkbox', { name: /Bed rows/ })).toBeChecked();
  await expect(form.getByRole('radio', { name: /Friday, after the bed passes/ })).not.toBeChecked();
  await expect(form.getByRole('checkbox', { name: /Bundle budget/ })).not.toBeChecked();
}

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name.includes('narrow'), 'Conversation history opens from the desktop settings index');
});

test('answers the person chose come back after the frame scrolls far away and back, and after Read as text', async ({ page }) => {
  await open(page, { mockAskCanvasFiller: '24' });
  const canvas = canvasOf(page);
  await fill(canvas);
  // The state is posted after a short debounce while the person is in the frame.
  await page.waitForTimeout(600);

  // Far away (at least 1500 px past it): the frame unmounts to a placeholder.
  const scroller = page.locator('.chat-scroll:visible');
  const away = await scroller.evaluate((el) => { const from = el.scrollTop; el.scrollTop = el.scrollHeight; return el.scrollTop - from; });
  expect(away).toBeGreaterThanOrEqual(1500);
  await expect(canvas.locator('[data-frame-placeholder]')).toBeAttached();
  await expect(canvas.locator('iframe')).toHaveCount(0);
  await canvas.scrollIntoViewIfNeeded();
  await expect(canvas.locator('iframe')).toHaveCount(1);
  await expectFilled(canvas);

  // Read as text, then the canvas again: a fresh frame, the same answers.
  await canvas.locator('summary[aria-label="Canvas actions"]').first().click();
  await canvas.getByRole('menuitem', { name: 'Read as text' }).click();
  const text = canvas.getByRole('region', { name: `Text of ${TITLE}` });
  await expect(text).toBeVisible();
  await expect(canvas.locator('iframe')).toHaveCount(0);
  await text.getByRole('button', { name: 'Show canvas' }).click();
  await expectFilled(canvas);

  // And from the canvas's saved state, after the whole window reloads.
  await page.waitForTimeout(1000);
  await page.reload();
  await openChat(page);
  await expectFilled(canvasOf(page));
});

test('a Send from one copy turns the other copy\'s Send off until it settles: one answer message', async ({ page }) => {
  await open(page);
  const canvas = canvasOf(page);
  const panel = await expandToPane(page);
  await expect(panel).toHaveAttribute('data-saved', 'true');
  await fill(canvas);
  await fill(panel);
  const userTurns = await page.locator('.turn.user').count();
  // The send is held at its working-folder check, before anything is queued.
  await page.evaluate(() => {
    let release: () => void = () => {};
    window.__mockHoldOnce = { command: 'chat_preflight', release: new Promise<void>((r) => { release = () => r(); }) };
    (window as unknown as { __releaseAskSend: () => void }).__releaseAskSend = () => release();
  });
  await clickAskSend(sendOf(canvas));
  await expect.poll(() => calls(page, 'chat_preflight')).toBeGreaterThan(0);
  await expect(sendOf(panel)).toHaveAttribute('aria-disabled', 'true');
  await expect(statusOf(panel)).toHaveText('Sending your answers…');
  await sendOf(panel).click({ force: true });
  await page.waitForTimeout(800);
  await page.evaluate(() => (window as unknown as { __releaseAskSend: () => void }).__releaseAskSend());
  await expect(canvas.locator('[data-ask-state="answered"]')).toBeVisible();
  await expect(panel.locator('[data-ask-state="answered"]')).toBeVisible();
  await page.waitForTimeout(800);
  expect(await calls(page, 'chat_send')).toBe(1);
  await expect(page.locator('.turn.user')).toHaveCount(userTurns + 1);
});

test('while the agent is answering, Send reads Queue answers and holds them for after the turn, once', async ({ page }) => {
  await open(page);
  const canvas = canvasOf(page);
  await fill(canvas);
  await page.evaluate(() => localStorage.setItem('mockChatHold', '1'));
  await page.evaluate(async url => (await import(url)).setChatBusy(true), '/src/store/runtime.ts');
  const send = sendOf(canvas, 'Queue answers');
  await expect(send).toBeVisible();
  const userTurns = await page.locator('.turn.user').count();

  await clickAskSend(send);
  await expect.poll(() => calls(page, 'chat_send')).toBe(1);
  const args = await lastArgs(page);
  expect(args.after).toBe('queue');
  expect(args.prompt).toContain(DRAFT);
  expect(await calls(page, 'chat_steer')).toBe(0);
  await expect(statusOf(canvas)).toHaveText('Queued. Your answers go as soon as this turn ends.');
  await expect(send).toHaveAttribute('aria-disabled', 'true');
  // Never parked in the message box.
  await expect(composer(page)).toHaveValue('');

  // A second press, and the side panel's copy, send nothing more.
  await send.click({ force: true });
  const panel = await expandToPane(page);
  await expect(panel).toHaveAttribute('data-saved', 'true');
  await expect(sendOf(panel, 'Queue answers')).toHaveAttribute('aria-disabled', 'true');
  await sendOf(panel, 'Queue answers').click({ force: true });
  await page.waitForTimeout(1500);
  expect(await calls(page, 'chat_send')).toBe(1);
  expect(await calls(page, 'chat_steer')).toBe(0);
  await expect(page.locator('.turn.user')).toHaveCount(userTurns);
});

test('a Send that didn’t go keeps the answers out of the message box; both copies offer Send now and Take them back; the panel’s Send now sends one answer', async ({ page }) => {
  await open(page);
  const canvas = canvasOf(page);
  await fill(canvas);
  const userTurns = await page.locator('.turn.user').count();
  await page.evaluate(() => localStorage.setItem('mockChatSendError', 'A turn is already waiting in this conversation.'));
  await clickAskSend(sendOf(canvas));
  await expect.poll(() => calls(page, 'chat_send')).toBe(1);

  // Not sent, and it says so under the canvas, with what to do; never in the box.
  await expect(unsentOf(canvas)).toContainText('Your answers weren’t sent.');
  await expect(unsentOf(canvas).getByRole('button', { name: 'Send now', exact: true })).toBeVisible();
  await expect(unsentOf(canvas).getByRole('button', { name: 'Take them back', exact: true })).toBeVisible();
  await expect(sendOf(canvas)).toHaveCount(0);
  await expect(composer(page)).toHaveValue('');
  await expect(page.locator('.turn.user')).toHaveCount(userTurns);

  // The side panel's copy shows the same callout.
  const panel = await expandToPane(page);
  await expect(panel).toHaveAttribute('data-saved', 'true');
  await expect(unsentOf(panel)).toContainText('Your answers weren’t sent.');
  await expect(sendOf(panel)).toHaveCount(0);

  // Take them back: the callout goes everywhere, Send is back, the answers stay.
  await unsentOf(canvas).getByRole('button', { name: 'Take them back', exact: true }).click();
  await expect(unsentOf(canvas)).toHaveCount(0);
  await expect(unsentOf(panel)).toHaveCount(0);
  await expect(sendOf(canvas)).toBeVisible();
  await expectFilled(canvas);
  expect(await calls(page, 'chat_send')).toBe(1);

  // Not sent again; then Send now from the panel's copy sends one answer.
  await clickAskSend(sendOf(canvas));
  await expect.poll(() => calls(page, 'chat_send')).toBe(2);
  await expect(unsentOf(panel)).toBeVisible();
  await page.evaluate(() => localStorage.removeItem('mockChatSendError'));
  // Send now collects the form it sits under, as it is now: a check added
  // after the failed send goes too.
  await fill(panel);
  await formIn(panel).getByRole('checkbox', { name: /Bundle budget/ }).check();
  await clickAskSend(unsentOf(panel).getByRole('button', { name: 'Send now', exact: true }));
  await expect.poll(() => calls(page, 'chat_send')).toBe(3);
  expect((await lastArgs(page)).prompt).toContain('"answers":{"ship-day":"Monday, with a second look","checks":["Bed rows","Bundle budget"]}');
  await expect(canvas.locator('[data-ask-state="answered"]')).toBeVisible();
  await expect(panel.locator('[data-ask-state="answered"]')).toBeVisible();
  await expect(unsentOf(canvas)).toHaveCount(0);
  await expect(unsentOf(panel)).toHaveCount(0);
  await expect(page.locator('.turn.user')).toHaveCount(userTurns + 1);
  await page.waitForTimeout(800);
  expect(await calls(page, 'chat_send')).toBe(3);
});

test('an earlier revision of an ask-canvas has no Send; Go to v2 brings the current one into view', async ({ page }) => {
  await open(page, { mockAskCanvasRevised: '1' });
  const fences = page.locator('.turn .chat-canvas[data-canvas-id="release-choices"]');
  await expect(fences).toHaveCount(2);
  const v1 = fences.first();
  const v2 = fences.last();
  await expect(v1).toHaveAttribute('data-revision', '1');
  await expect(v2).toHaveAttribute('data-revision', '2');
  await v1.scrollIntoViewIfNeeded();
  await expect(v1.locator('.chat-ask-send-btn')).toHaveCount(0);
  const go = v1.getByRole('button', { name: 'Go to v2', exact: true });
  await expect(go).toBeVisible();
  await go.click();
  await expect(v2.locator('.chat-ask-send-btn')).toBeInViewport();
  await expect(v2.locator('.chat-ask-send-btn')).toBeFocused();
});
