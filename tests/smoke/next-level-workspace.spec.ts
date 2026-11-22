import { expect, test, type Page } from './fixtures/test';
import { choose, filesPicker, openProjectFiles, openWorkspaceResult, previewPane, resultReviewBar, showWorkspaceList } from './choose';

/**
 * Next-level programme, first slice (P02/P03/P05/P08): Work return rows,
 * Results review state, the Plan view and file-conflict recovery, against the
 * `?demo=1` fixture (src/mockDemo.ts). Browser mock only: no agent, vault or
 * native window is involved. Since the COO rebuild L4 (2026-11-10) a Result
 * and PLAN.md open in the Preview pane (the review bar; PLAN.md's live boxes)
 * and a Result is edited in the Library reader, as any vault file is.
 */

const REVIEW = 'workspaces/local-q3-review/outputs/q3-regional-review.md';

async function open(page: Page) {
  await page.goto('/?mock=1&demo=1');
  await expect(page.locator('.home-project-row').first()).toBeVisible();
}

/** Writes through the mock IPC, as an external editor or agent would. */
async function externalEdit(page: Page, path: string, edit: (text: string) => string) {
  await page.evaluate(async ({ path, src }) => {
    const { mockInvoke } = await import('/src/mockApi.ts');
    const note = await mockInvoke<{ content: string }>('read_work_file', { path });
    const next = new Function('t', `return (${src})(t)`)(note.content);
    const r = await mockInvoke<{ conflict: boolean }>('vault_save_note_cas', { path, content: next, baseline: note.content });
    if (r.conflict) throw new Error('external edit conflicted');
  }, { path, src: edit.toString() });
}

/** The review result in the Preview pane, under its review bar. */
async function openReview(page: Page) {
  await openWorkspaceResult(page, 'local-q3-review', REVIEW);
  const bar = resultReviewBar(page);
  await expect(bar).toBeVisible();
  return bar;
}
/** The review result in the Library reader, where it is edited. */
async function openInReader(page: Page) {
  await expect(page.locator('.top-bar')).toBeVisible();
  await page.evaluate(async path => {
    const store = '/src/store/runtime.ts', runtime = await import(/* @vite-ignore */ store);
    runtime.setActivePage('vault');
    runtime.setOpenVaultNoteRequest(path);
  }, REVIEW);
  const reader = page.getByRole('region', { name: 'File reader' });
  await expect(reader).toBeVisible();
  return reader;
}
/** From Work's complete list (the rail is collapsed on narrow windows). */
async function openQ3(page: Page) {
  if (!(await page.locator('.work-card').first().isVisible())) await page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true }).click();
  await showWorkspaceList(page);
  await page.locator('.work-card').filter({ hasText: 'Q3 regional operations review' }).click();
}

test('accepting a result records that version; a later edit needs a new review', async ({ page }) => {
  await open(page);
  await page.getByRole('button', { name: 'Open Q3 regional operations review', exact: true }).click();
  let bar = await openReview(page);
  const states = bar.locator('.result-states');
  await expect(states).toContainText('Draft');
  await expect(states).toContainText('Not checked');
  await expect(states).toContainText('Written by Revise the labour section');
  await bar.getByRole('button', { name: 'Accept', exact: true }).click();
  await expect(states).toContainText('Accepted');
  // The file changes outside Hollow.
  await externalEdit(page, REVIEW, t => t.replace('Draft 3', 'Draft 4'));
  await page.goto('/?mock=1');
  await openQ3(page);
  bar = await openReview(page);
  await expect(states).toContainText('Changed since accepted');
  await bar.getByRole('button', { name: 'Compare with accepted' }).click();
  await expect(page.locator('.diff-view')).toContainText('Accepted version');
  await expect(page.locator('.diff-line.del')).toContainText('Draft 3');
  await expect(page.locator('.diff-line.add')).toContainText('Draft 4');
  // A failed read of the accepted version fails here, not the page (10-23).
  await bar.getByRole('button', { name: 'Show the file' }).first().click();
  await page.evaluate(() => { window.__mockFailCommandOnce = 'result_snapshot'; });
  await bar.getByRole('button', { name: 'Compare with accepted' }).click();
  const failed = page.getByRole('alert').filter({ hasText: 'Couldn’t read the accepted version' });
  await expect(failed).toBeVisible();
  await expect(page.getByText('Work could not load')).toHaveCount(0);
  await failed.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page.locator('.diff-line.add')).toContainText('Draft 4');
});

test('PLAN.md in the Preview pane shows its checklist, and Source is the file', async ({ page }) => {
  await open(page);
  await openQ3(page);
  await openProjectFiles(page);
  await choose(filesPicker(page), { label: 'PLAN.md' });
  const pane = previewPane(page);
  await expect(pane.getByRole('tab', { name: 'PLAN.md', exact: true })).toHaveAttribute('aria-selected', 'true');
  // PLAN.md's six rows are live boxes in the Markdown view (COO rebuild L4:
  // the Plan section drew them).
  await expect(pane.getByRole('checkbox')).toHaveCount(6);
  await pane.getByRole('group', { name: 'Show the file as' }).getByRole('button', { name: 'Source', exact: true }).click();
  await expect(pane.locator('.chat-preview-source')).toContainText('- [ ] Revise section 3 with the September close');
  await expect(pane.getByRole('checkbox')).toHaveCount(0);
});

test('a save that meets an external edit opens a three-way comparison; drafts survive a reload', async ({ page }, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'the Library file list keeps its 280 px column beside an open file, so at 390 px the reader is 50 px wide (below the 720 px window minimum)');
  await open(page);
  let reader = await openInReader(page);
  await reader.getByRole('button', { name: 'Edit', exact: true }).click();
  const editor = reader.locator('.cm-content');
  await editor.click();
  await page.keyboard.press('ControlOrMeta+End');
  await page.keyboard.type('\n\nA note from my draft.');
  await expect(reader.locator('.work-file-state')).toHaveText('Unsaved draft');
  // The draft survives a reload (it is stored, not only in memory).
  await page.goto('/?mock=1');
  reader = await openInReader(page);
  await expect(page.locator('.work-file-restored')).toContainText('Restored your unsaved draft');
  // Meanwhile the file changes on disk.
  await externalEdit(page, REVIEW, t => t.replace('## 2. Revenue', '## 2. Revenue (finance)'));
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  const resolver = page.locator('.conflict-resolver');
  await expect(resolver).toContainText('This file changed while you were editing');
  await expect(resolver).toContainText('The changes don’t overlap');
  await resolver.getByRole('button', { name: 'Changes on disk' }).click();
  await expect(resolver.locator('.diff-line.add')).toContainText('(finance)');
  await resolver.getByRole('button', { name: 'Review the merge…' }).click();
  await expect(resolver.getByLabel('Merged text')).toHaveValue(/Revenue \(finance\)[\s\S]*A note from my draft\./);
  await resolver.getByRole('button', { name: 'Save merged version' }).click();
  await expect(resolver).toHaveCount(0);
  const saved = await page.evaluate(async path => (await (await import('/src/mockApi.ts')).mockInvoke<{ content: string }>('read_work_file', { path })).content, REVIEW);
  expect(saved).toContain('(finance)');
  expect(saved).toContain('A note from my draft.');
  // After saving, the draft is gone: a reload shows no restore notice.
  await page.reload();
  reader = await openInReader(page);
  await expect(reader).toContainText('Q3 regional operations review');
  await expect(page.locator('.work-file-restored')).toHaveCount(0);
});

test('a second outside edit during a conflict rebuilds the merge instead of saving a stale one', async ({ page }, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'the Library file list keeps its 280 px column beside an open file, so at 390 px the reader is 50 px wide (below the 720 px window minimum)');
  // Slice 4 item 1a (review finding A·D2): the resolver used to seed its merge
  // once, so a second outside edit updated the comparison but not the merge
  // text, and saving wrote that stale merge over the second edit.
  const onDisk = () => page.evaluate(async path => (await (await import('/src/mockApi.ts')).mockInvoke<{ content: string }>('read_work_file', { path })).content, REVIEW);
  await open(page);
  const reader = await openInReader(page);
  await reader.getByRole('button', { name: 'Edit', exact: true }).click();
  await reader.locator('.cm-content').click();
  await page.keyboard.press('ControlOrMeta+End');
  await page.keyboard.type('\n\nA note from my draft.');
  await expect(reader.locator('.work-file-state')).toHaveText('Unsaved draft');
  await externalEdit(page, REVIEW, t => t.replace('## 2. Revenue', '## 2. Revenue (finance)'));
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  const resolver = page.locator('.conflict-resolver');
  await resolver.getByRole('button', { name: 'Review the merge…' }).click();
  await expect(resolver.getByLabel('Merged text')).toHaveValue(/Revenue \(finance\)[\s\S]*A note from my draft\./);
  // While that merge is on screen, the file changes on disk again.
  await externalEdit(page, REVIEW, t => t.replace('## 4. Decisions needed', '## 4. Decisions needed (board)'));
  await resolver.getByRole('button', { name: 'Save merged version' }).click();
  // The save is refused and the resolver starts over against the new file.
  await expect(resolver.getByRole('button', { name: 'Review the merge…' })).toBeVisible();
  const between = await onDisk();
  expect(between).toContain('Decisions needed (board)');
  expect(between).not.toContain('A note from my draft.');
  await resolver.getByRole('button', { name: 'Changes on disk' }).click();
  await expect(resolver.locator('.diff-line.add').filter({ hasText: 'Decisions needed (board)' })).toHaveCount(1);
  await resolver.getByRole('button', { name: 'Review the merge…' }).click();
  await expect(resolver.getByLabel('Merged text')).toHaveValue(/Revenue \(finance\)[\s\S]*Decisions needed \(board\)[\s\S]*A note from my draft\./);
  await resolver.getByRole('button', { name: 'Save merged version' }).click();
  await expect(resolver).toHaveCount(0);
  const saved = await onDisk();
  expect(saved).toContain('(finance)');
  expect(saved).toContain('Decisions needed (board)');
  expect(saved).toContain('A note from my draft.');
});
