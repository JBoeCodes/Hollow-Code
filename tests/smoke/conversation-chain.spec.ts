import { expect, test, type Page } from './fixtures/test';

/**
 * One conversation from the person's side (2026-11-07, Alex: "Why are
 * there two Main agent chats?"). A chat that continued elsewhere (a
 * handover, or a Main agent conversation that isn't the newest) shows a
 * plain banner with Open where its composer was, and the messages that
 * moved; the current chat folds the chats before it under "Earlier in this
 * conversation", and their approval and question cards show in it.
 *
 * The chain is seeded through the mock's `mockChains` for the open
 * conversation ("Wire the session store"), then read again.
 */

const composer = (page: Page) => page.getByRole('textbox', { name: 'Chat message' });

async function openConversation(page: Page): Promise<string> {
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
  return page.evaluate(async () => (await import('/src/store/chatRuntime.ts')).chatAgentSessionKey() as string);
}

/** Seed the open conversation's chain and read it again. */
async function seedChain(page: Page, chain: (key: string) => unknown) {
  const key = await page.evaluate(async () => (await import('/src/store/chatRuntime.ts')).chatAgentSessionKey() as string);
  await page.evaluate(async ([k, view]) => {
    localStorage.setItem('mockChains', JSON.stringify({ [k]: view }));
    await (await import('/src/components/ChatChainBar.tsx')).refreshChatChain();
  }, [key, chain(key)] as const);
}

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name.includes('narrow'), 'Conversation history opens from the desktop settings index');
  await page.goto('/?mock=1');
});

test('a conversation that continued elsewhere shows where, in place of its composer', async ({ page }) => {
  await openConversation(page);
  await seedChain(page, key => ({
    key, current: 's-20261107-heir', currentTitle: 'COO, continued', main: true, earlier: [],
    forwards: [{ from: key, to: 's-20261107-heir', at: '2026-11-07T21:18:00Z', preview: 'Install' }],
  }));
  const banner = page.locator('.chat-continued');
  await expect(banner).toContainText('This conversation continued in COO, continued.');
  await expect(banner.getByRole('button', { name: 'Open', exact: true })).toBeVisible();
  await expect(banner).toContainText('“Install” was sent here after this conversation continued, so it went there');
  await expect(composer(page)).toBeHidden();
});

test("the current conversation folds the earlier chats and shows their cards", async ({ page }) => {
  const key = await openConversation(page);
  await seedChain(page, k => ({
    key: k, current: k, currentTitle: 'Wire the session store', main: true,
    earlier: [{ key: 's-20261107-morning', title: 'Morning thread', createdAt: '2026-11-07T13:00:00Z' }],
    forwards: [{ from: 's-20261107-morning', to: k, at: '2026-11-07T19:04:00Z', preview: 'Install' }],
  }));
  const earlier = page.locator('.chat-earlier');
  await expect(earlier.locator('summary')).toHaveText('Earlier in this conversation (1 chat)');
  await earlier.locator('summary').click();
  await expect(earlier.getByRole('button', { name: 'Morning thread', exact: true })).toBeVisible();
  await expect(earlier).toContainText('was sent in an earlier chat after it continued here, so it came here');
  await expect(composer(page)).toBeVisible();

  // An approval asked in the earlier chat shows here; another chat's doesn't.
  await page.evaluate(async () => {
    const { setPendingApprovals } = await import('/src/store/chatApprovals.ts');
    setPendingApprovals([
      { id: 'appr-earlier', title: 'Computer control', action: 'Quit Safari', kind: 'computer', sessionKey: 's-20261107-morning' },
      { id: 'appr-other', title: 'Computer control', action: 'Quit Mail', kind: 'computer', sessionKey: 's-20261107-elsewhere' },
    ]);
  });
  await expect(page.locator('.chat-approval')).toHaveCount(1);
  await expect(page.locator('.chat-approval')).toContainText('Quit Safari');

  // A question card asked in the earlier chat docks here.
  await page.evaluate(async () => {
    const card = {
      id: 'int-chain-1', sessionKey: 's-20261107-morning', turnOrd: 1, runId: 'run-chain', agent: 'claude',
      nativeSessionId: 'native-old', toolUseId: 'tu-1', source: 'mcp', mode: 'questions', status: 'pending',
      request: { mode: 'questions', title: 'Check-in', supersedeOnUserMessage: true, questions: [
        { id: 'go', header: 'Install', prompt: 'Install main now?', required: true, answerMode: 'single', allowCustom: false, options: [{ id: 'do-it', label: 'Do it' }, { id: 'not-now', label: 'Not now' }] },
      ] },
      answer: null, createdAt: '2026-11-07T19:00:00Z', resolvedAt: null,
    };
    localStorage.setItem('mockPendingInteractions', JSON.stringify([card]));
    await (await import('/src/store/chatRuntime.ts')).refreshChatInteractions();
  });
  await expect(page.locator('.question-dock')).toContainText('Install main now?');
  expect(key).toBeTruthy();
});
