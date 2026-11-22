import { expect, test, type FrameLocator, type Locator, type Page } from './fixtures/test';
import { chooseNew } from './choose';

/**
 * Canvas C3, wave 2 C2: revisions and saved state. Re-emitting a canvas id
 * makes a new revision; the head shows "v3", an older message says it shows
 * an earlier version, and the version list opens any kept revision
 * read-only. The person's view state in kit components (the chosen tab, a
 * table's sort, an open hc-details) is saved through bridge `state` and
 * restored from bridge `init`, including on the next revision. A frame can
 * save only its own canvas's state, only while the person is using it.
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

/** The saved state of `id` in the one workspace store. */
const storedState = (page: Page, id = 'release-status') => page.evaluate((canvasId) => {
  const stores = JSON.parse(localStorage.getItem('mockCanvases') ?? '{}') as Record<string, Record<string, { canvas: { state: unknown; revision: number } }>>;
  const ws = Object.entries(stores).find(([k]) => k.startsWith('ws:'))?.[1];
  return ws?.[canvasId]?.canvas ?? null;
}, id);

/** A canvas with the three kit components that keep view state. */
const canvas = (build: number) => `<hc-canvas id="release-status" title="Release status">
  <p>Build ${build}</p>
  <hc-tabs label="Views">
    <section label="Summary"><p>29 checks passed</p></section>
    <section label="Lanes">
      <hc-table label="Lane hours">Lane | Hours
Alpha | 3
Beta | 12
Gamma | 7</hc-table>
    </section>
  </hc-tabs>
  <hc-details summary="Why it matters"><p>Install waits on the bed.</p></hc-details>
</hc-canvas>`;

const VIEW = { 'hc:hc-tabs:0': 'Lanes', 'hc:hc-table:0': ['Hours', 'descending'], 'hc:hc-details:0': true };

async function openWorkspace(page: Page) {
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Produce a checked release report.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Release');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Release');
}

/** One turn whose reply is `text`, finished. A canvas reply streams, and
 *  its canvas mounts before the turn ends, so the live turn records it. */
async function reply(page: Page, prompt: string, runId: string, text: string) {
  const sends = await calls(page, 'chat_send');
  const composer = page.getByLabel('Chat message');
  await composer.fill(prompt);
  await composer.press('Meta+Enter');
  await expect.poll(() => calls(page, 'chat_send')).toBe(sends + 1);
  await emit(page, { type: 'runStarted', asked: prompt, runId, recall: [] });
  if (text.includes('```hollow-canvas')) {
    await emit(page, { type: 'assistantDelta', text });
    await expect(page.locator('.work-conversation .chat-msg').last().locator('.chat-canvas iframe')).toBeVisible();
  } else {
    await emit(page, { type: 'assistantText', text });
  }
  await emit(page, { type: 'done', sessionId: null });
}

const canvasIn = (msg: Locator) => msg.locator('.chat-canvas[data-canvas-id="release-status"]');
const frameIn = (msg: Locator): FrameLocator => canvasIn(msg).frameLocator('iframe[title="Canvas: Release status"]');

/** The view the kit shows: the selected tab, the sorted column and order, and whether the details are open. */
async function expectView(frame: FrameLocator, view: { tab: string; sort: string | null; open: boolean }) {
  await expect(frame.getByRole('tab', { name: view.tab })).toHaveAttribute('aria-selected', 'true');
  const hours = frame.locator('th', { hasText: 'Hours' });
  await expect(hours).toHaveAttribute('aria-sort', view.sort ?? 'none');
  await expect(frame.getByRole('button', { name: 'Why it matters' })).toHaveAttribute('aria-expanded', String(view.open));
}

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name.includes('narrow'), 'desktop layout; the canvas behaviour is the same');
  await installEventBridge(page);
});

test('a revised canvas keeps the view the person left, shows v3, and opens an earlier revision read-only', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await openWorkspace(page);
  const transcript = page.locator('.work-conversation');
  const msg = (n: number) => transcript.locator('.chat-msg').filter({ has: page.locator('.chat-canvas') }).nth(n);

  await reply(page, 'Where is the release?', 'run-rev-1', `Here it is.\n\n\`\`\`hollow-canvas\n${canvas(1)}\n\`\`\``);
  const first = frameIn(msg(0));
  await expect(first.getByText('Build 1')).toBeVisible();
  await expect.poll(() => calls(page, 'record_canvas')).toBe(1);
  await expect(canvasIn(msg(0)).locator('.chat-artifact-version')).toHaveText('v1');
  // Bridge init reached the frame: the read-only global and the motion hint.
  await expect.poll(() => first.locator('html').evaluate(() => (window as unknown as { hollowCanvas: { init: unknown } }).hollowCanvas.init))
    .toEqual({ canvasId: 'release-status', kit: 'kit@1', locale: expect.any(String), reducedMotion: true });
  await expect(first.locator('html')).toHaveAttribute('data-hc-motion', 'reduce');
  await expectView(first, { tab: 'Summary', sort: null, open: false });
  // Viewing saves nothing.
  expect(await calls(page, 'save_canvas_state')).toBe(0);

  // The person picks a tab, sorts a column (descending) and opens the details.
  await first.getByRole('tab', { name: 'Lanes' }).click();
  const sort = first.getByRole('button', { name: /Hours/ });
  await sort.click();
  await sort.click();
  await first.getByRole('button', { name: 'Why it matters' }).click();
  await expectView(first, { tab: 'Lanes', sort: 'descending', open: true });
  await expect.poll(async () => (await storedState(page))?.state).toEqual(VIEW);
  const saves = await calls(page, 'save_canvas_state');
  expect(saves).toBeGreaterThanOrEqual(1);
  expect(saves).toBeLessThanOrEqual(3); // debounced, not one per message

  // A frame can't write another canvas's state, and a script can't save while the person isn't using it.
  await first.locator('html').evaluate(() => {
    window.parent.postMessage({ __hc: 1, kind: 'state', canvasId: 'other-canvas', state: { x: 1 } }, '*');
  });
  await page.getByLabel('Chat message').focus();
  await first.locator('html').evaluate(() => {
    window.parent.postMessage({ __hc: 1, kind: 'state', canvasId: 'release-status', state: { x: 1 } }, '*');
  });
  await page.waitForTimeout(800);
  expect(await calls(page, 'save_canvas_state')).toBe(saves);
  expect((await storedState(page))?.state).toEqual(VIEW);
  expect(await storedState(page, 'other-canvas')).toBeNull();

  // The agent re-emits the same id: revision 2 opens where the person left it.
  await reply(page, 'Update it', 'run-rev-2', `Updated.\n\n\`\`\`hollow-canvas\n${canvas(2)}\n\`\`\``);
  const second = frameIn(msg(1));
  await expect(second.getByText('Build 2')).toBeVisible();
  await expect.poll(() => calls(page, 'record_canvas')).toBe(2);
  await expectView(second, { tab: 'Lanes', sort: 'descending', open: true });
  await expect(second.locator('tbody tr').first()).toContainText('Beta');
  expect((await storedState(page))?.revision).toBe(2);
  // The older message now says it shows an earlier version.
  await expect(canvasIn(msg(0)).locator('.chat-artifact-version')).toHaveText(/^v1 of 2/);
  await expect(canvasIn(msg(0))).toContainText('An earlier version of this canvas; v2 is current.');

  // A third revision: the label reads v3, and the earlier ones are one click away.
  await reply(page, 'And now?', 'run-rev-3', `Now.\n\n\`\`\`hollow-canvas\n${canvas(3)}\n\`\`\``);
  const third = canvasIn(msg(2));
  await expect(frameIn(msg(2)).getByText('Build 3')).toBeVisible();
  await expect.poll(() => calls(page, 'record_canvas')).toBe(3);
  const label = third.getByRole('button', { name: /^v3/ });
  await expect(label).toHaveText(/^v3/);
  await expect(label).toHaveAttribute('aria-expanded', 'false');
  await expectView(frameIn(msg(2)), { tab: 'Lanes', sort: 'descending', open: true });
  const savesBefore = await calls(page, 'save_canvas_state');

  await label.click();
  const versions = third.getByRole('group', { name: 'Versions of Release status' });
  await expect(versions.getByRole('button')).toHaveText(['v3 current', 'v2', 'v1']);
  await expect(versions.getByRole('button', { name: 'v3 current' })).toHaveAttribute('aria-pressed', 'true');
  const reads = await calls(page, 'read_canvas_revision');
  await versions.getByRole('button', { name: 'v1' }).click();
  await expect(third).toHaveAttribute('data-revision', '1');
  expect(await calls(page, 'read_canvas_revision')).toBe(reads + 1);
  await expect(frameIn(msg(2)).getByText('Build 1')).toBeVisible();
  await expect(third.locator('.chat-artifact-version')).toHaveText(/^v1 of 3/);
  await expect(third).toContainText('Showing v1, an earlier version, read-only.');
  // Read-only: it restores the saved view, but choosing a tab there saves nothing.
  await expectView(frameIn(msg(2)), { tab: 'Lanes', sort: 'descending', open: true });
  await frameIn(msg(2)).getByRole('tab', { name: 'Summary' }).click();
  await page.waitForTimeout(800);
  expect(await calls(page, 'save_canvas_state')).toBe(savesBefore);
  await expect(third.getByRole('status').filter({ hasText: 'Changes here aren’t kept. Open v3 to keep them' })).toHaveCount(1);
  await expect(third.getByText('· changes not kept', { exact: true })).toBeVisible();

  // Back to this reply's own revision, the current one.
  await third.getByRole('button', { name: 'Back to v3' }).click();
  await expect(third).toHaveAttribute('data-revision', '3');
  await expect(frameIn(msg(2)).getByText('Build 3')).toBeVisible();
  await expect(third.locator('.chat-artifact-version')).toHaveText(/^v3/);
  expect(await calls(page, 'record_canvas')).toBe(3);
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});

test('a failed state save stays visible and retries the owned edit', async ({ page }) => {
  await installEventBridge(page);
  await openWorkspace(page);
  await reply(page, 'Show status', 'run-state-retry', `\`\`\`hollow-canvas\n${canvas(1)}\n\`\`\``);
  const host = page.locator('.chat-canvas[data-canvas-id="release-status"]');
  await expect(host).toHaveAttribute('data-saved', 'true');
  await page.evaluate(async () => {
    const { api } = await import('/src/api.ts');
    const save = api.saveCanvasState;
    (window as any).__restoreStateSave = () => { api.saveCanvasState = save; };
    api.saveCanvasState = async () => { throw Error('disk unavailable'); };
  });
  await host.frameLocator('iframe').getByRole('tab', { name: 'Lanes', exact: true }).click();
  await expect(host.getByRole('button', { name: 'Try again', exact: true })).toBeVisible();
  expect((await storedState(page))?.state).toBeNull();
  await page.evaluate(() => (window as any).__restoreStateSave());
  await host.getByRole('button', { name: 'Try again', exact: true }).click();
  await expect.poll(async () => (await storedState(page))?.state).toMatchObject({ 'hc:hc-tabs:0': 'Lanes' });
  await expect(host.getByRole('button', { name: 'Try again', exact: true })).toHaveCount(0);
});

test('an html fence gets init but can save no state', async ({ page }) => {
  await openWorkspace(page);
  await reply(page, 'As a page?', 'run-html', '```html\n<h1>Release report</h1>\n```');
  const frame = page.locator('.work-conversation .chat-artifact iframe').last();
  const child = page.locator('.work-conversation .chat-artifact').last().frameLocator('iframe');
  await expect(child.getByText('Release report')).toBeVisible();
  await expect.poll(() => child.locator('html').evaluate(() => {
    const init = (window as unknown as { hollowCanvas: { init: { canvasId: unknown } | null } }).hollowCanvas.init;
    return init ? { canvasId: init.canvasId } : 'no init yet';
  })).toEqual({ canvasId: null });
  await frame.click();
  expect(await child.locator('html').evaluate(() => (window as unknown as { hollowCanvas: { set(k: string, v: unknown): boolean } }).hollowCanvas.set('k', 1))).toBe(false);
  await child.locator('html').evaluate(() => {
    window.parent.postMessage({ __hc: 1, kind: 'state', canvasId: 'release-status', state: { x: 1 } }, '*');
  });
  await page.waitForTimeout(800);
  expect(await calls(page, 'save_canvas_state')).toBe(0);
});
