import { expect, test } from './fixtures/test';

test('Saved memory can be pinned, dated and restored in its one home', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('button', { name: 'Library', exact: true }).click();
  await page.getByRole('navigation', { name: 'Library view' }).getByRole('button', { name: 'Learnings', exact: true }).click();
  const region=page.getByRole('region',{name:'Learnings',exact:true});
  await expect(region.getByText(/Profile source files: 1,200 bytes/)).toBeVisible();
  await region.getByRole('navigation',{name:'Learning state'}).getByRole('button',{name:/^Saved/}).click();
  const archived=region.locator('.review-card').filter({hasText:'archived.'}).first();
  await archived.getByRole('button',{name:'Restore memory',exact:true}).click();
  await expect(region.getByText('archived. Recalled 0 times.',{exact:false})).toHaveCount(0);
  const card=region.locator('.review-card').first();
  await card.getByRole('button',{name:'Pin to keep',exact:true}).click();
  await expect(card.getByRole('button',{name:'Unpin',exact:true})).toBeVisible();
  await card.getByText('Time-bound fact',{exact:true}).click();
  await card.getByLabel('Valid through',{exact:true}).fill('2027-01-15');
  await card.getByRole('button',{name:'Save expiry',exact:true}).click();
  await expect(card.getByText('Valid through 2027-01-15.',{exact:false})).toBeVisible();
});

test('Learnings owns review and rejected reasons', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('button', { name: 'Library', exact: true }).click();
  await page.getByRole('navigation', { name: 'Library view' }).getByRole('button', { name: 'Learnings', exact: true }).click();
  const learnings = page.getByRole('region', { name: 'Learnings', exact: true });
  await expect(learnings.getByRole('heading', { name: 'Learnings', exact: true })).toBeVisible();
  const card = learnings.locator('.review-card').first();
  await card.getByRole('button', { name: 'Reject', exact: true }).click();
  const why = card.getByRole('textbox', { name: 'Why reject this learning?' });
  await expect(card.getByRole('button', { name: 'Reject with reason', exact: true })).toBeDisabled();
  await why.fill('That was a one-time request.');
  // A review refresh must preserve an open reason draft and its row.
  await page.evaluate(async () => {
    const file = '/src/reviewInbox.ts';
    (await import(/* @vite-ignore */ file)).refreshReviewInbox();
  });
  await expect(why).toHaveValue('That was a one-time request.');
  await card.getByRole('button', { name: 'Reject with reason', exact: true }).click();
  // The decided row left and no other is pending: focus moves to the
  // Learning state tabs, never down to the page (J7 review).
  await expect(learnings.getByRole('navigation', { name: 'Learning state' }).getByRole('button', { name: /^Pending/ })).toBeFocused();
  await learnings.getByRole('navigation', { name: 'Learning state' }).getByRole('button', { name: /^Rejected/ }).click();
  await expect(learnings.getByText('Reason: That was a one-time request.', { exact: true })).toBeVisible();
  await learnings.getByRole('button', { name: 'Review again', exact: true }).click();
  await learnings.getByRole('navigation', { name: 'Learning state' }).getByRole('button', { name: /^Pending/ }).click();
  await expect(learnings.getByRole('button', { name: 'Save', exact: true }).first()).toBeVisible();
});
