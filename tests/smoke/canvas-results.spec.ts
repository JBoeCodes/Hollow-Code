import { expect, test, type Page } from './fixtures/test';
import { openWorkspaceResult, previewPane, resultReviewBar } from './choose';

/**
 * Canvas C3 · S3 in the browser mock: a saved chat canvas
 * (`outputs/canvases/<id>.canvas.html` plus its `.canvas.json` sidecar and
 * `.revisions/` store, as `workspace_canvases.rs` writes them) is one Result
 * of kind Canvas, titled from its sidecar with its current revision. Only the
 * person accepts it, and a new revision reads as changed. The files are
 * seeded straight into the mock vault; nothing renders a chat fence here.
 * Since the COO rebuild L4 (2026-11-10) it opens in the Preview pane under its
 * review bar (it was the workspace panel's Results section).
 */

const DIR = 'workspaces/local-q3-review/outputs/canvases';
const body = (title: string, inner: string) => `<hc-canvas id="release-status" title="${title}">\n${inner}\n</hc-canvas>\n`;
const sidecar = (title: string, revision: number) => JSON.stringify({ hollowCanvas: 1, id: 'release-status', title, revision, fenceSha256: 'f'.repeat(64), revisions: [] });

/** Writes the canvas's three files into the mock vault, as the store would. */
async function saveCanvas(page: Page, title: string, revision: number, inner: string) {
  await page.evaluate(({ files }) => {
    const notes = JSON.parse(localStorage.getItem('mockWorkspaceNotes') ?? '{}') as Record<string, string>;
    const times = JSON.parse(localStorage.getItem('mockWorkspaceFileTimes') ?? '{}') as Record<string, string>;
    const at = new Date().toISOString();
    for (const [path, text] of files) { notes[path] = text; times[path] = at; }
    localStorage.setItem('mockWorkspaceNotes', JSON.stringify(notes));
    localStorage.setItem('mockWorkspaceFileTimes', JSON.stringify(times));
  }, { files: [
    [`${DIR}/release-status.canvas.html`, body(title, inner)],
    [`${DIR}/release-status.canvas.json`, sidecar(title, revision)],
    [`${DIR}/.revisions/release-status/${String(revision).repeat(64).slice(0, 64)}.canvas.html`, body(title, inner)],
  ] });
  // A fresh page reads the vault again (no demo reseed).
  await page.goto('/?mock=1');
}

/** The canvas's file in the Preview pane, under its review bar. */
async function openCanvasResult(page: Page) {
  await openWorkspaceResult(page, 'local-q3-review', `${DIR}/release-status.canvas.html`);
  await expect(previewPane(page).getByRole('tab', { name: 'release-status.canvas.html', exact: true })).toHaveAttribute('aria-selected', 'true');
  const bar = resultReviewBar(page);
  await expect(bar).toBeVisible();
  return bar;
}

/** The workspace's Results as the mock lists them (the review bar's read). */
const resultPaths = (page: Page) => page.evaluate(async () => {
  const src = '/src/mockApi.ts', { mockInvoke } = await import(/* @vite-ignore */ src);
  return (await mockInvoke('workspace_results', { id: 'local-q3-review' }) as { path: string }[]).map(r => r.path);
});

test('a saved canvas is one Canvas result, accepted by the person, changed by a new revision', async ({ page }, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'the Preview pane split is desktop-only');
  await page.goto('/?mock=1&demo=1');
  await expect(page.locator('.home-project-row').first()).toBeVisible();
  await saveCanvas(page, 'Ship readiness', 3, '<p>29 checks passed</p>');
  await expect(page.locator('.home-project-row').first()).toBeVisible();

  // One result, titled from the sidecar with its revision: neither the
  // sidecar nor the revision store is a result of its own.
  const paths = await resultPaths(page);
  expect(paths.filter(p => p.startsWith(`${DIR}/`))).toEqual([`${DIR}/release-status.canvas.html`]);
  const bar = await openCanvasResult(page);
  await expect(bar.locator('.result-origin')).toContainText('Canvas v3');

  // Accept records this version; a new revision needs its own review.
  const states = bar.locator('.result-states');
  await bar.getByRole('button', { name: 'Accept', exact: true }).click();
  await expect(states).toContainText('Accepted');
  await saveCanvas(page, 'Ship readiness', 4, '<p>31 checks passed</p>');
  // The reload comes back to the open project, not Home: wait for the sidebar's projects.
  await expect(page.getByRole('navigation', { name: 'Project navigation' }).getByRole('button', { name: 'Q3 regional operations review', exact: true })).toBeVisible();
  const revised = await openCanvasResult(page);
  await expect(revised.locator('.result-states')).toContainText('Changed since accepted');
  await expect(revised.locator('.result-origin')).toContainText('Canvas v4');
  await expect(revised.getByRole('button', { name: 'Accept this new version', exact: true })).toBeVisible();
});
