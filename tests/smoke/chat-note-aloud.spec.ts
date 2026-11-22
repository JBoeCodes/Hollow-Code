import { test, expect, type Page } from './fixtures/test';
import { openChat as openChatTitled, openProjectFiles, paneCovers } from './choose';

// Save conversation as note and Read aloud (overnight 2026-11-07, D1). The
// Conversation tools' Save writes the whole conversation as a Markdown note
// the way a reply's Save as note does; a reply's Read aloud button (in More until 11-12) reads it aloud
// through the system voice (here a recording stand-in) and says Stop reading
// while it does. Fixture IPC only; no agent runs.
const cwd = '/Users/fixture/vault/workspaces/alpha-1';
const at = new Date(Date.now() - 5 * 60_000).toISOString();
const session = (key: string, title: string) => ({ key, title, agent: 'claude', harnessSessionId: null, cwd, createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 2, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'none' });
const summary = session('s-20261107-cccc0001', 'Where to host the API');
const other = session('s-20261107-cccc0002', 'Pricing page copy');
const REPLY = ['## Recommendation', '', 'Use **Fly** for the API: it is closest to users. See [the docs](https://fly.io/docs).', '', '```bash', 'fly deploy', '```', '', '- Cost is $12 a month', '- Setup takes 20 minutes'].join('\n');

async function seed(page: Page) {
  await page.addInitScript(([s, o, answer, c]) => {
    // A system voice that only records, so the page's calls are what is checked.
    // `speaking` is true from a piece's `speak` to its end; `quiet()` ends every
    // piece without an end event (what WebKit does after a cancel).
    const log = { spoken: [] as { text: string; ended: boolean; end: () => void }[], cancels: 0, quiet() { this.spoken.forEach(s => { s.ended = true; }); } };
    (window as unknown as { speechLog: typeof log }).speechLog = log;
    Object.defineProperty(window, 'speechSynthesis', { configurable: true, value: {
      speak: (u: { text: string; onend?: () => void }) => { const s = { text: u.text, ended: false, end: () => { s.ended = true; u.onend?.(); } }; log.spoken.push(s); },
      cancel: () => { log.cancels++; log.spoken.forEach(s => { s.ended = true; }); },
      get speaking() { return log.spoken.some(s => !s.ended); },
      get pending() { return false; },
      getVoices: () => [],
    } });
    if (sessionStorage.getItem('noteAloudSeeded')) return;
    sessionStorage.setItem('noteAloudSeeded', '1');
    localStorage.removeItem('hollow.railCollapsed');
    localStorage.removeItem('hollow.workspace');
    localStorage.removeItem('mockWorkspaceNotes');
    localStorage.setItem('hollow.railWorkspaces', JSON.stringify(['alpha-1']));
    localStorage.setItem('mockWorkspaces', JSON.stringify([{ id: 'alpha-1', title: 'Alpha', status: 'active', path: 'workspaces/alpha-1', cwd: c, modified: '2026-09-26T12:00:00Z' }]));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify([s, o]));
    const mk = (x: { createdAt: string }, ask: string) => [
      { id: 'u1', role: 'user', text: ask, tools: [], agent: 'claude', runId: 'r1', startedAt: x.createdAt, transcriptIncomplete: false },
      { id: 'a1', role: 'assistant', text: answer, tools: [], agent: 'claude', runId: 'r1', startedAt: x.createdAt, status: 'success', transcriptIncomplete: false },
      { id: 'u2', role: 'user', text: 'And what about cost?', tools: [], agent: 'claude', runId: 'r2', startedAt: x.createdAt, transcriptIncomplete: false },
      { id: 'a2', role: 'assistant', text: answer, tools: [], agent: 'claude', runId: 'r2', startedAt: x.createdAt, status: 'success', transcriptIncomplete: false },
    ];
    const sm = s as { key: string; createdAt: string };
    const om = o as { key: string; createdAt: string };
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify({
      [sm.key]: { summary: s, eventsRetained: true, turns: mk(sm, 'Where should I host the API?') },
      [om.key]: { summary: o, eventsRetained: true, turns: mk(om, 'Draft the pricing page copy') },
    }));
  }, [summary, other, REPLY, cwd] as const);
  await page.goto('/?mock=1');
}
// The row, not its ⋯ (which also names the chat once it is open).
const openChat = (page: Page, title: string) => openChatTitled(page, title);
const speech = (page: Page) => page.evaluate(() => {
  const l = (window as unknown as { speechLog: { spoken: { text: string }[]; cancels: number } }).speechLog;
  return { spoken: l.spoken.map(s => s.text), cancels: l.cancels };
});

test.beforeEach(async ({}, info) => { test.skip(info.project.name.includes('narrow'), 'The rail tree collapses to icons on narrow screens'); });

test('Save conversation as a note writes the whole conversation next to Copy conversation', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Where to host the API');
  await page.getByRole('button', { name: 'Actions for this conversation', exact: true }).click();
  const tools = page.getByRole('menu');
  await expect(tools.getByRole('menuitem', { name: 'Copy conversation (Markdown)' })).toBeVisible();
  await tools.getByRole('menuitem', { name: 'Save conversation as a note' }).click();
  const saved = page.locator('.snackbar').filter({ hasText: 'Saved as a note: workspaces/alpha-1/outputs/answers/' });
  await expect(saved.first()).toContainText(/\/outputs\/answers\/\d{4}-\d\d-\d\d-where-to-host-the-api\.md/);
  const notes = await page.evaluate(() => JSON.parse(localStorage.getItem('mockWorkspaceNotes') ?? '{}') as Record<string, string>);
  const path = Object.keys(notes).find(p => p.endsWith('-where-to-host-the-api.md'));
  expect(path).toBeTruthy();
  const body = notes[path!];
  expect(body).toMatch(/^# Where to host the API\n\n\*Saved from a conversation on \d{4}-\d\d-\d\d\.\*\n\n## You\n\nWhere should I host the API\?\n\n## Claude\n\n## Recommendation/);
  expect(body).toContain('And what about cost?');
  expect(body).toContain('```bash\nfly deploy\n```');
  // Saved again, it never overwrites: the next name.
  await page.getByRole('button', { name: 'Actions for this conversation', exact: true }).click();
  await tools.getByRole('menuitem', { name: 'Save conversation as a note' }).click();
  await expect(page.locator('.snackbar').filter({ hasText: /where-to-host-the-api-2\.md/ })).toBeVisible();
  // Open shows it in the Preview pane.
  await page.locator('.snackbar').filter({ hasText: /-2\.md/ }).getByRole('button', { name: 'Open', exact: true }).click();
  await expect(page.getByRole('complementary', { name: 'Preview', exact: true })).toBeVisible();
});

// The conversation's text, title and workspace are read at the click, not after
// the note module has loaded: another conversation opening meanwhile must not
// save its text under this one's name.
test('Save conversation as a note saves the conversation it was clicked in, though another opens while the note module loads', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Where to host the API');
  await page.getByRole('button', { name: 'Actions for this conversation', exact: true }).click();
  const tools = page.getByRole('menu');
  await expect(tools.getByRole('menuitem', { name: 'Save conversation as a note' })).toBeVisible();
  // The note module's request is held: the click waits for it.
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  await page.route(/replyNote/, async route => { await gate; await route.continue(); });
  await tools.getByRole('menuitem', { name: 'Save conversation as a note' }).click();
  await openChat(page, 'Pricing page copy');
  await expect(page.locator('.work-conversation')).toContainText('Draft the pricing page copy');
  release();
  await expect(page.locator('.snackbar').filter({ hasText: 'Saved as a note: workspaces/alpha-1/outputs/answers/' }).first()).toBeVisible();
  const notes = await page.evaluate(() => JSON.parse(localStorage.getItem('mockWorkspaceNotes') ?? '{}') as Record<string, string>);
  const path = Object.keys(notes).find(p => p.endsWith('-where-to-host-the-api.md'));
  expect(path, Object.keys(notes).join(', ')).toBeTruthy();
  expect(notes[path!]).toContain('Where should I host the API?');
  expect(notes[path!]).not.toContain('Draft the pricing page copy');
});

test('Read aloud reads a reply through the voice, shows Stop reading, and stops with the conversation', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Where to host the API');
  const reply = page.getByRole('group', { name: 'Reply details' }).last();
  await reply.hover();
  // Overnight 11-12 (C2): Read aloud is a button in the reply's row, not in More.
  await expect(reply.getByRole('button', { name: 'Stop reading' })).toHaveCount(0);
  await reply.getByRole('button', { name: 'Read aloud' }).click();
  // The words are read, not the markup, and the code block is named as skipped.
  await expect.poll(async () => (await speech(page)).spoken.length).toBeGreaterThan(0);
  const first = await speech(page);
  const said = first.spoken.join(' ');
  expect(said).toContain('Recommendation.');
  expect(said).toContain('Use Fly for the API: it is closest to users. See the docs.');
  expect(said).toContain('Code block skipped.');
  expect(said).toContain('Cost is $12 a month.');
  expect(said).not.toMatch(/[*#`]|https?:/);
  // Reading: the same button says Stop reading.
  const stop = reply.getByRole('button', { name: 'Stop reading' });
  await expect(stop).toBeVisible();
  // Stop reading cancels the voice.
  const before = (await speech(page)).cancels;
  await stop.click();
  await expect(stop).toHaveCount(0);
  expect((await speech(page)).cancels).toBeGreaterThan(before);
  // Read again, and let the voice finish: the button goes away on its own.
  await reply.hover();
  await reply.getByRole('button', { name: 'Read aloud' }).click();
  await expect(reply.getByRole('button', { name: 'Stop reading' })).toBeVisible();
  await page.evaluate(() => {
    const l = (window as unknown as { speechLog: { spoken: { end?: () => void }[] } }).speechLog;
    l.spoken.at(-1)?.end?.();
  });
  await expect(reply.getByRole('button', { name: 'Stop reading' })).toHaveCount(0);
  // Reading, then opening another conversation stops the voice.
  await reply.hover();
  await reply.getByRole('button', { name: 'Read aloud' }).click();
  await expect(reply.getByRole('button', { name: 'Stop reading' })).toBeVisible();
  const cancels = (await speech(page)).cancels;
  await openChat(page, 'Pricing page copy');
  await expect.poll(async () => (await speech(page)).cancels).toBeGreaterThan(cancels);
  await expect(page.getByRole('button', { name: 'Stop reading' })).toHaveCount(0);
});

// The Grok review of #1195. A reply's Read aloud button (in More until 11-12).
async function readReply(_page: Page, reply: ReturnType<Page['locator']>) {
  await reply.hover();
  await reply.getByRole('button', { name: 'Read aloud' }).click();
}

test('a read whose end is never reported still ends, as WebKit needs', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Where to host the API');
  const reply = page.getByRole('group', { name: 'Reply details' }).last();
  await readReply(page, reply);
  await expect(reply.getByRole('button', { name: 'Stop reading' })).toBeVisible();
  // The voice goes quiet with no end event: three idle looks 400 ms apart end it.
  await page.evaluate(() => (window as unknown as { speechLog: { quiet: () => void } }).speechLog.quiet());
  await expect(reply.getByRole('button', { name: 'Stop reading' })).toHaveCount(0, { timeout: 5000 });
});

test('reading a second reply while the first speaks cancels once, waits a moment, then speaks and moves Stop reading', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Where to host the API');
  const replies = page.getByRole('group', { name: 'Reply details' });
  await readReply(page, replies.last());
  await expect(replies.last().getByRole('button', { name: 'Stop reading' })).toBeVisible();
  const before = await speech(page);
  await readReply(page, replies.first());
  // The row moves at once; the voice's new pieces follow the cancel.
  await expect(replies.first().getByRole('button', { name: 'Stop reading' })).toBeVisible();
  await expect(replies.last().getByRole('button', { name: 'Stop reading' })).toHaveCount(0);
  await expect.poll(async () => (await speech(page)).spoken.length).toBeGreaterThan(before.spoken.length);
  expect((await speech(page)).cancels).toBe(before.cancels + 1);
  // Its own end clears its row.
  await page.evaluate(() => {
    const l = (window as unknown as { speechLog: { spoken: { end: () => void }[] } }).speechLog;
    l.spoken.at(-1)?.end();
  });
  await expect(page.getByRole('button', { name: 'Stop reading' })).toHaveCount(0);
});

test('a Preview pane that covers the chat stops the voice', async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 900 });
  await seed(page);
  await openChat(page, 'Where to host the API');
  const reply = page.getByRole('group', { name: 'Reply details' }).last();
  await readReply(page, reply);
  await expect(reply.getByRole('button', { name: 'Stop reading' })).toBeVisible();
  const before = (await speech(page)).cancels;
  await openProjectFiles(page);
  expect(await paneCovers(page), 'a 1024px window puts the Preview pane over the chat').toBe(true);
  await expect.poll(async () => (await speech(page)).cancels).toBeGreaterThan(before);
});

test('Stop reading is also in the chat’s top row, and leaving the page stops the voice', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Where to host the API');
  const reply = page.getByRole('group', { name: 'Reply details' }).last();
  await readReply(page, reply);
  const top = page.locator('.chat-topbar-controls').getByRole('button', { name: 'Stop reading' });
  await expect(top).toBeVisible();
  const before = (await speech(page)).cancels;
  await top.click();
  await expect(top).toHaveCount(0);
  await expect(reply.getByRole('button', { name: 'Stop reading' })).toHaveCount(0);
  expect((await speech(page)).cancels).toBeGreaterThan(before);
  // Reading again, then opening Tasks (the Work page stays mounted behind it): the voice stops.
  await readReply(page, reply);
  await expect(top).toBeVisible();
  const reading = (await speech(page)).cancels;
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Tasks', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Tasks', level: 1 })).toBeVisible();
  await expect.poll(async () => (await speech(page)).cancels).toBeGreaterThan(reading);
});
