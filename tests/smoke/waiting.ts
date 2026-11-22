import { expect, type Page } from '@playwright/test';
import { choose } from './choose';

/** Helpers for the one "waiting on you" surface (decision 7), which replaced
 *  Activity, Home's "Needs you" and the five review inboxes. */

export type ReviewContext = 'tasks' | 'learning' | 'automations' | 'automationHealth' | 'budgets';

export function railButton(page: Page, name: string) {
  return page.getByRole('complementary', { name: 'Main navigation' }).getByRole('button', { name, exact: true });
}

/** The surface itself: Review's waiting list (a page since consolidation D). */
export function waitingPanel(page: Page) {
  return page.getByRole('region', { name: 'Waiting on you' });
}

/** The top bar's bell (U3): the one entry to what waits since the rail's
 *  Review row went. Its name stays "Notifications"; its description carries
 *  the waiting count, "count unavailable", unsaved drafts and the unread
 *  notifications ("Needs you, 3 waiting, 1 unsaved draft. 2 unread
 *  notifications"). Scoped to the top bar: the open popover has its own
 *  Notifications tab. */
export function waitingBell(page: Page) {
  return page.locator('.top-bar').getByRole('button', { name: 'Notifications', exact: true });
}

/** The bell's popover. */
export function bellDialog(page: Page) {
  return page.getByRole('dialog', { name: 'Notifications', exact: true });
}

/** The popover's two views ("Needs you" carries its count). */
export function bellViews(page: Page) {
  return bellDialog(page).getByRole('navigation', { name: 'Notification views' });
}

/** The one notification history (U3b): Bell → Notifications. Opens the bell
 *  unless it is open and chooses the Notifications view explicitly, since the
 *  popover opens to Needs you while something waits. */
export async function openBellHistory(page: Page) {
  const dialog = bellDialog(page);
  if (!await dialog.isVisible()) await waitingBell(page).click();
  await expect(dialog).toBeVisible();
  const tab = bellViews(page).getByRole('button', { name: 'Notifications', exact: true });
  await tab.click();
  await expect(tab).toHaveAttribute('aria-pressed', 'true');
  const history = dialog.getByRole('region', { name: 'Notification history', exact: true });
  await expect(history).toBeVisible();
  return history;
}

/** The history's rows; folded repeats are one row. */
export function bellHistoryRows(page: Page) {
  return bellDialog(page).getByRole('region', { name: 'Notification history', exact: true }).locator('.notif-row');
}

/** The bell's badge: the waiting (else unread) count, "!" when a count is
 *  unknown, partial or failed, "•" when only drafts wait. */
export function bellBadge(page: Page) {
  return waitingBell(page).locator('.notification-bell-count');
}

/** What the bell says waits: `null` for "count unavailable", 0 when it
 *  names no waiting count. */
export async function bellWaitingCount(page: Page): Promise<number | null> {
  const description = await waitingBell(page).getAttribute('aria-description') ?? '';
  if (/^Needs you, count unavailable\b/.test(description)) return null;
  const match = /^Needs you, (\d+) waiting\b/.exec(description);
  return match ? Number(match[1]) : 0;
}

/** Open the bell unless it is open, then choose See all needs you, without
 *  waiting for the page: for cancelled or lazy navigation tests. Clicks only,
 *  the person's own route. */
export async function navigateWaiting(page: Page) {
  const dialog = bellDialog(page);
  if (!await dialog.isVisible()) await waitingBell(page).click();
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: 'See all needs you', exact: true }).click();
}

/** Open the full waiting view, unfiltered, the way a person does: the bell,
 *  then See all needs you. */
export async function openWaiting(page: Page) {
  await navigateWaiting(page);
  await expect(bellDialog(page)).toHaveCount(0);
  const panel = waitingPanel(page);
  await expect(panel).toBeVisible();
  return panel;
}

/** The page that links to `context`'s review items. */
export async function goToReviewPage(page: Page, context: ReviewContext) {
  const destination = context === 'learning' ? 'Settings' : context === 'tasks' ? 'Tasks' : 'Automations';
  await railButton(page, destination).click();
  // Memory is part of Personalization (Knowledge until Settings PR 3).
  if (context === 'learning') {
    if ((page.viewportSize()?.width ?? 1000) <= 860) {
      await choose(page.getByRole('combobox', { name: 'Settings section' }), 'personalization');
    } else {
      await page.locator('.settings-index').getByRole('button', { name: 'Personalization', exact: true }).click();
    }
  }
}

/** The compact link a page keeps where its review inbox was. */
export function reviewLink(page: Page, context: ReviewContext) {
  return page.locator(`.waiting-link[data-suggestion-context="${context}"]`);
}

/** Go to the page and follow its link: the surface, filtered to that page's
 *  review items, every card open. */
export async function openReviews(page: Page, context: ReviewContext, linkText?: string | RegExp) {
  await goToReviewPage(page, context);
  const link = reviewLink(page, context);
  await expect(link).toBeVisible();
  // The link is on the page; following it opens Review (consolidation D), so
  // anything about the link is checked before it is followed.
  if (linkText) await expect(link).toContainText(linkText);
  await link.click();
  if (context === 'learning') {
    const learnings = page.getByRole('region', { name: 'Learnings', exact: true });
    await expect(learnings).toBeVisible();
    return learnings;
  }
  const panel = waitingPanel(page);
  await expect(panel).toBeVisible();
  await expect(panel).toHaveAttribute('data-filter', context);
  return panel;
}

/** A review item's status in the mock's review store. */
export function storedStatus(page: Page, id: string) {
  return page.evaluate((wanted) => {
    const items = JSON.parse(localStorage.getItem('mockReviewItems') ?? '[]') as Array<{ id: string; status: string }>;
    return items.find(item => item.id === wanted)?.status ?? null;
  }, id);
}

/** Leave Review the way a person does: by another destination in the rail. */
export async function leaveReview(page: Page) {
  await railButton(page, 'Projects').click();
  await expect(waitingPanel(page)).toHaveCount(0);
}
