import { expect, test, type Page } from './fixtures/test';
import { chooseNew } from './choose';

/**
 * Canvas gallery (overnight 10-11/12 F3): every canvas saved in a workspace.
 * Library → Results → Canvases since consolidation lane B (2026-10-13; journey
 * J6): a view of the Library page, not an overlay. Search and a workspace
 * filter; the selected canvas with its versions; Compare versions shows two
 * revisions side by side and the source change. Reached by clicks (Library →
 * Results → Canvases, a workspace's Results → Canvases: journey J6,
 * j6-canvas-result.journey.spec.ts) and, as aliases, the palette and `g x`.
 */

/** Tauri event delivery on top of the `?mock=1` IPC (as canvas-revisions.spec.ts). */
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

const canvas = (build: number) => `<hc-canvas id="release-status" title="Release status">
  <p>Build ${build}</p>
  <label>Scratch note <input aria-label="Scratch note" /></label>
</hc-canvas>`;

async function openWorkspace(page: Page) {
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Produce a checked release report.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Release');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Release');
}

/** One finished turn whose reply carries `text` (a canvas, saved as it mounts). */
async function reply(page: Page, prompt: string, runId: string, text: string, saves: number) {
  const sends = await calls(page, 'chat_send');
  const composer = page.getByLabel('Chat message');
  await composer.fill(prompt);
  await composer.press('Meta+Enter');
  await expect.poll(() => calls(page, 'chat_send')).toBe(sends + 1);
  await emit(page, { type: 'runStarted', asked: prompt, runId, recall: [] });
  await emit(page, { type: 'assistantDelta', text });
  await expect.poll(() => calls(page, 'record_canvas')).toBe(saves);
  await emit(page, { type: 'done', sessionId: null });
}

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name.includes('narrow'), 'desktop layout; the gallery lays out the same');
  await installEventBridge(page);
});

test('the gallery lists saved canvases, searches them, and compares two versions', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await openWorkspace(page);
  await reply(page, 'Where is the release?', 'run-g-1', `Here.\n\n\`\`\`hollow-canvas\n${canvas(1)}\n\`\`\``, 1);
  await reply(page, 'And now?', 'run-g-2', `Now.\n\n\`\`\`hollow-canvas\n${canvas(2)}\n\`\`\``, 2);

  await page.keyboard.press('Meta+k');
  await page.locator('.palette-input').fill('canvas gallery');
  await expect(page.getByRole('option', { name: /Open Canvases/ })).toBeVisible();
  await page.keyboard.press('Enter');
  const gallery = page.getByRole('region', { name: 'Results', exact: true });
  await expect(gallery).toBeVisible();
  const item = gallery.getByRole('button', { name: 'Release status', exact: true });
  await expect(item).toContainText('v2');
  await expect(item).toHaveAttribute('aria-pressed', 'true');
  // The current version, as Results shows it.
  await expect(gallery.locator('.result-canvas iframe')).toBeVisible();
  await expect(gallery.locator('.result-canvas')).toHaveAttribute('data-revision', '2');

  // Search: no match, then the match again.
  const search = gallery.getByRole('searchbox', { name: 'Find a result' });
  await search.fill('no such canvas');
  await expect(gallery.getByText('Nothing matches', { exact: false })).toBeVisible();
  await search.fill('release STATUS');
  await expect(item).toBeVisible();

  // Compare: v1 and v2 side by side, and the line that changed.
  await gallery.getByRole('button', { name: 'Compare versions' }).click();
  await expect(gallery.locator('.cg-compare figcaption')).toHaveText(['Before · v1', 'After · v2']);
  await expect(gallery.locator('.cg-compare iframe')).toHaveCount(2);
  await expect(gallery.locator('.cg-diff')).toHaveCount(0);
  const frames = await gallery.locator('.cg-compare iframe').elementHandles();
  const note = gallery.frameLocator('.cg-compare iframe').last().getByRole('textbox', { name: 'Scratch note' });
  await note.fill('Keep this local form value');
  await gallery.getByText('View source changes', { exact: true }).click();
  await expect(gallery.locator('.cg-diff')).toContainText('Build 1');
  await expect(gallery.locator('.cg-diff')).toContainText('Build 2');
  await gallery.getByText('View source changes', { exact: true }).click();
  await expect(gallery.locator('.cg-diff')).toHaveCount(0);
  for (const [index, frame] of frames.entries()) {
    expect(await gallery.locator('.cg-compare iframe').nth(index).evaluate((node, previous) => node === previous, frame)).toBe(true);
  }
  await expect(note).toHaveValue('Keep this local form value');
  // The selected canvas owns the disclosure, including pair swaps and
  // leaving/re-entering comparison. Version changes may replace frames.
  await gallery.getByText('View source changes', { exact: true }).click();
  await gallery.getByRole('combobox', { name: 'Before', exact: true }).click();
  await page.getByRole('option', { name: 'v2 (current)', exact: true }).click();
  await expect(gallery.locator('.cg-compare figcaption')).toHaveText(['Before · v2', 'After · v1']);
  await expect(gallery.locator('.cg-diff')).toBeVisible();
  await gallery.getByRole('button', { name: 'Compare versions' }).click();
  await gallery.getByRole('button', { name: 'Compare versions' }).click();
  await expect(gallery.locator('.cg-diff')).toBeVisible();
  await expect(gallery.locator('.cg-compare figcaption')).toHaveText(['Before · v2', 'After · v1']);
  // Viewing saved nothing.
  expect(await calls(page, 'save_canvas_state')).toBe(0);
  expect(await calls(page, 'record_canvas')).toBe(2);

  // A view of Library → Results: Escape stays; the Result type switch leaves.
  await page.keyboard.press('Escape');
  await expect(gallery).toBeVisible();
  await page.getByRole('group', { name: 'Result type' }).getByRole('button', { name: 'Files', exact: true }).click();
  await expect(gallery).toBeVisible();
  await expect(gallery.getByRole('region', { name: 'Selected canvas' })).toHaveCount(0);
  await expect(page.getByRole('searchbox', { name: 'Find a result' })).toBeVisible();
});

/** A saved canvas, version `n`. It was a curated Plan view (`section="plan"`,
 *  stored as `_plan`) until the COO rebuild L4 removed those; a `section`
 *  attribute is now ignored, so it would save under its own id the same. */
const curated = (n: number) => `<hc-canvas id="plan-glance" title="Plan at a glance">
  <p>Plan view ${n}</p>
</hc-canvas>`;

/** What a frame's bridge init gave it: its canvas id and one saved value. */
const initOf = (frame: import('@playwright/test').FrameLocator) => frame.locator('html').evaluate(() => {
  const api = (window as unknown as { hollowCanvas: { init: { canvasId: unknown } | null; get: (k: string) => unknown } }).hollowCanvas;
  return api.init ? { canvasId: api.init.canvasId, kept: api.get('a11') ?? null } : 'no init yet';
});

test('a saved canvas keeps its saved controls in the gallery at the current version only (A11)', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await openWorkspace(page);
  await page.evaluate(async (bodies) => {
    const { mockInvoke } = await import('/src/mockApi.ts');
    const ws = (JSON.parse(localStorage.getItem('mockWorkspaces')!) as { id: string }[])[0].id;
    let sha: string | null = null;
    for (const body of bodies) {
      const saved = await mockInvoke('record_canvas', { workspaceId: ws, sessionKey: null, body, expectedSha256: sha, turnId: null, kit: 'kit@1' }) as { canvas: { sha256: string } };
      sha = saved.canvas.sha256;
    }
    // The view the person left in the canvas, saved on the current version.
    const stores = JSON.parse(localStorage.getItem('mockCanvases')!) as Record<string, Record<string, { canvas: { state: unknown } }>>;
    const store = Object.entries(stores).find(([k]) => k.startsWith('ws:'))![1];
    store['plan-glance'].canvas.state = { a11: 'kept' };
    localStorage.setItem('mockCanvases', JSON.stringify(stores));
  }, [curated(1), curated(2)]);

  await page.keyboard.press('Meta+k');
  await page.locator('.palette-input').fill('canvas gallery');
  await page.keyboard.press('Enter');
  const gallery = page.getByRole('region', { name: 'Results', exact: true });
  await gallery.getByRole('button', { name: 'Plan at a glance', exact: true }).click();
  await expect(gallery.locator('.result-canvas')).toHaveAttribute('data-revision', '2');
  // The bridge uses the fence's own id.
  await expect.poll(() => initOf(gallery.frameLocator('.result-canvas iframe'))).toEqual({ canvasId: 'plan-glance', kept: 'kept' });

  // Compare: the older version opens without the saved view, the current one with it.
  await gallery.getByRole('button', { name: 'Compare versions' }).click();
  await expect(gallery.locator('.cg-compare figcaption')).toHaveText(['Before · v1', 'After · v2']);
  await expect.poll(() => initOf(gallery.frameLocator('.cg-compare iframe').first())).toEqual({ canvasId: 'plan-glance', kept: null });
  await expect.poll(() => initOf(gallery.frameLocator('.cg-compare iframe').last())).toEqual({ canvasId: 'plan-glance', kept: 'kept' });
  expect(await calls(page, 'save_canvas_state')).toBe(0);
});

test('g then x opens the gallery; an empty vault says so', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await openWorkspace(page);
  await page.locator('body').click({ position: { x: 5, y: 5 } });
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.keyboard.press('g');
  await page.keyboard.press('x');
  const gallery = page.getByRole('region', { name: 'Results', exact: true });
  await expect(gallery).toBeVisible();
  await expect(gallery.getByText('No results yet', { exact: false })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Library', exact: true })).toBeVisible();
});
