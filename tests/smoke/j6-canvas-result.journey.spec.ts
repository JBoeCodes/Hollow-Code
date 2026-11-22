/**
 * Journey J6 (consolidation lane B, 2026-10-13): inspect, compare and review a
 * canvas Result using visible controls only, no `g x`, no palette, no keys.
 * Library → Results → Canvases lists every saved canvas with the workspace
 * that owns it; the current version stays marked apart from older ones;
 * Open workspace goes to its owner, and the Library's canvases are one
 * click away again (its Results section went with the workspace panel in
 * the COO rebuild L4, 2026-11-10).
 * A required spec: it runs at the desktop size and at the native minimum
 * window, with and without Largest text, and may not skip
 * (scripts/check-required-specs.mjs).
 */
import { expect, test, type Page } from './fixtures/test';
import { openLibraryView } from './libraryView';

const canvas = (build: number) => `<hc-canvas id="release-status" title="Release status">
  <p>Build ${build}</p>
</hc-canvas>`;
async function noHorizontalScroll(page: Page) {
  const [scroll, client] = await page.evaluate(() => [document.scrollingElement!.scrollWidth, document.scrollingElement!.clientWidth]);
  expect(scroll, 'no horizontal page scroll').toBeLessThanOrEqual(client);
}

test('click-only: Library → Results → Canvases, compare versions, open the owner and come back to its canvases', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/?mock=1&demo=1');
  // Fixture: two saved versions of one canvas in the first demo workspace.
  const owner = await page.evaluate(async ([v1, v2]) => {
    const path = '/src/api.ts', { api } = await import(/* @vite-ignore */ path);
    const ws = (await api.listWorkspaces()).find((w: { kind?: string; id: string }) => !w.id.startsWith('_') && w.kind !== 'build') as { id: string; title: string };
    const first = await api.recordCanvas({ workspaceId: ws.id, sessionKey: 's-journey' }, v1, null);
    await api.recordCanvas({ workspaceId: ws.id, sessionKey: 's-journey' }, v2, first.canvas.sha256);
    return ws.title;
  }, [canvas(1), canvas(2)]);

  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true }).click();
  await openLibraryView(page, 'Results');
  await page.getByRole('group', { name: 'Result type' }).getByRole('button', { name: 'Canvases', exact: true }).click();
  const gallery = page.getByRole('region', { name: 'Results', exact: true });
  await expect(gallery).toBeVisible();
  await expect(page.locator('[aria-modal="true"]')).toHaveCount(0);

  // Workspace ownership and the current version, as Results shows them.
  const item = gallery.getByRole('button', { name: 'Release status', exact: true });
  await expect(item).toContainText(`${owner} · Canvas v2`);
  await item.click();
  await expect(gallery.locator('.result-canvas')).toHaveAttribute('data-revision', '2');
  // Add to Notion page… went with Notion writes (2026-11-10).
  await expect(gallery.getByRole('button', { name: /Add to Notion page/ })).toHaveCount(0);
  // Compare: the older version and the current one, side by side.
  await gallery.getByRole('button', { name: 'Compare versions' }).click();
  await expect(gallery.locator('.cg-compare figcaption')).toHaveText(['Before · v1', 'After · v2']);
  await gallery.getByText('View source changes', { exact: true }).click();
  await expect(gallery.locator('.cg-diff')).toContainText('Build 1');
  await noHorizontalScroll(page);

  // To the owner, and back to its canvases through the Library.
  // The gallery's own words stay Open workspace (docs/CANVAS.md, Gallery row).
  await gallery.getByRole('button', { name: 'Open workspace', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText(owner);
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true }).click();
  await openLibraryView(page, 'Results');
  await page.getByRole('group', { name: 'Result type' }).getByRole('button', { name: 'Canvases', exact: true }).click();
  await expect(gallery).toBeVisible();
  await expect(gallery.getByRole('button', { name: 'Release status', exact: true })).toContainText(`${owner} · Canvas v2`);
  await noHorizontalScroll(page);
});
