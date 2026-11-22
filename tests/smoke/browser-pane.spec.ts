import { openTasks } from './choose';
import { expect, test, type Page } from './fixtures/test';
import { chooseNew } from './choose';

// The Preview pane's web mode (2026-09-30 as the side panel's browser; run 15
// P1 moved it into the app-wide Preview pane). In `?mock=1` there is no native
// webview, so these check the pane's side of the contract: what it asks
// browser_pane.rs to open, place, hide and close (the fixture logs each call
// to `mockBrowserLog`). How the native page looks is checked in the app.

type Call = { command: string; args: { tabId?: string | null; url?: string; bounds?: { x: number; y: number; width: number; height: number } | null; action?: string } };
const calls = (page: Page) => page.evaluate(() => JSON.parse(localStorage.getItem('mockBrowserLog') ?? '[]') as Call[]);
const lastShow = async (page: Page) => (await calls(page)).filter(c => c.command === 'browser_show').at(-1);

/** Tauri events over the mock IPC (as preview-pane.spec.ts), to play a reply. */
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

async function openWorkspace(page: Page) {
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Ship the landing page.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Landing');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Landing');
}

const preview = (page: Page) => page.getByRole('complementary', { name: 'Preview', exact: true });

/** The web mode's start page, where the side panel's Browser tab opened (run
 *  15 P1 removed the panel). No click opens an empty web mode (a link opens
 *  its page; New tab is inside the pane), so this asks the store, as
 *  preview-pane.spec.ts does; the reply-link test opens it by a click. */
async function openStartPage(page: Page) {
  await page.evaluate(async () => {
    const store = '/src/store/preview.ts';
    (await import(/* @vite-ignore */ store)).openPreview({ kind: 'url', url: '' });
  });
  const pane = preview(page);
  await expect(pane.getByRole('textbox', { name: 'Address' })).toBeVisible();
  return pane;
}

test.beforeEach(async ({ page }) => {
  test.skip(page.viewportSize()!.width <= 1100, 'the split is desktop-only');
  await installEventBridge(page);
});

test('the web mode opens, places, hides and closes tabs', async ({ page }) => {
  await openWorkspace(page);
  const pane = await openStartPage(page);
  await expect(pane.locator('.browser-start')).toContainText('Open a page beside the conversation.');

  // A bare port is this Mac's dev server.
  const address = pane.getByRole('textbox', { name: 'Address' });
  await expect(address).toBeFocused();
  await address.fill('5173');
  await address.press('Enter');
  await expect(pane.getByRole('tab', { name: 'localhost:5173' })).toHaveAttribute('aria-selected', 'true');
  await expect.poll(async () => (await calls(page)).find(c => c.command === 'browser_open')?.args.url).toBe('http://localhost:5173');
  // Placed over the page area once its webview exists.
  await expect.poll(async () => (await lastShow(page))?.args.tabId).toBeTruthy();
  const area = await pane.locator('.browser-page').boundingBox();
  const placed = (await lastShow(page))!.args;
  const shown = placed.bounds!;
  // It starts after the pane's resize handle, so the handle still drags.
  const grip = await page.getByRole('separator', { name: 'Resize preview' }).boundingBox();
  expect(Math.abs(shown.x - (grip!.x + grip!.width))).toBeLessThanOrEqual(1);
  expect(Math.abs(shown.x + shown.width - (area!.x + area!.width))).toBeLessThanOrEqual(1);
  expect(Math.abs(shown.height - area!.height)).toBeLessThanOrEqual(1);
  await expect(address).toHaveValue('http://localhost:5173');

  // A dialog drawn over the page hides it; closing it shows the page again.
  await page.keyboard.press('Meta+k');
  await expect(page.getByRole('dialog')).toBeVisible();
  await expect.poll(async () => (await lastShow(page))?.args.tabId ?? null).toBeNull();
  await page.keyboard.press('Escape');
  await expect.poll(async () => (await lastShow(page))?.args.tabId).toBeTruthy();

  // Typing a search in the same tab navigates it; its toolbar drives it (the
  // pane's bar has a Back of its own, for what the pane opened).
  await address.fill('tauri webview');
  await address.press('Enter');
  await expect.poll(async () => (await calls(page)).filter(c => c.command === 'browser_open').at(-1)?.args.url).toBe('https://www.google.com/search?q=tauri%20webview');
  const tabs = pane.getByRole('tablist', { name: 'Preview tabs' }).getByRole('tab');
  await expect(tabs).toHaveCount(1);
  await pane.locator('.browser-toolbar').getByRole('button', { name: 'Back' }).click();
  await expect.poll(async () => (await calls(page)).find(c => c.command === 'browser_go')?.args.action).toBe('back');

  // Closing the pane hides every tab without closing it; ⌘\ brings the pane
  // back with the same tab on show. Closing the tab closes its webview.
  await pane.getByRole('button', { name: 'Close preview', exact: true }).click();
  await expect(pane).toHaveCount(0);
  await expect.poll(async () => (await lastShow(page))?.args.tabId ?? null).toBeNull();
  await page.keyboard.press('Meta+Backslash');
  await expect(tabs).toHaveCount(1);
  await expect(tabs).toHaveAttribute('aria-selected', 'true');
  await expect.poll(async () => (await lastShow(page))?.args.tabId).toBe(placed.tabId);
  expect((await calls(page)).some(c => c.command === 'browser_close')).toBe(false);
  await pane.getByRole('button', { name: /^Close (localhost|www\.google)/ }).click();
  await expect.poll(async () => (await calls(page)).some(c => c.command === 'browser_close')).toBe(true);
  await expect(pane).toHaveCount(0);
});

test('the pane keeps its keys: ⌘[ in the address field steps the page, not Hollow (P1-47)', async ({ page }) => {
  await openWorkspace(page);
  const pane = await openStartPage(page);
  const address = pane.getByRole('textbox', { name: 'Address' });
  await address.fill('localhost:5173');
  await address.press('Enter');
  await expect.poll(async () => (await lastShow(page))?.args.tabId).toBeTruthy();
  // Hollow has somewhere to go both ways: Tasks, then back to Landing. The
  // pane stays open beside every page (run 15 P1).
  const history = page.getByRole('group', { name: 'History' });
  await openTasks(page);
  await expect(page.getByRole('heading', { name: 'Tasks', level: 1 })).toBeVisible();
  await history.getByRole('button', { name: 'Back', exact: true }).click();
  await expect(page.getByRole('heading', { name: /^What should we build in .+\?$/ })).toBeVisible();
  await expect(address).toBeVisible();
  await expect(history.getByRole('button', { name: 'Forward', exact: true })).toBeEnabled();

  const goes = async () => (await calls(page)).filter(c => c.command === 'browser_go').map(c => c.args.action);
  await address.focus();
  const stayed = async () => {
    // Hollow stayed where it was: the same workspace, nothing stepped.
    await expect(page.getByRole('heading', { name: 'Tasks', level: 1 })).toHaveCount(0);
    await expect(page.getByRole('heading', { name: /^What should we build in .+\?$/ })).toBeVisible();
    await expect(history.getByRole('button', { name: 'Forward', exact: true })).toBeEnabled();
  };
  await page.keyboard.press('Meta+]');
  await expect.poll(goes).toEqual(['forward']);
  await stayed();
  await page.keyboard.press('Meta+[');
  await expect.poll(goes).toEqual(['forward', 'back']);
  await stayed();
  await page.keyboard.press('Meta+r');
  await expect.poll(goes).toEqual(['forward', 'back', 'reload']);
  await expect(page.locator('.work-header h1')).toHaveText('Landing');
  await expect(history.getByRole('button', { name: 'Forward', exact: true })).toBeEnabled();
  await expect(pane.getByRole('textbox', { name: 'Address' })).toBeVisible();

  // The keymap card lists the pane's three page keys.
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.keyboard.press('?');
  const card = page.getByRole('dialog', { name: 'Keyboard shortcuts' });
  await expect(card).toContainText('back (browser panel)');
  await expect(card).toContainText('forward (browser panel)');
  await expect(card).toContainText('reload (browser panel)');
});

test('Annotate’s bar keeps every action inside the Preview pane, Add to chat included', async ({ page }) => {
  await openWorkspace(page);
  const pane = await openStartPage(page);
  const address = pane.getByRole('textbox', { name: 'Address' });
  await address.fill('localhost:5173');
  await address.press('Enter');
  await expect.poll(async () => (await lastShow(page))?.args.tabId).toBeTruthy();
  await pane.getByRole('button', { name: 'Annotate' }).click();
  const region = pane.getByRole('region', { name: 'Annotate page' });
  const area = (await region.getByLabel('Marks').boundingBox())!;
  await page.mouse.click(area.x + area.width * 0.3, area.y + area.height * 0.3);
  await page.keyboard.press('Enter');
  await expect(region.getByRole('button', { name: 'Discard', exact: true })).toBeVisible();
  const box = (await region.boundingBox())!;
  const buttons = region.locator('.annotate-bar button');
  expect(await buttons.count()).toBeGreaterThan(6);
  for (const b of await buttons.all()) {
    const r = (await b.boundingBox())!;
    expect(r.x + r.width, `${await b.textContent()} sticks out of the pane`).toBeLessThanOrEqual(box.x + box.width + 1);
  }
  await expect(region.getByRole('button', { name: 'Add to chat' })).toBeInViewport();
  // The actions stay together at the right, Add to chat last.
  const add = (await region.getByRole('button', { name: 'Add to chat' }).boundingBox())!;
  expect(box.x + box.width - (add.x + add.width)).toBeLessThan(24);

  // Dragged to its narrowest, the pane still shows every action. (At 1100 px
  // and below the pane takes the page's place, with no grip.)
  if (page.viewportSize()!.width <= 1100) return;
  const grip =(await page.getByRole('separator', { name: 'Resize preview' }).boundingBox())!;
  await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2);
  await page.mouse.down();
  await page.mouse.move(grip.x + 600, grip.y + grip.height / 2, { steps: 8 });
  await page.mouse.up();
  const narrow = (await region.boundingBox())!;
  expect(narrow.width).toBeLessThan(box.width);
  for (const b of await buttons.all()) {
    const r = (await b.boundingBox())!;
    expect(r.x + r.width, `${await b.textContent()} sticks out of the narrow pane`).toBeLessThanOrEqual(narrow.x + narrow.width + 1);
  }
});

test('Annotate keeps ⌘Z to itself, ⇧⌘Z restores a mark, and Discard asks first (P1-47)', async ({ page }) => {
  await openWorkspace(page);
  const pane = await openStartPage(page);
  const address = pane.getByRole('textbox', { name: 'Address' });
  await address.fill('localhost:5173');
  await address.press('Enter');
  await expect.poll(async () => (await lastShow(page))?.args.tabId).toBeTruthy();
  await pane.getByRole('button', { name: 'Annotate' }).click();
  const region = pane.getByRole('region', { name: 'Annotate page' });
  const area = (await region.getByLabel('Marks').boundingBox())!;
  for (const [i, at] of [[1, 0.25], [2, 0.6]] as const) {
    await page.mouse.click(area.x + area.width * at, area.y + area.height * at);
    await region.getByRole('textbox', { name: `Comment for mark ${i}` }).fill(`Note ${i}`);
    await page.keyboard.press('Enter');
  }
  const numbers = region.getByRole('button', { name: /^Mark \d/ });
  await expect(numbers).toHaveCount(2);

  // ⌘Z in the composer is the composer's: the marks stay.
  await page.getByLabel('Chat message').focus();
  await page.keyboard.press('Meta+z');
  await expect(numbers).toHaveCount(2);

  // On the snapshot ⌘Z takes the last mark back and ⇧⌘Z restores it, note and all.
  await region.getByRole('group', { name: 'Annotation tool' }).getByRole('button', { name: 'Box', exact: true }).focus();
  await page.keyboard.press('Meta+z');
  await expect(numbers).toHaveCount(1);
  await page.keyboard.press('Shift+Meta+z');
  await expect(region.getByRole('button', { name: 'Mark 2: Note 2' })).toBeVisible();
  // The same through the bar's buttons.
  await region.getByRole('button', { name: 'Undo', exact: true }).click();
  await expect(numbers).toHaveCount(1);
  await region.getByRole('button', { name: 'Redo', exact: true }).click();
  await expect(numbers).toHaveCount(2);
  // With nothing focused the keys are the snapshot's too.
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.keyboard.press('Meta+z');
  await expect(numbers).toHaveCount(1);
  await page.keyboard.press('Shift+Meta+z');
  await expect(numbers).toHaveCount(2);

  // Discard asks first; ⌘Z behind the question leaves the marks, and Cancel keeps them.
  await region.getByRole('button', { name: 'Discard', exact: true }).click();
  const confirm = page.getByRole('dialog', { name: 'Discard 2 marks?' });
  await expect(confirm).toBeVisible();
  // Focus fallen back to the page (a click on the dialog's dead chrome).
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.keyboard.press('Meta+z');
  await expect(confirm).toBeVisible();
  await confirm.getByRole('button', { name: 'Cancel' }).click();
  await expect(confirm).toHaveCount(0);
  await expect(numbers).toHaveCount(2);
  await region.getByRole('button', { name: 'Discard', exact: true }).click();
  await confirm.getByRole('button', { name: 'Discard', exact: true }).click();
  await expect(region).toHaveCount(0);
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('hollow.annotateDrafts') ?? '[]').length)).toBe(0);
});

test('a web link in a reply opens in the Preview pane’s web mode', async ({ page }) => {
  await openWorkspace(page);
  const composer = page.getByLabel('Chat message');
  await composer.fill('Start the site');
  await composer.press('Meta+Enter');
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBeGreaterThan(0);
  await emit(page, { type: 'runStarted', asked: 'Start the site', runId: 'run-browser', recall: [] });
  await emit(page, { type: 'assistantText', text: 'It is live at [the dev server](http://localhost:4321/).' });
  await emit(page, { type: 'done', sessionId: null });

  // Nothing opens until the link is clicked (once the reply has rendered: its
  // placeholder's links do nothing).
  const pane = preview(page);
  await expect(pane).toHaveCount(0);
  const conversation = page.locator('.work-conversation');
  const link = conversation.getByRole('link', { name: 'the dev server' });
  await expect(link).toBeVisible();
  await expect(conversation.locator('.reply-loading')).toHaveCount(0);
  await link.click();
  await expect(pane.getByRole('tab', { selected: true })).toHaveAttribute('title', /^Web page\nhttp:\/\/localhost:4321/);
  await expect(pane.getByRole('tab', { name: 'localhost:4321' })).toBeVisible();
  await expect.poll(async () => (await calls(page)).find(c => c.command === 'browser_open')?.args.url).toBe('http://localhost:4321/');
});

test('Annotate marks up the page and drafts it into the conversation', async ({ page }) => {
  await openWorkspace(page);
  const pane = await openStartPage(page);
  const address = pane.getByRole('textbox', { name: 'Address' });
  await address.fill('localhost:5173');
  await address.press('Enter');
  await expect.poll(async () => (await lastShow(page))?.args.tabId).toBeTruthy();
  const shown = (await lastShow(page))!.args.bounds!;

  await pane.getByRole('button', { name: 'Annotate' }).click();
  const region = pane.getByRole('region', { name: 'Annotate page' });
  await expect(region.getByRole('img', { name: /^Snapshot of/ })).toBeVisible();
  // The live page hides while it's being marked up.
  await expect.poll(async () => (await lastShow(page))?.args.tabId ?? null).toBeNull();

  // A comment where the user clicks…
  const marks = region.getByLabel('Marks', { exact: true });
  let area = (await marks.boundingBox())!;
  await page.mouse.click(area.x + area.width * 0.25, area.y + area.height * 0.25);
  await region.getByRole('textbox', { name: 'Comment for mark 1' }).fill('Make this button blue');
  await page.keyboard.press('Enter');
  await expect(region.getByRole('textbox')).toHaveCount(0);
  // …and a box dragged around an area.
  await region.getByRole('group', { name: 'Annotation tool' }).getByRole('button', { name: 'Box', exact: true }).click();
  area = (await marks.boundingBox())!;
  await page.mouse.move(area.x + area.width * 0.5, area.y + area.height * 0.5);
  await page.mouse.down();
  await page.mouse.move(area.x + area.width * 0.8, area.y + area.height * 0.8, { steps: 5 });
  await page.mouse.up();
  await region.getByRole('textbox', { name: 'Comment for mark 2' }).fill('Too much space');
  await page.keyboard.press('Enter');
  await expect(region.getByRole('button', { name: /^Mark \d/ })).toHaveCount(2);
  // Undo takes the last mark back; draw it again.
  await region.getByRole('button', { name: 'Undo' }).click();
  await expect(region.getByRole('button', { name: /^Mark \d/ })).toHaveCount(1);
  area = (await marks.boundingBox())!;
  await page.mouse.move(area.x + area.width * 0.5, area.y + area.height * 0.5);
  await page.mouse.down();
  await page.mouse.move(area.x + area.width * 0.8, area.y + area.height * 0.8, { steps: 5 });
  await page.mouse.up();
  await region.getByRole('textbox', { name: 'Comment for mark 2' }).fill('Too much space');
  await page.keyboard.press('Enter');

  await region.getByRole('button', { name: 'Add to chat' }).click();
  await expect(region).toHaveCount(0);
  const composer = page.getByLabel('Chat message');
  await expect(composer).toHaveValue(/^I marked up “Fixture page” \(http:\/\/localhost:5173\); the annotated screenshot is attached\./);
  await expect(composer).toHaveValue(/\n1\. Make this button blue — on <button\.cta> "Start free trial"/);
  await expect(composer).toHaveValue(/\n2\. Box: Too much space — around <button\.cta>/);
  await expect(page.locator('.chat-attachments .chat-attach')).toHaveCount(1);
  await expect(page.locator('.chat-attachments .chat-attach')).toHaveAttribute('title', 'annotated-localhost-5173.png');
  // Each mark's point went to the page in its own viewport pixels: the
  // comment a quarter in, the box's centre at 65%.
  const inspect = (await calls(page)).find(c => c.command === 'browser_inspect')!;
  const points = (inspect.args as unknown as { points: [number, number][] }).points;
  expect(points).toHaveLength(2);
  expect(Math.abs(points[0][0] - shown.width * 0.25)).toBeLessThanOrEqual(3);
  expect(Math.abs(points[1][1] - shown.height * 0.65)).toBeLessThanOrEqual(3);
  // Nothing is sent until the user sends it, and the page shows again.
  expect(await page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBe(0);
  await expect.poll(async () => (await lastShow(page))?.args.tabId).toBeTruthy();
});
