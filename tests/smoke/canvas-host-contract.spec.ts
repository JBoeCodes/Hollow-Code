import { expect, test, type Locator, type Page } from './fixtures/test';
import { chooseNew } from './choose';

/**
 * The canvas host contract (plan L3 X7–X9, X16 gallery; 2026-10-14; review U
 * canvases; `src/canvas/hostContract.ts`). Every host draws one head: the
 * permission boundary shows as a tag before anything is refused, the saved
 * state is said in words, Read as text shows the canvas's words in the page,
 * an older version says "Older version (vN of M) · read only" and offers no
 * pin, and the gallery says what it is for and shows each action once.
 * Compare from a reply opens the gallery on that canvas, with a way back.
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
  <h2>Build ${build}</h2>
  <hc-table label="Lane hours">Lane | Hours
Alpha | 3
Beta | 12</hc-table>
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

const fence = (build: number) => `Here.\n\n\`\`\`hollow-canvas\n${canvas(build)}\n\`\`\``;
const canvasIn = (msg: Locator) => msg.locator('.chat-canvas[data-canvas-id="release-status"]');
const pinIn = (scope: Locator) => scope.locator('summary[aria-label="Pin Release status"]');

async function openMenu(scope: Locator) {
  await scope.locator('summary[aria-label="Canvas actions"]').first().click();
}

async function openGallery(page: Page) {
  await page.keyboard.press('Meta+k');
  await page.locator('.palette-input').fill('canvas gallery');
  await page.keyboard.press('Enter');
  const gallery = page.getByRole('region', { name: 'Results', exact: true });
  await expect(gallery).toBeVisible();
  return gallery;
}

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name.includes('narrow'), 'desktop layout; the heads are the same');
  await installEventBridge(page);
});

test('an inline canvas says its permission boundary, saved state and version, reads as text, and marks an older version read-only', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await openWorkspace(page);
  const transcript = page.locator('.work-conversation');
  const msg = (n: number) => transcript.locator('.chat-msg').filter({ has: page.locator('.chat-canvas') }).nth(n);

  await reply(page, 'Where is the release?', 'run-hc-1', fence(1), 1);
  const first = canvasIn(msg(0));
  await expect(first.locator('iframe[title="Canvas: Release status"]')).toBeVisible();
  const head = first.locator('.chat-artifact[data-host="inline"] .chat-artifact-head');

  // X7: the permission boundary is in the head before anything is refused.
  // Outside the app the fallback runs no scripts, and the tag says so.
  const perm = head.locator('.chat-artifact-perm');
  await expect(perm).toBeVisible();
  await expect(perm).toHaveText(/no internet$/);
  await expect(perm).toHaveAttribute('title', /internet/);
  await expect(first.locator('.artifact-ask:not([data-want="fallback"])')).toHaveCount(0);

  // The saved state in words, not only as an attribute.
  await expect(first).toHaveAttribute('data-saved', 'true');
  await expect(head.locator('[data-saved-state]')).toHaveText('Saved in this workspace');
  await expect(head.locator('.chat-artifact-version')).toHaveText('v1');
  await expect(head.locator('[data-older]')).toHaveCount(0);
  await expect(pinIn(head)).toHaveCount(1);

  // X9: Read as text shows the canvas's words in the page, then the canvas again.
  await openMenu(first);
  await first.getByRole('menuitem', { name: 'Read as text' }).click();
  const text = first.getByRole('region', { name: 'Text of Release status' });
  await expect(text).toBeVisible();
  await expect(text.getByRole('heading', { name: 'Build 1' })).toBeVisible();
  await expect(text.getByRole('columnheader', { name: 'Hours' })).toBeVisible();
  await expect(text.getByRole('cell', { name: 'Beta' })).toBeVisible();
  await expect(first.locator('iframe')).toHaveCount(0);
  await expect(text).toBeFocused();
  await text.getByRole('button', { name: 'Show canvas' }).click();
  await expect(first.locator('iframe[title="Canvas: Release status"]')).toBeVisible();

  // X8: a second version. The older reply says it shows an older version,
  // read only, and offers no pin.
  await reply(page, 'And now?', 'run-hc-2', fence(2), 2);
  const second = canvasIn(msg(1));
  await expect(second.locator('iframe[title="Canvas: Release status"]')).toBeVisible();
  await expect(first.locator('[data-older]')).toHaveText('Older version (v1 of 2) · read only');
  await expect(pinIn(first)).toHaveCount(0);
  await expect(second.locator('[data-older]')).toHaveCount(0);
  await expect(pinIn(second)).toHaveCount(1);

  // Picking v1 in the current reply: older, read only, no pin.
  await second.getByRole('button', { name: /^v2/ }).click();
  await second.getByRole('group', { name: 'Versions of Release status' }).getByRole('button', { name: 'v1' }).click();
  await expect(second).toHaveAttribute('data-revision', '1');
  await expect(second.locator('[data-older]')).toHaveText('Older version (v1 of 2) · read only');
  await expect(pinIn(second)).toHaveCount(0);
  await second.getByRole('button', { name: 'Back to v2' }).click();
  await expect(second.locator('[data-older]')).toHaveCount(0);

  // X7e: Compare from a reply opens the gallery on this canvas, with a way back.
  await openMenu(second);
  await second.getByRole('menuitem', { name: 'Compare versions in the gallery' }).click();
  const gallery = page.getByRole('region', { name: 'Results', exact: true });
  await expect(gallery).toBeVisible();
  await expect(gallery.locator('.cg-compare figcaption')).toHaveText(['Before · v1', 'After · v2']);
  await gallery.getByRole('button', { name: 'Back to the conversation' }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Release');
  // Viewing saved nothing.
  expect(await calls(page, 'record_canvas')).toBe(2);
  expect(await calls(page, 'save_canvas_state')).toBe(0);
});

test('expanded canvases keep metadata, versions and pin behind the corner menu', async ({ page }) => {
  await openWorkspace(page);
  await reply(page, 'First report', 'compact-1', fence(1), 1);
  await reply(page, 'Second report', 'compact-2', fence(2), 2);
  const inline = page.locator('.work-conversation .chat-artifact[data-host="inline"]').last();
  await expect(inline.locator('.chat-artifact-title')).toHaveText('Release status');
  await inline.getByRole('button', { name: 'Expand', exact: true }).click();
  const expanded = page.locator('.chat-artifact[data-host="expanded"]');
  await expect(expanded).toHaveClass(/artifact-compact/);
  await expect(expanded.locator('.chat-artifact-title')).toHaveCount(0);
  await expect(expanded.locator('iframe')).toHaveAttribute('aria-label', /Release status · v2 · Saved in this workspace/);
  const permission = expanded.getByRole('img', { name: /no internet/ });
  await expect(permission).toBeVisible();
  await expect(permission).toHaveAttribute('title', /internet/);
  await openMenu(expanded);
  const metadata = expanded.getByRole('menuitem', { name: /Release status · v2 · Saved in this workspace/ });
  await expect(metadata).toBeFocused();
  await expect(metadata).toHaveAttribute('aria-disabled', 'true');
  await expanded.getByRole('menuitem', { name: 'Versions and pin…' }).click();
  const controls = expanded.getByRole('group', { name: 'Canvas versions and pin', exact: true });
  await expect(controls.getByRole('button', { name: /^v2/ })).toBeFocused();
  await expect(pinIn(controls)).toBeVisible();
  await controls.getByRole('button', { name: /^v2/ }).click();
  await controls.getByRole('group', { name: 'Versions of Release status' }).getByRole('button', { name: 'v1' }).click();
  await expect(expanded.locator('iframe')).toHaveAttribute('aria-label', /Older version.*v1 of 2.*read only/);
  await expect(controls.getByRole('button', { name: /^v1/ })).toBeFocused();
  await expect(pinIn(expanded)).toHaveCount(0);
  await controls.getByRole('button', { name: /^v1/ }).click();
  await page.keyboard.press('Escape');
  await expect(controls).toBeVisible();
  await expect(controls.getByRole('group', { name: 'Versions of Release status' })).toHaveCount(0);
  await page.locator('.side-expanded-canvas').getByRole('button', { name: 'Back to v2', exact: true }).click();
  await expect(controls.getByRole('button', { name: /^v2/ })).toBeFocused();
  await expect(pinIn(controls)).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(expanded.getByRole('button', { name: 'Canvas actions', exact: true })).toBeFocused();
  await expect(controls).toHaveCount(0);
  expect(await calls(page, 'record_canvas')).toBe(2);
});

test('the gallery menu opens the saved revision’s source conversation', async ({ page }) => {
  await openWorkspace(page);
  await reply(page, 'Source route', 'run-source-route', fence(1), 1);
  // Synthetic chat events do not write the host's session store. Supply
  // the exact recorded source and transcript for the real key-based opener.
  await page.evaluate(async () => {
    const { api } = await import('/src/api.ts');
    const ws = (await api.listWorkspaces()).find(w => w.title === 'Release')!;
    const c = await api.readCanvas({ workspaceId: ws.id }, 'release-status');
    const notes = JSON.parse(localStorage.getItem('mockWorkspaceNotes') ?? '{}');
    notes[`${ws.path}/outputs/canvases/release-status.canvas.html`] = c!.html;
    notes[`${ws.path}/outputs/canvases/release-status.canvas.json`] = JSON.stringify({ hollowCanvas: 1, id: c!.id, title: c!.title, revision: c!.revision });
    localStorage.setItem('mockWorkspaceNotes', JSON.stringify(notes));
    const key = c!.revisions[0].session!;
    const summary = { key, title: 'Source route', agent: 'codex', cwd: ws.cwd, harnessSessionId: null, resumable: 'briefOnly', createdAt: new Date().toISOString(), lastActivityAt: new Date().toISOString(), endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [] };
    const sessions = JSON.parse(localStorage.getItem('mockWorkspaceSessions') ?? '[]');
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify([...sessions.filter((s: { key: string }) => s.key !== key), summary]));
    const conversations = JSON.parse(localStorage.getItem('mockWorkspaceConversations') ?? '{}');
    conversations[key] = { summary, eventsRetained: false, turns: [{ id: 'source-turn', role: 'assistant', text: 'Source route', tools: [], agent: 'codex', runId: 'run-source-route', startedAt: summary.createdAt, status: 'success', receipt: null, transcriptIncomplete: false }] };
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify(conversations));
  });
  const gallery = await openGallery(page);
  const detail = gallery.locator('.result-canvas');
  await openMenu(detail);
  await detail.getByRole('menuitem', { name: 'Open source chat', exact: true }).click();
  await expect(page.locator('.work-conversation')).toContainText('Source route');
  // (The workspace's Results section offered the same menu until the COO
  // rebuild L4 removed it with the workspace panel.)
});

test('the gallery says what it is for, when each canvas was updated, and shows each action once', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await openWorkspace(page);
  await reply(page, 'Where is the release?', 'run-hg-1', fence(1), 1);
  await reply(page, 'And now?', 'run-hg-2', fence(2), 2);

  const gallery = await openGallery(page);
  await expect(gallery).toHaveAttribute('aria-description', 'Workspace output files and saved canvases, newest first.');
  const item = gallery.getByRole('button', { name: 'Release status', exact: true });
  await expect(item).toContainText('just now');
  const detail = gallery.locator('.library-results-detail');
  await expect(detail.locator('.result-canvas iframe')).toBeVisible();
  const head = detail.locator('.chat-artifact[data-host="gallery"] .chat-artifact-head');
  await expect(head.locator('[data-saved-state]')).toHaveText(/^Saved in this workspace · updated /);
  await expect(head.locator('.chat-artifact-perm')).toHaveText(/no internet$/);

  // One of each: Pin and Copy source in the canvas head, Compare as a button.
  await expect(pinIn(detail)).toHaveCount(1);
  await expect(detail.locator('button', { hasText: /^Copy source$/ })).toHaveCount(1);
  await expect(detail.locator('button', { hasText: /^Read as text$/ })).toHaveCount(1);
  await expect(detail.locator('button', { hasText: /^Compare versions$/ })).toHaveCount(1);
  await expect(detail.locator('button', { hasText: /^Compare versions in the gallery$/ })).toHaveCount(0);
  await openMenu(detail);
  await expect(detail.getByRole('menuitem', { name: 'Copy source', exact: true })).toBeVisible();
  await expect(detail.getByRole('menuitem', { name: 'Read as text', exact: true })).toBeVisible();
  await page.keyboard.press('Escape');

  // An older version in the gallery is marked, and offers no pin.
  await head.getByRole('button', { name: /^v2/ }).click();
  await detail.getByRole('group', { name: 'Versions of Release status' }).getByRole('button', { name: 'v1' }).click();
  await expect(detail.locator('.result-canvas')).toHaveAttribute('data-revision', '1');
  await expect(detail.locator('[data-older]')).toHaveText('Older version (v1 of 2) · read only');
  await expect(pinIn(detail)).toHaveCount(0);

  // Opened from the palette there is no workspace to go back to.
  await expect(gallery.locator('.library-results-back')).toHaveCount(0);
});
