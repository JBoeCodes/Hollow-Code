import { openTasks, showWorkspaceList, openProjectFiles, closeProjectFiles, paneCovers, previewPane, projectFilesButton } from './choose';
import { expect, test, type Locator, type Page } from './fixtures/test';
import { openLibraryView } from './libraryView';

/**
 * Review F07 (2026-10-13): the required supported-minimum journeys. Only the
 * chromium-minimum (720 × 480, the native window's minimum) and
 * chromium-minimum-large-text (720 × 480, Largest text) projects run this
 * file, and nothing in it may skip (scripts/check-required-specs.mjs).
 * Every step uses visible controls, the way a person with a mouse would.
 */

async function noHorizontalScroll(page: Page) {
  const [scroll, client] = await page.evaluate(() => [document.scrollingElement!.scrollWidth, document.scrollingElement!.clientWidth]);
  expect(scroll, 'no horizontal page scroll').toBeLessThanOrEqual(client);
}
/** Nothing in `target` is wider than its box (H14). The shell's `.canvas`
 *  clips, so a top bar or a pane scroller that overflows sideways never
 *  shows in the document's scroller: `noHorizontalScroll` can't see it. */
async function fitsSideways(target: Locator, name: string) {
  await expect(target, name).toBeVisible();
  const [scroll, client] = await target.evaluate(el => [el.scrollWidth, el.clientWidth]);
  // One pixel of slack: a fractional box width (Largest text) can round scrollWidth up.
  expect(scroll, `${name}: nothing wider than its box`).toBeLessThanOrEqual(client + 1);
}
/** The control is on screen, not clipped by the window. */
async function onScreen(page: Page, target: Locator) {
  await target.scrollIntoViewIfNeeded();
  const box = (await target.boundingBox())!;
  const vp = page.viewportSize()!;
  expect(box.x, 'left edge on screen').toBeGreaterThanOrEqual(0);
  expect(box.x + box.width, 'right edge on screen').toBeLessThanOrEqual(vp.width + 0.5);
  expect(box.y + box.height, 'bottom edge on screen').toBeLessThanOrEqual(vp.height + 0.5);
}
async function go(page: Page, name: 'Projects' | 'Library' | 'Automations') {
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name, exact: true }).click();
}

test.beforeEach(async ({ page }) => { await page.goto('/?mock=1&demo=1'); });

test('the text size the project asks for is in effect', async ({ page }, info) => {
  const scale = await page.evaluate(() => document.documentElement.dataset.textScale);
  expect(scale).toBe(info.project.name.includes('large-text') ? '125' : '100');
  await noHorizontalScroll(page);
});

test('Tasks: open a task, edit it, return to the list', async ({ page }) => {
  // Tasks opens from Work's view actions since Sam's sidebar port.
  await openTasks(page);
  await noHorizontalScroll(page);
  await page.locator('.issue-row.task-row').filter({ hasText: 'Draft airport pricing options' }).locator('.issue-row-main').click();
  const detail = page.getByRole('complementary', { name: 'Task details' });
  await expect(detail).toBeVisible();
  await noHorizontalScroll(page);
  await detail.getByRole('button', { name: 'Close task details' }).click();
  await expect(page.locator('.issue-row.task-row').filter({ hasText: 'Draft airport pricing options' })).toBeVisible();

  await noHorizontalScroll(page);
});

test('Library: choose a note, read it, and come back', async ({ page }) => {
  await go(page, 'Library');
  await openLibraryView(page, 'Notes');
  await noHorizontalScroll(page);
  await page.getByRole('button', { name: /^Weekly ops sync 0/ }).first().click();
  await expect(page.getByText('Fixture vault note.').first()).toBeVisible();
  await onScreen(page, page.getByRole('button', { name: 'Edit', exact: true }));
  await noHorizontalScroll(page);
  await page.getByRole('button', { name: /All files/ }).click();
  await expect(page.getByRole('button', { name: /^Weekly ops sync 0/ }).first()).toBeVisible();
});

test('Library: edit a note, save it, and the saved text is what you read after coming back', async ({ page }) => {
  await go(page, 'Library');
  await openLibraryView(page, 'Notes');
  await page.getByRole('button', { name: /^Weekly ops sync 0/ }).first().click();
  const reader = page.getByRole('region', { name: 'File reader' });
  const state = reader.locator('.work-file-state');
  await expect(reader.getByText('Fixture vault note.').first()).toBeVisible();
  const edit = reader.getByRole('button', { name: 'Edit', exact: true });
  await onScreen(page, edit);
  await edit.click();
  const editor = reader.locator('.cm-content[contenteditable=true]');
  await expect(editor).toBeVisible();
  await noHorizontalScroll(page);
  await editor.click();
  await page.keyboard.press('ControlOrMeta+a');
  await page.keyboard.type('Edited at the smallest window.');
  await expect(state).toHaveAttribute('data-state', 'draft');
  const save = reader.getByRole('button', { name: 'Save', exact: true });
  await onScreen(page, save);
  await save.click();
  // Saved, not a conflict or a draft still waiting.
  await expect(state).toHaveAttribute('data-state', /^(saved|editing)$/);
  await noHorizontalScroll(page);
  const back = reader.getByRole('button', { name: '← All files', exact: true });
  await onScreen(page, back);
  await back.click();
  await page.getByRole('button', { name: /^Weekly ops sync 0/ }).first().click();
  // Reopened from the file: reading, no editor, no restored draft, the saved words.
  await expect(state).toHaveAttribute('data-state', 'viewing');
  await expect(reader.locator('.cm-content')).toHaveCount(0);
  await expect(reader.locator('.work-file-restored')).toHaveCount(0);
  await expect(reader.getByText('Fixture vault note.')).toHaveCount(0);
  await expect(reader.getByText('Edited at the smallest window.').first()).toBeVisible();
});

test('Settings: the section picker searches controls and opens the section', async ({ page }) => {
  await page.getByRole('button', { name: /Settings/ }).first().click();
  const picker = page.getByRole('combobox', { name: 'Settings section' });
  await picker.click();
  const pop = page.locator('.hs-pop');
  await expect(pop).toBeVisible();
  await page.keyboard.type('text size');
  await pop.getByRole('option', { name: /^General/ }).click();
  await expect(page.getByRole('group', { name: 'Text size' }).or(page.getByRole('radiogroup', { name: 'Text size' })).first()).toBeVisible();
  await noHorizontalScroll(page);
});

test('Workspace: Project files opens beside the conversation and closing it brings the draft back', async ({ page }) => {
  await go(page, 'Projects');
  await showWorkspaceList(page);
  await page.getByRole('region', { name: 'Projects' }).locator('.work-card').first().click();
  const composer = page.getByLabel('Chat message');
  await composer.fill('Keep this draft');
  // COO rebuild L4: the top bar's Project files (it replaced the workspace
  // panel button) opens the project's WORKSPACE.md in the Preview pane. The
  // native minimum has no room beside the chat, so the pane covers the page,
  // the conversation mounted and hidden under it.
  await openProjectFiles(page);
  await expect(previewPane(page).getByRole('tab', { name: 'WORKSPACE.md', exact: true })).toHaveAttribute('aria-selected', 'true');
  if (await paneCovers(page)) await expect(page.getByRole('region', { name: 'Workspace conversation' })).toBeHidden();
  await noHorizontalScroll(page);
  await fitsSideways(page.locator('.top-bar'), 'the top bar');
  await fitsSideways(previewPane(page), 'the Preview pane');
  // Project files, pressed, is on screen and returns to the same chat, draft and all.
  await onScreen(page, projectFilesButton(page));
  await closeProjectFiles(page);
  await expect(page.getByLabel('Chat message')).toHaveValue('Keep this draft');
});

/* ── H14 (2026-10-15): the top bar's global controls and the question dock ── */

type Box = { x: number; y: number; width: number; height: number };
/** Two boxes share no area (touching edges is fine). */
const apart = (a: Box, b: Box) => a.x + a.width <= b.x + 0.5 || b.x + b.width <= a.x + 0.5 || a.y + a.height <= b.y + 0.5 || b.y + b.height <= a.y + 0.5;
/** `inner` is drawn inside `outer`: not cut off by it. */
async function within(inner: Locator, outer: Locator) {
  const i = (await inner.boundingBox())!;
  const o = (await outer.boundingBox())!;
  expect(i.x, 'left edge inside').toBeGreaterThanOrEqual(o.x - 0.5);
  expect(i.x + i.width, 'right edge inside').toBeLessThanOrEqual(o.x + o.width + 0.5);
  expect(i.y, 'top edge inside').toBeGreaterThanOrEqual(o.y - 0.5);
  expect(i.y + i.height, 'bottom edge inside').toBeLessThanOrEqual(o.y + o.height + 0.5);
}

/** The Main agent orb: on screen and inside the bar; and no two of the
 *  bar's controls (the breadcrumb side and an open workspace's own buttons
 *  included) overlap. The orb's two badges are put in by hand, as
 *  MainAgentOrb / SessionIndicator draw them, because the mock's Main agent
 *  waits on nothing: the waiting count reaches past the orb. The bell that
 *  sat beside it went with P1-14 (the rail's Review row carries the count). */
async function topBarClear(page: Page) {
  const bar = page.locator('.top-bar');
  const vp = page.viewportSize()!;
  const barBox = (await bar.boundingBox())!;
  const orb = bar.locator('.main-agent-orb');
  await expect(orb, 'orb').toBeVisible();
  const box = (await orb.boundingBox())!;
  expect(box.x, 'orb: left edge on screen').toBeGreaterThanOrEqual(0);
  expect(box.x + box.width, 'orb: right edge on screen').toBeLessThanOrEqual(vp.width + 0.5);
  expect(box.y, 'orb: inside the bar').toBeGreaterThanOrEqual(barBox.y - 0.5);
  expect(box.y + box.height, 'orb: inside the bar').toBeLessThanOrEqual(barBox.y + barBox.height + 0.5);
  // U3: one bell, named Notifications, inside the bar; no separate Review entry.
  await expect(bar.getByRole('button', { name: /^Review/ }), 'no Review entry in the bar').toHaveCount(0);
  const bell = bar.getByRole('button', { name: 'Notifications', exact: true });
  await expect(bell, 'one bell (U3)').toHaveCount(1);
  await expect(bell, 'bell').toBeVisible();
  const bellBox = (await bell.boundingBox())!;
  expect(bellBox.x + bellBox.width, 'bell: right edge on screen').toBeLessThanOrEqual(vp.width + 0.5);
  expect(bellBox.y + bellBox.height, 'bell: inside the bar').toBeLessThanOrEqual(barBox.y + barBox.height + 0.5);

  const controls = await bar.evaluate(el => [...el.querySelectorAll<HTMLElement>('button, summary')]
    .filter(b => !b.parentElement?.closest('button, summary') && !b.closest('.studio-action-menu-body'))
    .map(b => ({ name: b.getAttribute('aria-label') ?? b.textContent?.trim() ?? b.className, r: b.getBoundingClientRect() }))
    .filter(c => c.r.width >= 2 && c.r.height >= 2)
    .map(c => ({ name: c.name, x: c.r.x, y: c.r.y, width: c.r.width, height: c.r.height })));
  for (let i = 0; i < controls.length; i++) for (let j = i + 1; j < controls.length; j++) {
    expect(apart(controls[i], controls[j]), `"${controls[i].name}" and "${controls[j].name}" overlap`).toBe(true);
  }
  for (const c of controls) expect(c.x + c.width, `"${c.name}" on screen`).toBeLessThanOrEqual(vp.width + 0.5);

  const badges = await bar.locator('.main-agent-orb').evaluate(orb => {
    const waiting = document.createElement('span'); waiting.className = 'main-orb-count'; waiting.textContent = '99+';
    const live = document.createElement('span'); live.className = 'main-agent-orb-count'; live.textContent = '9';
    orb.querySelector('.main-indicator')!.append(waiting); orb.append(live);
    const r = [waiting, live].map(b => { const x = b.getBoundingClientRect(); return { x: x.x, y: x.y, width: x.width, height: x.height }; });
    waiting.remove(); live.remove();
    return r;
  });
  // The orb's widest badges stay on screen and off an open workspace's
  // Project files button (lane H review of H14; it sits where the workspace
  // panel button was, COO rebuild L4).
  const toggle = projectFilesButton(page);
  for (const b of badges) {
    expect(b.x + b.width, 'the orb\'s count on screen').toBeLessThanOrEqual(vp.width + 0.5);
    if (await toggle.isVisible()) expect(apart(b, (await toggle.boundingBox())!), 'the orb\'s count stays off Project files').toBe(true);
  }
  await fitsSideways(bar, 'the top bar');
  await noHorizontalScroll(page);
}

test('Top bar: the COO orb and its counts stay on screen at Home, in a workspace and in the COO', async ({ page }) => {
  await expect(page.locator('.top-bar-global')).toBeVisible();
  await topBarClear(page);

  // An open workspace adds its own buttons on the left and Project files on the right.
  await go(page, 'Projects');
  await showWorkspaceList(page);
  await page.getByRole('region', { name: 'Projects' }).locator('.work-card').first().click();
  await expect(page.locator('.top-bar-crumbs .crumb-project')).toBeVisible();
  await expect(projectFilesButton(page)).toBeVisible();
  await topBarClear(page);
  if (process.env.HOLLOW_FINAL_TOPBAR_CAPTURE) await page.screenshot({ path: `${process.env.HOLLOW_FINAL_TOPBAR_CAPTURE}-${await page.evaluate(() => document.documentElement.dataset.textScale)}.png` });
  // Check both sides of the compact branch treatment, including Largest
  // text: fixed controls must fit when the branch label becomes visible.
  const originalViewport = page.viewportSize()!;
  for (const width of [861, 1076]) {
    await page.setViewportSize({ width, height: originalViewport.height });
    await topBarClear(page);
  }
  await page.setViewportSize(originalViewport);

  // The Main agent: the orb pressed.
  await page.locator('.top-bar .main-agent-orb').click();
  await expect(page.locator('.top-bar .main-agent-orb')).toHaveAttribute('aria-pressed', 'true');
  await topBarClear(page);
  // Its actions stay on screen in the top bar (the page header band went 11-11).
  await onScreen(page, page.locator('.top-bar').getByRole('button', { name: 'Actions for COO', exact: true }));
});

test('Question dock: it takes the message box\'s place, its buttons stay on screen, and Hide leaves the waiting chip', async ({ page }) => {
  await go(page, 'Projects');
  await showWorkspaceList(page);
  await page.getByRole('region', { name: 'Projects' }).locator('.work-card').first().click();
  const composer = page.getByLabel('Chat message');
  await expect(composer).toBeVisible();
  // The chat's name takes the breadcrumb's place in the top bar's one header
  // row since the top-bar polish (#1492).
  await expect(page.locator('.top-bar .chat-topbar-controls .chat-title')).toBeVisible();
  // Two questions (as tests/smoke/question-dock.spec.ts puts them): a single
  // choice with "Other…", then a multiple choice, so Back, Next and Send all show.
  await page.evaluate(async ([url]) => {
    const rt = await import(url);
    const key: string = rt.chatAgentSessionKey() ?? 'minimum-dock';
    const card = {
      id: 'int-dock-min', sessionKey: key, turnOrd: 1, runId: 'run-dock-min', agent: 'claude',
      nativeSessionId: 'native-dock-min', toolUseId: 'tu-min', source: 'mcp', mode: 'questions', status: 'pending',
      request: { mode: 'questions', title: 'Morning check-in', supersedeOnUserMessage: true, questions: [
        { id: 'install', header: 'Install main', prompt: 'Install main 4f196bf4 now?', required: true, answerMode: 'single', allowCustom: true,
          options: [{ id: 'do-it', label: 'Do it' }, { id: 'not-now', label: 'Not now', recommended: true }, { id: 'never', label: 'Never' }] },
        { id: 'checks', header: 'Hand checks', prompt: 'Which hand checks first?', required: false, answerMode: 'multi', allowCustom: false,
          options: [{ id: 'voiceover', label: 'VoiceOver' }, { id: 'glass', label: 'Glass' }, { id: 'trackpad', label: 'Trackpad' }] },
      ] },
      answer: null, createdAt: '2026-10-15T12:00:00Z', resolvedAt: null,
    };
    localStorage.setItem('mockPendingInteractions', JSON.stringify([card]));
    // A demo workspace may have no conversation yet: give it a key.
    if (!rt.chatAgentSessionKey()) rt.setChatAgentSessionKeyPersistent(key);
    rt.setChatSessionId('native-dock-min');
    await rt.refreshChatInteractions();
  }, ['/src/store/chatRuntime.ts'] as const);

  const dock = page.locator('.question-dock');
  await expect(dock).toBeVisible();
  await expect(composer).toBeHidden();
  await expect(dock.locator('.question-dock-count')).toHaveText('1 of 2');
  await onScreen(page, dock);
  await noHorizontalScroll(page);
  const back = dock.getByRole('button', { name: 'Back', exact: true });
  const skip = dock.getByRole('button', { name: 'Don’t answer' });
  const other = dock.getByRole('option', { name: /Other…/ });
  for (const control of [back, skip, other, dock.getByRole('button', { name: 'Hide the questions' })]) {
    await expect(control).toBeVisible();
    await onScreen(page, control);
    await within(control, dock);
  }
  // "Other…" brings its field and Next.
  await other.click();
  await dock.getByRole('textbox').fill('After the demo');
  const next = dock.getByRole('button', { name: 'Next', exact: true });
  await onScreen(page, next);
  await within(next, dock);
  await next.click();
  await expect(dock.locator('.question-dock-count')).toHaveText('2 of 2');
  await expect(dock.locator('.question-dock-trail')).toContainText('After the demo');
  await expect(back).toBeEnabled();
  for (const control of [back, skip, dock.getByRole('button', { name: 'Send answers', exact: true })]) {
    await onScreen(page, control);
    await within(control, dock);
  }
  await noHorizontalScroll(page);

  // Hidden: the message box is back with the chip over it.
  await dock.getByRole('button', { name: 'Hide the questions' }).click();
  await expect(dock).toBeHidden();
  await expect(composer).toBeVisible();
  const chip = page.getByRole('button', { name: '2 questions waiting' });
  await expect(chip).toBeVisible();
  await onScreen(page, chip);
  await noHorizontalScroll(page);
  await chip.click();
  await expect(dock.locator('.question-dock-count')).toHaveText('2 of 2');
  await skip.click();
  await expect(dock).toHaveCount(0);
  await expect(composer).toBeVisible();
});
