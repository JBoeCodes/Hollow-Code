import { test, expect, type Page } from './fixtures/test';
import { openChat } from './choose';

// CH1 (2026-11-18), Branch from here: a finished reply's More actions offers
// it; it asks first (nothing is made on Cancel), branches through
// `branch_chat` and opens the branch, whose notice names the original and has
// Back to the original. Fixture IPC only; no agent runs.
const cwd = '/Users/fixture/vault/workspaces/alpha-1';
const at = '2026-11-18T12:00:00Z';
const session = (key: string, title: string, extra: Record<string, unknown> = {}) => ({ key, title, agent: 'claude', harnessSessionId: null, cwd, createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 2, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'none', ...extra });
const original = session('s-20261118-bbbb0001', 'Where to host the API');
const branch = session('s-20261118-bbbb0002', 'Where to host the API (branch)', { parentKey: original.key, forkKind: 'branch' });

async function seed(page: Page) {
  await page.addInitScript(([o, b, c]) => {
    if (sessionStorage.getItem('branchSeeded')) return;
    sessionStorage.setItem('branchSeeded', '1');
    localStorage.removeItem('hollow.railCollapsed');
    localStorage.removeItem('hollow.workspace');
    localStorage.setItem('hollow.railWorkspaces', JSON.stringify(['alpha-1']));
    localStorage.setItem('mockWorkspaces', JSON.stringify([{ id: 'alpha-1', title: 'Alpha', status: 'active', path: 'workspaces/alpha-1', cwd: c, modified: '2026-09-26T12:00:00Z' }]));
    const ob = o as { key: string; createdAt: string };
    const bb = b as { key: string; createdAt: string };
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify([o]));
    localStorage.setItem('mockBranchKey', bb.key);
    localStorage.setItem('mockBranchInfo', JSON.stringify({ [bb.key]: { key: bb.key, fromKey: ob.key, fromTitle: 'Where to host the API', through: 1, checkpoint: null, ownTurns: 0 } }));
    const turns = (x: { createdAt: string }) => [
      { id: 'u1', role: 'user', text: 'Where should I host the API?', tools: [], agent: 'claude', runId: 'r1', startedAt: x.createdAt, transcriptIncomplete: false },
      { id: 'a1', role: 'assistant', text: 'Use Fly: it is closest to users.', tools: [], agent: 'claude', runId: 'r1', startedAt: x.createdAt, status: 'success', transcriptIncomplete: false },
    ];
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify({
      [ob.key]: { summary: o, eventsRetained: true, turns: turns(ob) },
      [bb.key]: { summary: b, eventsRetained: true, turns: turns(bb) },
    }));
    // `branch_chat` makes the branch: the test adds it to the session list.
    (window as unknown as { __addBranch: () => void }).__addBranch = () => localStorage.setItem('mockWorkspaceSessions', JSON.stringify([o, b]));
  }, [original, branch, cwd] as const);
  await page.goto('/?mock=1');
}

test.beforeEach(async ({}, info) => { test.skip(info.project.name.includes('narrow'), 'The rail tree collapses to icons on narrow screens'); });

test('Branch from here asks first, and Cancel makes nothing', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Where to host the API');
  const reply = page.getByRole('group', { name: 'Reply details' }).last();
  await reply.hover();
  await reply.getByRole('button', { name: 'More actions' }).click();
  await page.getByRole('menuitem', { name: 'Branch from here…' }).click();
  const ask = page.getByRole('dialog', { name: 'Branch from this reply?' });
  await expect(ask).toContainText('The original stays as it is');
  await ask.getByRole('button', { name: 'Cancel' }).click();
  await expect(ask).toHaveCount(0);
  expect(await page.evaluate(() => (window as unknown as { __mockApiCalls?: Record<string, number> }).__mockApiCalls?.branch_chat ?? 0)).toBe(0);
});

test('Branch from here confirms, branches and opens the branch with its notice', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Where to host the API');
  const reply = page.getByRole('group', { name: 'Reply details' }).last();
  await reply.hover();
  await reply.getByRole('button', { name: 'More actions' }).click();
  // The backend adds the branch to the session list as it makes it.
  await page.evaluate(() => (window as unknown as { __addBranch: () => void }).__addBranch());
  await page.getByRole('menuitem', { name: 'Branch from here…' }).click();
  await page.getByRole('dialog', { name: 'Branch from this reply?' }).getByRole('button', { name: 'Branch', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { __mockApiCalls?: Record<string, number> }).__mockApiCalls?.branch_chat ?? 0)).toBe(1);
  const notice = page.getByRole('status', { name: 'Where this chat branched from' });
  await expect(notice).toContainText('A branch of “Where to host the API” after turn 1');
  await expect(notice.getByRole('button', { name: 'Back to the original' })).toBeVisible();
});
