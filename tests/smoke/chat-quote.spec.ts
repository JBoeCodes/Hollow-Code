import { test, expect, type Page } from './fixtures/test';
import { openChat } from './choose';

// Quote (overnight 2026-11-07, D1, audit gap 9): text selected in a reply or a
// message of yours gets a Quote button just above it, which puts the passage in
// the message box as a Markdown quote with the caret under it. Nothing is sent.
// Fixture IPC only; no agent runs.
const cwd = '/Users/fixture/vault/workspaces/alpha-1';
const at = new Date(Date.now() - 5 * 60_000).toISOString();
const summary = { key: 's-20261107-cccc0001', title: 'Where to host the API', agent: 'claude', harnessSessionId: null, cwd, createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'none' };

async function seed(page: Page, reply: string, prompt = 'Where should the API run?') {
  await page.addInitScript(([s, text, c, ask]) => {
    if (sessionStorage.getItem('quoteSeeded')) return;
    sessionStorage.setItem('quoteSeeded', '1');
    localStorage.removeItem('hollow.railCollapsed');
    localStorage.removeItem('hollow.workspace');
    localStorage.setItem('hollow.railWorkspaces', JSON.stringify(['alpha-1']));
    localStorage.setItem('mockWorkspaces', JSON.stringify([{ id: 'alpha-1', title: 'Alpha', status: 'active', path: 'workspaces/alpha-1', cwd: c, modified: '2026-09-26T12:00:00Z' }]));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify([s]));
    const sm = s as { key: string; createdAt: string };
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify({ [sm.key]: { summary: s, eventsRetained: true, turns: [
      { id: 'u1', role: 'user', text: ask, tools: [], agent: 'claude', runId: 'r1', startedAt: sm.createdAt, transcriptIncomplete: false },
      { id: 'a1', role: 'assistant', text, tools: [], agent: 'claude', runId: 'r1', startedAt: sm.createdAt, status: 'success', transcriptIncomplete: false },
    ] } }));
  }, [summary, reply, cwd, prompt] as const);
  await page.goto('/?mock=1');
  await openChat(page, 'Where to host the API');
  // The reply's Markdown loads lazily: nothing to select until its first paragraph is there.
  await expect(replyParagraph(page, 0)).toBeVisible();
  // The reply is decorated a moment after its first render (paths are marked, blocks swapped in), which
  // replaces text nodes; a selection made in that gap is lost with them. Wait for the reply to hold still.
  await page.evaluate(() => new Promise<void>(resolve => {
    const reply = document.querySelector('.work-conversation .turn.assistant')!;
    let quiet = setTimeout(resolve, 300);
    const hard = setTimeout(resolve, 3000);
    new MutationObserver(() => { clearTimeout(quiet); quiet = setTimeout(() => { clearTimeout(hard); resolve(); }, 300); })
      .observe(reply, { childList: true, subtree: true, characterData: true });
  }));
}

const quoteButton = (page: Page) => page.getByRole('button', { name: 'Quote the selected text in your message' });
const messageBox = (page: Page) => page.getByRole('textbox', { name: 'Chat message' });
function replyParagraph(page: Page, n: number) { return page.locator('.work-conversation .turn.assistant .md-view p').nth(n); }
/** A selection from the middle of one paragraph to the middle of another, as a drag would make it. */
async function select(page: Page, from: number, fromOffset: number, to: number, toOffset: number) {
  await page.evaluate(([a, ao, b, bo]) => {
    const ps = document.querySelectorAll('.work-conversation .turn.assistant .md-view p');
    const range = document.createRange();
    range.setStart(ps[a].firstChild!, ao);
    range.setEnd(ps[b].firstChild!, bo);
    const sel = window.getSelection()!;
    sel.removeAllRanges(); sel.addRange(range);
  }, [from, fromOffset, to, toOffset] as const);
  await page.locator('.work-conversation .chat-scroll').dispatchEvent('mouseup');
}

test.beforeEach(async ({}, info) => { test.skip(info.project.name.includes('narrow'), 'The rail tree collapses to icons on narrow screens'); });

test('selecting a paragraph shows Quote just above it, and Quote puts it in the box as a quote', async ({ page }) => {
  await seed(page, 'The staging key is rotated on Fridays.\n\nThe release notes go out on Monday.');
  const first = replyParagraph(page, 0);
  await expect(first).toBeVisible();
  await first.click({ clickCount: 3 });
  await expect(quoteButton(page)).toBeVisible();
  // Centred on the selected words, its bottom edge 8px above them. Both are
  // read in one step, since the transcript may still be settling its scroll and
  // the button follows it a frame behind.
  await expect.poll(() => page.evaluate(() => {
    const pill = document.querySelector('.quote-selection')!.getBoundingClientRect();
    const words = window.getSelection()!.getRangeAt(0).getBoundingClientRect();
    return Math.max(Math.abs(pill.x + pill.width / 2 - (words.x + words.width / 2)), Math.abs(pill.bottom + 8 - words.y));
  }), 'centred on the selection and just above it').toBeLessThan(3);
  await quoteButton(page).click();
  await expect(messageBox(page)).toHaveValue('> The staging key is rotated on Fridays.\n\n');
  await expect(messageBox(page)).toBeFocused();
  // The button goes with the selection, and nothing was sent.
  await expect(quoteButton(page)).toHaveCount(0);
  expect(await page.evaluate(() => window.getSelection()?.toString())).toBe('');
  expect(await page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBe(0);
});

test('Quote goes after a draft, and a selection across paragraphs keeps a blank quote line between them', async ({ page }) => {
  await seed(page, 'The staging key is rotated on Fridays.\n\nThe release notes go out on Monday.');
  await messageBox(page).fill('What do you think?');
  await select(page, 0, 4, 1, 11);
  await expect(quoteButton(page)).toBeVisible();
  await quoteButton(page).click();
  await expect(messageBox(page)).toHaveValue('What do you think?\n\n> staging key is rotated on Fridays.\n>\n> The release\n\n');
  // The caret is under the quote, so the next words land there.
  await page.keyboard.type('Why Friday?');
  await expect(messageBox(page)).toHaveValue('What do you think?\n\n> staging key is rotated on Fridays.\n>\n> The release\n\nWhy Friday?');
});

test('the button leaves when the selection does, and a click on plain text brings none', async ({ page }) => {
  await seed(page, 'The staging key is rotated on Fridays.\n\nThe release notes go out on Monday.');
  await select(page, 0, 0, 0, 11);
  await expect(quoteButton(page)).toBeVisible();
  // A click that collapses the selection takes the button away.
  await page.evaluate(() => window.getSelection()?.removeAllRanges());
  await expect(quoteButton(page)).toHaveCount(0);
  // Nothing selected, nothing offered.
  await replyParagraph(page, 1).click();
  await expect(quoteButton(page)).toHaveCount(0);
  // Words selected in your own message are quotable too.
  // (A triple-click takes the whole line wherever it lands; a double-click takes the word under the pointer, which the middle of a wide block may miss.)
  await page.locator('.work-conversation .turn.user').getByText('Where should the API run?').first().click({ clickCount: 3 });
  await expect(quoteButton(page)).toBeVisible();
  await quoteButton(page).click();
  await expect(messageBox(page)).toHaveValue('> Where should the API run?\n\n');
});

test('the button follows its text while the transcript scrolls and goes when the text leaves the view', async ({ page }) => {
  const reply = Array.from({ length: 40 }, (_, i) => `Paragraph ${i + 1} of the long answer, with a few more words so it takes a line.`).join('\n\n');
  await seed(page, reply);
  const scroller = page.locator('.work-conversation .chat-scroll');
  const target = replyParagraph(page, 20);
  await target.scrollIntoViewIfNeeded();
  await target.click({ clickCount: 3 });
  await expect(quoteButton(page)).toBeVisible();
  const before = (await quoteButton(page).boundingBox())!.y;
  await scroller.evaluate(el => { el.scrollTop -= 40; });
  await expect.poll(async () => (await quoteButton(page).boundingBox())?.y ?? -1).toBeGreaterThan(before + 20);
  // Scrolled a long way off: nothing to point at.
  await scroller.evaluate(el => { el.scrollTop = 0; });
  await expect(quoteButton(page)).toHaveCount(0);
});

// Review of #1207: what a selection crosses that is not the passage, keyboard
// selections, a drag released outside the transcript, a selection taller than
// the view, and a folded long message.
test('a selection across a code block and a table quotes their words, never their buttons or labels', async ({ page }) => {
  await seed(page, 'Before the code.\n\n```ts\nconst port = 8080;\n```\n\n| Region | Cost |\n| --- | --- |\n| us-east | 12 |\n\nAfter the table.');
  await expect(page.locator('.work-conversation .reply-table')).toBeVisible();
  await select(page, 0, 0, 1, 9);
  await expect(quoteButton(page)).toBeVisible();
  await quoteButton(page).click();
  const value = await messageBox(page).inputValue();
  for (const words of ['> Before the code.', 'const port = 8080;', 'us-east', '> After the']) expect(value, value).toContain(words);
  for (const chrome of ['wrap', 'copy', 'Copy table', '> ts\n']) expect(value, value).not.toContain(chrome);
});

test('a keyboard selection shows Quote too, and Quote takes the selection as it is at the click', async ({ page }) => {
  await seed(page, 'The staging key is rotated on Fridays.\n\nThe release notes go out on Monday.');
  // Made without a mouse, as Shift and the arrows make it: no mouse-up comes.
  await page.evaluate(() => {
    const words = document.querySelector('.work-conversation .turn.assistant .md-view p')!.firstChild!;
    window.getSelection()!.setBaseAndExtent(words, 0, words, 11);
  });
  await expect(quoteButton(page)).toBeVisible();
  // Grown after the button came: the quote has all of it.
  await page.evaluate(() => {
    window.getSelection()!.extend(document.querySelector('.work-conversation .turn.assistant .md-view p')!.firstChild!, 15);
  });
  await quoteButton(page).click();
  await expect(messageBox(page)).toHaveValue('> The staging key\n\n');
});

// A press whose mouse-up never reaches the page must not hold the bar back from
// the keyboard for good: a drag cut short when the window loses focus, and a
// ctrl-click on macOS that opens the context menu.
for (const [when, fire] of [['the window loses focus', 'blur'], ['a context menu opens', 'contextmenu']] as const) {
  test(`a press that gets no mouse-up does not hide Quote from the keyboard after ${when}`, async ({ page }) => {
    await seed(page, 'The staging key is rotated on Fridays.\n\nThe release notes go out on Monday.');
    await page.evaluate(() => {
      document.dispatchEvent(new MouseEvent('mousedown', { button: 0, bubbles: true }));
      const words = document.querySelector('.work-conversation .turn.assistant .md-view p')!.firstChild!;
      window.getSelection()!.setBaseAndExtent(words, 0, words, 11);
    });
    // The press is still down: the keyboard selection waits for its mouse-up.
    await page.waitForTimeout(400);
    await expect(quoteButton(page)).toHaveCount(0);
    await page.evaluate(event => {
      if (event === 'blur') window.dispatchEvent(new Event('blur'));
      else document.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }));
    }, fire);
    // Shift and the arrows again: no mouse-up has come, and the bar shows.
    await page.evaluate(() => {
      window.getSelection()!.extend(document.querySelector('.work-conversation .turn.assistant .md-view p')!.firstChild!, 15);
    });
    await expect(quoteButton(page)).toBeVisible();
    await quoteButton(page).click();
    await expect(messageBox(page)).toHaveValue('> The staging key\n\n');
  });
}

test('a drag released over the composer quotes only its transcript part', async ({ page }) => {
  await seed(page, 'The staging key is rotated on Fridays.\n\nThe release notes go out on Monday.');
  await page.evaluate(() => {
    const words = document.querySelectorAll('.work-conversation .turn.assistant .md-view p')[1].firstChild!;
    const composer = document.querySelector('.work-conversation textarea')!.parentElement!;
    window.getSelection()!.setBaseAndExtent(words, 4, composer, 0);
  });
  await messageBox(page).dispatchEvent('mouseup');
  await expect(quoteButton(page)).toBeVisible();
  await quoteButton(page).click();
  await expect(messageBox(page)).toHaveValue('> release notes go out on Monday.\n\n');
});

test('a selection taller than the view puts Quote inside the view', async ({ page }) => {
  await seed(page, Array.from({ length: 40 }, (_, i) => `Paragraph ${i + 1} of the long answer, with a few more words so it takes a line.`).join('\n\n'));
  await select(page, 0, 0, 39, 9);
  const scroller = page.locator('.work-conversation .chat-scroll');
  // Both ends out of view.
  await scroller.evaluate(el => { el.scrollTop = (el.scrollHeight - el.clientHeight) / 2; });
  await expect(quoteButton(page)).toBeVisible();
  await expect.poll(async () => {
    const pill = (await quoteButton(page).boundingBox())!;
    const view = (await scroller.boundingBox())!;
    return pill.y >= view.y && pill.y + pill.height <= view.y + view.height;
  }, 'the button sits inside the transcript\'s view').toBe(true);
});

test('a folded long message quotes only what shows: its hidden lines need Show more first', async ({ page }) => {
  await seed(page, 'Short answer.', Array.from({ length: 20 }, (_, i) => `Line ${i + 1} of the long prompt`).join('\n'));
  const text = page.locator('.work-conversation .turn.user .text');
  await expect(text).toHaveClass(/chat-user-clipped/);
  const selectWhole = () => page.evaluate(() => {
    const range = document.createRange();
    range.selectNodeContents(document.querySelector('.work-conversation .turn.user .text')!);
    const sel = window.getSelection()!;
    sel.removeAllRanges(); sel.addRange(range);
  });
  // The whole message, its clipped lines too: nothing is offered.
  await selectWhole();
  await page.locator('.work-conversation .chat-scroll').dispatchEvent('mouseup');
  await page.waitForTimeout(400);
  await expect(quoteButton(page)).toHaveCount(0);
  // Its first line shows, so it quotes.
  await page.evaluate(() => {
    const words = document.querySelector('.work-conversation .turn.user .text')!.firstChild!;
    window.getSelection()!.setBaseAndExtent(words, 0, words, 'Line 1 of the long prompt'.length);
  });
  await expect(quoteButton(page)).toBeVisible();
  await quoteButton(page).click();
  await expect(messageBox(page)).toHaveValue('> Line 1 of the long prompt\n\n');
  // Opened, the whole message quotes.
  await messageBox(page).fill('');
  await page.locator('.work-conversation .turn.user').getByRole('button', { name: 'Show more' }).click();
  await expect(text).not.toHaveClass(/chat-user-clipped/);
  await selectWhole();
  await expect(quoteButton(page)).toBeVisible();
  await quoteButton(page).click();
  await expect(messageBox(page)).toHaveValue(/> Line 20 of the long prompt\n\n$/);
});
