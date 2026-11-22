import { expect, test, type Page } from './fixtures/test';
import { choose, openAdvancedGroup } from './choose';
import { openReviews } from './waiting';

// The optional Jev duplicate check (jev_decide.rs): its own switch in
// Settings → Advanced → Search & indexes → Context prioritizer,
// off by default and unavailable without a key, and the "possibly the same
// as …" hint it leaves on suggestion cards. Without Jev nothing changes:
// cards carry no hint and every switch is off.

async function openPrioritizer(page: Page) {
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), 'advanced');
  } else {
    await page.locator('.settings-index').getByRole('button', { name: 'Advanced', exact: true }).click();
  }
  await openAdvancedGroup(page, 'indexes');
  await expect(page.getByRole('heading', { name: 'Context prioritizer', exact: true })).toBeVisible();
}

const toggle = (page: Page) => page.getByRole('switch', { name: 'Use Jev to check suggestions for duplicates' });

test('the duplicate check is off and unavailable without a key', async ({ page }) => {
  await page.goto('/?mock=1');
  await openPrioritizer(page);
  await expect(page.getByText('Jev duplicate check', { exact: false }).first()).toBeVisible();
  await expect(toggle(page)).toBeDisabled();
  await expect(toggle(page)).toHaveAttribute('aria-checked', 'false');
});

test('the duplicate check switches on independently of ranking and layout', async ({ page }) => {
  await page.addInitScript(() => {
    if (!localStorage.getItem('mockJev')) localStorage.setItem('mockJev', JSON.stringify({ enabled: false, keyPresent: true, decisionsEnabled: false }));
    localStorage.setItem('mockJevDecisionsReport', JSON.stringify({ at: '2026-09-26T12:00:00Z', engine: 'jev', subject: 'tasks', checked: 3, dropped: 1, flagged: 1, note: null }));
  });
  await page.goto('/?mock=1');
  await openPrioritizer(page);
  await expect(toggle(page)).toBeEnabled();
  await toggle(page).click();
  await expect(page.getByText('Duplicate check preference saved.')).toBeVisible();
  await expect(toggle(page)).toHaveAttribute('aria-checked', 'true');
  const saved = await page.evaluate(() => JSON.parse(localStorage.getItem('mockJev') ?? '{}'));
  expect(saved).toMatchObject({ enabled: false, decisionsEnabled: true });
  expect(saved).not.toHaveProperty('whiteboardEnabled');
  await expect(page.getByText(/Last duplicate check \(tasks, Jev\): 3 checked, 1 not queued, 1 flagged for review/)).toBeVisible();
});

test('a flagged suggestion says what it may repeat; an unflagged one says nothing', async ({ page }) => {
  await page.addInitScript(() => {
    if (localStorage.getItem('mockReviewItems')) return;
    const base = {
      kind: 'heartbeat_task', source: 'heartbeat', risk: 'local_write', status: 'pending',
      createdAt: '2026-09-26T12:00:00Z', updatedAt: '2026-09-26T12:00:00Z',
      sourcePath: 'calendar/review.md', snoozedUntil: null, error: null,
    };
    localStorage.setItem('mockReviewItems', JSON.stringify([
      { ...base, id: 'hb-dup', fingerprint: 'hb-dup', payload: { title: 'Book the offsite venue', note: 'From the planning call.', possibleDuplicate: 'Reserve the venue for the offsite' } },
      { ...base, id: 'hb-new', fingerprint: 'hb-new', payload: { title: 'Plan the team dinner', note: 'Also from the call.' } },
    ]));
  });
  await page.goto('/?mock=1');
  const panel = await openReviews(page, 'tasks');
  const flagged = panel.locator('.review-inbox-card').filter({ hasText: 'Book the offsite venue' });
  await expect(flagged.locator('.review-possible-duplicate')).toHaveText('Possibly the same as “Reserve the venue for the offsite” (Jev duplicate check).');
  const plain = panel.locator('.review-inbox-card').filter({ hasText: 'Plan the team dinner' });
  await expect(plain.locator('.review-possible-duplicate')).toHaveCount(0);
});
