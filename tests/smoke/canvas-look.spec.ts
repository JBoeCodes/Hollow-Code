import { expect, test, type Page } from './fixtures/test';
import { ROUTES } from '../../src/routeLedger';
import { home, walk } from './routeWalk';

const route = ROUTES.find(r => r.id === 'canvas.save-image')!;
const theme = { name: 'Paper', source: '', accent: '#aaaaaa', foreground: '#999999', background: '#fef0d0', isDark: false, colors: Array(16).fill(null), cursor: null, selectionForeground: null, selectionBackground: null, success: null, warning: null, error: null };
async function setup(page: Page) {
  await page.addInitScript(theme => localStorage.setItem('mockCustomThemes', JSON.stringify([theme])), theme);
  await home(page, route);
  const ws = await page.evaluate(async () => {
    const { api } = await import('/src/api.ts');
    return (await api.listWorkspaces()).find(w => !w.id.startsWith('_') && w.kind !== 'build')!;
  });
  await walk(page, route.walk.slice(0, -2));
  return ws;
}
async function picker(page: Page, title: string) {
  // A narrow window starts with the sidebar hidden (icon rail); show it to reach the project rows.
  const showSidebar = page.getByRole('button', { name: 'Show sidebar', exact: true });
  if (await showSidebar.isVisible()) await showSidebar.click();
  await page.getByRole('navigation', { name: 'Project navigation' }).getByRole('button', { name: title, exact: true }).click({ button: 'right' });
  await page.getByRole('menuitem', { name: /^Canvas look…/ }).click();
  const dialog = page.getByRole('dialog', { name: /^Canvas look ·/ });
  await expect(dialog).toBeFocused();
  return dialog;
}
const galleryFrame = (page: Page) => page.frameLocator('.result-canvas iframe').locator('html');
for (const appTheme of ['light', 'dark', 'glass']) test(`workspace picker updates Results and removal restores ${appTheme}`, async ({ page }, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'at 390 px the Canvas look dialog is taller than the window and scrolls, so Save is below the fold; this test asserts it is in view');
  const ws = await setup(page);
  await page.evaluate(theme => {
    document.documentElement.dataset.theme = theme === 'glass' ? 'dark' : theme;
    if (theme === 'glass') document.documentElement.dataset.glass = '';
  }, appTheme);
  const source = await page.evaluate(async id => {
    const { api } = await import('/src/api.ts');
    return (await api.readCanvas({ workspaceId: id }, 'release-status'))!.html;
  }, ws.id);
  const dialog = await picker(page, ws.title);
  await dialog.getByRole('combobox', { name: 'Canvas look' }).click();
  await page.getByRole('option', { name: 'Paper', exact: true }).click();
  await expect(dialog.getByText('Contrast adjustments')).toBeVisible();
  await expect(dialog.getByRole('img', { name: `Background ${theme.background}, text ${theme.foreground}, accent ${theme.accent}` })).toBeVisible();
  await expect(dialog.locator('.chat-artifact-head')).toHaveCount(0);
  await expect.poll(() => dialog.evaluate(el => el.getBoundingClientRect().width)).toBeLessThanOrEqual(640);
  await expect(dialog.getByRole('button', { name: 'Save', exact: true })).toBeInViewport();
  await expect.poll(() => dialog.frameLocator('iframe').locator('html').evaluate(el => getComputedStyle(el).getPropertyValue('--app-canvas'))).toBe(theme.background);
  await page.screenshot({ path: `${process.env.HOLLOW_SCRATCH_DIR ?? test.info().outputDir}/canvas-look-${appTheme}.png` });
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect.poll(() => galleryFrame(page).evaluate(el => getComputedStyle(el).getPropertyValue('--app-canvas'))).toBe(theme.background);
  expect(await page.evaluate(async id => {
    const { api } = await import('/src/api.ts');
    return (await api.readCanvas({ workspaceId: id }, 'release-status'))!.html;
  }, ws.id)).toBe(source);
  const remove = await picker(page, ws.title);
  await remove.getByRole('button', { name: 'Remove', exact: true }).click();
  const appBackground = await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--canvas').trim());
  await expect.poll(() => galleryFrame(page).evaluate(el => getComputedStyle(el).getPropertyValue('--app-canvas'))).toBe(appBackground);
});

test('live frames retain their document and state; another workspace stays on the app theme', async ({ page }) => {
  const ws = await setup(page);
  // Wait for this gallery's frame; the route click can finish before it mounts.
  const frame = galleryFrame(page);
  await expect.poll(() => frame.evaluate(() => location.href).catch(() => '')).toBe('about:srcdoc');
  await frame.evaluate(() => { (window as any).__lookSentinel = 'retained'; });
  await page.evaluate(async id => {
    const { api } = await import('/src/api.ts');
    const { refreshWorkspaces } = await import('/src/store/workspaces.ts');
    await api.setWorkspaceCanvasTheme(id, 'Paper');
    await refreshWorkspaces();
  }, ws.id);
  await expect.poll(() => galleryFrame(page).evaluate(el => getComputedStyle(el).getPropertyValue('--app-canvas'))).toBe(theme.background);
  expect(await frame.evaluate(() => (window as any).__lookSentinel)).toBe('retained');
  await frame.evaluate(() => {
    (window as any).__themeMessages = 0;
    window.addEventListener('hollow-theme', () => { (window as any).__themeMessages++; });
  });
  await page.evaluate(async () => {
    const { refreshWorkspaces } = await import('/src/store/workspaces.ts');
    const { refetchCustomThemes } = await import('/src/store/theme.ts');
    await refreshWorkspaces(); await refetchCustomThemes();
    const { namedLook } = await import('/src/store/canvasLook.ts');
    await new Promise(resolve => setTimeout(resolve, 100));
    if (!namedLook('Paper')?.look) throw Error('Refresh did not settle');
  });
  expect(await frame.evaluate(() => (window as any).__themeMessages)).toBe(0);
  await page.evaluate(async () => {
    const { setTheme } = await import('/src/store/theme.ts');
    setTheme('dark');
  });
  await expect.poll(() => galleryFrame(page).evaluate(el => getComputedStyle(el).getPropertyValue('--app-canvas'))).toBe(theme.background);
  // Another existing workspace gets its own saved canvas, then opens in the same gallery.
  await page.evaluate(async id => {
    const { api } = await import('/src/api.ts');
    const other = (await api.listWorkspaces()).find(w => w.id !== id && !w.kind)!;
    await api.recordCanvas({ workspaceId: other.id }, '<hc-canvas id="other-look" title="Other look"><p>Other workspace</p></hc-canvas>', null);
    const { openCanvasGallery } = await import('/src/store/canvasGallery.ts');
    const { closeCanvasGallery } = await import('/src/store/canvasGallery.ts');
    closeCanvasGallery();
    await new Promise(resolve => setTimeout(resolve, 0));
    await openCanvasGallery(other.id, { workspaceId: other.id, id: 'other-look', compare: false, back: null });
  }, ws.id);
  // The mock write does not emit the native vault-change event. Library is
  // already mounted, so refresh its listing through the visible control.
  await page.getByRole('region', { name: 'Results', exact: true }).getByRole('button', { name: 'Refresh results' }).click();
  await expect(page.locator('.result-canvas')).toContainText('Other look');
  const appBackground = await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--canvas').trim());
  await expect.poll(() => galleryFrame(page).evaluate(el => getComputedStyle(el).getPropertyValue('--app-canvas'))).toBe(appBackground);
});

test('missing theme falls back visibly and can be removed; cached edits update live', async ({ page }) => {
  const ws = await setup(page);
  await page.evaluate(async id => {
    const { api } = await import('/src/api.ts');
    const { refreshWorkspaces } = await import('/src/store/workspaces.ts');
    await api.setWorkspaceCanvasTheme(id, 'Paper'); await refreshWorkspaces();
  }, ws.id);
  await expect.poll(() => galleryFrame(page).evaluate(el => getComputedStyle(el).getPropertyValue('--app-canvas'))).toBe(theme.background);
  await page.evaluate(async () => {
    const list = JSON.parse(localStorage.getItem('mockCustomThemes')!); list[0].background = '#eeeeff';
    localStorage.setItem('mockCustomThemes', JSON.stringify(list));
    const { refetchCustomThemes } = await import('/src/store/theme.ts'); await refetchCustomThemes();
  });
  await expect.poll(() => galleryFrame(page).evaluate(el => getComputedStyle(el).getPropertyValue('--app-canvas'))).toBe('#eeeeff');
  await page.evaluate(async () => {
    localStorage.setItem('mockCustomThemes', '[]');
    const { refetchCustomThemes } = await import('/src/store/theme.ts'); await refetchCustomThemes();
  });
  const dialog = await picker(page, ws.title);
  await expect(dialog.getByRole('status')).toContainText("isn't on this Mac");
  await expect(dialog.getByRole('button', { name: 'Save', exact: true })).toBeDisabled();
  await dialog.getByRole('button', { name: 'Remove', exact: true }).click();
  await expect(dialog).toHaveCount(0);
});

test('a late theme read cannot revive a deleted palette', async ({ page }) => {
  const ws = await setup(page);
  await page.evaluate(async id => {
    const { api } = await import('/src/api.ts');
    const { refreshWorkspaces } = await import('/src/store/workspaces.ts');
    await api.setWorkspaceCanvasTheme(id, 'Paper'); await refreshWorkspaces();
  }, ws.id);
  await expect.poll(() => galleryFrame(page).evaluate(el => getComputedStyle(el).getPropertyValue('--app-canvas'))).toBe(theme.background);
  await page.evaluate(async () => {
    const { api } = await import('/src/api.ts');
    const read = api.readCustomTheme;
    (window as any).__restoreThemeRead = () => { api.readCustomTheme = read; };
    api.readCustomTheme = async name => {
      const old = await read(name);
      return new Promise(resolve => { (window as any).__releaseThemeRead = () => resolve(old); });
    };
    const { refetchCustomThemes } = await import('/src/store/theme.ts');
    await refetchCustomThemes();
  });
  await expect.poll(() => page.evaluate(() => typeof (window as any).__releaseThemeRead)).toBe('function');
  await expect.poll(() => galleryFrame(page).evaluate(el => getComputedStyle(el).getPropertyValue('--app-canvas'))).toBe(theme.background);
  await page.evaluate(async () => {
    (window as any).__restoreThemeRead();
    localStorage.setItem('mockCustomThemes', '[]');
    const { refetchCustomThemes } = await import('/src/store/theme.ts'); await refetchCustomThemes();
    (window as any).__releaseThemeRead();
  });
  const appBackground = await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--canvas').trim());
  await expect.poll(() => galleryFrame(page).evaluate(el => getComputedStyle(el).getPropertyValue('--app-canvas'))).toBe(appBackground);
  const dialog = await picker(page, ws.title);
  await expect(dialog.getByRole('status')).toContainText("isn't on this Mac");
});
