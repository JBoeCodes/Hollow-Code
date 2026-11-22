import { expect, test, type Page } from './fixtures/test';
import { chooseNew } from './choose';

/**
 * Canvas C3: the transcript's frame budget. At most six canvas and html
 * frames are live in a transcript; one scrolled out of view unmounts to a
 * placeholder of the height it had, and mounts again when it comes back,
 * with the person's view state restored through bridge `init`. Every canvas
 * is still saved once, live or not.
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

const COUNT = 12;
const BUDGET = 6;
/** A canvas of a fixed content height, with tabs whose choice is view state. */
const canvas = (n: number) => `\`\`\`hollow-canvas
<hc-canvas id="lane-${n}" title="Lane ${n}">
  <hc-tabs label="Views">
    <section label="Summary"><div style="height:200px">Lane ${n} summary</div></section>
    <section label="Detail"><div style="height:200px">Lane ${n} detail</div></section>
  </hc-tabs>
</hc-canvas>
\`\`\``;

async function openWorkspace(page: Page) {
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Track twelve lanes.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Lanes');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Lanes');
}

const scroller = (page: Page) => page.locator('.work-conversation .chat-scroll');
/** Live frames in the transcript, and each canvas's slot: live or not, and its height. */
const census = (page: Page) => page.locator('.work-conversation').evaluate(root => ({
  frames: root.querySelectorAll('iframe.chat-artifact-frame').length,
  slots: [...root.querySelectorAll<HTMLElement>('.chat-canvas')].map(c => {
    const slot = c.querySelector<HTMLElement>('.chat-artifact-slot');
    return { id: c.dataset.canvasId, live: slot?.dataset.frameLive === 'true', frame: !!slot?.querySelector('iframe'), height: Math.round(slot?.getBoundingClientRect().height ?? 0) };
  }),
}));
/** The person scrolls there: a wheel, then the move (a bare move up just after
 *  frames park or unpark is a layout clamp since the chat jump fix, 11-10). */
const scrollTo = (page: Page, where: 'top' | 'bottom' | number) => scroller(page).evaluate((el, to) => {
  el.dispatchEvent(new WheelEvent('wheel', { deltaY: to === 'bottom' ? 100 : -100, bubbles: true }));
  el.scrollTop = to === 'top' ? 0 : to === 'bottom' ? el.scrollHeight : to;
}, where);
const frameOf = (page: Page, n: number) => page.locator(`.chat-canvas[data-canvas-id="lane-${n}"]`).frameLocator('iframe');

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name.includes('narrow'), 'desktop layout; the budget is the same');
  await installEventBridge(page);
});

test('a twelve-canvas transcript keeps at most six frames live, holds each place at its height, and restores state on return', async ({ page }) => {
  test.setTimeout(120_000);
  await openWorkspace(page);
  const composer = page.getByLabel('Chat message');
  await composer.fill('Show every lane');
  await composer.press('Meta+Enter');
  await expect.poll(() => calls(page, 'chat_send')).toBe(1);
  await emit(page, { type: 'runStarted', asked: 'Show every lane', runId: 'run-lanes', recall: [] });

  // The reply streams one canvas at a time; each mounts as it arrives at the
  // bottom, and the budget never lets more than six be live.
  let most = 0;
  for (let n = 1; n <= COUNT; n++) {
    await emit(page, { type: 'assistantDelta', text: `${n === 1 ? 'Every lane.\n\n' : '\n\n'}${canvas(n)}` });
    await scrollTo(page, 'bottom');
    await expect(frameOf(page, n).getByText(`Lane ${n} summary`)).toBeVisible();
    most = Math.max(most, (await census(page)).frames);
  }
  await emit(page, { type: 'done', sessionId: null });
  expect(most).toBeLessThanOrEqual(BUDGET);

  // Every canvas was saved once, live or not.
  await expect.poll(() => calls(page, 'record_canvas')).toBe(COUNT);

  // At the bottom: at most six frames, the rest placeholders, and every place
  // keeps the height its frame had (the frames all have one height).
  await scrollTo(page, 'bottom');
  await expect.poll(async () => (await census(page)).frames).toBeLessThanOrEqual(BUDGET);
  // Let the newest frames report their size (and the height transition end).
  await expect.poll(async () => new Set((await census(page)).slots.map(s => s.height)).size, { timeout: 5000 }).toBe(1);
  const bottom = await census(page);
  expect(bottom.slots).toHaveLength(COUNT);
  const liveHeight = bottom.slots.find(s => s.frame)!.height;
  expect(liveHeight).toBeGreaterThan(200);
  const parked = bottom.slots.filter(s => !s.live);
  expect(parked.length).toBeGreaterThanOrEqual(COUNT - BUDGET);
  for (const s of bottom.slots) expect(Math.abs(s.height - liveHeight), `${s.id} keeps its height`).toBeLessThanOrEqual(1);
  await expect(page.locator('.chat-canvas[data-canvas-id="lane-1"] [data-frame-placeholder]')).toHaveCount(1);
  await expect(page.locator('.chat-canvas[data-canvas-id="lane-1"] iframe')).toHaveCount(0);

  // Back to the top: the first canvas mounts again. The person picks a tab.
  const height = await scroller(page).evaluate(el => el.scrollHeight);
  await scrollTo(page, 'top');
  await expect(frameOf(page, 1).getByText('Lane 1 summary')).toBeVisible();
  expect(Math.abs(await scroller(page).evaluate(el => el.scrollHeight) - height)).toBeLessThanOrEqual(2);
  await frameOf(page, 1).getByRole('tab', { name: 'Detail' }).click();
  await expect(frameOf(page, 1).getByText('Lane 1 detail')).toBeVisible();
  await expect.poll(() => calls(page, 'save_canvas_state')).toBeGreaterThanOrEqual(1);

  // Scroll down in steps: never more than six live at any stop.
  const total = await scroller(page).evaluate(el => el.scrollHeight - el.clientHeight);
  for (let at = 0; at <= total; at += 250) {
    await scrollTo(page, at);
    await page.waitForTimeout(60);
    const now = await census(page);
    expect(now.frames, `frames live at ${at}px`).toBeLessThanOrEqual(BUDGET);
  }
  await scrollTo(page, 'bottom');
  await expect(page.locator('.chat-canvas[data-canvas-id="lane-1"] iframe')).toHaveCount(0);

  // And back: lane 1 mounts again on the tab the person chose (bridge init).
  await scrollTo(page, 'top');
  await expect(frameOf(page, 1).getByRole('tab', { name: 'Detail' })).toHaveAttribute('aria-selected', 'true');
  await expect(frameOf(page, 1).getByText('Lane 1 detail')).toBeVisible();
  // Remounting reads nothing new into the store and records nothing.
  expect(await calls(page, 'record_canvas')).toBe(COUNT);
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});

test('a placeholder on screen offers Show, which mounts its frame in place of the oldest', async ({ page }) => {
  // Tall enough that all eight frames are on screen at once.
  await page.setViewportSize({ width: 1280, height: 2400 });
  await openWorkspace(page);
  const composer = page.getByLabel('Chat message');
  await composer.fill('Show small lanes');
  await composer.press('Meta+Enter');
  await expect.poll(() => calls(page, 'chat_send')).toBe(1);
  await emit(page, { type: 'runStarted', asked: 'Show small lanes', runId: 'run-small', recall: [] });
  // Eight one-line html fences: more frames than the budget fit on screen at once.
  const html = Array.from({ length: 8 }, (_, i) => `\`\`\`html\n<p>Row ${i + 1}</p>\n\`\`\``).join('\n\n');
  await emit(page, { type: 'assistantText', text: html });
  await emit(page, { type: 'done', sessionId: null });
  const artifacts = page.locator('.work-conversation .chat-artifact');
  await expect(artifacts).toHaveCount(8);
  await expect.poll(async () => (await census(page)).frames).toBe(BUDGET);
  const slots = page.locator('.work-conversation .chat-artifact-slot');
  const parked = await slots.evaluateAll(els => els.map((el, i) => (el.querySelector('[data-frame-placeholder] button') ? i : -1)).filter(i => i >= 0));
  expect(parked).toHaveLength(8 - BUDGET);
  const pick = slots.nth(parked[0]);
  await expect(pick.getByRole('button', { name: 'Show HTML' })).toBeVisible();
  await pick.getByRole('button', { name: 'Show HTML' }).click();
  await expect(pick.locator('iframe')).toHaveCount(1);
  await expect(pick.locator('[data-frame-placeholder]')).toHaveCount(0);
  // Another frame on screen gave up its place: still six.
  expect((await census(page)).frames).toBe(BUDGET);
});
