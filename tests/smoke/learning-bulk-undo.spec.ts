import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from './fixtures/test';
import { railButton, storedStatus } from './waiting';

const LEARNINGS = [
  { id: 'proposal-learning-a', subject: 'Release verification' },
  { id: 'proposal-learning-b', subject: 'Weekly status goes to the vault' },
];

async function openLearnings(page: Page) {
  await page.addInitScript(learnings => {
    const at = '2026-11-08T08:00:00Z';
    if (localStorage.getItem('mockReviewItems')) return;
    localStorage.setItem('mockReviewItems', JSON.stringify(learnings.map(({id,subject}) => ({
      id, kind:'learning', status:'pending', createdAt:at, updatedAt:at, fingerprint:`fp-${id}`,
      payload:{kind:'preference',targetKind:'operator',targetPath:'me/Guardrails.md',subject,statement:`${subject}: a statement.`,rationale:'Fixture.',evidenceExcerpt:'A quoted fixture.',sourceRunId:'run-chat-1',
        // The duplicate check flagged the first one (the bulk bar says so).
        ...(id === 'proposal-learning-a' ? {possibleDuplicate:'learnings/release.md'} : {})},
      risk:'local_write',source:'chat',sourcePath:null,snoozedUntil:null,error:null,
    }))));
  }, LEARNINGS);
  await page.goto('/?mock=1');
  await railButton(page, 'Library').click();
  await page.getByRole('navigation', {name:'Library view'}).getByRole('button', {name:'Learnings',exact:true}).click();
  const panel = page.getByRole('region', {name:'Learnings',exact:true});
  await expect(panel.getByRole('heading', {name:'Learnings',exact:true})).toBeVisible();
  return panel;
}

async function tickBoth(page: Page) {
  for (const {subject} of LEARNINGS) await page.getByRole('checkbox', {name:subject,exact:true}).check();
}

async function expectBothPending(page: Page) {
  for (const {id,subject} of LEARNINGS) {
    await expect(page.getByRole('checkbox', {name:subject,exact:true})).toBeVisible();
    expect(await storedStatus(page,id)).toBe('pending');
  }
}

test('bulk learning review and Undo live in Learnings across visits', async ({page}) => {
  await page.emulateMedia({reducedMotion:'reduce'});
  const panel = await openLearnings(page);
  await expectBothPending(page);
  await tickBoth(page);
  // Still selectable, but said: one of them may repeat a saved learning.
  await expect(panel.getByRole('group', {name:'Bulk review learnings'}).getByRole('status')).toHaveText('One may repeat a saved learning');
  await panel.getByRole('textbox', {name:'Reason to reject selected'}).fill('These are fixture-only decisions.');
  await panel.getByRole('button', {name:'Reject selected',exact:true}).click();
  for (const {id} of LEARNINGS) await expect.poll(() => storedStatus(page,id)).toBe('rejected');
  // The batch's own 10 s Undo, from the shared snackbar.
  await expect(page.locator('.snackbar').filter({hasText:'Rejected 2 learnings'}).getByRole('button', {name:'Undo'})).toBeVisible();
  const undo = panel.getByRole('button', {name:/^Undo last batch/});
  await expect(undo).toBeVisible();
  await railButton(page,'Projects').click();
  await railButton(page,'Library').click();
  await expect(undo).toBeVisible();
  await undo.focus();
  await page.keyboard.press('Shift+Tab');
  await expect(undo).not.toBeFocused();
  await page.keyboard.press('Tab');
  await expect(undo).toBeFocused();
  await expect(undo).toHaveText('Undo last batch (2 learnings rejected)');
  await page.keyboard.press('Enter');
  await expectBothPending(page);
  // Nothing in that batch is left to undo, so the button goes.
  await expect(undo).toHaveCount(0);
  await tickBoth(page);
  await panel.getByRole('button', {name:'Save selected (2)',exact:true}).click();
  for (const {id} of LEARNINGS) await expect.poll(() => storedStatus(page,id)).toBe('accepted');
  await expect(undo).toBeVisible();
  const audit = await new AxeBuilder({page}).include('[aria-label="Learnings"]').analyze();
  expect(audit.violations.map(v => v.id)).toEqual([]);
});

test('a failed learning Undo keeps the same batch available for retry', async ({page}) => {
  const panel = await openLearnings(page);
  await tickBoth(page);
  await panel.getByRole('button', {name:'Save selected (2)',exact:true}).click();
  const undo = panel.getByRole('button', {name:/^Undo last batch/});
  await expect(undo).toBeVisible();
  await page.evaluate(() => { window.__mockFailCommandOnce = 'undo_learning_batch'; });
  await undo.click();
  await expect(panel.getByText(/mock undo_learning_batch failure/)).toBeVisible();
  await expect(undo).toBeVisible();
  await undo.click();
  await expectBothPending(page);
  await expect(undo).toHaveCount(0);
});
