import { test, expect, type Page } from './fixtures/test';
import { chooseNew } from './choose';

// A new conversation's pane (overnight 2026-11-07, D1, audit gap 7): the
// greeting, the message box and the starters sit together mid-pane, the box
// under the greeting and the starters under the box, and the box keeps the
// pane's full width (Alex asked twice that chat fill its pane). The first
// message puts the box back at the bottom. Fixture IPC only; no agent runs.
async function openNewProject(page: Page, size: { width: number; height: number }) {
  await page.setViewportSize(size);
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Plan the launch');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Launch plan');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.getByRole('heading', { name: /^What should we build in .+\?$/ })).toBeVisible();
}

const pane = (page: Page) => page.locator('.work-conversation');
const greeting = (page: Page) => page.getByRole('heading', { name: /^What should we build in .+\?$/ });
const form = (page: Page) => pane(page).locator('form.chat-input');
const box = async (l: ReturnType<Page['locator']>) => (await l.boundingBox())!;
// Where the words sit, not the element's box: a full-width heading is centred
// in the pane whatever its text-align, so only the text itself shows it.
const textBox = (l: ReturnType<Page['locator']>) => l.evaluate(el => {
  const range = document.createRange();
  range.selectNodeContents(el);
  const b = range.getBoundingClientRect();
  return { x: b.x, width: b.width };
});

// Every size sets its own window, so the narrow project would run the same
// cases again: the desktop project covers them (a 500px case is below).
test.beforeEach(async ({}, info) => { test.skip(!info.project.name.includes('desktop'), 'Geometry at fixed desktop sizes'); });

test('a new conversation seats the box under the greeting and the starters under the box', async ({ page }) => {
  await openNewProject(page, { width: 1440, height: 900 });
  const starter = pane(page).getByRole('button', { name: /Find the next step/ });
  const [p, h, f, s] = [await box(pane(page)), await box(greeting(page)), await box(form(page)), await box(starter)];
  expect(h.y + h.height, 'greeting above the box').toBeLessThanOrEqual(f.y);
  expect(f.y + f.height, 'starters under the box').toBeLessThanOrEqual(s.y);
  // Mid-pane, not at the bottom: the box's centre sits between 30% and 70% of the pane.
  const centre = f.y + f.height / 2 - p.y;
  expect(centre).toBeGreaterThan(p.height * 0.3);
  expect(centre).toBeLessThan(p.height * 0.7);
  // The greeting's words are centred on the pane (its box always is).
  const words = await textBox(greeting(page));
  expect(words.width, 'the greeting is shorter than the pane').toBeLessThan(p.width - 64);
  expect(Math.abs(words.x + words.width / 2 - (p.x + p.width / 2))).toBeLessThan(2);
  // The box fills the pane: the form spans it and the composer keeps only the 32px gutters.
  const c = await box(form(page).locator('.chat-composer'));
  expect(f.width).toBeGreaterThanOrEqual(p.width - 1);
  expect(Math.abs(c.width - (p.width - 64))).toBeLessThan(2);
  // A starter still fills the box.
  await starter.click();
  await expect(form(page).getByRole('textbox')).not.toHaveValue('');
});

test('the first message puts the box back at the bottom of the pane', async ({ page }) => {
  await openNewProject(page, { width: 1440, height: 900 });
  await page.evaluate(async () => {
    const path = '/src/store/chatRuntime.ts';
    (await import(/* @vite-ignore */ path)).setChatTurns([{ role: 'assistant', text: 'Here is the plan.', tools: [] }]);
  });
  await expect(greeting(page)).toHaveCount(0);
  const [p, f] = [await box(pane(page)), await box(form(page))];
  expect(p.y + p.height - (f.y + f.height), 'box flush with the pane bottom').toBeLessThan(4);
});

test('a short window keeps the box at the bottom, in view', async ({ page }) => {
  await openNewProject(page, { width: 1200, height: 500 });
  const [p, f] = [await box(pane(page)), await box(form(page))];
  expect(p.y + p.height - (f.y + f.height), 'box flush with the pane bottom').toBeLessThan(4);
});

// Measured 521 to 900px high and 500 to 1440 wide (the review's "below the
// first screen" case): the whole group stays in view with no scrolling, so the
// box is never below the fold.
for (const [width, height] of [[1440, 900], [1440, 760], [1440, 640], [1440, 521], [1000, 700], [500, 844]] as const) {
  test(`the whole group fits the pane at ${width} by ${height}`, async ({ page }) => {
    await openNewProject(page, { width, height });
    const p = await box(pane(page));
    expect(await pane(page).evaluate(el => el.scrollHeight - el.clientHeight), 'the pane has nothing to scroll').toBeLessThanOrEqual(1);
    const [f, last] = [await box(form(page)), await box(pane(page).locator('.chat-empty-suggestions > *').last())];
    expect(f.y + f.height, 'box above the pane bottom').toBeLessThanOrEqual(p.y + p.height);
    expect(last.y + last.height, 'starters above the pane bottom').toBeLessThanOrEqual(p.y + p.height);
    expect(f.y, 'box below the greeting').toBeGreaterThan((await box(greeting(page))).y);
  });
}

// Composer menus stay within the visible pane at short heights.
for (const height of [900, 760, 640, 521]) {
  test(`what opens above the box stays inside the window at ${height}px high`, async ({ page }) => {
    await openNewProject(page, { width: 1440, height });
    const p = await box(pane(page));
    const inside = async (name: string, menu: ReturnType<Page['locator']>, top: number) => {
      await expect(menu, name).toBeVisible();
      // Measured once it has stopped moving: an entrance animation or content
      // that arrives late changes its box.
      let b = await box(menu);
      for (let i = 0; i < 15; i++) {
        await page.waitForTimeout(80);
        const next = await box(menu);
        const still = next.y === b.y && next.height === b.height;
        b = next;
        if (still) break;
      }
      expect(b.y, `${name}: its top is below the edge`).toBeGreaterThanOrEqual(top - 1);
    };
    await page.getByRole('button', { name: 'Attach and commands' }).click();
    await inside('the + menu', page.locator('.composer-commands .studio-action-menu-body'), p.y);
    await page.keyboard.press('Escape');
    const message = form(page).getByRole('textbox');
    await message.fill('/');
    await inside('the slash list', page.locator('.slash-popup'), p.y);
    await message.fill('');
    await page.locator('.usage-menu-trigger').click();
    await inside('the usage menu', page.locator('.usage-menu-pop'), p.y);
    await page.keyboard.press('Escape');
    await page.getByRole('button', { name: /^Folder and connectors:/ }).click();
    await inside('folder and connectors', page.locator('.composer-resources .studio-action-menu-body'), 0);
  });
}
