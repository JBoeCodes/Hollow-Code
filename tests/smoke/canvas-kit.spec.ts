import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page } from './fixtures/test';
import { KIT_FIXTURE } from '../../src/canvas/kit/fixture';

// Canvas C2 (kit@1). Outside Tauri the artifact renders from srcdoc in the
// default Offline mode: its CSP allows only the runtime's and the kit's
// hashes, so these tests also prove the kit hash works and artifact scripts
// stay blocked.

const kitFrame = (page: Page) =>
  page.getByRole('region', { name: 'Canvas kit fixture' }).frameLocator('iframe[title="Rendered HTML artifact"]');

test('kit components upgrade in the default Offline document while artifact scripts stay blocked', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/?mock=1&window=smoke-artifact');
  const region = page.getByRole('region', { name: 'Canvas kit fixture' });
  await expect(region.locator('.chat-artifact')).toHaveAttribute('data-sandbox', 'offline');
  const frame = kitFrame(page);
  await expect(frame.getByRole('heading', { name: 'Kit fixture' })).toBeVisible();
  await expect(frame.locator('body')).not.toHaveAttribute('data-script-ran', '1');
  expect(await frame.locator('body').evaluate(() => (window as unknown as { HollowKit?: { version: string } }).HollowKit?.version)).toBe('kit@1');

  // hc-stat: label, value and a toned delta that carries a word, not just colour.
  const bed = frame.locator('hc-stat[label="Bed checks"]');
  await expect(bed).toHaveAttribute('data-hc-ready', '');
  await expect(frame.getByRole('group', { name: 'Release' })).toBeVisible();
  await expect(bed.locator('.hc-stat-value')).toHaveText('38');
  await expect(bed.locator('.hc-stat-delta')).toHaveText(/Success:\s*0 fail/);
  await expect(frame.locator('hc-stat[label="Skipped"] .hc-stat-delta')).toHaveText(/▲\s*\+2/);
  // Attribute values reach the page as text, never markup.
  await expect(frame.locator('hc-stat[label="Injected"] .hc-stat-value')).toHaveText('<img src=x onerror=alert(1)>');
  await expect(frame.locator('img')).toHaveCount(0);

  const bar = frame.getByRole('progressbar', { name: 'Plan' });
  await expect(bar).toHaveAttribute('aria-valuenow', '3');
  await expect(bar).toHaveAttribute('aria-valuemax', '5');
  await expect(frame.locator('hc-progress .hc-progress-text')).toHaveText('3 of 5');
  // Reduced motion: the fill doesn't animate.
  expect(await frame.locator('.hc-progress-fill').evaluate(el => getComputedStyle(el).transitionDuration)).toBe('0s');

  const callout = frame.getByRole('note');
  await expect(callout).toContainText('Heads up');
  await expect(callout).toContainText('Install waits for the bed.');
  await expect(callout).not.toHaveAttribute('title', /.*/);

  // hc-details is a keyboard-operable disclosure.
  const toggle = frame.getByRole('button', { name: 'Why it matters' });
  const body = frame.getByText('Hidden reasoning body.');
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await expect(body).toBeHidden();
  await toggle.focus();
  await page.keyboard.press('Enter');
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
  await expect(body).toBeVisible();
  await page.keyboard.press('Space');
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await expect(body).toBeHidden();
});

test('kit components follow a live theme switch', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('theme', 'light'));
  await page.goto('/?mock=1&window=smoke-artifact');
  const badge = kitFrame(page).locator('hc-badge');
  await expect(badge).toHaveAttribute('data-hc-ready', '');
  const iconColour = () => badge.evaluate(el => getComputedStyle(el, '::before').color);
  const hostDanger = () => page.evaluate(() => {
    const probe = document.createElement('span');
    probe.style.color = 'var(--error)';
    document.body.append(probe);
    const c = getComputedStyle(probe).color;
    probe.remove();
    return c;
  });
  await expect.poll(iconColour).toBe(await hostDanger());
  const light = await iconColour();
  await page.evaluate(() => { document.documentElement.dataset.theme = 'dark'; });
  const dark = await hostDanger();
  expect(dark).not.toBe(light);
  await expect.poll(iconColour).toBe(dark);
});

test('without the kit script, kit markup stays readable', async ({ page }) => {
  // The same fixture with the kit stylesheet but no kit script (before upgrade,
  // or anywhere the kit isn't inlined), and scripts blocked outright.
  const css = readFileSync(join(process.cwd(), 'src/canvas/kit/generated/kit.min.css'), 'utf8');
  await page.setContent(`<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="script-src 'none'">
<style>${css}</style></head><body>${KIT_FIXTURE}</body></html>`);
  await expect(page.locator('[data-hc-ready]')).toHaveCount(0);
  await expect(page.locator('body')).not.toHaveAttribute('data-script-ran', '1');
  for (const text of ['Install waits for the bed.', 'Blocked', 'Hidden reasoning body.']) {
    await expect(page.getByText(text)).toBeVisible();
  }
  const generated = (selector: string, pseudo: '::before' | '::after') =>
    page.locator(selector).evaluate((el, p) => getComputedStyle(el, p).content, pseudo);
  expect(await generated('hc-stat[label="Bed checks"]', '::before')).toContain('Bed checks');
  expect(await generated('hc-stat[label="Bed checks"]', '::after')).toContain('38');
  expect(await generated('hc-progress', '::before')).toContain('Plan');
  expect(await generated('hc-progress', '::after')).toContain('5');
  expect(await generated('hc-callout', '::before')).toContain('Heads up');
  expect(await generated('hc-details', '::before')).toContain('Why it matters');
});
