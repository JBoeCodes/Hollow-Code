import { showWorkspaceList } from './choose';
import { test, expect, type Page } from './fixtures/test';
import { chooseNew } from './choose';

// Build programme PR B4: leaving a conversation whose turn is still running
// keeps it running. Its events are buffered while another conversation is on
// screen and replayed when it comes back. The event bridge delivers backend
// events to the app's own listener (commands still go through ?mock=1).

async function installEventBridge(page: Page) {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (event: unknown) => void>();
    const listeners = new Map<string, number[]>();
    let next = 1;
    const w = window as unknown as Record<string, unknown>;
    w.__TAURI_INTERNALS__ = {
      transformCallback(callback: (event: unknown) => void) {
        const id = next++;
        callbacks.set(id, callback);
        return id;
      },
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
    localStorage.setItem('mockChatMintKeys', '1');
  });
}

function emitChat(page: Page, payload: Record<string, unknown>) {
  return page.evaluate(event => {
    (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void }).__emitTauriEvent('chat-event', event);
  }, payload);
}

async function createWorkspace(page: Page, name: string) {
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill(`Produce the ${name} result.`);
  await page.getByRole('textbox', { name: 'Project name' }).fill(name);
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText(name);
}

test('a running conversation keeps going while another is open, and comes back whole', async ({ page }, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'uses the wide header to switch');
  await installEventBridge(page);
  await page.goto('/?mock=1');
  await createWorkspace(page, 'Alpha');
  const composer = page.locator('.work-conversation').getByRole('textbox', { name: 'Chat message' });
  await composer.fill('Start the long build.');
  await composer.press('ControlOrMeta+Enter');
  const key = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}').sessionKey ?? null);
  // The accepted key is the one the store adopts; read it from the send reply.
  const adopted = await page.evaluate(() => localStorage.getItem('chatAgentSessionKey'));
  const k = (adopted ?? key) as string;
  expect(k).toMatch(/^s-/);
  await emitChat(page, { type: 'runStarted', runId: 'run-alpha', sessionKey: k, conversationKey: k, recall: [] });
  await emitChat(page, { type: 'assistantDelta', text: 'Part one', conversationKey: k });
  const conversation = page.locator('.work-conversation');
  await expect(conversation).toContainText('Part one');

  // Another workspace opens while Alpha's turn is still running.
  await createWorkspace(page, 'Beta');
  await emitChat(page, { type: 'assistantDelta', text: ' and part two', conversationKey: k });
  await expect(conversation).not.toContainText('Part one');
  await expect(conversation).not.toContainText('part two');

  // Back to Alpha: everything it said meanwhile is there, still working.
  await page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true }).click();
  await showWorkspaceList(page);
  await page.locator('.work-card', { hasText: 'Alpha' }).click();
  await expect(conversation).toContainText('Part one and part two');
  await emitChat(page, { type: 'done', sessionId: null, sessionKey: k, conversationKey: k });
  await expect(conversation.getByRole('textbox', { name: 'Chat message' })).toBeEnabled();
});


for (const theme of ['light', 'dark', 'glass']) test(`a background build completion refreshes that workspace PR without waiting for the cache TTL: ${theme}`, async ({ page }, info) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.addInitScript(theme => { localStorage.setItem('theme', theme === 'light' ? 'light' : 'dark'); localStorage.setItem('glass', theme === 'glass' ? 'on' : 'off'); }, theme);
  await installEventBridge(page);
  await page.addInitScript(() => {
    localStorage.setItem('hollow.railWorkspaces', JSON.stringify(['alpha', 'other']));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify(['alpha', 'other'].map(id => ({ key: `s-b7-${id}`, title: `${id} build`, agent: 'codex', cwd: `/fixture/${id}`, createdAt: new Date().toISOString(), lastActivityAt: new Date().toISOString(), turnCount: 1, stints: [] }))));
    localStorage.setItem('mockWorkspaces', JSON.stringify(['alpha', 'other'].map(id => ({ id, title: id, status: 'active', path: `workspaces/${id}`, cwd: `/fixture/${id}`, modified: '', build: { exists: true, branch: 'fake' } }))));
  });
  await page.goto('/?mock=1');
  await expect(page.locator('.app')).toBeVisible();
  await page.evaluate(async () => {
    const s = await import('/src/store/runtime.ts');
    const w = await import('/src/store/workspaces.ts');
    const { api } = await import('/src/api.ts');
    await w.refreshWorkspaces(); await w.refreshRailGitStates();
    const state = window as unknown as { b7Calls: {ids: string[], fresh: string[]}[] };
    state.b7Calls = [];
    const original = api.buildRailStates;
    api.buildRailStates = async (ids, fresh) => { state.b7Calls.push({ids, fresh: fresh ?? []}); return original(ids, fresh); };
    s.resetChatState(); s.setChatCwdPersistent('/fixture/alpha'); s.setChatAgentSessionKeyPersistent('s-b7-alpha'); s.setChatBusy(true);
    s.backgroundRunningChat();
    s.resetChatState(); s.setChatCwdPersistent('/fixture/other'); s.setChatAgentSessionKeyPersistent('s-b7-other');
    await w.refreshWorkspaces({join:true});
    // Let any chain queued by the background transition settle before testing done.
    await new Promise(resolve => setTimeout(resolve, 50));
    localStorage.setItem('mockRailPr', JSON.stringify({alpha:'open'}));
  });
  expect(await page.evaluate(() => (window as unknown as {b7Calls:{fresh:string[]}[]}).b7Calls.some(c => c.fresh.includes('alpha')))).toBe(false);
  await emitChat(page, { type: 'done', sessionKey: 's-b7-alpha', conversationKey: 's-b7-alpha', sessionId: null });
  await expect.poll(() => page.evaluate(() => (window as unknown as {b7Calls:{fresh:string[]}[]}).b7Calls.some(c => c.fresh.includes('alpha')))).toBe(true);
  await expect.poll(() => page.evaluate(async () => (await import('/src/store/workspaces.ts')).railGitStates().alpha?.prState)).toBe('open');
  expect(await page.evaluate(async () => (await import('/src/store/runtime.ts')).chatAgentSessionKey())).toBe('s-b7-other');
  const calls = await page.evaluate(() => (window as unknown as {b7Calls:{fresh:string[]}[]}).b7Calls);
  expect(calls.every(c => !c.fresh.includes('other'))).toBe(true);
  if ((page.viewportSize()?.width ?? 1000) > 860) await expect(page.locator('[title="PR #12 open · fake"]')).toBeVisible();
  if (theme === 'glass') { await expect(page.locator('html')).toHaveAttribute('data-glass', 'on'); await page.addStyleTag({content:'html[data-glass] { background:var(--canvas); }'}); }
  if (process.env.B7_CAPTURE_DIR) await page.screenshot({ path: `${process.env.B7_CAPTURE_DIR}/b7-${theme}-${info.project.name}.png` });
});
