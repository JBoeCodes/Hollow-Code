import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

/** P1-27, chat transcript fixes: Esc in the Connectors popover, prefills
 * that keep a draft, find inside a closed "Worked for…" fold, Retry under a
 * failed turn, Preferences, Copy conversation and "Finished ↓"; and (C3)
 * your message's bubble, its actions and Edit, and Retry under a finished
 * last reply. Commands go
 * through the `?mock=1` IPC; a turn's events through a Tauri event bridge.
 * Nothing leaves the browser. */

const MOCK_KEY = 's-20260925-mock0001';

function callCount(page: Page, command: string) {
  return page.evaluate(cmd => window.__mockApiCalls?.[cmd] ?? 0, command);
}

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

function emit(page: Page, payload: Record<string, unknown>) {
  return page.evaluate(event => {
    (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void }).__emitTauriEvent('chat-event', event);
  }, payload);
}

async function openChat(page: Page) {
  await page.goto('/?mock=1');
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), 'advanced');
  } else {
    await page.locator('.settings-index').getByRole('button', { name: 'Advanced', exact: true }).click();
  }
  await page.locator('#settings-group-this-mac > summary').click(); // History & recovery: Advanced → This Mac
  await page.getByRole('button', { name: 'Open Library, History', exact: true }).click();
  await page.locator('.library-tabs').getByRole('button', { name: 'Conversations', exact: true }).click(); // Settings PR 4 cut the Conversation history button
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toBeVisible();
}

/** Send a message and start its turn; returns the turn's tag. */
async function startTurn(page: Page, prompt: string, runId: string) {
  const sends = await callCount(page, 'chat_send');
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await composer.fill(prompt);
  await composer.press('Meta+Enter');
  await expect.poll(() => callCount(page, 'chat_send')).toBe(sends + 1);
  const clientTurnId = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}').clientTurnId);
  const tag = { clientTurnId, sessionKey: MOCK_KEY };
  await emit(page, { type: 'runStarted', runId, recall: [], ...tag });
  return tag;
}

const PROSE = Array.from({ length: 60 }, (_, i) => `Closing paragraph ${i + 1} of the release notes.`).join('\n\n');

for (const tools of [false, true]) test(`short transcript jump stays above content ${tools ? 'with' : 'without'} conversation tools`, async ({ page }) => {
  await page.setViewportSize({ width: 720, height: 480 });
  await page.addInitScript(() => localStorage.setItem('hollow.studio.textScale', '125'));
  await installEventBridge(page);
  await openChat(page);
  await page.locator('.library-conversations').getByRole('button', { name: 'New chat', exact: true }).click();
  const tag = await startTurn(page, 'Long notes', 'run-short-1');
  await emit(page, { type: 'assistantText', text: PROSE, ...tag });
  await emit(page, { type: 'done', sessionId: null, ...tag });
  if (tools) {
    const next = await startTurn(page, 'Add a conclusion', 'run-short-2');
    await emit(page, { type: 'assistantText', text: 'Finished.', ...next });
    await emit(page, { type: 'done', sessionId: null, ...next });
  }
  const scroller = page.locator('.chat-main .chat-scroll');
  await scroller.hover();
  await page.mouse.wheel(0, -4000);
  const jump = page.locator('.chat-main .chat-jump-latest');
  await expect(jump).toBeVisible();
  await expect(page.locator('.chat-main .chat-nav-tools')).toHaveCount(0);
  if (tools) await expect(page.getByRole('button', { name: 'Actions for this conversation', exact: true })).toBeVisible();
  const content = (await scroller.boundingBox())!;
  const button = (await jump.boundingBox())!;
  // Keep at least one full action row readable, not a one-pixel scroll slit.
  expect(content.height).toBeGreaterThanOrEqual(button.height);
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toBeInViewport();
  expect(await jump.evaluate(el => { const r = el.getBoundingClientRect(); return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2)); })).toBe(true);
  expect(button.y + button.height).toBeLessThanOrEqual(content.y + 1);
  for (const tool of await page.locator('.chat-main .chat-nav-btn').all()) {
    expect(button.x + button.width).toBeLessThanOrEqual((await tool.boundingBox())!.x);
  }
  await jump.click();
  await expect(jump).toHaveCount(0);
  const action = scroller.locator('.turn-footer button').last();
  await action.scrollIntoViewIfNeeded();
  expect(await action.evaluate(el => { const r = el.getBoundingClientRect(); return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2)); })).toBe(true);
});

test('Esc in the Folder and connectors menu closes it and keeps the run', async ({ page }, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'the composer hides Connectors on phone-narrow widths');
  await installEventBridge(page);
  await openChat(page);
  await startTurn(page, 'Refactor the parser', 'run-cx-1');
  const button = page.getByRole('button', { name: /^Folder and connectors:/ });
  const menu = page.getByRole('menu', { name: /^Folder and connectors:/ });
  const stop = page.getByRole('button', { name: 'Stop', exact: true });
  await expect(stop).toBeVisible();

  // From the keyboard: Esc closes the menu and gives focus back to its button.
  await button.focus();
  await page.keyboard.press('Enter');
  await expect(menu).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(menu).toBeHidden();
  await expect(button).toBeFocused();
  expect(await callCount(page, 'chat_cancel')).toBe(0);
  await expect(stop).toBeVisible();

  // WKWebView doesn't focus a clicked button: focus stays in the composer.
  // That one Esc closes the menu only: the slash command stays typed.
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await composer.fill('/he');
  await expect(page.locator('.slash-popup')).toBeVisible();
  await button.click();
  await expect(menu).toBeVisible();
  await composer.focus();
  await page.keyboard.press('Escape');
  await expect(menu).toBeHidden();
  await expect(composer).toHaveValue('/he');
  expect(await callCount(page, 'chat_cancel')).toBe(0);
  await composer.fill('');

  // A click outside closes it too, and opening it reads the library again.
  const reads = await callCount(page, 'list_connectors');
  await button.click();
  await expect.poll(() => callCount(page, 'list_connectors')).toBe(reads + 1);
  await page.locator('.chat-scroll').click({ position: { x: 10, y: 10 } });
  await expect(menu).toBeHidden();
  expect(await callCount(page, 'chat_cancel')).toBe(0);
});

test('Teach keeps a half-written draft', async ({ page }) => {
  await installEventBridge(page);
  await openChat(page);
  const tag = await startTurn(page, 'Check the release', 'run-teach-1');
  await emit(page, { type: 'assistantText', text: 'The release is ready.', ...tag });
  await emit(page, { type: 'done', sessionId: null, ...tag });
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await composer.fill('abc');
  const answer = page.locator('.chat-main .turn.assistant').filter({ hasText: 'The release is ready.' });
  // The thumbs sit on every reply's row again (C2); Teach lives under More.
  await expect(answer.getByRole('button', { name: 'Useful', exact: true })).toHaveCount(1);
  await expect(answer.getByRole('button', { name: 'Not useful', exact: true })).toHaveCount(1);
  await answer.getByRole('button', { name: 'More actions', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Teach Hollow…', exact: true }).click();
  await expect(composer).toHaveValue(/^abc\n\nTeach Hollow from this exchange/);
  await answer.getByRole('button', { name: 'More actions', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Teach Hollow…', exact: true }).click();
  expect((await composer.inputValue()).match(/Teach Hollow/g)).toHaveLength(1);
  expect(await callCount(page, 'chat_send')).toBe(1);
});

test('a find hit inside a closed fold opens it and scrolls to the step', async ({ page }) => {
  await installEventBridge(page);
  await openChat(page);
  const tag = await startTurn(page, 'Prepare the release notes', 'run-fold-1');
  await emit(page, { type: 'assistantText', text: 'Reading the changelog.', ...tag });
  await emit(page, { type: 'toolUse', id: 'tu-1', name: 'Read', input: { file_path: '/vault/CHANGELOG.md' }, ...tag });
  await emit(page, { type: 'toolResult', id: 'tu-1', content: 'ok', isError: false, ...tag });
  await emit(page, { type: 'turnBoundary', ...tag });
  await emit(page, { type: 'assistantText', text: 'The zebra migration needs a note.', ...tag });
  await emit(page, { type: 'toolUse', id: 'tu-2', name: 'Read', input: { file_path: '/vault/MIGRATIONS.md' }, ...tag });
  await emit(page, { type: 'toolResult', id: 'tu-2', content: 'ok', isError: false, ...tag });
  await emit(page, { type: 'turnBoundary', ...tag });
  await emit(page, { type: 'assistantText', text: PROSE, ...tag });
  await emit(page, { type: 'done', sessionId: null, ...tag });

  const head = page.locator('.chat-main .chat-run-fold-head');
  await expect(head).toHaveAttribute('aria-expanded', 'false');
  await page.getByRole('button', { name: 'Actions for this conversation', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Find in this conversation', exact: true }).click();
  await page.getByRole('searchbox', { name: 'Find in this conversation' }).fill('zebra');
  await expect(page.locator('.chat-find-count')).toHaveText('1 of 1');
  await expect(head).toHaveAttribute('aria-expanded', 'true');
  const step = page.locator('.chat-main .chat-run-step.turn-hit');
  await expect(step).toHaveCount(1);
  await expect(step).toContainText('The zebra migration needs a note.');
  await expect(step).toBeInViewport();
});

test('a failed last turn offers Retry, which sends the message once', async ({ page }) => {
  await installEventBridge(page);
  await openChat(page);
  const tag = await startTurn(page, 'Draft the release notes', 'run-fail-1');
  await emit(page, { type: 'error', message: 'spawn claude: No such file or directory', ...tag });
  await emit(page, { type: 'done', sessionId: null, ...tag });

  const retry = page.getByRole('button', { name: 'Retry', exact: true });
  await expect(retry).toBeVisible();
  await expect(page.getByRole('button', { name: 'Retry with another agent', exact: true })).toBeVisible();
  // Not a stopped run: no Continue.
  await expect(page.getByRole('button', { name: 'Continue', exact: true })).toHaveCount(0);
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await composer.fill('a draft of mine');
  await retry.click();
  await expect.poll(() => callCount(page, 'chat_send')).toBe(2);
  // The same message, composed as any send is (page context first).
  expect(await page.evaluate(() => localStorage.getItem('mockLastChatPrompt'))).toMatch(/\n\nDraft the release notes$/);
  await expect(retry).toHaveCount(0);
  await expect(composer).toHaveValue('a draft of mine');
  expect(await callCount(page, 'chat_send')).toBe(2);
});

test('Retry with another agent switches the agent, then sends the message', async ({ page }) => {
  await installEventBridge(page);
  await openChat(page);
  const tag = await startTurn(page, 'Draft the release notes', 'run-fail-2');
  await emit(page, { type: 'error', message: 'usage limit reached', ...tag });
  await emit(page, { type: 'done', sessionId: null, ...tag });
  await page.getByRole('button', { name: 'Retry with another agent', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Codex', exact: true }).click();
  await expect.poll(() => callCount(page, 'chat_send')).toBe(2);
  const args = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}'));
  expect(args.agent).toBe('codex');
  expect(String(args.prompt)).toContain('Draft the release notes');
});

test('Copy conversation puts the whole conversation on the clipboard as Markdown', async ({ page }) => {
  await installEventBridge(page);
  await page.addInitScript(() => {
    Object.defineProperty(navigator.clipboard, 'writeText', { configurable: true, value: async (text: string) => { (window as unknown as { copied: string }).copied = text; } });
  });
  await openChat(page);
  let tag = await startTurn(page, 'First question', 'run-copy-1');
  await emit(page, { type: 'assistantText', text: 'First answer.', ...tag });
  await emit(page, { type: 'done', sessionId: null, ...tag });
  tag = await startTurn(page, 'Second question', 'run-copy-2');
  await emit(page, { type: 'assistantText', text: 'Second answer.\n\n```hollow-next\nShip it\n```', ...tag });
  await emit(page, { type: 'done', sessionId: null, ...tag });
  await page.getByRole('button', { name: 'Actions for this conversation', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Copy conversation (Markdown)', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { copied?: string }).copied ?? '')).toContain('## You\n\nFirst question');
  const copied = await page.evaluate(() => (window as unknown as { copied: string }).copied);
  expect(copied).toContain('## Claude\n\nSecond answer.');
  expect(copied).not.toContain('hollow-next');
});

test('Preferences opens Settings at Knowledge', async ({ page }) => {
  await openChat(page);
  const pill = page.getByRole('button', { name: 'Preferences', exact: true });
  await expect(pill).toHaveAttribute('title', /Settings → Personalization/);
  await pill.click();
  // Knowledge is Personalization since Settings PR 3.
  await expect(page.getByRole('heading', { name: 'Personalization', exact: true, level: 2 })).toBeVisible();
});

test('a run that ends below the view says Finished on Jump to latest', async ({ page }) => {
  await installEventBridge(page);
  await openChat(page);
  const tag = await startTurn(page, 'Write the long notes', 'run-fin-1');
  await emit(page, { type: 'assistantText', text: PROSE, ...tag });
  const scroller = page.locator('.chat-main .chat-scroll');
  await scroller.hover();
  await page.mouse.wheel(0, -4000);
  const jump = page.locator('.chat-main .chat-jump-latest');
  await expect(jump).toBeVisible();
  await expect(jump).toHaveText('Jump to latest');
  await expect(jump).toHaveAccessibleName('Jump to latest');
  await emit(page, { type: 'done', sessionId: null, ...tag });
  await expect(jump).toHaveText('Finished');
  // Its name starts with the word it shows.
  await expect(jump).toHaveAccessibleName('Finished: jump to latest');
  await jump.click();
  await expect(jump).toHaveCount(0);
});

test('an error after a successful result is bookkeeping: no Retry', async ({ page }) => {
  await installEventBridge(page);
  await openChat(page);
  const tag = await startTurn(page, 'Summarize the notes', 'run-ok-1');
  await emit(page, { type: 'assistantText', text: 'Here is the summary.', ...tag });
  await emit(page, { type: 'result', result: 'Here is the summary.', costUsd: null, isError: false, subtype: 'success', sessionId: null, ...tag });
  await emit(page, { type: 'error', message: 'history finalization failed: disk full', ...tag });
  await emit(page, { type: 'done', sessionId: null, ...tag });
  await expect(page.locator('.chat-main')).toContainText('history finalization failed');
  // No failure row. The reply finished, so its footer may offer its own
  // Retry (C3); that one is covered below.
  await expect(page.locator('.chat-main .chat-retry')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Retry with another agent', exact: true })).toHaveCount(0);
});

test('a live stall offers Continue, not Retry', async ({ page }) => {
  await installEventBridge(page);
  await openChat(page);
  const tag = await startTurn(page, 'Run the long check', 'run-stall-1');
  await emit(page, { type: 'assistantText', text: 'Checking.', ...tag });
  await emit(page, { type: 'error', message: 'The agent stopped responding.', resultKind: 'stalled', ...tag });
  await emit(page, { type: 'done', sessionId: 'native-stall-1', ...tag });
  await expect(page.getByRole('button', { name: 'Continue', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Retry', exact: true })).toHaveCount(0);
});

// Overnight 2026-11-04 C3: your message as a quiet bubble with hover
// actions, Edit on the last one, and Retry under a finished last reply.

async function finishTurn(page: Page, prompt: string, runId: string, answer: string) {
  const tag = await startTurn(page, prompt, runId);
  await emit(page, { type: 'assistantText', text: answer, ...tag });
  await emit(page, { type: 'result', result: answer, costUsd: null, isError: false, subtype: 'success', sessionId: null, ...tag });
  await emit(page, { type: 'done', sessionId: null, ...tag });
}

test('your message is a bubble with no visible label; hover shows its time, Copy and Edit on the last one', async ({ page }) => {
  await installEventBridge(page);
  await page.addInitScript(() => {
    Object.defineProperty(navigator.clipboard, 'writeText', { configurable: true, value: async (text: string) => { (window as unknown as { copied: string }).copied = text; } });
  });
  await openChat(page);
  await finishTurn(page, 'First question', 'run-msg-1', 'First answer.');
  await finishTurn(page, 'Second question', 'run-msg-2', 'Second answer.');
  const mine = page.locator('.chat-main .turn.user');
  const last = mine.last();
  // "Prompt" is its name for screen readers only; no visible label or band.
  await expect(last.locator('.chat-user-label')).toHaveCount(0);
  await expect(last.locator('.chat-user-chip .sr-only')).toHaveText('Prompt');
  await expect(last.locator('.chat-user-chip .text')).toHaveText('Second question');
  const chip = (await last.locator('.chat-user-chip').boundingBox())!;
  const column = (await page.locator('.chat-main .chat-scroll').boundingBox())!;
  expect(chip.width).toBeLessThan(column.width * 0.86);
  // The actions wait for hover (or keyboard focus).
  const actions = last.getByRole('group', { name: 'Message actions' });
  await page.getByRole('textbox', { name: 'Chat message' }).hover();
  await expect(actions).toHaveCSS('opacity', '0');
  await last.locator('.chat-user-chip').hover();
  await expect(actions).toHaveCSS('opacity', '1');
  await expect(actions.locator('time.chat-user-time')).toHaveText(/\d{1,2}:\d{2}/);
  await expect(actions.locator('time.chat-user-time')).toHaveAttribute('title', /^Sent /);
  await actions.getByRole('button', { name: 'Copy message', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { copied?: string }).copied ?? '')).toBe('Second question');
  // Edit is on the last message only.
  await expect(page.locator('.chat-main').getByRole('button', { name: 'Edit message', exact: true })).toHaveCount(1);
  await expect(actions.getByRole('button', { name: 'Edit message', exact: true })).toHaveAttribute('title', /the earlier answer stays/);
  await expect(mine.first().getByRole('button', { name: 'Edit message', exact: true })).toHaveCount(0);
});

test('Edit puts your last message back in the box before your draft, and sends nothing', async ({ page }) => {
  await installEventBridge(page);
  await openChat(page);
  await finishTurn(page, 'Draft the release notes', 'run-edit-1', 'Here are the notes.');
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await composer.fill('a draft of mine');
  const users = await page.locator('.chat-main .turn.user').count();
  const last = page.locator('.chat-main .turn.user').last();
  await last.locator('.chat-user-chip').hover();
  await last.getByRole('button', { name: 'Edit message', exact: true }).click();
  await expect(composer).toHaveValue('Draft the release notes\n\na draft of mine');
  await expect(composer).toBeFocused();
  expect(await composer.evaluate(el => { const t = el as HTMLTextAreaElement; return t.selectionStart === t.value.length && t.selectionEnd === t.value.length; })).toBe(true);
  // Nothing was sent, and the message and its answer stay.
  expect(await callCount(page, 'chat_send')).toBe(1);
  await expect(page.locator('.chat-main .turn.user')).toHaveCount(users);
  await expect(page.locator('.chat-main .turn.assistant').filter({ hasText: 'Here are the notes.' })).toHaveCount(1);
});

test('a finished last reply offers Retry, which sends the message again as a new message', async ({ page }) => {
  await installEventBridge(page);
  await openChat(page);
  await finishTurn(page, 'Draft the release notes', 'run-regen-1', 'Here are the notes.');
  // The reply's own footer, not the failure row.
  await expect(page.locator('.chat-main .chat-retry')).toHaveCount(0);
  const retry = page.locator('.chat-main .turn-footer').getByRole('button', { name: 'Retry', exact: true });
  await expect(retry).toHaveCount(1);
  await expect(retry).toHaveAttribute('title', 'Send your last message again as a new message');
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await composer.fill('a draft of mine');
  const users = await page.locator('.chat-main .turn.user').count();
  await retry.click();
  await expect.poll(() => callCount(page, 'chat_send')).toBe(2);
  expect(await page.evaluate(() => localStorage.getItem('mockLastChatPrompt'))).toMatch(/\n\nDraft the release notes$/);
  // A new message; the earlier reply stays, the draft is untouched, and
  // Retry goes while the new turn runs.
  await expect(page.locator('.chat-main .turn.user')).toHaveCount(users + 1);
  await expect(page.locator('.chat-main .turn.assistant').filter({ hasText: 'Here are the notes.' })).toHaveCount(1);
  await expect(composer).toHaveValue('a draft of mine');
  await expect(page.getByRole('button', { name: 'Retry', exact: true })).toHaveCount(0);
});

// Retry with another model (overnight 11-07, D1, audit gap 10): this agent's
// other models, from the model menu's catalog, in More under the latest reply
// and under a failed last turn. The model becomes the chat's own, as an agent
// switch does.
test('a finished reply’s More menu offers Retry with another model, which keeps the model for the chat', async ({ page }) => {
  await installEventBridge(page);
  await openChat(page);
  await finishTurn(page, 'Draft the release notes', 'run-model-1', 'Here are the notes.');
  await page.locator('.chat-main .turn-footer').last().getByRole('button', { name: 'More actions', exact: true }).click();
  // The catalog in its own order, four at most (the mock lists five).
  for (const name of ['Opus 5.5', 'Fable 5.1', 'Sonnet 5.5', 'Haiku 5.5']) await expect(page.getByRole('menuitem', { name: `Retry with ${name}`, exact: true })).toBeVisible();
  await expect(page.getByRole('menuitem', { name: /Retry with Sonnet 5 · pinned/ })).toHaveCount(0);
  await page.getByRole('menuitem', { name: 'Retry with Fable 5.1', exact: true }).click();
  await expect.poll(() => callCount(page, 'chat_send')).toBe(2);
  const args = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}'));
  expect(args.model).toBe('fable');
  expect(args.agent).toBe('claude');
  expect(String(args.prompt)).toContain('Draft the release notes');
  // The model is the chat's now: its button says so, and the next reply's menu leaves it out.
  await expect(page.getByRole('button', { name: /^Model: Claude · fable · / })).toBeVisible();
  const tag = { clientTurnId: args.clientTurnId, sessionKey: MOCK_KEY };
  await emit(page, { type: 'runStarted', runId: 'run-model-2', recall: [], ...tag });
  await emit(page, { type: 'assistantText', text: 'A second take.', ...tag });
  await emit(page, { type: 'result', result: 'A second take.', costUsd: null, isError: false, subtype: 'success', sessionId: null, ...tag });
  await emit(page, { type: 'done', sessionId: null, ...tag });
  await page.locator('.chat-main .turn-footer').last().getByRole('button', { name: 'More actions', exact: true }).click();
  await expect(page.getByRole('menuitem', { name: 'Retry with Opus 5.5', exact: true })).toBeVisible();
  await expect(page.getByRole('menuitem', { name: 'Retry with Fable 5.1', exact: true })).toHaveCount(0);
});

test('a failed last turn offers Retry with another model, which switches the model, then sends the message', async ({ page }) => {
  await installEventBridge(page);
  await openChat(page);
  const tag = await startTurn(page, 'Draft the release notes', 'run-fail-3');
  await emit(page, { type: 'error', message: 'The model is overloaded.', ...tag });
  await emit(page, { type: 'done', sessionId: null, ...tag });
  await page.getByRole('button', { name: 'Retry with another model', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Haiku 5.5', exact: true }).click();
  await expect.poll(() => callCount(page, 'chat_send')).toBe(2);
  const args = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}'));
  expect(args.model).toBe('haiku');
  expect(args.agent).toBe('claude');
  expect(String(args.prompt)).toContain('Draft the release notes');
});

test('a finished reply’s More menu keeps Retry with another agent', async ({ page }) => {
  await installEventBridge(page);
  await openChat(page);
  await finishTurn(page, 'Draft the release notes', 'run-regen-2', 'Here are the notes.');
  await page.locator('.chat-main .turn-footer').last().getByRole('button', { name: 'More actions', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Retry with Codex', exact: true }).click();
  await expect.poll(() => callCount(page, 'chat_send')).toBe(2);
  const args = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}'));
  expect(args.agent).toBe('codex');
  expect(String(args.prompt)).toContain('Draft the release notes');
});
