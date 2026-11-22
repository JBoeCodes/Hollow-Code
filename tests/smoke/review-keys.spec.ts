import { expect, test, type Page } from './fixtures/test';
import { navigateWaiting, openWaiting, railButton, storedStatus } from './waiting';

/**
 * P0-10: Review's keys and accepts act only on what is on screen (the case of
 * keys in the Main agent drawer beside Incoming went with the drawer, run 15
 * P3). Incoming never accepts a Result (it doesn't show one's content).
 * Learnings are decided in Library → Learnings (overhaul J7): there Shift-R
 * asks for the reason and a plain r does nothing, and Review again brings a
 * rejected one back; Needs you lists them, but a row opens that home, with
 * its bulk bar, and never a card of its own.
 */

const at = '2026-10-22T08:00:00Z';
const learning = (id: string, subject: string) => ({
  id, kind: 'learning', status: 'pending', createdAt: at, updatedAt: at, fingerprint: `fp-${id}`,
  payload: {
    kind: 'preference', targetKind: 'operator', targetPath: 'me/Guardrails.md', subject,
    statement: `${subject}: a statement.`, rationale: 'Seeded for P0-10.', evidenceExcerpt: 'Seeded.', sourceRunId: 'run-chat-1',
  },
  risk: 'local_write', source: 'chat', sourcePath: null, snoozedUntil: null, error: null,
});
const LEARNINGS = [learning('proposal-p10-a', 'Release checks'), learning('proposal-p10-b', 'Weekly status')];
const RESULT = {
  id: 'result:w1:outputs/q3.md', kind: 'result', sourceId: 'outputs/q3.md', title: 'Q3 summary ready', detail: null, state: 'pending',
  at: '2026-10-22T09:00:00Z', workspaceId: 'w1', sessionKey: null, path: 'outputs/q3.md', category: null, link: null,
};

async function seed(page: Page, items: object[]) {
  await page.addInitScript(({ items, extra }) => {
    if (localStorage.getItem('mockReviewItems')) return;
    localStorage.setItem('mockReviewItems', JSON.stringify(items));
    localStorage.setItem('mockNeedsYouExtra', JSON.stringify(extra));
  }, { items, extra: [RESULT] });
}

async function assertNoIncoming(page: Page) {
  await navigateWaiting(page);
  await expect(page.getByRole('navigation', { name: 'Review views' })).toHaveCount(0);
  await expect(page.getByRole('navigation', { name: 'Waiting filters' }).getByRole('button', { name: 'Incoming', exact: true })).toHaveCount(0);
  await expect(page.getByRole('region', { name: /^Incoming/ })).toHaveCount(0);
}

test('Shift-R on a learning asks for its reason, a plain r does not, and Review again brings it back', async ({ page }) => {
  await seed(page, LEARNINGS.slice(0, 1));
  await page.goto('/?mock=1');
  await railButton(page, 'Library').click();
  await page.getByRole('navigation', { name: 'Library view' }).getByRole('button', { name: 'Learnings', exact: true }).click();
  const home = page.getByRole('region', { name: 'Learnings', exact: true });
  const actions = home.locator('[data-learning-id="proposal-p10-a"]');
  await actions.focus();
  await page.keyboard.press('r');
  const why = home.getByRole('textbox', { name: 'Why reject this learning?' });
  await expect(why).toHaveCount(0);
  await page.keyboard.press('Shift+R');
  await expect(why).toBeVisible();
  expect(await storedStatus(page, 'proposal-p10-a')).toBe('pending');
  await why.fill('Only for that release.');
  await home.getByRole('button', { name: 'Reject with reason', exact: true }).click();
  await expect.poll(() => storedStatus(page, 'proposal-p10-a')).toBe('rejected');
  await home.getByRole('navigation', { name: 'Learning state' }).getByRole('button', { name: /^Rejected/ }).click();
  await home.getByRole('button', { name: 'Review again', exact: true }).click();
  await expect.poll(() => storedStatus(page, 'proposal-p10-a')).toBe('pending');
});

test('Needs you has no Incoming, so a result is not filed there', async ({ page }) => {
  await seed(page, LEARNINGS.slice(0, 1));
  await page.goto('/?mock=1');
  await assertNoIncoming(page);
  await expect(page.locator('.triage-row')).toHaveCount(0);
});

test('a learning row in Needs you opens Library Learnings, where its bulk bar is', async ({ page }) => {
  await seed(page, LEARNINGS);
  await page.goto('/?mock=1');
  const panel = await openWaiting(page);
  const row = panel.locator('.waiting-item[data-id="review:proposal-p10-a"]').getByRole('button').first();
  await expect(row).toContainText('Decide in Library Learnings');
  // A link to the one home: no card opens here and nothing is decided.
  await expect(row).not.toHaveAttribute('aria-expanded', /.*/);
  await expect(panel.getByRole('group', { name: 'Bulk review learnings' })).toHaveCount(0);
  await row.click();
  const home = page.getByRole('region', { name: 'Learnings', exact: true });
  await expect(home.getByRole('group', { name: 'Bulk review learnings' })).toBeVisible();
  for (const { id } of LEARNINGS) expect(await storedStatus(page, id)).toBe('pending');
});
