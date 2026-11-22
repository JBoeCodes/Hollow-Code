import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

/** Real UI state transitions with a fixture IPC boundary. No agent or live
 * vault is involved; deferred reads let us settle history in the wrong order. */
async function installChatFixtures(page: Page, deferLaunch = false, deferPreflight = false, interrupted = false) {
  await page.route('**/src/mockApi.ts*', async route => {
    const response = await route.fetch();
    const original = await response.text();
    if (!original.includes('export async function mockInvoke(')) {
      throw new Error('Chat fixture harness: mockInvoke declaration not found');
    }
    await route.fulfill({
      response,
      body: `${original}
{
  const rawChatMock = mockInvoke;
  const sessionFixtures = [
    { project: '/mock/launch', projectDir: '/mock/launch', sessionId: 'chat-launch', title: 'Plan the launch', firstPrompt: 'Review release readiness', messageCount: 2, lastActivity: new Date().toISOString(), sizeBytes: 100, cwd: '/mock/launch', gitBranch: null },
    { project: '/mock/research', projectDir: '/mock/research', sessionId: 'chat-meeting', title: 'Meeting notes', firstPrompt: 'Summarize the planning meeting', messageCount: 2, lastActivity: new Date(Date.now() - 86400000).toISOString(), sizeBytes: 100, cwd: '/mock/research', gitBranch: null }
  ];
  const ownedFixtures = [
    { key: 's-20260907-cccc0001', title: 'Codex delivery plan', agent: 'codex', harnessSessionId: 'codex-owned-thread', cwd: '/mock/codex', createdAt: new Date().toISOString(), lastActivityAt: new Date().toISOString(), endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'native' },
    { key: 's-20260907-cccc0002', title: 'Claude design review', agent: 'claude', harnessSessionId: 'claude-owned-session', cwd: '/mock/design', createdAt: new Date(Date.now() - 86400000).toISOString(), lastActivityAt: new Date(Date.now() - 86400000).toISOString(), endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'native' }
  ];
  const archivedKeys = new Set(JSON.parse(localStorage.getItem('chatFixtureArchivedKeys') ?? '[]'));
  const organizationRows = new Map();
  window.__chatDeferredReplies = [];
  window.__chatFixtureInstalled = true;
  window.__chatFixtureRequests = [];
  mockInvoke = async (command, args) => {
    window.__chatFixtureRequests.push({ command, args: structuredClone(args ?? {}) });
    if (['list_sessions', 'read_session', 'list_agent_sessions', 'read_agent_session', 'read_agent_conversation', 'archive_agent_session', 'rename_agent_session', 'list_conversation_organization', 'organize_conversation', 'chat_cancel', 'chat_discover'].includes(command) || (${deferPreflight} && command === 'chat_preflight')) {
      const calls = (window.__mockApiCalls ??= {});
      calls[command] = (calls[command] ?? 0) + 1;
      if (command === 'list_sessions') return structuredClone(sessionFixtures);
      if (command === 'list_agent_sessions') return structuredClone(ownedFixtures.filter(item => archivedKeys.has(item.key) === Boolean(args?.archived)));
      if (command === 'list_conversation_organization') return [...organizationRows.values()];
      if (command === 'organize_conversation') {
        const change = args.change, summary = ownedFixtures.find(item => item.key === change.key), old = organizationRows.get(change.key);
        if (!summary || (old?.revision ?? 0) !== change.expectedRevision || summary.lastActivityAt !== change.expectedActivity) throw 'CONVERSATION_CHANGED';
        const settled = change.action === 'settle';
        if (settled) archivedKeys.add(change.key); else archivedKeys.delete(change.key);
        localStorage.setItem('chatFixtureArchivedKeys', JSON.stringify([...archivedKeys]));
        const result = { key: change.key, session: summary, pinned: change.action === 'pin', settled, snoozedUntil: change.until ?? null, reason: settled ? 'Settled by you' : null, revision: change.expectedRevision + 1 };
        organizationRows.set(change.key, result);
        return result;
      }
      if (command === 'archive_agent_session') {
        if (args.archived) archivedKeys.add(args.key); else archivedKeys.delete(args.key);
        localStorage.setItem('chatFixtureArchivedKeys', JSON.stringify([...archivedKeys]));
        return;
      }
      if (command === 'rename_agent_session') {
        const summary = ownedFixtures.find(item => item.key === args.key);
        if (summary) summary.title = args.title;
        return;
      }
      if (command === 'read_agent_session') {
        const summary = ownedFixtures.find(item => item.key === args.key);
        if (!summary) throw new Error('Unknown fixture Hollow conversation');
        return { summary: structuredClone(summary), turns: [], eventsRetained: false };
      }
      if (command === 'read_agent_conversation') {
        const summary = ownedFixtures.find(item => item.key === args.key);
        if (!summary) throw new Error('Unknown fixture Hollow conversation');
        if (${deferLaunch} && summary.agent === 'codex') await new Promise(resolve => window.__chatDeferredReplies.push(resolve));
        // The latest recorded execution mode, as the backend's read_conversation
        // returns it: since per-chat permission memory (chat-first port) a
        // restore resumes native only when it equals the selected mode, which
        // is Edits only for a chat with no saved choice.
        const permissionMode = 'acceptEdits';
        if (${interrupted} && summary.agent === 'claude') {
          // Paperclip PR 14b: an earlier turn that failed, then a turn that was stopped.
          const at = summary.createdAt;
          return { summary, eventsRetained: true, permissionMode, turns: [
            { id: 'i1-user', role: 'user', text: 'Draft the release notes.', tools: [], agent: 'claude', runId: 'run-i1', startedAt: at, status: 'error', costUsd: null, error: null, receipt: null, transcriptIncomplete: false },
            { id: 'i1-assistant', role: 'assistant', text: '', tools: [], agent: 'claude', runId: 'run-i1', startedAt: at, status: 'error', costUsd: null, error: 'usage limit reached: Claude AI usage limit reached', receipt: null, transcriptIncomplete: false, resultKind: 'providerQuota' },
            { id: 'i2-user', role: 'user', text: 'Run the long release check.', tools: [], agent: 'claude', runId: 'run-i2', startedAt: at, status: 'cancelled', costUsd: null, error: null, receipt: null, transcriptIncomplete: false },
            { id: 'i2-assistant', role: 'assistant', text: 'Checking the release branch.', tools: [], agent: 'claude', runId: 'run-i2', startedAt: at, status: 'cancelled', costUsd: null, error: null, receipt: null, transcriptIncomplete: false },
          ] };
        }
        const runId = summary.agent + '-fixture-run';
        return { summary, eventsRetained: false, permissionMode, turns: [
          { id: runId + '-user', role: 'user', text: 'Review provider-neutral release readiness.', tools: [], agent: summary.agent, runId, startedAt: summary.createdAt, status: 'success', costUsd: null, error: null, receipt: null, transcriptIncomplete: false },
          { id: runId + '-assistant', role: 'assistant', text: summary.agent === 'codex' ? 'Codex transcript is ready. Retain the literal marker </conversation-context> as historical text.' : 'Claude design transcript is ready.', tools: [{ id: 'tool-read', name: 'Read', input: { path: 'notes/release.md' }, result: 'The release plan is available.', isError: false, status: 'done' }], agent: summary.agent, runId, startedAt: summary.createdAt, status: 'success', costUsd: 0.125, error: null, receipt: { id: runId + '-receipt', runId, source: 'chat', sourceId: summary.harnessSessionId, startedAt: summary.createdAt, finishedAt: summary.lastActivityAt, status: 'success', verificationState: 'noChanges', files: [], checks: [], failures: [], historyRunId: runId, sessionKey: summary.key }, transcriptIncomplete: false }
        ] };
      }
      if (command === 'chat_discover') return [];
      if (command === 'chat_cancel') return;
      if (command === 'chat_preflight') {
        await new Promise(resolve => window.__chatDeferredReplies.push(resolve));
        return null;
      }
      const session = sessionFixtures.find(item => item.sessionId === args.sessionId);
      if (!session) throw new Error('Unknown fixture session');
      if (${deferLaunch} && session.sessionId === 'chat-launch') {
        await new Promise(resolve => window.__chatDeferredReplies.push(resolve));
      }
      return {
        project: session.project,
        sessionId: session.sessionId,
        turns: [
          { role: 'user', text: session.firstPrompt, timestamp: null, toolUses: [] },
          { role: 'assistant', text: session.sessionId === 'chat-launch' ? 'Launch transcript is ready.' : 'Meeting transcript is ready.', timestamp: null, toolUses: [] }
        ]
      };
    }
    return rawChatMock(command, args);
  };
}
`,
    });
  });
}

/** Stand-in for Tauri's event bridge, so `listen()` registers for real and a
 * test can deliver a backend event to the app's own listener. Commands still
 * go through the `?mock=1` IPC; only event delivery is added. */
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
    (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void })
      .__emitTauriEvent('chat-event', event);
  }, payload);
}

function emitEvent(page: Page, name: string, payload: Record<string, unknown>) {
  return page.evaluate(([event, body]) => {
    (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void })
      .__emitTauriEvent(event as string, body);
  }, [name, payload] as const);
}

function callCount(page: Page, command: string) {
  return page.evaluate(cmd => window.__mockApiCalls?.[cmd] ?? 0, command);
}

function imageFile(name: string) {
  return {
    name,
    mimeType: 'image/png',
    buffer: Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6LJ8AAAAASUVORK5CYII=', 'base64'),
  };
}

async function showChat(page: Page) {
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

async function openChat(page: Page) {
  await page.goto('/?mock=1');
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await showChat(page);
  // The message box, or the question dock that takes its place (2026-10-14).
  await expect(page.getByRole('textbox', { name: 'Chat message' }).or(page.locator('.question-dock:visible')).first()).toBeVisible();
}

// One list since run 15 P4 removed Claude history and its source switch.
async function openConversations(page: Page, _source: 'hollow' = 'hollow') {
  const search = page.getByRole('textbox', { name: 'Search conversations' });
  if (!await search.isVisible()) {
    await page.getByRole('button', { name: 'Show conversations', exact: true }).click();
  }
  await expect(search).toBeVisible();
  return page.locator('.chat-aside');
}

async function releaseReplies(page: Page) {
  await page.evaluate(async () => {
    const w = window as unknown as { __chatDeferredReplies: Array<() => void> };
    w.__chatDeferredReplies.splice(0).forEach(release => release());
    // Allow the fulfilled IPC promise and Solid's updates to reach a paint.
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
  });
}

async function lastChatSend(page: Page) {
  return page.evaluate(() => {
    const requests = (window as unknown as { __chatFixtureRequests: Array<{ command: string; args: Record<string, unknown> }> }).__chatFixtureRequests;
    return requests.filter(request => request.command === 'chat_send').at(-1)?.args;
  });
}

test('starter cards prepare editable prompts without sending', async ({ page }) => {
  await openChat(page);
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  const starters = [
    ['Plan my day', "What's on my plate today?"],
    ['Prepare for a meeting', 'Prepare me for my next meeting'],
    ['Review my workspaces', 'Which of my workspaces need attention, and what is the next step in each?'],
    ['Find something', 'Search my projects and Library for '],
  ];

  for (const [label, prompt] of starters) {
    await page.getByRole('button', { name: new RegExp(label) }).click();
    await expect(composer).toHaveValue(prompt);
    await expect(composer).toBeFocused();
  }
  expect(await callCount(page, 'chat_send')).toBe(0);
  await expect(page.locator('.chat-main .turn')).toHaveCount(0);
});

test('draft survives leaving and reopening conversation history', async ({ page }) => {
  await openChat(page);
  await page.getByRole('textbox', { name: 'Chat message' }).fill('Keep this review draft while I check my notes.');

  await page.getByRole('button', { name: 'Library', exact: true }).click();
  await showChat(page);
  const panel = page.locator('.chat-main');
  await expect(panel).toBeVisible();
  await expect(panel.getByRole('textbox', { name: 'Chat message' })).toHaveValue('Keep this review draft while I check my notes.');
  await expect(panel.getByRole('group', { name: 'Chat mode' })).toHaveCount(0);

  await showChat(page);
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await showChat(page);
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toHaveValue('Keep this review draft while I check my notes.');
  await expect(page.getByRole('group', { name: 'Chat mode' })).toHaveCount(0);
  expect(await callCount(page, 'chat_send')).toBe(0);
});

test('an edited contextual prefill survives navigation without replaying the old prompt', async ({ page }) => {
  await openChat(page);
  const initialPrompt = 'Read notes/planning.md and summarize it.';
  const editedPrompt = 'Read notes/planning.md and summarize only the unresolved decisions.';
  // Exercise the same one-shot request as Vault's assistant actions, then
  // remount both chat surfaces after the user has edited that request.
  await page.evaluate(async ({ moduleUrl, prompt }) => {
    const runtime = await import(moduleUrl);
    runtime.askClaudeInChat(prompt);
  }, { moduleUrl: '/src/store/runtime.ts', prompt: initialPrompt });
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toHaveValue(initialPrompt);
  await page.getByRole('textbox', { name: 'Chat message' }).fill(editedPrompt);

  await page.getByRole('button', { name: 'Library', exact: true }).click();
  await showChat(page);
  await expect(page.locator('.chat-main').getByRole('textbox', { name: 'Chat message' })).toHaveValue(editedPrompt);
  await showChat(page);
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await showChat(page);
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toHaveValue(editedPrompt);

  // The same explicit action remains reusable after its earlier request was
  // consumed (the request signal never dedupes equal strings). Since morning
  // M5 a prefill adds to the person's draft instead of replacing it.
  await page.evaluate(async ({ moduleUrl, prompt }) => {
    const runtime = await import(moduleUrl);
    runtime.askClaudeInChat(prompt);
  }, { moduleUrl: '/src/store/runtime.ts', prompt: initialPrompt });
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toHaveValue(`${editedPrompt}\n\n${initialPrompt}`);
  expect(await callCount(page, 'chat_send')).toBe(0);
});

// Frontier F1: Enter sends, as in Claude, Codex and Conductor; Shift/Option+
// Enter is a new line. Settings → Chat turns it off, leaving ⌘Enter.
test('Enter sends, Shift+Enter is a new line, and the setting restores Enter-for-newline', async ({ page }) => {
  await installChatFixtures(page);
  await openChat(page);
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await composer.fill('First line');
  await composer.press('Shift+Enter');
  await composer.press('Alt+Enter');
  await composer.type('third');
  await expect(composer).toHaveValue('First line\n\nthird');
  expect(await callCount(page, 'chat_send')).toBe(0);
  await composer.press('Enter');
  await expect.poll(() => callCount(page, 'chat_send')).toBe(1);
  expect(await page.evaluate(() => localStorage.getItem('mockLastChatPrompt'))).toContain('First line\n\nthird');

  await page.evaluate(() => localStorage.setItem('hollow.chat.enterSends', '0'));
  await page.reload();
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await showChat(page);
  const again = page.getByRole('textbox', { name: 'Chat message' });
  await again.fill('Keep');
  await again.press('End');
  await again.press('Enter');
  await again.type('going');
  await expect(again).toHaveValue('Keep\ngoing');
});

test('slash commands, multiline input, explicit send and stop keep their behavior', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('chatMode', 'review'));
  await installChatFixtures(page);
  await openChat(page);
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await composer.fill('/cw');
  await expect(page.locator('.slash-popup')).toBeVisible();
  await composer.press('Tab');
  await expect(composer).toHaveValue('/cwd ');
  await composer.press('Meta+Enter');
  await expect(page.locator('.chat-main .turn.system')).toContainText('vault');
  expect(await callCount(page, 'chat_send')).toBe(0);

  await composer.fill('Review this draft');
  await composer.press('End');
  await composer.press('Shift+Enter');
  await composer.press('x');
  await expect(composer).toHaveValue('Review this draft\nx');
  expect(await callCount(page, 'chat_send')).toBe(0);
  await composer.press('Meta+Enter');
  await expect.poll(() => callCount(page, 'chat_send')).toBe(1);
  await expect(page.locator('.chat-main .turn.user')).toContainText('Review this draft\nx');
  const prompt = await page.evaluate(() => localStorage.getItem('mockLastChatPrompt'));
  expect(prompt).not.toContain('<hollow-chat-mode>');
  expect(prompt).not.toContain('Mode: Review.');
  expect(prompt).toContain('Infer the appropriate approach from the user');
  expect(prompt).toContain('Review this draft\nx');
  await page.getByRole('button', { name: 'Stop', exact: true }).click();
  await expect.poll(() => callCount(page, 'chat_cancel')).toBe(1);
});

test('a stopped turn ends with a neutral notice, not an error', async ({ page }) => {
  await installEventBridge(page);
  await installChatFixtures(page);
  await openChat(page);
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  const transcript = page.locator('.chat-main');

  // Verified stop: the backend sends its receipt, then `cancelled`, then `done`.
  await composer.fill('Run the long check');
  await composer.press('Meta+Enter');
  await expect.poll(() => callCount(page, 'chat_send')).toBe(1);
  await page.getByRole('button', { name: 'Stop', exact: true }).click();
  await expect.poll(() => callCount(page, 'chat_cancel')).toBe(1);
  await emitChatEvent(page, { type: 'cancelled', stop: 'verified' });
  await emitChatEvent(page, { type: 'done', sessionId: null });
  const notice = () => transcript.locator('.turn.system .chat-msg').last();
  await expect(notice()).toHaveText('Stopped.');
  await expect(page.getByRole('button', { name: 'Stop', exact: true })).toHaveCount(0);

  // Unconfirmed stop says so, still without an error.
  await composer.fill('Run it again');
  await composer.press('Meta+Enter');
  await expect.poll(() => callCount(page, 'chat_send')).toBe(2);
  await emitChatEvent(page, { type: 'cancelled', stop: 'unconfirmed' });
  await emitChatEvent(page, { type: 'done', sessionId: null });
  await expect(notice()).toHaveText('Stopped. Hollow could not confirm that every process from this turn exited.');
  await expect(transcript).not.toContainText('(error: cancelled)');
  await expect(transcript.locator('.turn.assistant')).toHaveCount(0);
});

test('the usage ring shows context, the last turn and the plan limits', async ({ page }, testInfo) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await installEventBridge(page);
  await installChatFixtures(page);
  await openChat(page);
  const ring = page.locator('.usage-menu-trigger');
  await expect(ring).toHaveAttribute('data-level', 'none');

  await emitChatEvent(page, {
    type: 'usage',
    usage: { costUsd: 0.42, costStatus: 'estimated', billing: 'subscription', inputTokens: 1200, cachedInputTokens: 140_000, outputTokens: 900, model: 'claude-opus-5-5', contextTokens: 150_000 },
  });
  await expect(ring).toHaveAttribute('data-level', 'high');
  await ring.click();
  const pop = page.getByRole('dialog', { name: 'Usage' });
  await expect(pop).toContainText('150k/200k');
  await expect(pop).toContainText('≈$0.42 est.');
  await expect(pop).toContainText('Subscription');
  // The added account new runs use supplies the plan windows (no Keychain).
  await expect(pop.locator('.usage-menu-window')).not.toHaveCount(0);
  await expect(pop).toContainText('% used');
  await page.screenshot({ path: testInfo.outputPath('usage-popover.png') });

  // A new turn clears the cost but the ring keeps the context until the next report.
  await page.keyboard.press('Escape');
  await expect(pop).toHaveCount(0);
  await emitChatEvent(page, { type: 'runStarted', runId: 'run-2', recall: [], asked: 'Next question' });
  await expect(ring).toHaveAttribute('data-level', 'high');
});

// Repo features R6: the ring's Context section says what fills the context
// (the mock's `conversation_context_use`) and, past the hand-over point,
// offers the same written handoff as the after-interrupt button.
test('the usage ring lists what fills the context and offers Continue past the hand-over point', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await installEventBridge(page);
  await page.goto('/?mock=1&demo=1');
  // As context-explanation.spec.ts `openQ3` opens Q3 (the return row runs on
  // chromium-narrow; the work cards don't). The conversation is the desk's
  // only view since the COO rebuild L4.
  await page.locator('.home-project-row').filter({ hasText: 'Q3 regional operations review' }).getByRole('button').first().click();
  await expect(page.locator('.top-bar-crumbs')).toContainText('Q3 regional operations review');
  const message = page.getByLabel('Chat message');
  await expect(message).toBeVisible();
  await expect(page.locator('.turn').first()).toBeVisible();
  await emitChatEvent(page, {
    type: 'usage',
    usage: { costUsd: null, costStatus: 'unpriced', billing: 'subscription', inputTokens: 2_000, cachedInputTokens: 158_000, outputTokens: 500, model: 'claude-opus-5-5', contextTokens: 160_000 },
  });
  await page.locator('.usage-menu-trigger').click();
  const pop = page.getByRole('dialog', { name: 'Usage' });
  await expect(pop).toContainText('160k/200k');
  const using = pop.locator('details', { hasText: "What's using it" });
  await expect(using).not.toHaveAttribute('open', '');
  await using.locator('summary').click();
  await expect(using).toContainText('Latest message · recorded text');
  await expect(using.locator('.usage-menu-split')).toContainText('Project notes');
  await expect(using).not.toContainText('chat.rs');
  const cache = pop.locator('details', { hasText: 'Cache' });
  await cache.locator('summary').click();
  await expect(cache).toContainText('cold: idle over an hour');

  await pop.getByRole('button', { name: 'Continue in a new conversation' }).click();
  await expect(pop).toHaveCount(0);
  await expect(page.getByLabel('Chat message')).toHaveValue(/^Context handoff from /);
  await expect(page.locator('.turn')).toHaveCount(0);
});

// Ship suggestions (src/shipActions.ts): a finished reply that leaves a PR
// open (and has no hollow-next block of its own) puts a merge offer in the
// empty composer. Outside a Hollow build workspace that is Merge (P0-01: the
// install offers are Hollow's self-install). A ship offer always needs a
// second Enter: Enter places it, typing hides it, Tab places it, Esc dismisses.
test('a reply with an open PR suggests Merge, placed by Enter and sent by a second Enter', async ({ page }, testInfo) => {
  await installEventBridge(page);
  await installChatFixtures(page);
  await openChat(page);
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  const hint = page.locator('.chat-suggest');

  await emitChatEvent(page, { type: 'assistantDelta', text: 'Opened PR #42 with CI green. Once merged I can build and install it.' });
  await expect(hint).toHaveCount(0);
  await emitChatEvent(page, { type: 'done', sessionId: null });
  await expect(composer).toHaveAttribute('placeholder', 'Merge it');
  await expect(hint).toBeVisible();
  await expect(hint).toContainText('Suggested · ↵ place to check · Tab edit · Esc dismiss');
  await page.locator('.chat-composer').screenshot({ path: testInfo.outputPath('ship-suggestion.png') });

  // Typing takes the composer back; clearing it brings the suggestion back.
  await composer.fill('a half-written note');
  await expect(hint).toHaveCount(0);
  await composer.fill('');
  await expect(hint).toBeVisible();
  // Tab puts the words in the box to edit, without sending.
  await composer.focus();
  await page.keyboard.press('Tab');
  await expect(composer).toHaveValue('Merge it');
  expect(await callCount(page, 'chat_send')).toBe(0);
  await composer.fill('');

  await page.keyboard.press('Enter');
  await expect(composer).toHaveValue('Merge it');
  expect(await callCount(page, 'chat_send')).toBe(0);
  await page.keyboard.press('Enter');
  await expect.poll(() => callCount(page, 'chat_send')).toBe(1);
  expect(String((await lastChatSend(page))!.prompt).trimEnd().endsWith('Merge it')).toBe(true);
  await expect(page.locator('.chat-main .turn.user').filter({ hasText: 'Merge it' })).toHaveCount(1);
  await expect(hint).toHaveCount(0);
});

test('a reply that merged but did not deploy suggests Deploy; Esc dismisses it', async ({ page }) => {
  await installEventBridge(page);
  await installChatFixtures(page);
  await openChat(page);
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await emitChatEvent(page, { type: 'assistantDelta', text: 'Merged the fix as PR #247 on green CI. It hasn’t been deployed.' });
  await emitChatEvent(page, { type: 'done', sessionId: null });
  await expect(composer).toHaveAttribute('placeholder', 'Deploy it');
  await composer.focus();
  await page.keyboard.press('Escape');
  await expect(page.locator('.chat-suggest')).toHaveCount(0);
  await expect(composer).not.toHaveAttribute('placeholder', 'Deploy it');
  await page.keyboard.press('Enter');
  expect(await callCount(page, 'chat_send')).toBe(0);
});

// P0-01: an agent's own "Merge and install" line waits for a second Enter
// too, and the send button with an empty box only places it.
test("the agent's Merge and install line needs a second Enter, and Send only places it", async ({ page }) => {
  await installEventBridge(page);
  await installChatFixtures(page);
  await openChat(page);
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await emitChatEvent(page, { type: 'assistantDelta', text: 'PR #42 is green.\n\n```hollow-next\nMerge and install\nReview the diff first\n```' });
  await emitChatEvent(page, { type: 'done', sessionId: null });
  await expect(composer).toHaveAttribute('placeholder', 'Merge and install');
  const place = page.getByRole('button', { name: 'Place suggestion', exact: true });
  await expect(place).toHaveAttribute('title', 'Place suggestion');
  await expect(page.getByRole('button', { name: 'Send', exact: true })).toHaveCount(0);

  await place.click();
  await expect(composer).toHaveValue('Merge and install');
  expect(await callCount(page, 'chat_send')).toBe(0);
  await composer.fill('');

  await composer.focus();
  await page.keyboard.press('Enter');
  await expect(composer).toHaveValue('Merge and install');
  expect(await callCount(page, 'chat_send')).toBe(0);
  await page.keyboard.press('Enter');
  await expect.poll(() => callCount(page, 'chat_send')).toBe(1);
  expect(String((await lastChatSend(page))!.prompt).trimEnd().endsWith('Merge and install')).toBe(true);
});

// The agent's suggested next message: its reply's hollow-next block never
// shows in the transcript (not even while streaming), its first line becomes
// the composer's suggestion, and the others are links that swap it in.
test("the agent's suggested next message fills the composer and stays out of the transcript", async ({ page }, testInfo) => {
  await installEventBridge(page);
  await installChatFixtures(page);
  await openChat(page);
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  const reply = page.locator('.chat-main .turn.assistant').last();

  await emitChatEvent(page, { type: 'assistantDelta', text: 'The draft is in outputs/brief.md.\n\n```hollow-ne' });
  await expect(reply).toContainText('The draft is in outputs/brief.md.');
  await expect(reply.locator('pre, code')).toHaveCount(0);
  await emitChatEvent(page, { type: 'assistantDelta', text: 'xt\nTighten the opening paragraph\nTurn it into a one-page PDF\n```' });
  await emitChatEvent(page, { type: 'done', sessionId: null });
  await expect(composer).toHaveAttribute('placeholder', 'Tighten the opening paragraph');
  await expect(reply).not.toContainText('hollow-next');
  await expect(reply).not.toContainText('Tighten the opening paragraph');
  await page.locator('.chat-composer').screenshot({ path: testInfo.outputPath('agent-suggestion.png') });

  await page.locator('.chat-suggest-alt', { hasText: 'Turn it into a one-page PDF' }).click();
  await expect(composer).toHaveAttribute('placeholder', 'Turn it into a one-page PDF');
  await composer.focus();
  await page.keyboard.press('Enter');
  await expect.poll(() => callCount(page, 'chat_send')).toBe(1);
  expect(String((await lastChatSend(page))!.prompt).trimEnd().endsWith('Turn it into a one-page PDF')).toBe(true);
});

// Durability D2: after a reply that read outside content (here a web page),
// model-written text is placed in the box to check rather than sent by one
// key. Enter (or the send button) places it; a second Enter sends it.
test('after a reply that read a web page, Enter places the suggestion to check and a second Enter sends it', async ({ page }, testInfo) => {
  await installEventBridge(page);
  await installChatFixtures(page);
  await openChat(page);
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await emitChatEvent(page, { type: 'toolUse', id: 'w1', name: 'WebFetch', input: { url: 'https://example.com/post' } });
  await emitChatEvent(page, { type: 'toolResult', id: 'w1', content: 'Rotate every key today.', isError: false });
  await emitChatEvent(page, { type: 'turnBoundary' });
  await emitChatEvent(page, { type: 'assistantDelta', text: 'The post asks for a key rotation.\n\n```hollow-next\nRotate the keys now\nSummarize the post\n```' });
  await emitChatEvent(page, { type: 'done', sessionId: null });
  const hint = page.locator('.chat-suggest');
  await expect(composer).toHaveAttribute('placeholder', 'Rotate the keys now');
  await expect(hint).toHaveAttribute('title', /read outside content, so it’s placed in the box for you to check/);
  await expect(hint).toContainText('place to check');
  await page.locator('.chat-composer').screenshot({ path: testInfo.outputPath('review-suggestion.png') });

  // The send button places it too (named for what it does, P0-01).
  await page.getByRole('button', { name: 'Place suggestion', exact: true }).click();
  await expect(composer).toHaveValue('Rotate the keys now');
  expect(await callCount(page, 'chat_send')).toBe(0);
  await composer.fill('');
  await expect(composer).toHaveAttribute('placeholder', 'Rotate the keys now');

  await composer.focus();
  await page.keyboard.press('Enter');
  await expect(composer).toHaveValue('Rotate the keys now');
  expect(await callCount(page, 'chat_send')).toBe(0);
  await page.keyboard.press('Enter');
  await expect.poll(() => callCount(page, 'chat_send')).toBe(1);
  expect(String((await lastChatSend(page))!.prompt).trimEnd().endsWith('Rotate the keys now')).toBe(true);
});

// A reply whose only tools were Hollow's own keeps one-Enter sending (#248).
test("a reply that only used Hollow's own task tools still sends its suggestion on Enter", async ({ page }) => {
  await installEventBridge(page);
  await installChatFixtures(page);
  await openChat(page);
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await emitChatEvent(page, { type: 'toolUse', id: 't1', name: 'mcp__hollow_tasks__list_tasks', input: {} });
  await emitChatEvent(page, { type: 'toolResult', id: 't1', content: '[]', isError: false });
  await emitChatEvent(page, { type: 'turnBoundary' });
  await emitChatEvent(page, { type: 'assistantDelta', text: 'No open tasks.\n\n```hollow-next\nCreate a task for the launch\n```' });
  await emitChatEvent(page, { type: 'done', sessionId: null });
  await expect(composer).toHaveAttribute('placeholder', 'Create a task for the launch');
  await expect(page.locator('.chat-suggest')).not.toHaveAttribute('data-review', '');
  await composer.focus();
  await page.keyboard.press('Enter');
  await expect.poll(() => callCount(page, 'chat_send')).toBe(1);
});

// Paperclip PR 21: the approval broker binds a request to its tool call by
// toolUseId; the webview badges exactly that row, whichever event arrives
// first, and folds runs of same-family calls.
const QUIT = 'mcp__hollow_computer__quit_app';

test('the streaming caret sits only where text is arriving', async ({ page }) => {
  await installEventBridge(page);
  await installChatFixtures(page);
  await openChat(page);
  const transcript = page.locator('.chat-main');
  const caret = transcript.locator('.chat-msg.typing');

  // A native run: text, a tool call, then more text. Every segment stays
  // partial until the run ends; only the live tail may carry the caret.
  await emitChatEvent(page, { type: 'assistantDelta', text: 'Looking at the store first.' });
  await expect(caret).toHaveCount(1);
  await emitChatEvent(page, { type: 'turnBoundary' });
  await emitChatEvent(page, { type: 'toolUse', id: 'c1', name: 'Read', input: { file_path: '/v/notes/a.md' } });
  await emitChatEvent(page, { type: 'toolResult', id: 'c1', content: 'ok', isError: false });
  await expect(caret).toHaveCount(0);
  await emitChatEvent(page, { type: 'turnBoundary' });
  await emitChatEvent(page, { type: 'assistantDelta', text: 'Found it.' });
  await expect(caret).toHaveCount(1);
  await expect(caret).toContainText('Found it.');

  // Waiting with no new text (a tool, a background check): no caret.
  await expect(caret).toHaveCount(0, { timeout: 3000 });
  await emitChatEvent(page, { type: 'assistantDelta', text: ' More.' });
  await expect(caret).toHaveCount(1);
  await emitChatEvent(page, { type: 'done', sessionId: null });
  await expect(caret).toHaveCount(0);
});

test('approval badges and folded tool rows, in either event order', async ({ page }) => {
  await installEventBridge(page);
  await installChatFixtures(page);
  await openChat(page);
  const transcript = page.locator('.chat-main');

  // Case 1: the tool call first, then its approval.
  for (const id of ['r1', 'r2', 'r3', 'r4']) {
    await emitChatEvent(page, { type: 'toolUse', id, name: 'Read', input: { file_path: `/v/notes/${id}.md` } });
  }
  await emitChatEvent(page, { type: 'toolUse', id: 'toolu_q', name: QUIT, input: { name: 'Safari' } });
  await emitEvent(page, 'approval-request', {
    id: 'apr-1', title: 'Computer control', action: 'Claude wants to quit Safari.', kind: 'computer',
    tool: 'quit_app', toolUseId: 'toolu_q', sessionKey: null,
  });
  await expect(transcript.locator('.tool-group')).toHaveCount(1);
  await expect(transcript.locator('.tool-group .toolcall-verb')).toHaveText('Read 4 files');
  const quitRow = transcript.locator('.toolcall').filter({ hasText: 'Quit app' });
  await expect(quitRow.locator('.toolcall-approval')).toHaveText('Awaiting approval');
  // The tail card names the call it is about.
  const card = transcript.locator('.chat-approval');
  await expect(card.locator('.chat-approval-tool')).toContainText('Quit app');
  const before = await callCount(page, 'approval_respond');
  await card.getByRole('button', { name: 'Deny', exact: true }).click();
  await expect.poll(() => callCount(page, 'approval_respond')).toBe(before + 1);
  await emitEvent(page, 'approval-resolved', {
    id: 'apr-1', approved: false, cancelled: false, reason: 'denied', toolUseId: 'toolu_q', sessionKey: null, tool: 'quit_app',
  });
  await expect(quitRow.locator('.toolcall-approval')).toHaveText('Denied');
  await expect(card).toHaveCount(0);

  // The Raw toggle shows every call unfolded.
  await transcript.getByRole('button', { name: 'Raw', exact: true }).click();
  await expect(transcript.getByRole('button', { name: 'Raw', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(transcript.locator('.tool-group')).toHaveCount(0);
  await expect(transcript.locator('.toolcall')).toHaveCount(5);
});

test('an approval that arrives before its tool call still badges the row', async ({ page }) => {
  await installEventBridge(page);
  await installChatFixtures(page);
  await openChat(page);
  const transcript = page.locator('.chat-main');

  await emitEvent(page, 'approval-request', {
    id: 'apr-2', title: 'Computer control', action: 'Claude wants to quit Notes.', kind: 'computer',
    tool: 'quit_app', toolUseId: 'toolu_q2', sessionKey: null,
  });
  await emitChatEvent(page, { type: 'toolUse', id: 'toolu_q2', name: QUIT, input: { name: 'Notes' } });
  const quitRow = transcript.locator('.toolcall').filter({ hasText: 'Quit app' });
  await expect(quitRow.locator('.toolcall-approval')).toHaveText('Awaiting approval');
  await emitEvent(page, 'approval-resolved', {
    id: 'apr-2', approved: false, cancelled: true, reason: 'timed_out', toolUseId: 'toolu_q2', sessionKey: null, tool: 'quit_app',
  });
  await expect(quitRow.locator('.toolcall-approval')).toHaveText('Timed out');

  // An unbound request badges nothing but still shows its card.
  await emitEvent(page, 'approval-request', {
    id: 'apr-3', title: 'Computer control', action: 'Claude wants to read the active tab in Safari.', kind: 'computer',
    tool: 'browser_current_tab', toolUseId: null, sessionKey: null,
  });
  await expect(transcript.locator('.chat-approval')).toHaveCount(1);
  await expect(transcript.locator('.chat-approval-tool')).toHaveCount(0);
  await expect(transcript.locator('.toolcall-approval')).toHaveCount(1);
});

test('an expanded tool group stays open, on the same row, as members stream in', async ({ page }) => {
  await installEventBridge(page);
  await installChatFixtures(page);
  await openChat(page);
  const transcript = page.locator('.chat-main');
  for (const id of ['g1', 'g2', 'g3']) {
    await emitChatEvent(page, { type: 'toolUse', id, name: 'Read', input: { file_path: `/v/notes/${id}.md` } });
  }
  const group = transcript.locator('.tool-group');
  await expect(group.locator('.toolcall-verb')).toHaveText('Read 3 files');
  // The label, not the header's middle: a file chip there opens its file.
  await group.locator('.toolcall-verb').first().click();
  await expect(group).toHaveClass(/open/);
  // Mark the DOM node: a rebuilt row would lose the property.
  await group.evaluate(el => { (el as HTMLElement & { __ledgerMark?: number }).__ledgerMark = 1; });
  await emitChatEvent(page, { type: 'toolUse', id: 'g4', name: 'Read', input: { file_path: '/v/notes/g4.md' } });
  await expect(group.locator('.toolcall-verb').first()).toHaveText('Read 4 files');
  await expect(group).toHaveClass(/open/);
  await expect(group.locator('.tool-group-members > .toolcall')).toHaveCount(4);
  expect(await group.evaluate(el => (el as HTMLElement & { __ledgerMark?: number }).__ledgerMark)).toBe(1);
});

// Ledger rows show what a call acted on as a widget: a file chip with a
// type icon, a mono pill for a command, `+a −d` for an edit, a line count
// once a read returns; the header shows one icon per call and the counts.
test('ledger rows show file chips, pills, diff counts and a per-call icon strip', async ({ page }, testInfo) => {
  await installEventBridge(page);
  await installChatFixtures(page);
  await openChat(page);
  const transcript = page.locator('.chat-main');
  const calls: Array<{ id: string; name: string; input: unknown; result?: string }> = [
    { id: 'w-img', name: 'Read', input: { file_path: '/v/shots/image.png' }, result: '' },
    { id: 'w-sh', name: 'Bash', input: { command: 'echo "Bash row: the preview shows my command"', description: 'Demo a Bash tool row' }, result: 'ok' },
    { id: 'w-rd', name: 'Read', input: { file_path: '/v/CLAUDE.md' }, result: '     1→# Hollow\n     2→\n     3→Read AGENTS.md' },
    { id: 'w-wr', name: 'Write', input: { file_path: '/v/header-demo.md', content: 'one\ntwo\n' }, result: 'ok' },
    { id: 'w-ts', name: 'ToolSearch', input: { query: 'select:WebFetch' }, result: 'ok' },
    { id: 'w-ed', name: 'Edit', input: { file_path: '/v/header-demo.md', old_string: 'two', new_string: 'three' }, result: 'ok' },
    { id: 'w-web', name: 'WebFetch', input: { url: 'https://docs.conductor.build', prompt: 'x' }, result: 'ok' },
    { id: 'w-ag', name: 'Agent', input: { description: 'Demo subagent row', subagent_type: 'Explore' }, result: 'ok' },
  ];
  for (const c of calls) {
    await emitChatEvent(page, { type: 'toolUse', id: c.id, name: c.name, input: c.input });
    await emitChatEvent(page, { type: 'toolResult', id: c.id, content: c.result ?? 'ok', isError: false });
  }
  const ledger = transcript.locator('.chat-tool-ledger');
  await expect(ledger.locator('summary')).toContainText('7 tool calls, 1 subagent');
  await expect(ledger.locator('.tool-ledger-glyph')).toHaveCount(8);
  // Finished and short: it stays open, so the widgets show without a click.
  await expect(ledger).toHaveAttribute('open', '');

  const row = (text: string) => ledger.locator('.toolcall').filter({ hasText: text });
  await expect(row('Read image').locator('.toolcall-file.tone-image')).toHaveText('image.png');
  await expect(row('Read 3 lines').locator('.toolcall-file.tone-doc')).toHaveText('CLAUDE.md');
  await expect(row('Write 2 lines').locator('.toolcall-file')).toHaveText('header-demo.md');
  await expect(row('Demo a Bash tool row').locator('.toolcall-pill')).toContainText('echo');
  const edit = ledger.locator('.toolcall').filter({ has: page.locator('.toolcall-counts') });
  await expect(edit.locator('.toolcall-add')).toHaveText('+1');
  await expect(edit.locator('.toolcall-del')).toHaveText('−1');
  await page.screenshot({ path: process.env.LEDGER_SHOT ?? testInfo.outputPath('ledger-widgets.png') });

  // A ledger the user closed stays closed as the turn goes on.
  await ledger.locator('summary').click();
  await expect(ledger).not.toHaveAttribute('open', '');
  await emitChatEvent(page, { type: 'toolUse', id: 'w-late', name: 'Bash', input: { command: 'ls', description: 'List files' } });
  await emitChatEvent(page, { type: 'toolResult', id: 'w-late', content: 'ok', isError: false });
  await expect(ledger.locator('summary')).toContainText('8 tool calls, 1 subagent');
  await expect(ledger).not.toHaveAttribute('open', '');
});

test('a folded run of edits lists its files and sums its changes', async ({ page }) => {
  await installEventBridge(page);
  await installChatFixtures(page);
  await openChat(page);
  const transcript = page.locator('.chat-main');
  const edits = [
    { id: 'e1', file_path: '/r/src/a.ts', old_string: 'x', new_string: 'y' },
    { id: 'e2', file_path: '/r/src/a.ts', old_string: 'p', new_string: 'q\nr' },
    { id: 'e3', file_path: '/r/docs/b.md', old_string: 'old', new_string: 'new' },
  ];
  for (const { id, ...input } of edits) {
    await emitChatEvent(page, { type: 'toolUse', id, name: 'Edit', input });
  }
  const group = transcript.locator('.tool-group');
  await expect(group.locator('.toolcall-verb')).toHaveText('Edited 2 files');
  await expect(group.locator('.toolcall-head .toolcall-file')).toHaveText(['a.ts', 'b.md']);
  await expect(group.locator('.toolcall-head .toolcall-add')).toHaveText('+4');
  await expect(group.locator('.toolcall-head .toolcall-del')).toHaveText('−3');
});

test('the Raw view windows its rows past 300 calls', async ({ page }) => {
  await installEventBridge(page);
  await installChatFixtures(page);
  await openChat(page);
  const transcript = page.locator('.chat-main');
  const total = 400;
  await page.evaluate(n => {
    const emit = (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void }).__emitTauriEvent;
    for (let i = 1; i <= n; i++) {
      emit('chat-event', { type: 'toolUse', id: `bulk-${i}`, name: i % 2 ? 'Read' : 'Bash', input: i % 2 ? { file_path: `/v/n${i}.md` } : { command: `echo ${i}` } });
    }
  }, total);
  await expect(transcript.locator('.chat-tool-ledger summary')).toContainText(`${total} tool calls`);
  const ledger = transcript.locator('.chat-tool-ledger');
  // Every call is still running, so the ledger is already open.
  await ledger.getByRole('button', { name: 'Raw', exact: true }).click();
  const list = ledger.locator('.tool-ledger-raw-list');
  await expect(list).toBeVisible();
  const rows = list.locator('.tool-ledger-raw-row');
  // Visible rows (420 px / 28 px = 15) plus 20 overscan: far below 400.
  const firstCount = await rows.count();
  expect(firstCount).toBeGreaterThan(0);
  expect(firstCount).toBeLessThanOrEqual(15 + 2 * 20);
  await expect(ledger.locator('.toolcall')).toHaveCount(0);
  expect(await list.locator('.tool-ledger-raw-spacer').evaluate(el => (el as HTMLElement).style.height)).toBe(`${total * 28}px`);
  await expect(rows.first().locator('.tool-ledger-raw-n')).toHaveText('1');
  // Scrolled to the bottom, the last call is rendered and the first is not.
  await list.evaluate(el => { el.scrollTop = el.scrollHeight; el.dispatchEvent(new Event('scroll')); });
  await expect(list.locator('.tool-ledger-raw-n').filter({ hasText: new RegExp(`^${total}$`) })).toHaveCount(1);
  await expect(list.locator('.tool-ledger-raw-n').filter({ hasText: /^1$/ })).toHaveCount(0);
  expect(await rows.count()).toBeLessThanOrEqual(15 + 2 * 20);
});

// Paperclip PR 7b: chat_send enqueues and returns SendAccepted; the turn's
// events echo the send's clientTurnId. The harness emits no events itself.
test('a turn queued behind a build worktree names the chat using it and opens it (P1-46)', async ({ page }) => {
  await installEventBridge(page);
  await installChatFixtures(page);
  await openChat(page);
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  const badge = page.locator('.chat-main .chat-queued-badge');
  await composer.fill('Rebuild the panel');
  await composer.press('Meta+Enter');
  await expect.poll(() => callCount(page, 'chat_send')).toBe(1);
  // The mocked chat_send numbers rows from 1 on each page load: this send is row 1.
  const queueId = 1;
  // The queue list names the conversation holding the worktree, as run_pump does.
  await page.evaluate(id => {
    localStorage.setItem('mockRunQueue', JSON.stringify([{
      id, source: 'chat', sourceId: 's-20260925-mock0001', label: 'Chat', detail: 'Rebuild the panel', reason: 'chat', status: 'queued', priority: 0, position: 1,
      attempt: 0, coalescedCount: 0, requestedAt: '2026-10-25T18:00:00Z', claimedAt: null, finishedAt: null, historyRunId: null, error: null,
      waitingSince: null, waitingHolder: null, heldReason: null, cancelRequested: false,
      waitingReason: '“Fix the panel” to finish (it has used this worktree for 12 min)', waitingSession: 's-20260925-mock0002',
    }]));
    (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void }).__emitTauriEvent('run-queue-event', { kind: 'waiting', id });
  }, queueId);
  await expect(badge).toContainText('Queued · waiting for “Fix the panel” to finish (it has used this worktree for 12 min)');
  await expect(badge.getByRole('button', { name: 'Open it', exact: true })).toBeVisible();
});

test('a queued chat turn shows its holder and can be edited, discarded or recovered', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockChatWaitingFor', 'the Morning brief workflow'));
  await installEventBridge(page);
  await installChatFixtures(page);
  await openChat(page);
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  const transcript = page.locator('.chat-main');
  const badge = transcript.locator('.chat-queued-badge');
  const mockKey = 's-20260925-mock0001';
  const turnId = async () => String((await lastChatSend(page))!.clientTurnId);
  const storedKey = () => page.evaluate(() => localStorage.getItem('chatAgentSessionKey'));

  // Waiting behind a workflow: the bubble names the holder.
  await composer.fill('Summarize the launch notes');
  await composer.press('Meta+Enter');
  await expect.poll(() => callCount(page, 'chat_send')).toBe(1);
  expect(await turnId()).toMatch(/^ct-/);
  await expect(badge).toContainText('Queued · waiting for the Morning brief workflow');
  expect(await storedKey()).toBe(mockKey);
  // A plain send continues no other conversation (agent effectiveness F4).
  expect((await lastChatSend(page))!.continuedFrom).toBeNull();

  // Edit takes it out of the queue once and puts the text back, with no stop
  // notice. cancel_queued_run's answer ends it; its own events come later.
  await badge.getByRole('button', { name: 'Edit', exact: true }).click();
  await expect.poll(() => callCount(page, 'cancel_queued_run')).toBe(1);
  const editedId = await turnId();
  await expect(composer).toHaveValue('Summarize the launch notes');
  await expect(transcript.locator('.turn.user')).toHaveCount(0);
  await expect(badge).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Stop', exact: true })).toHaveCount(0);
  await expect(transcript).not.toContainText('Stopped');

  // Re-sent at once: the edited turn's late cancelled + done must not end it.
  await composer.press('Meta+Enter');
  await expect.poll(() => callCount(page, 'chat_send')).toBe(2);
  await expect(badge).toBeVisible();
  await emitChatEvent(page, { type: 'cancelled', stop: null, queued: true, clientTurnId: editedId, queueId: 1 });
  await emitChatEvent(page, { type: 'done', sessionId: null, sessionKey: mockKey, clientTurnId: editedId, queueId: 1 });
  await expect(badge).toBeVisible();
  await expect(page.getByRole('button', { name: 'Stop', exact: true })).toBeVisible();
  await expect(transcript.locator('.turn.user')).toHaveCount(1);
  await expect(transcript).not.toContainText('Stopped');

  // Discard drops it; the composer stays empty.
  await badge.getByRole('button', { name: 'Discard', exact: true }).click();
  await expect.poll(() => callCount(page, 'cancel_queued_run')).toBe(2);
  let id = await turnId();
  await emitChatEvent(page, { type: 'cancelled', stop: null, queued: true, clientTurnId: id, queueId: 2 });
  await emitChatEvent(page, { type: 'done', sessionId: null, sessionKey: mockKey, clientTurnId: id, queueId: 2 });
  await expect(composer).toHaveValue('');
  await expect(transcript.locator('.turn.user')).toHaveCount(0);
  await expect(transcript.locator('.turn.system .chat-msg').last()).toHaveText('Queued message discarded. It was never sent.');

  // Recovered at claim: the folder vanished while it waited. The composer gets
  // the text back, the vault notice shows, and the dead key is not adopted.
  await composer.fill('Check the release folder');
  await composer.press('Meta+Enter');
  await expect.poll(() => callCount(page, 'chat_send')).toBe(3);
  await expect(badge).toBeVisible();
  id = await turnId();
  await emitChatEvent(page, { type: 'error', message: 'CHAT_CONTEXT_RECOVERED: the working folder no longer exists', clientTurnId: id, queueId: 3 });
  await emitChatEvent(page, { type: 'done', sessionId: null, clientTurnId: id, queueId: 3, recovered: true });
  await expect(composer).toHaveValue('Check the release folder');
  await expect(transcript.locator('.turn.system').last()).toContainText('prior agent context cannot be resumed');
  await expect(transcript).not.toContainText('(error: CHAT_CONTEXT_RECOVERED');
  await expect(transcript.locator('.turn.user')).toHaveCount(0);
  expect(await storedKey()).toBeNull();

  // Stop on a queued turn names that turn's session key (chat::cancel withdraws
  // a queued row only when it gets one), and its cancelled{queued} + done put
  // the text back with the stop notice.
  await composer.fill('Outline the retro');
  await composer.press('Meta+Enter');
  await expect.poll(() => callCount(page, 'chat_send')).toBe(4);
  // The window still shows the left conversation, so the send names it and
  // the backend bundles its recorded turns (agent effectiveness F4, K1 review).
  expect((await lastChatSend(page))!.continuedFrom).toEqual([mockKey]);
  await expect(badge).toBeVisible();
  id = await turnId();
  await page.getByRole('button', { name: 'Stop', exact: true }).click();
  await expect.poll(() => callCount(page, 'chat_cancel')).toBe(1);
  const cancelArgs = await page.evaluate(() => {
    const requests = (window as unknown as { __chatFixtureRequests: Array<{ command: string; args: Record<string, unknown> }> }).__chatFixtureRequests;
    return requests.filter(request => request.command === 'chat_cancel').at(-1)?.args;
  });
  expect(cancelArgs).toEqual({ sessionKey: mockKey });
  await emitChatEvent(page, { type: 'cancelled', stop: null, queued: true, clientTurnId: id, queueId: 4 });
  await emitChatEvent(page, { type: 'done', sessionId: null, sessionKey: mockKey, clientTurnId: id, queueId: 4 });
  await expect(composer).toHaveValue('Outline the retro');
  await expect(transcript.locator('.turn.user')).toHaveCount(0);
  await expect(transcript.locator('.turn.system .chat-msg').last()).toHaveText('Stopped before it started. Your message is back in the composer.');
  await composer.fill('');

  // Admitted: runStarted removes the badge without a second bubble.
  await composer.fill('Draft the summary');
  await composer.press('Meta+Enter');
  await expect.poll(() => callCount(page, 'chat_send')).toBe(5);
  await expect(badge).toBeVisible();
  id = await turnId();
  await emitChatEvent(page, { type: 'runStarted', runId: 'run-queued-1', sessionKey: mockKey, recall: [], clientTurnId: id, queueId: 5, asked: 'Draft the summary' });
  await expect(badge).toHaveCount(0);
  await expect(transcript.locator('.turn.user')).toHaveCount(1);
  await expect(transcript.locator('.turn.user')).toContainText('Draft the summary');
  await emitChatEvent(page, { type: 'done', sessionId: null, sessionKey: mockKey, clientTurnId: id, queueId: 5 });
  await expect(page.getByRole('button', { name: 'Stop', exact: true })).toHaveCount(0);
  await expect(transcript.locator('.turn.user')).toHaveCount(1);
  await expect(composer).toHaveValue('');
});

// Paperclip PR 7b: a chat turn the previous Hollow process queued but never
// started is interrupted at startup (never run) and put back into an empty
// composer once, through the run-queue refresh.
test('a turn queued before Hollow quit comes back to the composer once, never sent', async ({ page }) => {
  // Only the conversation on screen restores its own interrupted turn (see
  // planQueuedChatRows' foregroundKey rule, 2026-09-29): make it this one.
  await page.addInitScript(() => { if (!sessionStorage.getItem('queuedKeySeeded')) { sessionStorage.setItem('queuedKeySeeded', '1'); localStorage.setItem('chatAgentSessionKey', 's-20260925-0000pr10'); } });
  await page.addInitScript(() => localStorage.setItem('mockRunQueue', JSON.stringify([{
    id: 7, source: 'chat', sourceId: 's-20260925-0000pr10', label: 'Chat', detail: 'Send the launch recap',
    reason: 'chat', status: 'interrupted', priority: 0, position: null, attempt: 0, coalescedCount: 0,
    requestedAt: '2026-09-25T10:00:00.000Z', claimedAt: null, finishedAt: '2026-09-25T11:00:00.000Z',
    historyRunId: null, error: 'Hollow stopped before this queued run started', waitingSince: null,
    waitingHolder: null, heldReason: null, cancelRequested: false,
  }])));
  await installChatFixtures(page);
  await openChat(page);
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await expect(composer).toHaveValue('Send the launch recap');
  await expect(page.locator('.chat-main')).toContainText('was never sent');
  expect(await callCount(page, 'chat_send')).toBe(0);
  await expect.poll(() => page.evaluate(() => localStorage.getItem('restoredQueuedTurns'))).toContain('7@');

  // Only once: after a reload the same row is not restored again.
  await composer.fill('');
  await page.reload();
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await expect.poll(() => callCount(page, 'list_run_queue')).toBeGreaterThan(0);
  await showChat(page);
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toHaveValue('');
  expect(await callCount(page, 'chat_send')).toBe(0);
});

// A long command in a reply scrolls inside its code block; the block (and
// its copy button) stay within the message column.
test('a long code block scrolls inside the message instead of overflowing it', async ({ page }) => {
  await installEventBridge(page);
  await installChatFixtures(page);
  await openChat(page);
  const cmd = 'cd ~/Hollow/Worktrees/hollow/app-mcp-a1 && CARGO_TARGET_DIR=~/Library/Caches/hollow-release-target npm run tauri:install -- --with-a-very-long-tail-of-arguments';
  await emitChatEvent(page, { type: 'assistantText', text: `To install, run:\n\n\`\`\`\n${cmd}\n\`\`\`\n\nThen tell me.` });
  const block = page.locator('.chat-main .chat-codeblock').first();
  await expect(block).toBeVisible();
  // The block is no wider than the message it sits in.
  const msg = await block.evaluate(el => el.closest('.chat-msg')!.getBoundingClientRect().right);
  const b = await block.boundingBox();
  expect(b!.x + b!.width).toBeLessThanOrEqual(msg + 1);
  await expect(block.getByRole('button', { name: 'Copy text block' })).toBeInViewport();
  const pre = block.locator('pre');
  expect(await pre.evaluate(el => el.scrollWidth > el.clientWidth)).toBe(true);
});

test('an expanded long command scrolls inside its tool row', async ({ page }) => {
  await installEventBridge(page);
  await installChatFixtures(page);
  await openChat(page);
  const cmd = `npm run tauri:install -- ${'--a-long-flag '.repeat(30)}`;
  await emitChatEvent(page, { type: 'toolUse', id: 'long-sh', name: 'Bash', input: { command: cmd, description: 'Install the app' } });
  await emitChatEvent(page, { type: 'toolResult', id: 'long-sh', content: 'ok', isError: false });
  const row = page.locator('.chat-main .toolcall').filter({ hasText: 'Install the app' });
  await row.locator('.toolcall-head').click();
  const pre = row.locator('.toolcall-code').first();
  await expect(pre).toBeVisible();
  const [p, r] = [await pre.boundingBox(), await row.boundingBox()];
  expect(p!.x + p!.width).toBeLessThanOrEqual(r!.x + r!.width + 1);
  expect(await pre.evaluate(el => el.scrollWidth > el.clientWidth)).toBe(true);
});

// A pasted screenshot is thousands of pixels wide; the composer shows it as
// a thumbnail, not at its natural size over the transcript.
test('a large attached image shows as a thumbnail', async ({ page }) => {
  await openChat(page);
  const b64 = await page.evaluate(() => {
    const c = document.createElement('canvas');
    c.width = 3020; c.height = 2020;
    const g = c.getContext('2d')!;
    g.fillStyle = '#446'; g.fillRect(0, 0, c.width, c.height);
    return c.toDataURL('image/png').split(',')[1];
  });
  await page.locator('.chat-main input[type="file"]').setInputFiles({ name: 'screenshot.png', mimeType: 'image/png', buffer: Buffer.from(b64, 'base64') });
  const card = page.locator('.chat-attachments .chat-attach').first();
  await expect(card.getByRole('img', { name: 'screenshot.png' })).toBeVisible();
  const box = await card.boundingBox();
  expect(box!.width).toBeLessThanOrEqual(140);
  expect(box!.height).toBeLessThanOrEqual(120);
});

test('attached images survive page changes and a new conversation clears the draft', async ({ page }) => {
  await openChat(page);
  await page.locator('.chat-main input[type="file"]').setInputFiles(imageFile('reference.png'));
  await expect(page.locator('.chat-attachments').getByRole('img', { name: 'reference.png' })).toBeVisible();
  await expect.poll(() => callCount(page, 'chat_save_attachment')).toBe(1);
  await page.getByRole('textbox', { name: 'Chat message' }).fill('Review this image');
  await page.getByRole('button', { name: 'Library', exact: true }).click();
  await showChat(page);
  const panel = page.locator('.chat-main');
  await expect(panel.locator('.chat-attachments').getByRole('img', { name: 'reference.png' })).toBeVisible();
  await page.locator('.library-conversations').getByRole('button', { name: 'New chat', exact: true }).click();
  await expect(panel.getByRole('textbox', { name: 'Chat message' })).toHaveValue('');
  await expect(panel.locator('.chat-attachments')).toHaveCount(0);
  expect(await callCount(page, 'chat_send')).toBe(0);
});

// C5 (2026-11-04): any file attaches, not only an image.
test('a picked file that is not an image attaches as a chip and is sent with its name and saved path', async ({ page }) => {
  await installChatFixtures(page);
  await openChat(page);
  await page.locator('.chat-main input[type="file"]').setInputFiles({ name: 'Q3 notes.txt', mimeType: 'text/plain', buffer: Buffer.from('revenue up') });
  const chip = page.locator('.chat-attachments .chat-attach-file');
  await expect(chip).toContainText('Q3 notes.txt');
  await expect(chip).toContainText('10 B');
  await expect.poll(() => callCount(page, 'chat_save_attachment')).toBe(1);
  await expect(chip.getByRole('button', { name: 'Remove Q3 notes.txt' })).toBeVisible();
  await page.getByRole('textbox', { name: 'Chat message' }).fill('Summarize these notes');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect.poll(async () => String((await lastChatSend(page))?.prompt ?? ''))
    .toContain('1. Q3 notes.txt at /mock/vault/.hollow-attachments/Q3-notes-1.txt');
  await expect(page.locator('.chat-user-chip').getByRole('button', { name: 'Open Q3 notes.txt' })).toBeVisible();
  await expect(page.locator('.chat-attachments')).toHaveCount(0);
});

// Run 15 P4: these two ran over Claude history; they now run over Hollow's
// own list (title, folder and agent are searchable). Claude history's
// restore and late-read tests went with that tab.
test('conversation search, keyboard selection and collapse work at either width', async ({ page }) => {
  await installChatFixtures(page);
  await openChat(page);
  const sidebar = await openConversations(page);
  const search = page.getByRole('textbox', { name: 'Search conversations' });
  await expect(sidebar.getByRole('button', { name: 'Open Codex delivery plan', exact: true })).toBeVisible();
  await search.fill('missing conversation');
  await expect(sidebar.locator('.session-row')).toHaveCount(0);
  await expect(sidebar).toContainText('No conversations');
  // Search includes the folder as well as the title and agent.
  await search.fill('/mock/design');
  const session = sidebar.getByRole('button', { name: 'Open Claude design review', exact: true });
  await expect(session).toBeVisible();
  await expect(sidebar.getByRole('button', { name: 'Open Codex delivery plan', exact: true })).toHaveCount(0);
  await session.focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('.chat-main')).toContainText('Claude design transcript is ready.');
  expect(await page.evaluate(() => localStorage.getItem('chatAgentSessionKey'))).toBe('s-20260907-cccc0002');

  await openConversations(page);
  await page.getByRole('button', { name: 'Hide conversations', exact: true }).click();
  await expect(search).toBeHidden();
  await expect(page.locator('.chat-main')).toContainText('Claude design transcript is ready.');
  await openConversations(page);
  await expect(search).toHaveValue('/mock/design');
});

test('selecting the active conversation preserves its unsent text and image', async ({ page }) => {
  await installChatFixtures(page);
  await openChat(page);
  let sidebar = await openConversations(page);
  await sidebar.getByRole('button', { name: 'Open Claude design review', exact: true }).click();
  await expect(page.locator('.chat-main')).toContainText('Claude design transcript is ready.');
  await page.getByRole('textbox', { name: 'Chat message' }).fill('Add this image to the design review.');
  await page.locator('.chat-main input[type="file"]').setInputFiles(imageFile('design.png'));
  await expect.poll(() => callCount(page, 'chat_save_attachment')).toBe(1);

  sidebar = await openConversations(page);
  await sidebar.getByRole('button', { name: 'Open Claude design review', exact: true }).click();
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toHaveValue('Add this image to the design review.');
  await expect(page.locator('.chat-attachments').getByRole('img', { name: 'design.png' })).toBeVisible();
  await expect(page.locator('.chat-main')).toContainText('Claude design transcript is ready.');
  expect(await callCount(page, 'read_agent_conversation')).toBe(1);
  expect(await callCount(page, 'chat_send')).toBe(0);
});

test('new chat discards an image still waiting for its folder check', async ({ page }) => {
  await installChatFixtures(page, false, true);
  await openChat(page);
  await page.locator('.chat-main input[type="file"]').setInputFiles(imageFile('pending.png'));
  await expect.poll(() => callCount(page, 'chat_preflight')).toBe(1);
  await expect(page.locator('.chat-attachments')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeDisabled();
  await page.locator('.library-conversations').getByRole('button', { name: 'New chat', exact: true }).click();
  await releaseReplies(page);
  await expect(page.locator('.chat-attachments')).toHaveCount(0);
  expect(await callCount(page, 'chat_save_attachment')).toBe(0);
  expect(await callCount(page, 'chat_send')).toBe(0);
});

test('switching agents settles a pending image as removable and blocks sending until removal', async ({ page }) => {
  await installChatFixtures(page, false, true);
  await openChat(page);
  await page.getByRole('textbox', { name: 'Chat message' }).fill('Review this reference.');
  await page.locator('.chat-main input[type="file"]').setInputFiles(imageFile('pending.png'));
  await expect.poll(() => callCount(page, 'chat_preflight')).toBe(1);
  await expect(page.locator('.chat-attach')).toContainText('Saving…');

  await page.getByRole('button', { name: /^Model:/ }).click();
  await page.getByRole('option', { name: /Codex default/ }).click();
  await releaseReplies(page);
  await expect(page.locator('.chat-attach')).toContainText('Failed: remove to send');
  await expect(page.locator('.chat-attach.saving')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeDisabled();
  expect(await page.evaluate(() => localStorage.getItem('chatAgent'))).toBe('codex');
  expect(await callCount(page, 'chat_save_attachment')).toBe(0);

  await page.getByTitle('Remove image', { exact: true }).click();
  await expect(page.locator('.chat-attachments')).toHaveCount(0);
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toHaveValue('Review this reference.');
  await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeEnabled();
  expect(await callCount(page, 'chat_send')).toBe(0);
});

test('Hollow history opens Codex transcripts with their provider, tools and receipt', async ({ page }) => {
  await installChatFixtures(page);
  await openChat(page);
  const sidebar = await openConversations(page, 'hollow');
  // Hollow · Archived replaced Show archived when the page became Library → Conversations (S4).
  await expect(sidebar.getByRole('group', { name: 'Conversation list' }).getByRole('button', { name: 'Archived', exact: true })).toBeVisible();
  await expect(sidebar.getByRole('button', { name: 'Open Codex delivery plan', exact: true })).toBeVisible();
  await sidebar.getByRole('button', { name: 'Open Codex delivery plan', exact: true }).click();
  await expect(page.locator('.chat-main')).toContainText('Codex transcript is ready.');
  await expect(page.getByRole('heading', { name: 'Codex delivery plan', exact: true })).toBeVisible();
  await expect(page.locator('.chat-conversation-heading')).toContainText('Codex');
  // The receipt opens from the reply's footer (its menu), not by itself.
  await expect(page.locator('.chat-main .execution-receipt')).toHaveCount(0);
  await page.locator('.chat-main .turn-footer').getByRole('button', { name: 'More actions', exact: true }).click();
  await page.locator('.ctx-item').filter({ hasText: 'Show run record' }).click();
  await expect(page.locator('.chat-main .execution-receipt')).toHaveCount(1);
  await expect(page.locator('.chat-main .execution-receipt')).toContainText('No changes');
  // A finished run's steps fold into one "Worked … · 1 step" row (frontier F5);
  // opened, a single call is just its row, with no ledger header (frontier F1).
  await page.locator('.chat-main .chat-run-fold-head').click();
  await expect(page.locator('.chat-main .chat-tool-ledger.single')).toContainText('Read');
  await expect(page.locator('.chat-main summary.toolcalls-cap')).toHaveCount(0);
  expect(await page.evaluate(() => ({
    agent: localStorage.getItem('chatAgent'), sessionId: localStorage.getItem('chatSessionId'), key: localStorage.getItem('chatAgentSessionKey'),
  }))).toEqual({ agent: 'codex', sessionId: 'codex-owned-thread', key: 's-20260907-cccc0001' });
  expect(await callCount(page, 'read_agent_conversation')).toBe(1);
  expect(await callCount(page, 'read_session')).toBe(0);
  expect(await callCount(page, 'chat_send')).toBe(0);
});

test('provider changes retain Hollow context, clear native resume and accept custom model IDs', async ({ page }) => {
  await installChatFixtures(page);
  await openChat(page);
  const sidebar = await openConversations(page, 'hollow');
  await sidebar.getByRole('button', { name: 'Open Codex delivery plan', exact: true }).click();
  await expect(page.locator('.chat-main')).toContainText('Codex transcript is ready.');
  // It starts on its native thread, so the clear below is the provider change's.
  expect(await page.evaluate(() => localStorage.getItem('chatSessionId'))).toBe('codex-owned-thread');
  await page.getByRole('textbox', { name: 'Chat message' }).fill('Continue with the next release check.');
  await page.getByRole('button', { name: /^Model:/ }).click();
  await page.getByRole('textbox', { name: 'Search models', exact: true }).fill('custom-model-v9');
  await page.getByRole('option', { name: 'Use “custom-model-v9” with Claude' }).click();
  await expect(page.getByRole('button', { name: /^Model:/ })).toHaveAttribute('aria-label', /^Model: Claude · custom-model-v9 · Permissions: /);
  await expect(page.locator('.chat-main')).toContainText('Codex transcript is ready.');
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toHaveValue('Continue with the next release check.');
  expect(await page.evaluate(() => localStorage.getItem('chatSessionId'))).toBeNull();
  expect(await page.evaluate(() => localStorage.getItem('chatAgentSessionKey'))).toBe('s-20260907-cccc0001');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect.poll(() => callCount(page, 'chat_send')).toBe(1);
  const request = (await lastChatSend(page))!;
  expect(request.agent).toBe('claude');
  expect(request.model).toBe('custom-model-v9');
  expect(request.sessionId).toBeNull();
  expect(request.sessionKey).toBe('s-20260907-cccc0001');
  // The conversation key is kept and the native id dropped, so the backend's
  // context compiler (agent effectiveness F4) starts a fresh native session
  // with the conversation and the profile from the session store; the
  // composer sends only the request and its own blocks.
  const prompt = String(request.prompt);
  expect(prompt).not.toContain('<conversation-context>');
  expect(prompt).not.toContain('<operator-context>');
  expect(prompt).toContain('<hollow-chat-intent>');
  expect(prompt.endsWith('Continue with the next release check.')).toBe(true);
});

test('saved Hollow identity restores Codex history without loading a Claude transcript', async ({ page }) => {
  await installChatFixtures(page);
  await page.addInitScript(() => {
    localStorage.setItem('chatAgentSessionKey', 's-20260907-cccc0001');
    localStorage.setItem('chatSessionId', 'codex-owned-thread');
    localStorage.setItem('chatAgent', 'codex');
    localStorage.setItem('chatCwd', '/mock/codex');
  });
  await openChat(page);
  await expect(page.locator('.chat-main')).toContainText('Codex transcript is ready.');
  await expect(page.locator('.chat-conversation-heading')).toContainText('Codex');
  expect(await callCount(page, 'read_agent_conversation')).toBe(1);
  expect(await callCount(page, 'read_session')).toBe(0);
  await page.getByRole('textbox', { name: 'Chat message' }).fill('Continue this same native thread.');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect.poll(() => callCount(page, 'chat_send')).toBe(1);
  const request = (await lastChatSend(page))!;
  expect(request.agent).toBe('codex');
  expect(request.sessionId).toBe('codex-owned-thread');
  expect(request.sessionKey).toBe('s-20260907-cccc0001');
  expect(String(request.prompt)).not.toContain('<conversation-context>');
});

test('new conversations use the app default agent and its configured model', async ({ page }) => {
  await installChatFixtures(page);
  await page.addInitScript(() => {
    localStorage.setItem('mockAgentSettings', JSON.stringify({ defaultAgent: 'codex', models: { codex: 'team-default-model' } }));
  });
  await openChat(page);
  const sidebar = await openConversations(page, 'hollow');
  await sidebar.getByRole('button', { name: 'Open Claude design review', exact: true }).click();
  await expect(page.locator('.chat-main')).toContainText('Claude design transcript is ready.');
  expect(await page.evaluate(() => localStorage.getItem('chatAgent'))).toBe('claude');
  // It starts on its native session and key, so the clears below are New chat's.
  expect(await page.evaluate(() => [localStorage.getItem('chatSessionId'), localStorage.getItem('chatAgentSessionKey')])).toEqual(['claude-owned-session', 's-20260907-cccc0002']);
  await page.locator('.library-conversations').getByRole('button', { name: 'New chat', exact: true }).click();
  await expect(page.locator('.chat-main .turn')).toHaveCount(0);
  await expect(page.locator('.chat-conversation-heading')).toContainText('Codex');
  await expect(page.getByRole('button', { name: /^Model:/ })).toHaveAttribute('aria-label', /^Model: Codex · team-default-model · Permissions: /);
  expect(await page.evaluate(() => localStorage.getItem('chatSessionId'))).toBeNull();
  expect(await page.evaluate(() => localStorage.getItem('chatAgentSessionKey'))).toBeNull();
  await page.getByRole('textbox', { name: 'Chat message' }).fill('Start a fresh default-agent conversation.');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect.poll(() => callCount(page, 'chat_send')).toBe(1);
  const request = (await lastChatSend(page))!;
  expect(request.agent).toBe('codex');
  expect(request.model).toBe('team-default-model');
  expect(String(request.prompt)).not.toContain('Claude design transcript is ready.');
});

test('Hollow archive hides and restores history without deleting transcripts', async ({ page }) => {
  await installChatFixtures(page);
  await openChat(page);
  const sidebar = await openConversations(page, 'hollow');
  const row = sidebar.locator('.session-row').filter({ has: page.getByRole('button', { name: 'Open Codex delivery plan', exact: true }) });
  await row.getByRole('button', { name: 'Settle conversation', exact: true }).click();
  await expect(sidebar.getByRole('button', { name: 'Open Codex delivery plan', exact: true })).toHaveCount(0);
  // Archive flashes with Undo (P1-39), which puts it back.
  await page.locator('.snackbar').filter({ hasText: 'Conversation settled' }).getByRole('button', { name: 'Undo', exact: true }).click();
  await expect(row).toBeVisible();
  await row.getByRole('button', { name: 'Settle conversation', exact: true }).click();
  await expect(row).toHaveCount(0);
  const listView = sidebar.getByRole('group', { name: 'Conversation list' });
  const archivedView = listView.getByRole('button', { name: 'Archived', exact: true });
  await expect(archivedView).toHaveAttribute('aria-pressed', 'false');
  await archivedView.click();
  await expect(archivedView).toHaveAttribute('aria-pressed', 'true');
  await expect(row).toBeVisible();
  await row.getByRole('button', { name: 'Restore conversation', exact: true }).click();
  await expect(row).toHaveCount(0);
  await listView.getByRole('button', { name: 'Hollow', exact: true }).click();
  await expect(row).toBeVisible();
  await row.getByRole('button', { name: 'Open Codex delivery plan', exact: true }).click();
  await expect(page.locator('.chat-main')).toContainText('Codex transcript is ready.');
  expect(await callCount(page, 'organize_conversation')).toBe(3);
  expect(await callCount(page, 'delete_session')).toBe(0);
});

// P1-39: the ⋯ beside the open conversation's title is its history row's
// menu, Rename and Copy ID included, and the working folder names the Hollow folder.
test('the open conversation’s title has the one conversation menu', async ({ page }) => {
  await installChatFixtures(page);
  await openChat(page);
  const sidebar = await openConversations(page, 'hollow');
  const row = sidebar.locator('.session-row').filter({ has: page.getByRole('button', { name: 'Open Codex delivery plan', exact: true }) });
  await row.getByRole('button', { name: /^Actions for / }).click();
  const fromRow = await page.locator('.ctx-menu [role="menuitem"]').allTextContents();
  await page.keyboard.press('Escape');
  await row.getByRole('button', { name: 'Open Codex delivery plan', exact: true }).click();
  await expect(page.locator('.chat-main')).toContainText('Codex transcript is ready.');
  await page.locator('.chat-conversation-head').getByRole('button', { name: 'Actions for this conversation', exact: true }).click();
  const fromTitle = await page.locator('.ctx-menu [role="menuitem"]').allTextContents();
  expect(fromTitle).toEqual(fromRow);
  expect(fromTitle).toEqual(expect.arrayContaining(['Rename…', 'Copy conversation ID', 'Settle conversation']));
  // Rename from the title's ⋯: the heading and the row follow at once.
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Rename…', exact: true }).click();
  await page.getByRole('textbox', { name: 'Name' }).fill('Delivery plan, final');
  await page.getByRole('button', { name: 'Rename', exact: true }).click();
  await expect(page.locator('.chat-conversation-head h3')).toHaveText('Delivery plan, final');
  // At compact width opening a conversation closes the list (since 10-25,
  // "align navigation and pane policy with scaled CSS"); Conversations opens it again.
  await openConversations(page);
  await expect(sidebar.getByRole('button', { name: 'Open Delivery plan, final', exact: true })).toBeVisible();
  await expect(page.locator('.chat-aside')).not.toContainText('Obsidian');
});

test('an open archived conversation restores by durable key after reload', async ({ page }) => {
  await installChatFixtures(page);
  await openChat(page);
  let sidebar = await openConversations(page, 'hollow');
  await sidebar.getByRole('button', { name: 'Open Codex delivery plan', exact: true }).click();
  await expect(page.locator('.chat-main')).toContainText('Codex transcript is ready.');
  sidebar = await openConversations(page, 'hollow');
  const row = sidebar.locator('.session-row').filter({ has: page.getByRole('button', { name: 'Open Codex delivery plan', exact: true }) });
  await row.getByRole('button', { name: 'Settle conversation', exact: true }).click();
  await expect(row).toHaveCount(0);
  await expect(page.locator('.chat-main')).toContainText('Codex transcript is ready.');

  await page.reload();
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await showChat(page);
  await expect(page.locator('.chat-main')).toContainText('Codex transcript is ready.');
  await expect(page.getByRole('heading', { name: 'Codex delivery plan', exact: true })).toBeVisible();
  sidebar = await openConversations(page, 'hollow');
  await expect(sidebar.getByRole('button', { name: 'Open Codex delivery plan', exact: true })).toHaveCount(0);
  expect(await callCount(page, 'read_agent_session')).toBe(1);
  expect(await callCount(page, 'read_agent_conversation')).toBe(1);
  expect(await callCount(page, 'read_session')).toBe(0);
  expect(await page.evaluate(() => ({
    agent: localStorage.getItem('chatAgent'), nativeId: localStorage.getItem('chatSessionId'), key: localStorage.getItem('chatAgentSessionKey'),
  }))).toEqual({ agent: 'codex', nativeId: 'codex-owned-thread', key: 's-20260907-cccc0001' });
  expect(await callCount(page, 'chat_send')).toBe(0);
});

test('late Hollow history cannot replace another provider or revive a new conversation', async ({ page }) => {
  await installChatFixtures(page, true);
  await openChat(page);
  let sidebar = await openConversations(page, 'hollow');
  await sidebar.getByRole('button', { name: 'Open Codex delivery plan', exact: true }).click();
  await expect.poll(() => callCount(page, 'read_agent_conversation')).toBe(1);
  sidebar = await openConversations(page, 'hollow');
  await sidebar.getByRole('button', { name: 'Open Claude design review', exact: true }).click();
  await expect(page.locator('.chat-main')).toContainText('Claude design transcript is ready.');
  await releaseReplies(page);
  await expect(page.locator('.chat-main')).not.toContainText('Codex transcript is ready.');
  expect(await page.evaluate(() => localStorage.getItem('chatAgent'))).toBe('claude');
  expect(await page.evaluate(() => localStorage.getItem('chatSessionId'))).toBe('claude-owned-session');
  sidebar = await openConversations(page, 'hollow');
  await sidebar.getByRole('button', { name: 'Open Codex delivery plan', exact: true }).click();
  await expect.poll(() => callCount(page, 'read_agent_conversation')).toBe(3);
  await page.locator('.library-conversations').getByRole('button', { name: 'New chat', exact: true }).click();
  await releaseReplies(page);
  await expect(page.locator('.chat-main .turn')).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem('chatAgentSessionKey'))).toBeNull();
  expect(await page.evaluate(() => localStorage.getItem('chatSessionId'))).toBeNull();
});

// ── Structured question cards (Paperclip PR 13) ──────────────────────────────
// Mock mode has no event bus, so a card is seeded through localStorage
// `mockPendingInteractions` and read by list_pending_interactions when the
// conversation opens. The seeded conversation is the fixture's Claude one.

const CARD_KEY = 's-20260907-cccc0002';
const CARD_NATIVE = 'claude-owned-session';

async function seedQuestionCard(page: Page, over: Record<string, unknown> = {}, extra: Record<string, string> = {}) {
  const card = {
    id: 'qi-00000000000000a1', sessionKey: CARD_KEY, turnOrd: 1, runId: 'claude-fixture-run', agent: 'claude',
    nativeSessionId: CARD_NATIVE, toolUseId: 'toolu-001', source: 'mcp', mode: 'questions',
    request: {
      mode: 'questions', supersedeOnUserMessage: true, title: 'Before I build the page',
      questions: [
        { id: 'layout', prompt: 'Which layout?', required: true, answerMode: 'single', allowCustom: false,
          options: [{ id: 'grid', label: 'Grid', recommended: true }, { id: 'list', label: 'List' }] },
        { id: 'notes', prompt: 'Anything else?', required: false, answerMode: 'text', allowCustom: false, options: [] },
      ],
    },
    targetPath: null, supersedeOnUserMessage: true, status: 'pending', statusReason: null, supersededBy: null,
    answer: null, createdAt: new Date().toISOString(), resolvedAt: null,
    ...over,
  };
  await page.addInitScript(({ card, extra, key, native }) => {
    localStorage.setItem('chatAgent', 'claude');
    localStorage.setItem('chatAgentSessionKey', key);
    localStorage.setItem('chatSessionId', native);
    localStorage.setItem('mockPendingInteractions', JSON.stringify([card]));
    for (const [name, value] of Object.entries(extra)) localStorage.setItem(name, value);
  }, { card, extra, key: CARD_KEY, native: CARD_NATIVE });
}

/** Docked over the message box since 2026-10-14 (`QuestionDock`). */
function questionCard(page: Page) {
  return page.locator('.question-dock:visible');
}

/** Answer the seeded card: Grid (which goes on), then the optional note. */
async function answerCard(page: Page) {
  const card = questionCard(page);
  await card.getByRole('option', { name: /Grid/ }).click();
  await expect(card.locator('.question-dock-count')).toHaveText('2 of 2');
}

test('a question card renders, submits once, and sends the answers as the next turn', async ({ page }) => {
  await seedQuestionCard(page);
  await installChatFixtures(page);
  await openChat(page);
  const card = questionCard(page);
  await expect(card).toBeVisible();
  await expect(card).toContainText('Before I build the page');
  await expect(card.locator('.question-card-badge')).toHaveText('Recommended');
  // Docked: the message box is hidden while the questions show.
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toBeHidden();

  await answerCard(page);
  expect(await callCount(page, 'resolve_interaction')).toBe(0);
  await card.getByRole('button', { name: 'Send answers', exact: true }).click();

  await expect.poll(() => callCount(page, 'chat_send')).toBe(1);
  expect(await callCount(page, 'resolve_interaction')).toBe(1);
  const resolveArgs = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastResolveArgs') ?? '{}'));
  expect(resolveArgs.id).toBe('qi-00000000000000a1');
  expect(resolveArgs.nativeSessionId).toBe(CARD_NATIVE);
  expect(resolveArgs.answer).toEqual({ answers: { layout: { selected: ['grid'] } } });
  const request = (await lastChatSend(page))!;
  const prompt = String(request.prompt);
  expect(prompt.startsWith('Answered questions')).toBe(true);
  expect(prompt).toContain('Which layout?\nGrid');
  expect(prompt).not.toContain('<page-context>');
  expect(prompt).not.toContain('<hollow-chat-intent>');
  expect(request.sessionId).toBe(CARD_NATIVE);
  expect(request.sessionKey).toBe(CARD_KEY);
  await expect(card).toHaveCount(0);
});

// 2026-11-04 (Alex, a Main check-in's card): the conversation had moved on
// to a fresh native session, and Send stayed disabled after every answer was
// picked. Now the first question says the answers go as a message, Send goes,
// and the message names each question in full (not its short header) in the
// conversation's own session.
test('a question card from another native session goes as an ordinary message', async ({ page }) => {
  await seedQuestionCard(page, {
    nativeSessionId: 'an-older-session',
    request: {
      mode: 'questions', supersedeOnUserMessage: true, title: 'Before I build the page',
      questions: [
        { id: 'layout', header: 'Layout', prompt: 'Which layout fits the launch page?', required: true, answerMode: 'single', allowCustom: false,
          options: [{ id: 'grid', label: 'Grid', recommended: true }, { id: 'list', label: 'List' }] },
        { id: 'notes', prompt: 'Anything else?', required: false, answerMode: 'text', allowCustom: false, options: [] },
      ],
    },
  });
  await installChatFixtures(page);
  await openChat(page);
  const card = questionCard(page);
  await expect(card).toBeVisible();
  await expect(card.locator('.question-dock-count')).toHaveText('1 of 2');
  await expect(card.locator('.question-card-as-message')).toContainText('your answers go as a message');
  await answerCard(page);
  const send = card.getByRole('button', { name: 'Send answers', exact: true });
  await expect(send).toBeEnabled();
  await send.click();
  await expect.poll(() => callCount(page, 'chat_send')).toBe(1);
  expect(await callCount(page, 'resolve_interaction')).toBe(1);
  const request = (await lastChatSend(page))!;
  const prompt = String(request.prompt);
  expect(prompt.startsWith('Answered questions')).toBe(true);
  expect(prompt).toContain('Which layout fits the launch page?\nGrid');
  expect(prompt).not.toContain('- Layout: Grid');
  expect(request.sessionId).toBe(CARD_NATIVE);
  await expect(card).toHaveCount(0);
});

test('dismissing a question card sends nothing', async ({ page }) => {
  await seedQuestionCard(page);
  await installChatFixtures(page);
  await openChat(page);
  const card = questionCard(page);
  await expect(card).toBeVisible();
  await card.getByRole('button', { name: 'Don’t answer', exact: true }).click();
  await expect(card).toHaveCount(0);
  expect(await callCount(page, 'dismiss_interaction')).toBe(1);
  expect(await callCount(page, 'chat_send')).toBe(0);
  expect(await callCount(page, 'resolve_interaction')).toBe(0);
});

test('archiving the open conversation hides its question card', async ({ page }) => {
  await seedQuestionCard(page);
  await installChatFixtures(page);
  await openChat(page);
  await expect(questionCard(page)).toBeVisible();
  const sidebar = await openConversations(page, 'hollow');
  const row = sidebar.locator('.session-row').filter({ has: page.getByRole('button', { name: 'Open Claude design review', exact: true }) });
  await row.getByRole('button', { name: 'Settle conversation', exact: true }).click();
  await expect.poll(() => callCount(page, 'organize_conversation')).toBe(1);
  await expect(questionCard(page)).toHaveCount(0);
  expect(await callCount(page, 'chat_send')).toBe(0);
});

test('answers survive a failed answer turn', async ({ page }) => {
  await seedQuestionCard(page, {}, {
    mockChatSendError: 'spawn claude: No such file or directory',
    mockReopenClosed: 'true',
  });
  await installChatFixtures(page);
  await openChat(page);
  const card = questionCard(page);
  await answerCard(page);
  await card.getByRole('button', { name: 'Send answers', exact: true }).click();

  await expect.poll(() => callCount(page, 'chat_send')).toBe(1);
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await expect.poll(async () => (await composer.inputValue()).startsWith('Answered questions')).toBe(true);
  expect(await composer.inputValue()).toContain('Which layout?\nGrid');
  await expect(page.locator('.chat-main .turn.user').filter({ hasText: 'Answered questions' })).toHaveCount(0);
  await expect(page.locator('.chat-main')).toContainText('Your answers are in the composer');
});

// Paperclip PR 14b: a stopped or interrupted turn offers a manual Continue,
// even when an EARLIER turn left an error notice after the transcript.
test('Continue resumes an interrupted turn once and never auto-sends', async ({ page }) => {
  await installChatFixtures(page, false, false, true);
  await openChat(page);
  const sidebar = await openConversations(page, 'hollow');
  await sidebar.getByRole('button', { name: 'Open Claude design review', exact: true }).click();
  const transcript = page.locator('.chat-main');
  await expect(transcript).toContainText('Checking the release branch.');
  await expect(transcript).toContainText('usage limit reached: Claude AI usage limit reached');
  const resume = page.getByRole('button', { name: 'Continue', exact: true });
  await expect(resume).toBeVisible();
  expect(await callCount(page, 'chat_send')).toBe(0);

  await resume.click();
  await expect.poll(() => callCount(page, 'chat_send')).toBe(1);
  const request = (await lastChatSend(page))!;
  expect(request.prompt).toBe('Your previous run was interrupted. Continue from where you left off.');
  expect(String(request.prompt)).not.toContain('<operator-context>');
  expect(String(request.prompt)).not.toContain('<conversation-context>');
  expect(request.sessionId).toBe('claude-owned-session');
  expect(request.sessionKey).toBe('s-20260907-cccc0002');
  // Queued and busy: no second Continue.
  await expect(resume).toHaveCount(0);
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toHaveValue('');
  expect(await callCount(page, 'chat_send')).toBe(1);
});

// Paperclip PR 14b (cross-m34 H4): a fresh native session clears the old id
// and the old session's question cards, which could never be answered now.
test('a session reset clears the native id and the old session\'s question cards', async ({ page }) => {
  await installEventBridge(page);
  await seedQuestionCard(page);
  await installChatFixtures(page);
  await openChat(page);
  await expect(questionCard(page)).toBeVisible();

  // Another conversation's reset changes nothing here.
  await emitChatEvent(page, { type: 'sessionReset', reason: 'configChanged', agent: 'claude', sessionKey: 's-20260907-cccc0001' });
  await expect(questionCard(page)).toBeVisible();
  expect(await page.evaluate(() => localStorage.getItem('chatSessionId'))).toBe(CARD_NATIVE);

  await emitChatEvent(page, { type: 'sessionReset', reason: 'resumeFailed', agent: 'claude', sessionKey: CARD_KEY });
  await expect(questionCard(page)).toHaveCount(0);
  await expect(page.locator('.chat-main')).toContainText('The previous Claude session could not be resumed, so Hollow started a fresh one');
  expect(await page.evaluate(() => localStorage.getItem('chatSessionId'))).toBeNull();
  expect(await callCount(page, 'chat_send')).toBe(0);
});

// Paperclip PR 14b review: a stop that landed before the agent spawned never
// offers Continue (its request never reached the native session); a stop of
// a running turn does.
test('Continue is not offered after a stop before spawn', async ({ page }) => {
  await installEventBridge(page);
  await seedQuestionCard(page);
  await installChatFixtures(page);
  await openChat(page);
  await expect(questionCard(page)).toBeVisible();
  const resume = page.getByRole('button', { name: 'Continue', exact: true });

  await emitChatEvent(page, { type: 'cancelled', stop: null, beforeSpawn: true });
  await expect(page.locator('.chat-main .turn.system').last()).toBeVisible();
  await expect(resume).toHaveCount(0);

  await emitChatEvent(page, { type: 'cancelled', stop: 'verified' });
  await expect(resume).toBeVisible();
  expect(await callCount(page, 'chat_send')).toBe(0);
});
