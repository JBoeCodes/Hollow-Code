import { expect, test, type Page } from './fixtures/test';
import { chooseNew } from './choose';

const crossing = 'a -> b -> c\nc -> a\na -> d';
const canvas = (text: string) => `<hc-canvas id="layout-test" title="Layout test"><hc-flow label="Release">${text}</hc-flow></hc-canvas>`;
const calls = (page: Page, name: string) => page.evaluate(n => window.__mockApiCalls?.[n] ?? 0, name);
const emit = (page: Page, payload: Record<string, unknown>) => page.evaluate(p => {
  (window as unknown as { emit: (p: unknown) => void }).emit(p);
}, payload);
async function reply(page: Page, text = crossing, finish = true) {
  const composer = page.getByLabel('Chat message'), before = await calls(page, 'chat_send');
  await composer.fill('Draw a flow'); await composer.press('Meta+Enter');
  await expect.poll(() => calls(page, 'chat_send')).toBe(before + 1);
  await emit(page, { type: 'runStarted', asked: 'Draw a flow', runId: `layout-${before}`, recall: [] });
  await emit(page, { type: 'assistantDelta', text: `\n\n\`\`\`hollow-canvas\n${canvas(text)}\n\`\`\`` });
  if (finish) await emit(page, { type: 'done', sessionId: null });
}
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (event: unknown) => void>(), listeners = new Map<string, number[]>(); let next = 1;
    const w = window as unknown as Record<string, unknown>;
    w.__TAURI_INTERNALS__ = {
      transformCallback(cb: (event: unknown) => void) { const id = next++; callbacks.set(id, cb); return id; },
      invoke(command: string, args: { event?: string; handler?: number }) {
        if (command === 'plugin:event|listen' && args?.event && args.handler) {
          listeners.set(args.event, [...(listeners.get(args.event) ?? []), args.handler]); return Promise.resolve(next++);
        }
        if (command === 'plugin:event|unlisten') return Promise.resolve();
        return Promise.reject(new Error(`not in Tauri: ${command}`));
      },
    };
    w.emit = (payload: unknown) => { for (const id of listeners.get('chat-event') ?? []) callbacks.get(id)?.({ event: 'chat-event', id, payload }); };
    localStorage.setItem('chatAgent', 'claude');
  });
  await page.goto('/?mock=1'); await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Check flow layout.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Layouts');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Layouts');
});

test('settled crossing drafts a fix, keeps existing text and attaches without sending', async ({ page }) => {
  await page.evaluate(async () => {
    const theme = { name: 'Paper', source: '', accent: '#aaaaaa', foreground: '#999999', background: '#fef0d0', isDark: false, colors: Array(16).fill(null) };
    localStorage.setItem('mockCustomThemes', JSON.stringify([theme]));
    const { api } = await import('/src/api.ts');
    const { refreshWorkspaces } = await import('/src/store/workspaces.ts');
    await api.setWorkspaceCanvasTheme((await api.listWorkspaces()).find(w => w.title === 'Layouts')!.id, 'Paper');
    await refreshWorkspaces();
  });
  await reply(page, crossing, false);
  await expect(page.locator('.chat-canvas-layout')).toHaveCount(0);
  await emit(page, { type: 'done', sessionId: null });
  const audit = page.locator('.work-conversation .chat-canvas-layout');
  await expect(audit.locator('summary')).toContainText('1 layout problem');
  await audit.locator('summary').click();
  await expect(audit).toContainText('may run through');
  await audit.screenshot({ path: `${process.env.HOLLOW_SCRATCH_DIR ?? test.info().outputDir}/r11-findings.png` });
  const before = await calls(page, 'chat_send');
  await page.getByLabel('Chat message').fill('Keep my draft.');
  await audit.getByRole('button', { name: 'Ask to fix' }).click();
  await expect(page.getByLabel('Chat message')).toHaveValue(/Keep my draft\.[\s\S]*id layout-test[\s\S]*same id/);
  await expect(page.locator('.chat-attach-remove')).toHaveCount(1);
  expect(await calls(page, 'chat_send')).toBe(before);
  const request = await page.evaluate(() => JSON.parse(localStorage.getItem('mockCanvasLastExport')!));
  expect(request).toMatchObject({ target: 'attachment', preset: 'doc', look: 'match', format: 'png', copy: false });
  expect(request.themeName).toBe('Paper');
  expect(Object.fromEntries(request.theme)['--app-canvas']).toBe('#fef0d0');
  expect(request.source).toBe(canvas(crossing));
  await page.screenshot({ path: `${process.env.HOLLOW_SCRATCH_DIR ?? test.info().outputDir}/r11-layout.png` });
});
// Repo ideas 2026-10-27 #1: the same row carries the design check of the
// canvas's hand-written parts, from its source and the frame's measured report.
test('hand-written HTML gets design findings in the same row, without sending', async ({ page }) => {
  const composer = page.getByLabel('Chat message'), before = await calls(page, 'chat_send');
  await composer.fill('Draw a canvas'); await composer.press('Meta+Enter');
  await expect.poll(() => calls(page, 'chat_send')).toBe(before + 1);
  await emit(page, { type: 'runStarted', asked: 'Draw a canvas', runId: `design-${before}`, recall: [] });
  const html = `<hc-canvas id="design-test" title="Design test"><style>.fine{color:#777;font-size:9px}</style><p class="fine">Fine print</p><hc-flow label="Release">${crossing}</hc-flow></hc-canvas>`;
  await emit(page, { type: 'assistantDelta', text: `\n\n\`\`\`hollow-canvas\n${html}\n\`\`\`` });
  await emit(page, { type: 'done', sessionId: null });
  const audit = page.locator('.work-conversation .chat-canvas-layout');
  await expect(audit).toHaveCount(1);
  await expect(audit.locator('summary')).toContainText('layout and design problems');
  await audit.locator('summary').click();
  await expect(audit).toContainText('Hand-written HTML and CSS');
  await expect(audit).toContainText('written as literals');
  // The frame's measured report replaces the source's size guess.
  await expect(audit).toContainText('"Fine print" renders at 9px');
  expect(await audit.locator('.chat-canvas-design li').count()).toBeLessThanOrEqual(5);
  const sends = await calls(page, 'chat_send');
  await audit.getByRole('button', { name: 'Ask to fix' }).click();
  await expect(composer).toHaveValue(/id design-test[\s\S]*hand-written HTML and CSS[\s\S]*same id/);
  expect(await calls(page, 'chat_send')).toBe(sends);
});
test('clean diagrams and earlier revisions have no layout line', async ({ page }) => {
  await reply(page);
  await expect(page.locator('.work-conversation .chat-canvas-layout')).toHaveCount(1);
  await reply(page, 'a -> b');
  await expect(page.locator('.work-conversation .chat-canvas').last()).toHaveAttribute('data-revision', '2');
  await expect(page.locator('.work-conversation .chat-canvas-layout')).toHaveCount(0);
});
test('large diagrams require Check layout', async ({ page }) => {
  await reply(page, Array.from({length:40}, (_, i) => `n${i}: ${i === 0 ? 'x'.repeat(70) : 'Step'}`).join('\n'));
  const check = page.getByRole('button', { name: 'Check layout', exact: true });
  await expect(check).toBeVisible();
  await expect(page.locator('.chat-canvas-layout')).toHaveCount(0);
  await check.click();
  await expect(page.locator('.chat-canvas-layout summary')).toContainText('1 layout problem');
});
test('capture failure preserves the useful text draft', async ({ page }) => {
  await page.evaluate(() => localStorage.setItem('mockCanvasExportError', 'Capture timed out'));
  await reply(page);
  await page.locator('.chat-canvas-layout summary').click();
  await page.getByRole('button', { name: 'Ask to fix', exact: true }).click();
  await expect(page.locator('.chat-canvas-note[role=status]')).toContainText("Couldn't attach a picture");
  await expect(page.getByLabel('Chat message')).toHaveValue(/id layout-test/);
  await expect(page.locator('.chat-attach-remove')).toHaveCount(0);
});
test('a changed draft discards a late capture', async ({ page }) => {
  await page.evaluate(() => localStorage.setItem('mockCanvasExportDelay', '500'));
  await reply(page);
  await page.locator('.chat-canvas-layout summary').click();
  await page.getByRole('button', { name: 'Ask to fix', exact: true }).click();
  await expect.poll(() => calls(page, 'canvas_export')).toBe(1);
  await page.getByLabel('Chat message').fill('A different draft');
  await expect(page.getByRole('button', { name: 'Ask to fix', exact: true })).toBeEnabled();
  await expect(page.locator('.chat-attach-remove')).toHaveCount(0);
});
test('other agents get a complete text draft without capture', async ({ page }) => {
  await reply(page);
  await page.evaluate(async () => { const s = await import('/src/store/chatRuntime.ts'); s.setChatAgent('grok'); });
  await page.locator('.chat-canvas-layout summary').click();
  await page.getByRole('button', { name: 'Ask to fix', exact: true }).click();
  await expect(page.getByLabel('Chat message')).toHaveValue(/id layout-test/);
  expect(await calls(page, 'canvas_export')).toBe(0);
  await expect(page.locator('.chat-attach-remove')).toHaveCount(0);
});

test('expanded current canvas shows the same audit in dark theme', async ({ page }) => {
  await page.evaluate(() => { document.documentElement.dataset.theme = 'dark'; });
  await reply(page);
  await page.locator('.work-conversation').getByRole('button', { name: 'Expand', exact: true }).click();
  const expanded = page.locator('.side-expanded-canvas');
  await expect(expanded.locator('.chat-canvas-layout summary')).toContainText('1 layout problem');
  await expanded.locator('.chat-canvas-layout summary').click();
  await expanded.getByRole('button', { name: 'Ask to fix', exact: true }).click();
  await expect(page.getByLabel('Chat message')).toHaveValue(/id layout-test/);
  await expect(page.locator('.chat-attach-remove')).toHaveCount(1);
  await expanded.screenshot({ path: `${process.env.HOLLOW_SCRATCH_DIR ?? test.info().outputDir}/r11-expanded-dark.png` });
});

test('conversation version change discards a late capture', async ({ page }) => {
  await page.evaluate(() => localStorage.setItem('mockCanvasExportDelay', '500'));
  await reply(page);
  await page.locator('.chat-canvas-layout summary').click();
  await page.getByRole('button', { name: 'Ask to fix', exact: true }).click();
  await expect.poll(() => calls(page, 'canvas_export')).toBe(1);
  await page.evaluate(async () => { const s = await import('/src/store/chatRuntime.ts'); s.setChatConversationVersion(v => v + 1); });
  await expect(page.getByRole('button', { name: 'Ask to fix', exact: true })).toBeEnabled();
  await expect(page.locator('.chat-attach-remove')).toHaveCount(0);
});

test('a missing composer sink never queues the capture for the next composer', async ({ page }) => {
  await reply(page);
  await page.evaluate(async () => {
    const s = await import('/src/store/chatRuntime.ts');
    const release = s.registerChatImageSink(() => {}); release();
  });
  await page.locator('.chat-canvas-layout summary').click();
  await page.getByRole('button', { name: 'Ask to fix', exact: true }).click();
  await expect(page.locator('.chat-canvas-note[role=status]')).toContainText("Couldn't attach a picture");
  const late = await page.evaluate(async () => {
    const s = await import('/src/store/chatRuntime.ts'); let count = 0;
    const release = s.registerChatImageSink(() => { count++; }); release(); return count;
  });
  expect(late).toBe(0);
  await expect(page.getByLabel('Chat message')).toHaveValue(/id layout-test/);
});

for (const missing of [false, true]) test(`correction palette read ${missing ? 'failure explains the missing theme' : 'discards after a draft change'}`, async ({ page }) => {
  await reply(page);
  await page.evaluate(async missing => {
    const { api } = await import('/src/api.ts');
    const { refreshWorkspaces } = await import('/src/store/workspaces.ts');
    localStorage.setItem('mockCustomThemes', JSON.stringify([{ name: 'Paper', source: '', accent: '#aaaaaa', foreground: '#999999', background: '#fef0d0', isDark: false, colors: Array(16).fill(null) }]));
    await api.setWorkspaceCanvasTheme((await api.listWorkspaces()).find(w => w.title === 'Layouts')!.id, 'Paper');
    await refreshWorkspaces();
    const read = api.readCustomTheme;
    api.readCustomTheme = async name => {
      if (missing) throw new Error('Theme removed');
      const value = await read(name);
      return new Promise(resolve => { (window as any).__releasePalette = () => resolve(value); });
    };
  }, missing);
  await page.locator('.chat-canvas-layout summary').click();
  await page.getByRole('button', { name: 'Ask to fix', exact: true }).click();
  if (missing) {
    await expect(page.locator('.chat-canvas-note[role=status]')).toContainText("Couldn't read canvas look “Paper”");
    await expect(page.getByLabel('Chat message')).toHaveValue(/id layout-test/);
  } else {
    await expect.poll(() => page.evaluate(() => typeof (window as any).__releasePalette)).toBe('function');
    await page.getByLabel('Chat message').fill('New draft');
    await page.evaluate(() => (window as any).__releasePalette());
    await expect(page.getByRole('button', { name: 'Ask to fix', exact: true })).toBeEnabled();
  }
  expect(await calls(page, 'canvas_export')).toBe(0);
  await expect(page.locator('.chat-attach-remove')).toHaveCount(0);
});
