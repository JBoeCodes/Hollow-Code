import { expect, test, type Page } from './fixtures/test';
import AxeBuilder from '@axe-core/playwright';
import { join } from 'node:path';
async function open(page: Page, theme = 'light') {
  await page.addInitScript(t => { localStorage.setItem('theme', t === 'light' ? 'light' : 'dark'); localStorage.setItem('glass', t === 'glass' ? 'on' : 'off'); }, theme);
  await page.goto('/?mock=1&demo=studio&missions=1');
  await page.evaluate(async () => {
    // Day-boundary-safe synthetic finishes; the real view still filters to today.
    const queue = JSON.parse(localStorage.getItem('mockRunQueue') ?? '[]');
    for (const row of queue) if (row.finishedAt) row.finishedAt = new Date().toISOString();
    localStorage.setItem('mockRunQueue', JSON.stringify(queue));
    const path = '/src/store/missionControl.ts'; (await import(/* @vite-ignore */ path)).openMissionControl();
  });
  const dialog = page.getByRole('region', { name: 'Running', exact: true });
  await expect(dialog.locator('.mc-track-segment')).toHaveCount(6);
  return dialog;
}
for (const theme of ['light', 'dark', 'glass']) test(`mission tracks ${theme}: evidence and responsive layout`, async ({ page }, info) => {
  const dialog = await open(page, theme);
  await expect(dialog.locator('.mc-track-cost')).toContainText('$0.84');
  await expect(dialog.locator('.mc-track-cost')).toContainText('1 of 6 with cost · 1 unknown · 4 pending');
  await expect(dialog.locator('.mc-track-segment[data-state="done"]')).toContainText('Finished');
  await expect(dialog.locator('.mc-track-segment[data-mc-id="q41"]')).toContainText('Gating');
  const overflow = await dialog.evaluate(el => el.scrollWidth > el.clientWidth + 1);
  expect(overflow).toBe(false);
  expect((await new AxeBuilder({ page }).include('.mc-card').withTags(['wcag2a', 'wcag2aa']).analyze()).violations).toEqual([]);
  if (process.env.HOLLOW_LANE_SHOTS) {
    await dialog.screenshot({ path: join(process.env.HOLLOW_LANE_SHOTS, `v4-${theme}-${info.project.name}.png`) });
  }
});
test('tracks select finished turns, preserve focus across feed updates, and cycle views', async ({ page }) => {
  const dialog = await open(page);
  const done = dialog.locator('.mc-track-segment[data-state="done"]');
  await done.focus();
  await expect(done).toHaveAttribute('aria-current', 'true');
  await expect(dialog.locator('.mc-detail .mc-turn')).toContainText('$0.84');
  const clock = await dialog.locator('.mc-detail .mc-turn-clock').textContent();
  await page.waitForTimeout(1100);
  await expect(dialog.locator('.mc-detail .mc-turn-clock')).toHaveText(clock!);
  await page.evaluate(async () => { const path = '/src/store/missionControl.ts'; (await import(/* @vite-ignore */ path)).feedChatEvent({ type: 'assistantDelta', conversationKey: 's-20261001-bld00001', text: ' A new line.' }); });
  await expect(done).toBeFocused();
  await page.evaluate(async () => { const path = '/src/store/runQueue.ts'; const store = await import(/* @vite-ignore */ path); store.setRunQueue(store.runQueue().map((r: any) => r.id === 44 ? { ...r, status: 'claimed' } : r)); });
  await expect(done).toBeFocused();
  await page.keyboard.press('ArrowDown');
  await expect(dialog.locator('.mc-track-segment:focus')).toHaveCount(1);
  await page.keyboard.press('v');
  await expect(dialog.locator('.mc-sky')).toBeVisible();
  await page.keyboard.press('v');
  await expect(dialog.locator('.mc-grid')).toBeVisible();
  await page.keyboard.press('v');
  await expect(dialog.locator('.mc-tracks')).toBeVisible();
  await dialog.locator('.mc-track-segment[data-state="done"]').focus();
  await page.keyboard.press('Enter');
  await expect(dialog).toBeHidden();
});
test('track spark pauses offscreen, hidden and away; both motion gates stay static', async ({ page }) => {
  const dialog = await open(page);
  const segment = dialog.locator('.mc-track-segment[data-active="true"]').first();
  const spark = segment.locator('.mc-track-spark');
  await segment.scrollIntoViewIfNeeded();
  await expect(spark).toHaveCSS('animation-play-state', 'running');
  await segment.evaluate(el => { (el as HTMLElement).style.transform = 'translateY(5000px)'; });
  await expect(segment).toHaveAttribute('data-visible', 'false');
  await expect(spark).toHaveCSS('animation-play-state', 'paused');
  await segment.evaluate(el => { (el as HTMLElement).style.transform = ''; });
  await segment.scrollIntoViewIfNeeded();
  await expect(spark).toHaveCSS('animation-play-state', 'running');
  await page.evaluate(() => { Object.defineProperty(document, 'hidden', { configurable: true, value: true }); document.dispatchEvent(new Event('visibilitychange')); });
  await expect(spark).toHaveCSS('animation-play-state', 'paused');
  await page.evaluate(() => { Object.defineProperty(document, 'hidden', { configurable: true, value: false }); document.dispatchEvent(new Event('visibilitychange')); document.body.classList.add('is-away'); });
  await expect(spark).toHaveCSS('animation-play-state', 'paused');
  await page.evaluate(() => { document.body.classList.remove('is-away'); document.documentElement.dataset.motion = 'reduced'; });
  await expect(spark).toHaveCSS('animation-name', 'none');
  await page.evaluate(() => { delete document.documentElement.dataset.motion; });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await expect(spark).toHaveCSS('animation-name', 'none');
  await expect(segment.locator('.mc-track-block')).toBeVisible();
});

test('saved view survives, and tracks retain finished work without live turns', async ({ page }) => {
  const dialog = await open(page);
  await dialog.getByRole('button', { name: 'List', exact: true }).click();
  await expect(dialog.locator('.mc-grid')).toBeVisible();
  await page.reload();
  await page.evaluate(async () => { const path = '/src/store/missionControl.ts'; (await import(/* @vite-ignore */ path)).openMissionControl(); });
  await expect(dialog.locator('.mc-grid')).toBeVisible();
  await dialog.getByRole('button', { name: 'Tracks', exact: true }).click();
  await page.evaluate(async () => {
    const path = '/src/store/runQueue.ts'; const store = await import(/* @vite-ignore */ path);
    store.setRunQueue(store.runQueue().filter((r: any) => ['done', 'failed'].includes(r.status)).map((r: any) => ({ ...r, finishedAt: new Date().toISOString() })));
    const feedPath = '/src/store/missionControl.ts'; const feed = await import(/* @vite-ignore */ feedPath);
    feed.setMissionFeed({});
  });
  await expect(dialog.locator('.mc-track-segment')).toHaveCount(2);
  await expect(dialog.locator('.mc-track-cost')).toContainText('0 of 2 with cost · 2 unknown · 0 pending');
  await dialog.locator('.mc-track-segment[data-state="done"]').focus();
  await expect(dialog.locator('.mc-detail .mc-turn')).toBeVisible();
});
