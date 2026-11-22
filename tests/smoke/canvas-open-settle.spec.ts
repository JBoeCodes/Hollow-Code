import { openChat as openChatTitled, openTasks, openProjectFiles, paneCovers, previewPane } from './choose';
import { expect, test, type Page } from './fixtures/test';

/**
 * Settling an opened conversation (10-12, `src/chatSettle.ts`): opening a
 * conversation with canvases, or coming back to it from another page,
 * scrolled to its end while the canvases were still short placeholders. Their
 * growth then left the view a canvas or two back from the end (Alex:
 * "chat keeps throwing me up pretty far back, to an older canvas"). The
 * transcript runs with `overflow-anchor: none`, as in WebKit; Chromium's own
 * scroll anchoring would hide the bug.
 */

const PROSE = (n: number, word: string) => Array.from({ length: n }, (_, i) => `${word} paragraph ${i + 1} of the notes.`).join('\n\n');
const canvas = (id: string, title: string, rows: number) => '```hollow-canvas\n<hc-canvas id="' + id + '" title="' + title + '">\n<hc-stats>\n<hc-stat label="Done" value="42"></hc-stat>\n</hc-stats>\n<hc-table label="Groups">\n' + Array.from({ length: rows }, (_, i) => `Row ${i} | ${i}`).join('\n') + '\n</hc-table>\n</hc-canvas>\n```';

/** Seven replies with canvases, the same id reused in five of them (as in the
 *  Main agent conversation the report came from). */
async function seed(page: Page) {
  const cwd = '/Users/fixture/vault/workspaces/alpha-1';
  const at = new Date(Date.now() - 5 * 60_000).toISOString();
  const key = 's-20261012-settle01';
  const turns: unknown[] = [];
  for (let k = 0; k < 7; k++) {
    turns.push({ id: `${key}-u${k}`, role: 'user', text: `Ask ${k}`, tools: [], agent: 'claude', runId: `run-${k}`, startedAt: at, transcriptIncomplete: false });
    const id = k === 0 ? 'tmax-overview' : k === 1 ? 'hollow-cleanup-next' : 'vault-cleanup-ready';
    turns.push({ id: `${key}-a${k}`, role: 'assistant', text: `${PROSE(5, `Lead ${k}`)}\n\n${canvas(id, `Cleanup ${k}`, 8 + k)}\n\n${PROSE(3, `Tail ${k}`)}`, tools: [], agent: 'claude', runId: `run-${k}`, startedAt: at, status: 'success', transcriptIncomplete: false });
  }
  const session = { key, title: 'Cleanup chat', agent: 'claude', harnessSessionId: 'sess-native', cwd, createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 7, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'native' };
  await page.addInitScript(([s]) => {
    if (sessionStorage.getItem('settleSeeded')) return;
    sessionStorage.setItem('settleSeeded', '1');
    for (const [k, v] of Object.entries(s as Record<string, unknown>)) localStorage.setItem(k, typeof v === 'string' ? v : JSON.stringify(v));
  }, [{
    'hollow.railWorkspaces': ['alpha-1'],
    mockWorkspaces: [{ id: 'alpha-1', title: 'Alpha', status: 'active', path: 'workspaces/alpha-1', cwd, modified: at }],
    mockWorkspaceSessions: [session],
    mockWorkspaceConversations: { [key]: { summary: session, eventsRetained: true, turns } },
  }] as const);
  await page.goto('/?mock=1');
  await page.addStyleTag({ content: '.chat-scroll, .chat-scroll * { overflow-anchor: none !important; }' });
}

// The sidebar lists no conversations since L8 (11-10): Search chats opens it.
const openChat = (page: Page) => openChatTitled(page, 'Cleanup chat');

/** How far the transcript is from its end, and which reply's canvas shows. */
const geometry = (page: Page) => page.evaluate(() => {
  const s = document.querySelector<HTMLElement>('.work-conversation .chat-scroll')!;
  const box = s.getBoundingClientRect();
  const shown = [...s.querySelectorAll<HTMLElement>('.chat-canvas')]
    .filter(c => { const r = c.getBoundingClientRect(); return r.bottom > box.top && r.top < box.bottom; })
    .map(c => Number(c.closest<HTMLElement>('.turn')?.dataset.turnIndex));
  return { top: Math.round(s.scrollTop), gap: Math.round(s.scrollHeight - s.scrollTop - s.clientHeight), shown };
});

/** The frames have drawn: one is up and the transcript's height has held
 *  still for 600 ms (frames out of view stay parked, so not all of them). */
async function framesDrawn(page: Page) {
  await expect(page.locator('.work-conversation iframe[title^="Canvas: Cleanup"]').first()).toBeVisible();
  const height = () => page.evaluate(() => document.querySelector<HTMLElement>('.work-conversation .chat-scroll')!.scrollHeight);
  let last = -1;
  await expect.poll(async () => { const h = await height(); const still = h === last; last = h; return still; }, { intervals: [600], timeout: 8000 }).toBe(true);
}

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name.includes('narrow'), 'one desktop layout is enough for the scroll geometry');
  await page.emulateMedia({ reducedMotion: 'reduce' });
});

test('opening a conversation with canvases ends at its end once the frames draw, and again on coming back', async ({ page }) => {
  await seed(page);
  await openChat(page);
  await framesDrawn(page);
  let g = await geometry(page);
  expect(g.gap, `opened ${g.gap}px above the end, on canvases ${g.shown}`).toBeLessThan(4);
  expect(g.shown).toContain(13);

  // Away to another page and back: U4 retains the hidden transcript.
  await openTasks(page);
  await expect(page.getByRole('heading', { name: 'Tasks', level: 1 })).toBeVisible();
  await expect(page.locator('.work-conversation')).toHaveCount(1);
  await expect(page.locator('.work-conversation')).toBeHidden();
  await openChat(page);
  await framesDrawn(page);
  g = await geometry(page);
  expect(g.gap, `came back ${g.gap}px above the end, on canvases ${g.shown}`).toBeLessThan(4);
  expect(g.shown).toContain(13);
});

// Lane C's open medium (11-04, after #1168): coming back to a chat with the
// pointer already resting on a canvas frame, here back through the workspace
// panel's Close in a narrow window, pressed from the keyboard so the pointer
// stays put. The frames grow after the show and come to sit under the still
// pointer, and their own re-anchor in the show's first second counted as the
// reader's scroll, so the view stopped short of the end. A frame under the
// pointer is the reader's only after a real pointer move since the show
// (`readerOverFrame`).
type Point = { x: number; y: number };

/** Opened wide (a window under 861 px starts with the sidebar folded, so its
 *  chat row isn't there to click), then narrowed so the Preview pane
 *  covers the chat. A point on the lowest canvas frame on screen at the end,
 *  and one beside the transcript's text at that height. */
async function narrowAtTheEnd(page: Page): Promise<{ frame: Point; beside: Point }> {
  await seed(page);
  await openChat(page);
  await framesDrawn(page);
  await page.setViewportSize({ width: 720, height: 900 });
  await framesDrawn(page);
  await expect.poll(async () => (await geometry(page)).gap, 'at the end after narrowing').toBeLessThan(4);
  const at = await page.evaluate(() => {
    const s = document.querySelector<HTMLElement>('.work-conversation .chat-scroll')!.getBoundingClientRect();
    const frames = [...document.querySelectorAll<HTMLIFrameElement>('.work-conversation iframe[title^="Canvas: Cleanup"]')].reverse();
    for (const f of frames) {
      const r = f.getBoundingClientRect();
      const top = Math.max(r.top, s.top), bottom = Math.min(r.bottom, s.bottom);
      if (bottom - top > 40) {
        const y = Math.round((top + bottom) / 2);
        return { frame: { x: Math.round(r.left + r.width / 2), y }, beside: { x: Math.round(s.left + 8), y } };
      }
    }
    return null;
  });
  expect(at, 'a canvas frame shows at the end').not.toBeNull();
  return at!;
}

/** Opens the Preview pane (Project files) over the chat with the pointer at `rest`,
 *  closes it from the keyboard and, once a canvas frame is hovered, scrolls
 *  the transcript 68 px up with no input event. That scroll stands in for
 *  the frames' own re-anchor as they unpark (68 px in lane C's diagnostic
 *  run; headless Chromium's frames don't always re-anchor on their own, and
 *  with the fix off the resting case passed 3 runs in 3 without it) or for a
 *  reader's wheel chained out of the frame, as in the bare-scroll frame test
 *  below. With `moveTo`, once the chat has shown, the pointer first moves
 *  within the gutter beside the text (`rest` is there), which this window
 *  sees, then onto the frame. The rule only holds in the show's first
 *  second, so the wait and the scroll run in the page, timed from before the
 *  Close, and the scroll's event is sent at once, before that frame's resize
 *  observers can put the view back and leave the rule unasked. A cycle whose
 *  frame came under the pointer too late proves nothing, scrolls nothing,
 *  and is tried again from the end. */
async function backThroughClose(page: Page, rest: Point, moveTo?: Point) {
  for (let attempt = 1; attempt <= 3; attempt++) {
    await openProjectFiles(page);
    expect(await paneCovers(page), 'the Preview pane covers the chat at 720 px').toBe(true);
    // The pointer goes where it rests, over the pane for now.
    await page.mouse.move(rest.x, rest.y);
    const close = previewPane(page).getByRole('button', { name: 'Close preview', exact: true });
    await close.focus();
    const before = await page.evaluate(() => performance.now());
    await page.keyboard.press('Enter');
    if (moveTo) {
      // Shown: laid out again, then one more frame for the controller's
      // resize observer, which runs after this frame's callbacks.
      await page.evaluate(async () => {
        const frame = () => new Promise(r => requestAnimationFrame(r));
        for (;;) {
          const s = document.querySelector<HTMLElement>('.work-conversation .chat-scroll');
          if (s?.offsetParent && s.clientHeight > 0 && !document.querySelector('#preview-pane')) break;
          await frame();
        }
        await frame();
      });
      await page.mouse.move(rest.x + 12, rest.y);
      await page.mouse.move(moveTo.x, moveTo.y, { steps: 4 });
    }
    const took = await page.evaluate(async start => {
      while (performance.now() - start < 850) {
        const s = document.querySelector<HTMLElement>('.work-conversation .chat-scroll');
        if (s && !document.querySelector('#preview-pane') && document.querySelector('.work-conversation iframe:hover')) {
          s.scrollTop -= 68;
          s.dispatchEvent(new Event('scroll'));
          return Math.round(performance.now() - start);
        }
        await new Promise(r => requestAnimationFrame(r));
      }
      return null;
    }, before);
    if (took !== null) return took;
    await framesDrawn(page);
    // A frame's own re-anchor after that second is a scroll like any other:
    // back to the end, as a person would (End), so the next Close is a show
    // at the end.
    if ((await geometry(page)).gap >= 4) {
      await page.evaluate(() => {
        const s = document.querySelector<HTMLElement>('.work-conversation .chat-scroll')!;
        s.dispatchEvent(new KeyboardEvent('keydown', { key: 'End' }));
        s.scrollTop = s.scrollHeight;
        s.dispatchEvent(new Event('scroll'));
      });
      await expect.poll(async () => (await geometry(page)).gap, 'back at the end for another try').toBeLessThan(4);
    }
  }
  throw new Error('no canvas frame came under the pointer in the first 850 ms after the Close, in 3 tries');
}

test('coming back with the pointer resting on a canvas frame still ends at the end', async ({ page }) => {
  const { frame } = await narrowAtTheEnd(page);
  await backThroughClose(page, frame);
  await framesDrawn(page);
  const g = await geometry(page);
  expect(g.gap, `came back ${g.gap}px above the end, on canvases ${g.shown}`).toBeLessThan(4);
  const under = await page.evaluate(({ x, y }) => document.elementFromPoint(x, y)?.tagName ?? null, frame);
  expect(under, 'the pointer rests on a canvas frame').toBe('IFRAME');
});

test('coming back and moving onto a canvas frame, a scroll out of it is the reader’s', async ({ page }) => {
  const { frame, beside } = await narrowAtTheEnd(page);
  await backThroughClose(page, beside, frame);
  await framesDrawn(page);
  const g = await geometry(page);
  expect(g.gap, `the reader's scroll was put back (${g.gap}px above the end)`).toBeGreaterThan(60);
  const under = await page.evaluate(({ x, y }) => document.elementFromPoint(x, y)?.tagName ?? null, frame);
  expect(under, 'the pointer is on a canvas frame').toBe('IFRAME');
});

test('the person’s own scroll while it settles wins, over a canvas frame too', async ({ page }) => {
  await seed(page);
  await openChat(page);
  // A wheel over the transcript itself.
  await expect(page.locator('.work-conversation .chat-canvas').first()).toBeAttached();
  const scroller = page.locator('.work-conversation .chat-scroll');
  await expect.poll(async () => (await geometry(page)).top).toBeGreaterThan(1500);
  // Let the opening/restoration placement land; canvases are still settling.
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  const box = (await scroller.boundingBox())!;
  await page.mouse.move(box.x + 8, box.y + box.height / 2);
  await page.mouse.wheel(0, -1500);
  await expect.poll(async () => (await geometry(page)).gap).toBeGreaterThan(600);
  await framesDrawn(page);
  expect((await geometry(page)).gap).toBeGreaterThan(600);

  // Over a canvas frame the transcript's own listeners see nothing: the
  // scroll chains out of the frame (headless Chromium doesn't chain a wheel
  // out of a frame, so it is made here as a bare scroll with no input event).
  // Work Home closes the workspace and really unmounts its transcript.
  // Other pages retain it, so they cannot exercise a fresh settle here.
  await page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true }).click();
  await expect(page.locator('.work-conversation')).toHaveCount(0);
  await openChat(page);
  await expect(page.locator('.work-conversation .chat-canvas').first()).toBeAttached();
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  // The pointer is on the frame it scrolls out of (a bare move up with no
  // reader input and no frame under the pointer is a layout clamp since the
  // chat jump fix, 11-10, and goes back to the end).
  const frame = page.locator('.work-conversation iframe[title^="Canvas: Cleanup"]').last();
  await expect(frame).toBeVisible();
  const fb = (await frame.boundingBox())!;
  await page.mouse.move(fb.x + fb.width / 2, fb.y + Math.min(fb.height / 2, 40));
  await page.evaluate(() => { const s = document.querySelector<HTMLElement>('.work-conversation .chat-scroll')!; s.scrollTop = Math.max(0, s.scrollTop - 1500); });
  await expect.poll(async () => (await geometry(page)).gap).toBeGreaterThan(600);
  await framesDrawn(page);
  expect((await geometry(page)).gap).toBeGreaterThan(600);
});

// H10 (2026-10-15): a click inside a canvas frame is the person's too. None of
// the frame's events reach the transcript (sandboxed, no same-origin); focus
// moving into it blurs the window with the frame active
// (`isTranscriptFrameFocus`), which ends the settle like a click in the prose.
test('a click inside a canvas frame while it settles ends the settle', async ({ page }) => {
  await seed(page);
  await openChat(page);
  await framesDrawn(page);
  const turn = page.locator('.work-conversation .turn[data-turn-index="13"]');
  const frame = turn.locator('iframe[title^="Canvas: Cleanup"]');
  const above = page.locator('.work-conversation .turn[data-turn-index="1"] .chat-canvas').first();
  // Still settling (its quiet time is 3 s): growth above goes back to the end.
  const top0 = (await geometry(page)).top;
  await above.evaluate(el => (el as HTMLElement).style.paddingBottom = '300px');
  await expect.poll(async () => { const g = await geometry(page); return g.gap < 4 && g.top >= top0 + 296; }).toBe(true);
  // A click on the last canvas's last row, inside its frame. The row is in
  // view already, so Playwright doesn't scroll the transcript to reach it (a
  // scroll of its own could end the settle and pass this for the wrong reason).
  const before = (await geometry(page)).top;
  await turn.frameLocator('iframe[title^="Canvas: Cleanup"]').getByText('Row 13', { exact: true }).click();
  await expect(frame).toBeFocused();
  expect((await geometry(page)).top).toBe(before);
  // The place is theirs now: the next growth above stays where it lands.
  await above.evaluate(el => (el as HTMLElement).style.paddingBottom = '900px');
  await expect.poll(async () => (await geometry(page)).gap).toBeGreaterThan(500);
  await page.waitForTimeout(250);
  expect((await geometry(page)).gap).toBeGreaterThan(500);
});

const markedTop = (page: Page) => page.locator('[data-reading-marker]').evaluate(el => el.getBoundingClientRect().top);
async function markReading(page: Page) {
  return page.evaluate(() => {
    document.querySelector('[data-reading-marker]')?.removeAttribute('data-reading-marker');
    const s = document.querySelector<HTMLElement>('.work-conversation .chat-scroll')!;
    const box = s.getBoundingClientRect();
    const blocks = [...s.querySelectorAll<HTMLElement>('.turn p, .turn pre, .turn .chat-canvas')];
    // A canvas that starts above the view isn't the reading place; the text
    // after it is (night ST4, `chatReadingAnchor.ts`).
    const el = blocks.find(n => { const r = n.getBoundingClientRect(); return r.height > 0 && r.bottom > box.top + 48 && r.top < box.bottom && !(n.matches('.chat-canvas') && r.top < box.top); })!;
    el.setAttribute('data-reading-marker', 'true');
    return el.getBoundingClientRect().top;
  });
}
async function readMiddle(page: Page, native = false) {
  await seed(page);
  await openChat(page);
  await framesDrawn(page);
  if (native) await page.addStyleTag({ content: '.chat-scroll, .chat-scroll * { overflow-anchor: auto !important; }' });
  await page.locator('.work-conversation .turn[data-turn-index="7"] p').filter({ hasText: 'Tail 3 paragraph 1' }).first().evaluate(el => {
    const s = el.closest('.chat-scroll') as HTMLElement;
    s.dispatchEvent(new WheelEvent('wheel', { deltaY: -100, bubbles: true }));
    s.scrollTop += el.getBoundingClientRect().top - s.getBoundingClientRect().top - 40;
  });
  await page.waitForTimeout(800);
}
async function grow(page: Page, turn = 3, height = 300) {
  await page.locator(`.work-conversation .turn[data-turn-index="${turn}"] .chat-canvas`).first().evaluate((el, h) => (el as HTMLElement).style.paddingBottom = `${h}px`, height);
  await page.waitForTimeout(250);
}

// V4: late growth in an earlier canvas must hold the text being read, including
// text below that canvas in the SAME turn. Raw scrollTop or a turn-top pin is
// insufficient. Padding simulates a frame's asynchronous height report.
test('reading prose stays still when a canvas above grows or shrinks', async ({ page }) => {
  await seed(page);
  await openChat(page);
  await framesDrawn(page);
  const reading = page.locator('.work-conversation .turn[data-turn-index="7"] p').filter({ hasText: 'Tail 3 paragraph 1' }).first();
  await reading.evaluate(el => {
    const s = el.closest('.chat-scroll') as HTMLElement;
    s.dispatchEvent(new WheelEvent('wheel', { deltaY: -100, bubbles: true }));
    s.scrollTop += el.getBoundingClientRect().top - s.getBoundingClientRect().top - 40;
  });
  await page.waitForTimeout(800);
  const y = () => reading.evaluate(el => el.getBoundingClientRect().top);
  const before = await y();
  const above = page.locator('.work-conversation .turn[data-turn-index="7"] .chat-canvas').first();
  await above.evaluate(el => (el as HTMLElement).style.paddingBottom = '400px');
  await page.waitForTimeout(250);
  await expect.poll(async () => Math.abs(await y() - before)).toBeLessThan(3);
  await above.evaluate(el => (el as HTMLElement).style.paddingBottom = '0px');
  await page.waitForTimeout(250);
  await expect.poll(async () => Math.abs(await y() - before)).toBeLessThan(3);
  const below = page.locator('.work-conversation .turn[data-turn-index="11"] .chat-canvas').first();
  await below.evaluate(el => (el as HTMLElement).style.paddingBottom = '300px');
  await page.waitForTimeout(250);
  await expect.poll(async () => Math.abs(await y() - before)).toBeLessThan(3);
});


// Night ST4 (2026-10-14; follow-up of #581): scrolling UP, a canvas enters
// from the top with only its lower part on screen. The reading anchor used to
// pick that canvas (the first block crossing the reading line) and hold its
// top, so when it finished laying out every line below it moved down by the
// growth. The text under it is what's being read; it stays put.
test('scrolling up past a canvas that is still growing, the text under it stays put', async ({ page }) => {
  await seed(page);
  await openChat(page);
  await framesDrawn(page);
  const canvasEl = page.locator('.work-conversation .turn[data-turn-index="7"] .chat-canvas').first();
  const reading = page.locator('.work-conversation .turn[data-turn-index="7"] p').filter({ hasText: 'Tail 3 paragraph 1' }).first();
  // The person scrolls up until only the canvas's last 120 px show at the top.
  await canvasEl.evaluate(el => {
    const s = el.closest('.chat-scroll') as HTMLElement;
    s.dispatchEvent(new WheelEvent('wheel', { deltaY: -120, bubbles: true }));
    s.scrollTop += el.getBoundingClientRect().bottom - s.getBoundingClientRect().top - 120;
  });
  await page.waitForTimeout(800);
  const straddles = await canvasEl.evaluate(el => {
    const s = el.closest('.chat-scroll') as HTMLElement;
    const r = el.getBoundingClientRect(), box = s.getBoundingClientRect();
    return r.top < box.top && r.bottom > box.top + 60;
  });
  expect(straddles, 'the canvas straddles the top edge').toBe(true);
  const y = () => reading.evaluate(el => el.getBoundingClientRect().top);
  const before = await y();
  // It finishes laying out 400 px taller, then settles 150 px shorter.
  await canvasEl.evaluate(el => (el as HTMLElement).style.paddingBottom = '400px');
  await page.waitForTimeout(250);
  await expect.poll(async () => Math.abs(await y() - before)).toBeLessThan(3);
  await canvasEl.evaluate(el => (el as HTMLElement).style.paddingBottom = '250px');
  await page.waitForTimeout(250);
  await expect.poll(async () => Math.abs(await y() - before)).toBeLessThan(3);
});

// Night ST4 review: with no text after it on screen, a canvas ending its turn
// gives way to the next turn's text; a canvas taller than the view is itself
// what's read and keeps its top.
test('a canvas ending its turn gives way to the next turn; a canvas taller than the view keeps its top', async ({ page }) => {
  await seed(page);
  await openChat(page);
  await framesDrawn(page);
  const turn7 = page.locator('.work-conversation .turn[data-turn-index="7"]');
  const canvasEl = turn7.locator('.chat-canvas').first();
  // No text after the canvas in its own turn.
  await turn7.evaluate(t => {
    for (const p of t.querySelectorAll<HTMLElement>('p')) if ((p.textContent ?? '').startsWith('Tail 3')) p.style.display = 'none';
    for (const f of t.querySelectorAll<HTMLElement>('.turn-footer')) f.style.display = 'none';
  });
  const scrollTo = (offset: number) => canvasEl.evaluate((el, off) => {
    const s = el.closest('.chat-scroll') as HTMLElement;
    s.dispatchEvent(new WheelEvent('wheel', { deltaY: -120, bubbles: true }));
    s.scrollTop += el.getBoundingClientRect().bottom - s.getBoundingClientRect().top - off;
  }, offset);
  await scrollTo(120);
  await page.waitForTimeout(800);
  const next = page.locator('.work-conversation .turn[data-turn-index="8"]');
  const nextTop = () => next.evaluate(el => el.getBoundingClientRect().top);
  const before = await nextTop();
  await canvasEl.evaluate(el => (el as HTMLElement).style.paddingBottom = '400px');
  await page.waitForTimeout(250);
  await expect.poll(async () => Math.abs(await nextTop() - before)).toBeLessThan(3);

  // Taller than the view, top and bottom both off screen: its top holds.
  await canvasEl.evaluate(el => (el as HTMLElement).style.paddingBottom = '2400px');
  await page.waitForTimeout(400);
  await canvasEl.evaluate(el => {
    const s = el.closest('.chat-scroll') as HTMLElement;
    s.dispatchEvent(new WheelEvent('wheel', { deltaY: -120, bubbles: true }));
    s.scrollTop += el.getBoundingClientRect().top - s.getBoundingClientRect().top + 200;
  });
  await page.waitForTimeout(800);
  const spans = await canvasEl.evaluate(el => {
    const s = el.closest('.chat-scroll') as HTMLElement;
    const r = el.getBoundingClientRect(), box = s.getBoundingClientRect();
    return r.top < box.top && r.bottom > box.bottom;
  });
  expect(spans, 'the canvas runs past both edges').toBe(true);
  const top = () => canvasEl.evaluate(el => el.getBoundingClientRect().top);
  const at = await top();
  await canvasEl.evaluate(el => (el as HTMLElement).style.paddingBottom = '2700px');
  await page.waitForTimeout(250);
  await expect.poll(async () => Math.abs(await top() - at)).toBeLessThan(3);
});

test('native browser anchoring and Hollow do not both compensate', async ({ page }) => {
  await readMiddle(page, true);
  const at = await markReading(page);
  await grow(page);
  expect(Math.abs(await markedTop(page) - at)).toBeLessThan(3);
  await grow(page, 3, 0);
  expect(Math.abs(await markedTop(page) - at)).toBeLessThan(3);
});

for (const intent of ['wheel', 'touchmove', 'keyboard', 'focus', 'bare-scroll']) {
  test(`reading anchor yields to ${intent} and holds the new reading place`, async ({ page }) => {
    await readMiddle(page);
    const s = page.locator('.work-conversation .chat-scroll');
    const before = (await geometry(page)).top;
    if (intent === 'wheel') {
      const box = (await s.boundingBox())!;
      await page.mouse.move(box.x + 8, box.y + box.height / 2);
      await page.mouse.wheel(0, -260);
    } else if (intent === 'keyboard') {
      await s.focus();
      await page.keyboard.press('PageUp');
    } else if (intent === 'focus') {
      await page.locator('.work-conversation .turn[data-turn-index="3"] p').first().evaluate(el => {
        (el as HTMLElement).tabIndex = -1;
        (el as HTMLElement).focus();
      });
    } else {
      await s.evaluate((el, kind) => {
        if (kind === 'touchmove') el.dispatchEvent(new Event('touchmove', { bubbles: true }));
        el.scrollTop -= 260; // bare scroll also represents chaining out of an iframe
      }, intent);
    }
    await expect.poll(async () => Math.abs((await geometry(page)).top - before)).toBeGreaterThan(100);
    await page.waitForTimeout(350);
    const at = await markReading(page);
    await grow(page, 1);
    expect(Math.abs(await markedTop(page) - at)).toBeLessThan(3);
  });
}

test('find and latest take priority, then a send follows its own reply', async ({ page }) => {
  await readMiddle(page);
  await page.keyboard.press('Meta+f');
  await page.getByRole('searchbox', { name: 'Find in this conversation' }).fill('Ask 5');
  await page.waitForTimeout(500);
  const at = await markReading(page);
  await grow(page, 1);
  expect(Math.abs(await markedTop(page) - at)).toBeLessThan(3);
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: /Jump to latest/ }).click();
  await expect.poll(async () => (await geometry(page)).gap).toBeLessThan(4);
  await readMiddle(page);
  const composer = page.getByLabel('Chat message');
  await composer.fill('Please continue from here');
  await composer.press('Meta+Enter');
  await expect.poll(async () => (await geometry(page)).gap).toBeLessThan(4);
});

test('growth between a manual scroll and its pending reset frame keeps the new place', async ({ page }) => {
  await readMiddle(page);
  const at = await page.evaluate(() => {
    const s = document.querySelector<HTMLElement>('.work-conversation .chat-scroll')!;
    const above = s.querySelector<HTMLElement>('.turn[data-turn-index="1"] .chat-canvas')!;
    // This layout frame is already queued when the person moves the view.
    requestAnimationFrame(() => { above.style.paddingBottom = '300px'; });
    s.dispatchEvent(new WheelEvent('wheel', { deltaY: -100, bubbles: true }));
    s.scrollTop -= 100;
    s.dispatchEvent(new Event('scroll'));
    const box = s.getBoundingClientRect();
    const n = [...s.querySelectorAll<HTMLElement>('.turn p, .turn pre, .turn .chat-canvas')]
      .find(n => { const r = n.getBoundingClientRect(); return r.height > 0 && r.bottom > box.top + 48 && r.top < box.bottom && !(n.matches('.chat-canvas') && r.top < box.top); })!;
    n.setAttribute('data-reading-marker', 'true');
    return n.getBoundingClientRect().top;
  });
  await page.waitForTimeout(300);
  expect(Math.abs(await markedTop(page) - at)).toBeLessThan(3);
});

test('a replaced Markdown paragraph retains its place through simultaneous earlier growth', async ({ page }) => {
  await readMiddle(page);
  const at = await markReading(page);
  await page.locator('[data-reading-marker]').evaluate(el => {
    const s = el.closest('.chat-scroll')!;
    el.replaceWith(el.cloneNode(true));
    (s.querySelector('.turn[data-turn-index="1"] .chat-canvas') as HTMLElement).style.paddingBottom = '300px';
  });
  await page.waitForTimeout(300);
  expect(Math.abs(await markedTop(page) - at)).toBeLessThan(3);
});

test('a smooth find jump is not cancelled by a reading correction', async ({ page }) => {
  await readMiddle(page);
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.keyboard.press('Meta+f');
  const wanted = await page.locator('.work-conversation .turn[data-turn-index="2"]').evaluate(el => {
    const s = el.closest('.chat-scroll') as HTMLElement;
    const r = el.getBoundingClientRect(), b = s.getBoundingClientRect();
    return Math.max(0, s.scrollTop + r.top - b.top - (s.clientHeight - r.height) / 2);
  });
  await page.getByRole('searchbox', { name: 'Find in this conversation' }).fill('Ask 1');
  await page.waitForTimeout(90);
  await grow(page, 1);
  await expect.poll(async () => Math.abs((await geometry(page)).top - wanted)).toBeLessThan(4);
});

for (const theme of ['light', 'dark', 'glass']) test(`reading appearance ${theme}`, async ({ page }, info) => {
  await page.addInitScript(theme => {
    localStorage.setItem('theme', theme === 'light' ? 'light' : 'dark');
    localStorage.setItem('glass', theme === 'glass' ? 'on' : 'off');
  }, theme);
  await readMiddle(page);
  if (theme === 'glass') await page.addStyleTag({ content: 'html[data-glass] { background:var(--canvas); }' });
  for (const width of [1440, 600]) {
    await page.setViewportSize({ width, height: 920 });
    await page.waitForTimeout(300);
    const at = await markReading(page);
    await grow(page, 1, width === 1440 ? 300 : 600);
    expect(Math.abs(await markedTop(page) - at)).toBeLessThan(3);
    await page.screenshot({ path: info.outputPath(`reading-${theme}-${width}.png`) });
  }
});

test('reading inside a list anchors its visible child instead of the tall list item', async ({ page }) => {
  await readMiddle(page);
  await page.evaluate(() => {
    const s = document.querySelector<HTMLElement>('.work-conversation .chat-scroll')!;
    const turn = s.querySelector('.turn[data-turn-index="7"]')!;
    const list = document.createElement('ul');
    list.innerHTML = '<li><div class="chat-canvas" data-growing-list style="height:200px">Earlier list canvas</div><p data-reading-marker>Reading this paragraph inside the same list item.</p></li>';
    turn.append(list);
    const p = list.querySelector('p')!;
    s.dispatchEvent(new WheelEvent('wheel', { deltaY: -100, bubbles: true }));
    s.scrollTop += p.getBoundingClientRect().top - s.getBoundingClientRect().top - 40;
  });
  await page.waitForTimeout(300);
  const at = await markedTop(page);
  await page.locator('[data-growing-list]').evaluate(el => (el as HTMLElement).style.height = '500px');
  await page.waitForTimeout(300);
  expect(Math.abs(await markedTop(page) - at)).toBeLessThan(3);
});

test('an unchanged-position scroll notification after growth preserves the reading reference', async ({ page }) => {
  await readMiddle(page);
  const at = await markReading(page);
  await page.evaluate(() => {
    const s = document.querySelector<HTMLElement>('.work-conversation .chat-scroll')!;
    const above = s.querySelector<HTMLElement>('.turn[data-turn-index="1"] .chat-canvas')!;
    above.style.paddingBottom = '300px';
    // WebKit can deliver an already-queued scroll notification at the SAME
    // scrollTop after new layout. It is not a new reader destination.
    s.dispatchEvent(new Event('scroll'));
  });
  await page.waitForTimeout(300);
  expect(Math.abs(await markedTop(page) - at)).toBeLessThan(3);
});
