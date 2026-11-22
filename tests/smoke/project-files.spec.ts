import { expect, test, type Page } from './fixtures/test';
import { closeProjectFiles, expectConversationWithPane, filesPicker, optionLabels, previewPane, projectFilesButton, showWorkspaceList } from './choose';

/**
 * Project files (COO rebuild L4, 2026-11-10): the workspace panel and its
 * sections are gone. The top bar's Project files opens the project's
 * WORKSPACE.md in the Preview pane beside the conversation; the pane's Files
 * picker pins the project's notes (WORKSPACE.md, PLAN.md, SOURCES.md and
 * AGENTS.md, those that exist) in a "Project notes" group; and a workspace's
 * PLAN.md ticks its checkboxes in the Markdown view through
 * `toggle_plan_row`, then reads the file again (the box shows what the file
 * says).
 */

const PLAN = 'workspaces/local-q3-review/PLAN.md';
const ROW = 'Revise section 3 with the September close';

const calls = (page: Page, command: string) => page.evaluate(c => window.__mockApiCalls?.[c] ?? 0, command);
const planText = (page: Page) => page.evaluate(async path => {
  const src = '/src/mockApi.ts', { mockInvoke } = await import(/* @vite-ignore */ src);
  return (await mockInvoke('read_work_file', { path }) as { content: string }).content;
}, PLAN);

async function openQ3(page: Page) {
  await page.goto('/?mock=1&demo=1');
  await expect(page.getByRole('region', { name: 'Projects' })).toBeVisible();
  await showWorkspaceList(page);
  await page.locator('.work-card').filter({ hasText: 'Q3 regional operations review' }).first().click();
  await expect(page.locator('.top-bar-crumbs')).toContainText('Q3 regional operations review');
}

test('Project files opens WORKSPACE.md, the picker pins the notes, and a PLAN.md box ticks the file', async ({ page }) => {
  await openQ3(page);
  const toggle = projectFilesButton(page);
  await expect(toggle).toHaveAttribute('aria-pressed', 'false');
  await expect(previewPane(page)).toHaveCount(0);

  // Project files: WORKSPACE.md in the Preview pane, beside the conversation.
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-pressed', 'true');
  const pane = previewPane(page);
  await expect(pane).toBeVisible();
  await expect(pane.getByRole('tab', { name: 'WORKSPACE.md', exact: true })).toHaveAttribute('aria-selected', 'true');
  await expectConversationWithPane(page);

  // The Files picker pins the project's notes that exist, in pin order (the
  // demo's Q3 workspace has no AGENTS.md).
  const picker = filesPicker(page);
  await expect(picker).toBeVisible();
  const labels = await optionLabels(picker);
  expect(labels.slice(0, 3)).toEqual(['WORKSPACE.md', 'PLAN.md', 'SOURCES.md']);
  expect(labels).not.toContain('AGENTS.md');
  await picker.click();
  await expect(page.locator('.hs-pop .hs-group').first()).toHaveText('Project notes');
  await page.locator('.hs-pop').getByRole('option', { name: /^PLAN\.md/ }).first().click();
  await expect(pane.getByRole('tab', { name: 'PLAN.md', exact: true })).toHaveAttribute('aria-selected', 'true');

  // PLAN.md's boxes are live in the Markdown view: a click ticks the file.
  const box = pane.getByRole('checkbox', { name: ROW, exact: true });
  await expect(box).toBeEnabled();
  await expect(box).not.toBeChecked();
  expect(await planText(page)).toContain(`- [ ] ${ROW}`);
  const before = await calls(page, 'toggle_plan_row');
  await box.click();
  await expect.poll(() => calls(page, 'toggle_plan_row')).toBe(before + 1);
  await expect.poll(() => planText(page)).toContain(`- [x] ${ROW}`);
  // The file was read again: the box shows what PLAN.md says now.
  await expect(box).toBeChecked();
  // Only that line changed.
  const text = await planText(page);
  expect(text).toContain('- [ ] Add the overtime analysis to the appendix');
  expect(text).toContain('- [x] Gather the regional P&L and staffing exports');

  // Unticking restores it.
  await box.click();
  await expect.poll(() => planText(page)).toContain(`- [ ] ${ROW}`);
  await expect(box).not.toBeChecked();

  // Pressed, Project files hides the pane; the conversation stays.
  await closeProjectFiles(page);
});
