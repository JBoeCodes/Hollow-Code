import { createHash } from 'node:crypto';
import { expect, test, type Page } from './fixtures/test';
import { chooseNew } from './choose';

/**
 * Plan L3 X2–X5 (2026-10-14; review U library, ASG 5): a source says what it
 * is first (a snapshot, and how it relates to its original), offers View
 * snapshot / Open original / Check for changes, and a reviewed refresh shows
 * the changed lines, refuses when either side moved on after the check, and
 * records the refresh in SOURCES.md. The Library reader on a file already
 * attached offers View snapshot instead of attaching it again. The workspace
 * panel's Sources cards went in the COO rebuild L4 (2026-11-10), so these run
 * from the Library reader, which keeps the same source actions.
 *
 * The mock reads an original under `…/vault/` from the vault fixture
 * (`mockWorkspaces.ts`); the mock vault root is `/tmp/hollow/vault`.
 */

const SNAPSHOT = '# Plan\n\nOld line\n';
const ORIGINAL = '# Plan\n\nNew line\nAnother line\n';
const sha = (text: string) => createHash('sha256').update(text, 'utf8').digest('hex');
const CAPTURED = '2026-10-02T09:00:00Z';
const MANIFEST = `# Sources\n\n## plan.md\n\n- Snapshot: [plan.md](sources/plan.md)\n- Original: "/tmp/hollow/vault/library/plan.md"\n- Captured: ${CAPTURED}\n- SHA-256: \`${sha(SNAPSHOT)}\`\n`;

async function seed(page: Page) {
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Keep the plan current.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Sources check');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Sources check');
  const root = await page.evaluate(([snapshot, original, manifest, snapSha, captured]) => {
    const w = JSON.parse(localStorage.getItem('mockWorkspaces')!)[0];
    const notes = JSON.parse(localStorage.getItem('mockWorkspaceNotes') ?? '{}');
    Object.assign(notes, {
      'library/plan.md': original,
      [`${w.path}/SOURCES.md`]: manifest,
      [`${w.path}/sources/plan.md`]: snapshot,
      [`${w.path}/sources/.provenance/plan.md.json`]: JSON.stringify({ original: '/tmp/hollow/vault/library/plan.md', captured, sha256: snapSha, size: snapshot.length, modified: null }),
      [`${w.path}/sources/gone.md`]: '# Gone\n',
      [`${w.path}/sources/.provenance/gone.md.json`]: JSON.stringify({ original: '/Users/fixture/Downloads/gone.md', captured, sha256: 'd'.repeat(64), size: 7, modified: null }),
    });
    localStorage.setItem('mockWorkspaceNotes', JSON.stringify(notes));
    localStorage.setItem('mockSourceFreshness', JSON.stringify({ 'sources/plan.md': 'changed', 'sources/gone.md': 'unavailable' }));
    return w.path as string;
  }, [SNAPSHOT, ORIGINAL, MANIFEST, sha(SNAPSHOT), CAPTURED] as const);
  await page.reload();
  await expect(page.locator('.work-header h1')).toHaveText('Sources check');
  return root;
}

/** The Library reader on the original, `library/plan.md`. */
async function openOriginal(page: Page) {
  await page.evaluate(async () => {
    const path = '/src/store/runtime.ts', runtime = await import(/* @vite-ignore */ path);
    runtime.setActivePage('vault');
    runtime.setOpenVaultNoteRequest('library/plan.md');
  });
  await expect(page.getByRole('heading', { name: 'Library', exact: true })).toBeVisible();
  const reader = page.getByRole('region', { name: 'File reader' });
  await expect(reader).toContainText('New line');
  return reader;
}
const readMock = (page: Page, path: string) => page.evaluate(p => JSON.parse(localStorage.getItem('mockWorkspaceNotes') ?? '{}')[p] as string, path);
const calls = (page: Page, command: string) => page.evaluate(c => (window.__mockApiCalls ?? {})[c] ?? 0, command);

test.beforeEach(({}, info) => { test.skip(info.project.name === 'chromium-narrow', 'desktop layout'); });

test('Check for changes shows the changed lines, refuses a stale replace, then replaces and records it', async ({ page }) => {
  const root = await seed(page);
  const plan = await openOriginal(page);
  await plan.getByRole('button', { name: 'View snapshot', exact: true }).click();
  await expect(plan.locator('.work-file-state')).toHaveText('Snapshot · read only');
  await page.evaluate(() => localStorage.setItem('mockSourceRefreshConflict', 'true'));
  await plan.getByRole('button', { name: 'Check for changes', exact: true }).click();
  const panel = page.getByRole('region', { name: 'Check for changes: plan.md' });
  await expect(panel.locator('.src-refresh-status')).toHaveText(/^The original changed: 17 bytes → 30 bytes\. Captured 2 Oct( \d{4})?\.$/);
  await expect(panel.locator('.src-refresh-diff li[data-kind="removed"]')).toContainText(['Old line']);
  await expect(panel.locator('.src-refresh-diff li[data-kind="added"]')).toContainText(['New line', 'Another line']);
  await expect(panel).not.toContainText('drops those edits');
  expect(await calls(page, 'apply_source_refresh')).toBe(0);

  // The original moved on after the check: nothing is replaced.
  await panel.getByRole('button', { name: 'Replace snapshot', exact: true }).click();
  await expect(panel.getByRole('alert')).toHaveText('The original or the snapshot changed after you checked. Check again.');
  expect(await readMock(page, `${root}/sources/plan.md`)).toBe(SNAPSHOT);
  await panel.getByRole('button', { name: 'Check again', exact: true }).click();
  await expect(panel.getByRole('button', { name: 'Replace snapshot', exact: true })).toBeVisible();

  await panel.getByRole('button', { name: 'Replace snapshot', exact: true }).click();
  await expect(plan.locator('.source-replaced')).toHaveText(/^Snapshot replaced\. SOURCES\.md records the refresh/);
  expect(await calls(page, 'apply_source_refresh')).toBe(2);
  expect(await readMock(page, `${root}/sources/plan.md`)).toBe(ORIGINAL);
  const manifest = await readMock(page, `${root}/SOURCES.md`);
  expect(manifest.match(/\n## plan\.md\n/g)).toHaveLength(2);
  expect(manifest).toContain(`- SHA-256: \`${sha(ORIGINAL)}\`\n- Refreshed: replaced the snapshot captured ${CAPTURED}`);
  expect(await readMock(page, `${root}/sources/.previous/${sha(SNAPSHOT).slice(0, 12)}-plan.md`)).toBe(SNAPSHOT);

  // A second check compares content: same now.
  await plan.getByRole('button', { name: 'Check for changes', exact: true }).click();
  await expect(panel.locator('.src-refresh-status')).toHaveText('The original matches the snapshot (same content).');
});

test('the Library reader on an attached original offers View snapshot rather than attaching again', async ({ page }) => {
  await seed(page);
  const reader = await openOriginal(page);
  await expect(reader.locator('.source-state strong')).toHaveText('Library note: edits save here');
  await expect(reader).toContainText('New line');
  await expect(reader.getByRole('button', { name: 'View snapshot', exact: true })).toBeVisible();
  await expect(reader).toContainText('Already in Sources check as a snapshot.');
  await expect(reader.getByRole('button', { name: /^Use in / })).toHaveCount(0);
  const attaches = await calls(page, 'attach_workspace_source');

  await reader.getByRole('button', { name: 'View snapshot', exact: true }).click();
  await expect(reader.locator('.work-file-state')).toHaveText('Snapshot · read only');
  await expect(reader.locator('.source-state strong')).toHaveText(/^Snapshot captured 2 Oct( \d{4})?; the original has changed since$/);
  await expect(reader.getByRole('button', { name: 'Check for changes', exact: true })).toBeVisible();
  expect(await calls(page, 'attach_workspace_source')).toBe(attaches);
});
