import { expect, test, type Page } from './fixtures/test';

// The found-context review card (CX20) at the top of Library, Learnings:
// Scan this Mac lists what the mock scan found, each item decided on its
// own. Fixture and knobs: src/mockContextReview.ts.

async function openLearnings(page: Page) {
  await page.goto('/?mock=1');
  await page.getByRole('button', { name: 'Library', exact: true }).click();
  await page.getByRole('navigation', { name: 'Library view' }).getByRole('button', { name: 'Learnings', exact: true }).click();
  return page.getByRole('region', { name: 'Found context', exact: true });
}

test('Scan this Mac lists found files; Accept copies one, a secret stays blocked', async ({ page }) => {
  await page.addInitScript(() => { localStorage.setItem('mockContextLayers', 'connected'); });
  const card = await openLearnings(page);
  await expect(card.getByRole('heading', { name: 'Found context', exact: true })).toBeVisible();
  await card.getByRole('button', { name: 'Scan this Mac', exact: true }).click();
  await expect(card.getByText('Found 7 files.', { exact: true })).toBeVisible();

  // No select-all and no bulk accept: one Accept per item.
  await expect(card.getByRole('checkbox')).toHaveCount(0);

  // A duplicate says so, naming the first file while that one is still listed
  // (once it is accepted it leaves the list and the line names no file).
  await expect(card.locator('[data-review-id="ctx-c20a00000004"]').getByText(/Same text as CLAUDE\.md/)).toBeVisible();

  const claude = card.locator('[data-review-id="ctx-c20a00000001"]');
  await expect(claude.getByText('Personal (suggested)', { exact: true })).toBeVisible();
  await claude.getByRole('button', { name: 'Accept', exact: true }).click();
  await expect(card.getByText(/Copied into your personal context as memory\/imported\/claude-code\/CLAUDE\.md/)).toBeVisible();
  await expect(claude).toHaveCount(0);

  // A secret-like file can't be accepted and says why.
  const secret = card.locator('[data-review-id="ctx-c20a00000003"]');
  await expect(secret.getByRole('button', { name: 'Accept', exact: true })).toBeDisabled();
  // Two lines say so: the excerpt's "No excerpt" and the note on why it stays blocked.
  await expect(secret.locator('.context-review-withheld')).toHaveText(/looks like it holds a secret/);
  await expect(secret.getByRole('note')).toContainText(/won't copy it anywhere/);

  // A project's file is a suggested pull request in its own repo; Accept writes nothing.
  const project = card.locator('[data-review-id="ctx-c20a00000005"]');
  await expect(project.getByText(/Hollow writes nothing and opens nothing/)).toBeVisible();

  // Skip moves an item under Skipped; Unskip brings it back.
  const ship = card.locator('[data-review-id="ctx-c20a00000002"]');
  await ship.getByRole('button', { name: 'Skip', exact: true }).click();
  await card.getByText('Skipped (1)', { exact: true }).click();
  await card.locator('[data-review-id="ctx-c20a00000002"]').getByRole('button', { name: 'Unskip', exact: true }).click();
  await expect(card.getByText(/^Skipped \(/)).toHaveCount(0);
});

test('An auto-memory entry shows why it got its layer; Home copies one, Stale proposes Skip', async ({ page }) => {
  await page.addInitScript(() => { localStorage.setItem('mockContextLayers', 'connected'); });
  const card = await openLearnings(page);
  await card.getByRole('button', { name: 'Scan this Mac', exact: true }).click();
  await expect(card.getByText('Found 7 files.', { exact: true })).toBeVisible();

  const runner = card.locator('[data-review-id="ctx-c20a00000006"]');
  await expect(runner.getByText('Home (this Mac) (suggested)', { exact: true })).toBeVisible();
  await expect(runner.getByText(/^Why this layer: Home by rules disk-runner/)).toBeVisible();
  await runner.getByRole('button', { name: 'Accept', exact: true }).click();
  await expect(card.getByText(/Copied into this Mac's home folder as home\/mock-mac\/runner\.md/)).toBeVisible();
  await expect(runner).toHaveCount(0);

  const index = card.locator('[data-review-id="ctx-c20a00000007"]');
  await expect(index.getByText('Stale (Skip it) (suggested)', { exact: true })).toBeVisible();
  await expect(index.getByRole('button', { name: 'Accept', exact: true })).toBeDisabled();
  await expect(index.getByText(/^Hollow proposes Skip for this entry\. Stale by rule index/)).toBeVisible();
  await index.getByRole('button', { name: 'Skip', exact: true }).click();
  await expect(card.getByText('Skipped (1)', { exact: true })).toBeVisible();
});

test('Without the clones Accept says to connect them; an org fact confirms before its pull request', async ({ page }) => {
  await page.addInitScript(() => { localStorage.setItem('mockContextReview', 'org'); });
  const card = await openLearnings(page);
  const fact = card.locator('[data-review-id="org-c20a00000001"]');
  await expect(fact.getByText('Organization (suggested)', { exact: true })).toBeVisible();
  await expect(fact.getByRole('button', { name: 'Accept and open pull request', exact: true })).toBeDisabled();
  await expect(fact.getByText('Connect the organization context in Settings, Sources, Context first.', { exact: true })).toBeVisible();
});

test('An org fact opens its pull request only after the confirm', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('mockContextReview', 'org');
    localStorage.setItem('mockContextLayers', 'connected');
  });
  const card = await openLearnings(page);
  const fact = card.locator('[data-review-id="org-c20a00000001"]');
  await fact.getByRole('button', { name: 'Accept and open pull request', exact: true }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByText('Open a pull request?', { exact: true })).toBeVisible();
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(fact).toBeVisible();
  await fact.getByRole('button', { name: 'Accept and open pull request', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Open pull request', exact: true }).click();
  await expect(card.getByText(/Pull request opened: https:\/\/github\.com\/mock-org\/org-context\/pull\/12/)).toBeVisible();
  await expect(fact).toHaveCount(0);
});
