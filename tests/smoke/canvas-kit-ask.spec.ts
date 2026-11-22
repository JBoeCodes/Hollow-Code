import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import AxeBuilder from '@axe-core/playwright';
import { expect, test, type FrameLocator, type Locator, type Page, type TestInfo } from './fixtures/test';
import { MOVED_STATUS } from '../../src/canvas/askSend';
import { clickAskSend } from './askSend';

// Canvas C4 A1: the ask kit's inputs (src/canvas/kit/ask/gallery.html) in the
// smoke harness, in an ask-canvas frame with the runtime, the display kit and
// the ask kit. Screenshots in light, dark and glass at both widths are
// attached (no pixel baselines, as in canvas-kit-gallery.spec.ts); then
// keyboard and pointer ranking, validation inside the frame, Hollow's Send
// under the frame collecting the draft (question cards, 2026-10-14; the
// harness keeps what it would send on `window.__askSent`, a chat sends it,
// canvas-ask-draft.spec.ts), axe and reduced motion.

const ASK = 'Canvas ask gallery';
const UNSAVED = 'Canvas ask gallery, not a saved canvas';
const FALLBACK = 'Canvas ask gallery without the kit script';
const SOURCE = readFileSync(join(process.cwd(), 'src/canvas/kit/ask/gallery.html'), 'utf8');
const TAGS = [...new Set(SOURCE.match(/<hc-[a-z]+/g)!.map(t => t.slice(1)))].sort();

type Theme = 'light' | 'dark';

const region = (page: Page, name: string) => page.getByRole('region', { name, exact: true });
const askFrame = (page: Page, name = ASK): FrameLocator => region(page, name).frameLocator('iframe[title="Rendered HTML artifact"]');
/** Hollow's own Send under the frame. */
const hostSend = (page: Page, name = ASK) => region(page, name).getByRole('button', { name: 'Send answers', exact: true });
const hostStatus = (page: Page) => region(page, ASK).locator('.chat-ask-send-status');
const sent = (page: Page) => page.evaluate(() => (window as unknown as { __askSent?: Array<{ answers: Record<string, unknown>; summary: string }> }).__askSent ?? []);

/** A host token resolved to a computed colour. */
const hostColour = (page: Page, token: string, prop = 'color') => page.evaluate(([t, p]) => {
  const probe = document.createElement('span');
  probe.style.setProperty(p, `var(${t})`);
  document.body.append(probe);
  const c = getComputedStyle(probe).getPropertyValue(p);
  probe.remove();
  return c;
}, [token, prop] as const);

/** Open the harness (optionally glass), expand the ask gallery and wait until
 *  every kit element in it has upgraded. */
async function openAsk(page: Page, theme: Theme, { glass = false, name = ASK } = {}): Promise<FrameLocator> {
  await page.addInitScript(([t, g]) => {
    localStorage.setItem('theme', t);
    localStorage.setItem('glass', g ? 'on' : 'off');
  }, [theme, glass] as const);
  await page.goto('/?mock=1&window=smoke-artifact');
  const r = region(page, name);
  await r.getByRole('button', { name: 'Expand artifact' }).click();
  await r.locator('iframe').scrollIntoViewIfNeeded();
  const frame = askFrame(page, name);
  await expect.poll(() => frame.locator('main').evaluate(m => {
    const kit = [...m.querySelectorAll('*')].filter(e => e.localName.startsWith('hc-'));
    return kit.length > 0 ? kit.filter(e => !e.hasAttribute('data-hc-ready')).map(e => e.localName) : ['none yet'];
  }), 'every ask element upgraded').toEqual([]);
  return frame;
}

async function attach(info: TestInfo, name: string, target: Locator) {
  await info.attach(`${name}-${info.project.name}.png`, { body: await target.screenshot(), contentType: 'image/png' });
}

/** Nothing off the frame's right edge outside a scroll container, and no
 *  sideways scroll of the document. */
const layout = (frame: FrameLocator) => frame.locator('main').evaluate((main) => {
  const doc = document.documentElement;
  const width = doc.clientWidth;
  const offRight: string[] = [];
  for (const el of main.querySelectorAll('*')) {
    const r = el.getBoundingClientRect();
    let scroller = el.parentElement;
    let scrolls = false;
    while (scroller && scroller !== main) {
      if (/(auto|scroll)/.test(getComputedStyle(scroller).overflowX)) { scrolls = true; break; }
      scroller = scroller.parentElement;
    }
    if (!scrolls && r.width && r.right > width + 1 && !el.closest('.hc-sr, [hidden]')) offRight.push(`${el.localName}.${el.className} ${Math.round(r.right)}>${width}`);
  }
  return { scrollWidth: doc.scrollWidth, width, offRight: offRight.slice(0, 8) };
});

const rankOrder = (frame: FrameLocator) => frame.locator('hc-rank .hc-rank-item .hc-opt-text').allTextContents();

for (const theme of ['light', 'dark'] as const) {
  test(`ask gallery in ${theme}: every input upgrades, follows the host tokens and fits its frame`, async ({ page }, info) => {
    const frame = await openAsk(page, theme);
    const manifest = readFileSync(join(process.cwd(), 'src/canvas/kit/ask/manifest.ts'), 'utf8');
    expect(TAGS).toEqual([...manifest.matchAll(/^\s+'(hc-[a-z]+)',$/gm)].map(m => m[1]).concat('hc-checklist').sort());
    for (const tag of TAGS) expect(await frame.locator(tag).count(), tag).toBeGreaterThan(0);

    // Named, native controls: a form, radio groups, checkboxes, a slider, a
    // number box, a text area, a date, a grid of radios, a rank list.
    const form = frame.getByRole('form', { name: 'Release choices' });
    await expect(form).toBeVisible();
    await expect(form.getByRole('radiogroup', { name: /When should the release ship\?/ })).toBeVisible();
    await expect(form.getByRole('radio', { name: /Friday, after the bed passes\s*Recommended/ })).toBeVisible();
    await expect(form.getByRole('checkbox')).toHaveCount(4);
    await expect(form.getByRole('slider', { name: 'Kit budget' })).toHaveValue('24');
    await expect(form.getByRole('spinbutton', { name: 'Kit budget (KB)' })).toHaveValue('24');
    await expect(form.getByRole('textbox', { name: 'Anything else?' })).toHaveAttribute('maxlength', '400');
    await expect(form.getByLabel('Install date')).toHaveAttribute('type', 'date');
    await expect(form.getByRole('radio', { name: 'Speed: High' })).toBeVisible();
    await expect(form.getByRole('radio', { name: /Week 3\s*15 tasks/ })).toBeVisible();
    await expect(form.locator('.hc-rank-item')).toHaveCount(4);
    // The Send button is Hollow's, under the frame, not the canvas's.
    await expect(form.getByRole('button', { name: 'Send answers' })).toHaveCount(0);
    await expect(hostSend(page)).toBeVisible();

    await attach(info, `kit-ask-gallery-${theme}`, region(page, ASK));

    const ink = await hostColour(page, '--ink');
    expect(await frame.locator('body').evaluate(b => getComputedStyle(b).color)).toBe(ink);
    const box = await layout(frame);
    expect(box.offRight).toEqual([]);
    expect(box.scrollWidth).toBeLessThanOrEqual(box.width);
  });
}

test('ask gallery under glass keeps opaque tokens and stays inside its card', async ({ page }, info) => {
  const frame = await openAsk(page, 'dark', { glass: true });
  await expect(page.locator('html')).toHaveAttribute('data-glass', 'on');
  const tokens = await frame.locator('html').evaluate(el => {
    const cs = getComputedStyle(el);
    return ['--app-surface', '--app-canvas', '--app-ink'].map(n => cs.getPropertyValue(n).trim());
  });
  for (const t of tokens) expect(t, 'opaque token').toMatch(/^#[0-9a-f]{6}$/i);
  const box = await layout(frame);
  expect(box.offRight).toEqual([]);
  expect(box.scrollWidth).toBeLessThanOrEqual(box.width);
  await attach(info, 'kit-ask-gallery-glass', region(page, ASK));
});

test('hc-rank reorders by keyboard: Up, Down, Home and End move the focused item and announce its place', async ({ page }) => {
  const frame = await openAsk(page, 'light');
  expect(await rankOrder(frame)).toEqual(['Inputs', 'Draft to the composer', 'Lock after sending', 'Waiting on you']);
  const live = frame.locator('hc-rank [aria-live="polite"]');
  const item = (label: string) => frame.locator('.hc-rank-item', { hasText: label });

  // Tab reaches the items (each is a tab stop, described by the hint).
  await item('Inputs').focus();
  await expect(item('Inputs')).toHaveAttribute('aria-describedby', /hcq-hint/);
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('ArrowDown');
  expect(await rankOrder(frame)).toEqual(['Draft to the composer', 'Lock after sending', 'Inputs', 'Waiting on you']);
  await expect(live).toHaveText('Inputs: position 3 of 4');
  await expect(item('Inputs')).toBeFocused();
  await expect(item('Inputs').locator('.hc-rank-pos')).toHaveText('3');

  await page.keyboard.press('Home');
  expect(await rankOrder(frame)).toEqual(['Inputs', 'Draft to the composer', 'Lock after sending', 'Waiting on you']);
  await expect(live).toHaveText('Inputs: position 1 of 4');
  await page.keyboard.press('ArrowUp');
  await expect(live).toHaveText('Inputs: position 1 of 4');

  await item('Draft to the composer').focus();
  await page.keyboard.press('End');
  expect(await rankOrder(frame)).toEqual(['Inputs', 'Lock after sending', 'Waiting on you', 'Draft to the composer']);
  await expect(live).toHaveText('Draft to the composer: position 4 of 4');
  await expect(item('Draft to the composer')).toBeFocused();

  // Tab still leaves the item for the next one (arrows don't trap focus).
  await page.keyboard.press('Shift+Tab');
  await expect(item('Waiting on you')).toBeFocused();
});

test('hc-rank reorders by dragging an item with the pointer', async ({ page }) => {
  const frame = await openAsk(page, 'light');
  const last = frame.locator('.hc-rank-item', { hasText: 'Waiting on you' });
  const first = frame.locator('.hc-rank-item', { hasText: 'Inputs' });
  await last.scrollIntoViewIfNeeded();
  const from = (await last.boundingBox())!;
  const to = (await first.boundingBox())!;
  await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
  await page.mouse.down();
  for (let i = 1; i <= 8; i++) await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2 - ((from.y - to.y + 6) * i) / 8);
  await expect(last).toHaveAttribute('data-hc-dragging', '');
  await page.mouse.up();
  expect(await rankOrder(frame)).toEqual(['Waiting on you', 'Inputs', 'Draft to the composer', 'Lock after sending']);
  await expect(last).not.toHaveAttribute('data-hc-dragging', '');
  await expect(frame.locator('hc-rank [aria-live="polite"]')).toHaveText('Waiting on you: position 1 of 4');
});

test('Send validates inside the frame: errors are named, described and focused, and clear once fixed', async ({ page }) => {
  const frame = await openAsk(page, 'light');
  const drafts = await watchDrafts(page);
  const form = frame.getByRole('form', { name: 'Release choices' });
  await form.getByRole('spinbutton', { name: 'Kit budget (KB)' }).fill('25');
  await clickAskSend(hostSend(page));

  // Required choice, a range off its step, a required matrix row set.
  await expect(form.getByRole('status')).toHaveText('3 answers need attention.');
  await expect(hostStatus(page)).toHaveText('3 answers need attention.');
  const choice = form.getByRole('radiogroup', { name: /When should the release ship\?/ });
  await expect(choice).toHaveAttribute('aria-invalid', 'true');
  await expect(frame.locator('hc-choice .hc-field-error')).toHaveText('!Choose one option.');
  await expect(frame.locator('hc-range .hc-field-error')).toHaveText('!Use steps of 2 from 16.');
  await expect(form.getByRole('spinbutton', { name: 'Kit budget (KB)' })).toHaveAttribute('aria-invalid', 'true');
  await expect(frame.locator('hc-matrix[name="fit"] .hc-field-error')).toHaveText('!Answer every row (3 left).');
  // The error describes its group, and focus is on the first field's control.
  const errId = await frame.locator('hc-choice .hc-field-error').getAttribute('id');
  await expect(choice).toHaveAttribute('aria-describedby', errId!);
  await expect(form.getByRole('radio').first()).toBeFocused();
  expect(await drafts()).toEqual([]);

  // Fixing an answer clears its message at once.
  await form.getByRole('radio', { name: /Monday, with a second look/ }).check();
  await expect(frame.locator('hc-choice .hc-field-error')).toBeHidden();
  await expect(choice).not.toHaveAttribute('aria-invalid', 'true');
  await form.getByRole('spinbutton', { name: 'Kit budget (KB)' }).fill('26');
  await expect(frame.locator('hc-range .hc-field-error')).toBeHidden();
  for (const row of ['Speed', 'Cost']) await form.getByRole('radio', { name: `${row}: Low` }).check();
  await expect(frame.locator('hc-matrix[name="fit"] .hc-field-error')).toHaveText('!Answer every row (1 left).');
  expect(await drafts()).toEqual([]);
  expect(await sent(page)).toEqual([]);
});

/** Collect the bridge drafts the host window receives from any frame. */
async function watchDrafts(page: Page): Promise<() => Promise<unknown[]>> {
  await page.evaluate(() => {
    const w = window as unknown as { __drafts: unknown[] };
    w.__drafts = [];
    addEventListener('message', e => { if (e.data && e.data.kind === 'draft') w.__drafts.push(e.data); });
  });
  return () => page.evaluate(() => (window as unknown as { __drafts: unknown[] }).__drafts);
}

/** Count the `collect`s Hollow's Send posts to the frame, by the replies
 *  (`draft` or `ask-invalid`) the host window gets back. Not by a listener in
 *  the frame: the kit stops a host `collect` before any other listener there
 *  hears it (overnight 10 A7). */
async function watchCollects(page: Page): Promise<() => Promise<number>> {
  await page.evaluate(() => {
    const w = window as unknown as { __collects: number };
    w.__collects = 0;
    addEventListener('message', e => { if (e.data && e.data.__hc === 1 && (e.data.kind === 'draft' || e.data.kind === 'ask-invalid')) w.__collects++; });
  });
  return () => page.evaluate(() => (window as unknown as { __collects: number }).__collects);
}

async function answerRequired(form: Locator) {
  await form.getByRole('radio', { name: /Friday, after the bed passes/ }).check();
  for (const [row, opt] of [['Speed', 'High'], ['Cost', 'Low'], ['Risk', 'Medium']]) await form.getByRole('radio', { name: `${row}: ${opt}` }).check();
}

const SUMMARY = [
  'When should the release ship?: Friday, after the bed passes',
  'Which checks must pass first?: Bed rows, Bundle budget',
  'Kit budget: 24 KB',
  'Build order: 1. Waiting on you; 2. Inputs; 3. Draft to the composer; 4. Lock after sending',
  'Anything else?: Ship it',
  'Install date: 2026-10-12',
  'Rate each option: Speed: High; Cost: Low; Risk: Medium',
  'Pick the busiest week: Week 3',
].join('\n');

test('Hollow\'s Send collects the draft {canvasId, answers, summary, nonce} and sends it; a draft nobody asked for is refused', async ({ page }) => {
  const frame = await openAsk(page, 'light');
  const drafts = await watchDrafts(page);
  const form = frame.getByRole('form', { name: 'Release choices' });
  await answerRequired(form);
  await form.getByRole('checkbox', { name: /Bed rows/ }).check();
  await form.getByRole('checkbox', { name: /Bundle budget/ }).check();
  await form.getByRole('textbox', { name: 'Anything else?' }).fill('  Ship it  ');
  await form.getByLabel('Install date').fill('2026-10-12');
  await form.getByRole('radio', { name: /Week 3/ }).check();
  await frame.locator('.hc-rank-item', { hasText: 'Waiting on you' }).focus();
  await page.keyboard.press('Home');
  await clickAskSend(hostSend(page));

  await expect(hostStatus(page)).toHaveText('Sent.');
  const answers = {
    'ship-day': 'Friday, after the bed passes', checks: ['Bed rows', 'Bundle budget'], budget: 24,
    order: ['Waiting on you', 'Inputs', 'Draft to the composer', 'Lock after sending'],
    'hand-checks': null, notes: 'Ship it', install: '2026-10-12', fit: { Speed: 'High', Cost: 'Low', Risk: 'Medium' }, week: 'Week 3',
  };
  expect(await sent(page)).toEqual([{ answers, summary: SUMMARY }]);
  const got = await drafts() as Array<Record<string, unknown>>;
  expect(got).toHaveLength(1);
  expect(got[0]).toEqual({ __hc: 1, kind: 'draft', canvasId: 'kit-ask-gallery', answers, summary: SUMMARY, nonce: expect.stringMatching(/^[0-9a-f]{32}$/) });
  // No copy box: nothing to copy by hand.
  await expect(form.getByRole('textbox', { name: 'Your answers' })).toHaveCount(0);

  // The canvas's own script replays that draft, nonce and all: refused.
  await frame.locator('body').evaluate((_, d) => parent.postMessage(d, '*'), got[0]);
  await page.waitForTimeout(300);
  expect(await sent(page)).toHaveLength(1);
  // Each press of Send collects again, with a fresh nonce.
  await clickAskSend(hostSend(page));
  await expect.poll(async () => (await sent(page)).length).toBe(2);
  const again = await drafts() as Array<{ nonce: string }>;
  expect(again.at(-1)!.nonce).not.toBe(got[0].nonce);
});

test('a Send click right after the frame changes height is ignored, and Hollow says so; half a second later it collects (2026-10-15)', async ({ page }) => {
  const frame = await openAsk(page, 'light');
  const collects = await watchCollects(page);
  const before = (await region(page, ASK).locator('iframe[title="Rendered HTML artifact"]').boundingBox())!.height;
  // The canvas reports a new height: the frame grows and Hollow's Send moves
  // down under it (then back, when the kit measures itself again).
  await frame.locator('body').evaluate((_, h) => parent.postMessage({ __hc: 1, kind: 'size', h }, '*'), Math.round(before) + 300);
  await hostSend(page).click();
  await expect(hostStatus(page)).toHaveText(MOVED_STATUS.text);
  expect(await collects()).toBe(0);
  // Half a second after it settles, the same click collects (the form then
  // says what needs attention: nothing is filled in).
  await page.waitForTimeout(700);
  await hostSend(page).click();
  await expect.poll(collects).toBe(1);
  await expect(hostStatus(page)).toHaveText(/answers? needs? attention\./);
  expect(await sent(page)).toEqual([]);
});

test('a size report that keeps the frame the same height starts no hold: a Send click right after it collects (EH8)', async ({ page }) => {
  const frame = await openAsk(page, 'light');
  const collects = await watchCollects(page);
  const iframe = region(page, ASK).locator('iframe[title="Rendered HTML artifact"]');
  const frameH = async () => (await iframe.boundingBox())!.height;
  // Let the kit's own size reports settle, then outlast any hold they began.
  let last = -1;
  await expect.poll(async () => { const h = await frameH(); const same = h === last; last = h; return same; }, { intervals: [250] }).toBe(true);
  await page.waitForTimeout(700);
  const before = await frameH();
  // The canvas reports the height it already has (the runtime's own measure,
  // sandbox-doc.js): the frame doesn't change size and Send doesn't move.
  await frame.locator('body').evaluate(() => {
    const d = document.documentElement;
    const b = document.body;
    let h = Math.max(d.offsetHeight, b.scrollHeight, b.offsetHeight);
    if (d.scrollHeight > innerHeight) h = Math.max(h, d.scrollHeight);
    parent.postMessage({ __hc: 1, kind: 'size', h: Math.ceil(h) }, '*');
  });
  await page.waitForTimeout(50);
  expect(await frameH()).toBe(before);
  // A plain click (not clickAskSend, which would retry past a hold) collects
  // at once, and Hollow never says Send moved.
  await hostSend(page).click();
  await expect.poll(collects).toBe(1);
  await expect(hostStatus(page)).toHaveText(/answers? needs? attention\./);
  await expect(hostStatus(page)).not.toHaveText(MOVED_STATUS.text);
  expect(await sent(page)).toEqual([]);
});

test('the Send status line is a live region before its first message, and the same one says it (EH9)', async ({ page }) => {
  await openAsk(page, 'light');
  const status = hostStatus(page);
  // Mounted empty, takes no room, isn't named by Send yet.
  await expect(status).toHaveCount(1);
  await expect(status).toHaveAttribute('role', 'status');
  await expect(status).toHaveText('');
  expect((await status.boundingBox())?.height ?? 0).toBeLessThanOrEqual(1);
  await expect(hostSend(page)).not.toHaveAttribute('aria-describedby', /.+/);
  await status.evaluate(el => { (window as unknown as { __statusEl: Element }).__statusEl = el; });
  // Past any hold from the kit's own size reports (clickAskSend clicks again
  // after one, as the person is told to).
  await clickAskSend(hostSend(page));
  await expect(status).toHaveText(/answers? needs? attention\./);
  await expect(status).not.toHaveAttribute('data-empty', /.*/);
  expect((await status.boundingBox())!.height).toBeGreaterThan(1);
  // The element that was there all along spoke, and Send now points at it.
  expect(await status.evaluate(el => el === (window as unknown as { __statusEl: Element }).__statusEl)).toBe(true);
  await expect(hostSend(page)).toHaveAttribute('aria-describedby', (await status.getAttribute('id'))!);
});

test('a field left empty answers null, an empty hc-multi too, as the agent is told (C6 P1)', async ({ page }) => {
  const frame = await openAsk(page, 'light');
  const form = frame.getByRole('form', { name: 'Release choices' });
  await answerRequired(form);
  await clickAskSend(hostSend(page));
  await expect(hostStatus(page)).toHaveText('Sent.');
  const got = await sent(page);
  expect(got).toHaveLength(1);
  expect(got[0].answers).toMatchObject({ checks: null, notes: null, install: null, week: null });
  // The summary leaves the empty ones out.
  expect(got[0].summary).not.toMatch(/Which checks/);
});

test('one Send collects every form in the canvas, and a form that needs attention holds them all back', async ({ page }) => {
  const frame = await openAsk(page, 'light');
  await frame.locator('main').evaluate((m) => m.insertAdjacentHTML('beforeend',
    '<hc-ask title="Second form" send="Send second"><hc-choice name="owner" label="Who ships it?" required>\nAda\nGrace\n</hc-choice></hc-ask>'));
  const first = frame.getByRole('form', { name: 'Release choices' });
  const second = frame.getByRole('form', { name: 'Second form' });
  await expect(second.getByRole('radio', { name: 'Grace' })).toBeVisible();
  await answerRequired(first);
  await clickAskSend(hostSend(page));
  await expect(second.getByRole('status')).toHaveText('One answer needs attention.');
  await expect(hostStatus(page)).toHaveText('One answer needs attention.');
  expect(await sent(page)).toEqual([]);
  await second.getByRole('radio', { name: 'Grace' }).check();
  await clickAskSend(hostSend(page));
  await expect(hostStatus(page)).toHaveText('Sent.');
  const [d] = await sent(page);
  expect(d.answers).toMatchObject({ 'ship-day': 'Friday, after the bed passes', owner: 'Grace' });
  expect(d.summary).toMatch(/\nWho ships it\?: Grace$/);
});

test('a refused message from the frame is logged without its payload, thirty a minute at most (C6 P1)', async ({ page }) => {
  const frame = await openAsk(page, 'light');
  const refusals = () => page.evaluate(() => (window as unknown as { __mockBridgeRefusals?: unknown[] }).__mockBridgeRefusals ?? []);
  // The frame's own script posts a draft nobody asked for: refused, never sent.
  await frame.locator('body').evaluate(() => {
    parent.postMessage({ __hc: 1, kind: 'draft', canvasId: 'kit-ask-gallery', answers: { notes: 'private note' }, summary: 'Notes: private note', nonce: '0123456789abcdef0123456789abcdef' }, '*');
  });
  await expect.poll(refusals).toHaveLength(1);
  const [draft] = await refusals() as Array<Record<string, unknown>>;
  expect(draft).toMatchObject({ kind: 'draft', reason: 'unrequested', canvasId: 'kit-ask-gallery', conversation: null, suppressed: 0 });
  expect(draft.bytes).toBeGreaterThan(100);
  expect(await sent(page)).toEqual([]);
  // The frame's own script: an unknown kind, another canvas's state, junk.
  await frame.locator('body').evaluate(() => {
    parent.postMessage({ __hc: 1, kind: 'exfil', secret: 'private note' }, '*');
    parent.postMessage({ __hc: 1, kind: 'state', canvasId: 'someone-else', state: { secret: 'private note' } }, '*');
    parent.postMessage('private note', '*');
  });
  await expect.poll(refusals).toHaveLength(4);
  expect((await refusals()).slice(1)).toEqual([
    expect.objectContaining({ kind: 'other', reason: 'unknown-kind', canvasId: 'kit-ask-gallery' }),
    expect.objectContaining({ kind: 'state', reason: 'canvas' }),
    expect.objectContaining({ kind: 'other', reason: 'not-object', bytes: 14 }),
  ]);
  // A flood: the log takes thirty a minute in all.
  await frame.locator('body').evaluate(() => { for (let i = 0; i < 100; i++) parent.postMessage({ __hc: 1, kind: 'size', h: -i }, '*'); });
  await expect.poll(refusals).toHaveLength(30);
  await page.waitForTimeout(500);
  expect(await refusals()).toHaveLength(30);
  expect(JSON.stringify(await refusals())).not.toContain('private');
});

test('a frame that is not a saved canvas has no Send and never posts a draft', async ({ page }) => {
  const frame = await openAsk(page, 'light', { name: UNSAVED });
  const drafts = await watchDrafts(page);
  const form = frame.getByRole('form', { name: 'Release choices' });
  await answerRequired(form);
  await expect(hostSend(page, UNSAVED)).toHaveCount(0);
  await expect(form.getByRole('button')).toHaveCount(0);
  expect(await drafts()).toEqual([]);
});

test('without the kit script the questions and options stay readable, and nothing upgrades', async ({ page }) => {
  await page.goto('/?mock=1&window=smoke-artifact');
  const r = region(page, FALLBACK);
  await r.locator('iframe').scrollIntoViewIfNeeded();
  const frame = r.frameLocator('iframe');
  await expect(frame.getByText('Friday, after the bed passes')).toBeVisible();
  await expect(frame.locator('[data-hc-ready]')).toHaveCount(0);
  // Each question's label is generated text (`attr(label)`; hc-ask's title).
  const before = (sel: string) => frame.locator(sel).evaluate(el => getComputedStyle(el, '::before').content);
  for (const label of ['When should the release ship?', 'Build order', 'Rate each option']) {
    expect(await before(`[label="${label}"]`)).toMatch(/^(attr\(label\)|".+")$/);
  }
  expect(await before('hc-ask')).toMatch(/^(attr\(title\)|"Release choices")$/);
  const box = await layout(frame);
  expect(box.scrollWidth).toBeLessThanOrEqual(box.width);
});

const serious = (results: Awaited<ReturnType<AxeBuilder['analyze']>>) => results.violations
  .filter(v => v.impact === 'serious' || v.impact === 'critical')
  .map(v => `${v.id} (${v.impact}): ${v.nodes.slice(0, 4).map(n => n.target.join(' >>> ')).join(' | ')}`);

for (const theme of ['light', 'dark'] as const) {
  test(`axe in ${theme}: no serious or critical violations in the ask gallery, its errors and its fallback`, async ({ page }) => {
    const frame = await openAsk(page, theme);
    // Show every error state too: axe checks the messages and invalid fields.
    await clickAskSend(hostSend(page));
    await expect(frame.getByRole('status')).toHaveText('2 answers need attention.');
    await region(page, FALLBACK).locator('iframe').scrollIntoViewIfNeeded();
    await expect(region(page, FALLBACK).frameLocator('iframe').getByText('Friday, after the bed passes')).toBeVisible();
    for (const name of [ASK, FALLBACK]) {
      const results = await new AxeBuilder({ page }).include(`[aria-label="${name}"]`).analyze();
      expect(serious(results), name).toEqual([]);
      const inFrame = [...results.passes, ...results.incomplete].some(r => r.nodes.some(n => n.target.length > 1));
      expect(inFrame, `${name}: axe reached inside the frame`).toBe(true);
    }
  });
}

/** The ask kit's transitions, as computed now (kit-ask.css). */
const transitions = (frame: FrameLocator) => Promise.all(['.hc-opt', '.hc-rank-item'].map(sel =>
  frame.locator(sel).first().evaluate(el => getComputedStyle(el).transitionDuration)));

test('reduced motion: the inputs\' transitions are off, and nothing runs after choosing and ranking', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const frame = await openAsk(page, 'light');
  expect(await transitions(frame)).toEqual(['0s', '0s']);
  await frame.getByRole('radio', { name: /Monday/ }).check();
  await frame.locator('.hc-rank-item').first().focus();
  await page.keyboard.press('ArrowDown');
  expect(await frame.locator('body').evaluate(() => document.getAnimations().filter(a => a.playState === 'running').length)).toBe(0);
});

test('static ask rows remain responsive without introducing motion when full motion is allowed', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  const frame = await openAsk(page, 'light');
  expect(await transitions(frame)).toEqual(['0s', '0s']);
  const choice = frame.getByRole('radio', { name: /Monday/ });
  await choice.check();
  await expect(choice).toBeChecked();
  const first = frame.locator('.hc-rank-item').first();
  const label = await first.getAttribute('data-hc-item');
  await first.focus();
  await page.keyboard.press('ArrowDown');
  await expect(frame.locator('.hc-rank-item').nth(1)).toHaveAttribute('data-hc-item', label!);
  expect(await frame.locator('body').evaluate(() => document.getAnimations().filter(a => a.playState === 'running').length)).toBe(0);
});
