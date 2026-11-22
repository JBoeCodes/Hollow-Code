import { showWorkspaceList } from './choose';
import { expect, test, type Page } from './fixtures/test';

/**
 * Long chats (overnight 10-11/12 P2). The transcript renders the latest
 * TURN_WINDOW turns of a long thread and its canvases live within a six-frame
 * budget. These cases run on the mock's 1,000-turn thread with
 * `overflow-anchor: none` (WebKit has no scroll anchoring) and pin what a
 * person sees while the window moves: new turns arriving while they read
 * further up, a finished canvas turn, Jump to latest, and the outline.
 */

/** Tauri event delivery on top of the `?mock=1` IPC (as canvas-inline.spec.ts). */
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
const calls = (page: Page, command: string) => page.evaluate(c => window.__mockApiCalls?.[c] ?? 0, command);

const PROSE = (n: number, word: string) => Array.from({ length: n }, (_, i) => `${word} paragraph ${i + 1} of the notes.`).join('\n\n');
const CANVAS = '```hollow-canvas\n<hc-canvas id="long-status" title="Long status" height="300">\n  <p>Still on track</p>\n</hc-canvas>\n```';
const scroller = (page: Page) => page.locator('.work-conversation .chat-scroll');
// A person presses Show earlier from the top of the drawn turns, where the button is. Playwright's click does
// not scroll a view that is pinned to the bottom (the button counts as reachable), and the pinned view then
// follows the new turns down, so the specs set the top the way a reader would arrive, without reader input
// (a programmatic scroll never counts as a wheel or key, so nothing loads by itself).
const atTop = (page: Page) => scroller(page).evaluate(el => { el.scrollTop = 0; });
const centre = async (page: Page): Promise<[number, number]> => { const box = (await scroller(page).boundingBox())!; return [box.x + box.width / 2, box.y + box.height / 2]; };

async function openLongThread(page: Page) {
  await page.goto('/?mock=1&demo=large');
  await showWorkspaceList(page);
  await page.locator('.work-card').filter({ hasText: 'Results view for workspaces' }).click();
  await expect(page.locator('.turn').first()).toBeVisible();
  await expect(page.getByRole('button', { name: /Show 80 earlier messages/ })).toBeVisible();
  await page.addStyleTag({ content: '.chat-scroll, .chat-scroll * { overflow-anchor: none !important; }' });
}

async function ask(page: Page, prompt: string, runId: string) {
  const sends = await calls(page, 'chat_send');
  const composer = page.getByLabel('Chat message');
  await composer.fill(prompt);
  await composer.press('Meta+Enter');
  await expect.poll(() => calls(page, 'chat_send')).toBe(sends + 1);
  await emit(page, { type: 'runStarted', asked: prompt, runId, recall: [] });
}

/** The turn at the top of the transcript's view, and how far below the
 *  scroller's top it sits. */
const reading = (page: Page) => page.evaluate(() => {
  const s = document.querySelector<HTMLElement>('.work-conversation .chat-scroll')!;
  const top = s.getBoundingClientRect().top;
  const t = [...s.querySelectorAll<HTMLElement>('.turn[data-turn-index]')].find(el => el.getBoundingClientRect().bottom > top + 1)!;
  return { index: t.dataset.turnIndex, offset: Math.round(t.getBoundingClientRect().top - top) };
});
/** Resolves once the transcript's scroller has held still (scrollTop and
 *  scrollHeight unchanged) for three consecutive frames. A wheel scroll, a
 *  window re-render or a load's re-pin can still be landing after a threshold
 *  poll passes, so a baseline read straight away can be taken mid-flight.
 *  Gives up after 5 s, naming where the scroller was, so a scroller that never
 *  stops fails here rather than at the test's timeout. */
const scrollSettled = (page: Page) => page.evaluate(() => new Promise<void>((resolve, reject) => {
  const s = document.querySelector<HTMLElement>('.work-conversation .chat-scroll')!;
  const deadline = performance.now() + 5000;
  let last = '', same = 0;
  const tick = () => {
    const now = `${s.scrollTop}|${s.scrollHeight}`;
    same = now === last ? same + 1 : 0;
    last = now;
    if (same >= 3) resolve();
    else if (performance.now() > deadline) reject(new Error(`chat scroller did not settle within 5 s (scrollTop ${s.scrollTop}, scrollHeight ${s.scrollHeight}, clientHeight ${s.clientHeight})`));
    else requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}));
const gap = (page: Page) => page.evaluate(() => { const s = document.querySelector<HTMLElement>('.work-conversation .chat-scroll')!; return Math.round(s.scrollHeight - s.scrollTop - s.clientHeight); });

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name.includes('narrow'), 'one desktop layout is enough for the scroll geometry');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await installEventBridge(page);
});

test('reading further up a long thread, new turns arriving do not move what you read', async ({ page }) => {
  await openLongThread(page);
  await ask(page, 'Keep going with the review.', 'run-long-1');
  await emit(page, { type: 'assistantDelta', text: PROSE(6, 'Opening') });
  await expect.poll(() => gap(page)).toBeLessThan(80);
  // The person scrolls up to read while the reply goes on.
  const box = (await scroller(page).boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.wheel(0, -2400);
  await expect.poll(() => gap(page)).toBeGreaterThan(1500);
  await scrollSettled(page);
  const rendered = () => page.locator('.work-conversation .chat-scroll .turn[data-turn-index]').count();
  const renderedBefore = await rendered();
  const before = await reading(page);
  // New turns arrive below (a queued turn starting, notices): the window of
  // rendered turns moves past the oldest ones above.
  await page.evaluate(async moduleUrl => {
    const runtime = await import(moduleUrl);
    for (let i = 0; i < 6; i++) runtime.pushChatTurn({ role: 'system', text: `Notice ${i + 1}: a background step finished.`, tools: [] });
  }, '/src/store/runtime.ts');
  // The window kept its start: the new turns render below, none above went.
  await expect.poll(rendered).toBe(renderedBefore + 6);
  await expect.poll(() => reading(page)).toEqual(before);
  // Deliberate stillness check: the wait IS the assertion. Having got back
  // there, the reading position must also stay put (no late re-anchor).
  await page.waitForTimeout(300);
  expect(await reading(page)).toEqual(before);
  // Back at the bottom, the window trims to the latest turns again, and the
  // view is still at the end.
  await page.getByRole('button', { name: /Jump to latest/ }).click();
  await expect.poll(rendered).toBeLessThanOrEqual(80);
  await expect.poll(() => gap(page)).toBeLessThan(80);
});

test('turns loaded with Show earlier stay (within the cap) after a visit to the bottom', async ({ page }) => {
  await openLongThread(page);
  const rendered = () => page.locator('.work-conversation .chat-scroll .turn[data-turn-index]').count();
  await atTop(page);
  await page.getByRole('button', { name: /Show 80 earlier messages/ }).click();
  await expect.poll(rendered).toBe(160);
  await page.getByRole('button', { name: /Jump to latest/ }).click();
  await expect.poll(() => gap(page)).toBeLessThan(80);
  await page.mouse.move(...(await centre(page)));
  await page.mouse.wheel(0, -2000);
  // The wheel landed and the scroller is still (any trim it triggered would
  // have re-rendered by now and moved scrollHeight)...
  await expect.poll(() => gap(page)).toBeGreaterThan(300);
  await scrollSettled(page);
  // ...then a deliberate stillness check: the wait IS the assertion that
  // nothing trims the loaded turns afterwards either (160 is within the cap).
  await page.waitForTimeout(300);
  expect(await rendered()).toBe(160);
});

test('reading back draws at most 200 turns: Show earlier trims the end, Show later brings it back, Jump to latest returns', async ({ page }) => {
  await openLongThread(page);
  const rendered = () => page.locator('.work-conversation .chat-scroll .turn[data-turn-index]').count();
  const first = () => page.locator('.work-conversation .chat-scroll .turn[data-turn-index]').first().getAttribute('data-turn-index');
  // Pressing a button scrolls it into view, which is not the reader's input, so nothing loads by itself here.
  for (let i = 0; i < 3; i++) {
    await atTop(page);
    await page.getByRole('button', { name: /^Show (80|20) earlier messages/ }).click();
  }
  await expect.poll(rendered).toBeLessThanOrEqual(200);
  await expect.poll(rendered).toBeGreaterThan(80);
  await expect(page.getByRole('button', { name: /later messages? \d+ below/ })).toBeVisible();
  const before = Number(await first());
  await page.getByRole('button', { name: /^Show 80 later messages/ }).click();
  await expect.poll(rendered).toBeLessThanOrEqual(200);
  // The end moved down; the oldest turns that were off screen went, so the start moved down too (or held).
  await expect.poll(async () => Number(await first())).toBeGreaterThanOrEqual(before);
  // Jump to latest: the latest turns again, no later button, and the end in view.
  await page.getByRole('button', { name: /Jump to latest/ }).click();
  await expect.poll(rendered).toBeLessThanOrEqual(200);
  await expect.poll(() => gap(page)).toBeLessThan(80);
  await expect(page.getByRole('button', { name: /later messages? \d+ below/ })).toHaveCount(0);
});

test('scrolling to the start of what is drawn loads earlier turns by itself, at most 200 drawn', async ({ page }) => {
  await openLongThread(page);
  const rendered = () => page.locator('.work-conversation .chat-scroll .turn[data-turn-index]').count();
  await page.mouse.move(...(await centre(page)));
  for (let i = 0; i < 30; i++) await page.mouse.wheel(0, -1500);
  await expect.poll(rendered).toBeGreaterThan(80);
  await expect.poll(rendered).toBeLessThanOrEqual(200);
});

test('a conversation with turns you have not seen shows where they start, and Dismiss clears it', async ({ page }) => {
  // The mock's long thread has 1,000 turns; the stored visit says 990 were seen.
  await page.addInitScript(() => localStorage.setItem('hollow.chatVisits.v1', JSON.stringify({ 's-demo-longthread': { seen: 990, at: 1 } })));
  await openLongThread(page);
  const mark = page.locator('.chat-visit-mark');
  await expect(mark).toHaveCount(1);
  await expect(mark).toContainText('10 new messages since you were here');
  await expect(mark.locator('xpath=following-sibling::*[1]')).toHaveAttribute('data-turn-index', '990');
  await page.getByRole('button', { name: 'Dismiss the new messages marker' }).click();
  await expect(mark).toHaveCount(0);
});

test('leaving a conversation records how far you read, so a first visit shows no marker', async ({ page }) => {
  await openLongThread(page);
  await expect(page.locator('.chat-visit-mark')).toHaveCount(0);
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: /^Projects/ }).click();
  const seen = await page.evaluate(() => JSON.parse(localStorage.getItem('hollow.chatVisits.v1') ?? '{}')['s-demo-longthread']?.seen);
  expect(seen).toBe(1000);
});

test('in a long thread, a finished canvas turn anchors its canvas; Jump to latest and the outline still work', async ({ page }) => {
  await openLongThread(page);
  await ask(page, 'Where does the review stand?', 'run-long-2');
  await emit(page, { type: 'assistantDelta', text: `Here it is.\n\n${CANVAS}\n\n` });
  await expect(page.locator('.work-conversation iframe[title="Canvas: Long status"]')).toBeVisible();
  await emit(page, { type: 'assistantDelta', text: PROSE(30, 'Closing') });
  await emit(page, { type: 'done', sessionId: null });
  const canvasTop = () => page.evaluate(() => {
    const s = document.querySelector<HTMLElement>('.work-conversation .chat-scroll')!;
    const c = [...s.querySelectorAll<HTMLElement>('.chat-canvas')].pop();
    return c ? Math.round(c.getBoundingClientRect().top - s.getBoundingClientRect().top) : null;
  });
  await expect.poll(async () => { const t = await canvasTop(); return t !== null && t >= 0 && t <= 40; }).toBe(true);
  // Jump to latest goes to the end.
  const box = (await scroller(page).boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.wheel(0, -3000);
  await page.getByRole('button', { name: /Jump to latest/ }).click();
  // Within the app's "at the bottom" band (80 px): something at the end of
  // this fixture grows ~34 px after the jump (open finding, lane A P2).
  await expect.poll(() => gap(page)).toBeLessThan(80);
  // The outline jumps to an early prompt, outside the rendered window.
  await page.getByRole('button', { name: 'Actions for this conversation', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Outline', exact: true }).click();
  await page.getByRole('region', { name: 'Outline', exact: true }).getByRole('button', { name: /Step 2: review/ }).first().click();
  await expect(page.locator('.turn[data-turn-index="4"]')).toBeInViewport();
});


test('a long conversation finishing its load pins an existing reading position', async ({page}) => {
  await openLongThread(page);
  const box = (await scroller(page).boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.wheel(0, -2000);
  await expect.poll(() => gap(page)).toBeGreaterThan(300);
  await scrollSettled(page);
  // Replay the asynchronous loading boundary while the existing scroller is
  // above the bottom. The conversation identity resets its previous pin.
  await page.evaluate(async () => {
    const s = await import('/src/store/runtime.ts');
    s.setChatSessionLoading(true);
    s.setChatConversationVersion(v => v + 1);
  });
  await expect(page.getByText('Opening conversation…', {exact:true})).toBeVisible();
  await page.evaluate(async () => (await import('/src/store/runtime.ts')).setChatSessionLoading(false));
  await expect(page.getByText('Opening conversation…', {exact:true})).toHaveCount(0);
  // The finished load re-pins the reading position: take the baseline once it has.
  await scrollSettled(page);
  const before = await reading(page);
  const renderedBefore = await page.locator('.work-conversation .chat-scroll .turn[data-turn-index]').count();
  await page.evaluate(async () => {
    const s = await import('/src/store/runtime.ts');
    for (let i = 0; i < 6; i++) s.setChatTurns(s.chatTurns.length, {role:'system',text:`Loaded background notice ${i}`,tools:[]});
  });
  await expect.poll(() => page.locator('.work-conversation .chat-scroll .turn[data-turn-index]').count()).toBe(renderedBefore + 6);
  await expect.poll(() => reading(page)).toEqual(before);
});

test('opening and closing the pane keeps the paragraph being read visible', async ({ page }) => {
  await openLongThread(page);
  const box = (await scroller(page).boundingBox())!;
  await page.mouse.move(box.x + box.width / 3, box.y + box.height / 2);
  await page.mouse.wheel(0, -2000);
  await expect.poll(() => gap(page)).toBeGreaterThan(300);
  await scrollSettled(page);
  const text = await scroller(page).evaluate(el => {
    const top = el.getBoundingClientRect().top;
    return [...el.querySelectorAll('.turn p')].find(p => {
      const r = p.getBoundingClientRect();
      return r.bottom > top + 48 && r.top < el.getBoundingClientRect().bottom;
    })!.textContent!;
  });
  const paragraph = scroller(page).locator('.turn p').filter({ hasText: text }).first();
  await page.getByRole('button', { name: 'Project files', exact: true }).click();
  const pane = page.getByRole('complementary', { name: 'Preview', exact: true });
  await expect(pane).toBeVisible();
  await expect(paragraph).toBeInViewport();
  await pane.getByRole('button', { name: /Close.*[Pp]review/ }).click();
  await expect(page.locator('#side-panel-card')).toBeVisible();
  await expect(paragraph).toBeInViewport();
  await expect.poll(() => gap(page)).toBeGreaterThan(300);
});
