import { expect, test, type Page } from './fixtures/test';
import { bellBadge, bellDialog, bellViews, waitingBell, waitingPanel } from './waiting';

/**
 * U3 (overnight 2026-11-04): the bell's Needs you view only lists and routes.
 * Opening the bell or choosing one of its rows never accepts, approves,
 * discards or marks anything read; a row opens the full view at that exact
 * item id; a partial or failed read is said, on the bell and in the view,
 * never shown as nothing waiting; See all needs you is always there. A
 * pending learning is listed and counted too, but its row opens Library →
 * Learnings, its one home (overhaul J7 review).
 * Against the demo mock's review items (a learning proposal, a budget pause).
 */

test.beforeEach(({}, info) => { test.skip(info.project.name !== 'chromium-desktop', 'list states, not layout: one size covers them'); });

/** Commands that settle, discard or mark read: none may run on the way. */
const MUTATIONS = [
  'resolve_review_item', 'resolve_learning_proposals_batch', 'snooze_review_item', 'resolve_interaction',
  'dismiss_interaction', 'resolve_commitment_proposal', 'mark_notification_read', 'mark_notifications_read',
  'mark_all_notifications_read', 'delete_notification', 'delete_notifications',
] as const;
const mutations = (page: Page) => page.evaluate(names => {
  const seen = (window as unknown as { __mockApiCalls?: Record<string, number> }).__mockApiCalls ?? {};
  return Object.fromEntries(names.map(name => [name, seen[name] ?? 0]));
}, [...MUTATIONS]);
const stored = (page: Page) => page.evaluate(() => Object.fromEntries((JSON.parse(localStorage.getItem('mockReviewItems') ?? '[]') as { id: string; status: string }[]).map(i => [i.id, i.status])));
const requestedId = (page: Page) => page.evaluate(async () => {
  const p = '/src/store/waitingOnYou.ts';
  return ((await import(/* @vite-ignore */ p)).waitingItemRequest() as { id: string } | null)?.id ?? null;
});
const needsTab = (page: Page) => bellViews(page).getByRole('button', { name: /^Needs you/ });
const notificationsTab = (page: Page) => bellViews(page).getByRole('button', { name: 'Notifications', exact: true });
const seeAll = (page: Page) => bellDialog(page).getByRole('button', { name: 'See all needs you', exact: true });
const UNREAD = String.raw`\. \d+ unread notifications$`;

/** Replace the waiting read in the page: `mode` is a partial read naming one
 *  missing source, a failure, an empty list, or the real read again; `load`
 *  reads it at once (else the next refresh does). */
async function waitingRead(page: Page, mode: 'partial' | 'fail' | 'empty' | 'real', load = true) {
  await page.evaluate(async ([mode, load]) => {
    const apiPath = '/src/api.ts', waitingPath = '/src/store/waitingOnYou.ts';
    const { api } = await import(/* @vite-ignore */ apiPath);
    const w = window as unknown as { __realListNeedsYou?: (...args: unknown[]) => Promise<Record<string, unknown>> };
    w.__realListNeedsYou ??= api.listNeedsYou;
    const real = w.__realListNeedsYou!;
    api.listNeedsYou = mode === 'real' ? real
      : mode === 'fail' ? async () => { throw new Error('synthetic waiting failure'); }
      : mode === 'empty' ? async () => ({ items: [], total: 0, nextCursor: null, unavailable: [] })
      : async (...args: unknown[]) => ({ ...(await real(...args)), unavailable: ['Fixture tasks'] });
    if (load) await (await import(/* @vite-ignore */ waitingPath)).waitingList.load();
  }, [mode, load] as const);
}

test('a Needs you row opens that exact item and settles nothing on the way', async ({ page }) => {
  // Hold the full view's chunk so the exact request can be read before the
  // view consumes it; then let it land and open the item in place.
  let release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/src/components/WaitingOnYouPanel.tsx*', async route => { await held; await route.continue(); });
  await page.goto('/?mock=1&demo=1');
  const before = await stored(page);
  const calls = await mutations(page);
  let id: string | null = null;
  try {
    await expect(waitingBell(page)).toHaveAttribute('aria-description', new RegExp(String.raw`^Needs you, [1-9]\d* waiting${UNREAD}`));
    await waitingBell(page).click();
    await expect(bellDialog(page)).toBeVisible();
    // Something waits, so it opens to Needs you.
    await expect(needsTab(page)).toHaveAttribute('aria-pressed', 'true');
    const summary = bellDialog(page).getByRole('region', { name: 'Needs you summary' });
    const row = summary.getByRole('button', { name: /Budget pause/ });
    await expect(row).toBeVisible();
    await expect(seeAll(page)).toBeVisible();
    // Opening lists; it settles nothing.
    expect(await stored(page)).toEqual(before);
    expect(await mutations(page)).toEqual(calls);
    id = await page.evaluate(async () => {
      const p = '/src/store/waitingOnYou.ts';
      const items = (await import(/* @vite-ignore */ p)).waitingList.items() as { id: string; title: string }[];
      return items.find(item => item.title.includes('Budget pause'))?.id ?? null;
    });
    expect(id, 'the summary row is a listed waiting item').not.toBeNull();
    await row.click();
    await expect(bellDialog(page)).toHaveCount(0);
    // The full view is asked for that exact id, nothing broader.
    await expect.poll(() => requestedId(page)).toBe(id);
    await expect(page.locator('.canvas-split > .page').getByRole('heading', { name: 'Needs you', level: 1 })).toBeVisible();
    expect(await stored(page)).toEqual(before);
  } finally { release(); }
  // Landed: the view opens that item in place and the request is spent.
  const budget = waitingPanel(page).locator('.review-inbox-card[data-review-id="review-budget-pause-1"]');
  await expect(budget).toBeVisible();
  await expect.poll(() => requestedId(page)).toBeNull();
  // Opened, not settled: its buttons stay the person's own click.
  await expect(budget.getByRole('button', { name: 'Keep paused', exact: true })).toBeVisible();
  await expect(budget.getByRole('button', { name: 'Raise and resume', exact: true })).toBeVisible();
  expect(await stored(page)).toEqual(before);
  expect(await mutations(page)).toEqual(calls);
});

test('a pending learning is counted on the bell, and its row opens Library Learnings without deciding it', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  const before = await stored(page);
  const calls = await mutations(page);
  await waitingBell(page).click();
  const summary = bellDialog(page).getByRole('region', { name: 'Needs you summary' });
  await summary.getByRole('button', { name: /Release verification/ }).click();
  await expect(bellDialog(page)).toHaveCount(0);
  const learnings = page.getByRole('region', { name: 'Learnings', exact: true });
  await expect(learnings.locator('.review-card').filter({ hasText: 'Release verification' })).toBeVisible();
  // The Library tab says how many wait; Needs you is no second decision surface.
  const tab = page.getByRole('navigation', { name: 'Library view' }).getByRole('button', { name: 'Learnings', exact: true });
  await expect(tab.locator('.ui-tab-count')).toHaveText('1');
  await expect(tab).toHaveAttribute('title', '1 pending');
  expect(await requestedId(page)).toBeNull();
  expect(await stored(page)).toEqual(before);
  expect(await mutations(page)).toEqual(calls);
});

test('a partial read is said on the bell and in Needs you, never as nothing waiting', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await expect(waitingBell(page)).toHaveAttribute('aria-description', new RegExp(String.raw`^Needs you, \d+ waiting${UNREAD}`));
  await waitingRead(page, 'partial');
  await expect(waitingBell(page)).toHaveAttribute('aria-description', new RegExp(String.raw`^Needs you, \d+ waiting\. Some waiting sources unavailable${UNREAD}`));
  await expect(bellBadge(page)).toHaveText('!');
  await waitingBell(page).click();
  await expect(needsTab(page)).toHaveAttribute('aria-pressed', 'true');
  await expect(bellDialog(page).getByText('Couldn’t read Fixture tasks, so anything waiting there isn’t listed.', { exact: true })).toBeVisible();
  await expect(bellDialog(page).getByText(/No decisions waiting/)).toHaveCount(0);
  await expect(seeAll(page)).toBeVisible();
  // The full view says it as well.
  await seeAll(page).click();
  await expect(waitingPanel(page)).toBeVisible();
  await expect(waitingPanel(page)).toContainText('Couldn’t read Fixture tasks');
  await waitingRead(page, 'real');
  await expect(waitingBell(page)).toHaveAttribute('aria-description', new RegExp(String.raw`^Needs you, \d+ waiting${UNREAD}`));
});

test('a failed read opens Needs you with the failure said, and Refresh recovers it', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  const calls = await mutations(page);
  await waitingRead(page, 'fail');
  await expect(waitingBell(page)).toHaveAttribute('aria-description', new RegExp(String.raw`^Needs you, count unavailable${UNREAD}`));
  await expect(bellBadge(page)).toHaveText('!');
  await waitingBell(page).click();
  await expect(needsTab(page)).toHaveAttribute('aria-pressed', 'true');
  const failed = bellDialog(page).getByRole('alert').filter({ hasText: 'Couldn’t load what is waiting' });
  await expect(failed).toBeVisible();
  await expect(bellDialog(page).getByText(/No decisions waiting/)).toHaveCount(0);
  await expect(seeAll(page)).toBeVisible();
  // Notifications stay readable beside the failure.
  await notificationsTab(page).click();
  await expect(bellDialog(page).getByRole('button', { name: 'Refresh', exact: true })).toBeVisible();
  await needsTab(page).click();
  await expect(failed).toBeVisible();
  // The source is back; the view's own Refresh reads it.
  await waitingRead(page, 'real', false);
  await bellDialog(page).getByRole('button', { name: 'Refresh needs you', exact: true }).click();
  await expect(failed).toHaveCount(0);
  await expect(waitingBell(page)).toHaveAttribute('aria-description', new RegExp(String.raw`^Needs you, \d+ waiting${UNREAD}`));
  expect(await mutations(page)).toEqual(calls);
});

test('with nothing waiting the bell opens to Notifications, and See all needs you stays', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await waitingRead(page, 'empty');
  await expect(waitingBell(page)).toHaveAttribute('aria-description', new RegExp(String.raw`^Needs you${UNREAD}`));
  await waitingBell(page).click();
  await expect(notificationsTab(page)).toHaveAttribute('aria-pressed', 'true');
  await expect(seeAll(page)).toBeVisible();
  await needsTab(page).click();
  await expect(bellDialog(page).getByText('No decisions waiting.', { exact: false })).toBeVisible();
  await seeAll(page).click();
  await expect(bellDialog(page)).toHaveCount(0);
  await expect(waitingPanel(page)).toBeVisible();
});
