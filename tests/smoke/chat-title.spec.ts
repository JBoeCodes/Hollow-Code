import { test, expect, type Page } from './fixtures/test';
import { openChat } from './choose';

// The open chat's name above it, with its ⋯ and a double-click to rename in
// place (overnight 2026-11-07, D1), and ↑ in an empty box recalling this
// conversation's own messages first. Fixture IPC only; no agent is involved.
const cwd = (id: string) => `/Users/fixture/vault/workspaces/${id}`;
const workspace = (id: string, title: string) =>
  ({ id, title, status: 'active', path: `workspaces/${id}`, cwd: cwd(id), modified: '2026-09-26T12:00:00Z' });
const session = (key: string, title: string, ws: string, minutesAgo: number) => {
  const at = new Date(Date.now() - minutesAgo * 60_000).toISOString();
  return { key, title, agent: 'claude', harnessSessionId: null, cwd: cwd(ws), createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'none' };
};

async function seed(page: Page) {
  await page.addInitScript(() => {
    if (sessionStorage.getItem('titleSeeded')) return;
    sessionStorage.setItem('titleSeeded', '1');
    localStorage.removeItem('hollow.railCollapsed');
    localStorage.removeItem('hollow.workspace');
    localStorage.removeItem('chatHistory');
    localStorage.setItem('hollow.railWorkspaces', JSON.stringify(['alpha-1']));
  });
  await page.addInitScript(([workspaces, sessions]) => {
    if (sessionStorage.getItem('titleSeededData')) return;
    sessionStorage.setItem('titleSeededData', '1');
    localStorage.setItem('mockWorkspaces', JSON.stringify(workspaces));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify(sessions));
    const conversations: Record<string, unknown> = {};
    // Two questions each, so ↑ has an order to follow: the later one first.
    for (const s of sessions as any[]) conversations[s.key] = { summary: s, eventsRetained: true, turns: [
      { id: `${s.key}-u1`, role: 'user', text: `Asked first in ${s.title}`, tools: [], agent: 'claude', runId: `run-${s.key}-1`, startedAt: s.createdAt, transcriptIncomplete: false },
      { id: `${s.key}-a1`, role: 'assistant', text: `First answer for ${s.title}`, tools: [], agent: 'claude', runId: `run-${s.key}-1`, startedAt: s.createdAt, status: 'success', transcriptIncomplete: false },
      { id: `${s.key}-u2`, role: 'user', text: `Asked in ${s.title}`, tools: [], agent: 'claude', runId: `run-${s.key}-2`, startedAt: s.createdAt, transcriptIncomplete: false },
      { id: `${s.key}-a2`, role: 'assistant', text: `Answer for ${s.title}`, tools: [], agent: 'claude', runId: `run-${s.key}-2`, startedAt: s.createdAt, status: 'success', transcriptIncomplete: false },
    ] };
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify(conversations));
  }, [
    [workspace('alpha-1', 'Alpha')],
    [session('s-20260926-aaaa0001', 'Draft the launch memo', 'alpha-1', 5), session('s-20260926-aaaa0002', 'Review pricing', 'alpha-1', 180)],
  ] as const);
  await page.goto('/?mock=1');
}

/** The stored name of the launch memo's conversation (the sidebar lists no
 *  conversations since L8, 11-10, so the record is what follows a rename). */
const storedTitle = (page: Page) => page.evaluate(() => (JSON.parse(localStorage.getItem('mockWorkspaceSessions') ?? '[]') as { key: string; title: string }[]).find(s => s.key === 's-20260926-aaaa0001')?.title);
// In the top bar's one header row since the top-bar polish (#1492).
const row = (page: Page) => page.locator('.top-bar .chat-topbar-controls .chat-title-row');
const composer = (page: Page) => page.locator('.work-conversation .chat-composer textarea');

test.beforeEach(async ({}, info) => { test.skip(info.project.name.includes('narrow'), 'The rail tree collapses to icons on narrow screens'); });

test('the open chat shows its name, and a double-click renames it in place', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Draft the launch memo');
  await expect(row(page).locator('.chat-title')).toHaveText('Draft the launch memo');
  // Escape keeps the old name and puts the focus back in the box.
  await row(page).locator('.chat-title').dblclick();
  const field = row(page).getByRole('textbox', { name: 'Conversation name' });
  await expect(field).toBeFocused();
  await field.fill('Not this one');
  await field.press('Escape');
  await expect(field).toHaveCount(0);
  await expect(row(page).locator('.chat-title')).toHaveText('Draft the launch memo');
  await expect(composer(page)).toBeFocused();
  // Enter saves; the stored name follows.
  await row(page).locator('.chat-title').dblclick();
  await expect(field).toBeFocused();
  await field.fill('Launch memo, round two');
  await field.press('Enter');
  await expect(row(page).locator('.chat-title')).toHaveText('Launch memo, round two');
  await expect.poll(() => storedTitle(page)).toBe('Launch memo, round two');
  await expect(composer(page)).toBeFocused();
});

test('a rename that fails shows the failure, keeps the old name and still returns to the box', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Draft the launch memo');
  await expect(row(page).locator('.chat-title')).toHaveText('Draft the launch memo');
  await row(page).locator('.chat-title').dblclick();
  const field = row(page).getByRole('textbox', { name: 'Conversation name' });
  await expect(field).toBeFocused();
  await field.fill('Never saved');
  await page.evaluate(() => { window.__mockFailCommandOnce = 'rename_agent_session'; });
  await field.press('Enter');
  // The field closes at once (the rename runs behind it), the failure is said
  // in a toast, and both the title and the stored name keep the old one.
  await expect(page.locator('.snackbar').filter({ hasText: 'mock rename_agent_session failure' }).first()).toBeVisible();
  await expect(field).toHaveCount(0);
  await expect(row(page).locator('.chat-title')).toHaveText('Draft the launch memo');
  expect(await storedTitle(page)).toBe('Draft the launch memo');
  await expect(composer(page)).toBeFocused();
});

test('the ⋯ beside the name is the visible route to Rename… and the rest of the conversation menu', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Review pricing');
  await expect(row(page).locator('.chat-title')).toHaveText('Review pricing');
  await row(page).getByRole('button', { name: 'Actions for this conversation' }).click();
  const menu = page.locator('.ctx-menu');
  await expect(menu.getByRole('menuitem', { name: 'Move to project…', exact: true })).toBeVisible();
  await expect(menu.getByRole('menuitem', { name: 'Settle conversation', exact: true })).toBeVisible();
  await menu.getByRole('menuitem', { name: 'Rename…', exact: true }).click();
  const field = row(page).getByRole('textbox', { name: 'Conversation name' });
  await expect(field).toBeFocused();
  // Leaving the field saves, as the sidebar's does; an empty name changes nothing.
  await field.fill('   ');
  await composer(page).click();
  await expect(field).toHaveCount(0);
  await expect(row(page).locator('.chat-title')).toHaveText('Review pricing');
});

test('what is typed in the name survives a refresh of the sessions, and Enter saves it and returns to the box', async ({ page }) => {
  // The Work page re-reads the sessions every minute; a fake clock makes that one step.
  await page.clock.install();
  await seed(page);
  await openChat(page, 'Draft the launch memo');
  await expect(row(page).locator('.chat-title')).toHaveText('Draft the launch memo');
  await row(page).locator('.chat-title').dblclick();
  const field = row(page).getByRole('textbox', { name: 'Conversation name' });
  await expect(field).toBeFocused();
  await field.fill('Launch memo, half typed');
  // Something changed in this very conversation on disk (a new message at
  // 1:00), so the re-read hands the field a new session object.
  await page.evaluate(() => {
    const sessions = JSON.parse(localStorage.getItem('mockWorkspaceSessions') ?? '[]');
    for (const s of sessions) if (s.key === 's-20260926-aaaa0001') s.lastActivityAt = new Date(Date.now() + 3_600_000).toISOString();
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify(sessions));
  });
  await page.clock.fastForward(61_000);
  await expect.poll(() => page.evaluate(() => document.querySelector<HTMLInputElement>('.chat-title-input')?.value)).toBe('Launch memo, half typed');
  await expect(field).toBeFocused();
  await field.press('Enter');
  await expect(composer(page)).toBeFocused();
  await expect(row(page).locator('.chat-title')).toHaveText('Launch memo, half typed');
});

test('a half-typed name that comes back after the row left the list is not selected: the caret is at its end', async ({ page }) => {
  await page.clock.install();
  await seed(page);
  await openChat(page, 'Draft the launch memo');
  await expect(row(page).locator('.chat-title')).toHaveText('Draft the launch memo');
  await row(page).locator('.chat-title').dblclick();
  const field = row(page).getByRole('textbox', { name: 'Conversation name' });
  await expect(field).toBeFocused();
  await field.fill('Half typed name');
  // A re-read that finds the conversation gone from the list takes the row, and
  // the field in it, away; the next one that finds it again brings both back.
  const gone = () => page.evaluate(() => {
    const all = JSON.parse(localStorage.getItem('mockWorkspaceSessions') ?? '[]');
    localStorage.setItem('keptSessions', JSON.stringify(all));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify(all.filter((s: { key: string }) => s.key !== 's-20260926-aaaa0001')));
  });
  await gone();
  await page.clock.fastForward(61_000);
  await expect(page.locator('.chat-title-input')).toHaveCount(0);
  await page.evaluate(() => localStorage.setItem('mockWorkspaceSessions', localStorage.getItem('keptSessions')!));
  await page.clock.fastForward(61_000);
  await expect(field).toBeFocused();
  // Nothing is selected: the next key adds to the draft instead of replacing it.
  await expect.poll(() => page.evaluate(() => {
    const input = document.querySelector<HTMLInputElement>('.chat-title-input')!;
    return [input.value, input.selectionStart, input.selectionEnd];
  })).toEqual(['Half typed name', 15, 15]);
  await field.press('Enter');
  await expect(row(page).locator('.chat-title')).toHaveText('Half typed name');
});

test('↑ in an empty box recalls this conversation’s own messages first, newest first', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Draft the launch memo');
  await expect(page.locator('.work-conversation')).toContainText('Answer for Draft the launch memo');
  await composer(page).click();
  await composer(page).press('ArrowUp');
  await expect(composer(page)).toHaveValue('Asked in Draft the launch memo');
  // Again: the one before it in this conversation, not another chat's prompt.
  await composer(page).press('ArrowUp');
  await expect(composer(page)).toHaveValue('Asked first in Draft the launch memo');
  // Another conversation starts again from its own newest message, not from
  // the place the last one left off (no typing clears it: only the switch can).
  await openChat(page, 'Review pricing');
  await expect(page.locator('.work-conversation')).toContainText('Answer for Review pricing');
  await composer(page).click();
  await expect(composer(page)).toHaveValue('');
  await composer(page).press('ArrowUp');
  await expect(composer(page)).toHaveValue('Asked in Review pricing');
  // ↓ past the newest clears the box again.
  await composer(page).press('ArrowDown');
  await expect(composer(page)).toHaveValue('');
});

// A recalled message in the box is still text to edit (a Codex read): an arrow
// over a selected range collapses it and Shift extends it, so neither is a
// recall that swaps or wipes the message.
test('↑ and ↓ over a selected word, or with Shift, are the box’s own', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Draft the launch memo');
  await expect(page.locator('.work-conversation')).toContainText('Answer for Draft the launch memo');
  await composer(page).click();
  await composer(page).press('ArrowUp');
  await expect(composer(page)).toHaveValue('Asked in Draft the launch memo');
  const select = (from: number | 'end', to: number | 'end') => composer(page).evaluate(
    (el: HTMLTextAreaElement, [a, b]) => el.setSelectionRange(a === 'end' ? el.value.length : a, b === 'end' ? el.value.length : b),
    [from, to] as const,
  );
  await select(0, 5);
  await composer(page).press('ArrowDown');
  await expect(composer(page)).toHaveValue('Asked in Draft the launch memo');
  await select(3, 3);
  await composer(page).press('Shift+ArrowUp');
  await expect(composer(page)).toHaveValue('Asked in Draft the launch memo');
  // Plain ↓ at the end is still a recall step: past the newest clears the box.
  await select('end', 'end');
  await composer(page).press('ArrowDown');
  await expect(composer(page)).toHaveValue('');
});

// Text put in the box without typing (the Quote does it) leaves the recall's
// place behind: ↓ would wipe the message and the quote with it.
test('↓ after text was put in a recalled box without typing keeps it', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Draft the launch memo');
  await expect(page.locator('.work-conversation')).toContainText('Answer for Draft the launch memo');
  await composer(page).click();
  await composer(page).press('ArrowUp');
  await expect(composer(page)).toHaveValue('Asked in Draft the launch memo');
  await page.evaluate(async () => {
    const path = '/src/store/chatRuntime.ts';
    (await import(/* @vite-ignore */ path)).setChatInput('Asked in Draft the launch memo\n\n> a quoted passage');
  });
  await expect(composer(page)).toHaveValue(/a quoted passage/);
  await composer(page).evaluate((el: HTMLTextAreaElement) => { el.focus(); el.setSelectionRange(el.value.length, el.value.length); });
  await composer(page).press('ArrowDown');
  await expect(composer(page)).toHaveValue(/a quoted passage/);
  // ↑ from the top of that draft is the box's own too: it was never a recall.
  await composer(page).evaluate((el: HTMLTextAreaElement) => el.setSelectionRange(0, 0));
  await composer(page).press('ArrowUp');
  await expect(composer(page)).toHaveValue(/a quoted passage/);
});
