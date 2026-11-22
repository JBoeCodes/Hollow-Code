import { chooseNew } from './choose';
import { expect, test, type Page } from './fixtures/test';

/**
 * Canvas focus (10-11): a turn that finishes with a canvas keeps the top of
 * its last canvas in view instead of scrolling to the closing prose. The
 * anchor holds through late layout shifts until the person scrolls or sends;
 * their own scroll releases it and a "Back to canvas" chip brings it back.
 * Turns without a canvas still end at the bottom. The rules themselves
 * (including "someone scrolled away mid-stream stays put") are unit-tested in
 * src/canvasFocus.test.ts. The transcript runs with `overflow-anchor: none`,
 * as in WebKit. The last four cases cover following the stream around a
 * canvas (overnight 10-11/12 A1).
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

const PROSE = (n: number, word: string) => Array.from({ length: n }, (_, i) => `${word} paragraph ${i + 1} of the closing notes.`).join('\n\n');
const CANVAS = '```hollow-canvas\n<hc-canvas id="release-status" title="Release status" height="300">\n  <p>29 checks passed</p>\n</hc-canvas>\n```';

async function openWorkspace(page: Page) {
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Produce a checked release report.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Release');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Release');
  // As in WebKit (the app's web view): no browser scroll anchoring to keep a
  // canvas in place on its own, so these tests see Hollow's re-anchoring.
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

/** Where the last canvas sits in the transcript's scroller, and how far the
 *  scroller is from its bottom. */
const geometry = (page: Page) => page.evaluate(() => {
  const s = document.querySelector<HTMLElement>('.work-conversation .chat-scroll')!;
  const c = [...s.querySelectorAll<HTMLElement>('.chat-canvas')].pop();
  return {
    canvasTop: c ? Math.round(c.getBoundingClientRect().top - s.getBoundingClientRect().top) : null,
    gap: Math.round(s.scrollHeight - s.scrollTop - s.clientHeight),
  };
});

/** A canvas reply long enough that its closing prose pushes the canvas off
 *  screen when the chat ends at the bottom. */
async function canvasTurn(page: Page, runId: string) {
  await ask(page, 'Where is the release?', runId);
  await emit(page, { type: 'assistantDelta', text: `Here is where the release stands.\n\n${CANVAS}\n\n` });
  // The frame mounts mid-turn and settles its height before more text comes.
  await expect(page.locator('.work-conversation iframe[title="Canvas: Release status"]')).toBeVisible();
  await page.waitForTimeout(200);
  await emit(page, { type: 'assistantDelta', text: PROSE(40, 'Closing') });
  // Mid-stream: the canvas is not snapped to while the reply streams.
  await expect.poll(async () => (await geometry(page)).canvasTop).not.toBe(16);
  await emit(page, { type: 'done', sessionId: null });
}

/** The canvas top sits in view, just under the scroller's top. */
const canvasInView = async (page: Page) => { const t = (await geometry(page)).canvasTop; return t !== null && t >= 0 && t <= 40 ? true : t; };

const chip = (page: Page) => page.getByRole('button', { name: /^Back to canvas/ });

for (const tools of [false, true]) test(`short transcript keeps both return controls clear ${tools ? 'with' : 'without'} conversation tools`, async ({ page }) => {
  await page.setViewportSize({ width: 720, height: 480 });
  await page.addInitScript(() => localStorage.setItem('hollow.studio.textScale', '125'));
  await openWorkspace(page);
  if (tools) {
    await ask(page, 'Start with a summary', 'run-short-first');
    await emit(page, { type: 'assistantDelta', text: 'The release is being checked.' });
    await emit(page, { type: 'done', sessionId: null });
  }
  await canvasTurn(page, 'run-short-canvas');
  const scroller = page.locator('.work-conversation .chat-scroll');
  await scroller.hover();
  await page.mouse.wheel(0, 4000);
  await expect(chip(page)).toBeVisible();
  await expect(page.locator('.work-conversation .chat-nav-tools')).toHaveCount(0);
  if (tools) await expect(page.getByRole('button', { name: 'Actions for this conversation', exact: true })).toBeVisible();
  await expect.poll(async () => (await geometry(page)).gap).toBeLessThan(4);
  await page.mouse.wheel(0, -150);
  const jump = page.locator('.work-conversation .chat-jump-latest');
  await expect(jump).toBeVisible();
  await expect(chip(page)).toBeVisible();
  const content = (await scroller.boundingBox())!;
  const backBox = (await chip(page).boundingBox())!;
  const jumpBox = (await jump.boundingBox())!;
  expect(content.height).toBeGreaterThanOrEqual(Math.max(backBox.height, jumpBox.height));
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toBeInViewport();
  expect(Math.max(backBox.y + backBox.height, jumpBox.y + jumpBox.height)).toBeLessThanOrEqual(content.y + 1);
  expect(jumpBox.x + jumpBox.width).toBeLessThanOrEqual(backBox.x);
  for (const control of [jump, chip(page)]) {
    expect(await control.evaluate(el => { const r = el.getBoundingClientRect(); return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2)); })).toBe(true);
    const box = (await control.boundingBox())!;
    for (const tool of await page.locator('.work-conversation .chat-nav-btn').all()) {
      expect(box.x + box.width).toBeLessThanOrEqual((await tool.boundingBox())!.x);
    }
  }
  await chip(page).focus();
  await page.keyboard.press('Enter');
  await expect.poll(() => canvasInView(page)).toBe(true);
  await expect(page.locator('.work-conversation .chat-canvas').last()).toBeFocused();
  // At a normal height, floating recoveries follow transcript actions in
  // the focus order, and keyboard travel reaches Back then the composer.
  await page.setViewportSize({ width: 1440, height: 920 });
  await scroller.hover();
  await page.mouse.wheel(0, 4000);
  await expect(chip(page)).toBeVisible();
  await expect.poll(async () => (await geometry(page)).gap).toBeLessThan(4);
  await page.mouse.wheel(0, -150);
  await expect(jump).toBeVisible();
  expect(await jump.evaluate(el => !!(document.querySelector('.work-conversation .chat-scroll')!.compareDocumentPosition(el) & Node.DOCUMENT_POSITION_FOLLOWING))).toBe(true);
  expect((await jump.boundingBox())!.y).toBeGreaterThan((await scroller.boundingBox())!.y);
  await jump.focus();
  await page.keyboard.press('Tab');
  await expect(chip(page)).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toBeFocused();
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name.includes('narrow'), 'one desktop layout is enough for the scroll geometry');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await installEventBridge(page);
});

test('a finished canvas turn keeps the canvas top in view, through a late layout shift', async ({ page }) => {
  await openWorkspace(page);
  await canvasTurn(page, 'run-focus-1');
  await expect.poll(() => canvasInView(page)).toBe(true);
  expect((await geometry(page)).gap).toBeGreaterThan(200);
  await expect(chip(page)).toHaveCount(0);

  // Something above settles taller after the turn: the canvas stays put.
  await page.evaluate(() => {
    const first = document.querySelector<HTMLElement>('.work-conversation .chat-scroll .turn');
    const pad = document.createElement('div');
    pad.style.height = '260px';
    first?.prepend(pad);
  });
  await expect.poll(() => canvasInView(page)).toBe(true);
  await page.waitForTimeout(300);
  expect(await canvasInView(page)).toBe(true);
});

test('the person’s own scroll releases the anchor; the chip brings the canvas back', async ({ page }) => {
  await openWorkspace(page);
  await canvasTurn(page, 'run-focus-2');
  await expect.poll(() => canvasInView(page)).toBe(true);

  const scroller = page.locator('.work-conversation .chat-scroll');
  const box = (await scroller.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.wheel(0, 4000);
  await expect.poll(async () => (await geometry(page)).gap).toBeLessThan(4);
  await expect(chip(page)).toBeVisible();
  await expect(chip(page)).toHaveAccessibleName('Back to canvas: Release status');
  // A late shift no longer moves them: the canvas isn't brought back.
  const scrollTop = () => page.evaluate(() => document.querySelector<HTMLElement>('.work-conversation .chat-scroll')!.scrollTop);
  const at = await scrollTop();
  await page.evaluate(() => {
    const first = document.querySelector<HTMLElement>('.work-conversation .chat-scroll .turn');
    const pad = document.createElement('div'); pad.style.height = '120px'; first?.prepend(pad);
  });
  await page.waitForTimeout(300);
  expect(await scrollTop()).toBe(at);

  // Keyboard: Enter on the focused chip brings the canvas back, hides the
  // chip, and puts focus on the canvas.
  await chip(page).focus();
  await page.keyboard.press('Enter');
  await expect.poll(() => canvasInView(page)).toBe(true);
  await expect(chip(page)).toHaveCount(0);
  await expect(page.locator('.work-conversation .chat-canvas').last()).toBeFocused();

  // Away again, then Escape on the chip dismisses it and stays put.
  await page.mouse.wheel(0, 4000);
  await expect(chip(page)).toBeVisible();
  await chip(page).focus();
  await page.keyboard.press('Escape');
  await expect(chip(page)).toHaveCount(0);
  expect((await geometry(page)).gap).toBeLessThan(4);
});

test('a turn without a canvas still ends at the bottom', async ({ page }) => {
  await openWorkspace(page);
  await ask(page, 'Summarise the release.', 'run-focus-3');
  await emit(page, { type: 'assistantDelta', text: PROSE(40, 'Plain') });
  await emit(page, { type: 'done', sessionId: null });
  await expect.poll(async () => (await geometry(page)).gap).toBeLessThan(4);
  await page.waitForTimeout(300);
  expect((await geometry(page)).gap).toBeLessThan(4);
  await expect(chip(page)).toHaveCount(0);
});

test('the next message releases the anchor and follows the new reply', async ({ page }) => {
  await openWorkspace(page);
  await canvasTurn(page, 'run-focus-4');
  await expect.poll(() => canvasInView(page)).toBe(true);
  await ask(page, 'And the risks?', 'run-focus-5');
  await emit(page, { type: 'assistantDelta', text: PROSE(30, 'Risk') });
  await expect.poll(async () => (await geometry(page)).gap).toBeLessThan(80);
  await emit(page, { type: 'done', sessionId: null });
  await expect.poll(async () => (await geometry(page)).gap).toBeLessThan(4);
  await expect(chip(page)).toHaveCount(0);
});

test('an ask-canvas gets the same treatment, with motion on (a smooth scroll)', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await openWorkspace(page);
  await ask(page, 'When should we ship?', 'run-focus-6');
  const askCanvas = '```hollow-canvas\n<hc-canvas id="ship-day" title="Ship day" ask>\n<hc-ask title="Ship day" send="Use this">\n<hc-choice name="day" label="When?">\n* Friday\nMonday\n</hc-choice>\n</hc-ask>\n</hc-canvas>\n```';
  await emit(page, { type: 'assistantDelta', text: `Pick a day.\n\n${askCanvas}\n\n` });
  await expect(page.locator('.work-conversation iframe[title="Canvas: Ship day"]')).toBeVisible();
  await page.waitForTimeout(200);
  await emit(page, { type: 'assistantDelta', text: PROSE(40, 'Context') });
  await expect.poll(async () => (await geometry(page)).gap).toBeLessThan(80);
  // Record how the transcript is asked to move: smoothly, with motion on.
  await page.evaluate(() => {
    const s = document.querySelector<HTMLElement>('.work-conversation .chat-scroll')!;
    const asked: string[] = [];
    (window as unknown as { __asked: string[] }).__asked = asked;
    const to = s.scrollTo.bind(s) as (o: ScrollToOptions) => void;
    s.scrollTo = ((o: ScrollToOptions) => { asked.push(String(o?.behavior)); to(o); }) as typeof s.scrollTo;
  });
  await emit(page, { type: 'done', sessionId: null });
  await expect.poll(() => canvasInView(page)).toBe(true);
  await page.waitForTimeout(900);
  expect(await canvasInView(page)).toBe(true);
  expect(await page.evaluate(() => (window as unknown as { __asked: string[] }).__asked)).toContain('smooth');
});


test('a canvas drawn before a tool call opens its "Worked for…" fold and stays in view', async ({ page }) => {
  await openWorkspace(page);
  await ask(page, 'Check the release, then summarise.', 'run-focus-8');
  await emit(page, { type: 'assistantDelta', text: `First, the status.\n\n${CANVAS}\n\n` });
  await expect(page.locator('.work-conversation iframe[title="Canvas: Release status"]')).toBeVisible();
  await emit(page, { type: 'toolUse', id: 'f1', name: 'Read', input: { file_path: 'PLAN.md' } });
  await emit(page, { type: 'toolResult', id: 'f1', content: 'plan', isError: false });
  await emit(page, { type: 'assistantDelta', text: PROSE(40, 'Summary') });
  await emit(page, { type: 'done', sessionId: null });
  await expect(page.locator('.work-conversation .chat-run-fold-head')).toBeVisible();
  await expect.poll(() => canvasInView(page)).toBe(true);
});

// Overnight 10-11/12 A1: the chat followed each streamed delta, not the
// content's size. A message renders its text up to 80 ms after the delta
// (ChatMessage's throttle) and a canvas frame settles its height later still,
// so the scroller was pinned to a stale height and nothing followed the real
// growth: with a canvas in the reply's first delta the chat stayed at its top
// while the rest of the reply streamed in below.
const scrollTopOf = (page: Page) => page.evaluate(() => document.querySelector<HTMLElement>('.work-conversation .chat-scroll')!.scrollTop);

test('a canvas early in a reply: the chat keeps following the text that streams in after it', async ({ page }) => {
  await openWorkspace(page);
  await ask(page, 'Where is the release?', 'run-follow-1');
  await emit(page, { type: 'assistantDelta', text: `Here is where the release stands.\n\n${CANVAS}\n\n` });
  await expect(page.locator('.work-conversation iframe[title="Canvas: Release status"]')).toBeVisible();
  await page.waitForTimeout(200);
  await emit(page, { type: 'assistantDelta', text: PROSE(40, 'Closing') });
  await expect.poll(() => scrollTopOf(page)).toBeGreaterThan(200);
  await expect.poll(async () => (await geometry(page)).gap).toBeLessThan(80);
  // Still following: the next delta is followed too.
  await emit(page, { type: 'assistantDelta', text: `\n\n${PROSE(10, 'More')}` });
  await expect.poll(async () => (await geometry(page)).gap).toBeLessThan(80);
});

test('a canvas frame mounting at the bottom of a streaming reply is followed', async ({ page }) => {
  await openWorkspace(page);
  await ask(page, 'Where is the release?', 'run-follow-2');
  await emit(page, { type: 'assistantDelta', text: `${PROSE(30, 'Opening')}\n\n` });
  await expect.poll(async () => (await geometry(page)).gap).toBeLessThan(80);
  await emit(page, { type: 'assistantDelta', text: `${CANVAS}\n\n` });
  await expect(page.locator('.work-conversation iframe[title="Canvas: Release status"]')).toBeVisible();
  await expect.poll(async () => (await geometry(page)).gap).toBeLessThan(80);
  await page.waitForTimeout(400);
  expect((await geometry(page)).gap).toBeLessThan(80);
});

test('mid-stream, the person’s own scroll up still stops the follow', async ({ page }) => {
  await openWorkspace(page);
  await ask(page, 'Where is the release?', 'run-follow-3');
  await emit(page, { type: 'assistantDelta', text: `${PROSE(30, 'Opening')}\n\n` });
  await expect.poll(async () => (await geometry(page)).gap).toBeLessThan(80);
  const scroller = page.locator('.work-conversation .chat-scroll');
  const box = (await scroller.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.wheel(0, -600);
  await expect.poll(async () => (await geometry(page)).gap).toBeGreaterThan(400);
  const at = await scrollTopOf(page);
  await emit(page, { type: 'assistantDelta', text: `${CANVAS}\n\n${PROSE(10, 'Closing')}` });
  await expect(page.locator('.work-conversation iframe[title="Canvas: Release status"]')).toBeAttached();
  await page.waitForTimeout(400);
  expect(await scrollTopOf(page)).toBe(at);
});

test('mid-stream, a find jump is not pulled back by content still growing (review of A1)', async ({ page }) => {
  await openWorkspace(page);
  await ask(page, 'Zanzibar: where is the release?', 'run-follow-4');
  await emit(page, { type: 'assistantDelta', text: 'On track.' });
  await emit(page, { type: 'done', sessionId: null });
  await ask(page, 'And the risks?', 'run-follow-5');
  await emit(page, { type: 'assistantDelta', text: PROSE(40, 'Risk') });
  await expect.poll(() => scrollTopOf(page)).toBeGreaterThan(200);
  await expect.poll(async () => (await geometry(page)).gap).toBeLessThan(80);
  // Something at the bottom keeps growing every frame (a frame settling, a
  // chart drawing): each frame's growth schedules a follow for the next one.
  await page.evaluate(() => {
    const turns = document.querySelectorAll<HTMLElement>('.work-conversation .chat-scroll .turn');
    const grow = document.createElement('div');
    turns[turns.length - 1].append(grow);
    let h = 0;
    const w = window as unknown as { __grow: boolean };
    w.__grow = true;
    const step = () => { if (!w.__grow) return; h += 2; grow.style.height = `${h}px`; requestAnimationFrame(step); };
    requestAnimationFrame(step);
  });
  await page.getByRole('button', { name: 'Actions for this conversation', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Find in this conversation', exact: true }).click();
  await page.getByRole('searchbox', { name: 'Find in this conversation' }).fill('Zanzibar');
  await page.waitForTimeout(500);
  await page.evaluate(() => { (window as unknown as { __grow: boolean }).__grow = false; });
  expect((await geometry(page)).gap).toBeGreaterThan(400);
});
