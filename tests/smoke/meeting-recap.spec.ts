import { expect, test } from './fixtures/test';

// Today and Agenda no longer open a recap (overnight 11-08 S3). The dialog
// still opens from a recording's Recap the meeting action.
for (const theme of ['light', 'dark', 'glass']) test(`meeting recap ${theme}: opens the pack from a recording action`, async ({ page }) => {
  await page.addInitScript(t => {
    localStorage.setItem('theme', t === 'light' ? 'light' : 'dark');
    localStorage.setItem('glass', t === 'glass' ? 'on' : 'off');
  }, theme);
  await page.goto('/?mock=1');
  await expect(page.getByRole('button', { name: /Settings/ }).first()).toBeVisible();
  await page.evaluate(async () => {
    const m = await import('/src/store/prepPack.ts');
    m.openMeetingRecap('finished', 'Finished planning meeting', new Date(Date.now() - 60_000).toISOString());
  });
  await expect(page.getByRole('dialog', { name: /Recap: Finished planning meeting/ })).toBeVisible();
});
