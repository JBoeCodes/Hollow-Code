import { expect, test } from './fixtures/test';

for (const theme of ['light', 'dark', 'glass']) test(`meeting prep ${theme}: missing attendees can retry without losing the meeting`, async ({ page }, info) => {
  await page.addInitScript(t => { localStorage.setItem('theme', t === 'light' ? 'light' : 'dark'); localStorage.setItem('glass', t === 'glass' ? 'on' : 'off'); }, theme);
  await page.goto('/?mock=1');
  await expect(page.getByRole('button', { name: /Settings/ }).first()).toBeVisible();
  await page.evaluate(async () => {
    // @ts-expect-error dev-server fixture modules
    const { api } = await import('/src/api.ts');
    // @ts-expect-error dev-server fixture modules
    const prep = await import('/src/store/prepPack.ts');
    let calls = 0;
    api.prepPack = async (id: string) => {
      (window as any).__prepCalls = ++calls;
      if (calls === 1) throw new Error("Couldn't read attendee information. Check Calendar access, then try again.");
      return { meetingId: id, subject: 'Synthetic planning', start: '2026-10-12T13:00:00Z', path: 'outputs/prep/synthetic.md', workspaceId: null, markdown: '# Prep: Synthetic planning\n\nAttendee information is ready.', kept: false };
    };
    prep.openPrepPack('synthetic-meeting', 'Synthetic planning');
  });
  const dialog = page.getByRole('dialog', { name: /Prep: Synthetic planning/ });
  await expect(dialog.getByRole('alert')).toContainText("Couldn't read attendee information");
  const retry = dialog.getByRole('button', { name: 'Try again', exact: true });
  await expect(retry).toBeVisible();
  if (theme === 'glass') await page.addStyleTag({ content: 'html[data-glass] { background:var(--canvas); }' });
  if (process.env.A8_CAPTURE_DIR) await dialog.screenshot({ path: `${process.env.A8_CAPTURE_DIR}/a8-${theme}-${info.project.name}.png`, animations: 'disabled' });
  await retry.focus(); await page.keyboard.press('Enter');
  await expect(dialog.locator('.prep-body')).toContainText('Attendee information is ready.');
  await expect(dialog.getByRole('alert')).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).__prepCalls)).toBe(2);
  await expect(dialog.getByRole('button', { name: 'Open as a note' })).toBeEnabled();
  await expect(dialog.getByRole('button', { name: 'Write it again' })).toBeFocused();
});
