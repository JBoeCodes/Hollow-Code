import { expect, test, type Page } from './fixtures/test';
import { fileURLToPath } from 'node:url';
import { choose, chooseNew, optionLabels } from './choose';

/**
 * Videos play inside Hollow (inline video, 2026-10-17). The smoke runs
 * Playwright's Chromium, which has no H.264, so every granted token is
 * served `tests/fixtures/media/tiny.webm` (VP9) through `/__media__/*`, where
 * `mediaSrc` points outside Tauri. The file names stay mp4/mov: the player
 * never trusts the name, and Chromium sniffs the bytes.
 */
const TINY = fileURLToPath(new URL('../fixtures/media/tiny.webm', import.meta.url));

/** Tauri event delivery on top of the `?mock=1` IPC (as preview-pane.spec.ts). */
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
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Render a release video.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Release video');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Release video');
}

async function sendTurn(page: Page, prompt: string, events: Record<string, unknown>[], runId = 'run-video') {
  const composer = page.getByLabel('Chat message');
  const sent = await page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0);
  await composer.fill(prompt);
  await composer.press('Meta+Enter');
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBeGreaterThan(sent);
  await emit(page, { type: 'runStarted', asked: prompt, runId, recall: [] });
  for (const e of events) await emit(page, e);
  await emit(page, { type: 'done', sessionId: null });
}

/** Serve the fixture for every token, or fail the ones `broken` names. */
async function routeMedia(page: Page, broken = false) {
  await page.route('**/__media__/*', route => (broken
    ? route.fulfill({ status: 404, body: '' })
    : route.fulfill({ path: TINY, contentType: 'video/webm' })));
}

test.beforeEach(async ({ page }) => { await installEventBridge(page); });

test('a video named in a reply plays in the side panel', async ({ page }) => {
  test.skip(page.viewportSize()!.width <= 1100, 'the split is desktop-only');
  await routeMedia(page);
  await openWorkspace(page);
  await sendTurn(page, 'Render the clip', [
    { type: 'assistantText', text: 'Rendered `outputs/clip.mov`.' },
  ]);
  const pane = page.getByRole('complementary', { name: 'Preview' });
  await expect(pane).toHaveCount(0);
  const reply = page.locator('.work-conversation .chat-msg').last();
  await reply.locator('code.file-ref').click();
  // The tab's tooltip starts with the file's kind (UI clutter pass, 2026-11-06).
  await expect(pane.getByRole('tab', { selected: true })).toHaveAttribute('title', /^Video\b/);

  const video = pane.locator('figure.media-preview video[controls][preload="metadata"]');
  await expect(video).toHaveCount(1);
  await expect(video).toHaveAttribute('aria-label', 'Video: clip.mov');
  await expect(video).not.toHaveAttribute('autoplay', /.*/);
  await expect(video).not.toHaveAttribute('crossorigin', /.*/);
  await expect(video).toHaveAttribute('src', /^\/__media__\/[0-9a-f]{32}#t=0\.001$/);
  // It really loads: the fixture's metadata arrives.
  await expect.poll(() => video.evaluate(v => (v as HTMLVideoElement).readyState)).toBeGreaterThanOrEqual(1);
  // The pane keeps one set of actions: its header menu, which now opens QuickTime.
  await expect(pane.locator('figure.media-preview .media-preview-actions')).toHaveCount(0);
  await pane.getByRole('button', { name: 'File actions' }).click();
  await expect(page.getByRole('menuitem', { name: 'Open in QuickTime Player' })).toBeVisible();
  await expect(page.getByRole('menuitem', { name: 'Open in default app' })).toBeVisible();
  await page.keyboard.press('Escape');

  // Refresh releases the token and mints a new one.
  const before = await video.getAttribute('src');
  await pane.getByRole('button', { name: 'Refresh preview' }).click();
  await expect(video).not.toHaveAttribute('src', before!);
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('mockMediaReleased') ?? '[]') as string[]))
    .toContain(before!.slice('/__media__/'.length, before!.indexOf('#')));
});

test('a refused video says why and keeps Open and Reveal', async ({ page }) => {
  test.skip(page.viewportSize()!.width <= 1100, 'the split is desktop-only');
  await routeMedia(page);
  await page.addInitScript(() => localStorage.setItem('mockMediaRefusal', JSON.stringify({
    '/Users/me/Movies/far.mp4': 'OUTSIDE_ROOTS',
    '/Users/me/Movies/gone.mp4': 'MISSING',
  })));
  await openWorkspace(page);
  await sendTurn(page, 'Where are they?', [
    { type: 'assistantText', text: 'One is `/Users/me/Movies/far.mp4`, the other `/Users/me/Movies/gone.mp4`.' },
  ]);
  const pane = page.getByRole('complementary', { name: 'Preview' });
  const reply = page.locator('.work-conversation .chat-msg').last();
  await reply.locator('code.file-ref').first().click();
  const status = pane.locator('figure.media-preview [role="status"]');
  await expect(status).toContainText('Outside Hollow’s folders');
  await expect(status.getByRole('button', { name: 'Open in QuickTime Player' })).toBeVisible();
  await expect(status.getByRole('button', { name: 'Reveal in Finder' })).toBeVisible();
  await expect(pane.locator('figure.media-preview video')).toHaveCount(0);

  await reply.locator('code.file-ref').last().click();
  await expect(status).toContainText('File not found');
  await expect(status.getByRole('button')).toHaveCount(0);
});

test('a video that fails to load falls back to a message, never a black box', async ({ page }) => {
  test.skip(page.viewportSize()!.width <= 1100, 'the split is desktop-only');
  await routeMedia(page, true);
  await openWorkspace(page);
  await sendTurn(page, 'Render', [{ type: 'assistantText', text: 'Done: `outputs/broken.mp4`.' }]);
  const pane = page.getByRole('complementary', { name: 'Preview' });
  await page.locator('.work-conversation .chat-msg').last().locator('code.file-ref').click();
  // The same file granted again: not a change, so Chromium's
  // MEDIA_ERR_SRC_NOT_SUPPORTED for the refused load reads as a format problem.
  await expect(pane.locator('figure.media-preview [role="status"]'))
    .toContainText('This video’s format doesn’t play inside Hollow');
  await expect(pane.locator('figure.media-preview video')).toHaveCount(0);
});

/** A receipt for the run `sendTurn` starts, listing vault-relative files. */
const receiptEvent = (files: { path: string; changeType?: string }[], runId = 'run-video') => ({
  type: 'evidenceReceipt',
  receipt: {
    id: `receipt-${runId}`, runId, source: 'chat', sourceId: null,
    startedAt: new Date(Date.now() - 30_000).toISOString(), finishedAt: new Date().toISOString(),
    status: 'success', verificationState: 'notVerified', checks: [], failures: [], historyRunId: runId,
    files: files.map(f => ({ path: f.path, changeType: f.changeType ?? 'create', attribution: 'observed', tool: null, additions: null, deletions: null })),
  },
});

test('a video made through Bash plays under the reply, and never pops the panel open', async ({ page }) => {
  await routeMedia(page);
  await openWorkspace(page);
  await sendTurn(page, 'Render it', [
    { type: 'toolUse', id: 'b1', name: 'Bash', input: { command: 'ffmpeg -i in.mov renders/render.mp4' } },
    { type: 'toolResult', id: 'b1', content: 'done', isError: false },
    { type: 'assistantText', text: 'Rendered it. <video src="/tmp/hollow/vault/renders/render.mp4" controls></video>' },
    receiptEvent([{ path: 'renders/render.mp4' }]),
  ]);
  const reply = page.locator('.work-conversation .turn').filter({ has: page.locator('.reply-media') }).last();
  const strip = reply.locator('.reply-media');
  await expect(strip.locator('figure.media-preview')).toHaveCount(1);
  // The raw tag the agent wrote is dropped from the Markdown: the strip has the playable copy.
  await expect(reply.locator('.turn-md video')).toHaveCount(0);
  const video = strip.locator('video[controls][preload="metadata"]');
  await expect(video).toHaveAttribute('aria-label', 'Video: render.mp4');
  await expect(video).not.toHaveAttribute('autoplay', /.*/);
  await expect.poll(() => video.evaluate(v => (v as HTMLVideoElement).readyState)).toBeGreaterThanOrEqual(1);
  for (const name of ['Open in QuickTime Player', 'Reveal in Finder', 'Open in Preview pane']) {
    await expect(strip.getByRole('button', { name })).toBeVisible();
  }
  // Listed in the Files picker without opening it.
  const pane = page.getByRole('complementary', { name: 'Preview' });
  await expect(pane).toHaveCount(0);
  if (page.viewportSize()!.width > 1100) {
    await strip.getByRole('button', { name: 'Open in Preview pane' }).click();
    await expect(pane.getByRole('tab', { selected: true })).toHaveText('render.mp4');
    await expect(pane.locator('figure.media-preview video')).toHaveCount(1);
  }
});

test('a reply with five videos shows three players and lists two more', async ({ page }) => {
  await routeMedia(page);
  await openWorkspace(page);
  await sendTurn(page, 'Render five', [
    { type: 'assistantText', text: 'Five cuts.' },
    receiptEvent(['a', 'b', 'c', 'd', 'e'].map(n => ({ path: `renders/${n}.mp4` }))),
  ]);
  const strip = page.locator('.work-conversation .reply-media').last();
  await expect(strip.locator('figure.media-preview')).toHaveCount(3);
  await expect(strip.locator('.reply-media-more summary')).toHaveText('+2 more videos');
  await strip.locator('.reply-media-more summary').click();
  await expect(strip.locator('.reply-media-more li')).toHaveText(['d.mp4', 'e.mp4']);
  // One plays at a time.
  const videos = strip.locator('video');
  await expect.poll(() => videos.count()).toBeGreaterThanOrEqual(2);
  await expect.poll(() => videos.nth(1).evaluate(v => (v as HTMLVideoElement).readyState)).toBeGreaterThanOrEqual(1);
  const paused = await strip.evaluate(async el => {
    const [a, b] = [...el.querySelectorAll('video')] as HTMLVideoElement[];
    a.muted = true; b.muted = true;
    await a.play();
    await b.play();
    return { a: a.paused, b: b.paused };
  });
  expect(paused).toEqual({ a: true, b: false });
});

test('a video Result opens in the Preview pane under its review bar', async ({ page }, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'the Preview pane split is desktop-only');
  await routeMedia(page);
  const Q3 = 'workspaces/local-q3-review';
  await page.goto('/?mock=1&demo=1');
  await expect(page.getByRole('region', { name: 'Projects' })).toBeVisible();
  await page.evaluate(q3 => {
    const conversations = JSON.parse(localStorage.getItem('mockWorkspaceConversations') ?? '{}');
    const turn = conversations['s-20260930-q3a00001'].turns.find((t: { id: string }) => t.id === 'r1a-assistant');
    turn.receipt.files = [{ path: `${q3}/outputs/walkthrough.mp4`, changeType: 'create', attribution: 'observed', tool: null, additions: null, deletions: null }];
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify(conversations));
    const notes = JSON.parse(localStorage.getItem('mockWorkspaceNotes') ?? '{}');
    notes[`${q3}/outputs/walkthrough.mp4`] = 'not really a video';
    localStorage.setItem('mockWorkspaceNotes', JSON.stringify(notes));
  }, Q3);
  await page.goto('/?mock=1');
  await page.getByRole('button', { name: 'Open Q3 regional operations review', exact: true }).click();
  const chip = page.getByRole('list', { name: 'Changed files' }).getByRole('listitem').filter({ hasText: 'walkthrough.mp4' }).getByRole('button');
  // The chip opens the Preview pane (2026-11-04); its right-click Review
  // result opens it there too, under its review bar (COO rebuild L4: it was
  // Open in Results). The browser mock serves a workspace's vault files as
  // text, so the pane's player itself is the other tests' business.
  await expect(chip).toHaveAttribute('title', /Preview$/);
  await chip.click({ button: 'right' });
  await page.getByRole('menu').getByRole('menuitem', { name: 'Review result', exact: true }).click();
  const pane = page.getByRole('complementary', { name: 'Preview' });
  await expect(pane.getByRole('tab', { selected: true })).toHaveText('walkthrough.mp4');
  const bar = pane.getByRole('region', { name: 'Result review', exact: true });
  await expect(bar.locator('.result-origin')).toContainText('Video');
  await expect(bar.getByRole('button', { name: 'Accept', exact: true })).toBeVisible();
});

test('a video-only run never auto-opens an older file, and its video joins the Files picker', async ({ page }) => {
  test.skip(page.viewportSize()!.width <= 1100, 'the split is desktop-only');
  await routeMedia(page);
  await openWorkspace(page);
  const pane = page.getByRole('complementary', { name: 'Preview' });
  // A finished turn opens nothing by itself (run 15 P1).
  await sendTurn(page, 'Write notes', [
    { type: 'toolUse', id: 'w1', name: 'Write', input: { file_path: '/tmp/hollow/vault/notes/notes.md', content: '# Notes' } },
    { type: 'toolResult', id: 'w1', content: 'ok', isError: false },
    { type: 'assistantText', text: 'Wrote the notes.' },
  ], 'run-notes');
  await expect(page.locator('.work-conversation .chat-msg').last()).toContainText('Wrote the notes.');
  await expect(pane).toHaveCount(0);

  // A later run that only renders a video (its receipt arrives before done)
  // leaves the panel where the person put it.
  await sendTurn(page, 'Render', [
    { type: 'toolUse', id: 'b1', name: 'Bash', input: { command: 'ffmpeg -i in.mov renders/render.mp4' } },
    { type: 'toolResult', id: 'b1', content: 'done', isError: false },
    { type: 'assistantText', text: 'Rendered.' },
    receiptEvent([{ path: 'renders/render.mp4' }], 'run-render'),
  ], 'run-render');
  await expect(page.locator('.work-conversation .reply-media').last().locator('figure.media-preview')).toHaveCount(1);
  await page.waitForTimeout(300);
  await expect(pane).toHaveCount(0);

  // Open the video, switch to the notes: the video is still listed, first.
  await page.locator('.work-conversation .reply-media').last().getByRole('button', { name: 'Open in Preview pane' }).click();
  await expect(pane.getByRole('tab', { selected: true })).toHaveAttribute('title', /^Video\b/);
  const files = pane.getByRole('combobox', { name: 'Files', exact: true });
  await choose(files, '/tmp/hollow/vault/notes/notes.md');
  await expect(files).toHaveAttribute('data-value', '/tmp/hollow/vault/notes/notes.md');
  // The project's notes are pinned above the conversation's files (COO rebuild L4).
  const notes = ['WORKSPACE.md', 'PLAN.md', 'SOURCES.md', 'AGENTS.md'];
  expect((await optionLabels(files)).filter(l => !notes.includes(l))).toEqual(['render.mp4', 'notes.md']);
});

test('players past the budget of four wait as cards with a poster frame (PR 3)', async ({ page }) => {
  await routeMedia(page);
  await openWorkspace(page);
  await sendTurn(page, 'Render three', [
    { type: 'assistantText', text: 'First three.' },
    receiptEvent(['a', 'b', 'c'].map(n => ({ path: `renders/${n}.mp4` })), 'run-one'),
  ], 'run-one');
  await sendTurn(page, 'Render three more', [
    { type: 'assistantText', text: 'Three more.' },
    receiptEvent(['d', 'e', 'f'].map(n => ({ path: `renders/${n}.mp4` })), 'run-two'),
  ], 'run-two');
  const conversation = page.locator('.work-conversation');
  await expect(conversation.locator('.reply-media figure.media-preview')).toHaveCount(6);
  // Never more than four mounted; the rest (waiting, or put back off screen) are cards.
  const videos = conversation.locator('.reply-media video');
  const cards = conversation.locator('.reply-media .media-preview-card');
  await expect.poll(() => videos.count()).toBeLessThanOrEqual(4);
  await expect.poll(() => cards.count()).toBeGreaterThanOrEqual(2);
  await expect.poll(() => cards.locator('img.media-preview-poster').count()).toBeGreaterThanOrEqual(1);
  await expect(cards.locator('img.media-preview-poster').first()).toHaveAttribute('alt', '');
  // Pressing Play on a waiting card mounts it at once (an explicit open evicts a paused player).
  const waiting = conversation.locator('.reply-media figure.media-preview').filter({ has: page.locator('.media-preview-card') }).last();
  const path = await waiting.getAttribute('data-media-path');
  await waiting.getByRole('button', { name: /^Play / }).click();
  await expect(conversation.locator(`.reply-media figure[data-media-path="${path}"] video`)).toHaveCount(1);
  await expect.poll(() => videos.count()).toBeLessThanOrEqual(4);
});
