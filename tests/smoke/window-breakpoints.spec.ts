import { openTasks, openProjectFiles, closeProjectFiles } from './choose';
import { expect, test } from './fixtures/test';
import AxeBuilder from '@axe-core/playwright';
import { chooseNew } from './choose';

test('medium Automations back returns to its list', async ({ page }) => {
  await page.setViewportSize({ width: 1000, height: 700 });
  await page.addInitScript(() => localStorage.setItem('hollow.studio.textScale', '100'));
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Automations', exact: true }).click();
  await page.locator('.workflow-row').filter({ hasText: 'Morning Brief' }).click();
  await page.getByRole('button', { name: '← All automations', exact: true }).click();
  await expect(page.locator('.workflow-viewer')).toBeHidden();
  await expect(page.locator('.workflow-row').filter({ hasText: 'Morning Brief' })).toBeVisible();
});

test('Largest navigation dismisses the overlay rail at 1000px', async ({ page }) => {
  await page.setViewportSize({ width: 1000, height: 700 });
  await page.addInitScript(() => localStorage.setItem('hollow.studio.textScale', '125'));
  await page.goto('/?mock=1&demo=1');
  await expect(page.locator('.work-rail')).toHaveClass(/collapsed/);
  await page.getByRole('button', { name: 'Show sidebar', exact: true }).click();
  await expect(page.locator('.work-rail')).not.toHaveClass(/collapsed/);
  await expect(page.locator('.work-rail')).toHaveCSS('position', 'absolute');
  await openTasks(page);
  await expect(page.locator('.work-rail')).toHaveClass(/collapsed/);
});

test('Largest history selection reveals its conversation at 1000px', async ({ page }) => {
  await page.setViewportSize({ width: 1000, height: 700 });
  await page.addInitScript(() => localStorage.setItem('hollow.studio.textScale', '125'));
  await page.goto('/?mock=1&demo=1');
  await page.evaluate(async () => {
    const path = '/src/store/runtime.ts';
    (await import(/* @vite-ignore */ path)).setActivePage('chat');
  });
  await expect(page.getByRole('heading', { name: 'Conversation history', exact: true })).toBeVisible();
  const show = page.getByRole('button', { name: 'Show conversations', exact: true });
  await expect(show).toBeVisible();
  await show.click();
  await page.locator('.session-row button').first().click();
  await expect(page.locator('.chat-workspace .chat-main')).toBeVisible();
  await expect(show).toBeVisible();
});

test('Largest Preview pane starts closed at 1280px', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.addInitScript(() => localStorage.setItem('hollow.studio.textScale', '125'));
  await page.goto('/?mock=1&demo=1');
  await expect(page.getByRole('complementary', { name: 'Main navigation', exact: true })).toBeVisible();
  await expect.poll(() => page.evaluate(async () => {
    const path = '/src/store/preview.ts';
    return (await import(/* @vite-ignore */ path)).previewTarget();
  })).toBeNull();
  await expect(page.getByRole('complementary', { name: 'Preview', exact: true })).toHaveCount(0);
});

test('Largest side pane focus reveals the composer at 1280px', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.addInitScript(() => localStorage.setItem('hollow.studio.textScale', '125'));
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Check focus after opening a panel.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Focus');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Focus');
  // Creating through the visible form invokes Work's focusConversation route.
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toBeFocused();
  // Project files opens the Preview pane beside the conversation where there
  // is room and over it where there isn't (COO rebuild L4: it was the
  // workspace panel). Hidden again, the same composer takes a chat focus
  // request.
  await openProjectFiles(page);
  await closeProjectFiles(page);
  await page.evaluate(async () => {
    const path = '/src/store/chat.ts';
    const chat = await import(/* @vite-ignore */ path);
    chat.requestChatFocus();
  });
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toBeVisible();
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toBeFocused();
});

test('em media queries ignore app text size but container em queries follow it', async ({ page }) => {
  await page.setViewportSize({ width: 1000, height: 700 });
  await page.goto('/?mock=1&demo=1');
  const result = await page.evaluate(() => {
    const host = document.createElement('div');
    host.style.cssText = 'width:1000px;container:probe / inline-size;font-size:16px';
    host.innerHTML = '<i id="em-probe"></i>';
    const style = document.createElement('style');
    style.textContent = '#em-probe {display:block;width:1px} @container probe (max-width:53.75em) { #em-probe {width:2px} }';
    document.body.append(host, style);
    const before = host.firstElementChild!.getBoundingClientRect().width;
    const mediaBefore = matchMedia('(max-width:53.75em)').matches;
    const originalRootFont = document.documentElement.style.fontSize;
    document.documentElement.style.fontSize = '20px';
    host.style.fontSize = '20px';
    const after = host.firstElementChild!.getBoundingClientRect().width;
    const mediaAfter = matchMedia('(max-width:53.75em)').matches;
    document.documentElement.style.fontSize = originalRootFont;
    host.remove(); style.remove();
    return { before, after, mediaBefore, mediaAfter };
  });
  expect(result).toEqual({ before: 1, after: 2, mediaBefore: false, mediaAfter: false });
});

test('Largest changes the live compact layout and Escape follows the overlay rail', async ({ page }) => {
  await page.setViewportSize({ width: 1000, height: 700 });
  await page.addInitScript(() => localStorage.setItem('hollow.studio.textScale', '100'));
  await page.goto('/?mock=1&demo=1');
  const rail = page.locator('.work-rail');
  const close = page.getByRole('button', { name: 'Close sidebar', exact: true });
  const setScale = async (scale: '100' | '125') => page.evaluate(async value => {
    const path = '/src/store/studio.ts';
    const studio = await import(/* @vite-ignore */ path);
    studio.setStudioTextScale(value);
  }, scale);
  // Normalize the person's saved rail state through its own visible toggle.
  if (await rail.evaluate(el => el.classList.contains('collapsed'))) await page.getByRole('button', { name: 'Show sidebar', exact: true }).click();
  await expect(close).toBeHidden();
  await setScale('125');
  await expect(rail).toHaveClass(/collapsed/);
  await page.getByRole('button', { name: 'Show sidebar', exact: true }).click();
  await expect(close).toBeVisible();
  await expect(rail).toHaveCSS('position', 'absolute');
  await page.keyboard.press('Escape');
  await expect(rail).toHaveClass(/collapsed/);
  await setScale('100');
  // Returning to roomy layout must not override the person's collapsed choice.
  await expect(rail).toHaveClass(/collapsed/);
  await page.getByRole('button', { name: 'Show sidebar', exact: true }).click();
  await expect(close).toBeHidden();
  await page.keyboard.press('Escape');
  await expect(rail).not.toHaveClass(/collapsed/);
});

test('main-window compact layouts are reachable at the native minimum', async ({ page }) => {
  await page.setViewportSize({ width: 720, height: 480 });
  await page.goto('/?mock=1&demo=1');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Tasks', exact: true }).focus();
  await page.keyboard.press('?');
  const grid = page.locator('.keymap-grid');
  await expect(grid).toBeVisible();
  expect(await grid.evaluate(el => getComputedStyle(el).gridTemplateColumns.split(' ').length)).toBe(1);
});

test('medium Tasks has no Agenda week', async ({ page }) => {
  await page.setViewportSize({ width: 1000, height: 700 });
  await page.addInitScript(() => localStorage.setItem('hollow.studio.textScale', '100'));
  await page.goto('/?mock=1&demo=1');
  await openTasks(page);
  await expect(page.getByRole('group', { name: 'Agenda range' })).toHaveCount(0);
});

test('wide Tasks keeps a readable title beside its details pane', async ({ page }) => {
  await page.setViewportSize({ width: 1150, height: 700 });
  await page.emulateMedia({ colorScheme: 'light', reducedMotion: 'reduce' });
  await page.addInitScript(() => {
    localStorage.setItem('theme', 'light');
    localStorage.setItem('hollow.studio.textScale', '100');
  });
  await page.goto('/?mock=1&demo=1');
  const rail = page.locator('.work-rail');
  if (await rail.evaluate(el => el.classList.contains('collapsed'))) await page.getByRole('button', { name: 'Show sidebar', exact: true }).click();
  await openTasks(page);
  const task = page.locator('.issue-row.task-row').filter({ hasText: 'Draft airport pricing options' }).locator('.issue-row-main');
  await task.click();
  await expect(page.getByRole('complementary', { name: 'Task details' })).toBeVisible();
  const title = task.locator('.issue-title');
  await expect(title).toBeVisible();
  expect((await title.boundingBox())!.width, 'title retains room for a readable word before truncation').toBeGreaterThanOrEqual(80);
  await task.scrollIntoViewIfNeeded();
  await expect(title).toBeInViewport();
  expect((await new AxeBuilder({ page }).include('.tasks-workspace').withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze()).violations).toEqual([]);
});

test('root containment cannot turn an offscreen fixed overlay into document scrolling', async ({ page }) => {
  await page.setViewportSize({ width: 720, height: 480 });
  await page.addInitScript(() => localStorage.setItem('hollow.studio.textScale', '125'));
  await page.goto('/?mock=1&demo=1');
  const offset = await page.evaluate(async () => {
    const overlay = document.createElement('div');
    overlay.style.cssText = 'position:fixed;top:150vh;width:1px;height:1px';
    document.body.append(overlay);
    scrollBy(0, 500);
    await new Promise(requestAnimationFrame);
    const y = scrollY;
    overlay.remove();
    scrollTo(0, 0);
    return y;
  });
  expect(offset).toBe(0);
});

