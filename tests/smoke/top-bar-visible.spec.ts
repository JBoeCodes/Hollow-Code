import { expect, test, type Page } from './fixtures/test';
import { waitingBell } from './waiting';
import { chooseNew, openProjectFiles, previewPane, projectFilesButton } from './choose';

/**
 * The top bar never leaves the window (10-11 fix). The ambient layer
 * (styles/motion.css) reaches 20% past `.canvas`; while `.canvas` was
 * `overflow: hidden` it stayed a scroll container, so a focus or
 * scrollIntoView in the chat scrolled the whole column, top bar and side
 * panel tabs included, up under the window edge. Measured, not eyeballed,
 * at a wide window like the one it was reported in.
 */

test.use({ viewport: { width: 1920, height: 813 } });

/** Every scroll the app (or WebKit's focus handling) might ask for. */
async function tryToScrollTheColumn(page: Page) {
  await page.evaluate(() => {
    for (const el of [document.querySelector('[aria-label="Chat message"]'), document.querySelector('.page')?.lastElementChild]) {
      (el as HTMLElement | null)?.scrollIntoView({ block: 'center' });
      (el as HTMLElement | null)?.scrollIntoView({ block: 'end' });
    }
    const canvas = document.querySelector<HTMLElement>('.canvas');
    if (canvas) canvas.scrollTop = 10_000;
  });
}

async function expectTopBarInView(page: Page) {
  expect(await page.evaluate(() => document.querySelector<HTMLElement>('.canvas')!.scrollTop)).toBe(0);
  const bar = await page.locator('.top-bar').boundingBox();
  expect(bar?.y).toBe(0);
  expect(bar?.height).toBeGreaterThanOrEqual(44);
  // Search and New left the top bar for the sidebar (overnight 11-04 U1);
  // its global group keeps the bell and the Main agent orb.
  await expect(page.locator('.top-bar-global').getByRole('button', { name: /^(Search anything|New)$/ })).toHaveCount(0);
  for (const control of [
    waitingBell(page),
    page.locator('.top-bar-global').getByRole('button', { name: /^(COO|Back from the COO) ·/ }),
  ]) {
    await expect(control).toBeVisible();
    const box = await control.boundingBox();
    expect(box!.y).toBeGreaterThanOrEqual(0);
    expect(box!.y + box!.height).toBeLessThanOrEqual(44);
  }
}

test('Home: the top bar and its global buttons stay in the window', async ({ page }) => {
  await page.goto('/?mock=1');
  await expect(page.locator('.top-bar-global')).toBeVisible();
  await tryToScrollTheColumn(page);
  await expectTopBarInView(page);
});

test('a workspace chat, Project files on and off: the top bar stays in the window', async ({ page }) => {
  await page.goto('/?mock=1');
  // A project (Chats has no Project files).
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Keep the top bar in view.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Top bar');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Top bar');
  // Project files stands where the workspace panel button was (COO rebuild
  // L4; overnight 11-04 U4 before it).
  const toggle = projectFilesButton(page);
  await openProjectFiles(page);
  const pane = previewPane(page);

  await pane.focus();
  await tryToScrollTheColumn(page);
  await expectTopBarInView(page);
  // The Preview pane's tabs sit below the bar, not under the window edge.
  const box = await pane.getByRole('tablist', { name: 'Preview tabs' }).boundingBox();
  expect(box!.y).toBeGreaterThanOrEqual(44);
  await expect(toggle).toBeInViewport();

  // The same button hides the pane again.
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-pressed', 'false');
  await expect(pane).toHaveCount(0);
  await page.getByLabel('Chat message').focus();
  await tryToScrollTheColumn(page);
  await expectTopBarInView(page);
});
