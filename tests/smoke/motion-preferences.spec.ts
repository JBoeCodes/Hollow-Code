import { expect, test } from './fixtures/test';

test('Library outline respects system-only motion reduction', async ({ page }, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'the Outline is hidden below 820 px of reader width, and at 390 px the reader is 50 px wide beside the 280 px file list');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/?mock=1');
  await page.getByRole('button', { name: 'Library', exact: true }).click();
  await page.getByRole('button', { name: 'New note' }).click();
  await page.getByLabel('Note name').fill('Motion outline');
  await page.getByRole('button', { name: 'Create note' }).click();
  const reader = page.getByRole('region', { name: 'File reader' });
  await reader.getByRole('button', { name: 'Edit', exact: true }).click();
  await reader.locator('.cm-content[contenteditable=true]').fill('# First\n\nNotes\n\n## Second\n\nMore\n\n## Third\n\nEnd');
  await reader.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(reader.getByRole('button', { name: 'Save', exact: true })).toHaveCount(0);
  await reader.getByRole('button', { name: 'Preview', exact: true }).click();
  await page.evaluate(() => {
    document.documentElement.dataset.motion = 'full';
    const original = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = function(options) {
      if (typeof options === 'object') this.setAttribute('data-scroll-behavior', options.behavior ?? 'auto');
      original.call(this, options);
    };
  });
  await reader.getByRole('navigation', { name: 'Outline', exact: true }).getByRole('button', { name: 'Third' }).click();
  await expect(reader.getByRole('heading', { name: 'Third', exact: true })).toHaveAttribute('data-scroll-behavior', 'auto');
});

test('Settings search respects system-only motion reduction', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/?mock=1');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.evaluate(() => {
    document.documentElement.dataset.motion = 'full';
    const original = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = function(options) {
      if (typeof options === 'object') this.setAttribute('data-scroll-behavior', options.behavior ?? 'auto');
      original.call(this, options);
    };
  });
  await page.getByRole('searchbox', { name: 'Find a setting' }).fill('reduce motion');
  await page.getByRole('searchbox', { name: 'Find a setting' }).press('Enter');
  const row = page.locator('#settings-general .settings-found');
  await expect(row).toContainText('Motion');
  await expect(row).toHaveAttribute('data-scroll-behavior', 'auto');
});

test('away state keeps spinners running and boot content opaque', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).waitFor();
  const result = await page.evaluate(() => {
    document.documentElement.dataset.motion = 'full';
    document.body.classList.add('is-away');
    const spinner = document.createElement('span');
    spinner.className = 'spinner-ring'; document.body.append(spinner);
    const playState = getComputedStyle(spinner).animationPlayState;
    spinner.remove();
    document.documentElement.classList.add('is-booting');
    const child = document.createElement('div');
    document.querySelector('.canvas')!.append(child);
    const boot = child.getAnimations().find(a => (a as CSSAnimation).animationName === 'boot-canvas');
    const frames = (boot?.effect as KeyframeEffect | null)?.getKeyframes() ?? [];
    const ambientBoot = [...document.querySelectorAll('.ambient-layer')].some(el =>
      el.getAnimations().some(a => (a as CSSAnimation).animationName === 'boot-canvas'));
    child.remove(); document.documentElement.classList.remove('is-booting');
    document.body.classList.remove('is-away');
    return { playState, hasBoot: !!boot, hasOpacity: frames.some(f => 'opacity' in f), ambientBoot };
  });
  expect(result).toEqual({ playState: 'running', hasBoot: true, hasOpacity: false, ambientBoot: false });
});
