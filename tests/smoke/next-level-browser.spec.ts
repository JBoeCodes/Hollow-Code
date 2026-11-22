import { expect, test, type Page } from './fixtures/test';

// Next-level P15: browser and annotation review, in the Preview pane's web
// mode since run 15 P1. `?mock=1` has no native webview, so these check the
// pane's side of the contract (what it asks browser_pane.rs to show or hide,
// from `mockBrowserLog`) and the DOM: one app-wide tab list (P15 had scoped
// tabs to a workspace; P1 made them one list), the dev preview strip, failed
// loads, captures labelled as captures, drafts that survive closing the pane,
// and saved reviews with a before/after compare (src/mockReviews.ts).

type Call = { command: string; args: { tabId?: string | null; url?: string; action?: string } };
const calls = (page: Page) => page.evaluate(() => JSON.parse(localStorage.getItem('mockBrowserLog') ?? '[]') as Call[]);
const lastShow = async (page: Page) => (await calls(page)).filter(c => c.command === 'browser_show').at(-1);
const shownTab = async (page: Page) => (await lastShow(page))?.args.tabId ?? null;

const BILLING = '/Users/fixture/vault/workspaces/work-billing';
const SYNC = '/Users/fixture/vault/workspaces/work-sync';

async function start(page: Page, opts: { services?: boolean } = {}) {
  await page.addInitScript(({ billing, sync, services }) => {
    if (localStorage.getItem('mockWorkspaces')) return;
    localStorage.setItem('mockWorkspaces', JSON.stringify([
      { id: 'work-billing', title: 'Billing', status: 'active', path: 'workspaces/work-billing', cwd: billing, modified: '2026-09-29T12:00:00Z' },
      { id: 'work-sync', title: 'Sync service', status: 'active', path: 'workspaces/work-sync', cwd: sync, modified: '2026-09-28T12:00:00Z' },
    ]));
    const session = (key: string, title: string, cwd: string) => ({ key, title, agent: 'claude', cwd, createdAt: '2026-09-29T12:00:00Z', lastActivityAt: '2026-09-29T12:00:00Z', endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], harnessSessionId: null, resumable: 'briefOnly' });
    const sessions = [session('s-20261001-bill0001', 'Invoice page', billing), session('s-20261001-sync0001', 'Sync docs', sync)];
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify(sessions));
    const turn = (key: string, role: string, text: string) => ({ id: `${key}-${role}`, role, text, tools: [], agent: 'claude', runId: null, startedAt: '2026-09-29T12:00:00Z', status: 'success', costUsd: null, error: null, receipt: null, transcriptIncomplete: false, resultKind: null, stoppedBeforeSpawn: false });
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify(Object.fromEntries(sessions.map(s => [s.key, { summary: s, eventsRetained: false, turns: [turn(s.key, 'user', `Work on ${s.title}`), turn(s.key, 'assistant', 'On it.')] }]))));
    localStorage.setItem('hollow.railWorkspaces', JSON.stringify(['work-billing', 'work-sync']));
    if (services) localStorage.setItem('mockServices', JSON.stringify([
      { scope: billing, name: 'web', command: 'npm run dev', cwd: billing, port: 5173, kind: 'dev-server', status: 'running', exitCode: null, pid: 4242, startedAt: '2026-10-01T12:00:00Z', log: '' },
    ]));
  }, { billing: BILLING, sync: SYNC, services: !!opts.services });
  await page.goto('/?mock=1');
  await expect(page.getByLabel('Main navigation')).toBeVisible();
}

const rail = (page: Page) => page.getByRole('complementary', { name: 'Main navigation' });
const pane = (page: Page) => page.getByRole('complementary', { name: 'Preview', exact: true });

async function openWorkspace(page: Page, title: string) {
  await rail(page).getByRole('button', { name: title, exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText(title);
}

/** A workspace with the web mode's start page beside it, where the side
 *  panel's Browser tab opened (run 15 P1 removed the panel). No click opens
 *  an empty web mode (a link opens its page; New tab is inside the pane), so
 *  this asks the store, as preview-pane.spec.ts does. */
async function openWebMode(page: Page, title: string) {
  await openWorkspace(page, title);
  await page.evaluate(async () => {
    const store = '/src/store/preview.ts';
    (await import(/* @vite-ignore */ store)).openPreview({ kind: 'url', url: '' });
  });
  await expect(pane(page).getByRole('textbox', { name: 'Address' })).toBeVisible();
}

async function openAddress(page: Page, text: string) {
  const address = pane(page).getByRole('textbox', { name: 'Address' });
  await address.fill(text);
  await address.press('Enter');
}

test.beforeEach(async ({ page }) => {
  test.skip(page.viewportSize()!.width <= 1100, 'the split is desktop-only');
});

test('the tabs are one app-wide list, and only the selected tab’s page is ever shown', async ({ page }) => {
  await start(page);
  await openWebMode(page, 'Billing');
  await expect(pane(page).getByText("It doesn't connect that service to Hollow or let agents use it", { exact: false })).toBeVisible();
  await openAddress(page, '5173');
  const tabs = pane(page).getByRole('tablist', { name: 'Preview tabs' }).getByRole('tab');
  await expect(tabs).toHaveCount(1);
  await expect.poll(() => shownTab(page)).toBeTruthy();
  const billingTab = await shownTab(page);

  // Another workspace: the pane stays beside it with the same tabs (one list
  // since run 15 P1; P15 kept one per workspace) and the same page on show.
  await openWorkspace(page, 'Sync service');
  await expect(tabs).toHaveCount(1);
  await expect(pane(page).getByRole('tab', { name: 'localhost:5173' })).toHaveAttribute('aria-selected', 'true');
  await expect.poll(() => shownTab(page)).toBe(billingTab);
  // New tab is the start page: no page on show until one opens.
  await pane(page).getByRole('button', { name: 'New tab', exact: true }).click();
  await pane(page).getByRole('button', { name: /^Browser/ }).click();
  await expect(pane(page).locator('.browser-start')).toBeVisible();
  await expect.poll(() => shownTab(page)).toBeNull();
  await openAddress(page, 'localhost:4000');
  await expect(pane(page).getByRole('tab', { name: 'localhost:4000' })).toHaveAttribute('aria-selected', 'true');
  await expect.poll(() => shownTab(page)).toBeTruthy();
  expect(await shownTab(page)).not.toBe(billingTab);

  // Back in Billing: both tabs, the selected one still on show; picking the
  // first shows its page again.
  await openWorkspace(page, 'Billing');
  await expect(tabs).toHaveCount(2);
  await expect(pane(page).getByRole('tab', { name: 'localhost:4000' })).toHaveAttribute('aria-selected', 'true');
  await pane(page).getByRole('tab', { name: 'localhost:5173' }).click();
  await expect(pane(page).getByRole('tab', { name: 'localhost:5173' })).toHaveAttribute('aria-selected', 'true');
  await expect.poll(() => shownTab(page)).toBe(billingTab);

  // ⌘] while focus is in the pane drives the page (the toolbar's Forward;
  // the pane's bar has one of its own).
  await pane(page).locator('.browser-toolbar').getByRole('button', { name: 'Forward' }).focus();
  await page.keyboard.press('Meta+]');
  await expect.poll(async () => (await calls(page)).filter(c => c.command === 'browser_go').at(-1)?.args.action).toBe('forward');
});

test('a running dev server shows as a dev preview with Open and Stop', async ({ page }) => {
  await start(page, { services: true });
  await openWebMode(page, 'Billing');
  const dev = pane(page).getByRole('group', { name: 'Dev preview' });
  await expect(dev).toContainText('web');
  await expect(dev).toContainText(':5173');
  await dev.getByRole('button', { name: 'Open' }).click();
  await expect(pane(page).getByRole('tab', { name: 'localhost:5173' })).toBeVisible();
  await expect(dev).toContainText('Showing');
  await dev.getByRole('button', { name: 'Stop' }).click();
  // The confirmation is drawn over the page area: the page hides under it.
  await expect(page.getByRole('dialog')).toBeVisible();
  await expect.poll(() => shownTab(page)).toBeNull();
  await page.getByRole('dialog').getByRole('button', { name: 'Stop' }).click();
  await expect(dev).toHaveCount(0);
  const log = await page.evaluate(() => JSON.parse(localStorage.getItem('mockBuildLog') ?? '[]') as { command: string }[]);
  expect(log.some(l => l.command === 'stop_service')).toBe(true);
});

test('a page that fails to open says so and can be tried again', async ({ page }) => {
  await start(page);
  await page.evaluate(() => localStorage.setItem('mockBrowserOpenFail', 'offline.test'));
  await openWebMode(page, 'Billing');
  await openAddress(page, 'offline.test');
  const failed = pane(page).getByRole('alert');
  await expect(failed).toContainText("Couldn't open offline.test");
  await expect(failed).toContainText('the server did not answer');
  await expect(pane(page).getByRole('tab', { name: 'offline.test' })).toBeVisible();
  expect(await shownTab(page)).toBeNull();
  await page.evaluate(() => localStorage.removeItem('mockBrowserOpenFail'));
  await failed.getByRole('button', { name: 'Try again' }).click();
  await expect(pane(page).getByRole('alert')).toHaveCount(0);
  await expect.poll(() => shownTab(page)).toBeTruthy();
});

test('an annotation is labelled as a capture and its draft survives closing the pane', async ({ page }) => {
  await start(page);
  await openWebMode(page, 'Billing');
  await openAddress(page, 'localhost:5173');
  await expect.poll(() => shownTab(page)).toBeTruthy();
  await pane(page).getByRole('button', { name: 'Annotate' }).click();
  const region = pane(page).getByRole('region', { name: 'Annotate page' });
  await expect(region.locator('.annotate-capture')).toHaveText(/^Capture · \d{2}:\d{2} · \d+×\d+$/);
  await expect(region.locator('.annotate-capture')).toHaveAttribute('title', /not the live page/);
  const area = (await region.getByLabel('Marks').boundingBox())!;
  await page.mouse.click(area.x + area.width * 0.3, area.y + area.height * 0.3);
  await region.getByRole('textbox', { name: 'Comment for mark 1' }).fill('Make this button blue');
  await page.keyboard.press('Enter');

  // Closing the pane (or leaving the tab, or the workspace) ends it; the
  // marks stay, and ⌘\ brings the pane back with its tab.
  await pane(page).getByRole('button', { name: 'Close preview', exact: true }).click();
  await expect(pane(page)).toHaveCount(0);
  await page.keyboard.press('Meta+Backslash');
  const notice = pane(page).getByRole('status', { name: 'Unsent annotations' });
  await expect(notice).toContainText('Unsent annotation of localhost:5173');
  await expect(notice).toContainText('1 mark');
  const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('hollow.annotateDrafts') ?? '[]'));
  expect(stored).toHaveLength(1);
  expect(stored[0]).toMatchObject({ scope: 'work-billing', url: 'http://localhost:5173' });
  expect(stored[0].view.width).toBeGreaterThan(100);
  await notice.getByRole('button', { name: 'Restore' }).click();
  await expect(region.getByRole('button', { name: 'Mark 1: Make this button blue' })).toBeVisible();

  // Sent: the draft goes, and the chat is told it was a capture.
  await region.getByRole('button', { name: 'Add to chat' }).click();
  await expect(region).toHaveCount(0);
  await expect(page.getByLabel('Chat message')).toHaveValue(/attached\. Captured \d{2}:\d{2} at a \d+×\d+ viewport \(a snapshot, not the live page\)\./);
  await expect(pane(page).getByRole('status', { name: 'Unsent annotations' })).toHaveCount(0);
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('hollow.annotateDrafts') ?? '[]').length)).toBe(0);
});

test('a saved review compares a newer capture and links each requested change to it', async ({ page }) => {
  await start(page);
  await openWebMode(page, 'Billing');
  await openAddress(page, 'localhost:5173');
  await expect.poll(() => shownTab(page)).toBeTruthy();
  await pane(page).getByRole('button', { name: 'Annotate' }).click();
  const region = pane(page).getByRole('region', { name: 'Annotate page' });
  const area = (await region.getByLabel('Marks').boundingBox())!;
  await page.mouse.click(area.x + area.width * 0.25, area.y + area.height * 0.25);
  await region.getByRole('textbox', { name: 'Comment for mark 1' }).fill('Make this button blue');
  await page.keyboard.press('Enter');
  await page.mouse.click(area.x + area.width * 0.7, area.y + area.height * 0.6);
  await region.getByRole('textbox', { name: 'Comment for mark 2' }).fill('Less space here');
  await page.keyboard.press('Enter');
  await region.getByRole('button', { name: 'Save to workspace' }).click();

  const reviews = pane(page).getByRole('region', { name: 'Page reviews' });
  await expect(reviews).toContainText('Page review');
  await expect(reviews).toContainText('0 of 2 changes done');
  await expect(reviews.locator('.review-capture-label').first()).toHaveText(/^Capture · \d{2}:\d{2} · \d+×\d+ · localhost:5173$/);
  // The review is a workspace file (so Results lists it), with URL, time and viewport.
  const saved = await page.evaluate(() => {
    const notes = JSON.parse(localStorage.getItem('mockWorkspaceNotes') ?? '{}') as Record<string, string>;
    const path = Object.keys(notes).find(p => p.endsWith('.annotations.json'))!;
    return { path, file: JSON.parse(notes[path]) };
  });
  expect(saved.path).toBe('workspaces/work-billing/outputs/reviews/page-review.annotations.json');
  expect(saved.file.captures[0]).toMatchObject({ id: 'c1', url: 'http://localhost:5173', image: 'outputs/reviews/page-review/c1.png' });
  expect(saved.file.captures[0].capturedAt).toBeTruthy();
  expect(saved.file.captures[0].viewport.width).toBeGreaterThan(100);
  expect(saved.file.captures[0].marks).toHaveLength(2);
  // The page hides while the review is open.
  await expect.poll(() => shownTab(page)).toBeNull();

  await reviews.getByRole('button', { name: 'Compare with a newer capture' }).click();
  const compare = reviews.getByRole('group', { name: 'Before and after' });
  await expect(compare.locator('.review-capture-label').nth(0)).toHaveText(/^Before · Capture · /);
  await expect(compare.locator('.review-capture-label').nth(1)).toHaveText(/^After · Capture · /);
  await compare.getByRole('group', { name: 'Change 1' }).getByRole('button', { name: 'Done' }).click();
  await compare.getByRole('group', { name: 'Change 2' }).getByRole('button', { name: 'Not yet' }).click();
  await compare.getByRole('textbox', { name: 'Note for change 2' }).fill('still loose');
  await compare.getByRole('button', { name: 'Save comparison' }).click();

  await expect(reviews).toContainText('1 of 2 changes done');
  const changes = reviews.locator('.browser-review-changes li');
  await expect(changes.nth(0)).toContainText('Make this button blue');
  await expect(changes.nth(0)).toContainText(/Done · in Capture · \d{2}:\d{2}/);
  await expect(changes.nth(1)).toContainText(/Not yet · in Capture .* · still loose/);
  await expect(reviews.getByRole('group', { name: 'Captures' }).getByRole('button')).toHaveCount(2);
  const after = await page.evaluate(() => JSON.parse(JSON.parse(localStorage.getItem('mockWorkspaceNotes') ?? '{}')['workspaces/work-billing/outputs/reviews/page-review.annotations.json']));
  expect(after.captures.map((c: { id: string; parent: string | null }) => [c.id, c.parent])).toEqual([['c1', null], ['c2', 'c1']]);
  expect(after.links.map((l: { mark: number; result: string; status: string }) => [l.mark, l.result, l.status])).toEqual([[0, 'c2', 'done'], [1, 'c2', 'notDone']]);

  // The list shows the progress; closing shows the page again.
  await reviews.getByRole('button', { name: 'All reviews' }).click();
  await expect(reviews.getByRole('button', { name: 'Open review Page review' })).toContainText('1 of 2 changes done');
  await reviews.getByRole('button', { name: 'Close reviews (Esc)' }).click();
  await expect(reviews).toHaveCount(0);
  await expect.poll(() => shownTab(page)).toBeTruthy();

  // (Results listed the review as a page review and opened it back in Page
  // reviews until the COO rebuild L4 removed the Results section.)
  const misses = await page.evaluate(() => window.__mockApiMisses ?? []);
  expect(misses).toEqual([]);
});
