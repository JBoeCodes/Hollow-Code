import { expect, test, type Page } from './fixtures/test';

/**
 * An open question card stays open when a turn finishes (overnight 11-08 E1).
 * The dock is the same one question-dock.spec.ts opens. Choosing moves to
 * "2 of 3", then a finished turn's `done` event must leave the dock and that
 * step where they are. Written for the morning browser run; this lane does
 * not run Playwright per PR.
 */

const composer = (page: Page) => page.getByRole('textbox', { name: 'Chat message' });
const dock = (page: Page) => page.locator('.question-dock');

const options = (ids: string[], recommended?: string) => ids.map(label => ({ id: label.toLowerCase().replace(/\W+/g, '-'), label, ...(label === recommended ? { recommended: true } : {}) }));

async function openWithCard(page: Page): Promise<void> {
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.locator('.settings-index').getByRole('button', { name: 'Advanced', exact: true }).click();
  await page.locator('#settings-group-this-mac > summary').click(); // History & recovery: Advanced → This Mac
  await page.getByRole('button', { name: 'Open Library, History', exact: true }).click();
  await page.locator('.library-tabs').getByRole('button', { name: 'Conversations', exact: true }).click(); // Settings PR 4 cut the Conversation history button
  const search = page.getByRole('textbox', { name: 'Search conversations' });
  const show = page.getByRole('button', { name: 'Show conversations', exact: true });
  await expect(search.or(show).first()).toBeVisible();
  if (!await search.isVisible()) await show.click();
  await page.locator('.chat-aside').getByRole('button', { name: 'Open Wire the session store', exact: true }).click();
  await expect(composer(page)).toBeVisible();
  const questions = [
    { id: 'install', header: 'Install main', prompt: 'Install main 4f196bf4 now?', required: true, answerMode: 'single', allowCustom: true, options: options(['Do it', 'Not now', 'Never'], 'Not now') },
    { id: 'codex', header: 'Codex read-only', prompt: 'Make Codex default to read-only?', required: true, answerMode: 'single', allowCustom: true, options: options(['Do it', 'Not now', 'Never'], 'Do it') },
    { id: 'checks', header: 'Hand checks', prompt: 'Which hand checks first?', required: false, answerMode: 'multi', allowCustom: false, options: options(['VoiceOver', 'Glass', 'Trackpad']) },
  ];
  await page.evaluate(async ([url, qs]) => {
    const rt = await import(url);
    rt.setChatSessionId('native-dock-1');
    const card = {
      id: 'int-dock-1', sessionKey: rt.chatAgentSessionKey(), turnOrd: 1, runId: 'run-dock', agent: 'claude',
      nativeSessionId: 'native-dock-1', toolUseId: 'tu-1', source: 'mcp', mode: 'questions', status: 'pending',
      request: { mode: 'questions', title: 'Morning check-in', supersedeOnUserMessage: true, questions: qs },
      answer: null, createdAt: '2026-10-14T12:00:00Z', resolvedAt: null,
    };
    localStorage.setItem('mockPendingInteractions', JSON.stringify([card]));
    await rt.refreshChatInteractions();
  }, ['/src/store/chatRuntime.ts', questions] as const);
  await expect(dock(page)).toBeVisible();
  await page.waitForTimeout(550);
}

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name.includes('narrow'), 'Conversation history opens from the desktop settings index');
  await page.goto('/?mock=1');
});

test('a finished turn leaves the open card and the answered step', async ({ page }) => {
  await openWithCard(page);
  await expect(dock(page).locator('.question-dock-count')).toHaveText('1 of 3');
  await dock(page).getByRole('option', { name: 'Do it', exact: true }).click();
  await expect(dock(page).locator('.question-dock-count')).toHaveText('2 of 3');
  const delivered = await page.evaluate(async () => {
    const rt = await import('/src/store/chatRuntime.ts');
    const conversationKey = rt.chatAgentSessionKey();
    const emit = (window as unknown as { __mockEmit: (name: string, payload: unknown) => number }).__mockEmit;
    return emit('chat-event', { type: 'done', conversationKey });
  });
  expect(delivered).toBeGreaterThan(0);
  await expect(dock(page)).toBeVisible();
  await expect(dock(page).locator('.question-dock-count')).toHaveText('2 of 3');
  const statuses = await page.evaluate(async () => {
    const rt = await import('/src/store/chatRuntime.ts');
    return rt.chatInteractions().map((card: { status: string }) => card.status);
  });
  expect(statuses).toEqual(['pending']);
});
