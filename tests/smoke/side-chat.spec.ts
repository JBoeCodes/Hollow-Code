import { test, expect, type Page } from './fixtures/test';
import { openChat } from './choose';

// t3code parity 7.6, the side chat pane: a button beside the chat's name opens a second chat
// beside the transcript; a question goes through `side_chat_send` (never `chat_send`, so the main
// chat's run, transcript and message box are not touched), and Close side chat puts it away.
// Fixture IPC only; no agent runs.
const cwd = '/Users/fixture/vault/workspaces/alpha-1';
const at = '2026-11-20T12:00:00Z';
const main = { key: 's-20261120-side0001', title: 'Where to host the API', agent: 'claude', harnessSessionId: null, cwd, createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 2, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'none' };

async function seed(page: Page) {
  await page.addInitScript(([m, c]) => {
    if (sessionStorage.getItem('sideChatSeeded')) return;
    sessionStorage.setItem('sideChatSeeded', '1');
    localStorage.removeItem('hollow.railCollapsed');
    localStorage.removeItem('hollow.workspace');
    localStorage.removeItem('hollow.sideChatOpen');
    localStorage.setItem('hollow.railWorkspaces', JSON.stringify(['alpha-1']));
    localStorage.setItem('mockWorkspaces', JSON.stringify([{ id: 'alpha-1', title: 'Alpha', status: 'active', path: 'workspaces/alpha-1', cwd: c, modified: '2026-09-26T12:00:00Z' }]));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify([m]));
    const mm = m as { key: string; createdAt: string };
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify({
      [mm.key]: { summary: m, eventsRetained: true, turns: [
        { id: 'u1', role: 'user', text: 'Where should I host the API?', tools: [], agent: 'claude', runId: 'r1', startedAt: mm.createdAt, transcriptIncomplete: false },
        { id: 'a1', role: 'assistant', text: 'Use Fly: it is closest to users.', tools: [], agent: 'claude', runId: 'r1', startedAt: mm.createdAt, status: 'success', transcriptIncomplete: false },
      ] },
    }));
  }, [main, cwd] as const);
  await page.goto('/?mock=1');
}

const calls = (page: Page, name: string) => page.evaluate(n => (window as unknown as { __mockApiCalls?: Record<string, number> }).__mockApiCalls?.[n] ?? 0, name);

test.beforeEach(async ({}, info) => { test.skip(info.project.name.includes('narrow'), 'The rail tree collapses to icons on narrow screens'); });

test('the side chat opens beside the chat, asks its own question and closes', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Where to host the API');
  await page.getByRole('button', { name: 'Open side chat' }).click();
  const pane = page.getByRole('complementary', { name: 'Side chat' });
  await expect(pane).toBeVisible();
  await expect(pane).toContainText('Reads this chat and answers. It can’t change files.');
  await pane.getByRole('textbox', { name: 'Side chat message' }).fill('Why Fly and not Render?');
  await pane.getByRole('button', { name: 'Send to side chat' }).click();
  await expect.poll(() => calls(page, 'side_chat_send')).toBe(1);
  await expect(pane).toContainText('Why Fly and not Render?');
  // The main chat's own send path was never used, and its transcript is as it was.
  expect(await calls(page, 'chat_send')).toBe(0);
  await expect(page.getByText('Use Fly: it is closest to users.')).toHaveCount(1);
  await page.getByRole('button', { name: 'Close side chat' }).first().click();
  await expect(pane).toHaveCount(0);
});
