/**
 * Journey J4 (consolidation lane B, 2026-10-13): resolve a decision that
 * needs the person using visible controls only, no `g b`, no palette, no
 * keys. The top bar's bell says how many things wait, and its Needs you
 * view's See all opens the full view (U3: the rail's Review row went);
 * Decisions holds what only the person can settle; each one is answered by
 * its own card's buttons (nothing approves in bulk), and every count drops.
 * A learning's row opens Library → Learnings, where it is saved.
 * A required spec: it runs at the desktop size and at the native minimum
 * window, with and without Largest text, and may not skip
 * (scripts/check-required-specs.mjs). Against the demo mock's review items
 * (src/mockApi.ts: a learning proposal and a budget pause).
 */
import { expect, test, type Page } from './fixtures/test';
import { bellDialog, bellViews, navigateWaiting, waitingBell } from './waiting';

const reviewViews = (page: Page) => page.getByRole('navigation', { name: 'Waiting filters' });
const tab = (page: Page, name: string) => reviewViews(page).getByRole('button', { name: new RegExp(`^${name}`) });
/** The bell's description: "Needs you, N waiting. M unread notifications". */
const bellSays = (page: Page, waiting: number) => expect(waitingBell(page)).toHaveAttribute('aria-description', new RegExp(`^Needs you, ${waiting} waiting\\. \\d+ unread notifications$`));
const stored = (page: Page) => page.evaluate(() => Object.fromEntries((JSON.parse(localStorage.getItem('mockReviewItems') ?? '[]') as { id: string; status: string }[]).map(i => [i.id, i.status])));
async function noHorizontalScroll(page: Page) {
  const [scroll, client] = await page.evaluate(() => [document.scrollingElement!.scrollWidth, document.scrollingElement!.clientWidth]);
  expect(scroll, 'no horizontal page scroll').toBeLessThanOrEqual(client);
}

test('J4 click-only: resolve a decision that needs you', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  // The bell says what waits, and opens to Needs you (Home's counts went
  // with the chat-first Home, 2026-10-27).
  await bellSays(page, 3);
  // The bell is the one entry: the rail has no Review row (U3).
  await expect(page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: /^(Review|Needs you)\b/ })).toHaveCount(0);
  const before = await stored(page);
  await waitingBell(page).click();
  await expect(bellViews(page).getByRole('button', { name: /^Needs you/ })).toHaveAttribute('aria-pressed', 'true');
  // Opening the bell settles nothing: it only lists and routes.
  expect(await stored(page)).toEqual(before);
  await navigateWaiting(page);
  await expect(bellDialog(page)).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'Needs you', level: 1 })).toBeVisible();
  await expect(tab(page, 'Decisions')).toHaveText(/^Decisions\s*2$/);
  await tab(page, 'Decisions').click();
  await expect(tab(page, 'Decisions')).toHaveAttribute('aria-pressed', 'true');
  const waiting = page.getByRole('region', { name: 'Waiting on you' });
  const decisions = waiting.getByRole('region', { name: 'Review items and proposals' });
  // Decisions only: the failure waits in its own view.
  await expect(waiting.getByRole('region', { name: 'Failures' })).toHaveCount(0);

  // A paused budget: the new limit must be above the day's use; raise it.
  await decisions.getByRole('button', { name: /^Budget pause/ }).click();
  const budget = waiting.locator('.review-inbox-card').filter({ hasText: 'Budget pause' });
  await expect(budget.getByLabel('New limit')).toHaveValue('1800');
  await expect(budget.getByRole('button', { name: 'Keep paused', exact: true })).toBeVisible();
  await noHorizontalScroll(page);
  await budget.getByRole('button', { name: 'Raise and resume', exact: true }).click();
  await expect(budget).toHaveCount(0);
  await expect(tab(page, 'Decisions')).toHaveText(/^Decisions\s*1$/);
  await expect(tab(page, 'All')).toHaveText(/^All\s*2$/);
  await bellSays(page, 2);

  // A proposed learning: listed here, decided in its one home, Library →
  // Learnings (overhaul J7). Its row opens there; Save is the person's click.
  await decisions.getByRole('button', { name: /^Release verification/ }).click();
  const learning = page.getByRole('region', { name: 'Learnings', exact: true }).locator('.review-card').filter({ hasText: 'Release verification' });
  await expect(learning.getByRole('button', { name: 'Reject', exact: true })).toBeVisible();
  await learning.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(learning).toHaveCount(0);
  await bellSays(page, 1);
  expect(await stored(page)).toMatchObject({ 'proposal-learning-1': 'accepted', 'review-budget-pause-1': 'accepted' });

  // Back on Home the bell's count has dropped with it.
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Projects', exact: true })).toBeVisible();
  await bellSays(page, 1);
  // A quiet dot since the top-bar polish (#1492); the exact count is in the bell's description and its dialog.
  await expect(waitingBell(page).locator('.notification-bell-count')).toHaveText('•');
  await noHorizontalScroll(page);
});
