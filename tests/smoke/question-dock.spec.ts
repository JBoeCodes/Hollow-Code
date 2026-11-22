import { expect, test, type Page } from './fixtures/test';

/**
 * Docked question cards (2026-10-14): an agent's questions take the message
 * box's place, one at a time ("1 of 3"), as Claude Code asks them. Choosing
 * goes on; Back and the trail revisit; "Other…" takes the person's words;
 * the keyboard starts on the recommended option; the last answer sends
 * everything straight to the agent ("Answered questions"), with no copy and
 * no draft. Esc hides the dock behind a "N questions waiting" chip.
 *
 * The card is written to the mock's `mockPendingInteractions` for the open
 * conversation ("Wire the session store") and its native session.
 */

const calls = (page: Page, command: string) => page.evaluate(c => window.__mockApiCalls?.[c] ?? 0, command);
const composer = (page: Page) => page.getByRole('textbox', { name: 'Chat message' });
const dock = (page: Page) => page.locator('.question-dock');
const lastPrompt = async (page: Page) => (await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}'))).prompt as string;

const options = (ids: string[], recommended?: string) => ids.map(label => ({ id: label.toLowerCase().replace(/\W+/g, '-'), label, ...(label === recommended ? { recommended: true } : {}) }));

/** Opens the conversation and puts a card in it. `stopClock` (a test tagged
 *  `@clock`, whose page runs on Playwright's fake clock) stops the page's
 *  clock just before the card goes in, so the dock shows at a time the test
 *  controls and the choosing keys' half second is the test's to run out;
 *  without it, this waits that half second out in real time. */
async function openWithCard(page: Page, opts: { typed?: string; questions?: unknown[]; stopClock?: boolean } = {}): Promise<void> {
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
  if (opts.typed) await composer(page).fill(opts.typed);
  const questions = opts.questions ?? [
    { id: 'install', header: 'Install main', prompt: 'Install main 4f196bf4 now?', required: true, answerMode: 'single', allowCustom: true, options: options(['Do it', 'Not now', 'Never'], 'Not now') },
    { id: 'codex', header: 'Codex read-only', prompt: 'Make Codex default to read-only?', required: true, answerMode: 'single', allowCustom: true, options: options(['Do it', 'Not now', 'Never'], 'Do it') },
    { id: 'checks', header: 'Hand checks', prompt: 'Which hand checks first?', required: false, answerMode: 'multi', allowCustom: false, options: options(['VoiceOver', 'Glass', 'Trackpad']) },
  ];
  if (opts.stopClock) {
    // The page's clock ran on while the conversation opened; stop it a little
    // ahead of where it is now (pausing can only go forward).
    const now = await page.evaluate(() => Date.now());
    await page.clock.pauseAt(now + 1000);
  }
  await page.evaluate(async ([url, qs]) => {
    const rt = await import(url);
    // The card resumes the conversation's own native session.
    rt.setChatSessionId('native-dock-1');
    const card = {
      id: 'int-dock-1', sessionKey: rt.chatAgentSessionKey(), turnOrd: 1, runId: 'run-dock', agent: 'claude',
      nativeSessionId: 'native-dock-1', toolUseId: 'tu-1', source: 'mcp', mode: 'questions', status: 'pending',
      request: { mode: 'questions', title: 'Morning check-in', supersedeOnUserMessage: true, questions: qs },
      answer: null, createdAt: '2026-10-14T12:00:00Z', resolvedAt: null,
    };
    localStorage.setItem('mockPendingInteractions', JSON.stringify([card]));
    await rt.refreshChatInteractions();
  }, ['/src/store/chatRuntime.ts', questions] as const);
  if (!opts.typed) {
    await expect(dock(page)).toBeVisible();
    // Choosing keys wait half a second after the dock shows (GRACE_MS).
    if (!opts.stopClock) await page.waitForTimeout(550);
  }
}

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name.includes('narrow'), 'Conversation history opens from the desktop settings index');
  // A `@clock` test drives the page's time itself: the fake clock has to be
  // in before the page loads, so the app's Date.now is Playwright's.
  if (info.tags.includes('@clock')) await page.clock.install();
  await page.goto('/?mock=1');
});

test('a tall question dock stays inside a short conversation at Largest', async ({page}) => {
  await page.setViewportSize({width:1440,height:920});
  await page.addInitScript(() => localStorage.setItem('hollow.studio.textScale','125'));
  await page.goto('/?mock=1');
  await openWithCard(page,{questions:[{id:'review',header:'Review',prompt:'Which material should we review first?',required:true,answerMode:'single',allowCustom:true,
    options:['Project summary','Open tasks','Source notes'].map((label,i)=>({id:String(i),label,description:'Read the supporting material before deciding the next local step.'}))}]});
  // Compact history intentionally shows either its list or the conversation.
  await page.getByRole('button', { name: 'Hide conversations', exact: true }).click();
  await page.setViewportSize({width:720,height:480});
  await expect(dock(page)).toBeVisible();
  await expect.poll(async()=>{const r=await dock(page).boundingBox();return r!.y+r!.height;}).toBeLessThanOrEqual(480);
  const decline=dock(page).getByRole('button',{name:'Don’t answer',exact:true});
  await decline.scrollIntoViewIfNeeded();
  await expect(decline).toBeInViewport();
  await decline.focus();
  await expect(decline).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(composer(page)).toBeVisible();
});

test('one question at a time over the message box; choosing goes on; the last answer sends straight to the agent', async ({ page }) => {
  await openWithCard(page);
  // Docked: the message box is gone while it shows, and the transcript holds no card.
  await expect(composer(page)).toBeHidden();
  await expect(page.locator('.chat-scroll .question-card, .turn .question-dock')).toHaveCount(0);
  await expect(dock(page).locator('.question-dock-count')).toHaveText('1 of 3');
  await expect(dock(page).locator('.question-dock-prompt')).toHaveText('Install main 4f196bf4 now?');
  // The recommended option is marked, and the keyboard starts on it.
  const list = dock(page).getByRole('listbox');
  await expect(list).toBeFocused();
  await expect(dock(page).locator('.question-dock-option.is-cursor')).toContainText('Not now');
  await expect(dock(page).locator('.question-dock-option.is-cursor .question-card-badge')).toHaveText('Recommended');
  await expect(dock(page).getByRole('option')).toHaveText([/Do it/, /Not now/, /Never/, /Other…/]);

  // Enter takes the highlighted option and moves on.
  await page.keyboard.press('Enter');
  await expect(dock(page).locator('.question-dock-count')).toHaveText('2 of 3');
  await expect(dock(page).locator('.question-dock-trail')).toContainText('Install mainNot now');
  // A number key picks directly.
  await page.keyboard.press('3');
  await expect(dock(page).locator('.question-dock-count')).toHaveText('3 of 3');
  // Back (a click) revisits; the earlier answer is kept and changeable.
  await dock(page).getByRole('button', { name: 'Back', exact: true }).click();
  await expect(dock(page).locator('.question-dock-count')).toHaveText('2 of 3');
  await expect(dock(page).locator('.question-dock-option.is-cursor')).toContainText('Never');
  await dock(page).getByRole('option', { name: /Do it/ }).click();
  await expect(dock(page).locator('.question-dock-count')).toHaveText('3 of 3');

  // A multi-choice question: Space ticks, Enter (or the button) sends.
  expect(await calls(page, 'chat_send')).toBe(0);
  await page.keyboard.press('Space');
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Space');
  await page.keyboard.press('Enter');
  await expect.poll(() => calls(page, 'chat_send')).toBe(1);
  expect(await lastPrompt(page)).toBe('Answered questions\n\nInstall main 4f196bf4 now?\nNot now\n\nMake Codex default to read-only?\nDo it\n\nWhich hand checks first?\nVoiceOver, Glass');
  // The dock goes, the message box is back, and the transcript keeps the answers.
  await expect(dock(page)).toHaveCount(0);
  await expect(composer(page)).toBeVisible();
  await expect(page.locator('.turn.user').last()).toContainText('Answered questions');
});

test('"Other…" takes the person\'s own words, and Esc hides the questions behind a chip that brings them back', async ({ page }) => {
  await openWithCard(page);
  await dock(page).getByRole('option', { name: /Other…/ }).click();
  const other = dock(page).getByRole('textbox', { name: 'Install main: your answer' });
  await expect(other).toBeFocused();
  // Enter with nothing typed says why and stays.
  await other.press('Enter');
  await expect(dock(page).getByRole('alert')).toHaveText('Type your answer');
  await other.fill('After the demo');
  await other.press('Enter');
  await expect(dock(page).locator('.question-dock-count')).toHaveText('2 of 3');

  // Esc: the message box comes back, with a chip for the questions.
  await page.keyboard.press('Escape');
  await expect(dock(page)).toBeHidden();
  await expect(composer(page)).toBeVisible();
  await expect(composer(page)).toBeFocused();
  const chip = page.getByRole('button', { name: '3 questions waiting' });
  await expect(chip).toBeVisible();
  await chip.click();
  await expect(dock(page)).toBeVisible();
  await page.waitForTimeout(550);
  // The hide button does the same as Esc.
  await dock(page).getByRole('button', { name: 'Hide the questions' }).click();
  await expect(chip).toBeVisible();
  await chip.click();
  // It opens where the person left it, with the answers given before hiding.
  await expect(dock(page).locator('.question-dock-count')).toHaveText('2 of 3');
  await expect(dock(page).getByRole('listbox')).toBeFocused();
  await expect(dock(page).locator('.question-dock-trail')).toContainText('Install mainAfter the demo');
  await dock(page).locator('.question-dock-trail').getByRole('button').first().click();
  await expect(dock(page).getByRole('textbox', { name: 'Install main: your answer' })).toHaveValue('After the demo');
  expect(await calls(page, 'chat_send')).toBe(0);
});

test('keys with ⌘, ⌃ or ⌥ are left alone, and "Don\'t answer" closes the card without sending', async ({ page }) => {
  await openWithCard(page);
  // (⌘1 is Hollow's own Work shortcut; it passes through the dock too.)
  for (const key of ['Control+1', 'Alt+1', 'Control+Enter', 'Alt+Enter']) await page.keyboard.press(key);
  await expect(dock(page).locator('.question-dock-count')).toHaveText('1 of 3');
  await dock(page).getByRole('button', { name: 'Don’t answer' }).click();
  await expect(dock(page)).toHaveCount(0);
  expect(await calls(page, 'dismiss_interaction')).toBe(1);
  await expect(page.getByRole('button', { name: /questions? waiting/ })).toHaveCount(0);
  await expect(composer(page)).toBeVisible();
  expect(await calls(page, 'chat_send')).toBe(0);
});

test('a card that arrives while the person is typing waits behind the chip, so their keys never answer it', async ({ page }) => {
  await openWithCard(page, { typed: 'Half a sentence' });
  await expect(page.getByRole('button', { name: '3 questions waiting' })).toBeVisible();
  await expect(dock(page)).toBeHidden();
  await expect(composer(page)).toHaveValue('Half a sentence');
  await composer(page).press('Space');
  await composer(page).press('2');
  expect(await calls(page, 'chat_send')).toBe(0);
  await expect(composer(page)).toHaveValue('Half a sentence 2');
});

test('Enter in a text answer waits out the same half second: nothing goes until then (2026-10-15)', { tag: '@clock' }, async ({ page }) => {
  // On the fake clock (EH6), stopped before the card goes in: the dock's
  // `shownAt` and every Enter below read times this test sets, so the check
  // can't be skipped or raced by a slow run. Before, it read the wall clock
  // and skipped whenever Enter landed after the half second.
  await openWithCard(page, {
    stopClock: true,
    questions: [{ id: 'when', header: 'Ship day', prompt: 'When should it ship?', required: true, answerMode: 'text', allowCustom: false, options: [] }],
  });
  const field = dock(page).getByRole('textbox', { name: 'When should it ship?' });
  await expect(field).toBeFocused();
  await field.fill('Friday');
  // 0 ms after the dock showed, then 499 ms (the last moment inside the half
  // second): Enter is held back each time. The real-time wait lets a send that
  // did start reach `chat_send` (its path has no timers); the page's clock
  // stays stopped through it, so the half second doesn't run out meanwhile.
  await field.press('Enter');
  await page.clock.runFor(499);
  await field.press('Enter');
  await page.waitForTimeout(300);
  // Nothing started: no preflight, no answer resolved, no send.
  for (const command of ['chat_preflight', 'resolve_interaction', 'chat_send']) expect(await calls(page, command), command).toBe(0);
  await expect(dock(page)).toBeVisible();
  await expect(field).toBeFocused();
  // Enter held back: no new line was typed either.
  await expect(field).toHaveValue('Friday');

  // At 500 ms (GRACE_MS) Enter sends as before; then time runs again.
  await page.clock.runFor(1);
  await field.press('Enter');
  await page.clock.resume();
  await expect.poll(() => calls(page, 'chat_send')).toBe(1);
  expect(await lastPrompt(page)).toBe('Answered questions\n\nWhen should it ship?\nFriday');
  await expect(dock(page)).toHaveCount(0);
});
