import { createHash } from 'node:crypto';
import { chooseNew, openChat } from './choose';
import { expect, test, type Locator, type Page } from './fixtures/test';

/**
 * Canvas C3, wave 2 C1: a `hollow-canvas` fence renders in place in every
 * chat. While it streams: nothing extra until the root tag closes, then a
 * "Drawing <title>…" skeleton, then the canvas once the fence ends (mid-turn
 * too). It opens in Canvas mode (outside Tauri the srcdoc fallback renders
 * Offline and says so), html fences in quick and workspace chats default to
 * Canvas too, a broken canvas shows its source and "Ask to fix" (drafts, never
 * sends), Expand opens the same canvas in the Preview pane, and re-rendering a
 * transcript never records (the mock store counts calls).
 */

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
const calls = (page: Page, command: string) => page.evaluate(c => window.__mockApiCalls?.[c] ?? 0, command);

const CANVAS = '<hc-canvas id="release-status" title="Release status" height="300">\n  <p>29 checks passed</p>\n</hc-canvas>';

async function openWorkspace(page: Page) {
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Produce a checked release report.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Release');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Release');
}

async function ask(page: Page, prompt: string, runId: string) {
  const sends = await calls(page, 'chat_send');
  const composer = page.getByLabel('Chat message');
  await composer.fill(prompt);
  await composer.press('Meta+Enter');
  await expect.poll(() => calls(page, 'chat_send')).toBe(sends + 1);
  await emit(page, { type: 'runStarted', asked: prompt, runId, recall: [] });
}

/** 10-11 B1: the policy an artifact opens in (no mode buttons any more). */
const sandbox = (scope: Locator) => scope.locator('.chat-artifact').first();

/** Streams a canvas reply in steps and checks each streaming state. */
async function streamCanvas(page: Page, transcript: Locator) {
  const reply = transcript.locator('.chat-msg').last();
  await emit(page, { type: 'assistantDelta', text: 'Here is where the release stands.\n\n```hollow-canvas\n<hc-canvas id="release-status" ti' });
  await expect(reply).toContainText('Here is where the release stands.');
  // Pending: the root tag is still arriving. Nothing extra, not even the source.
  await expect(reply.locator('.chat-codeblock, .chat-canvas-skeleton, .chat-canvas-broken')).toHaveCount(0);
  await emit(page, { type: 'assistantDelta', text: 'tle="Release status" height="300">\n  <p>29 che' });
  const skeleton = reply.getByRole('status').filter({ hasText: 'Drawing Release status…' });
  await expect(skeleton).toBeVisible();
  expect(Math.round((await skeleton.boundingBox())!.height)).toBe(300);
  // The fence ends mid-turn: the canvas mounts before the turn is over.
  await emit(page, { type: 'assistantDelta', text: 'cks passed</p>\n</hc-canvas>\n```\n\nAll 29' });
  const frame = reply.locator('iframe[title="Canvas: Release status"]');
  await expect(frame).toBeVisible();
  await expect(page.frameLocator('iframe[title="Canvas: Release status"]').getByText('29 checks passed')).toBeVisible();
  await emit(page, { type: 'assistantDelta', text: ' checks passed.' });
  await emit(page, { type: 'done', sessionId: null });
  await expect(reply).toContainText('All 29 checks passed.');
  return reply;
}

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name.includes('narrow'), 'the Preview pane split and the rail tree are desktop-only');
  await installEventBridge(page);
});

test('a workspace canvas streams in place, opens in Canvas mode, is saved once and expands into the Preview pane', async ({ page }) => {
  await openWorkspace(page);
  await ask(page, 'Where is the release?', 'run-canvas-1');
  const transcript = page.locator('.work-conversation');
  const reply = await streamCanvas(page, transcript);

  // Inline, not a side-panel card; Canvas mode is the default (the browser
  // fallback renders Offline and says so).
  await expect(transcript.locator('.chat-visual-card')).toHaveCount(0);
  const canvas = reply.locator('.chat-canvas');
  await expect(canvas.locator('.chat-artifact-badge')).toHaveText('CANVAS');
  await expect(sandbox(canvas)).toHaveAttribute('data-sandbox', 'canvas');
  await expect(canvas.getByRole('group', { name: 'Artifact security mode' })).toHaveCount(0);
  await expect(canvas).toContainText('Scripts run only in the Hollow app; showing this without them.');
  // The internet stays the person's: one menu item, never a default.
  await canvas.locator('summary[aria-label="Canvas actions"]').click();
  await expect(canvas.getByRole('menuitem', { name: 'Allow internet for this canvas' })).toBeVisible();
  await page.keyboard.press('Escape');

  // Saved once, to this workspace's store, as v1.
  await expect.poll(() => calls(page, 'record_canvas')).toBe(1);
  await expect(canvas).toHaveAttribute('data-saved', 'true');
  await expect(canvas.locator('.chat-artifact-version')).toHaveText('v1');
  const saved = await page.evaluate(() => Object.entries(JSON.parse(localStorage.getItem('mockCanvases') ?? '{}') as Record<string, Record<string, unknown>>).map(([k, v]) => [k, Object.keys(v)]));
  expect(saved).toEqual([[expect.stringMatching(/^ws:/), ['release-status']]]);
  // A finished html or canvas reply never opens the Preview pane by itself.
  const pane = page.getByRole('complementary', { name: 'Preview' });
  await expect(pane).toHaveCount(0);

  // Expand: the same canvas, full height, in the Preview pane (run 15 P1; it
  // was the side panel's Visuals tab). Viewing it there reads the store and
  // records nothing.
  const reads = await calls(page, 'read_canvas');
  await canvas.getByRole('button', { name: 'Expand', exact: true }).click();
  await expect(pane).toBeVisible();
  await expect(pane.locator('iframe[title="Canvas: Release status"]')).toBeVisible();
  // Its header is the compact one since 10-19 ("More room for panel canvases"):
  // the version is in the frame's name and Canvas actions, not a label.
  await expect(pane.locator('.chat-artifact-version')).toHaveCount(0);
  await expect(pane.locator('iframe[title="Canvas: Release status"]')).toHaveAttribute('aria-label', /^Release status · v1\b/);
  await expect.poll(() => calls(page, 'read_canvas')).toBeGreaterThan(reads);
  expect(await calls(page, 'record_canvas')).toBe(1);

  // An html fence in a workspace chat renders inline too, in Canvas mode.
  await ask(page, 'And as a page?', 'run-canvas-2');
  await emit(page, { type: 'assistantText', text: 'As a page:\n\n```html\n<h1>Release report</h1>\n```' });
  await emit(page, { type: 'done', sessionId: null });
  const html = transcript.locator('.chat-msg').last().locator('.chat-artifact');
  await expect(html.locator('.chat-artifact-badge')).toHaveText('HTML');
  await expect(html).toHaveAttribute('data-sandbox', 'canvas');
  await expect(transcript.locator('.chat-visual-card')).toHaveCount(0);
  expect(await calls(page, 'record_canvas')).toBe(1);
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});

test('a broken canvas shows its source; Ask to fix fills the composer and sends nothing', async ({ page }) => {
  await openWorkspace(page);
  await ask(page, 'Draw the status', 'run-canvas-bad');
  await emit(page, { type: 'assistantText', text: 'Status:\n\n```hollow-canvas\n<hc-canvas id="Release Status" title="Release">\n<p>x</p>\n</hc-canvas>\n```' });
  await emit(page, { type: 'done', sessionId: null });
  const broken = page.getByRole('group', { name: 'Broken canvas: Release' });
  await expect(broken).toContainText('The canvas id must be 1–64 lowercase letters, digits or hyphens.');
  await expect(broken.locator('.chat-codeblock')).toContainText('<hc-canvas id="Release Status" title="Release">');
  const sends = await calls(page, 'chat_send');
  await broken.getByRole('button', { name: 'Ask to fix' }).click();
  const composer = page.getByLabel('Chat message');
  await expect(composer).toHaveValue('The canvas "Release" (id Release Status) is broken: "The canvas id must be 1–64 lowercase letters, digits or hyphens.". Please send a corrected hollow-canvas fence with the same id.');
  await expect(composer).toBeFocused();
  expect(await calls(page, 'chat_send')).toBe(sends);
  expect(await calls(page, 'record_canvas')).toBe(0);

  // A canvas that throws reports it over the bridge: its error, its source
  // (folded) and Ask to fix appear under the frame.
  await composer.fill('');
  await ask(page, 'Draw it again', 'run-canvas-err');
  await emit(page, { type: 'assistantText', text: `Again:\n\n\`\`\`hollow-canvas\n${CANVAS}\n\`\`\`` });
  await emit(page, { type: 'done', sessionId: null });
  const frame = page.locator('.work-conversation iframe[title="Canvas: Release status"]');
  await expect(frame).toBeVisible();
  const child = (await (await frame.elementHandle())!.contentFrame())!;
  await child.evaluate(() => window.parent.postMessage({ __hc: 1, kind: 'error', message: 'ReferenceError: data is not defined' }, '*'));
  const failed = page.getByRole('group', { name: 'Broken canvas: Release status' });
  await expect(failed).toContainText('This canvas reported an error');
  await expect(failed).toContainText('ReferenceError: data is not defined');
  await expect(frame).toBeVisible();
  await failed.getByRole('button', { name: 'Ask to fix' }).click();
  await expect(composer).toHaveValue(/^The canvas "Release status" \(id release-status\) is broken: "ReferenceError: data is not defined"\./);
  expect(await calls(page, 'chat_send')).toBe(sends + 1);
});

test('a reply that ends inside a canvas shows its source and Ask to fix, not a skeleton', async ({ page }) => {
  // Stop mid-canvas, an agent error, or a provider that drops the closing
  // fence: once the turn settles the open fence reads as ended.
  await openWorkspace(page);
  await ask(page, 'Draw the status', 'run-canvas-stopped');
  const reply = page.locator('.work-conversation .chat-msg').last();
  await emit(page, { type: 'assistantDelta', text: 'Status:\n\n```hollow-canvas\n<hc-canvas id="release-status" title="Release status" height="300">\n  <p>29 che' });
  await expect(reply.getByRole('status').filter({ hasText: 'Drawing Release status…' })).toBeVisible();
  await emit(page, { type: 'done', sessionId: null });
  const broken = page.getByRole('group', { name: 'Broken canvas: Release status' });
  await expect(broken).toContainText('The canvas ended before </hc-canvas>.');
  await expect(broken.locator('.chat-codeblock')).toContainText('<p>29 che');
  await expect(reply.locator('.chat-canvas-skeleton')).toHaveCount(0);
  const sends = await calls(page, 'chat_send');
  await broken.getByRole('button', { name: 'Ask to fix' }).click();
  await expect(page.getByLabel('Chat message')).toHaveValue('The canvas "Release status" (id release-status) is broken: "The canvas ended before </hc-canvas>.". Please send a corrected hollow-canvas fence with the same id.');
  expect(await calls(page, 'chat_send')).toBe(sends);
  expect(await calls(page, 'record_canvas')).toBe(0);

  // A whole canvas whose closing fence never came still renders, and its
  // live turn saves it once.
  await page.getByLabel('Chat message').fill('');
  await ask(page, 'Draw it again', 'run-canvas-unfenced');
  await emit(page, { type: 'assistantDelta', text: `Again:\n\n\`\`\`hollow-canvas\n${CANVAS}\n\`\`` });
  await emit(page, { type: 'done', sessionId: null });
  await expect(page.locator('.work-conversation iframe[title="Canvas: Release status"]')).toBeVisible();
  await expect.poll(() => calls(page, 'record_canvas')).toBe(1);
  await expect(page.locator('.work-conversation .chat-msg').last().locator('.chat-canvas')).toHaveAttribute('data-saved', 'true');
});

// ⌘N with no project open starts a quick chat again (Quick chats part 2,
// 11-14; L8 had sent it to the COO), so its canvas goes to Quick chats' store.
test('a quick chat draws a canvas inline and opens html fences in Canvas mode', async ({ page }) => {
  await page.goto('/?mock=1');
  await expect(page.getByLabel('Main navigation')).toBeVisible();
  await page.locator('body').click({ position: { x: 700, y: 20 } });
  await page.keyboard.press('Meta+n');
  await expect(page.locator('.work-conversation').getByRole('textbox', { name: 'Chat message' })).toBeVisible();
  await ask(page, 'Where is the release?', 'run-quick-1');
  const transcript = page.locator('.work-conversation');
  const reply = await streamCanvas(page, transcript);
  await expect(sandbox(reply.locator('.chat-canvas'))).toHaveAttribute('data-sandbox', 'canvas');
  await expect.poll(() => calls(page, 'record_canvas')).toBe(1);
  expect(await page.evaluate(() => Object.keys(JSON.parse(localStorage.getItem('mockCanvases') ?? '{}')))).toEqual(['ws:quick-chats']);

  await ask(page, 'As a page?', 'run-quick-2');
  await emit(page, { type: 'assistantText', text: '```html\n<h1>Release report</h1>\n```' });
  await emit(page, { type: 'done', sessionId: null });
  const html = transcript.locator('.chat-msg').last().locator('.chat-artifact');
  await expect(html).toHaveAttribute('data-sandbox', 'canvas');
});

test('a reply that arrives whole during its live run (Codex, a stream without deltas) saves its canvas once', async ({ page }) => {
  await openWorkspace(page);
  await ask(page, 'Where is the release?', 'run-whole-1');
  const transcript = page.locator('.work-conversation');
  // No assistantDelta: the reply arrives as one agent message, as Codex's
  // item.completed and the bed's fake claude deliver it.
  await emit(page, { type: 'assistantText', text: `Here it is.\n\n\`\`\`hollow-canvas\n${CANVAS}\n\`\`\`` });
  const canvas = transcript.locator('.chat-msg').last().locator('.chat-canvas');
  await expect(canvas.locator('iframe[title="Canvas: Release status"]')).toBeVisible();
  await expect.poll(() => calls(page, 'record_canvas')).toBe(1);
  await expect(canvas).toHaveAttribute('data-saved', 'true');
  await emit(page, { type: 'done', sessionId: null });
  await expect(canvas.locator('.chat-artifact-version')).toHaveText('v1');
  expect(await calls(page, 'record_canvas')).toBe(1);
});

test('a canvas arriving below keeps the transcript where the person scrolled it', async ({ page }) => {
  await openWorkspace(page);
  await ask(page, 'Where is each lane?', 'run-scroll-1');
  const scroller = page.locator('.work-conversation .chat-scroll');
  const lane = (n: number) => `\n\n\`\`\`hollow-canvas\n<hc-canvas id="lane-${n}" title="Lane ${n}" height="300">\n  <div style="height:260px">Lane ${n}</div>\n</hc-canvas>\n\`\`\``;
  await emit(page, { type: 'assistantDelta', text: `Every lane.${lane(1)}` });
  await expect(page.frameLocator('iframe[title="Canvas: Lane 1"]').getByText('Lane 1')).toBeVisible();
  // From here on, the conversation must never leave the page, not even for a
  // moment: a Suspense fallback around the transcript resets its scroll.
  await page.evaluate(() => {
    const w = window as unknown as { __removed: number };
    w.__removed = 0;
    new MutationObserver(ms => { for (const m of ms) for (const n of m.removedNodes) if (n instanceof HTMLElement && (n.matches('.chat-scroll-wrap') || n.querySelector('.chat-scroll'))) w.__removed++; })
      .observe(document.body, { childList: true, subtree: true });
  });
  for (let n = 2; n <= 5; n++) {
    // The person reads at the bottom; the next canvas arrives below.
    const before = await scroller.evaluate(el => { el.scrollTop = el.scrollHeight; return el.scrollTop; });
    if (n >= 3) expect(before, 'the transcript scrolls').toBeGreaterThan(0);
    await emit(page, { type: 'assistantDelta', text: lane(n) });
    await expect(page.frameLocator(`iframe[title="Canvas: Lane ${n}"]`).getByText(`Lane ${n}`)).toBeVisible();
    await page.waitForTimeout(300);
    const after = await scroller.evaluate(el => el.scrollTop);
    expect(after, `lane ${n} left the scroll at ${after}, not where it was (${before})`).toBeGreaterThanOrEqual(before - 2);
  }
  expect(await page.evaluate(() => (window as unknown as { __removed: number }).__removed)).toBe(0);
  await emit(page, { type: 'done', sessionId: null });
});

const sha = (text: string) => createHash('sha256').update(text, 'utf8').digest('hex');

test('reopening a conversation renders its canvases from the store and records nothing', async ({ page }) => {
  const cwd = '/Users/fixture/vault/workspaces/alpha-1';
  const at = new Date(Date.now() - 5 * 60_000).toISOString();
  const key = 's-20261005-canvas01';
  const unsaved = CANVAS.replace('release-status', 'draft-plan').replace('Release status', 'Draft plan');
  const fenceSha = sha(CANVAS);
  const session = { key, title: 'Release canvases', agent: 'claude', harnessSessionId: 'sess-native', cwd, createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'native' };
  const stored = {
    'ws:alpha-1': {
      'release-status': {
        bodies: { [fenceSha]: CANVAS },
        canvas: {
          scope: 'workspace', workspaceId: 'alpha-1', sessionKey: key, id: 'release-status', title: 'Release status', kit: 'kit@1',
          session: key, fenceSha256: fenceSha, createdAt: at, updatedAt: at, revision: 1, ask: false, height: 300,
          sha256: 'tok-1', path: 'workspaces/alpha-1/outputs/canvases/release-status.canvas.html', file: 'release-status.canvas.html',
          htmlChanged: false, state: null, stateSavedAt: null,
          revisions: [{ revision: 1, fenceSha256: fenceSha, title: 'Release status', savedAt: at, ask: false, height: 300, session: key, turnId: null }],
        },
      },
    },
  };
  await page.addInitScript(([seed]) => {
    if (sessionStorage.getItem('canvasSeeded')) return;
    sessionStorage.setItem('canvasSeeded', '1');
    for (const [k, v] of Object.entries(seed as Record<string, unknown>)) localStorage.setItem(k, typeof v === 'string' ? v : JSON.stringify(v));
  }, [{
    'hollow.railWorkspaces': ['alpha-1'],
    mockWorkspaces: [{ id: 'alpha-1', title: 'Alpha', status: 'active', path: 'workspaces/alpha-1', cwd, modified: at }],
    mockWorkspaceSessions: [session],
    mockWorkspaceConversations: { [key]: { summary: session, eventsRetained: true, turns: [
      { id: `${key}-u`, role: 'user', text: 'Draw the release', tools: [], agent: 'claude', runId: 'run-old', startedAt: at, transcriptIncomplete: false },
      { id: `${key}-a`, role: 'assistant', text: `Saved:\n\n\`\`\`hollow-canvas\n${CANVAS}\n\`\`\`\n\nNot saved:\n\n\`\`\`hollow-canvas\n${unsaved}\n\`\`\``, tools: [], agent: 'claude', runId: 'run-old', startedAt: at, status: 'success', transcriptIncomplete: false },
    ] } },
    mockCanvases: stored,
  }] as const);
  await page.goto('/?mock=1');
  await openChat(page, 'Release canvases');
  const transcript = page.locator('.work-conversation');
  const savedCanvas = transcript.locator('.chat-canvas[data-canvas-id="release-status"]');
  await expect(savedCanvas.locator('iframe[title="Canvas: Release status"]')).toBeVisible();
  await expect(savedCanvas).toHaveAttribute('data-saved', 'true');
  await expect(savedCanvas.locator('.chat-artifact-version')).toHaveText('v1');
  const other = transcript.locator('.chat-canvas[data-canvas-id="draft-plan"]');
  await expect(other.locator('iframe[title="Canvas: Draft plan"]')).toBeVisible();
  await expect(other).toHaveAttribute('data-saved', 'false');
  await expect(other.locator('.chat-artifact-version')).toHaveCount(0);
  await expect.poll(() => calls(page, 'read_canvas')).toBeGreaterThanOrEqual(2);
  expect(await calls(page, 'record_canvas')).toBe(0);
  // Nothing in the store changed.
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('mockCanvases') ?? '{}'))).toEqual(stored);
});

// Overnight 10-11/12 A2: on 10-11 an ask-canvas matrix row read
// "Back to canvas: <title>"; the parser opened a real title element, which
// swallowed the rest, and the card said only "The canvas ended before
// </hc-canvas>". Row text is now literal; elsewhere the card names the tag.
test('a stray tag in a data row is text; elsewhere the broken card names the tag, its line and the fix', async ({ page }) => {
  await openWorkspace(page);
  await ask(page, 'Give me the hand check', 'run-canvas-stray');
  const check = '<hc-canvas id="hand-check" title="Hand check" ask>\n<hc-ask title="Hand check" send="Send results">\n<hc-matrix name="steps" label="Steps" options="Pass | Fail | Skip">\nBack to canvas: <title>\nChip by keyboard\n</hc-matrix>\n</hc-ask>\n</hc-canvas>';
  await emit(page, { type: 'assistantText', text: `Check these.\n\n\`\`\`hollow-canvas\n${check}\n\`\`\`` });
  await emit(page, { type: 'done', sessionId: null });
  const frame = page.frameLocator('.work-conversation iframe[title="Canvas: Hand check"]');
  await expect(frame.getByText('Back to canvas: <title>').first()).toBeVisible();
  await expect(frame.getByText('Chip by keyboard').first()).toBeVisible();
  await expect(page.getByRole('group', { name: /^Broken canvas/ })).toHaveCount(0);
  await expect.poll(() => calls(page, 'record_canvas')).toBe(1);

  // Outside rows, a bare <title> in a canvas that has its </hc-canvas> is a
  // word too (R11, PER-1, 11-14).
  await ask(page, 'And the notes', 'run-canvas-stray-2');
  const notes = '<hc-canvas id="notes" title="Notes">\n<p>First</p>\n<p>Back to canvas: <title></p>\n<p>Last</p>';
  await emit(page, { type: 'assistantText', text: `Notes.\n\n\`\`\`hollow-canvas\n${notes}\n</hc-canvas>\n\`\`\`` });
  await emit(page, { type: 'done', sessionId: null });
  await expect(page.frameLocator('.work-conversation iframe[title="Canvas: Notes"]').getByText('Back to canvas: <title>').first()).toBeVisible();
  // Elsewhere something that hides the end still gets a card naming it, its
  // line and the fix (since R11 that is a comment, a quote or a declaration;
  // a raw-text tag is escaped).
  await ask(page, 'The notes again', 'run-canvas-stray-3');
  const hidden = notes.replace('<title>', '<!-- the end');
  await emit(page, { type: 'assistantText', text: `Notes.\n\n\`\`\`hollow-canvas\n${hidden}\n</hc-canvas>\n\`\`\`` });
  await emit(page, { type: 'done', sessionId: null });
  const broken = page.getByRole('group', { name: 'Broken canvas: Notes' });
  await expect(broken).toContainText('Line 3 of the canvas: a comment (<!--) is never closed with -->');
  await expect(broken).toContainText('write &lt;!-- for the text');
  await broken.getByRole('button', { name: 'Ask to fix' }).click();
  await expect(page.getByLabel('Chat message')).toHaveValue(/is broken: "Line 3 of the canvas: a comment \(<!--\) is never closed/);
});
