import { test, expect, type Page } from './fixtures/test';
import { chooseNew, openChat as openChatTitled } from './choose';

// Code blocks and tables in a reply (overnight 2026-11-07, D1): a Wrap toggle
// that holds for every block and is remembered, and tables that scroll in
// their own box and copy as text plus HTML. Fixture IPC only; no agent runs.
const cwd = '/Users/fixture/vault/workspaces/alpha-1';
const at = new Date(Date.now() - 5 * 60_000).toISOString();
const summary = { key: 's-20261107-bbbb0001', title: 'Where to host the API', agent: 'claude', harnessSessionId: null, cwd, createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'none' };
const LONG = 'cd ~/Hollow/Worktrees/hollow-release/ri27-n7-d1-blocks && CARGO_TARGET_DIR=~/Library/Caches/hollow-release-target npm run tauri:install -- --with-a-very-long-tail-of-arguments --and-another-one-after-it --and-a-third-for-good-measure';

async function seed(page: Page, reply: string) {
  await page.addInitScript(([s, text, c]) => {
    if (sessionStorage.getItem('blocksSeeded')) return;
    sessionStorage.setItem('blocksSeeded', '1');
    localStorage.removeItem('hollow.codeWrap');
    localStorage.removeItem('hollow.railCollapsed');
    localStorage.removeItem('hollow.workspace');
    localStorage.setItem('hollow.railWorkspaces', JSON.stringify(['alpha-1']));
    localStorage.setItem('mockWorkspaces', JSON.stringify([{ id: 'alpha-1', title: 'Alpha', status: 'active', path: 'workspaces/alpha-1', cwd: c, modified: '2026-09-26T12:00:00Z' }]));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify([s]));
    const sm = s as { key: string; createdAt: string };
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify({ [sm.key]: { summary: s, eventsRetained: true, turns: [
      { id: 'u1', role: 'user', text: 'Show me', tools: [], agent: 'claude', runId: 'r1', startedAt: sm.createdAt, transcriptIncomplete: false },
      { id: 'a1', role: 'assistant', text, tools: [], agent: 'claude', runId: 'r1', startedAt: sm.createdAt, status: 'success', transcriptIncomplete: false },
    ] } }));
  }, [summary, reply, cwd] as const);
  await page.goto('/?mock=1');
}
// The row, not its ⋯ (which also names the chat once it is open); a click on the open one is harmless.
const openChat = (page: Page) => openChatTitled(page, 'Where to host the API');
const overflows = (el: import('@playwright/test').Locator) => el.evaluate(e => e.scrollWidth > e.clientWidth + 1);

test.beforeEach(async ({}, info) => { test.skip(info.project.name.includes('narrow'), 'The rail tree collapses to icons on narrow screens'); });

test('Wrap turns long code lines into wrapped ones in every block, and is remembered', async ({ page }) => {
  await seed(page, ['First:', '', '```bash', LONG, '```', '', 'Second:', '', '```bash', LONG, '```'].join('\n'));
  await openChat(page);
  const blocks = page.locator('.work-conversation .chat-codeblock');
  await expect(blocks).toHaveCount(2);
  const pre = (i: number) => blocks.nth(i).locator('pre');
  const toggle = (i: number) => blocks.nth(i).getByRole('button', { name: 'Wrap long lines' });
  // Scrolling is the default: the long line runs past the box.
  expect(await overflows(pre(0))).toBe(true);
  await expect(toggle(0)).toHaveAttribute('aria-pressed', 'false');
  // One click wraps every block, and the line is still whole.
  await toggle(0).click();
  await expect(toggle(0)).toHaveAttribute('aria-pressed', 'true');
  await expect(toggle(1)).toHaveAttribute('aria-pressed', 'true');
  expect(await overflows(pre(0))).toBe(false);
  expect(await overflows(pre(1))).toBe(false);
  await expect(pre(0)).toContainText('--and-a-third-for-good-measure');
  // Copy still takes the code as written (one line, no wrapping newlines).
  await page.evaluate(() => { (window as unknown as { copied: string[] }).copied = []; Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async (v: string) => { (window as unknown as { copied: string[] }).copied.push(v); } } }); });
  await blocks.nth(0).getByRole('button', { name: 'Copy bash block' }).click();
  expect(await page.evaluate(() => (window as unknown as { copied: string[] }).copied)).toEqual([LONG]);
  // Remembered: after a reload the blocks are wrapped before anyone clicks.
  await page.reload();
  await openChat(page);
  await expect(blocks).toHaveCount(2);
  await expect(toggle(0)).toHaveAttribute('aria-pressed', 'true');
  expect(await overflows(pre(0))).toBe(false);
  // Off again: back to scrolling, and the setting is gone from storage.
  await toggle(1).click();
  await expect(toggle(0)).toHaveAttribute('aria-pressed', 'false');
  expect(await overflows(pre(0))).toBe(true);
  expect(await page.evaluate(() => localStorage.getItem('hollow.codeWrap'))).toBeNull();
});

test('a table in a reply scrolls in its own box and copies as text and as HTML', async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 900 });
  const note = 'Closest region picks itself and scale to zero is on the paid plan, with a slower cold start below the starter tier';
  await seed(page, [
    'A small one:', '',
    '| Option | Monthly cost | Setup time |', '|---|---:|---|',
    '| Fly | $12 | 20 minutes |', '| Render | $19 | 10 minutes<br>no card |',
    '| Koyeb | $9 | up to<ul><li>5 minutes</li><li>no card</li></ul> |', '',
    'A wide one:', '',
    '| Region | Provider | Plan | vCPU | Memory | Disk | Egress | p50 latency | p95 latency | Cold start | Monthly cost | Notes |',
    '|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|',
    `| us-east | Fly | shared-1x | 1 | 256 MB | 3 GB | 100 GB | 38 ms | 91 ms | 0.4 s | $12 | ${note} |`,
    `| eu-west | Render | starter | 0.5 | 512 MB | 1 GB | 100 GB | 44 ms | 120 ms | 2.1 s | $19 | ${note} |`,
  ].join('\n'));
  await openChat(page);
  const boxes = page.locator('.work-conversation .reply-table');
  await expect(boxes).toHaveCount(2);
  // The box has its own border and a named, focusable scroll region.
  const small = boxes.nth(0), wide = boxes.nth(1);
  await expect(small.getByRole('region', { name: 'Table 1 of 2' })).toBeVisible();
  await expect(wide.getByRole('region', { name: 'Table 2 of 2' })).toBeVisible();
  // The wide one scrolls inside its box; the page and the message do not.
  const scroll = wide.locator('.reply-table-scroll');
  expect(await overflows(scroll)).toBe(true);
  expect(await overflows(small.locator('.reply-table-scroll'))).toBe(false);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  const msgRight = await wide.evaluate(el => el.closest('.chat-msg')!.getBoundingClientRect().right);
  expect((await wide.boundingBox())!.x + (await wide.boundingBox())!.width).toBeLessThanOrEqual(msgRight + 1);
  // The header's rule stays (its row is also a tr:last-child); the last body row has none.
  const rule = (sel: string) => wide.locator(sel).first().evaluate(e => getComputedStyle(e).borderBottomWidth);
  expect(await rule('thead th')).toBe('1px');
  expect(await rule('tbody tr:first-child td')).toBe('1px');
  expect(await rule('tbody tr:last-child td')).toBe('0px');
  // A long cell wraps; a short one stays on one line.
  await expect(wide.locator('td.long').first()).toContainText('Closest region');
  await expect(wide.locator('td', { hasText: 'shared-1x' })).not.toHaveClass(/long/);
  // The keyboard can scroll it.
  await scroll.focus();
  await page.keyboard.press('ArrowRight');
  await expect.poll(() => scroll.evaluate(e => e.scrollLeft)).toBeGreaterThan(0);
  // Copy table: the clipboard gets tab-separated text and the HTML table.
  await page.evaluate(() => {
    const clips: Record<string, string>[] = [];
    (window as unknown as { clips: typeof clips }).clips = clips;
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: {
      writeText: async (v: string) => { clips.push({ 'text/plain': v }); },
      write: async (items: ClipboardItem[]) => { for (const item of items) { const one: Record<string, string> = {}; for (const type of item.types) one[type] = await (await item.getType(type)).text(); clips.push(one); } },
    } });
  });
  const copy = small.getByRole('button', { name: 'Copy table 1 of 2' });
  await copy.click();
  await expect(copy).toHaveText('Copied');
  const clips = await page.evaluate(() => (window as unknown as { clips: Record<string, string>[] }).clips);
  expect(clips).toHaveLength(1);
  // A line break inside a cell, and a block's start and end (words before a list, its items), read as a space, not as nothing ("10 minutesno card").
  expect(clips[0]['text/plain']).toBe('Option\tMonthly cost\tSetup time\nFly\t$12\t20 minutes\nRender\t$19\t10 minutes no card\nKoyeb\t$9\tup to 5 minutes no card');
  expect(clips[0]['text/html']).toMatch(/^<table>[\s\S]*<th[^>]*>Option<\/th>[\s\S]*<td[^>]*>Fly<\/td>/);
  // The label comes back, ready for the next copy.
  await expect(copy).toHaveText('Copy table');
  // The second table is its own copy (its twelve columns, header first).
  await wide.getByRole('button', { name: 'Copy table 2 of 2' }).click();
  await expect.poll(async () => (await page.evaluate(() => (window as unknown as { clips: Record<string, string>[] }).clips)).length).toBe(2);
  const second = (await page.evaluate(() => (window as unknown as { clips: Record<string, string>[] }).clips))[1]['text/plain'].split('\n');
  expect(second).toHaveLength(3);
  expect(second[0].split('\t')).toHaveLength(12);
  expect(second[2].split('\t').at(-1)).toBe(note);
});

// A second copy inside the Copied window keeps the label for its own full time:
// the first copy's timer must not cut it short (independent read, 11-07).
test('copying a table again inside the Copied window keeps the label for its own full time', async ({ page }) => {
  // The page's clock moves only when the test says so, so the two timers
  // overlap exactly as written whatever the machine's speed.
  await page.clock.install();
  await seed(page, '| Host | Cost |\n|---|---|\n| Fly | $12 |');
  await openChat(page);
  await page.evaluate(() => {
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async () => {}, write: async () => {} } });
  });
  const copy = page.locator('.work-conversation button.reply-table-copy');
  await expect(copy).toHaveCount(1);
  await page.clock.pauseAt(new Date(Date.now() + 5_000));
  await copy.click();
  await expect(copy).toHaveText('Copied');
  // Copied again 0.8 s later: the first copy's timer ends at 1.2 s, the second's at 2.0 s.
  await page.clock.runFor(800);
  await copy.click();
  await expect(copy).toHaveText('Copied');
  // At 1.5 s the first timer is past (it would put "Copy table" back), the second not yet.
  await page.clock.runFor(700);
  await expect(copy).toHaveText('Copied');
  // At 2.1 s the label does come back.
  await page.clock.runFor(600);
  await expect(copy).toHaveText('Copy table');
});

// Since the top-bar polish (#1492) the chat's name sits in the top bar's one
// header row, not above the thread: at every width it stays visible there,
// on one line with its ⋯ (the visible route to Rename… and the rest).
test('the chat name sits in the top bar row with its ⋯ at every width', async ({ page }) => {
  await seed(page, 'A short reply.');
  await openChat(page);
  const row = page.locator('.top-bar .chat-topbar-controls .chat-title-row');
  await expect(row.locator('.chat-title')).toHaveText('Where to host the API');
  await expect(page.locator('.work-conversation .chat-title')).toHaveCount(0);
  for (const width of [1400, 1100, 900, 700, 500]) {
    await page.setViewportSize({ width, height: 800 });
    await expect(row.locator('.chat-title'), `${width}px wide`).toBeVisible();
    const at = await page.evaluate(() => {
      const box = (s: string) => document.querySelector(s)!.getBoundingClientRect();
      const bar = box('.top-bar'), title = box('.top-bar .chat-title'), more = box('.top-bar .chat-title-row .chat-actions-menu');
      return { barTop: bar.top, barBottom: bar.bottom, titleTop: title.top, titleBottom: title.bottom, titleWidth: title.width, titleMid: (title.top + title.bottom) / 2, moreMid: (more.top + more.bottom) / 2, moreRight: more.right, view: window.innerWidth };
    });
    expect(at.titleTop, `${width}px wide: name inside the top bar`).toBeGreaterThanOrEqual(at.barTop);
    expect(at.titleBottom, `${width}px wide: name inside the top bar`).toBeLessThanOrEqual(at.barBottom);
    expect(at.titleWidth, `${width}px wide: some of the name shows`).toBeGreaterThan(40);
    expect(Math.abs(at.titleMid - at.moreMid), `${width}px wide: the ⋯ on the name's line`).toBeLessThan(2);
    expect(at.moreRight, `${width}px wide: the ⋯ in the window`).toBeLessThanOrEqual(at.view);
  }
});

/** Tauri event delivery on top of the `?mock=1` IPC (as canvas-frame-budget.spec.ts). */
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
const emit = (page: Page, payload: Record<string, unknown>) => page.evaluate(event => {
  (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void }).__emitTauriEvent('chat-event', event);
}, payload);

test('a table keeps where it was scrolled while its reply streams on, and when the reply ends', async ({ page }) => {
  test.setTimeout(60_000);
  await page.setViewportSize({ width: 900, height: 900 });
  await installEventBridge(page);
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Compare hosts.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Hosts');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Hosts');
  const composer = page.getByLabel('Chat message');
  await composer.fill('Compare the hosts');
  await composer.press('Meta+Enter');
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBe(1);
  await emit(page, { type: 'runStarted', asked: 'Compare the hosts', runId: 'run-hosts', recall: [] });
  const cells = (n: number, v: string) => Array.from({ length: n }, (_, i) => `${v}${i + 1}`);
  const table = [`| ${cells(14, 'Column ').join(' | ')} |`, `|${'---|'.repeat(14)}`, `| ${cells(14, 'a-long-value-').join(' | ')} |`].join('\n');
  await emit(page, { type: 'assistantDelta', text: `Here is the table:\n\n${table}\n\nThe first word` });
  const scroll = page.locator('.work-conversation .reply-table-scroll');
  await expect(scroll).toBeVisible();
  const left = () => scroll.evaluate(e => e.scrollLeft);
  expect(await scroll.evaluate(e => e.scrollWidth > e.clientWidth + 200)).toBe(true);
  // Scroll it, and mark this element so a rebuild would show.
  await scroll.evaluate(e => { e.scrollLeft = 180; e.dataset.first = '1'; });
  await expect.poll(left).toBeGreaterThan(150);
  const was = await left();
  // The reply grows after the table: the table's block is kept as it was
  // (gap 8: the marker stays) and so is where it was scrolled.
  await emit(page, { type: 'assistantDelta', text: ' and then more words after the table.' });
  await expect(page.getByText('and then more words after the table.')).toBeVisible();
  await expect(page.locator('.work-conversation .reply-table-scroll[data-first]')).toHaveCount(1);
  expect(Math.abs(await left() - was)).toBeLessThanOrEqual(1);
  // And once the turn ends.
  await emit(page, { type: 'done', sessionId: null });
  await expect(page.getByRole('group', { name: 'Reply details' })).toBeVisible();
  await expect(page.locator('.work-conversation .reply-table-scroll[data-first]')).toHaveCount(1);
  expect(Math.abs(await left() - was)).toBeLessThanOrEqual(1);
});

/** A new project, a sent message and a started run, for the streaming cases. */
async function startStream(page: Page, ask = 'Compare the hosts') {
  await page.setViewportSize({ width: 900, height: 900 });
  await installEventBridge(page);
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Compare hosts.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Hosts');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Hosts');
  await sendAndStart(page, ask, 'run-1', 1);
}
async function sendAndStart(page: Page, ask: string, runId: string, sends: number) {
  const composer = page.getByLabel('Chat message');
  await composer.fill(ask);
  await composer.press('Meta+Enter');
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBe(sends);
  await emit(page, { type: 'runStarted', asked: ask, runId, recall: [] });
}

// Smooth streaming (overnight 11-07, D1, gap 8): a reply's finished blocks stay
// mounted while its tail grows, so what you do in them holds.
test('while a reply streams on, a finished paragraph keeps its selection and a table its focus and Copied label', async ({ page }) => {
  test.setTimeout(60_000);
  await startStream(page);
  await page.evaluate(() => {
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async () => {}, write: async () => {} } });
  });
  await emit(page, { type: 'assistantDelta', text: 'The first paragraph is finished.\n\n| Host | Cost |\n|---|---|\n| Fly | $12 |\n\nThe last paragraph is still' });
  const reply = page.locator('.work-conversation .turn.assistant').last();
  const first = reply.locator('.md-view p').first();
  await expect(first).toHaveText('The first paragraph is finished.');
  // By class, not by name: with one table the button's name is its label, so
  // a name locator stops matching the moment it reads "Copied".
  const copy = reply.locator('button.reply-table-copy');
  await expect(copy).toHaveCount(1);
  await copy.click();
  await expect(copy).toHaveText('Copied');
  await expect(copy).toBeFocused();
  // Mark the very button: a rebuilt one would be a new element with the
  // label back at "Copy table" before its own 1.2 s were up.
  await copy.evaluate(b => { (b as HTMLElement).dataset.mark = '1'; });
  await first.evaluate(p => {
    const range = document.createRange();
    range.setStart(p.firstChild!, 4); range.setEnd(p.firstChild!, 9);
    const sel = window.getSelection()!;
    sel.removeAllRanges(); sel.addRange(range);
  });
  // Three more flushes of the growing paragraph.
  for (const words of [' growing', ' word by word', ' and then ends.']) {
    await emit(page, { type: 'assistantDelta', text: words });
    await expect(reply.locator('.md-view p').last()).toContainText(words.trim());
  }
  expect(await page.evaluate(() => {
    const sel = window.getSelection()!;
    return { text: sel.toString(), live: !!sel.anchorNode?.isConnected };
  })).toEqual({ text: 'first', live: true });
  const marked = reply.locator('button.reply-table-copy[data-mark]');
  await expect(marked).toHaveCount(1);
  await expect(marked).toBeFocused();
});

test('a reply that streamed in pieces ends with the same blocks as the same text arriving whole', async ({ page }) => {
  test.setTimeout(60_000);
  const pieces = [
    '## Where it runs\n\nThe API runs on `src/server.ts` behind', ' a proxy.\n\n- Fly for the',
    ' API\n- Render for the\n  workers\n\n| Host | Cost |\n|---|', '---|\n| Fly | $12 |\n| Render | $19 |\n\nThen the code:\n\n```ts\nconst port',
    ' = 8080;\n```\n\nAnd a closing line with **bold** words.',
  ];
  await startStream(page);
  for (const text of pieces) {
    await emit(page, { type: 'assistantDelta', text });
    // A flush between pieces, so each piece is its own render.
    await page.waitForTimeout(120);
  }
  await emit(page, { type: 'done', sessionId: null });
  const replies = page.locator('.work-conversation .turn.assistant .chat-msg');
  await expect(replies).toHaveCount(1);
  await expect(replies.first()).toContainText('closing line');
  await sendAndStart(page, 'Again, all at once', 'run-2', 2);
  await emit(page, { type: 'assistantDelta', text: pieces.join('') });
  await emit(page, { type: 'done', sessionId: null });
  await expect(replies).toHaveCount(2);
  await expect(replies.nth(1)).toContainText('closing line');
  const html = (n: number) => replies.nth(n).evaluate(el => [...el.querySelectorAll(':scope > .md-view')].map(v => v.innerHTML).join('\n<!-- next -->\n'));
  // Paths are marked from their own chunk, a moment after each render.
  const streamed = await html(0);
  expect(streamed).toContain('<table>');
  await expect.poll(() => html(1)).toBe(streamed);
});

test('a picture in a streaming reply is read once and stays put while the reply grows', async ({ page }) => {
  test.setTimeout(60_000);
  await startStream(page);
  await emit(page, { type: 'assistantDelta', text: 'Here is the chart:\n\n![chart](outputs/chart.png)\n\nThe tail is still' });
  const reply = page.locator('.work-conversation .turn.assistant').last();
  const picture = reply.getByRole('button', { name: 'Expand chart.png', exact: true });
  await expect(picture).toBeEnabled();
  const reads = () => page.evaluate(() => window.__mockApiCalls?.chat_preview_file ?? 0);
  const was = await reads();
  expect(was).toBeGreaterThan(0);
  // Mark the very button: a rebuilt block would be a new one, read again.
  await picture.evaluate(b => { (b as HTMLElement).dataset.mark = '1'; });
  for (const words of [' growing', ' word by word', ' and ends.']) {
    await emit(page, { type: 'assistantDelta', text: words });
    await expect(reply.locator('.md-view p').last()).toContainText(words.trim());
  }
  await emit(page, { type: 'done', sessionId: null });
  await expect(page.getByRole('group', { name: 'Reply details' })).toBeVisible();
  await expect(reply.locator('button.reply-local-image[data-mark] img')).toHaveCount(1);
  await expect(picture).toBeEnabled();
  expect(await reads()).toBe(was);
});

// Grok's review of #1210: a picture the reply names before its file exists must
// not stay failed (the old per-flush remount tried it again each time, and at
// the turn's end).
test('a picture whose file is not there yet loads on a later flush, or when the turn ends', async ({ page }) => {
  test.setTimeout(60_000);
  await startStream(page);
  const refuse = (names: string[]) => page.evaluate(list => {
    if (list.length) localStorage.setItem('mockPreviewRefusal', JSON.stringify(Object.fromEntries(list.map(name => [name, 'No such file']))));
    else localStorage.removeItem('mockPreviewRefusal');
  }, names);
  await refuse(['chart.png', 'later.png']);
  await emit(page, { type: 'assistantDelta', text: 'Two charts:\n\n![chart](outputs/chart.png)\n\n![later](outputs/later.png)\n\nThe tail is still' });
  const reply = page.locator('.work-conversation .turn.assistant').last();
  await expect(reply.locator('button.reply-local-image')).toHaveText(['Image not found: chart.png', 'Image not found: later.png']);
  // The first file is written and the reply goes on: the next flush loads it.
  await refuse(['later.png']);
  await emit(page, { type: 'assistantDelta', text: ' growing' });
  await expect(reply.getByRole('button', { name: 'Expand chart.png', exact: true })).toBeEnabled();
  await expect(reply.locator('button.reply-local-image').last()).toHaveText('Image not found: later.png');
  // The second is written after the last flush: the turn's end loads it.
  await refuse([]);
  await emit(page, { type: 'done', sessionId: null });
  await expect(page.getByRole('group', { name: 'Reply details' })).toBeVisible();
  await expect(reply.getByRole('button', { name: 'Expand later.png', exact: true })).toBeEnabled();
  await expect(reply.getByRole('button', { name: 'Expand chart.png', exact: true })).toBeEnabled();
});

test('with a single table the buttons carry plain names', async ({ page }) => {
  await seed(page, ['| a | b |', '|---|---|', '| 1 | 2 |'].join('\n'));
  await openChat(page);
  const box = page.locator('.work-conversation .reply-table');
  await expect(box.getByRole('button', { name: 'Copy table', exact: true })).toBeVisible();
  await expect(box.getByRole('region', { name: 'Table', exact: true })).toBeVisible();
});
