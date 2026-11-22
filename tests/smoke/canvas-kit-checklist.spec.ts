import { expect, test, type Page } from './fixtures/test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
const generated = resolve('src/canvas/kit/generated');
async function load(page: Page, body: string, ask = false) {
  await page.setContent(`<style>${readFileSync(resolve(generated, 'kit.min.css'), 'utf8')}${readFileSync(resolve(generated, 'kit-ask.min.css'), 'utf8')}</style>${body}`);
  await page.evaluate(() => {
    const w = window as any;
    w.hollowCanvas = { init: { canvasId: 'checks' }, get: () => null, set: () => true };
    w.drafts = [];
    addEventListener('message', e => { if (e.data?.kind === 'draft') w.drafts.push(e.data); });
  });
  await page.addScriptTag({ content: readFileSync(process.env.HOLLOW_CHECKLIST_BASELINE || resolve(generated, 'kit.min.js'), 'utf8') });
  if (ask) await page.addScriptTag({ content: readFileSync(resolve(generated, 'kit-ask.min.js'), 'utf8') });
}
/** Hollow's Send (question cards, 2026-10-14): the host asks the frame for
 *  its answers. Here the page is its own host. */
const collect = (page: Page) => page.evaluate(() => window.postMessage({ __hc: 1, kind: 'collect', nonce: '0123456789abcdef0123456789abcdef' }, '*'));
const form = `<hc-ask title="Verify" send="Use checks"><hc-checklist name="checks" label="Hand checks" required>[x] Keyboard
[ ] VoiceOver</hc-checklist></hc-ask>`;

test('static checklists show completion in words without task controls', async ({ page }) => {
  await load(page, '<hc-checklist label="Hand checks">[x] Build\n[ ] Native check</hc-checklist>');
  await expect(page.getByRole('region', { name: 'Hand checks' })).toBeVisible();
  await expect(page.getByRole('listitem')).toHaveCount(2);
  await expect(page.getByRole('listitem').first()).toHaveText('✓BuildDone');
  await expect(page.getByRole('listitem').last()).toHaveText('○Native checkNot done');
  await expect(page.locator('input,button')).toHaveCount(0);
});

test('ask checks start unanswered, require each row and draft exact choices', async ({ page }) => {
  await load(page, form, true);
  await expect(page.getByRole('radio')).toHaveCount(6);
  await expect(page.locator('input:checked')).toHaveCount(0);
  await collect(page);
  await expect(page.getByText('Answer every row (2 left).')).toBeVisible();
  await expect(page.getByRole('radio', { name: 'Keyboard: Pass', exact: true })).toBeFocused();
  await page.keyboard.press('Space');
  await page.getByRole('radio', { name: 'VoiceOver: Skip', exact: true }).check();
  await collect(page);
  await expect.poll(() => page.evaluate(() => (window as any).drafts.at(-1)?.answers)).toEqual({ checks: { Keyboard: 'Pass', VoiceOver: 'Skip' } });
  await page.evaluate(() => window.postMessage({ __hc: 1, kind: 'answered', summary: 'Keyboard: Pass; VoiceOver: Skip' }, '*'));
  await expect(page.getByRole('status')).toHaveText('Answered. You sent these answers.');
  await expect(page.locator('hc-matrix')).toHaveAttribute('inert', '');
});

test('wrapper rerenders preserve selections and prototype-looking labels are data', async ({ page }) => {
  await load(page, '<hc-ask title="Verify" send="Use checks"><hc-checklist name="checks" label="Checks">[ ] __proto__\n[ ] constructor</hc-checklist></hc-ask>', true);
  await page.getByRole('radio', { name: '__proto__: Fail', exact: true }).check();
  await page.locator('hc-checklist').evaluate(el => el.setAttribute('label', 'Updated checks'));
  await expect(page.getByRole('radio', { name: '__proto__: Fail', exact: true })).toBeChecked();
  await page.getByRole('radio', { name: 'constructor: Pass', exact: true }).check();
  await collect(page);
  await expect.poll(() => page.evaluate(() => JSON.stringify((window as any).drafts.at(-1)?.answers))).toBe('{"checks":{"__proto__":"Fail","constructor":"Pass"}}');
});

test('optional checklists draft null, duplicate rows and empty lists are disclosed', async ({ page }) => {
  await load(page, '<hc-ask title="Verify" send="Use checks"><hc-checklist name="checks" label="Checks">[x] Build\n[ ] Build</hc-checklist><hc-checklist name="empty" label="Empty"></hc-checklist></hc-ask>', true);
  await expect(page.getByText('1 repeated row shown once.')).toBeVisible();
  await expect(page.getByText('No checks listed.')).toBeVisible();
  await collect(page);
  await expect.poll(() => page.evaluate(() => (window as any).drafts.at(-1)?.answers)).toEqual({ checks: null, empty: null });
});

test('literal star labels stay distinct and retain exact answer keys', async ({ page }) => {
  await load(page, '<hc-ask title="Verify" send="Use checks"><hc-checklist name="checks" label="Checks" required>[ ] * A\n[ ] A</hc-checklist></hc-ask>', true);
  await expect(page.getByRole('radio')).toHaveCount(6);
  await page.getByRole('radio', { name: '* A: Fail', exact: true }).check();
  await page.getByRole('radio', { name: 'A: Skip', exact: true }).check();
  await collect(page);
  await expect.poll(() => page.evaluate(() => (window as any).drafts.at(-1)?.answers)).toEqual({ checks: { '* A': 'Fail', A: 'Skip' } });
});

test('label changes preserve focus and validation, source revisions refresh rows', async ({ page }) => {
  await load(page, form, true);
  await collect(page);
  await page.locator('hc-checklist').evaluate(el => el.setAttribute('label', 'Updated'));
  await expect(page.getByRole('radio', { name: 'Keyboard: Pass', exact: true })).toBeFocused();
  await expect(page.getByText('Answer every row (2 left).')).toBeVisible();
  await page.getByRole('radio', { name: 'Keyboard: Pass', exact: true }).check();
  await page.locator('hc-checklist').evaluate(el => {
    el.querySelector(':scope > [data-hc-source]')!.textContent = '[ ] Keyboard\n[ ] Contrast';
    el.setAttribute('label', 'Revised');
  });
  await expect(page.getByRole('radio', { name: 'VoiceOver: Skip', exact: true })).toHaveCount(0);
  await expect(page.getByRole('radio', { name: 'Contrast: Skip', exact: true })).toBeVisible();
  await expect(page.getByRole('radio', { name: 'Keyboard: Pass', exact: true })).toBeChecked();
  await expect(page.getByText('Answer every row (1 left).')).toBeVisible();
});

test('a checklist falls back to its static report when ask kit is absent', async ({ page }) => {
  await load(page, form);
  await expect(page.getByRole('list')).toBeVisible();
  await expect(page.getByRole('listitem')).toHaveCount(2);
});

test('checklists upgrade with both kits in the head before markup parses', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  const kit = readFileSync(resolve(generated, 'kit.min.js'), 'utf8');
  const ask = readFileSync(resolve(generated, 'kit-ask.min.js'), 'utf8');
  await page.setContent(`<!doctype html><html><head><script>${kit}</script><script>${ask}</script></head><body>${form}</body></html>`);
  await expect(page.getByRole('radio')).toHaveCount(6);
  await page.getByRole('radio', { name: 'Keyboard: Pass', exact: true }).check();
  await expect(page.getByRole('radio', { name: 'Keyboard: Pass', exact: true })).toBeChecked();
  expect(errors).toEqual([]);
});


test('checklist revisions retain scroll-region focus and recover removed row focus', async ({ page }) => {
  await load(page, form, true);
  const scroll = page.locator('.hc-matrix-scroll');
  await scroll.focus();
  await page.locator('hc-checklist').evaluate(el => el.setAttribute('label', 'Updated checks'));
  await expect(scroll).toBeFocused();
  await page.getByRole('radio', { name: 'VoiceOver: Pass', exact: true }).focus();
  await page.locator('hc-checklist').evaluate(el => {
    el.querySelector(':scope > [data-hc-source]')!.textContent = '[ ] Keyboard\n[ ] Contrast';
    el.setAttribute('label', 'Revised checks');
  });
  await expect(scroll).toBeFocused();
});
