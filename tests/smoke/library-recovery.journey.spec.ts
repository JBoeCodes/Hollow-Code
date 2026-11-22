import { expect, test, type Page } from './fixtures/test';
import { openLibraryView } from './libraryView';

// Required desktop/minimum/Largest coverage for independent recovery paths.

async function openResults(page: Page) {
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true }).click();
  await openLibraryView(page, 'Results');
}

test('files can be refreshed while canvas stores remain hung', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await expect(page.locator('.home-project-row').first()).toBeVisible();
  await page.evaluate(async () => {
    const path = '/src/api.ts', { api } = await import(/* @vite-ignore */ path);
    api.listCanvases = () => new Promise(() => {});
  });
  await openResults(page);
  await expect(page.locator('.library-result-row').first()).toBeVisible();
  await expect(page.getByText('Reading canvas stores…', { exact: true })).toBeVisible();
  const refresh = page.getByRole('button', { name: 'Refresh results', exact: true });
  await expect(refresh).toBeEnabled();
  await page.evaluate(() => {
    const w = window as unknown as { __mockOutsideWrite: (path: string, body: string, at: string) => void };
    w.__mockOutsideWrite('workspaces/local-q3-review/outputs/hung-store-file.md', '# Available despite a hung store\n', new Date().toISOString());
  });
  await refresh.click();
  const row = page.locator('.library-result-row').filter({ hasText: 'Available despite a hung store' });
  await expect(row).toBeVisible();
  await row.click();
  await expect(page.getByRole('complementary', { name: 'Preview', exact: true })).toBeVisible();
});

test('failed automatic store refresh retains a selected canvas and comparison state', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await expect(page.locator('.home-project-row').first()).toBeVisible();
  await page.evaluate(async () => {
    const path = '/src/mockApi.ts', { mockInvoke } = await import(/* @vite-ignore */ path);
    let sha: string | null = null;
    for (const n of [1, 2]) {
      const saved = await mockInvoke('record_canvas', {
        workspaceId: 'local-q3-review', sessionKey: null,
        body: `<hc-canvas id="retained-plan" title="Retained plan"><p>Version ${n}</p></hc-canvas>`,
        expectedSha256: sha, turnId: null, kit: 'kit@1',
      }) as { canvas: { sha256: string } };
      sha = saved.canvas.sha256;
    }
  });
  await openResults(page);
  await page.getByRole('button', { name: 'Retained plan', exact: true }).click();
  const detail = page.getByRole('region', { name: 'Selected canvas', exact: true });
  const compare = detail.getByRole('button', { name: 'Compare versions', exact: true });
  await compare.click();
  await expect(detail.locator('.cg-compare figcaption')).toHaveText(['Before · v1', 'After · v2']);
  await detail.locator('summary', { hasText: 'View source changes' }).click();
  const original = await detail.elementHandle();
  await page.evaluate(async () => {
    const path = '/src/api.ts', { api } = await import(/* @vite-ignore */ path);
    api.listCanvases = async () => { throw new Error('Synthetic store refresh failure'); };
  });
  await compare.focus();
  // Deliver the watcher input to its store; no native filesystem write runs.
  await page.evaluate(async () => {
    const path = '/src/store/vaultChanges.ts', { setVaultChange } = await import(/* @vite-ignore */ path);
    setVaultChange({ folders: ['workspaces'], paths: ['workspaces/local-q3-review/outputs/agent-result.md'], seq: Date.now() });
  });
  await expect(page.getByText('Some canvas stores couldn’t be read', { exact: true })).toBeVisible();
  await expect(detail).toBeVisible();
  expect(await original!.evaluate(el => el.isConnected)).toBe(true);
  await expect(compare).toHaveAttribute('aria-pressed', 'true');
  await expect(compare).toBeFocused();
  await expect(detail.locator('.cg-source')).toHaveAttribute('open', '');
  await expect(page.getByRole('button', { name: 'Retained plan', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByText(/Last known canvases, when available, remain visible/)).toBeVisible();
});
