import { expect, test, type Page } from './fixtures/test';
import { bellBadge, bellDialog, bellWaitingCount, openReviews, openWaiting, waitingBell, waitingPanel } from './waiting';

/**
 * P0-09: each Review view lists every item of its own kinds (its own query,
 * filtered before paging), counts by one rule, and a page filter from a
 * page's review link doesn't follow the person into another view.
 */

const at = (day: number, minute = 0) => `2026-10-${String(day).padStart(2, '0')}T09:${String(minute).padStart(2, '0')}:00Z`;
const base = { detail: null, workspaceId: null, sessionKey: null, path: null, category: null, link: null };

async function seed(page: Page, extra: object[]) {
  await page.addInitScript(items => localStorage.setItem('mockNeedsYouExtra', JSON.stringify(items)), extra);
}

const views = (page: Page) => page.getByRole('navigation', { name: 'Waiting filters' });

test('35 newer approvals never hide 2 older failures', async ({ page }) => {
  const approvals = Array.from({ length: 35 }, (_, i) => ({ ...base, id: `approval:a${i}`, kind: 'approval', sourceId: `a${i}`, title: `Approve step ${i}`, state: 'pending', at: at(20, i) }));
  const failures = ['Old sync failure', 'Old run failure'].map((title, i) => ({ ...base, id: `failure:f${i}`, kind: 'failure', sourceId: `f${i}`, title, state: 'unread', at: at(2, i) }));
  await seed(page, [...approvals, ...failures]);
  await page.goto('/?mock=1');
  await openWaiting(page);
  await views(page).getByRole('button', { name: /^Failures/ }).click();
  const panel = waitingPanel(page);
  const group = panel.getByRole('region', { name: 'Failures' });
  await expect(group.getByText('Old sync failure')).toBeVisible();
  await expect(group.getByText('Old run failure')).toBeVisible();
  // The tab counts what the view lists, with no page to load first.
  const rows = await group.locator('.rail-group-count').textContent();
  await expect(views(page).getByRole('button', { name: /^Failures/ }).locator('.ui-tab-count')).toHaveText(rows!);
  await expect(panel.getByRole('button', { name: /^Show \d+ more/ })).toHaveCount(0);
});

test('a page filter from Tasks stays on All; Results then shows results', async ({ page }) => {
  await seed(page, [{ ...base, id: 'result:w1:outputs/q3.md', kind: 'result', sourceId: 'outputs/q3.md', title: 'Q3 summary ready', state: 'pending', at: at(21), workspaceId: 'w1', path: 'outputs/q3.md' }]);
  // Tasks hides its verified-empty link. Seed a local suggestion to exercise it.
  await page.addInitScript(() => localStorage.setItem('mockReviewItems', JSON.stringify([{
    id: 'local-task-filter', fingerprint: 'local-task-filter', kind: 'heartbeat_task',
    source: 'heartbeat', risk: 'local_write', status: 'pending', sourcePath: null,
    createdAt: '2026-10-21T09:00:00Z', updatedAt: '2026-10-21T09:00:00Z',
    snoozedUntil: null, error: null, payload: { title: 'Check the project plan', note: 'Local test suggestion.' },
  }])));
  await page.goto('/?mock=1');
  await openReviews(page, 'tasks');
  await views(page).getByRole('button', { name: /^Results/ }).click();
  const panel = waitingPanel(page);
  await expect(panel).not.toContainText('No task suggestions are waiting.');
  await expect(panel.getByText('Q3 summary ready')).toBeVisible();
  await expect(panel).toHaveAttribute('data-filter', '');
});

/** U3: the top bar's bell is the one entry (the rail's Review row went); its
 *  description names the count honestly, then the unread notifications. */
const description = (page: Page) => expect(waitingBell(page));
const UNREAD = String.raw`\. \d+ unread notifications$`;

test('a failed read makes the bell say its count is unavailable, never nothing waiting', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await description(page).toHaveAttribute('aria-description', new RegExp(String.raw`^Needs you, \d+ waiting${UNREAD}`));
  await page.evaluate(async () => {
    const apiPath = '/src/api.ts', waitingPath = '/src/store/waitingOnYou.ts';
    const { api } = await import(/* @vite-ignore */ apiPath);
    (window as unknown as { __listNeedsYou: unknown }).__listNeedsYou = api.listNeedsYou;
    api.listNeedsYou = async () => { throw new Error('synthetic failure'); };
    await (await import(/* @vite-ignore */ waitingPath)).waitingList.load();
  });
  await description(page).toHaveAttribute('aria-description', new RegExp(String.raw`^Needs you, count unavailable${UNREAD}`));
  // Seen too: the badge says "!", never a quiet zero.
  await expect(bellBadge(page)).toHaveText('!');
  // The bell stays named Notifications, and the rail has no second entry.
  await expect(waitingBell(page)).toHaveAccessibleName('Notifications');
  await expect(page.getByRole('complementary', { name: 'Main navigation' }).getByRole('button', { name: /^(Review|Needs you)\b/ })).toHaveCount(0);
  // Opened, Needs you says the read failed instead of "No decisions waiting".
  await waitingBell(page).click();
  await expect(bellDialog(page).getByRole('alert').filter({ hasText: 'Couldn’t load what is waiting' })).toBeVisible();
  await expect(bellDialog(page).getByText('No decisions waiting.', { exact: false })).toHaveCount(0);
  await page.keyboard.press('Escape');
  await expect(bellDialog(page)).toHaveCount(0);
  await page.evaluate(async () => {
    const apiPath = '/src/api.ts', waitingPath = '/src/store/waitingOnYou.ts';
    (await import(/* @vite-ignore */ apiPath)).api.listNeedsYou = (window as unknown as { __listNeedsYou: never }).__listNeedsYou;
    await (await import(/* @vite-ignore */ waitingPath)).waitingList.load();
  });
  await description(page).toHaveAttribute('aria-description', new RegExp(String.raw`^Needs you, \d+ waiting${UNREAD}`));
  // A quiet dot since the top-bar polish (#1492); the exact count is in the bell's description and its dialog.
  await expect(bellBadge(page)).toHaveText('•');
});

test('an idle draft is named apart, waits in All only, and Discard has Undo', async ({ page }) => {
  const path = 'workspaces/alpha/outputs/slides.md';
  await page.addInitScript(([key, savedAt]) => localStorage.setItem(key, JSON.stringify({ text: 'mine', baseline: 'base', savedAt })), [`hollow.fileDraft.${path}`, new Date(Date.now() - 11 * 60_000).toISOString()]);
  await page.goto('/?mock=1&demo=1');
  await description(page).toHaveAttribute('aria-description', new RegExp(String.raw`^Needs you, \d+ waiting, 1 unsaved draft${UNREAD}`));
  // The count says only what waits on a decision; the draft is apart. The
  // badge is a quiet dot since the top-bar polish (#1492): the count is in
  // the bell's description.
  const number = (await bellWaitingCount(page))!;
  expect(number).toBeGreaterThan(0);
  await expect(bellBadge(page)).toHaveText('•');
  // The draft keeps its own words in Needs you, which points to See all.
  await waitingBell(page).click();
  await expect(bellDialog(page).getByText('1 draft in the full list', { exact: true })).toBeVisible();
  await openWaiting(page);
  const panel = waitingPanel(page);
  const drafts = panel.getByRole('region', { name: 'Unsaved drafts' });
  await expect(drafts.getByText('Unsaved draft: slides.md')).toBeVisible();
  await expect(views(page).getByRole('button', { name: /^All/ }).locator('.ui-tab-count')).toHaveText(String(number + 1));
  await views(page).getByRole('button', { name: /^Decisions/ }).click();
  await expect(panel.getByRole('region', { name: 'Unsaved drafts' })).toHaveCount(0);
  await views(page).getByRole('button', { name: /^All/ }).click();
  await drafts.getByRole('button', { name: 'Discard: Unsaved draft: slides.md' }).click();
  await expect(panel.getByRole('region', { name: 'Unsaved drafts' })).toHaveCount(0);
  await description(page).toHaveAttribute('aria-description', new RegExp(`^Needs you, ${number} waiting${UNREAD}`));
  await expect(bellBadge(page)).toHaveText('•');
  expect(await page.evaluate(key => localStorage.getItem(key), `hollow.fileDraft.${path}`)).toBeNull();
  await page.getByRole('button', { name: 'Undo', exact: true }).click();
  await expect(drafts.getByText('Unsaved draft: slides.md')).toBeVisible();
  await description(page).toHaveAttribute('aria-description', new RegExp(`^Needs you, ${number} waiting, 1 unsaved draft${UNREAD}`));
});
