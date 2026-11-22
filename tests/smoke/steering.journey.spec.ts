import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

/** Mid-turn steering (2026-10-21): while a turn runs the composer stays open.
 * Enter does what its button says (Steer now for an agent that takes
 * messages mid-turn, else Queue for after); Send options holds Queue for
 * after and Stop and send. A steer shows inline as the person's own row; a
 * held send waits as a chip with Cancel. Events come through a Tauri event
 * bridge; commands through the `?mock=1` IPC. */

const MOCK_KEY = 's-20260925-mock0001';

test('turn support survives background restore and ignores stale agent-wide refreshes', async ({ page }) => {
  await page.goto('/?mock=1');
  await expect(page.locator('.app')).toBeVisible();
  const result = await page.evaluate(async () => {
    const s = await import('/src/store/chatRuntime.ts');
    const { api } = await import('/src/api.ts');
    const old = { agent: 'codex', support: 'none' as const, note: 'Open Agent updates.' };
    const next = { ...old, support: 'native' as const, note: 'Takes the message in this turn.' };
    s.resetChatState(); s.setChatAgent('codex');
    s.setChatAgentSessionKeyPersistent('s-support-roundtrip');
    s.setChatBusy(true); s.setChatLiveRunId('r-support'); s.setChatTurnSteerSupport(old);
    s.setChatSteerSupport([next]);
    const before = s.steersNatively('codex');
    s.backgroundRunningChat(); s.resetChatState();
    s.restoreBackgroundChat('s-support-roundtrip');
    const restored = s.chatTurnSteerSupport();
    const after = s.steersNatively('codex');
    const original = api.chatSteerSupport;
    const completions: ((rows: Awaited<ReturnType<typeof original>>) => void)[] = [];
    api.chatSteerSupport = () => new Promise(resolve => completions.push(resolve));
    try {
      s.ensureChatSteerSupport(true); s.ensureChatSteerSupport(true);
      completions[1]([next]); await Promise.resolve();
      completions[0]([old]); await Promise.resolve();
      return { before, after, restored, current: s.chatSteerSupport(), turn: s.chatTurnSteerSupport() };
    } finally { api.chatSteerSupport = original; s.resetChatState(); }
  });
  expect(result.before).toBe(false);
  expect(result.after).toBe(false);
  expect(result.restored?.support).toBe('none');
  expect(result.turn?.support).toBe('none');
  expect(result.current?.[0].support).toBe('native');
});

function callCount(page: Page, command: string) {
  return page.evaluate(cmd => window.__mockApiCalls?.[cmd] ?? 0, command);
}

function lastAfter(page: Page) {
  return page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}').after ?? null);
}

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
  });
}

function emitChatEvent(page: Page, payload: Record<string, unknown>) {
  return page.evaluate(event => {
    (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void }).__emitTauriEvent('chat-event', event);
  }, payload);
}

async function openConversationHistory(page: Page) {
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), 'advanced');
  } else {
    await page.locator('.settings-index').getByRole('button', { name: 'Advanced', exact: true }).click();
  }
  await page.locator('#settings-group-this-mac > summary').click(); // History & recovery: Advanced → This Mac
  await page.getByRole('button', { name: 'Open Library, History', exact: true }).click();
  await page.locator('.library-tabs').getByRole('button', { name: 'Conversations', exact: true }).click(); // Settings PR 4 cut the Conversation history button
}

/** Open a chat, send a message and start its turn with a tool running. */
async function runningTurn(page: Page) {
  await page.goto('/?mock=1');
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await openConversationHistory(page);
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await composer.fill('Refactor the parser');
  await composer.press('Meta+Enter');
  await expect.poll(() => callCount(page, 'chat_send')).toBe(1);
  const clientTurnId = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}').clientTurnId);
  const tag = { clientTurnId, sessionKey: MOCK_KEY };
  await emitChatEvent(page, { type: 'runStarted', runId: 'run-st-1', recall: [], ...tag });
  await emitChatEvent(page, { type: 'assistantDelta', text: 'Reading the parser.', ...tag });
  await emitChatEvent(page, { type: 'toolUse', id: 'tu-1', name: 'Bash', input: { command: 'make check' }, ...tag });
  return { composer, tag };
}

test('Steer now: Enter sends the message into the running turn, shown as yours', async ({ page }, info) => {
  await installEventBridge(page);
  const { composer, tag } = await runningTurn(page);
  const hint = page.locator('.chat-busy-hint');
  // An empty box carries the hint as its placeholder, with the Stop key;
  // the line under the box shows once something is typed (P1-10).
  await expect(hint).toHaveCount(0);
  await expect(composer).toHaveAttribute('placeholder', 'Enter steers Claude at its next step · Esc stops this turn');
  await expect(page.getByRole('button', { name: 'Stop', exact: true })).toHaveAttribute('title', 'Stop (Esc)');
  await composer.fill('Also');
  // Esc stops only with an empty box (FIX-ESC), so the typed line doesn't
  // name it, and Esc over a typed message keeps the turn and the draft.
  await expect(hint).toHaveText('Enter steers Claude at its next step · Send options has Queue for after and Stop and send');
  await expect(hint).toHaveAttribute('aria-live', 'polite');
  await expect(page.getByRole('button', { name: 'Stop', exact: true })).toHaveAttribute('title', 'Stop');
  await composer.press('Escape');
  await page.waitForTimeout(300);
  expect(await callCount(page, 'chat_cancel')).toBe(0);
  await expect(composer).toHaveValue('Also');
  await expect(page.getByRole('button', { name: 'Steer now', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Stop', exact: true })).toBeVisible();

  await composer.fill('Also check the tests');
  await composer.press('Enter');
  await expect.poll(() => callCount(page, 'chat_steer')).toBe(1);
  const sent = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastSteer') ?? '{}'));
  expect(sent).toMatchObject({ sessionKey: MOCK_KEY, text: 'Also check the tests' });
  await expect(composer).toHaveValue('');

  const row = page.locator('.chat-steer-row');
  await expect(row).toHaveCount(1);
  await expect(row).toContainText('You');
  await expect(row).toContainText('Also check the tests');
  await expect(row).toContainText('Waiting for the agent’s next step');

  // The agent takes it at its next step; its later words go below.
  await emitChatEvent(page, { type: 'toolResult', id: 'tu-1', content: 'ok', isError: false, ...tag });
  await emitChatEvent(page, { type: 'steer', sessionKey: MOCK_KEY, steer: { id: sent.clientId, text: 'Also check the tests', at: new Date().toISOString(), afterTools: 1, state: 'delivered' }, ...tag });
  await expect(row).toContainText('Taken into this turn');
  await expect(row).toHaveCount(1);
  await page.screenshot({ path: info.outputPath('steer-row.png') });
  await emitChatEvent(page, { type: 'assistantText', text: 'Checked the tests too.', ...tag });
  const order = await page.locator('.chat-main .turn').evaluateAll(turns => turns.map(t => (t.textContent ?? '').trim()));
  const steerAt = order.findIndex(t => t.includes('Also check the tests'));
  const answerAt = order.findIndex(t => t.includes('Checked the tests too.'));
  expect(steerAt).toBeGreaterThan(-1);
  expect(answerAt).toBeGreaterThan(steerAt);
  await emitChatEvent(page, { type: 'done', sessionId: 'native-1', ...tag });
  await expect(page.locator('.chat-busy-hint')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeVisible();
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});

test('Send options: Queue for after and Stop and send wait as chips, with Cancel, by keyboard', async ({ page }, info) => {
  await installEventBridge(page);
  await page.addInitScript(() => localStorage.setItem('mockChatHold', '1'));
  const { composer, tag } = await runningTurn(page);

  await composer.fill('Then update the docs');
  // Keyboard only: focus Send options, open it, move to Queue for after.
  const options = page.getByRole('button', { name: 'Send options' });
  await options.focus();
  await page.keyboard.press('Enter');
  const queue = page.getByRole('menuitem', { name: /^Queue for after:/ });
  await expect(queue).toBeVisible();
  // Opening by keyboard focuses the first choice; the arrow moves on.
  await expect(page.getByRole('menuitem', { name: /^Steer now:/ })).toBeFocused();
  await page.keyboard.press('ArrowDown');
  await expect(queue).toBeFocused();
  await page.keyboard.press('Enter');
  await expect.poll(() => lastAfter(page)).toBe('queue');
  const chips = page.locator('.chat-held-chip');
  await expect(chips).toHaveCount(1);
  await expect(chips.first()).toContainText('Queued for after this turn');
  await expect(chips.first()).toContainText('Then update the docs');

  // Cancel puts the words back in the box.
  await chips.first().getByRole('button', { name: /^Cancel/ }).click();
  await expect.poll(() => callCount(page, 'chat_cancel_held')).toBe(1);
  await expect(chips).toHaveCount(0);
  await expect(composer).toHaveValue('Then update the docs');

  // Stop and send from the menu.
  await composer.fill('Use the v2 parser instead');
  await options.click();
  await page.getByRole('menuitem', { name: /^Stop and send:/ }).click();
  await expect.poll(() => lastAfter(page)).toBe('stop');
  await expect(chips).toHaveCount(1);
  await expect(chips.first()).toContainText('Stopping, then sending');
  await options.click();
  await page.screenshot({ path: info.outputPath('send-options.png') });
  await page.keyboard.press('Escape');

  // The turn stops; the held send then goes as the next turn.
  await emitChatEvent(page, { type: 'cancelled', stop: 'verified', ...tag });
  await emitChatEvent(page, { type: 'done', sessionId: 'native-1', ...tag });
  await expect(chips).toHaveCount(1);
  const heldId = await page.evaluate(() => localStorage.getItem('mockLastHeldId'));
  await emitChatEvent(page, { type: 'heldSent', heldId, after: 'stop', sessionKey: MOCK_KEY, queueId: 2, clientTurnId: 'ct-held-1', conversationKey: MOCK_KEY });
  await expect(chips).toHaveCount(0);
  await expect(page.locator('.chat-main .turn.user').last()).toContainText('Use the v2 parser instead');
  // Still queued, it can't be steered yet (P1-30): Enter queues until it starts.
  await expect(page.getByRole('button', { name: 'Queue for after', exact: true })).toBeVisible();
  await emitChatEvent(page, { type: 'runStarted', runId: 'run-st-2', recall: [], clientTurnId: 'ct-held-1', sessionKey: MOCK_KEY });
  await expect(page.getByRole('button', { name: 'Steer now', exact: true })).toBeVisible();
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});

test('an agent without a true steer queues on Enter and says why', async ({ page }) => {
  await installEventBridge(page);
  await page.addInitScript(() => { localStorage.setItem('mockSteerNative', 'codex'); localStorage.setItem('mockChatHold', '1'); });
  const { composer } = await runningTurn(page);
  await expect(composer).toHaveAttribute('placeholder', 'Enter queues it for after this turn · Esc stops this turn');
  await composer.fill('Also');
  await expect(page.locator('.chat-busy-hint')).toHaveText("Enter queues it for after this turn (Claude can't take a message mid-turn) · Send options has Queue for after and Stop and send");
  await expect(page.getByRole('button', { name: 'Queue for after', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Send options' }).click();
  await expect(page.getByRole('menuitem', { name: /can't take a message mid-turn/ })).toBeVisible();
  await page.keyboard.press('Escape');
  await composer.fill('After this, run the linter');
  await composer.press('Enter');
  await expect.poll(() => lastAfter(page)).toBe('queue');
  expect(await callCount(page, 'chat_steer')).toBe(0);
  await expect(page.locator('.chat-held-chip')).toHaveCount(1);
});

test('a steer refused because the turn was ending goes as the next message, and the chat says so', async ({ page }) => {
  await installEventBridge(page);
  await page.addInitScript(() => { localStorage.setItem('mockSteerRefuse', 'STEER_ENDING'); localStorage.setItem('mockChatHold', '1'); });
  const { composer } = await runningTurn(page);
  await composer.fill('One more thing');
  await composer.press('Enter');
  await expect.poll(() => callCount(page, 'chat_steer')).toBe(1);
  await expect(page.locator('.chat-main')).toContainText('The turn was finishing, so your message goes as the next one instead.');
  await expect.poll(() => lastAfter(page)).toBe('queue');
  await expect(page.locator('.chat-held-chip')).toContainText('One more thing');
});

test('P1-30: a refusal says what happened in words, never its code', async ({ page }) => {
  await installEventBridge(page);
  await page.addInitScript(() => { localStorage.setItem('mockSteerRefuse', 'STEER_NOT_RUNNING'); localStorage.setItem('mockChatHold', '1'); });
  const { composer } = await runningTurn(page);
  await composer.fill('One more thing');
  await composer.press('Enter');
  await expect.poll(() => callCount(page, 'chat_steer')).toBe(1);
  await expect(page.locator('.chat-main')).toContainText("hasn't started");
  await expect(page.locator('.chat-main')).not.toContainText('was finishing');
  await expect.poll(() => lastAfter(page)).toBe('queue');
  // One Hollow refuses outright (21st steer, too long) comes back in words.
  await page.evaluate(() => localStorage.setItem('mockSteerRefuse', 'STEER_INVALID'));
  await composer.fill('Too many');
  await composer.press('Enter');
  await expect.poll(() => callCount(page, 'chat_steer')).toBe(2);
  await expect(composer).toHaveValue('Too many');
  await expect(page.locator('.chat-main')).toContainText('20 messages');
  await expect(page.locator('body')).not.toContainText('STEER_');
});

test('P1-30: a draft over 3,000 characters queues and says why; /model while busy neither steers nor sends', async ({ page }) => {
  await installEventBridge(page);
  await page.addInitScript(() => { localStorage.setItem('mockChatHold', '1'); });
  const { composer } = await runningTurn(page);
  await composer.fill('x'.repeat(3_001));
  await expect(page.getByRole('button', { name: 'Queue for after', exact: true })).toBeVisible();
  await expect(page.locator('.chat-busy-hint')).toContainText('3,000 characters');
  await composer.fill('/model opus');
  await page.getByRole('button', { name: 'Steer now', exact: true }).click();
  await page.waitForTimeout(300);
  expect(await callCount(page, 'chat_steer')).toBe(0);
  expect(await callCount(page, 'chat_send')).toBe(1);
  await expect(composer).toHaveValue('/model opus');
  await expect(page.locator('.chat-main')).toContainText('/model changes the next turn');
});

test('P1-30: a steer the agent refused shows its reason, with Put back in the box and Send as next message (once)', async ({ page }) => {
  await installEventBridge(page);
  await page.addInitScript(() => { localStorage.setItem('mockChatHold', '1'); });
  const { composer, tag } = await runningTurn(page);
  await composer.fill('Also lint it');
  await composer.press('Enter');
  await expect.poll(() => callCount(page, 'chat_steer')).toBe(1);
  const sent = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastSteer') ?? '{}'));
  await emitChatEvent(page, { type: 'steer', sessionKey: MOCK_KEY, steer: { id: sent.clientId, text: 'Also lint it', at: new Date().toISOString(), afterTools: 1, state: 'notDelivered', reason: 'no active turn to steer' }, ...tag });
  const row = page.locator('.chat-steer-row').first();
  await expect(row).toContainText('Not taken: the agent said “no active turn to steer”');
  // While the turn still runs, Send as next message holds it for after (review).
  await composer.fill('And the docs');
  await composer.press('Enter');
  await expect.poll(() => callCount(page, 'chat_steer')).toBe(2);
  const second = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastSteer') ?? '{}'));
  await emitChatEvent(page, { type: 'steer', sessionKey: MOCK_KEY, steer: { id: second.clientId, text: 'And the docs', at: new Date().toISOString(), afterTools: 1, state: 'notDelivered' }, ...tag });
  const busyRow = page.locator('.chat-steer-row').nth(1);
  await expect(busyRow).toContainText('Not taken: the turn ended first');
  await busyRow.getByRole('button', { name: 'Send as next message', exact: true }).click();
  await expect.poll(() => lastAfter(page)).toBe('queue');
  await expect(page.locator('.chat-held-chip')).toContainText('And the docs');
  await expect(busyRow.getByRole('button', { name: 'Send as next message', exact: true })).toHaveCount(0);
  await emitChatEvent(page, { type: 'done', sessionId: 'native-1', ...tag });
  await row.getByRole('button', { name: 'Put back in the box', exact: true }).click();
  await expect(composer).toHaveValue('Also lint it');
  await composer.fill('');
  const sends = await callCount(page, 'chat_send');
  await row.getByRole('button', { name: 'Send as next message', exact: true }).click();
  await expect.poll(() => callCount(page, 'chat_send')).toBe(sends + 1);
  expect(await page.evaluate(() => localStorage.getItem('mockLastChatPrompt'))).toMatch(/Also lint it$/);
  await expect(row.getByRole('button', { name: 'Send as next message', exact: true })).toHaveCount(0);
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});
