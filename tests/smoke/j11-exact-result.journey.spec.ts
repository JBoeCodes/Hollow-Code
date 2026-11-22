/**
 * Journey J11 (plan completion L5, review V07 / UI review journey 3, matrix
 * V7d): review an exact Result with visible controls only, no palette, no
 * keys. Inspect its version and origin (the conversation that wrote it) and
 * its checks; accept exactly that version; an agent then writes a new one;
 * leave and come back; compare it with the accepted version; draft a
 * revision request; leave again (Library) and come back: the review state,
 * the accepted version and the unsent request are all still there; accept
 * the new version, and the history keeps both versions apart. The Result
 * opens in the Preview pane with its review bar (COO rebuild L4: it was the
 * workspace panel's Results section), from Library → Results.
 * A required spec: desktop, native minimum window, and Largest text; it may
 * not skip (scripts/check-required-specs.mjs).
 */
import { expect, test, type Locator, type Page } from './fixtures/test';
import { previewPane, resultReviewBar, showWorkspaceList } from './choose';
import { openLibraryView } from './libraryView';

const REVIEW = 'workspaces/local-q3-review/outputs/q3-regional-review.md';

/** An agent (or an outside editor) writes a new version through the mock IPC. */
async function agentWrites(page: Page, from: string, to: string) {
  await page.evaluate(async ({ path, from, to }) => {
    const src = '/src/mockApi.ts', { mockInvoke } = await import(/* @vite-ignore */ src);
    const note = await mockInvoke('read_work_file', { path }) as { content: string };
    const r = await mockInvoke('vault_save_note_cas', { path, content: note.content.replace(from, to), baseline: note.content }) as { conflict: boolean };
    if (r.conflict) throw new Error('fixture edit conflicted');
  }, { path: REVIEW, from, to });
}

/** A rail destination; a small window's sidebar first needs "Show sidebar". */
async function rail(page: Page, name: string) {
  const target = page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name, exact: true });
  const shown = await target.waitFor({ state: 'visible', timeout: 2_000 }).then(() => true, () => false);
  if (!shown) await page.getByRole('button', { name: 'Show sidebar', exact: true }).click();
  await target.click();
}

/** Projects → the Q3 workspace (its complete list works at every size). */
async function openQ3(page: Page) {
  await rail(page, 'Projects');
  await showWorkspaceList(page);
  await page.locator('.work-card').filter({ hasText: 'Q3 regional operations review' }).first().click();
  await expect(page.locator('.top-bar-crumbs')).toContainText('Q3 regional operations review');
}

/** Library → Results → the review result's "in its workspace" button: the
 *  workspace opens with the file in the Preview pane under its review bar. */
async function openResult(page: Page): Promise<Locator> {
  await rail(page, 'Library');
  await openLibraryView(page, 'Results');
  await page.getByRole('region', { name: 'Results', exact: true })
    .getByRole('button', { name: 'Open Q3 regional operations review in its workspace', exact: true }).first().click();
  await expect(page.locator('.top-bar-crumbs')).toContainText('Q3 regional operations review');
  await expect(previewPane(page).getByRole('tab', { name: 'q3-regional-review.md', exact: true })).toHaveAttribute('aria-selected', 'true');
  const bar = resultReviewBar(page);
  await expect(bar).toBeVisible();
  return bar;
}

async function noHorizontalScroll(page: Page) {
  const [scroll, client] = await page.evaluate(() => [document.scrollingElement!.scrollWidth, document.scrollingElement!.clientWidth]);
  expect(scroll, 'no horizontal page scroll').toBeLessThanOrEqual(client);
}

test('click-only: inspect, accept, compare, draft a revision, leave and come back to the same state', async ({ page }) => {
  test.setTimeout(60_000);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/?mock=1&demo=1');
  await openQ3(page);
  let detail = await openResult(page);
  const states = detail.locator('.result-states');

  // 1. Version, origin and checks, before anything is decided.
  await expect(states).toContainText('Draft');
  await expect(states).toContainText('Not checked');
  await expect(states.locator('.result-origin')).toContainText('Written by Revise the labour section');
  // The pane's tab names the file.
  await expect(previewPane(page).getByRole('tab', { name: 'q3-regional-review.md', exact: true })).toBeVisible();
  await noHorizontalScroll(page);

  // The origin is the exact conversation that wrote it.
  await states.locator('.result-origin').getByRole('button', { name: 'Revise the labour section' }).click();
  await expect(page.getByText('I revised').first()).toBeVisible();
  detail = await openResult(page);

  // 2. Accept exactly this version.
  await detail.getByRole('button', { name: 'Accept', exact: true }).click();
  await expect(states).toContainText('Accepted');
  await expect(page.getByText(/Accepted “Q3 regional operations review”\. Nothing was sent or published\./)).toBeVisible();
  const history = detail.locator('.result-history');
  await history.locator('summary').click();
  const firstVersion = (await history.locator('li').filter({ hasText: 'Accepted' }).first().innerText()).match(/version ([0-9a-f]{7})/)?.[1];
  expect(firstVersion, 'the history names the accepted version').toBeTruthy();

  // 3. An agent writes a new version; the person leaves and comes back.
  await agentWrites(page, 'Draft 3', 'Draft 4');
  await rail(page, 'Tasks');
  await expect(page.locator('.top-bar-crumbs')).toContainText('Tasks');
  await openQ3(page);
  detail = await openResult(page);
  await expect(detail.locator('.result-states')).toContainText('Changed since accepted');
  await expect(detail.locator('.result-note')).toContainText('The accepted version is kept; this one needs its own review.');

  // 4. Compare with the accepted version.
  await detail.getByRole('button', { name: 'Compare with accepted' }).click();
  await expect(detail.getByRole('button', { name: 'Show the file' })).toBeVisible();
  await expect(page.locator('.diff-view')).toContainText('Accepted version');
  await expect(page.locator('.diff-line.del').first()).toContainText('Draft 3');
  await expect(page.locator('.diff-line.add').first()).toContainText('Draft 4');

  // 5. Draft a revision request (it goes to the composer; nothing is sent).
  await detail.getByRole('button', { name: 'Ask for a revision' }).click();
  const composer = page.getByLabel('Chat message');
  await expect(composer).toHaveValue(/^Revise outputs\/q3-regional-review\.md \(“Q3 regional operations review”\): $/);
  await composer.pressSequentially('keep Draft 3’s section 3 wording');
  const draft = await composer.inputValue();

  // 6. Leave to the Library and come back: everything is where it was.
  await rail(page, 'Library');
  await expect(page.getByRole('navigation', { name: 'Library view' })).toBeVisible();
  await openQ3(page);
  await expect(page.getByLabel('Chat message')).toHaveValue(draft);
  detail = await openResult(page);
  await expect(detail.locator('.result-states')).toContainText('Changed since accepted');
  await detail.locator('.result-history summary').click();
  await expect(detail.locator('.result-history')).toContainText(`version ${firstVersion}`);
  const sentTurns = await page.evaluate(() => (window as unknown as { __mockApiCalls?: Record<string, number> }).__mockApiCalls?.chat_send ?? 0);
  expect(sentTurns, 'the revision request was only drafted').toBe(0);

  // 7. Accept the new version (the button says it is a new version): the
  // history keeps the two versions apart.
  await detail.getByRole('button', { name: 'Accept this new version', exact: true }).click();
  await expect(detail.locator('.result-states')).toContainText('Accepted');
  const acceptedVersions = async () => new Set((await detail.locator('.result-history li').filter({ hasText: 'Accepted' }).allTextContents())
    .map(v => v.match(/version ([0-9a-f]{7})/)?.[1]).filter(Boolean)).size;
  await expect.poll(acceptedVersions, { message: 'two different accepted versions in the history' }).toBe(2);
  await noHorizontalScroll(page);
});
