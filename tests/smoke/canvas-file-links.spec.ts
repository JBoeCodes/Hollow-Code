import { expect, test, type Page } from './fixtures/test';

// hc-file links in chat and curated canvases (HtmlArtifact): a click asks the
// host over bridge `navigate`, and the host opens the vault file in the side
// panel's Files view, resolved against the conversation's folder and kept
// inside it or the vault. A message the person didn't trigger opens nothing.

const REGION = 'Canvas file links';
const opened = (page: Page) => page.evaluate(() => [...(window as unknown as { openedFiles: string[] }).openedFiles]);

async function open(page: Page) {
  await page.goto('/?mock=1&window=smoke-artifact');
  const frame = page.getByRole('region', { name: REGION }).frameLocator('iframe[title="Rendered HTML artifact"]');
  await expect(frame.locator('hc-file').first()).toHaveAttribute('data-hc-ready', '');
  return frame;
}

// Activated from the keyboard (focus, Enter: a real input event, so user
// activation): the harness page reflows while the frames above it load, which
// can leave a pointer click on the page body under load. The kit spec covers
// the click → `navigate` half.
test('an activated hc-file opens that workspace file; one outside the vault opens nothing', async ({ page }) => {
  const frame = await open(page);
  await frame.getByRole('link', { name: /Report/ }).focus();
  await page.keyboard.press('Enter');
  await expect.poll(() => opened(page)).toEqual(['/tmp/hollow/vault/workspaces/w1/outputs/report.md']);

  await frame.getByRole('link', { name: /Outside/ }).focus();
  await page.keyboard.press('Enter');
  await page.waitForTimeout(300);
  expect(await opened(page)).toHaveLength(1);
});

test('a navigate the person did not click opens nothing', async ({ page }) => {
  const frame = await open(page);
  // Focus stays in the host page: the frame never got a click.
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await frame.locator('body').evaluate(() => parent.postMessage({ __hc: 1, kind: 'navigate', target: 'outputs/report.md' }, '*'));
  await page.waitForTimeout(300);
  expect(await opened(page)).toEqual([]);
});
