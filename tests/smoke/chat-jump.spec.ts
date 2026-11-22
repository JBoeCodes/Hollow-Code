import { openChat } from './choose';
import { expect, test, type Page } from './fixtures/test';

/**
 * Chat jump (11-10): an idle chat at its end slid further up with every
 * check-in, under a "Finished" pill (Alex: "every so often a chat randomly
 * jumps up to a much higher point"). A finished run folds its steps into
 * "Worked for…" as the composer drops its busy row; the transcript is shorter
 * for a moment inside that task, so the browser clamps `scrollTop` and the
 * view lands short of the end. The scroll controller read that move up as
 * the reader leaving the end and stopped following, so each later turn left
 * the view one more reply up. Opening a conversation for the first time did
 * the same (every reply swaps its loading view). A move up right after the
 * transcript changed, with no reader input, is now the layout's: the view
 * goes back to the end and keeps following. A reader's own scroll up still
 * lets go. The transcript runs with `overflow-anchor: none`, as in WebKit.
 */

const KEY = 's-20261110-chatjump1';
const PROSE = (n: number, word: string) => Array.from({ length: n }, (_, i) => `${word} paragraph ${i + 1} of the notes, long enough to wrap on a narrow window.`).join('\n\n');
const canvas = (id: string, rows: number) => '```hollow-canvas\n<hc-canvas id="' + id + '" title="Status ' + id + '">\n<hc-table label="Rows">\n' + Array.from({ length: rows }, (_, i) => `Row ${i} | ${i}`).join('\n') + '\n</hc-table>\n</hc-canvas>\n```';

/** A long conversation with canvases and check-ins, and the Tauri event
 *  bridge on top of the `?mock=1` IPC (as canvas-focus.spec.ts). */
async function seed(page: Page) {
  const cwd = '/Users/fixture/vault/workspaces/alpha-1';
  const at = new Date(Date.now() - 5 * 60_000).toISOString();
  const turns: unknown[] = [];
  for (let k = 0; k < 24; k++) {
    turns.push({ id: `${KEY}-u${k}`, role: 'user', text: k % 3 === 1 ? `Hollow · a chat you started has finished. Report ${k}.` : `Ask ${k}`, tools: [], agent: 'claude', runId: `run-${k}`, startedAt: at, transcriptIncomplete: false });
    turns.push({ id: `${KEY}-a${k}`, role: 'assistant', text: `${PROSE(3, `Lead ${k}`)}${k % 4 === 0 ? `\n\n${canvas(`c-${k}`, 6)}` : ''}\n\n${PROSE(2, `Tail ${k}`)}`, tools: [], agent: 'claude', runId: `run-${k}`, startedAt: at, status: 'success', transcriptIncomplete: false });
  }
  const session = { key: KEY, title: 'Jump chat', agent: 'claude', harnessSessionId: 'sess-native', cwd, createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 24, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'native' };
  await page.addInitScript(([s]) => {
    if (window !== window.top) return;
    const callbacks = new Map<number, (event: unknown) => void>();
    const listeners = new Map<string, number[]>();
    let next = 1;
    const w = window as unknown as Record<string, unknown>;
    w.__TAURI_INTERNALS__ = {
      transformCallback(callback: (event: unknown) => void) { const id = next++; callbacks.set(id, callback); return id; },
      invoke(command: string, args: { event?: string; handler?: number }) {
        if (command === 'plugin:event|listen' && args?.event && args.handler) { listeners.set(args.event, [...(listeners.get(args.event) ?? []), args.handler]); return Promise.resolve(next++); }
        if (command === 'plugin:event|unlisten') return Promise.resolve();
        return Promise.reject(new Error(`not in Tauri: ${command}`));
      },
    };
    w.__emitTauriEvent = (event: string, payload: unknown) => { for (const id of listeners.get(event) ?? []) callbacks.get(id)?.({ event, id, payload }); };
    if (sessionStorage.getItem('chatJumpSeeded')) return;
    sessionStorage.setItem('chatJumpSeeded', '1');
    for (const [k, v] of Object.entries(s as Record<string, unknown>)) localStorage.setItem(k, typeof v === 'string' ? v : JSON.stringify(v));
  }, [{
    'hollow.railWorkspaces': ['alpha-1'],
    mockWorkspaces: [{ id: 'alpha-1', title: 'Alpha', status: 'active', path: 'workspaces/alpha-1', cwd, modified: at }],
    mockWorkspaceSessions: [session],
    mockWorkspaceConversations: { [KEY]: { summary: session, eventsRetained: true, turns } },
  }] as const);
  await page.goto('/?mock=1');
  await page.addStyleTag({ content: '.chat-scroll, .chat-scroll * { overflow-anchor: none !important; }' });
  await openChat(page, 'Jump chat');
  await expect(scroller(page)).toBeVisible();
}

const scroller = (page: Page) => page.locator('.work-conversation .chat-scroll');
const jump = (page: Page) => page.locator('.work-conversation .chat-jump-latest');
const emit = (page: Page, payload: Record<string, unknown>) => page.evaluate(event => {
  (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void }).__emitTauriEvent('chat-event', event);
}, payload);
/** How far the transcript is from its end. */
const gap = (page: Page) => scroller(page).evaluate(s => Math.round(s.scrollHeight - s.scrollTop - s.clientHeight));

/** The transcript's height has held still for 600 ms. */
async function still(page: Page) {
  let last = -1;
  await expect.poll(async () => { const h = await scroller(page).evaluate(s => s.scrollHeight); const same = h === last; last = h; return same; }, { intervals: [600], timeout: 10_000 }).toBe(true);
}

/** What WebKit does inside a turn's end (and Chromium doesn't, so it is
 *  done here by hand): the transcript is shorter for a moment within one
 *  task, a layout read in that moment clamps `scrollTop`, and the full
 *  height is back before the frame. */
const clampOnce = (page: Page) => scroller(page).evaluate(s => {
  const turns = [...s.querySelectorAll<HTMLElement>('.turn[data-turn-index]')].slice(-6);
  for (const t of turns) t.style.display = 'none';
  void s.scrollTop;
  for (const t of turns) t.style.display = '';
});

/** A check-in the backend starts in this conversation: four steps, a short
 *  reply, its end (the steps fold into "Worked for…"). */
async function checkIn(page: Page, n: number) {
  await emit(page, { type: 'runStarted', asked: `Hollow · a chat you started has finished. Report ${n}.`, runId: `run-check-${n}`, queueId: 7000 + n, sessionKey: KEY, recall: [] });
  for (let i = 0; i < 4; i++) {
    await emit(page, { type: 'toolUse', id: `t${n}-${i}`, name: 'Read', input: { file_path: `PLAN-${i}.md` } });
    await emit(page, { type: 'toolResult', id: `t${n}-${i}`, content: 'plan '.repeat(40), isError: false });
  }
  await page.waitForTimeout(300);
  await emit(page, { type: 'assistantDelta', text: PROSE(2, `Check ${n}`) });
  await page.waitForTimeout(300);
  await emit(page, { type: 'done', sessionId: null, sessionKey: KEY, runId: `run-check-${n}` });
  await expect(page.locator(`.work-conversation .turn[data-turn-index]`).last()).toContainText(`Check ${n} paragraph 2`);
  await still(page);
}

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name.includes('narrow'), 'one desktop layout is enough for the scroll geometry');
  await page.emulateMedia({ reducedMotion: 'reduce' });
});

test('an idle chat at its end stays there as check-ins finish, and opens at its end', async ({ page }) => {
  await seed(page);
  await still(page);
  expect(await gap(page), 'opened short of the end').toBeLessThan(4);
  await expect(jump(page)).toHaveCount(0);

  // Typing a message as they arrive: the composer grows, nothing else.
  await page.getByLabel('Chat message').fill('every so often a chat randomly jumps up\nto a much higher point in the session');
  for (const n of [1, 2]) {
    await checkIn(page, n);
    if (n === 1) await clampOnce(page);
    await still(page);
    expect(await gap(page), `check-in ${n} left the view short of the end`).toBeLessThan(4);
    await expect(jump(page), `check-in ${n} let go of the end`).toHaveCount(0);
  }
});

test('the reader scrolling up still lets go of the end', async ({ page }) => {
  await seed(page);
  await still(page);
  await scroller(page).hover();
  await page.mouse.wheel(0, -900);
  await expect(jump(page)).toBeVisible();
  const top = await scroller(page).evaluate(s => s.scrollTop);
  await checkIn(page, 3);
  // They stay up there (not pulled to the end), and the pill says a run
  // finished below.
  expect(await gap(page)).toBeGreaterThan(400);
  expect(Math.abs(await scroller(page).evaluate(s => s.scrollTop) - top)).toBeLessThan(120);
  await expect(jump(page)).toHaveText(/Finished/);
});

// Review S4: only a clamp the transcript's own change made goes back. A move
// up with no change behind it and no input event (an assistive scroll) is
// the reader's.
test('a bare move up in a quiet chat still lets go of the end', async ({ page }) => {
  await seed(page);
  await still(page);
  await page.waitForTimeout(400);
  await scroller(page).evaluate(s => { s.scrollTop -= 300; });
  await expect(jump(page)).toBeVisible();
  expect(await gap(page)).toBeGreaterThan(250);
});
