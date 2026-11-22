import { expect, test } from './fixtures/test';
import AxeBuilder from '@axe-core/playwright';
import { join } from 'node:path';
import { choose } from './choose';

// Exercise the real tally and Settings view with a fake payload; no IPC write.
for (const theme of ['light', 'dark', 'glass']) test(`launch diagnostics masks secrets in the panel, tooltip and warning: ${theme}`, async ({ page }, info) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.addInitScript(theme => {
    localStorage.setItem('theme', theme === 'light' ? 'light' : 'dark');
    localStorage.setItem('glass', theme === 'glass' ? 'on' : 'off');
  }, theme);
  await page.goto('/?mock=1');
  await expect(page.locator('.app')).toBeVisible();
  const diagnostic = await page.evaluate(async () => {
    const tally = await import('/src/launchIpc.ts');
    const args = { id: 'synthetic-connector', input: { clientSecret: 'b4-short-secret', headers: { Custom: 'b4-header-value' }, env: { CUSTOM: 'b4-env-value' }, nested: [{ accessToken: 'b4-short-token' }] } };
    for (let i = 0; i < 3; i++) tally.noteIpc('test_connector_diagnostics', args, 0);
    const warnings: unknown[] = [];
    const previous = console.warn;
    console.warn = (...parts) => { warnings.push(parts); };
    try { tally.noteIpc('past_window', undefined, performance.now() + tally.LAUNCH_IPC_WINDOW_MS + 1); }
    finally { console.warn = previous; }
    return { report: tally.launchIpcReport(), warnings, original: args.input.clientSecret };
  });
  expect(diagnostic.original).toBe('b4-short-secret');
  expect(diagnostic.warnings).toHaveLength(1);
  const recorded = JSON.stringify({ report: diagnostic.report, warnings: diagnostic.warnings });
  for (const value of ['b4-short-secret', 'b4-header-value', 'b4-env-value', 'b4-short-token']) expect(recorded).not.toContain(value);
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) await choose(page.getByRole('combobox', { name: 'Settings section' }), 'advanced');
  else await page.locator('.settings-index').getByRole('button', { name: 'Advanced', exact: true }).click();
  // Launch calls sit in Advanced's closed Diagnostics group (Settings PR 3).
  await page.locator('#settings-group-diagnostics > summary').click();
  const row = page.locator('.launch-ipc-row').filter({ hasText: 'test_connector_diagnostics' });
  await expect(row).toBeVisible();
  await row.scrollIntoViewIfNeeded();
  await expect(row).toContainText('[redacted]');
  await expect(row.locator('.launch-ipc-count')).toHaveText('×3');
  const tooltip = await row.locator('.launch-ipc-args').getAttribute('title');
  expect(tooltip).toContain('synthetic-connector');
  for (const value of ['b4-short-secret', 'b4-header-value', 'b4-env-value', 'b4-short-token']) {
    expect(tooltip).not.toContain(value);
    await expect(page.locator('body')).not.toContainText(value);
  }
  expect((await new AxeBuilder({ page }).include('.launch-ipc-row').analyze()).violations).toEqual([]);
  if (theme === 'glass') { await expect(page.locator('html')).toHaveAttribute('data-glass', 'on'); await page.addStyleTag({ content: 'html[data-glass] { background:var(--canvas); }' }); }
  if (process.env.B4_CAPTURE_DIR) await row.locator('..').screenshot({ path: join(process.env.B4_CAPTURE_DIR, `b4-${theme}-${info.project.name}.png`) });
});
