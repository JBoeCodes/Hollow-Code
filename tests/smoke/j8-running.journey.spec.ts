/**
 * Journey J8 (consolidation lane B, 2026-10-13): inspect running work and
 * its evidence using visible controls only, no `g m`, no palette, no keys
 * (Activity's own j / k / Enter included). Work → Activity (formerly
 * Mission Control) shows what runs now and what finished today; a turn's
 * Open goes to its conversation, whose reply carries the run's receipt.
 * A required spec: it runs at the desktop size and at the native minimum
 * window, with and without Largest text, and may not skip
 * (scripts/check-required-specs.mjs). The run queue is seeded before load;
 * the conversations and receipts are the demo mock's (src/mockDemo.ts).
 */
import { expect, test, type Page } from './fixtures/test';

const RUNNING = 's-20261001-bld00001';   // Results view for workspaces · "Accept flow and revisions"
const FINISHED = 's-20260930-q3a00001';  // Q3 regional operations review · "Revise the labour section"

async function seedRuns(page: Page) {
  await page.addInitScript(([running, finished]) => {
    const at = (m: number) => new Date(Date.now() - m * 60_000).toISOString();
    const row = (id: number, sourceId: string, label: string, status: string, claimedAt: string, finishedAt: string | null) => ({
      id, source: 'chat', sourceId, label, detail: null, reason: 'chat', status, priority: 0, position: null, attempt: 1, coalescedCount: 0,
      requestedAt: claimedAt, claimedAt, finishedAt, historyRunId: null, error: null, waitingSince: null, waitingHolder: null, heldReason: null, cancelRequested: false,
    });
    // Finished a moment ago, so it is "today" even just after midnight.
    localStorage.setItem('mockRunQueue', JSON.stringify([
      row(41, running, 'Accept flow and revisions', 'running', at(6), null),
      row(38, finished, 'Revise the labour section', 'done', new Date(Date.now() - 2000).toISOString(), new Date().toISOString()),
    ]));
    localStorage.setItem('mockMissionFeed', JSON.stringify({ [running]: {
      key: running, queueId: 41, running: true, startedAt: Date.now() - 360_000, finishedAt: null, updatedAt: Date.now(), asked: null,
      lines: ['Running the Results tests…'], partial: '', tool: null, progress: 'usingTools', lastEventAt: at(0), costUsd: null, model: 'claude-opus-5-5', error: null,
    } }));
    localStorage.setItem('hollow.missionView', 'tracks');
  }, [RUNNING, FINISHED]);
}

const views = (page: Page) => page.getByRole('navigation', { name: 'Projects views' });
async function openActivity(page: Page) {
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: /^Projects(?:,|$)/ }).click();
  await views(page).getByRole('button', { name: 'Running', exact: true }).click();
  await expect(views(page).getByRole('button', { name: 'Running', exact: true })).toHaveAttribute('aria-pressed', 'true');
  const activity = page.locator('.mc-card');
  await expect(page.getByRole('region', { name: 'Running', exact: true })).toBeVisible();
  return activity;
}
async function noHorizontalScroll(page: Page) {
  const [scroll, client] = await page.evaluate(() => [document.scrollingElement!.scrollWidth, document.scrollingElement!.clientWidth]);
  expect(scroll, 'no horizontal page scroll').toBeLessThanOrEqual(client);
}

test('J8 click-only: inspect running work and its evidence', async ({ page }) => {
  await seedRuns(page);
  await page.goto('/?mock=1&demo=1');
  const activity = await openActivity(page);
  await expect(activity.locator('.mc-stats')).toContainText(/1\s*running/);
  await expect(activity.locator('.mc-stats')).toContainText(/1\s*finished today/);
  await expect(page.locator('[aria-modal="true"]')).toHaveCount(0);

  // Tracks: the running turn, with its latest output and a way to stop it.
  const detail = activity.getByRole('complementary', { name: 'Selected turn' });
  await activity.getByRole('button', { name: /^Accept flow and revisions, Running/ }).click();
  const live = detail.getByRole('article', { name: /^Accept flow and revisions, / });
  await expect(live).toContainText('Running the Results tests…');
  await expect(live.getByRole('button', { name: 'Open', exact: true })).toBeVisible();
  await expect(live.getByRole('button', { name: 'Stop', exact: true })).toBeVisible();
  await noHorizontalScroll(page);

  // The finished turn opens its conversation by its Open button.
  await activity.getByRole('button', { name: /^Revise the labour section, Finished/ }).click();
  const done = detail.getByRole('article', { name: /^Revise the labour section, / });
  await expect(done).toBeVisible();
  await done.getByRole('button', { name: 'Open', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Q3 regional operations review');
  const chat = page.locator('.chat-main');
  await expect(chat).toContainText('I revised section 3 · Labour cost');

  // Its evidence: the run's receipt, from that reply's own menu (the
  // first reply made the edit; the later one only answered).
  await expect(chat.locator('.execution-receipt')).toHaveCount(0);
  await chat.locator('.turn-footer').first().getByRole('button', { name: 'More actions', exact: true }).click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Show run record' }).click();
  const receipt = chat.locator('.execution-receipt');
  await expect(receipt).toHaveCount(1);
  await expect(receipt).toContainText('Run record');
  await receipt.getByLabel(/^Run record: Not verified\. 1 change\./).click();
  await expect(receipt).toContainText('q3-regional-review.md');
  await noHorizontalScroll(page);

  // Back to Activity, the List layout: the running turn opens from its card.
  await openActivity(page);
  await activity.getByRole('group', { name: 'Running layout' }).getByRole('button', { name: 'List', exact: true }).click();
  const runningList = activity.getByRole('list', { name: 'Running and queued turns' });
  await expect(activity.locator('.mc-finished')).toContainText('Revise the labour section');
  await runningList.getByRole('article', { name: /^Accept flow and revisions, / }).getByRole('button', { name: 'Open', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Results view for workspaces');
  await expect(chat).toContainText('Accepting now stores the content hash');
  await noHorizontalScroll(page);
});
