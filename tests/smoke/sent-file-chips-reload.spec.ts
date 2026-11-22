import { expect, test, type Page } from './fixtures/test';
import { openChat } from './choose';

/**
 * B1a (2026-11-15, ENG-104): a file chip sent with a message is still on that
 * message after the chat is reloaded from the store. The store keeps the
 * person's words and, in `turns.attachments`, the files each turn carried;
 * `read_agent_conversation` returns them as the user turn's `files`
 * (`phone_access::attachments::fill_conversation`), and the bubble shows them
 * as chips (`chatHydrate.ts` `savedUserText`). Fixture IPC only; no agent runs.
 */

const cwd = '/Users/fixture/vault/workspaces/alpha-1';
const at = new Date(Date.now() - 5 * 60_000).toISOString();
const summary = { key: 's-20261115-b1a00001', title: 'Report review', agent: 'claude', harnessSessionId: null, cwd, createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'none' };

async function seed(page: Page) {
  await page.addInitScript(([s, c]) => {
    if (sessionStorage.getItem('b1aSeeded')) return;
    sessionStorage.setItem('b1aSeeded', '1');
    localStorage.removeItem('hollow.railCollapsed');
    localStorage.removeItem('hollow.workspace');
    localStorage.setItem('hollow.railWorkspaces', JSON.stringify(['alpha-1']));
    localStorage.setItem('mockWorkspaces', JSON.stringify([{ id: 'alpha-1', title: 'Alpha', status: 'active', path: 'workspaces/alpha-1', cwd: c, modified: '2026-09-26T12:00:00Z' }]));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify([s]));
    const sm = s as { key: string; createdAt: string };
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify({ [sm.key]: { summary: s, eventsRetained: true, turns: [
      { id: 'u1', role: 'user', text: 'Hello', tools: [], agent: 'claude', runId: 'r1', startedAt: sm.createdAt, transcriptIncomplete: false },
      { id: 'a1', role: 'assistant', text: 'Hi. What do you need?', tools: [], agent: 'claude', runId: 'r1', startedAt: sm.createdAt, status: 'success', transcriptIncomplete: false },
    ] } }));
  }, [summary, cwd] as const);
}

/** Drop a small PDF on the composer, as composer-drop.spec.ts does. */
async function dropPdf(page: Page, name: string) {
  await page.locator('.chat-composer').first().evaluate((el, name) => {
    const dt = new DataTransfer();
    dt.items.add(new File([new TextEncoder().encode('%PDF-1.4')], name, { type: 'application/pdf' }));
    el.dispatchEvent(new DragEvent('dragover', { dataTransfer: dt, bubbles: true, cancelable: true }));
    el.dispatchEvent(new DragEvent('drop', { dataTransfer: dt, bubbles: true, cancelable: true }));
  }, name);
}

test.beforeEach(async ({}, info) => { test.skip(info.project.name.includes('narrow'), 'The rail tree collapses to icons on narrow screens'); });

test('a file chip sent with a message is still there after the chat reloads', async ({ page }) => {
  await seed(page);
  await page.goto('/?mock=1');
  await openChat(page, 'Report review');
  await expect(page.locator('.chat-user-chip').filter({ hasText: 'Hello' })).toBeVisible();

  await dropPdf(page, 'report.pdf');
  await expect(page.locator('.chat-attachments .chat-attach-file')).toContainText('report.pdf');
  await page.getByRole('textbox', { name: 'Chat message' }).first().fill('Summarize the report');
  await page.getByRole('button', { name: 'Send', exact: true }).first().click();
  await expect(page.locator('.chat-user-chip').getByRole('button', { name: 'Open report.pdf' })).toBeVisible();

  // What the store returns for that turn: the words, and the file from its
  // `<attached-files>` line as `files`. The fixture IPC has no store, so the
  // spec writes the turn as `read_agent_conversation` would return it.
  await page.evaluate(([key, startedAt]) => {
    const prompt = localStorage.getItem('mockLastChatPrompt') ?? '';
    const line = /^1\. (.+) at (\/.+)$/m.exec(prompt.slice(prompt.lastIndexOf('<attached-files')))!;
    const all = JSON.parse(localStorage.getItem('mockWorkspaceConversations') ?? '{}');
    all[key].turns.push(
      { id: 'u2', role: 'user', text: 'Summarize the report', files: [{ name: line[1], path: line[2], size: 8 }], tools: [], agent: 'claude', runId: 'r2', startedAt, transcriptIncomplete: false },
      { id: 'a2', role: 'assistant', text: 'It covers the quarter.', tools: [], agent: 'claude', runId: 'r2', startedAt, status: 'success', transcriptIncomplete: false },
    );
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify(all));
  }, [summary.key, at] as const);

  await page.reload();
  await openChat(page, 'Report review');
  const sent = page.locator('.chat-user-chip').filter({ hasText: 'Summarize the report' });
  await expect(sent.getByRole('button', { name: 'Open report.pdf' })).toBeVisible();
  await expect(sent.getByRole('button', { name: 'Open report.pdf' })).toHaveAttribute('title', '/mock/vault/.hollow-attachments/report-1.pdf');
  await expect(sent).toContainText('8 B');
});
