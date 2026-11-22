import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Locator, type Page } from './fixtures/test';
import { showSettingsPart } from './settingsPart';

/**
 * Keyboard and accessibility checks, separate from the click-only journeys
 * (plan completion L5, review U "Accessibility" and matrix V7c): a task
 * created, opened and completed with the keyboard alone; a recording read
 * with the keyboard alone; focus that comes back after an asynchronous
 * dialog; a menu that opens a modal dialog, which keeps global shortcuts out
 * and gives focus back to the menu's button when it closes; and nested
 * overlays (H12): the palette over the Main agent page, a confirm over
 * Settings, where Escape closes only the top layer and focus goes back to
 * what opened it; a palette row that doesn't navigate, which gives focus
 * back the same way, while one that does leaves it in the new view (A05).
 * Every stop is reached with Tab (and Shift+Tab), Enter, Space, arrows and
 * Escape: no clicks. The canvas keyboard review is canvas-a11y.spec.ts.
 *
 * Chromium only: VoiceOver and WKWebView focus are native checks
 * (docs/verification-2026-10-14/NATIVE_PLAN.md, VN9).
 */

test.beforeEach(({}, info) => { test.skip(info.project.name !== 'chromium-desktop', 'keyboard paths at the desktop size; the small sizes run the click-only journeys'); });

/** Press Tab (or Shift+Tab) until `target` has focus; fail after `max` presses. */
async function tabTo(page: Page, target: Locator, { back = false, max = 120 } = {}) {
  const handle = await target.elementHandle();
  for (let i = 0; i < max; i++) {
    if (await page.evaluate(el => el === document.activeElement, handle)) return i;
    await page.keyboard.press(back ? 'Shift+Tab' : 'Tab');
  }
  throw new Error(`${target} never took focus within ${max} ${back ? 'Shift+Tab' : 'Tab'} presses`);
}

/** The focused element shows a visible focus indicator (outline or ring). */
async function expectVisibleFocus(page: Page) {
  const ring = await page.evaluate(() => {
    const el = document.activeElement as HTMLElement | null;
    if (!el || el === document.body) return 'nothing focused';
    const s = getComputedStyle(el);
    const outline = s.outlineStyle !== 'none' && parseFloat(s.outlineWidth) > 0;
    const shadow = s.boxShadow && s.boxShadow !== 'none';
    return outline || shadow ? 'ok' : `no focus ring on ${el.outerHTML.slice(0, 120)}`;
  });
  expect(ring).toBe('ok');
}

async function axe(page: Page, include: string) {
  const { violations } = await new AxeBuilder({ page }).include(include).analyze();
  return violations.filter(v => v.impact === 'serious' || v.impact === 'critical').map(v => `${v.id}: ${v.nodes.map(n => n.target.join(' ')).join(', ')}`);
}

test('keyboard only: create a task, open it, complete it', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await expect(page.locator('.home-project-row').first()).toBeVisible();
  await page.keyboard.press('Meta+2');
  await expect(page.getByRole('heading', { name: 'Tasks', level: 1 })).toBeVisible();
  await tabTo(page, page.getByRole('button', { name: 'New local task', exact: true }));
  await expectVisibleFocus(page);
  await page.keyboard.press('Enter');
  // The editor takes focus on its title; type, then Tab to Create task.
  await expect.poll(() => page.evaluate(() => document.activeElement?.tagName)).toMatch(/^(INPUT|TEXTAREA)$/);
  await page.keyboard.type('Call the Garage 14 landlord');
  await tabTo(page, page.getByRole('button', { name: 'Create task' }));
  await expectVisibleFocus(page);
  await page.keyboard.press('Enter');
  // (The browser mock records a create without adding it to the demo list.)
  await expect.poll(() => page.evaluate(() => ((window as unknown as { __mockCreatedTasks?: { title?: string }[] }).__mockCreatedTasks ?? []).map(t => t.title))).toContain('Call the Garage 14 landlord');
  // Back through the list to a task, open it, then complete it from the list.
  const open = page.locator('.issue-row.task-row').filter({ hasText: 'Send the draft to Dana for review' }).locator('.issue-row-main');
  await tabTo(page, open, { back: true, max: 200 });
  await expectVisibleFocus(page);
  await page.keyboard.press('Enter');
  await expect(open).toHaveAttribute('aria-current', 'true');
  await tabTo(page, page.getByRole('button', { name: 'Mark Send the draft to Dana for review done' }), { back: true, max: 200 });
  await expectVisibleFocus(page);
  await page.keyboard.press('Enter');
  await expect(open).toHaveCount(0);
  expect(await axe(page, 'main')).toEqual([]);
});

test('keyboard only: read a recording, its summary and its transcript', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await expect(page.locator('.home-project-row').first()).toBeVisible();
  await page.keyboard.press('Meta+3');
  // Recordings is All → Meetings → Recordings (overnight 11-12 L4).
  const all = page.getByRole('region', { name: 'All', exact: true });
  await tabTo(page, all.getByRole('group', { name: 'Library filter' }).getByRole('button', { name: 'Meetings', exact: true }));
  await page.keyboard.press('Enter');
  await tabTo(page, all.getByRole('button', { name: 'Recordings', exact: true }));
  await page.keyboard.press('Enter');
  // The list is a listbox: one Tab stop (the selected recording), then the
  // arrows move the selection (they did nothing before the L5 fix).
  const list = page.getByRole('listbox', { name: 'Recordings' });
  await tabTo(page, list.getByRole('option', { selected: true }));
  await expectVisibleFocus(page);
  await page.keyboard.press('ArrowDown');
  await expect(list.getByRole('option', { name: /Vendor security call/ })).toBeFocused();
  await expect(list.getByRole('option', { name: /Vendor security call/ })).toHaveAttribute('aria-selected', 'true');
  await page.keyboard.press('End');
  await expect(list.getByRole('option').last()).toBeFocused();
  await page.keyboard.press('Home');
  await expect(list.getByRole('option').first()).toBeFocused();
  await page.keyboard.press('ArrowDown');
  await expect(list.getByRole('option', { name: /Vendor security call/ })).toBeFocused();
  const reader = page.getByRole('region', { name: 'Recording' });
  await expect(reader.getByRole('heading', { level: 1 })).toHaveText('Vendor security call');
  // Back up to the one with a transcript.
  await page.keyboard.press('ArrowUp');
  await expect(reader.getByRole('heading', { level: 1 })).toHaveText('Weekly ops sync');
  // The transcript's own search is reachable and filters by typing.
  const find = reader.getByRole('searchbox', { name: 'Find in transcript' });
  await tabTo(page, find, { max: 60 });
  await expectVisibleFocus(page);
  await page.keyboard.type('vendor');
  await expect(reader.getByRole('region', { name: 'Transcript' })).toBeVisible();
  expect(await axe(page, 'main')).toEqual([]);
});

test('focus comes back to the row after an asynchronous dialog closes', async ({ page }) => {
  // The resume confirm opens only after Hollow re-reads the interrupted chats.
  await page.goto('/?mock=1&demo=1&scenario=interrupted');
  await expect(page.locator('.home-project-row').first()).toBeVisible();
  // U3: the top bar's bell, then its See all needs you (the rail's Review
  // row went), both by keyboard.
  const bell = page.locator('.top-bar').getByRole('button', { name: 'Notifications', exact: true });
  await tabTo(page, bell);
  await page.keyboard.press('Enter');
  const seeAll = page.getByRole('dialog', { name: 'Notifications', exact: true }).getByRole('button', { name: 'See all needs you', exact: true });
  await tabTo(page, seeAll);
  await page.keyboard.press('Enter');
  const row = page.getByRole('region', { name: 'Waiting on you' }).getByRole('button', { name: /Resume 1 interrupted chat/ });
  await tabTo(page, row);
  await page.keyboard.press('Enter');
  const dialog = page.getByRole('dialog');
  await expect(dialog).toContainText('Revise the labour section');
  // Focus is inside the dialog, and Tab stays there.
  expect(await page.evaluate(() => !!document.activeElement?.closest('[role=dialog]'))).toBe(true);
  await page.keyboard.press('Tab');
  expect(await page.evaluate(() => !!document.activeElement?.closest('[role=dialog]'))).toBe(true);
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  // Back on the row that opened it (the list refreshes after the dialog), not on the page body.
  await expect(row).toBeFocused();
  const sent = await page.evaluate(() => (window as unknown as { __mockApiCalls?: Record<string, number> }).__mockApiCalls?.resume_interrupted_chats ?? 0);
  expect(sent, 'Escape resumes nothing').toBe(0);
});

test('a menu that opens a modal: Escape closes the modal, a global shortcut stays out of it, focus returns to the opener', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await expect(page.locator('.home-project-row').first()).toBeVisible();
  // Library actions (a menu) → Add (a dialog): the generic Add's visible home
  // since the top bar's New menu went (overnight 11-04 U1). Library first,
  // from the sidebar, by keyboard.
  const library = page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Library', exact: true });
  await tabTo(page, library);
  await page.keyboard.press('Enter');
  await expect(page.getByRole('heading', { name: 'Library', level: 1 })).toBeVisible();
  const actions = page.getByRole('button', { name: 'Library actions', exact: true });
  await tabTo(page, actions);
  await page.keyboard.press('Enter');
  const item = page.getByRole('menuitem', { name: 'Add a task, note, link, file or recording…' });
  await expect(item).toBeVisible();
  // A menu moves with the arrows.
  for (let i = 0; i < 10 && !(await item.evaluate(el => el === document.activeElement)); i++) await page.keyboard.press('ArrowDown');
  await expect(item).toBeFocused();
  await page.keyboard.press('Enter');
  const add = page.getByRole('dialog', { name: 'Add' });
  await expect(add).toBeVisible();
  await expect(add.getByRole('textbox', { name: 'What to add' })).toBeFocused();
  // Inside it, a global shortcut doesn't open another overlay on top of the
  // modal (shortcuts respect modals): ⌘K leaves the dialog and its text alone.
  await page.keyboard.type('Ideas for the offsite');
  await page.keyboard.press('Meta+k');
  await page.waitForTimeout(300);
  await expect(page.getByPlaceholder(/Find a conversation/)).toHaveCount(0);
  await expect(add.getByRole('textbox', { name: 'What to add' })).toBeFocused();
  // The Add dialog kept its text, and its own Escape closes it.
  await expect(add.getByRole('textbox', { name: 'What to add' })).toHaveValue('Ideas for the offsite');
  await page.keyboard.press('Escape');
  await expect(add).toHaveCount(0);
  // Focus returns to the menu that opened it, Library actions, not to the page body.
  await expect.poll(() => page.evaluate(() => document.activeElement === document.body ? 'body' : document.activeElement?.closest('details')?.querySelector('summary')?.getAttribute('aria-label') ?? document.activeElement?.outerHTML.slice(0, 80))).toBe('Library actions');
});

// Nested overlays (H12, matrix V7c): one Escape closes only the top layer,
// the layer under it stays open, and focus goes back to what opened the top
// layer, never to the page body.

/** The Main agent page, opened with ⌘⇧M (the key of the drawer it replaced
 *  in run 15 P3); focus in its message box. */
async function openMainPage(page: Page) {
  await page.goto('/?mock=1');
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await page.keyboard.press('Meta+Shift+M');
  const main = page.locator('.work-conversation');
  await expect(main).toBeVisible();
  const box = main.getByRole('textbox', { name: 'Chat message' });
  await expect(box).toBeFocused();
  return { main, box };
}

test('nested overlays: the palette over the COO page closes alone and gives focus back to its box', async ({ page }) => {
  const { main, box } = await openMainPage(page);
  await page.keyboard.type('Draft for the COO');
  await page.keyboard.press('Meta+k');
  const palette = page.getByRole('dialog', { name: 'Command palette' });
  await expect(palette.getByRole('combobox', { name: 'Search' })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(palette).toHaveCount(0);
  await expect(main).toBeVisible();
  // Back in the page's box with its draft (it fell to the body before H12).
  await expect(box).toBeFocused();
  await expect(box).toHaveValue('Draft for the COO');
});

// A05: running a palette row is not a dismissal, but a row that leaves the
// person where they were (Clear debug logs: an async call and a footer note,
// no page, workspace or view change) gives focus back like Escape does; a row
// that navigates (Tasks) leaves focus to the new view.

test('palette over the COO page: a row that does not navigate gives focus back to its box', async ({ page }) => {
  const { main, box } = await openMainPage(page);
  await page.keyboard.type('Draft for the COO');
  await page.keyboard.press('Meta+k');
  const palette = page.getByRole('dialog', { name: 'Command palette' });
  await expect(palette.getByRole('combobox', { name: 'Search' })).toBeFocused();
  await page.keyboard.type('Clear debug logs');
  await expect(palette.getByRole('option', { selected: true })).toHaveText(/^\s*Clear debug logs/);
  await page.keyboard.press('Enter');
  await expect(palette).toHaveCount(0);
  await expect(page.locator('.snackbar').filter({ hasText: 'No debug logs to clear' })).toBeVisible();
  await expect(main).toBeVisible();
  // Back in the page's box with its draft (it fell to the body before A05).
  await expect(box).toBeFocused();
  await expect(box).toHaveValue('Draft for the COO');
});

test('palette from a button that stays on screen: a row that navigates leaves focus to the new view', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  // The main navigation stays on every page, so the opener is still there
  // after the row navigates: only the location stamp keeps focus from going back.
  const opener = page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true });
  await opener.focus();
  await page.keyboard.press('Meta+k');
  const palette = page.getByRole('dialog', { name: 'Command palette' });
  await expect(palette.getByRole('combobox', { name: 'Search' })).toBeFocused();
  await page.keyboard.type('Tasks');
  await expect(palette.getByRole('option', { selected: true })).toHaveText(/^\s*Tasks\b/);
  await page.keyboard.press('Enter');
  await expect(palette).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'Tasks', level: 1 })).toBeVisible();
  // Past the return's 180 ms wait and its frame: focus was not pulled back.
  await page.waitForTimeout(500);
  await expect(opener).toBeVisible();
  await expect(opener).not.toBeFocused();
});

test('nested overlays: a confirm over Settings cancels alone; Settings stays and focus is back on the button', async ({ page }) => {
  await page.goto('/?mock=1');
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await page.keyboard.press('Meta+Comma');
  const heading = page.getByRole('heading', { name: 'Settings', level: 1, exact: true });
  await expect(heading).toBeVisible();
  // Accounts is a section of its own since Settings PR 3.
  const agents = page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Accounts', exact: true });
  await tabTo(page, agents, { max: 200 });
  await page.keyboard.press('Enter');
  await expect(agents).toHaveAttribute('aria-current', 'page');
  // Brings Accounts into view; scrolling moves no focus.
  await showSettingsPart(page, 'accounts');
  const remove = page.locator('.claude-acct').filter({ hasText: 'Second Max plan' }).getByRole('button', { name: 'Remove', exact: true });
  await tabTo(page, remove, { max: 400 });
  await page.keyboard.press('Enter');
  const confirm = page.getByRole('dialog', { name: 'Remove Second Max plan?' });
  // A danger confirm starts on Cancel.
  await expect(confirm.getByRole('button', { name: 'Cancel', exact: true })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(confirm).toHaveCount(0);
  await expect(heading).toBeVisible();
  await expect(agents).toHaveAttribute('aria-current', 'page');
  await expect(remove).toBeFocused();
  const removed = await page.evaluate(() => (window as unknown as { __mockApiCalls?: Record<string, number> }).__mockApiCalls?.remove_claude_account ?? 0);
  expect(removed, 'Escape removes nothing').toBe(0);
});

// The orb is the Main agent's visible route; its page holds the
// conversation (the drawer beside other pages went in run 15 P3).
test('keyboard only: the orb opens the COO page with focus in its box', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockWorkspaces', JSON.stringify([{ id: 'main', kind: 'main', title: 'COO', status: 'active', path: 'workspaces/main', cwd: '/Users/fixture/vault/workspaces/main', modified: '2026-10-12T04:00:00Z' }])));
  await page.goto('/?mock=1');
  const orb = page.locator('.top-bar').getByRole('button', { name: /^COO ·/ });
  await tabTo(page, orb, { max: 200 });
  await page.keyboard.press('Enter');
  await expect(page.locator('.work-conversation')).toBeVisible();
  await expect(page.locator('.work-conversation').getByRole('textbox', { name: 'Chat message' })).toBeFocused();
  await expect(page.getByRole('complementary', { name: 'COO drawer' })).toHaveCount(0);
});
