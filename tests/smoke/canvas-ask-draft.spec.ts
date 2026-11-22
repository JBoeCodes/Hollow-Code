import { expect, test, type FrameLocator, type Locator, type Page } from './fixtures/test';
import { clickAskSend } from './askSend';
import { COLLECT_TIMEOUT_MS } from '../../src/canvasBridge';

/**
 * Ask-canvas answers (Canvas C4 A2; question cards, 2026-10-14): Hollow
 * draws the Send button under the canvas, and pressing it sends the answers
 * straight to the agent as the person's next message, in every permission
 * mode. The sent message shows its readable lines while the exact JSON
 * rides along in a hidden `hollow-canvas-answer` block; the canvas then
 * locks (fields inert, Send gone, the answer shown, "Answered" under it) and
 * stays locked after a reload, because the lock is read from the
 * conversation's own transcript. What the person typed in the message box is
 * left alone. A canvas script can never send: the host takes answers only
 * as the reply to its own Send's `collect`, so drafts a script posts on its
 * own are refused and logged.
 *
 * `mockAskCanvasFixture` adds an ask-canvas reply to "Wire the session
 * store" (saved as v1 in that conversation's store), and replays the last
 * sent answer as the next user turn, as the backend's transcript would.
 */

const TITLE = 'Release choices';
const DRAFT = [
  'Answered canvas "release-choices":',
  '- When should the release ship?: Friday, after the bed passes',
  '- Which checks must pass first?: Bundle budget',
  '',
  '```hollow-canvas-answer',
  '{"canvasId":"release-choices","revision":1,"answers":{"ship-day":"Friday, after the bed passes","checks":["Bundle budget"]}}',
  '```',
].join('\n');
const LINES = '- When should the release ship?: Friday, after the bed passes\n- Which checks must pass first?: Bundle budget';

const calls = (page: Page, command: string) => page.evaluate(c => window.__mockApiCalls?.[c] ?? 0, command);
const composer = (page: Page) => page.getByRole('textbox', { name: 'Chat message' });
/** The canvas in the transcript (the side panel may show a copy too). */
const canvasOf = (page: Page) => page.locator('.turn .chat-canvas[data-canvas-id="release-choices"]');
const frameOf = (page: Page): FrameLocator => canvasOf(page).frameLocator(`iframe[title="Canvas: ${TITLE}"]`);
const formOf = (page: Page): Locator => frameOf(page).getByRole('form', { name: TITLE });
/** Hollow's own Send under the canvas. */
const sendOf = (canvas: Locator, name = 'Send answers') => canvas.getByRole('group', { name: 'Send this canvas’s answers' }).getByRole('button', { name, exact: true });
const statusOf = (canvas: Locator) => canvas.locator('.chat-ask-send-status');

async function openConversation(page: Page) {
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
  await expect(formOf(page).locator('.hc-ask-foot')).toBeAttached();
}

/** Every permission mode, Bypass included. */
async function bypass(page: Page) {
  await page.evaluate(async (url) => {
    const runtime = await import(url);
    runtime.setChatPermissionMode('bypassPermissions');
  }, '/src/store/runtime.ts');
  expect(await page.evaluate(async (url) => (await import(url)).chatPermissionMode(), '/src/store/runtime.ts')).toBe('bypassPermissions');
}

async function fill(page: Page, checks = 'Bundle budget') {
  const form = formOf(page);
  await form.getByRole('radio', { name: /Friday, after the bed passes/ }).check();
  await form.getByRole('checkbox', { name: new RegExp(checks) }).check();
}

/** Nothing queued, nothing sent: the turn queue is untouched. */
async function nothingSent(page: Page, userTurns: number) {
  await page.waitForTimeout(1200);
  expect(await calls(page, 'chat_send')).toBe(0);
  await expect(page.locator('.chat-queued-badge')).toHaveCount(0);
  await expect(page.locator('.turn.user')).toHaveCount(userTurns);
  expect(await page.evaluate(async (url) => (await import(url)).chatQueuedTurn(), '/src/store/runtime.ts')).toBeNull();
}

/** `check` passes at once and after every 100 ms step over `ms` of the
 *  page's own time: a "nothing happened" that one read straight after a
 *  click can't show, since a click's collect (a round trip to the frame) and
 *  any send land after the click returns.
 *
 *  For a `@clock` test (EH6). The page's clock stops, then runs forward a
 *  step at a time, so a timer that would do something late (a collect's
 *  COLLECT_TIMEOUT_MS, a script's loop) fires inside the window however
 *  slow the machine is; sampling in real time only covered whatever
 *  happened to fire before the samples ran out. Work on no timer (the
 *  frame's postMessage replies, a send's promise chain) gets a short
 *  real-time wait after each step. Time runs again afterwards.
 *
 *  The canvas is a sandboxed srcdoc frame (`allow-scripts`, no
 *  `allow-same-origin`). Playwright's clock is per frame: an init script in
 *  every frame of the context, then each call evaluated in every frame, so
 *  it reaches this one too. The first and last looks check that the frame's
 *  Date.now() is the page's stopped time. If that ever fails, the frame's
 *  timers run in real time, and this has to go back to sampling in real
 *  time (for 2000 ms or more) instead. */
async function holdsFor(page: Page, ms: number, check: () => Promise<void>) {
  const frameKeepsTime = async () => {
    const [inPage, inFrame] = [await page.evaluate(() => Date.now()), await frameOf(page).locator('body').evaluate(() => Date.now())];
    expect(inFrame, 'the canvas frame runs on the page\'s clock').toBe(inPage);
  };
  // Pausing can only go forward: stop a little ahead of where the page is.
  await page.clock.pauseAt(await page.evaluate(() => Date.now()) + 1000);
  await frameKeepsTime();
  await check();
  for (let ran = 0; ran < ms; ran += 100) {
    await page.clock.runFor(100);
    await page.waitForTimeout(50);
    await check();
  }
  await frameKeepsTime();
  await page.clock.resume();
}
/** Past a collect's timeout, so one that started late would end inside. */
const HOLD_MS = COLLECT_TIMEOUT_MS + 500;

const NONCE = '0123456789abcdef0123456789abcdef';
/** A draft the canvas's own script posts, as if from a loop in the frame. */
const scriptDraft = (page: Page, summary: string) => frameOf(page).locator('body').evaluate((_, [s, nonce]) => {
  parent.postMessage({ __hc: 1, kind: 'draft', canvasId: 'release-choices', answers: { 'ship-day': s }, summary: `Ship day: ${s}`, nonce }, '*');
}, [summary, NONCE] as const);

async function expectLocked(page: Page) {
  const canvas = canvasOf(page);
  await expect(canvas.locator('[data-ask-state="answered"]')).toHaveText('Answered. You sent this canvas’s answers.');
  await expect(canvas.locator('[data-ask-state="waiting"]')).toHaveCount(0);
  const form = formOf(page);
  await expect(form.getByRole('status')).toHaveText('Answered. You sent these answers.');
  await expect(frameOf(page).locator('.hc-ask-copy textarea')).toHaveValue(LINES);
  await expect(canvas.locator('.chat-ask-send')).toHaveCount(0);
  expect(await frameOf(page).locator('hc-choice, hc-multi').evaluateAll(els => els.map(e => (e as HTMLElement).inert))).toEqual([true, true]);
}

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name.includes('narrow'), 'Conversation history opens from the desktop settings index');
  // A `@clock` test holds a window on the page's time (holdsFor): the fake
  // clock has to be in before the page loads, so the app's timers and the
  // canvas frame's are Playwright's. It runs at real speed until paused.
  if (info.tags.includes('@clock')) await page.clock.install();
  await page.addInitScript(() => localStorage.setItem('mockAskCanvasFixture', '1'));
  await page.goto('/?mock=1');
});

test('Hollow\'s Send under an ask-canvas sends the answers straight away, even in Bypass; the canvas locks and stays locked after a reload', async ({ page }) => {
  await openConversation(page);
  // No Send inside the frame: the button is Hollow's.
  await expect(frameOf(page).locator('.hc-ask-send')).toHaveCount(0);
  await expect(sendOf(canvasOf(page))).toBeVisible();
  await bypass(page);
  await expect(canvasOf(page).locator('[data-ask-state="waiting"]')).toHaveText('Waiting on your answer');
  await composer(page).fill('My own note about the release');

  await fill(page);
  await clickAskSend(sendOf(canvasOf(page)));
  await expect.poll(() => calls(page, 'chat_send')).toBe(1);
  const args = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}'));
  expect(args.permissionMode).toBe('bypassPermissions');
  // The agent gets the exact JSON; the bubble shows only the lines.
  expect(args.prompt).toContain(DRAFT);
  const bubble = page.locator('.turn.user .chat-user-chip .text').last();
  await expect(bubble).toHaveText(`Answered canvas "release-choices":\n${LINES}`);
  await expect(page.locator('.turn.user').last()).not.toContainText('hollow-canvas-answer');
  await expect(page.locator('.turn.user').last()).not.toContainText('"canvasId"');
  await expectLocked(page);
  // What the person typed is still there, unsent.
  await expect(composer(page)).toHaveValue('My own note about the release');

  // Locked: whatever its script posts now, nothing more is sent.
  await scriptDraft(page, 'Monday');
  await page.waitForTimeout(400);
  expect(await calls(page, 'chat_send')).toBe(1);

  // A reload reopens the conversation from its transcript: still locked.
  await page.reload();
  await openConversation(page);
  await expect(page.locator('.turn.user .chat-user-chip .text').last()).toHaveText(`Answered canvas "release-choices":\n${LINES}`);
  await expectLocked(page);
  await scriptDraft(page, 'Monday');
  await page.waitForTimeout(400);
  expect(await calls(page, 'chat_send')).toBe(0);
});

test('answers that need attention stay in the canvas and Hollow says how many; nothing sends', async ({ page }) => {
  await openConversation(page);
  const userTurns = await page.locator('.turn.user').count();
  await clickAskSend(sendOf(canvasOf(page)));
  await expect(statusOf(canvasOf(page))).toHaveText(/answers? needs? attention\./);
  await expect(formOf(page).getByRole('status')).toHaveText(/needs? attention/);
  await nothingSent(page, userTurns);
  // Fixed, the same button sends.
  await fill(page);
  await clickAskSend(sendOf(canvasOf(page)));
  await expect.poll(() => calls(page, 'chat_send')).toBe(1);
  await expectLocked(page);
});

test('the Send button takes the form\'s own words, and each canvas sends only its own answers (C6 P1)', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockAskCanvasSecond', '1'));
  await page.reload();
  await openConversation(page);
  const owner = page.locator('.turn .chat-canvas[data-canvas-id="release-owner"]');
  await expect(owner).toHaveAttribute('data-saved', 'true');
  const ownerForm = owner.frameLocator('iframe[title="Canvas: Release owner"]').getByRole('form', { name: 'Release owner' });
  await ownerForm.getByRole('radio', { name: 'Grace' }).check();
  await clickAskSend(sendOf(owner, 'Send owner'));
  await expect.poll(() => calls(page, 'chat_send')).toBe(1);
  const args = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}'));
  expect(args.prompt).toContain('Answered canvas "release-owner":\n- Who ships it?: Grace\n');
  expect(args.prompt).not.toContain('release-choices');
  await expect(owner.locator('[data-ask-state="answered"]')).toBeVisible();
  // Release choices is still waiting on its own answers.
  await expect(canvasOf(page).locator('[data-ask-state="waiting"]')).toBeVisible();
});

test('a canvas script posting drafts in a loop never sends, and each refusal is logged without what it said', { tag: '@clock' }, async ({ page }) => {
  await openConversation(page);
  await bypass(page);
  const userTurns = await page.locator('.turn.user').count();
  // The person clicks into the canvas; its script then floods the bridge,
  // with and without a nonce it made up.
  await frameOf(page).locator('body').click({ position: { x: 4, y: 4 } });
  await frameOf(page).locator('body').evaluate((_, nonce) => {
    const post = (n: number) => parent.postMessage({ __hc: 1, kind: 'draft', canvasId: 'release-choices', answers: { 'ship-day': `loop ${n}` }, summary: `Ship day: loop ${n}`, nonce }, '*');
    for (let n = 0; n < 50; n++) post(n);
    let n = 50;
    (window as unknown as { __loop: number }).__loop = window.setInterval(() => post(n++), 50);
  }, NONCE);
  await nothingSent(page, userTurns);
  await expect(composer(page)).toHaveValue('');
  // C6 P1: the refused drafts are logged (at most thirty a minute), by
  // kind, reason and ids, never what they said.
  const logged = await page.evaluate(() => (window as unknown as { __mockBridgeRefusals?: Array<Record<string, unknown>> }).__mockBridgeRefusals ?? []);
  expect(logged.length).toBeGreaterThan(0);
  expect(logged.length).toBeLessThanOrEqual(30);
  for (const r of logged) expect(r).toMatchObject({ kind: 'draft', reason: 'unrequested', canvasId: 'release-choices', conversation: 's-20260811-fixture1' });
  expect(JSON.stringify(logged)).not.toContain('loop');

  // The person's own Send still works while the loop runs: their answers go.
  await fill(page);
  await clickAskSend(sendOf(canvasOf(page)));
  await expect.poll(() => calls(page, 'chat_send')).toBe(1);
  expect((await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}'))).prompt).toContain(DRAFT);
  // And no second send while the loop goes on (EH6): its interval runs on
  // the frame's clock, so it keeps posting through every step.
  await holdsFor(page, HOLD_MS, async () => {
    expect(await calls(page, 'chat_send')).toBe(1);
  });
  await expect(page.locator('.turn.user')).toHaveCount(userTurns + 1);
});

test('a canvas script that answers its own keys never sends, and never moves focus', async ({ page }) => {
  await openConversation(page);
  await bypass(page);
  const userTurns = await page.locator('.turn.user').count();
  await frameOf(page).locator('body').evaluate((_, nonce) => {
    const field = document.createElement('textarea');
    field.id = 'own-field';
    field.setAttribute('aria-label', 'Own field');
    field.addEventListener('keydown', () => parent.postMessage({ __hc: 1, kind: 'draft', canvasId: 'release-choices', answers: { 'ship-day': 'now' }, summary: 'Ship day: now', nonce }, '*'));
    document.body.prepend(field);
  }, NONCE);
  const own = frameOf(page).locator('#own-field');
  await own.click();
  await page.keyboard.press('a');
  await page.keyboard.press('Enter');
  await page.keyboard.press('b');
  await nothingSent(page, userTurns);
  await expect(own).toHaveValue('a\nb');
  await expect(composer(page)).toHaveValue('');
  await expect(composer(page)).not.toBeFocused();
});

test('a canvas script listening for Hollow\'s collect can\'t answer it: the form\'s own answers are sent (overnight 10 A7)', async ({ page, pageErrors }) => {
  pageErrors.allow(/a canvas script quiets the form/, 'the attack quiets the kit by making its form lookup throw');
  await openConversation(page);
  const canvas = canvasOf(page);
  // The canvas's own script, which runs after the kit, keeps the form quiet
  // (`document.querySelectorAll('hc-ask')` throws, so the kit's `collect`
  // finds no form and posts nothing) and listens in the capture phase for
  // Hollow's `collect`, to answer it with the nonce and answers of its own.
  // Before the fix the kit let every `collect` on to the frame's other
  // listeners, so this heard the nonce and its answer was sent. Quieting the
  // form is what lets this reach the pre-fix kit in Chromium, which runs a
  // window's listeners in the order they were added whatever the phase (the
  // kit, added first, answered first); WebKit runs capture listeners first at
  // the target. (The kit's own method names are minified, so the script
  // quiets it through the page instead.)
  await frameOf(page).locator('body').evaluate(() => {
    const w = window as unknown as { __heard: number };
    w.__heard = 0;
    const all = Document.prototype.querySelectorAll;
    Object.defineProperty(document, 'querySelectorAll', {
      configurable: true, writable: true,
      value(this: Document, selector: string) {
        if (selector === 'hc-ask') throw new Error('a canvas script quiets the form');
        return all.call(this, selector);
      },
    });
    addEventListener('message', e => {
      const d = e.data;
      if (!d || d.__hc !== 1 || d.kind !== 'collect') return;
      w.__heard++;
      e.stopImmediatePropagation();
      parent.postMessage({ __hc: 1, kind: 'draft', canvasId: 'release-choices', answers: { pick: 'EVIL' }, summary: 'Pick: EVIL', nonce: d.nonce }, '*');
    }, true);
  });
  await fill(page);
  await clickAskSend(sendOf(canvas));
  // The collect settles: something sent, or nothing handed over in its time.
  const notHanded = 'The canvas didn’t hand over its answers.';
  const lastSent = () => page.evaluate(() => localStorage.getItem('mockLastChatArgs'));
  await expect.poll(async () => await lastSent() !== null || (await statusOf(canvas).allTextContents()).includes(notHanded), { timeout: COLLECT_TIMEOUT_MS + 5000 }).toBe(true);
  expect(JSON.parse(await lastSent() ?? '{}').prompt ?? '', 'the canvas script\'s answer was sent').not.toContain('EVIL');
  expect(await frameOf(page).locator('body').evaluate(() => (window as unknown as { __heard: number }).__heard), 'the canvas script heard Hollow\'s collect').toBe(0);
  expect(await calls(page, 'chat_send')).toBe(0);
  await expect(statusOf(canvas)).toHaveText(notHanded);
  // The script lets the form answer again: the next Send sends the form's own.
  await frameOf(page).locator('body').evaluate(() => { delete (document as unknown as { querySelectorAll?: unknown }).querySelectorAll; });
  await clickAskSend(sendOf(canvas));
  await expect.poll(() => calls(page, 'chat_send')).toBe(1);
  const args = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}'));
  expect(args.prompt).not.toContain('EVIL');
  expect(args.prompt).toContain(DRAFT);
  expect(await frameOf(page).locator('body').evaluate(() => (window as unknown as { __heard: number }).__heard)).toBe(0);
  await expect(page.locator('.turn.user .chat-user-chip .text').last()).toHaveText(`Answered canvas "release-choices":\n${LINES}`);
  await expectLocked(page);
});

test('the canvas expanded in the Preview pane sends to the same conversation and locks with it', async ({ page }) => {
  await openConversation(page);
  await canvasOf(page).getByRole('button', { name: 'Expand', exact: true }).click();
  const panel = page.getByRole('complementary', { name: 'Preview' }).locator('.chat-canvas[data-canvas-id="release-choices"]');
  await expect(panel).toHaveAttribute('data-saved', 'true');
  const form = panel.frameLocator(`iframe[title="Canvas: ${TITLE}"]`).getByRole('form', { name: TITLE });
  await form.getByRole('radio', { name: /Friday, after the bed passes/ }).check();
  await form.getByRole('checkbox', { name: /Bundle budget/ }).check();
  await clickAskSend(sendOf(panel));
  await expect.poll(() => calls(page, 'chat_send')).toBe(1);
  expect((await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}'))).prompt).toContain(DRAFT);
  await expect(panel.locator('[data-ask-state="answered"]')).toBeVisible();
  await expect(panel.locator('.chat-ask-send')).toHaveCount(0);
  await expectLocked(page);
});

// A Send that didn't go (its callout, Send now and Take them back, in both
// copies) is P1-33's `canvas-ask-survive.spec.ts`; it never uses the box.
test('Send is off while a send goes: a second click collects and sends nothing; released, the answers go once (2026-10-15)', { tag: '@clock' }, async ({ page }) => {
  await openConversation(page);
  const canvas = canvasOf(page);
  const send = sendOf(canvas);
  const userTurns = await page.locator('.turn.user').count();
  // Held mid-send (before the turn is queued), Send is off, and a click does
  // nothing; released, the answers go once and the canvas locks.
  await fill(page);
  const preflights = await calls(page, 'chat_preflight');
  await page.evaluate(() => {
    let release: () => void = () => {};
    window.__mockHoldOnce = { command: 'chat_preflight', release: new Promise<void>((r) => { release = () => r(); }) };
    (window as unknown as { __releaseAskSend: () => void }).__releaseAskSend = () => release();
  });
  await clickAskSend(send);
  // Taken by the send and held: the turn isn't queued yet.
  await expect.poll(() => calls(page, 'chat_preflight')).toBe(preflights + 1);
  await expect(send).toHaveAttribute('aria-disabled', 'true');
  await expect(statusOf(canvas)).toHaveText('Sending your answers…');
  // The reason is tied to the button for screen readers.
  const statusId = await statusOf(canvas).getAttribute('id');
  expect(statusId).toBeTruthy();
  await expect(send).toHaveAttribute('aria-describedby', statusId!);
  await send.click({ force: true });
  // Nothing more over a while (EH6): a second collect would start a second
  // preflight (only the first is held) or change the status, and a second
  // send would reach chat_send; neither can show in one read straight away.
  await holdsFor(page, HOLD_MS, async () => {
    expect(await calls(page, 'chat_preflight')).toBe(preflights + 1);
    expect(await calls(page, 'chat_send')).toBe(0);
    expect(await statusOf(canvas).textContent()).toBe('Sending your answers…');
    expect(await send.getAttribute('aria-disabled')).toBe('true');
    expect(await send.getAttribute('aria-describedby')).toBe(statusId);
  });
  await page.evaluate(() => (window as unknown as { __releaseAskSend: () => void }).__releaseAskSend());
  await expect.poll(() => calls(page, 'chat_send')).toBe(1);
  await expectLocked(page);
  expect(await calls(page, 'chat_send')).toBe(1);
  await expect(page.locator('.turn.user')).toHaveCount(userTurns + 1);
});
