import { openTasks } from './choose';
import { expect, test, type Page } from './fixtures/test';
import { join } from 'node:path';
import AxeBuilder from '@axe-core/playwright';
async function study(page: Page, theme = 'light') {
  await page.addInitScript(t => { localStorage.setItem('theme', t === 'light' ? 'light' : 'dark'); localStorage.setItem('glass', t === 'glass' ? 'on' : 'off'); }, theme);
  await page.goto('/?mock=1');
  await page.evaluate(async () => {
    const path = '/tests/smoke/fixtures/main-orb.tsx';
    (window as any).disposeOrbStudy = (await import(/* @vite-ignore */ path)).mount();
  });
  return page.locator('.main-orb-study');
}
for (const theme of ['light', 'dark', 'glass']) test(`COO SVG states in ${theme}`, async ({ page }, info) => {
  const host = await study(page, theme);
  await expect(host.getByRole('img')).toHaveCount(21);
  await expect(host.locator('[data-state="waiting"] .main-orb-count')).toHaveText(['3', '3', '3']);
  await expect(host.locator('[data-state="night"] .main-orb-moon')).toHaveCount(3);
  await expect(host.locator('[data-state="failed"] .main-orb-error')).toHaveCount(3);
  expect((await new AxeBuilder({ page }).include('.main-orb-study').withTags(['wcag2a', 'wcag2aa']).analyze()).violations).toEqual([]);
  // Capture a complete study, including the native small sizes.
  if (process.env.HOLLOW_LANE_SHOTS) {
    await host.evaluate(el => { (el as HTMLElement).style.position = 'absolute'; (el as HTMLElement).style.height = 'max-content'; });
    await host.screenshot({ path: join(process.env.HOLLOW_LANE_SHOTS, `v3-${theme}-${info.project.name}.png`) });
  }
});
test('orb animations pause offscreen and when hidden, both motion gates settle, unmount releases', async ({ page }) => {
  const host = await study(page);
  const working = host.locator('[data-state="working"]').last();
  const arc = working.locator('.main-orb-orbit');
  await working.scrollIntoViewIfNeeded();
  await expect(arc).toHaveCSS('animation-play-state', 'running');
  await working.evaluate(el => { (el as HTMLElement).style.transform = 'translateY(5000px)'; });
  await expect(working).toHaveAttribute('data-visible', 'false');
  await expect(arc).toHaveCSS('animation-play-state', 'paused');
  await working.evaluate(el => { (el as HTMLElement).style.transform = ''; });
  await working.scrollIntoViewIfNeeded();
  await expect(arc).toHaveCSS('animation-play-state', 'running');
  await page.evaluate(() => { Object.defineProperty(document, 'hidden', { configurable: true, value: true }); document.dispatchEvent(new Event('visibilitychange')); });
  await expect(arc).toHaveCSS('animation-play-state', 'paused');
  await page.evaluate(() => { Object.defineProperty(document, 'hidden', { configurable: true, value: false }); document.dispatchEvent(new Event('visibilitychange')); document.body.classList.add('is-away'); });
  await expect(arc).toHaveCSS('animation-play-state', 'paused');
  await page.evaluate(() => { document.body.classList.remove('is-away'); document.documentElement.dataset.motion = 'reduced'; });
  await expect(arc).toHaveCSS('animation-name', 'none');
  await page.evaluate(() => { delete document.documentElement.dataset.motion; });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await expect(arc).toHaveCSS('animation-name', 'none');
  // The stock Fold artwork replaced the geometric fallback; reduced motion
  // keeps the image and state ring visible while stopping the orbit.
  await expect(working.locator('.main-icon-image')).toBeVisible();
  await expect(working.locator('.main-orb-shell')).toBeVisible();
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await expect(arc).toHaveCSS('animation-name', 'main-orb-swirl');
  await working.evaluate(el => { (window as any).savedOrb = el; });
  await page.evaluate(() => (window as any).disposeOrbStudy());
  expect(await page.evaluate(() => (window as any).savedOrb.getAnimations({ subtree: true }).length)).toBe(0);
});

test('the orb opens the COO in one click', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockWorkspaces', JSON.stringify([{ id: 'main', kind: 'main', title: 'COO', status: 'active', path: 'workspaces/main', cwd: '/Users/fixture/vault/workspaces/main', modified: '2026-10-12T04:00:00Z' }])));
  await page.goto('/?mock=1');
  const orb = page.locator('.top-bar').getByRole('button', { name: /^(COO|Back from the COO) ·/ });
  // No Ask ⌄ beside it any more (2026-10-21).
  await expect(page.getByRole('group', { name: 'Everywhere' }).getByRole('button', { name: /^Ask/ })).toHaveCount(0);
  // One click: the Main agent's page, no panel in between (2026-10-14).
  await orb.click();
  await expect(page.locator('.work-conversation')).toBeVisible();
  await expect(orb).toHaveAttribute('aria-pressed', 'true');
  await expect(orb).toHaveAccessibleName(/^Back from the COO ·/);
  // Pressed again: back to where you were.
  await orb.click();
  await expect(orb).toHaveAttribute('aria-pressed', 'false');
  await expect(page.locator('.work-conversation')).toBeHidden();
  // Keyboard: Enter on the orb opens it.
  await orb.focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('.work-conversation')).toBeVisible();
  // ⌘⇧M (the old drawer's key, run 15 P3) opens the page too, from any page;
  // there is no drawer and no Open beside your page any more.
  await openTasks(page);
  await expect(page.getByRole('heading', { name: 'Tasks', level: 1 })).toBeVisible();
  await expect(page.locator('.work-conversation')).toBeHidden();
  await page.keyboard.press('ControlOrMeta+Shift+M');
  await expect(page.locator('.work-conversation')).toBeVisible();
  await expect(page.locator('.work-conversation').getByRole('button', { name: /^Open beside/ })).toHaveCount(0);
  await expect(page.locator('.main-drawer')).toHaveCount(0);
});

test('check-ins off: the COO page has a way to the switch, highlighted in Settings → Background work (A07)', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('mockWorkspaces', JSON.stringify([{ id: 'main', kind: 'main', title: 'COO', status: 'active', path: 'workspaces/main', cwd: '/Users/fixture/vault/workspaces/main', modified: '2026-10-12T04:00:00Z' }]));
    localStorage.setItem('mockMainAgentStatus', JSON.stringify({ spentTodayUsd: 0, askedTodayUsd: 0, wakeSkipped: null, dailyCapUsd: 100, checkIns: false, lastCheckIn: null, nextCheckIn: null, checkInsToday: 0, suggestions: { answered: 0, taken: 0, declinedForGood: 0, takenShare: null }, actOnOwn: true }));
    localStorage.setItem('mockMainAgentSettings', JSON.stringify({ checkIns: false, intervalMin: 30, startHour: 8, endHour: 18, weekdaysOnly: true, dailyCapUsd: 100, actOnOwn: true, swarmMax: 25, swarmModel: 'claude-haiku-5-5', swarmConcurrency: 6 }));
  });
  await page.goto('/?mock=1');
  await page.locator('.top-bar').getByRole('button', { name: /^(COO|Back from the COO) ·/ }).click();
  // The way to the setting is a click in the top bar's Actions for COO, not a path.
  await expect(page.getByText('Settings → COO')).toHaveCount(0);
  // The menu is built when it opens: wait until the store has read the day, or it offers the plain label.
  await expect.poll(() => page.evaluate(async () => {
    const p = '/src/store/mainAgent.ts';
    return (await import(/* @vite-ignore */ p)).mainAgentStatus()?.checkIns;
  })).toBe(false);
  await page.locator('.top-bar').getByRole('button', { name: 'Actions for COO', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Turn check-ins on in Settings', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Background work', exact: true, level: 2 })).toBeVisible();
  // The same mark a Settings search hit leaves, on the check-ins row.
  await expect(page.locator('.settings-found').filter({ has: page.getByRole('switch', { name: 'COO check-ins' }) })).toBeVisible();
  await expect(page.getByRole('switch', { name: 'COO check-ins' })).toHaveAttribute('aria-checked', 'false');
});
