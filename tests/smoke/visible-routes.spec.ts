import { openTasks } from './choose';
/**
 * Visible routes (consolidation lane B, 2026-10-13): every consolidated
 * destination, view and menu is reached by clicking what is on screen, with
 * no shortcut, no palette and no deep link (roadmap: "No shortcut-only
 * features"). One test per route family; the route ledger
 * (docs/CONSOLIDATION_LEDGER.md) cites these titles verbatim. Not a journey:
 * the click-only journeys (*.journey.spec.ts) cover the minimum window.
 * Against the demo mock (src/mockDemo.ts) unless a test seeds its own data.
 */
import { expect, test, type Page } from './fixtures/test';
import { choose, chooseNew, closeProjectFiles, filesPicker, openGlobalSearch, openProjectFile, optionValues, previewPane, projectFilesButton, sidebar, newChatControl } from './choose';
import { bellDialog, navigateWaiting } from './waiting';
import { seedSubagents } from './subagentsFixture';
import { openLibraryView } from './libraryView';

test.beforeEach(({}, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'At 390px the sidebar is an overlay and Settings sections are a selector; the journeys cover the minimum window');
});

const main = (page: Page) => page.getByRole('navigation', { name: 'Main' });
const railButton = (page: Page, name: string) => main(page).getByRole('button', { name, exact: true });
/** U3: the rail has no Review row; the bell's See all needs you is its route. */
const noReviewRow = (page: Page) => expect(main(page).getByRole('button', { name: /^(Review|Needs you)\b/ })).toHaveCount(0);
const settingsButton = (page: Page) => page.getByLabel('Main navigation').getByRole('button', { name: /^Settings/ }).first();
const calls = (page: Page, command: string) => page.evaluate(c => (window as unknown as { __mockApiCalls?: Record<string, number> }).__mockApiCalls?.[c] ?? 0, command);
async function demo(page: Page) {
  await page.goto('/?mock=1&demo=1');
  await expect(page.getByRole('region', { name: 'Projects' })).toBeVisible();
}

test('route: rail destinations open their pages', async ({ page }) => {
  await demo(page);
  for (const name of ['Library', 'Automations', 'Projects']) {
    await railButton(page, name).click();
    await expect(page.getByRole('heading', { name, level: 1, exact: true })).toBeVisible();
  }
  // Needs you (was Review): the bell, then See all needs you (U3).
  await noReviewRow(page);
  await navigateWaiting(page);
  await expect(bellDialog(page)).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'Needs you', level: 1, exact: true })).toBeVisible();
  await settingsButton(page).click();
  await expect(page.getByRole('heading', { name: 'Settings', level: 1, exact: true })).toBeVisible();
  await expect(page.getByRole('navigation', { name: 'Settings sections' })).toBeVisible();
  await openTasks(page);
  await expect(page.getByRole('heading', {name:'Tasks',level:1,exact:true})).toBeVisible();
});

test('route: Work views', async ({ page }) => {
  await demo(page);
  const views = page.getByRole('navigation', { name: 'Projects views' });
  await expect(views.getByRole('button')).toHaveCount(2);
  await views.getByRole('button', { name: 'Home', exact: true }).click();
  await expect(views.getByRole('button', { name: 'Home', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('region', { name: 'Projects' })).toBeVisible();
  await views.getByRole('button', { name: 'Running', exact: true }).click();
  await expect(views.getByRole('button', { name: 'Running', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('region', { name: 'Running', exact: true })).toBeVisible();
  await views.getByRole('button', { name: 'Home', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Projects' }).getByRole('button', { name: 'Map', exact: true })).toHaveCount(0);
  await expect(views.getByRole('button', { name: 'Today', exact: true })).toHaveCount(0);
  await expect(page.getByRole('group', { name: 'Today range' })).toHaveCount(0);
});

test('route: Review views', async ({ page }) => {
  await demo(page);
  await navigateWaiting(page);
  const filters = page.getByRole('navigation', { name: 'Waiting filters' });
  const tab = (name: string) => filters.getByRole('button', { name: new RegExp(`^${name}`) });
  const waiting = page.getByRole('region', { name: 'Waiting on you' });
  await expect(filters.getByRole('button')).toHaveCount(4);
  await expect(tab('All')).toHaveAttribute('aria-pressed', 'true');
  await expect(waiting.getByRole('region', { name: 'Review items and proposals' })).toBeVisible();
  await expect(waiting.getByRole('region', { name: 'Failures' })).toBeVisible();

  await tab('Decisions').click();
  await expect(tab('Decisions')).toHaveAttribute('aria-pressed', 'true');
  await expect(waiting.getByRole('region', { name: 'Review items and proposals' })).toBeVisible();
  await expect(waiting.getByRole('region', { name: 'Failures' })).toHaveCount(0);

  await tab('Results').click();
  await expect(tab('Results')).toHaveAttribute('aria-pressed', 'true');
  await expect(waiting).toBeVisible();
  await expect(waiting.getByRole('region', { name: 'Review items and proposals' })).toHaveCount(0);

  await tab('Failures').click();
  await expect(tab('Failures')).toHaveAttribute('aria-pressed', 'true');
  await expect(waiting.getByRole('region', { name: 'Failures' })).toContainText('Close my day failed');
  await expect(waiting.getByRole('region', { name: 'Review items and proposals' })).toHaveCount(0);

  await expect(filters.getByRole('button', { name: 'Incoming', exact: true })).toHaveCount(0);
  await noReviewRow(page);
  await expect(page.getByRole('heading', { name: 'Needs you', level: 1 })).toBeVisible();
});

test('route: Tasks list and Commitments', async ({ page }) => {
  await demo(page);
  await openTasks(page);
  await expect(page.locator('.issue-list')).toBeVisible();
  await expect(page.getByRole('navigation', { name: 'Tasks and commitments' })).toHaveCount(0);
  await expect(page.getByRole('region', { name: 'Commitments', exact: true })).toHaveCount(0);
  await expect(page.getByRole('textbox', { name: 'Find a task' })).toBeVisible();
});

test('route: Library → Results → Canvases', async ({ page }) => {
  await demo(page);
  await railButton(page, 'Library').click();
  // Results is All → Outputs → Grid (overnight 11-12 L4); All stays selected.
  await openLibraryView(page, 'Results');
  await expect(page.getByRole('navigation', { name: 'Library view' }).getByRole('button', { name: 'All', exact: true })).toHaveAttribute('aria-pressed', 'true');
  const type = page.getByRole('group', { name: 'Result type' });
  await type.getByRole('button', { name: 'Canvases', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Results', exact: true })).toBeVisible();
  await type.getByRole('button', { name: 'Files', exact: true }).click();
  await expect(type.getByRole('button', { name: 'Files', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('region', { name: 'Results', exact: true })).toBeVisible();
});

test('route: Ask scopes and the COO page', async ({ page }) => {
  await demo(page);
  // No Ask ⌄ in the top bar (removed 2026-10-21): Work's Ask field holds the scopes.
  await expect(page.getByRole('group', { name: 'Everywhere' }).getByRole('button', { name: /^Ask/ })).toHaveCount(0);
  const form = page.getByRole('form', { name: 'Ask', exact: true });
  const where = form.getByRole('combobox', { name: 'Where to ask' });
  const submit = form.locator('button[type="submit"]');
  // Two scopes, each a chat (One Ask, 2026-11-03), each saying what context
  // it uses and what pressing does, with its own placeholder and its context
  // as a muted line (P1-11).
  const scopes: [string, string, string, string][] = [
    ['main', 'Across all my work', 'The COO, on its page: every workspace, your tasks and calendar. Enter sends; ⌥-Enter only puts it in its box', 'Ask the COO'],
    ['quick', 'New chat', 'A new quick chat outside any project, listed under Quick chats (with Add to project, for that project’s lead); your profile and memory still apply', 'Start the chat'],
  ];
  expect(await optionValues(where)).toEqual(scopes.map(s => s[0]));
  const placeholders = new Set<string | null>();
  for (const [value, label, context, action] of scopes) {
    await choose(where, value);
    await expect(where).toContainText(label);
    await expect(submit).toHaveText(action);
    await expect(submit).toHaveAttribute('title', context);
    await expect(form.locator('#studio-ask-hint')).toHaveText(context);
    placeholders.add(await form.getByRole('textbox', { name: 'Ask anything' }).getAttribute('placeholder'));
  }
  expect(placeholders.size).toBe(scopes.length);

  // The Main agent page, not a drawer beside this one (run 15 P3): the orb
  // opens it with nothing sent, and its old Open beside your page is gone.
  await page.getByRole('button', { name: 'Open Q3 regional operations review', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Q3 regional operations review');
  const sent = await calls(page, 'chat_send');
  await page.getByRole('group', { name: 'Everywhere' }).getByRole('button', { name: /^COO ·/ }).click();
  await expect(page.locator('.work-conversation')).toBeVisible();
  await expect(page.locator('.work-conversation').getByRole('button', { name: /^Open beside/ })).toHaveCount(0);
  await expect(page.getByRole('complementary', { name: 'COO drawer' })).toHaveCount(0);
  expect(await calls(page, 'chat_send')).toBe(sent);
});

test('route: creation from the sidebar and Library', async ({ page }) => {
  await demo(page);
  // Workspaces the person made (the built-in COO one, which the mock makes
  // without a kind, appears on first use).
  const own = () => page.evaluate(() => (JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]') as { id: string; kind?: string }[]).filter(w => !w.kind && w.id !== 'main').map(w => w.id).sort());
  const workspacesBefore = await own();
  // Every creation control is on screen without opening a menu (overnight
  // 11-04 U1; the top bar's New menu held them): the sidebar's Recent +, and
  // New chat is the Quick chats + (back 2026-11-14; the orb is the COO's
  // page). A project's row opens its lead and has no + since L8 (11-10):
  // another conversation in it is its ⋯'s New conversation (visible-menus).
  // The top bar has no New menu.
  await expect(newChatControl(page)).toBeVisible();
  await expect(sidebar(page).getByRole('button', { name: 'New project', exact: true }).first()).toBeVisible();
  await expect(sidebar(page).getByRole('button', { name: /^Q3 regional operations review(,|$)/ })).toBeVisible();
  await expect(sidebar(page).getByRole('button', { name: /^New conversation( in |$)/ })).toHaveCount(0);
  await expect(page.getByRole('group', { name: 'Everywhere' }).getByRole('button', { name: /^New( |$)/ })).toHaveCount(0);
  await expect(sidebar(page).getByRole('button', { name: 'New chat', exact: true })).toHaveCount(0);

  // New chat opens a quick chat (2026-11-14), nothing sent.
  await chooseNew(page, 'New chat');
  await expect(page.locator('.work-conversation')).toBeVisible();
  await expect(page.getByLabel('Chat message')).toBeVisible();
  await railButton(page, 'Projects').click();
  // New project: its form, cancelled.
  await chooseNew(page, 'New project');
  await expect(page.getByRole('textbox', { name: 'Project name' })).toBeVisible();
  await expect(page.getByRole('textbox', { name: 'Project instructions' })).toBeVisible();
  await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(page.getByRole('textbox', { name: 'Project name' })).toHaveCount(0);
  // Add: its sheet, cancelled.
  await chooseNew(page, 'Add a task, note, link, file or recording…');
  const add = page.getByRole('dialog', { name: 'Add' });
  await expect(add.getByRole('textbox', { name: 'What to add' })).toBeVisible();
  await expect(add.getByRole('group', { name: 'Add as' })).toBeVisible();
  await add.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(add).toHaveCount(0);
  // Nothing was created or sent.
  expect(await own()).toEqual(workspacesBefore);
  expect(await calls(page, 'chat_send')).toBe(0);
});

test('route: sidebar search, sidebar, back and forward', async ({ page }) => {
  await demo(page);
  // Search: Search chats (the top bar's icon by the sidebar's toggle since
  // L1, 11-12) reaches the palette from its last row (overnight 11-04 U1; it
  // was the top bar's Search anything), which closes from its backdrop.
  await expect(page.locator('.top-bar-global').getByRole('button', { name: /^Search/ })).toHaveCount(0);
  const palette = await openGlobalSearch(page);
  await expect(palette.getByRole('combobox', { name: 'Search' })).toBeVisible();
  await page.locator('.palette-backdrop').click({ position: { x: 8, y: 8 } });
  await expect(palette).toHaveCount(0);

  // Sidebar: hide, show.
  const rail = page.getByRole('navigation', { name: 'Project navigation' });
  await expect(rail.getByRole('button', { name: 'Q3 regional operations review', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Hide sidebar', exact: true }).click();
  await expect(rail.getByRole('button', { name: 'Q3 regional operations review', exact: true })).toBeHidden();
  await page.getByRole('button', { name: 'Show sidebar', exact: true }).click();
  await expect(rail.getByRole('button', { name: 'Q3 regional operations review', exact: true })).toBeVisible();

  // Back and forward between destinations.
  const history = page.getByRole('group', { name: 'History' });
  await openTasks(page);
  await railButton(page, 'Library').click();
  await expect(page.getByRole('heading', { name: 'Library', level: 1 })).toBeVisible();
  await history.getByRole('button', { name: 'Back', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Tasks', level: 1 })).toBeVisible();
  await history.getByRole('button', { name: 'Forward', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Library', level: 1 })).toBeVisible();
  await expect(history.getByRole('button', { name: 'Forward', exact: true })).toBeDisabled();
});

test('route: Settings help, usage, connectors, appearance, transfer, diagnostics, history', async ({ page }) => {
  await demo(page);
  await settingsButton(page).click();
  const sections = page.getByRole('navigation', { name: 'Settings sections' });
  const section = (name: string) => sections.getByRole('button', { name, exact: true });

  // The Help row is gone (Settings PR 6, C3); Show all keys sits in Keyboard shortcuts' toolbar.
  await section('Keyboard shortcuts').click();
  await expect(page.getByRole('heading', { name: 'Help', level: 3 })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'What’s new', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Take the tour', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Show all keys', exact: true }).click();
  const keys = page.getByRole('dialog', { name: 'Keyboard shortcuts' });
  await expect(keys).toBeVisible();
  await page.locator('.keymap-backdrop').click({ position: { x: 8, y: 8 } });
  await expect(keys).toHaveCount(0);
  // Export and Import sit in Advanced's closed Settings file group, Save
  // diagnostics in its Diagnostics group (Settings PR 3).
  await section('Advanced').click();
  await page.locator('#settings-group-settings-file > summary').click();
  for (const name of ['Export settings…', 'Import settings…']) await expect(page.getByRole('button', { name, exact: true })).toBeVisible();
  await page.locator('#settings-group-diagnostics > summary').click();
  await expect(page.getByRole('button', { name: 'Save diagnostics…', exact: true })).toBeVisible();

  await section('Usage').click();
  await expect(page.getByRole('heading', { name: 'Usage', level: 2 })).toBeVisible();
  // Settings PR 8: Usage opens on Limits.
  await expect(page.getByRole('heading', { name: 'Spread usage', level: 3 })).toBeVisible();
  await section('Connectors').click();
  await expect(page.getByRole('heading', { name: 'Connectors', level: 2 })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Your connectors', level: 3 })).toBeVisible();
  await section('General').click();
  await expect(page.getByRole('heading', { name: 'Appearance', level: 3 })).toBeVisible();
  await expect(page.getByRole('combobox', { name: 'Theme', exact: true })).toBeVisible(); // one Theme menu since Settings PR 5

  await section('Advanced').click(); // History & recovery is in Advanced → This Mac (Settings PR 3)
  await page.locator('#settings-group-this-mac > summary').click();
  await page.getByRole('button', { name: 'Open Library, History', exact: true }).click();
  await page.locator('.library-tabs').getByRole('button', { name: 'Conversations', exact: true }).click(); // Settings PR 4 cut the Conversation history button
  // Library → Conversations since S4: the view's heading under Library's h1.
  await expect(page.getByRole('heading', { name: 'Conversation history', level: 2 })).toBeVisible();
  await expect(page.getByRole('navigation', { name: 'Library view' }).getByRole('button', { name: 'Conversations', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('complementary', { name: 'Conversations' })).toBeVisible();
});

test('route: Project files and the workspace menu', async ({ page }) => {
  await demo(page);
  await page.getByRole('button', { name: 'Open Q3 regional operations review', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Q3 regional operations review');

  // No Preview pane opens with the workspace; the top bar's Project files
  // opens the project's notes in it beside the conversation (COO rebuild L4:
  // it was the workspace panel and its sections).
  await expect(previewPane(page)).toHaveCount(0);
  await expect(projectFilesButton(page)).toHaveAttribute('aria-pressed', 'false');

  // Its Files picker reaches every note the project has.
  for (const name of ['WORKSPACE.md', 'PLAN.md', 'SOURCES.md']) {
    await openProjectFile(page, name);
    await expect(filesPicker(page)).toBeVisible();
  }

  // Project files, pressed, leaves the conversation alone, as it was.
  await closeProjectFiles(page);
  await expect(page.getByRole('region', { name: 'Workspace conversation' })).toBeVisible();

  // The open workspace's "⋯": the same menu as right-click, with Merge into….
  await page.locator('.top-bar').getByRole('button', { name: 'Actions for Q3 regional operations review' }).click();
  const menu = page.getByRole('menu');
  await expect(menu.getByRole('menuitem', { name: 'Merge into…' })).toBeEnabled();
  await expect(menu.getByRole('menuitem', { name: 'New conversation' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(menu).toHaveCount(0);
});

test('route: the Preview pane opens on a click and hides itself', async ({ page }) => {
  await demo(page);
  const preview = page.getByRole('complementary', { name: 'Preview' });
  await expect(preview).toHaveCount(0);
  // A reply's changed-file chip previews that file beside the conversation.
  await page.getByRole('button', { name: 'Open Results view for workspaces', exact: true }).click();
  await page.getByRole('list', { name: 'Changed files' }).getByRole('button', { name: /^ResultsView\.tsx/ }).first().click();
  await expect(preview).toBeVisible();
  await expect(preview.getByText('ResultsView.tsx').first()).toBeVisible();
  // × hides it, and nothing opens it again by itself.
  await preview.getByRole('button', { name: 'Close preview', exact: true }).click();
  await expect(preview).toHaveCount(0);
  // The same chip brings it back; Esc inside it hides it.
  await page.getByRole('list', { name: 'Changed files' }).getByRole('button', { name: /^ResultsView\.tsx/ }).first().click();
  await expect(preview).toBeVisible();
  await preview.focus();
  await page.keyboard.press('Escape');
  await expect(preview).toHaveCount(0);
});

test('route: the Subagents chip opens the chat’s Subagents tab', async ({ page }) => {
  await seedSubagents(page);
  await demo(page);
  await page.getByRole('button', { name: 'Open Q3 regional operations review', exact: true }).click();
  await page.locator('.side-card-toggle').filter({ hasText: 'Subagents' }).click();
  // Inside the summary: the counts; no subagent in the sidebar.
  const chip = page.getByRole('button', { name: /^Subagents:/ });
  await expect(chip).toHaveAccessibleName('Subagents: 1 running, 1 queued, 3 done');
  await expect(page.getByRole('navigation', { name: 'Project navigation' }).getByText('Check the labour figures')).toHaveCount(0);
  await chip.click();
  const preview = page.getByRole('complementary', { name: 'Preview' });
  await expect(preview.getByRole('tab', { name: 'Subagents' })).toHaveAttribute('aria-selected', 'true');
  const tab = preview.getByRole('tabpanel', { name: 'Subagents' });
  await expect(tab.getByRole('heading', { name: 'Active · 2' })).toBeVisible();
  await expect(tab.locator('.subagents-done > summary')).toHaveText('3 done');
  await tab.locator('.subagents-done > summary').click();
  // A row opens its session in the same tab; the back arrow returns.
  await tab.getByRole('button', { name: /^Company review/ }).click();
  await expect(tab.getByRole('heading', { name: 'Company review' })).toBeVisible();
  await tab.getByRole('button', { name: 'Back to subagents', exact: true }).click();
  await expect(tab.getByRole('button', { name: /^Company review/ })).toBeFocused();
});
