import { expect, test } from './fixtures/test';
import { choose } from './choose';
import { contrast, parseColor } from '../../src/canvas/kit/palette';

async function interfaceSettings(page: import('@playwright/test').Page) {
  await page.goto('/?mock=1');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  const section = page.getByRole('combobox', { name: 'Settings section' });
  if ((page.viewportSize()?.width ?? 1440) <= 860) await choose(section, 'general');
  else await page.locator('.settings-index').getByRole('button', { name: 'General', exact: true }).click();
}

test('texture choices keep old ambient consent (the artwork preview was cut in Settings PR 6, C15)', async ({ page }) => {
  await page.addInitScript(() => { if (!localStorage.getItem('hollow.studio.experimentalVisuals')) localStorage.setItem('hollow.studio.experimentalVisuals','off'); });
  await page.emulateMedia({ reducedMotion:'no-preference' });
  await interfaceSettings(page);
  const texture=page.getByRole('group',{name:'Texture',exact:true});
  const ambient=page.getByRole('switch',{name:'Ambient motion',exact:true});
  await expect(ambient).toHaveAttribute('aria-checked','false');
  await expect(page.locator('.settings-visual-art')).toHaveCount(0);
  await texture.getByRole('button',{name:'Quiet',exact:true}).click();
  await expect(page.locator('html')).toHaveAttribute('data-texture-level','quiet');
  await texture.getByRole('button',{name:'Off',exact:true}).click();
  await expect(page.locator('html')).toHaveAttribute('data-texture-level','off');
  await interfaceSettings(page);
  await expect(texture.getByRole('button',{name:'Off',exact:true})).toHaveAttribute('aria-pressed','true');
  await expect(ambient).toHaveAttribute('aria-checked','false');
  await texture.getByRole('button',{name:'Full',exact:true}).click();
  await expect(page.locator('html')).toHaveAttribute('data-texture-level','full');
});

test('every accepted text scale has a visible Settings choice', async ({ page }) => {
  await interfaceSettings(page);
  const accepted = await page.evaluate(async () => {
    const path = '/src/store/studio.ts';
    return (await import(/* @vite-ignore */ path)).TEXT_SCALES.length;
  });
  await expect(page.getByRole('group', { name: 'Text size', exact: true }).getByRole('button')).toHaveCount(accepted);
});

test('Obsidian tells native controls to use the dark color scheme', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('theme', 'dark'));
  await page.goto('/?mock=1');
  await expect(page.locator('html')).toHaveCSS('color-scheme', 'dark');
});

test('forced colours keep the selected texture option visible', async ({ page }) => {
  await page.emulateMedia({ forcedColors: 'active' });
  await interfaceSettings(page);
  const texture = page.getByRole('group', { name: 'Texture', exact: true });
  await texture.getByRole('button', { name: 'Quiet', exact: true }).click();
  const quiet = texture.getByRole('button', { name: 'Quiet', exact: true });
  const full = texture.getByRole('button', { name: 'Full', exact: true });
  await expect(quiet).toHaveCSS('text-decoration-line', 'underline');
  await expect(full).toHaveCSS('text-decoration-line', 'none');
  await quiet.focus();
  await page.keyboard.press('Tab');
  await page.keyboard.press('Shift+Tab');
  await expect(quiet).toHaveCSS('outline-style', 'solid');
  await expect(quiet).toHaveCSS('outline-offset', '3px');
});

test('storeless windows clamp unsupported stored text sizes before rendering', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('hollow.studio.textScale', '999'));
  await page.goto('/?mock=1&window=smoke-artifact');
  await expect(page.locator('html')).toHaveAttribute('data-text-scale', '100');
});

test('storeless windows apply saved Texture Off before mounting a canvas', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('hollow.studio.texture', 'off'));
  // The quick-capture window this also checked was removed in S5 (2026-11-03).
  await page.goto('/?mock=1&window=smoke-artifact');
  await expect(page.locator('html')).toHaveAttribute('data-texture-level', 'off');
  const canvas = page.locator('.smoke-harness > section').first().locator('iframe');
  await expect.poll(() => canvas.contentFrame().locator('body').evaluate(() =>
    (window as unknown as { hollowCanvas?: { textureAllowed: boolean } }).hollowCanvas?.textureAllowed,
  )).toBe(false);
});

test('mounted canvas and document bodies follow Largest without a frame reload', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('hollow.studio.textScale', '100'));
  await page.goto('/?mock=1&window=smoke-artifact');
  const canvas = page.locator('.smoke-harness > section').first().locator('iframe');
  const documentFrame = page.locator('.smoke-harness > section').nth(1).locator('iframe');
  const canvasBody = canvas.contentFrame().locator('body');
  const documentBody = documentFrame.contentFrame().locator('body');
  await expect(canvasBody).toHaveCSS('font-size', '14px');
  await expect(documentBody).toHaveCSS('font-size', '15px');
  await canvasBody.evaluate(el => el.dataset.scaleWitness = 'retained');
  await page.evaluate(() => { document.documentElement.dataset.textScale = '125'; });
  await expect(canvasBody).toHaveCSS('font-size', '17.5px');
  await expect(documentBody).toHaveCSS('font-size', '18.75px');
  await expect(canvasBody).toHaveAttribute('data-scale-witness', 'retained');
});

test('Match macOS follows live changes until an explicit palette is selected', async ({ page }) => {
  // Start on an explicit palette so Use exercises a real selection.
  await page.addInitScript(() => localStorage.setItem('theme', 'light'));
  await page.emulateMedia({ colorScheme: 'light' });
  await interfaceSettings(page);
  const follow = page.getByRole('combobox', { name: 'Theme', exact: true });
  await choose(follow, { label: 'Match macOS' });
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expect(follow).toContainText('Match macOS');
  await choose(follow, { label: 'Light' });
  await page.emulateMedia({ colorScheme: 'light' });
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await expect(follow).not.toContainText('Match macOS');
});

test('the app bridge retains the system preference instead of only its effective polarity', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('theme', 'light'));
  await interfaceSettings(page);
  const theme = page.getByRole('combobox', { name: 'Theme', exact: true });
  await choose(theme, { label: 'Match macOS' });
  const preference = await page.evaluate(async () => {
    const path = '/src/appBridge.ts';
    const { UI_SETTINGS } = await import(/* @vite-ignore */ path);
    return await UI_SETTINGS.theme.get();
  });
  expect(preference).toBe('system');
  await choose(theme, { label: 'Light' });
  await page.evaluate(async () => {
    const path = '/src/appBridge.ts';
    const { UI_SETTINGS } = await import(/* @vite-ignore */ path);
    await UI_SETTINGS.theme.set('system');
  });
  await expect(theme).toContainText('Match macOS');
});

for (const newer of ['stock', 'custom', 'pending custom']) test(`the app bridge keeps the newer ${newer} choice when custom reads finish out of order`, async ({ page }) => {
  await interfaceSettings(page);
  const selected = await page.evaluate(async newer => {
    const bridgePath = '/src/appBridge.ts';
    const apiPath = '/src/api.ts';
    const { UI_SETTINGS } = await import(/* @vite-ignore */ bridgePath);
    const { api } = await import(/* @vite-ignore */ apiPath);
    const read = api.readCustomTheme;
    const fixture: import('../../src/types').CustomTheme = {
      name: 'Fixture', source: '', isDark: true, accent: '#bbbbbb',
      foreground: '#ffffff', background: '#000000', colors: [],
      cursor: null, selectionForeground: null, selectionBackground: null,
      success: null, warning: null, error: null,
    };
    let finishOld!: (value: typeof fixture) => void;
    let finishNew!: (value: typeof fixture) => void;
    api.readCustomTheme = (name: string) => new Promise(resolve => {
      if (name === 'Old') finishOld = resolve;
      else finishNew = resolve;
    });
    try {
      await UI_SETTINGS.theme.set('light');
      const old = UI_SETTINGS.theme.set('custom:Old');
      let during = 'light';
      if (newer !== 'stock') {
        const next = UI_SETTINGS.theme.set('custom:New');
        if (newer === 'pending custom') {
          finishOld({ ...fixture, name: 'Old' });
          await old;
          during = await UI_SETTINGS.theme.get();
        }
        finishNew({ ...fixture, name: 'New' });
        await next;
      } else await UI_SETTINGS.theme.set('light');
      finishOld({ ...fixture, name: 'Old' });
      await old;
      return { during, final: await UI_SETTINGS.theme.get() };
    } finally { api.readCustomTheme = read; }
  }, newer);
  expect(selected).toEqual({ during: 'light', final: newer === 'stock' ? 'light' : 'custom:New' });
});

for (const theme of ['light', 'dark']) test(`Increase Contrast gives inputs a 3:1 border in ${theme}`, async ({ page }) => {
  await page.addInitScript(theme => localStorage.setItem('theme', theme), theme);
  await page.emulateMedia({ contrast: 'more' });
  await interfaceSettings(page);
  await page.getByRole('button', { name: 'Edit themes…', exact: true }).click();
  await page.getByRole('dialog', { name: 'Edit themes' }).getByRole('button', { name: 'New theme…', exact: true }).click();
  const colors = await page.locator('textarea').first().evaluate(el => {
    const css = getComputedStyle(el);
    return [css.borderTopColor, css.backgroundColor];
  });
  expect(contrast(parseColor(colors[0])!, parseColor(colors[1])!)).toBeGreaterThanOrEqual(3);
});

test('Reduce Transparency makes the glass window opaque', async ({ page, context }) => {
  const cdp = await context.newCDPSession(page);
  await cdp.send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-transparency', value: 'reduce' }] });
  await page.addInitScript(() => { localStorage.setItem('glass', 'on'); });
  await page.goto('/?mock=1');
  await expect(page.locator('html')).toHaveAttribute('data-glass', 'on');
  await expect(page.locator('.work-rail')).toBeAttached();
  await expect(page.locator('body')).toHaveCSS('background-color', /^rgb\(/);
  await expect(page.locator('.work-rail')).toHaveCSS('background-color', /^rgb\(/);
});

test('the theme editor measures derived text and does not invent ratios for invalid drafts', async ({ page }) => {
  await interfaceSettings(page);
  await page.getByRole('button', { name: 'Edit themes…', exact: true }).click();
  await page.getByRole('dialog', { name: 'Edit themes' }).getByRole('button', { name: 'New theme…', exact: true }).click();
  const content = page.getByRole('textbox', { name: 'Theme source', exact: true });
  await content.fill('accent #bbbbbb\nforeground #ffffff\nbackground #000000\ncolor8 #4d4d4d\nis_dark true');
  const report = page.getByRole('status', { name: 'Theme text contrast' });
  await expect(report).toContainText('4.5:1');
  await expect(report).toContainText('adjusted');
  await content.fill('foreground nope');
  await expect(report).toContainText('unavailable');
  await expect(report).not.toContainText('AA met');
});
