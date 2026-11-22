import { expect, test, type Page } from './fixtures/test';
import { chooseNew } from './choose';

// The Preview pane (run 15 P1; it replaced the side panel, whose spec this
// was): hidden until a file or a link is clicked, never opened by a finished
// turn, one app-wide pane beside the page. The side panel's Logs, Visuals and
// Files tabs went with it (their tests too); the project's notes in the pane
// (COO rebuild L4) are covered by project-files and visible-routes.

/** Tauri event delivery on top of the `?mock=1` IPC (same bridge as
 *  chat-workspace.spec.ts), so a test can play a finished agent turn. */
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

const BOARD = '```hollow-whiteboard\n' + JSON.stringify({
  version: 1, title: 'Release flow', layout: 'auto',
  nodes: [{ id: 'build', label: 'Build' }, { id: 'ship', label: 'Ship', change: 'added' }],
  edges: [{ from: 'build', to: 'ship' }], steps: [],
}) + '\n```';

async function openWorkspace(page: Page) {
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Produce a checked release report.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Release');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Release');
}

async function sendTurn(page: Page, prompt: string, events: Record<string, unknown>[]) {
  const composer = page.getByLabel('Chat message');
  await composer.fill(prompt);
  await composer.press('Meta+Enter');
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBeGreaterThan(0);
  await emit(page, { type: 'runStarted', asked: prompt, runId: 'run-side-pane', recall: [] });
  for (const e of events) await emit(page, e);
  await emit(page, { type: 'done', sessionId: null });
}

/** The created workspace's folder (the mock names it `work-<time>-<n>`), so a
 *  written file and the reply's relative reference to it are one path. */
const workspaceCwd = (page: Page) => page.evaluate(() =>
  (JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]') as { title: string; cwd: string }[]).find(w => w.title === 'Release')!.cwd);

/** A turn that writes `notes/report.md` (not `outputs/`, which a footer chip
 *  opens in Results) and names it in the reply. */
const writeReport = (cwd: string) => [
  { type: 'toolUse', id: 'w1', name: 'Write', input: { file_path: `${cwd}/notes/report.md`, content: '# Report' } },
  { type: 'toolResult', id: 'w1', content: 'ok', isError: false },
  { type: 'assistantText', text: 'The report is in `notes/report.md`.' },
];

const preview = (page: Page) => page.getByRole('complementary', { name: 'Preview' });

test.beforeEach(async ({ page }) => { await installEventBridge(page); });

test('a turn that writes a file opens nothing by itself (Largest text at 1280px)', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.addInitScript(() => localStorage.setItem('hollow.studio.textScale', '125'));
  await openWorkspace(page);
  await sendTurn(page, 'Write the report', writeReport(await workspaceCwd(page)));
  await expect(page.getByLabel('Chat message')).toBeVisible();
  await expect(page.locator('.work-conversation .chat-msg').last()).toContainText('The report is in');
  await expect(preview(page)).toHaveCount(0);
});

test('an old whiteboard fence shows its JSON under a notice and opens nothing (whiteboards removed)', async ({ page }) => {
  await openWorkspace(page);
  await sendTurn(page, 'Explain the release on a whiteboard', [{ type: 'assistantText', text: `Here is the flow.\n\n${BOARD}` }]);
  const conversation = page.locator('.work-conversation');
  await expect(conversation.getByText('Whiteboards were removed from Hollow. This is the board\'s source.')).toBeVisible();
  await expect(conversation.locator('pre').filter({ hasText: '"Release flow"' })).toBeVisible();
  await expect(conversation.locator('.chat-visual-card, .whiteboard')).toHaveCount(0);
  await expect(preview(page)).toHaveCount(0);
});

const PIXEL = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6LJ8AAAAASUVORK5CYII=';

test('file chips, reply file paths, links and images open in the Preview pane', async ({ page }, testInfo) => {
  test.skip(page.viewportSize()!.width <= 1100, 'the split is desktop-only');
  await openWorkspace(page);
  await sendTurn(page, 'Check the plan', [
    { type: 'toolUse', id: 'r1', name: 'Read', input: { file_path: '/mock/vault/notes/plan.md' } },
    { type: 'toolResult', id: 'r1', content: '     1→# Plan\n     2→Ship it', isError: false },
    { type: 'toolUse', id: 'g1', name: 'Agent', input: { description: 'Draw the logo', subagent_type: 'designer' } },
    { type: 'toolImages', id: 'g1', images: [PIXEL] },
    { type: 'toolResult', id: 'g1', content: 'drawn', isError: false },
    { type: 'assistantText', text: 'Read it. The summary goes in `outputs/summary.md`; run `npm run build` after. See [the brief](WORKSPACE.md) and [the docs](https://example.com/a.md).' },
  ]);
  const pane = preview(page);
  await expect(pane).toHaveCount(0);
  const transcript = page.locator('.work-conversation');

  // A tool row's file chip opens that file; the row itself does not expand.
  // The finished run's steps are folded (frontier F5): open them first.
  await transcript.locator('.chat-run-fold-head').click();
  const readRow = transcript.locator('.toolcall').filter({ hasText: 'plan.md' }).first();
  await readRow.getByRole('button', { name: 'Preview plan.md', exact: true }).click();
  // One header row (UI clutter pass, 2026-11-06): the tab names the file;
  // its kind and full path are the tab's tooltip and the ••• menu's last
  // rows, beside Copy path.
  const shown = pane.getByRole('tab', { selected: true });
  await expect(shown).toHaveText('plan.md');
  await expect(shown).toHaveAttribute('title', /^Markdown\b[^\n]*\n\/mock\/vault\/notes\/plan\.md$/);
  await pane.getByRole('button', { name: 'File actions' }).click();
  await expect(page.getByRole('menuitem', { name: 'Copy path' })).toBeVisible();
  await expect(page.getByRole('menuitem', { name: '/mock/vault/notes/plan.md', exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(pane).toHaveCount(1);
  await expect(readRow).not.toHaveClass(/\bopen\b/);
  await readRow.getByRole('button', { name: 'Preview plan.md', exact: true }).hover();
  await page.screenshot({ path: process.env.FILE_PANE_SHOT ?? testInfo.outputPath('file-pane.png') });

  // A file path in the reply resolves against the conversation's folder.
  const reply = transcript.locator('.chat-msg').last();
  await expect(reply.locator('code.file-ref')).toHaveText(['summary.md']);
  await reply.locator('code.file-ref').click();
  await expect(shown).toHaveText('summary.md');
  await expect(shown).toHaveAttribute('title', /\n.*\/outputs\/summary\.md$/);
  // Back returns to the file before; × hides the pane.
  await pane.getByRole('button', { name: 'Back', exact: true }).click();
  await expect(shown).toHaveText('plan.md');
  await pane.getByRole('button', { name: 'Close preview', exact: true }).click();
  await expect(pane).toHaveCount(0);
  // Keyboard reaches it too.
  await reply.locator('code.file-ref').focus();
  await page.keyboard.press('Enter');
  await expect(shown).toHaveText('summary.md');

  // A local link opens in the pane; a web link opens in its web mode
  // (⌘-click: outside), and Hollow's own page stays put.
  await reply.getByRole('link', { name: 'the brief' }).click();
  await expect(shown).toHaveText('WORKSPACE.md');
  const here = page.url();
  await reply.getByRole('link', { name: 'the docs' }).click();
  await expect(pane.getByRole('tablist', { name: 'Preview tabs' }).getByRole('tab', { name: 'example.com' })).toBeVisible();
  expect(page.url()).toBe(here);
  const popup = page.waitForEvent('popup');
  await reply.getByRole('link', { name: 'the docs' }).click({ modifiers: ['Meta'] });
  expect((await popup).url()).toBe('https://example.com/a.md');
  expect(page.url()).toBe(here);

  // A generated image opens as an image, with no file actions.
  await transcript.getByRole('button', { name: 'Preview generated image' }).click();
  await expect(pane.locator('.chat-preview-img')).toHaveAttribute('src', PIXEL);
  await expect(pane.getByRole('button', { name: 'File actions' })).toHaveCount(0);
  await expect(pane.locator('summary[aria-label="File actions"]')).toHaveCount(0);
  await expect(shown).toHaveAttribute('title', 'Image from the conversation');
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});

test('on a narrow window the pane opens on a click, takes the page’s place, and × brings the conversation back', async ({ page }) => {
  test.skip(page.viewportSize()!.width > 1100, 'narrow only');
  await openWorkspace(page);
  await sendTurn(page, 'Write the report', writeReport(await workspaceCwd(page)));
  await expect(preview(page)).toHaveCount(0);
  await expect(page.getByLabel('Chat message')).toBeVisible();
  await page.locator('.work-conversation .chat-msg').last().locator('code.file-ref').click();
  await expect(preview(page).getByRole('tab', { selected: true })).toHaveText('report.md');
  await expect(preview(page).getByText('Mock file written by the agent.')).toBeVisible();
  await expect(page.getByLabel('Chat message')).toBeHidden();
  await preview(page).getByRole('button', { name: 'Close preview', exact: true }).click();
  await expect(preview(page)).toHaveCount(0);
  await expect(page.getByLabel('Chat message')).toBeVisible();
});

async function fontPx(page: Page, selector: string) {
  return page.locator(selector).first().evaluate(el => parseFloat(getComputedStyle(el).fontSize));
}

async function dragHandle(page: Page, dx: number) {
  const box = (await page.getByRole('separator', { name: 'Resize preview' }).boundingBox())!;
  const x = box.x + box.width / 2, y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + dx / 2, y, { steps: 4 });
  await page.mouse.move(x + dx, y, { steps: 4 });
  await page.mouse.up();
}

test('dragging the Preview pane’s edge resizes it, text keeps its size (R03), and the width is remembered', async ({ page }) => {
  test.skip(page.viewportSize()!.width <= 1100, 'the split is desktop-only');
  await openWorkspace(page);
  const cwd = await workspaceCwd(page);
  await sendTurn(page, 'Write the report', writeReport(cwd));
  await page.locator('.work-conversation .chat-msg').last().locator('code.file-ref').click();
  const pane = preview(page);
  const conversation = page.getByRole('region', { name: 'Workspace conversation' });
  await expect(pane.getByRole('tab', { selected: true })).toHaveText('report.md');
  await expect(pane.getByText('Mock file written by the agent.')).toBeVisible();

  const before = {
    pane: (await pane.boundingBox())!.width,
    paneText: await fontPx(page, '.preview-pane .preview-tab'),
    chatText: await fontPx(page, '.work-conversation .chat-input textarea'),
    preview: (await pane.locator('.chat-preview').first().boundingBox())!.width,
  };
  await dragHandle(page, -200);
  await expect.poll(async () => (await pane.boundingBox())!.width).toBeGreaterThan(before.pane + 150);
  // R03: resizing reflows; it never rescales type in either pane.
  expect(await fontPx(page, '.preview-pane .preview-tab')).toBe(before.paneText);
  expect(await fontPx(page, '.work-conversation .chat-input textarea')).toBe(before.chatText);
  await expect.poll(async () => (await pane.locator('.chat-preview').first().boundingBox())!.width).toBeGreaterThan(before.preview);
  // The page beside it never gets squeezed out.
  await dragHandle(page, -2000);
  expect((await conversation.boundingBox())!.width).toBeGreaterThanOrEqual(359);

  // The width is saved under the side panel's old key, so a dragged width
  // carries over a relaunch; double-click restores the default.
  const dragged = (await pane.boundingBox())!.width;
  await expect.poll(async () => Math.abs(Number(await page.evaluate(() => localStorage.getItem('sidePaneWidth'))) - dragged)).toBeLessThanOrEqual(2);
  await page.reload();
  await expect(page.locator('.work-header h1')).toHaveText('Release');
  // Nothing reopens by itself after a launch; open a file again.
  await expect(pane).toHaveCount(0);
  await page.evaluate(async path => {
    const store = '/src/store/preview.ts';
    (await import(/* @vite-ignore */ store)).openPreview(path);
  }, `${cwd}/notes/report.md`);
  await expect(pane.getByRole('tab', { selected: true })).toHaveText('report.md');
  await expect.poll(async () => Math.abs((await pane.boundingBox())!.width - dragged)).toBeLessThanOrEqual(2);
  const handle = page.getByRole('separator', { name: 'Resize preview' });
  await handle.dblclick();
  await expect.poll(async () => Math.abs((await pane.boundingBox())!.width - before.pane)).toBeLessThanOrEqual(2);
  await expect.poll(() => page.evaluate(() => localStorage.getItem('sidePaneWidth'))).toBeNull();

  // Keyboard: arrows resize the focused separator. The handle steps from the
  // width its ResizeObserver last reported, so wait until that has caught up
  // with the reset; pressing earlier steps from the stale dragged width.
  const start = (await pane.boundingBox())!.width;
  await expect(handle).toHaveAttribute('aria-valuenow', String(Math.round(start)));
  await handle.focus();
  await page.keyboard.press('ArrowLeft');
  await expect.poll(async () => Math.round((await pane.boundingBox())!.width - start)).toBe(16);

  // Closed, the conversation is not split: normal text size (the composer
  // writes at the 15px reading step, frontier F2).
  await pane.getByRole('button', { name: 'Close preview', exact: true }).click();
  await expect(pane).toHaveCount(0);
  await expect.poll(() => fontPx(page, '.work-conversation .chat-input textarea')).toBe(15);
});

test('a reply footer chip for a file the agent wrote opens it in the Preview pane', async ({ page }) => {
  test.skip(page.viewportSize()!.width <= 1100, 'the split is desktop-only');
  await openWorkspace(page);
  const cwd = await workspaceCwd(page);
  // The receipt names files relative to the vault (the mock's workspaces live
  // under /Users/fixture/vault); the chip finds the written file by that tail.
  const inVault = `${cwd.replace(/^\/Users\/fixture\/vault\//, '')}/notes/report.md`;
  const now = new Date().toISOString();
  await sendTurn(page, 'Write the report', [
    ...writeReport(cwd),
    { type: 'evidenceReceipt', receipt: {
      id: 'receipt-side-1', runId: 'run-side-pane', source: 'chat', sourceId: null, startedAt: now, finishedAt: now,
      status: 'success', verificationState: 'notVerified', checks: [], failures: [], historyRunId: 'run-side-pane', sessionKey: null,
      files: [{ path: inVault, changeType: 'create', attribution: 'confirmed', tool: 'Write', additions: 3, deletions: 0 }],
    } },
  ]);
  // Overnight 11-12 (C2, #1307): a reply's file chips sit above its
  // "Worked for" line, no longer in the footer.
  const chip = page.locator('.work-conversation').getByRole('list', { name: 'Changed files' }).getByRole('listitem').filter({ hasText: 'report.md' });
  await expect(chip).toHaveText('report.md+3−0');
  await expect(preview(page)).toHaveCount(0);
  await chip.click();
  await expect(preview(page).getByRole('tab', { selected: true })).toHaveText('report.md');
  await expect(preview(page).getByText('Mock file written by the agent.')).toBeVisible();
  await expect(page.locator('.work-header h1')).toHaveText('Release');
});

// It was the side panel's Visuals shelf test (panel-shelves.spec.ts): an
// expanded canvas keeps its frame (and its state) while more visuals arrive.
test('an expanded canvas stays mounted while other visuals arrive', async ({ page }) => {
  test.skip(page.viewportSize()!.width <= 1100, 'the split is desktop-only');
  await openWorkspace(page);
  await page.evaluate(async () => {
    const path = '/src/store/chatRuntime.ts'; const s = await import(/* @vite-ignore */ path);
    const code = (id: string) => '```hollow-canvas\n<hc-canvas id="' + id + '" title="' + id + '"><hc-stats><hc-stat label="Pass" value="3"></hc-stat></hc-stats></hc-canvas>\n```';
    s.setChatTurns([{ role: 'assistant', text: code('first') + '\n' + code('second'), tools: [] }]);
  });
  const first = page.locator('.work-conversation .chat-canvas').first();
  await expect(first).toBeVisible();
  await first.getByRole('button', { name: 'Expand', exact: true }).click();
  const pane = preview(page);
  await expect(pane.locator('iframe')).toBeVisible();
  await pane.locator('iframe').evaluate(el => el.setAttribute('data-retained', 'yes'));
  await page.evaluate(async () => {
    const path = '/src/store/chatRuntime.ts'; const s = await import(/* @vite-ignore */ path);
    s.setChatTurns([...s.chatTurns, { role: 'assistant', text: '```html\n<h1>Third</h1>\n```', tools: [] }]);
  });
  await expect.poll(() => page.evaluate(async () => {
    const path = '/src/store/chat.ts'; return (await import(/* @vite-ignore */ path)).chatVisuals().length;
  })).toBe(3);
  await expect(pane.locator('iframe')).toHaveAttribute('data-retained', 'yes');
});
