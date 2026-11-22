import { expect, test, type Page } from './fixtures/test';
import { chooseNew } from './choose';
import { openLibraryView } from './libraryView';

const source = `<hc-canvas id="live-status" title="Local progress" data-hollow-project="vault:projects/personal.md">
<hc-stats><hc-stat label="Open tasks" value="—" data-hollow-bind="tasks.open"></hc-stat>
<hc-stat label="Plan complete" value="—" data-hollow-bind="plan.done"></hc-stat>
<hc-stat label="Recorded today" value="—" data-hollow-bind="usage.reportedUsd"></hc-stat></hc-stats>
<hc-gauge label="Done" data-hollow-bind="plan.done"></hc-gauge>
<p>Local data only. <span data-hollow-bind="sessions.running">—</span> chats running.</p></hc-canvas>`;

async function setup(page: Page, body = source) {
  const destination = 'Projects';
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Track local progress.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Live pins');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Live pins');
  await page.evaluate(async html => {
    const path = '/src/api.ts', { api } = await import(path);
    const ws = (await api.listWorkspaces()).find((w: { title: string }) => w.title === 'Live pins');
    await api.recordCanvas({ workspaceId: ws.id, sessionKey: 's-live-source' }, html, null);
    localStorage.setItem('mockTasks', JSON.stringify([{ projectUrl: 'vault:projects/personal.md', bucket: 'today' }, { projectUrl: 'vault:projects/work.md', bucket: 'today' }]));
  }, body);
  await page.keyboard.press('Meta+k'); await page.locator('.palette-input').fill('canvas gallery'); await page.keyboard.press('Enter');
  const gallery = page.getByRole('region', { name: 'Results', exact: true }); await expect(gallery).toBeVisible();
  await gallery.locator('summary[aria-label="Pin Local progress"]').first().click();
  await gallery.getByRole('menuitem', { name: `Pin with live data to ${destination}`, exact: true }).click();
  await expect(gallery.getByText(`Pinned to ${destination}.`, { exact: true })).toBeVisible();
  // Library → Results → Canvases is a page view (consolidation lane B): Home
  // by ⌘1. (A workspace pin showed on its Overview until the COO rebuild L4
  // removed the workspace panel and new workspace pins.)
  await page.keyboard.press('Meta+1');
  const pin = page.locator('.pinned-canvas'); await expect(pin).toBeVisible(); await pin.scrollIntoViewIfNeeded();
  await expect(pin.locator('iframe')).toBeVisible();
  return pin;
}

test('pins from the gallery, refreshes exact-project values without remount, preserves missing costs', async ({ page }) => {
  const pin = await setup(page), frame = pin.frameLocator('iframe');
  await expect(frame.locator('hc-stat[label="Open tasks"]')).toHaveAttribute('value', '1');
  await expect(frame.locator('hc-stat[label="Recorded today"]')).toHaveAttribute('value', 'Unavailable');
  await pin.locator('iframe').evaluate(el => el.setAttribute('data-original', 'true'));
  await page.evaluate(() => localStorage.setItem('mockTasks', JSON.stringify([{ projectUrl: 'vault:projects/personal.md', bucket: 'done' }])));
  await pin.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(frame.locator('hc-stat[label="Open tasks"]')).toHaveAttribute('value', '0');
  await expect(pin.locator('iframe')).toHaveAttribute('data-original', 'true');
  await pin.locator('summary[aria-label="Canvas actions"]').click();
  await expect(pin.getByRole('menuitem', { name: /Allow internet/ })).toHaveCount(0);
  await expect(pin.getByRole('menuitem', { name: /Export canvas/ })).toHaveCount(0);
  await page.keyboard.press('Escape');
  const saves = await page.evaluate(() => window.__mockApiCalls?.record_canvas);
  await page.reload(); await page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true }).click(); await expect(page.locator('.pinned-canvas')).toBeVisible();
  expect(await page.evaluate(() => window.__mockApiCalls?.record_canvas ?? 0)).toBe(0);
  expect(saves).toBe(1);
  await page.locator('.pinned-canvas').getByRole('button', { name: 'Unpin', exact: true }).click();
  await expect(page.locator('.pinned-canvas')).toHaveCount(0);
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('hollow.canvasPins.v1') ?? '[]'))).toEqual([]);
  expect(await page.evaluate(() => document.activeElement?.tagName)).not.toBe('BODY');
});

test('pin feedback expires and its menu offers Show, Replace and Unpin', async ({ page }) => {
  await setup(page);
  const reopen = async () => {
    await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true }).click();
    await openLibraryView(page, 'Results');
    await page.getByRole('group', { name: 'Result type' }).getByRole('button', { name: 'Canvases', exact: true }).click();
  };
  await reopen();
  const gallery = page.getByRole('region', { name: 'Results', exact: true });
  await expect(gallery).toBeVisible();
  const menu = gallery.getByRole('button', { name: 'Pin Local progress', exact: true });
  await menu.click();
  await expect(gallery.getByRole('menuitem', { name: 'Unpin from Projects', exact: true })).toBeVisible();
  await gallery.getByRole('menuitem', { name: 'Replace pin in Projects', exact: true }).click();
  const notice = gallery.getByRole('status').filter({ hasText: 'Pinned to Projects.' });
  await expect(notice.getByRole('button', { name: 'Show', exact: true })).toBeVisible();
  await expect(notice).toHaveCount(0, { timeout: 6000 });
  await menu.click();
  await gallery.getByRole('menuitem', { name: 'Replace pin in Projects', exact: true }).click();
  await gallery.getByRole('status').filter({ hasText: 'Pinned to Projects.' }).getByRole('button', { name: 'Show', exact: true }).click();
  await expect(page.locator('.pinned-canvas')).toBeVisible();
  await reopen();
  await menu.click();
  await gallery.getByRole('menuitem', { name: 'Unpin from Projects', exact: true }).click();
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('hollow.canvasPins.v1')!))).toEqual([]);
});

test('read errors retain the pin, show unavailable values and recover on refresh', async ({ page }) => {
  const pin = await setup(page);
  await page.evaluate(() => localStorage.setItem('mockUsageError', '1'));
  await pin.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(pin.getByText('usage unavailable', { exact: false })).toBeVisible();
  await expect(pin.frameLocator('iframe').locator('hc-stat[label="Recorded today"]')).toHaveAttribute('value', 'Unavailable');
  await page.evaluate(() => localStorage.removeItem('mockUsageError'));
  await pin.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(pin.getByText('usage unavailable', { exact: false })).toHaveCount(0);
});

test('new revisions do not change the pinned source or add bindings', async ({ page }) => {
  const pin = await setup(page);
  await page.evaluate(async () => {
    const path = '/src/api.ts', { api } = await import(path);
    const ws = (await api.listWorkspaces()).find((w: { title: string }) => w.title === 'Live pins');
    const scope = { workspaceId: ws.id, sessionKey: 's-other-source' };
    const c = await api.readCanvas(scope, 'live-status');
    await api.recordCanvas(scope, '<hc-canvas id="live-status" title="Changed">New unapproved content</hc-canvas>', c.sha256);
  });
  await pin.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(pin.getByText(/A newer version exists/)).toBeVisible();
  await expect(pin.frameLocator('iframe').locator('hc-stat[label="Open tasks"]')).toHaveAttribute('value', '1');
  await expect(pin.frameLocator('iframe').getByText('New unapproved content')).toHaveCount(0);
  const old = await page.evaluate(() => JSON.parse(localStorage.getItem('hollow.canvasPins.v1')!)[0]);
  await page.evaluate(async () => {
    const { api } = await import('/src/api.ts');
    const read = api.readCanvasRevision;
    (window as any).__restorePinRead = () => { api.readCanvasRevision = read; };
    api.readCanvasRevision = async () => { throw Error('revision unavailable'); };
  });
  await pin.getByRole('button', { name: 'Update to v2', exact: true }).click();
  await expect(pin.getByRole('alert')).toContainText('revision unavailable');
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('hollow.canvasPins.v1')!)[0])).toEqual(old);
  await page.evaluate(() => (window as any).__restorePinRead());
  await pin.getByRole('button', { name: 'Update to v2', exact: true }).click();
  await expect(pin.frameLocator('iframe').getByText('New unapproved content')).toBeVisible();
  const expected = await page.evaluate(async () => {
    const { api } = await import('/src/api.ts');
    const { sha256Hex } = await import('/src/sectionMarkdown.ts');
    const pin = JSON.parse(localStorage.getItem('hollow.canvasPins.v1')!)[0];
    const revision = await api.readCanvasRevision({ workspaceId: pin.workspaceId }, pin.id, 2);
    return { pin, hash: sha256Hex(revision.html) };
  });
  expect(expected.pin).toMatchObject({ revision: 2, sha256: expected.hash });
});

test('pins stop reading and release the frame offscreen; foreign-vault pins stay hidden', async ({ page }) => {
  const pin = await setup(page);
  await page.evaluate(() => document.querySelector('.pinned-canvases')?.setAttribute('style', 'margin-top:5000px'));
  await expect(pin.locator('iframe')).toHaveCount(0);
  const before = await page.evaluate(() => window.__mockApiCalls?.read_canvas ?? 0);
  await page.clock.install(); await page.clock.fastForward(90_000);
  expect(await page.evaluate(() => window.__mockApiCalls?.read_canvas ?? 0)).toBe(before);
  await pin.scrollIntoViewIfNeeded(); await expect(pin.locator('iframe')).toBeVisible();
  await page.evaluate(() => {
    const pins = JSON.parse(localStorage.getItem('hollow.canvasPins.v1')!); pins[0].vault = '/other-vault';
    localStorage.setItem('hollow.canvasPins.v1', JSON.stringify(pins));
  });
  await page.reload(); await page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true }).click(); await expect(page.getByRole('textbox', { name: 'Ask anything' })).toBeVisible();
  await expect(page.locator('.pinned-canvas')).toHaveCount(0);
});

for (const theme of ['light', 'dark', 'glass']) test(`live pin ${theme} visual and reduced-motion check`, async ({ page }, info) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.addInitScript(t => {
    localStorage.setItem('theme', t === 'glass' ? 'dark' : t);
    if (t === 'glass') localStorage.setItem('glass', 'on'); else localStorage.removeItem('glass');
  }, theme);
  const pin = await setup(page);
  await expect(pin.frameLocator('iframe').locator('hc-stat[label="Open tasks"]')).toHaveAttribute('value', '1');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: info.outputPath(`live-pin-${theme}.png`), fullPage: true });
});
