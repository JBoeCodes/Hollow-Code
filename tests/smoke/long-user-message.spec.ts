import { expect, test, type Locator, type Page } from './fixtures/test';

/** Overnight 2026-11-04, a C3 follow-up: a long message of yours folds behind
 * Show more (src/userMessageFold.ts), as in ChatGPT and Claude; a short one
 * has no button; Find shows folded messages whole while it searches and keeps
 * the one it was on open as it closes; Copy still copies every word; and both
 * buttons work in place, while you read further up and while a reply streams.
 * Commands go through the `?mock=1` IPC; a turn's events through a Tauri event
 * bridge. Nothing leaves the browser. */

const MOCK_KEY = 's-20260925-mock0001';

/** 40 lines, past the fold's 12. The one word Find looks for is on line 38,
 *  far under the nine lines a folded message shows. */
const LONG = Array.from({ length: 40 }, (_, i) => (i === 37 ? 'Line 38 mentions the zebra migration.' : `Line ${i + 1} of the pasted log.`)).join('\n');
/** Another long message, with no match for Find. */
const OTHER = Array.from({ length: 20 }, (_, i) => `Row ${i + 1} of the other paste.`).join('\n');
/** A reply long enough that the transcript scrolls well past one view. */
const PROSE = Array.from({ length: 30 }, (_, i) => `Paragraph ${i + 1} of the answer.`).join('\n\n');

function callCount(page: Page, command: string) {
  return page.evaluate(cmd => window.__mockApiCalls?.[cmd] ?? 0, command);
}

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

function emit(page: Page, payload: Record<string, unknown>) {
  return page.evaluate(event => {
    (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void }).__emitTauriEvent('chat-event', event);
  }, payload);
}

async function openChat(page: Page) {
  await page.goto('/?mock=1');
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.locator('.settings-index').getByRole('button', { name: 'Advanced', exact: true }).click();
  await page.locator('#settings-group-this-mac > summary').click(); // History & recovery: Advanced → This Mac
  await page.getByRole('button', { name: 'Open Library, History', exact: true }).click();
  await page.locator('.library-tabs').getByRole('button', { name: 'Conversations', exact: true }).click(); // Settings PR 4 cut the Conversation history button
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toBeVisible();
}

/** Send a message and start its turn; returns the turn's tag. */
async function startTurn(page: Page, prompt: string, runId: string) {
  const sends = await callCount(page, 'chat_send');
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await composer.fill(prompt);
  await composer.press('Meta+Enter');
  await expect.poll(() => callCount(page, 'chat_send')).toBe(sends + 1);
  const clientTurnId = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}').clientTurnId);
  const tag = { clientTurnId, sessionKey: MOCK_KEY };
  await emit(page, { type: 'runStarted', runId, recall: [], ...tag });
  return tag;
}

/** Send a message, then answer it and finish its turn. */
async function finishTurn(page: Page, prompt: string, runId: string, answer: string) {
  const tag = await startTurn(page, prompt, runId);
  await emit(page, { type: 'assistantText', text: answer, ...tag });
  await emit(page, { type: 'result', result: answer, costUsd: null, isError: false, subtype: 'success', sessionId: null, ...tag });
  await emit(page, { type: 'done', sessionId: null, ...tag });
}

/** How much of the words the fold keeps out of view, in whole pixels: their
 *  full height (a range over the text, which no overflow clips) less their
 *  box's. A few pixels under 0 when nothing is hidden, since a range covers
 *  the glyphs and not the line's half-leading. */
function hiddenPx(words: Locator) {
  return words.evaluate(el => {
    const range = document.createRange();
    range.selectNodeContents(el);
    return Math.round(range.getBoundingClientRect().height - el.getBoundingClientRect().height);
  });
}

/** How far below the transcript's top `target` sits, in whole pixels. */
function offset(target: Locator) {
  return target.evaluate(el => Math.round(el.getBoundingClientRect().top - el.closest('.chat-scroll')!.getBoundingClientRect().top));
}

/** Scrolls the transcript so `target`'s top sits `at` pixels below its top. */
function placeAt(target: Locator, at: number) {
  return target.evaluate((el, y) => {
    const s = el.closest<HTMLElement>('.chat-scroll')!;
    s.scrollTop += el.getBoundingClientRect().top - s.getBoundingClientRect().top - y;
  }, at);
}

/** Resolves once the transcript's scroller has held still (scrollTop and
 *  scrollHeight unchanged) for three frames in a row, as long-chat.spec.ts. */
const scrollSettled = (page: Page) => page.evaluate(() => new Promise<void>((resolve, reject) => {
  const s = document.querySelector<HTMLElement>('.chat-main .chat-scroll')!;
  const deadline = performance.now() + 5000;
  let last = '', same = 0;
  const tick = () => {
    const now = `${s.scrollTop}|${s.scrollHeight}`;
    same = now === last ? same + 1 : 0;
    last = now;
    if (same >= 3) resolve();
    else if (performance.now() > deadline) reject(new Error(`chat scroller did not settle within 5 s (scrollTop ${s.scrollTop}, scrollHeight ${s.scrollHeight})`));
    else requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}));

test('a long message of yours folds behind Show more; a short one has no button', async ({ page }, info) => {
  test.skip(info.project.name.includes('narrow'), 'one desktop layout is enough: the fold is decided from the text');
  await installEventBridge(page);
  await page.addInitScript(() => {
    Object.defineProperty(navigator.clipboard, 'writeText', { configurable: true, value: async (text: string) => { (window as unknown as { copied: string }).copied = text; } });
  });
  await openChat(page);
  await finishTurn(page, 'A short question', 'run-fold-short', 'A short answer.');
  await finishTurn(page, LONG, 'run-fold-long', 'Noted.');
  const mine = page.locator('.chat-main .turn.user');

  const short = mine.filter({ hasText: 'A short question' });
  await expect(short.locator('.chat-user-chip .text')).toHaveText('A short question');
  await expect(short.getByRole('button', { name: /^Show (more|less)$/ })).toHaveCount(0);
  await expect(short.locator('.chat-user-chip .text')).not.toHaveClass(/chat-user-clipped/);

  // The long one shows its first lines; the rest waits under the fold.
  const long = mine.filter({ hasText: 'Line 1 of the pasted log.' });
  const words = long.locator('.chat-user-chip .text');
  const more = long.getByRole('button', { name: 'Show more', exact: true });
  await expect(more).toBeVisible();
  await expect(more).toHaveAttribute('aria-expanded', 'false');
  const id = await words.getAttribute('id');
  expect(id).toBeTruthy();
  await expect(more).toHaveAttribute('aria-controls', id!);
  await expect(words).toHaveClass(/chat-user-clipped/);
  expect(await hiddenPx(words)).toBeGreaterThan(100);

  // Copy copies every word of a folded message, not the nine lines shown.
  // (First, while the view is at the end: opening leaves it, and Jump to
  // latest then sits over the bottom of the transcript.)
  await long.locator('.chat-user-chip').hover();
  await long.getByRole('group', { name: 'Message actions' }).getByRole('button', { name: 'Copy message', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { copied?: string }).copied ?? '')).toBe(LONG);

  // Show more shows every line and becomes Show less; Show less folds it again.
  await more.click();
  const less = long.getByRole('button', { name: 'Show less', exact: true });
  await expect(less).toHaveAttribute('aria-expanded', 'true');
  await expect(words).not.toHaveClass(/chat-user-clipped/);
  expect(await hiddenPx(words)).toBeLessThanOrEqual(1);
  // A real button: the keyboard works it too.
  await less.press('Enter');
  await expect(more).toHaveAttribute('aria-expanded', 'false');
  await expect(words).toHaveClass(/chat-user-clipped/);
});

test('Find shows folded messages whole while it searches; closing it keeps the one it was on open', async ({ page }, info) => {
  test.skip(info.project.name.includes('narrow'), 'one desktop layout is enough: the fold is decided from the text');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await installEventBridge(page);
  await openChat(page);
  await finishTurn(page, OTHER, 'run-find-other', 'Noted.');
  await finishTurn(page, LONG, 'run-find-long', 'Noted too.');
  const mine = page.locator('.chat-main .turn.user');
  const other = mine.filter({ hasText: 'Row 1 of the other paste.' }).locator('.chat-user-chip .text');
  const long = mine.filter({ hasText: 'Line 1 of the pasted log.' });
  const words = long.locator('.chat-user-chip .text');
  await expect(words).toHaveClass(/chat-user-clipped/);
  await expect(other).toHaveClass(/chat-user-clipped/);

  await page.getByRole('button', { name: 'Actions for this conversation', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Find in this conversation', exact: true }).click();
  const find = page.getByRole('searchbox', { name: 'Find in this conversation' });
  await find.fill('zebra');
  await expect(page.locator('.chat-find-count')).toHaveText('1 of 1');
  // Every long message shows whole while Find searches, so the match on
  // line 38 isn't under a fold; their buttons step aside.
  await expect(words).not.toHaveClass(/chat-user-clipped/);
  await expect(other).not.toHaveClass(/chat-user-clipped/);
  expect(await hiddenPx(words)).toBeLessThanOrEqual(1);
  await expect(mine.getByRole('button', { name: /^Show (more|less)$/ })).toHaveCount(0);

  // Esc: the message Find took you to stays open, with Show less; the
  // other one folds again.
  await find.press('Escape');
  await expect(find).toHaveCount(0);
  await expect(long.getByRole('button', { name: 'Show less', exact: true })).toHaveAttribute('aria-expanded', 'true');
  await expect(words).not.toHaveClass(/chat-user-clipped/);
  expect(await hiddenPx(words)).toBeLessThanOrEqual(1);
  await expect(words).toBeInViewport();
  await expect(other).toHaveClass(/chat-user-clipped/);
});

test('Show more and Show less work in place while you read further up', async ({ page }, info) => {
  test.skip(info.project.name.includes('narrow'), 'one desktop layout is enough for the scroll geometry');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await installEventBridge(page);
  await openChat(page);
  // As in WebKit, which has no scroll anchoring of its own: only Hollow's
  // reading hold (chatReadingAnchor.ts) moves the view.
  await page.addStyleTag({ content: '.chat-scroll, .chat-scroll * { overflow-anchor: none !important; }' });
  await finishTurn(page, 'A short question', 'run-place-short', PROSE);
  await finishTurn(page, LONG, 'run-place-long', PROSE);
  const long = page.locator('.chat-main .turn.user').filter({ hasText: 'Line 1 of the pasted log.' });
  const words = long.locator('.chat-user-chip .text');
  const more = long.getByRole('button', { name: 'Show more', exact: true });
  const less = long.getByRole('button', { name: 'Show less', exact: true });
  await expect(more).toBeVisible();

  // Further up, the message's first line near the top and Show more on
  // screen under it: the words open downward, and nothing scrolls them off
  // the top (holding the button still would have).
  await placeAt(words, 8);
  await scrollSettled(page);
  const wordsAt = await offset(words);
  // Pressed as a person does: down, a pause, up. A press with no pause can
  // land before the reading hold is taken again, and then proves nothing.
  await more.click({ delay: 100 });
  await expect(less).toHaveAttribute('aria-expanded', 'true');
  await scrollSettled(page);
  expect(Math.abs((await offset(words)) - wordsAt)).toBeLessThanOrEqual(1);

  // Further down, the words' first lines above the view: Show less from the
  // keyboard (Enter resets the hold and clicks in one go, so nothing holds
  // the button) stays where it was as the words fold up above it.
  await placeAt(less, 300);
  await scrollSettled(page);
  expect(await offset(words)).toBeLessThan(0);
  const buttonAt = await offset(less);
  await less.press('Enter');
  await expect(more).toHaveAttribute('aria-expanded', 'false');
  await expect(more).toBeFocused();
  await scrollSettled(page);
  expect(Math.abs((await offset(more)) - buttonAt)).toBeLessThanOrEqual(1);
});

test('Show more while a reply streams lets go of the end: the words stay put and Jump to latest shows', async ({ page }, info) => {
  test.skip(info.project.name.includes('narrow'), 'one desktop layout is enough for the scroll geometry');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await installEventBridge(page);
  await openChat(page);
  await page.addStyleTag({ content: '.chat-scroll, .chat-scroll * { overflow-anchor: none !important; }' });
  const tag = await startTurn(page, LONG, 'run-stream-long');
  await emit(page, { type: 'assistantDelta', text: 'Reading the log.', ...tag });
  const long = page.locator('.chat-main .turn.user').filter({ hasText: 'Line 1 of the pasted log.' });
  const words = long.locator('.chat-user-chip .text');
  const more = long.getByRole('button', { name: 'Show more', exact: true });
  const jump = page.locator('.chat-main .chat-jump-latest');
  // Following the reply at the end, the folded message on screen above it
  // (a click that had to scroll to it would leave the end by itself).
  await expect(more).toBeInViewport();
  await expect(jump).toHaveCount(0);
  // The streaming caret goes after a quiet second (streamCaret.ts): measure
  // once it has, so nothing under the words changes while the view follows.
  await expect(page.locator('.chat-main .chat-msg.typing')).toHaveCount(0);
  await scrollSettled(page);
  const wordsAt = await offset(words);

  await more.click({ delay: 100 });
  await expect(long.getByRole('button', { name: 'Show less', exact: true })).toHaveAttribute('aria-expanded', 'true');
  // The words grew down from where they were and the end went below the
  // view: no longer following, so Jump to latest offers the end.
  await expect(jump).toBeVisible();
  await scrollSettled(page);
  expect(Math.abs((await offset(words)) - wordsAt)).toBeLessThanOrEqual(1);

  // The reply goes on below; nothing pulls the view down after it.
  for (let i = 0; i < 3; i++) await emit(page, { type: 'assistantDelta', text: `\n\n${PROSE}`, ...tag });
  await scrollSettled(page);
  expect(Math.abs((await offset(words)) - wordsAt)).toBeLessThanOrEqual(1);
  await expect(jump).toBeVisible();
  await emit(page, { type: 'done', sessionId: null, ...tag });
});
