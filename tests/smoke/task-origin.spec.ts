import { openTasks, previewPane } from './choose';
import { test, expect, type Page } from './fixtures/test';

// Consolidation G (2026-10-13): a task made from a workspace plan says so in
// its editor and leads back to that plan. Clicks only.
test('a task from a plan names the plan and opens it', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/?mock=1&demo=1');
  await openTasks(page);
  await page.getByText('Revise section 3 with the September close', { exact: true }).first().click();
  // The detail panel (wide) or the editor sheet (narrow) says where it came from.
  const from = page.locator('.task-origin-row, .task-origin-note').first();
  await expect(from).toContainText('From');
  await expect(from).toContainText('An item in “Q3 regional operations review”’s plan');
  // Copied, not synced (plan completion item 4), and the exact item it came from.
  await expect(from).toContainText('aren’t kept in sync');
  await from.getByRole('button', { name: 'Open the plan item' }).click();
  await expect(page.locator('.top-bar-crumbs')).toContainText('Q3 regional operations review');
  // PLAN.md in the Preview pane at the row, its line cited (COO rebuild L4:
  // it was the workspace panel's Plan section).
  const pane = previewPane(page);
  await expect(pane.getByRole('tab', { name: 'PLAN.md', exact: true })).toHaveAttribute('aria-selected', 'true');
  const mark = pane.locator('.receipt-source-target');
  await expect(mark).toHaveText('- [ ] Revise section 3 with the September close');
  await expect(mark).toHaveAttribute('data-line', '5');
});

// Plan completion item 4: where the origin names one item, the link opens
// that item, not its collection.
type MockTask = { url: string; title: string; origin: string | null; vaultPath: string; contentHash: string };
async function demoTaskWithOrigin(page: Page, title: string, origin: string) {
  await page.goto('/?mock=1&demo=1');
  await page.evaluate(([t, o]) => {
    const list = (window as unknown as { __mockDemoTasks: MockTask[] }).__mockDemoTasks;
    list.push({ ...list[0], url: `vault://tasks/${o.length}-origin.md`, vaultPath: `tasks/${o.length}-origin.md`, title: t, origin: o, contentHash: 'origin-test' });
  }, [title, origin] as const);
  await openTasks(page);
  await page.getByText(title, { exact: true }).first().click();
  return page.locator('.task-origin-row, .task-origin-note').first();
}

test('a task from a promise names the counterparty and does not open the ledger', async ({ page }) => {
  const from = await demoTaskWithOrigin(page, 'Send Jordan the notes (task)', 'commitment:commitments/send-jordan-release-notes.md');
  await expect(from).toContainText(/A promise you made/);
  await expect(from).toContainText('does not close it');
  await expect(from.getByRole('button', { name: 'Open the promise' })).toHaveCount(0);
  await expect(page.getByRole('region', { name: 'Commitments', exact: true })).toHaveCount(0);
});

test('a task from a recording opens that recording', async ({ page }) => {
  const from = await demoTaskWithOrigin(page, 'Follow up on the vendor questionnaire', 'meeting:rec1f00ba4c0ffee');
  await from.getByRole('button', { name: 'Open the recording' }).click();
  await expect(page.getByRole('listbox', { name: 'Recordings' }).getByRole('option', { name: /Vendor security call/ })).toHaveAttribute('aria-selected', 'true');
});

// Review UI_UX (Tasks), plan completion rows T7 and T9: before Work on this,
// the task says where it came from and which workspace it opens. (The scope
// tabs went with the issue-list rewrite, 11-10.)
test('the task pane shows origin and destination before Work on this', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await openTasks(page);
  await page.getByText('Revise section 3 with the September close', { exact: true }).first().click();
  const pane = page.getByRole('complementary', { name: 'Task details' });
  await expect(pane.locator('.task-origin-row, .task-origin-note').first()).toContainText('An item in “Q3 regional operations review”’s plan');
  const work = pane.locator('.task-workspace-link');
  await expect(work).toContainText('Q3 regional operations review');
  await expect(work).toContainText('Nothing is sent until you send it.');
});

// S03 follow-up, since the COO rebuild L4 without the Plan's canvas view:
// "Open the plan item" opens PLAN.md at that row, scrolled into the window
// (the plan is made to run past it).
test('a task from a plan far down PLAN.md opens the file at that row', async ({ page }, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'sets its own desktop-width window');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.setViewportSize({ width: 1440, height: 520 });
  await page.goto('/?mock=1&demo=1');
  await page.evaluate(async () => {
    // Forty open rows before the task's own, so the plan runs past the window.
    const mock = '/src/mockApi.ts';
    await import(/* @vite-ignore */ mock);
    const filler = Array.from({ length: 40 }, (_, i) => `- [ ] Filler step ${i + 1}`).join('\n');
    (window as unknown as { __mockOutsideWrite: (p: string, c: string) => void }).__mockOutsideWrite('workspaces/local-q3-review/PLAN.md',
      `# Plan\n\n${filler}\n- [ ] Revise section 3 with the September close\n- [ ] Send the draft to Dana for review\n`);
  });
  await openTasks(page);
  await page.getByText('Revise section 3 with the September close', { exact: true }).first().click();
  await page.locator('.task-origin-row, .task-origin-note').first().getByRole('button', { name: 'Open the plan item' }).click();
  const pane = previewPane(page);
  await expect(pane.getByRole('tab', { name: 'PLAN.md', exact: true })).toHaveAttribute('aria-selected', 'true');
  const row = pane.locator('.receipt-source-target');
  await expect(row).toHaveText('- [ ] Revise section 3 with the September close');
  await expect(row).toHaveAttribute('data-line', '43');
  await expect(row).toBeInViewport();
});
