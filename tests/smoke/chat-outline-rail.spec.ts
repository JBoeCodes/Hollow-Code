import { showWorkspaceList } from './choose';
import { expect, test, type Page } from './fixtures/test';

/**
 * The chat outline rail (overnight 11-12, E9), on the mock's 1,000-turn
 * thread (500 prompts): one dash per prompt in the chat's left gutter,
 * sampled to 40, the one in view marked; hover or focus opens the Outline
 * card, a row or a dash jumps there, Escape hands focus back to the strip.
 * The Outline button opens the same card.
 */

async function openLongThread(page: Page) {
  await page.goto('/?mock=1&demo=large');
  await showWorkspaceList(page);
  await page.locator('.work-card').filter({ hasText: 'Results view for workspaces' }).click();
  await expect(page.locator('.turn').first()).toBeVisible();
}

const strip = (page: Page) => page.getByRole('navigation', { name: 'Conversation outline' });
const card = (page: Page) => page.getByRole('region', { name: 'Outline', exact: true });

test.beforeEach(({}, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'the strip needs the desktop gutter; narrow keeps the Outline button');
});

test('the strip marks the prompt in view and jumps from the card', async ({ page }) => {
  await openLongThread(page);
  const dashes = strip(page).getByRole('button');
  await expect(dashes).toHaveCount(40);
  // At the end of the thread the last prompt is the one in view.
  await expect(dashes.last()).toHaveAttribute('aria-current', 'location');
  await expect(dashes.last()).toHaveAccessibleName('Step 499: review the next part of the layout.');
  // The strip sits in the gutter, left of the transcript's text.
  const stripBox = (await strip(page).boundingBox())!;
  const turnBox = (await page.locator('.turn.user').last().boundingBox())!;
  expect(stripBox.x + stripBox.width).toBeLessThanOrEqual(turnBox.x + 1);

  await strip(page).hover();
  await expect(card(page)).toBeVisible();
  await expect(card(page).getByRole('listitem')).toHaveCount(500);
  await expect(card(page).locator('[aria-current="location"]')).toContainText('Step 499');
  await card(page).getByRole('button', { name: /Step 2: review/ }).click();
  await expect(page.locator('.turn[data-turn-index="4"]')).toBeInViewport();
  // The current prompt follows the scroll: the first dash now.
  await expect(dashes.first()).toHaveAttribute('aria-current', 'location');
});

test('keyboard: focus opens the card, Escape returns to the strip, a dash jumps', async ({ page }) => {
  await openLongThread(page);
  const dashes = strip(page).getByRole('button');
  await dashes.last().focus();
  await expect(card(page)).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(card(page)).toHaveCount(0);
  await expect(dashes.last()).toBeFocused();
  // One tab stop, which moves with focus (#1322 re-review); Home moves
  // along the strip, Enter jumps.
  await page.keyboard.press('Home');
  await expect(dashes.first()).toBeFocused();
  await expect(dashes.first()).toHaveAttribute('tabindex', '0');
  await expect(strip(page).locator('button[tabindex="0"]')).toHaveCount(1);
  await page.keyboard.press('Enter');
  await expect(page.locator('.turn[data-turn-index="0"]')).toBeInViewport();
  await expect(dashes.first()).toHaveAttribute('aria-current', 'location');
  // End moves the stop too, so Shift+Tab leaves the strip rather than
  // landing on the current prompt's dash.
  await page.keyboard.press('End');
  await expect(dashes.last()).toBeFocused();
  await expect(dashes.last()).toHaveAttribute('tabindex', '0');
  await expect(dashes.first()).toHaveAttribute('tabindex', '-1');
  await page.keyboard.press('Shift+Tab');
  await expect(strip(page).locator(':focus')).toHaveCount(0);
  // Out of the strip, the stop is the current prompt's dash again.
  await expect(dashes.first()).toHaveAttribute('tabindex', '0');
});

test('the Outline button opens the same card and Escape gives focus back', async ({ page }) => {
  await openLongThread(page);
  const button = page.getByRole('button', { name: 'Actions for this conversation', exact: true });
  await button.click();
  await page.getByRole('menuitem', { name: 'Outline', exact: true }).click();
  await expect(card(page)).toBeVisible();
  await expect(page.getByRole('region', { name: 'Outline', exact: true })).toHaveCount(1);
  await card(page).getByRole('button', { name: /Step 1: review/ }).focus();
  await page.keyboard.press('Escape');
  await expect(card(page)).toHaveCount(0);
  await expect(button).toBeFocused();
  await expect(button).toHaveAttribute('aria-haspopup', 'menu');
});

test('the current prompt follows content growing with no scroll event', async ({ page }) => {
  await openLongThread(page);
  await page.getByRole('button', { name: 'Actions for this conversation', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Outline', exact: true }).click();
  await expect(card(page)).toBeVisible();
  const current = card(page).locator('[aria-current="location"]');
  // The thread opens at its end; jump to the middle so its turns are rendered.
  await card(page).getByRole('button', { name: /Step 20: review/ }).click();
  await expect(page.locator('.turn[data-turn-index="40"]')).toBeInViewport();
  if (!(await card(page).isVisible())) {
    await page.getByRole('button', { name: 'Actions for this conversation', exact: true }).click();
    await page.getByRole('menuitem', { name: 'Outline', exact: true }).click();
  }
  await expect(card(page)).toBeVisible();
  // Park the view mid-thread with turn 40 at the top, and stop the browser's
  // own scroll anchoring from writing scrollTop (that would be a scroll event).
  const setup = await page.evaluate(() => {
    const s = document.querySelector<HTMLElement>('.chat-scroll')!;
    s.style.overflowAnchor = 'none';
    const target = s.querySelector<HTMLElement>('.turn.user[data-turn-index="40"]')!;
    s.scrollTop += target.getBoundingClientRect().top - s.getBoundingClientRect().top;
    return { line: s.clientHeight / 3 };
  });
  // The reading line: the last prompt at or above a third of the way down.
  const expected = () => page.evaluate(({ line }) => {
    const s = document.querySelector<HTMLElement>('.chat-scroll')!;
    const box = s.getBoundingClientRect();
    const users = [...s.querySelectorAll<HTMLElement>('.turn.user[data-turn-index]')];
    const above = users.filter(u => u.getBoundingClientRect().top - box.top <= line);
    return (above[above.length - 1].textContent ?? '').match(/Step \d+/)![0];
  }, { line: setup.line });
  const before = await expected();
  await expect.poll(async () => (await current.textContent()) ?? '').toContain(before);
  // Grow the reply just before the current prompt: it moves below the line
  // with no scroll event, so only the size observers can notice.
  const after = await page.evaluate(() => {
    const s = document.querySelector<HTMLElement>('.chat-scroll')!;
    const box = s.getBoundingClientRect();
    const users = [...s.querySelectorAll<HTMLElement>('.turn.user[data-turn-index]')];
    const line = s.clientHeight / 3;
    const above = users.filter(u => u.getBoundingClientRect().top - box.top <= line);
    const last = above[above.length - 1];
    const reply = last.previousElementSibling as HTMLElement;
    const filler = document.createElement('div');
    filler.style.height = `${s.clientHeight * 2}px`;
    reply.appendChild(filler);
    const prev = above[above.length - 2];
    return (prev.textContent ?? '').match(/Step \d+/)![0];
  });
  expect(after).not.toBe(before);
  await expect.poll(async () => (await current.textContent()) ?? '').toContain(after);
});
