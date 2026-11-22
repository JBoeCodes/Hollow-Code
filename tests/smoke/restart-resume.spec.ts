import { expect, test, type Page } from './fixtures/test';
import { openWaiting } from './waiting';

/**
 * Restart resume, picked per chat (P1-40). The `interrupted` scenario offers
 * one chat; this spec replaces the offer with three of the person's chats
 * and one an agent started (the mock reads `mockInterruptedChats` on each
 * list), then resumes two of the three.
 */

test.beforeEach(({}, info) => { test.skip(info.project.name !== 'chromium-desktop', 'one dialog; the journeys cover the small sizes'); });

const REPORT = 'local-q3-review', BUILD = 'work-results-view';
const asked = new Date(Date.now() - 30 * 60_000).toISOString();
const offer = [
  { sessionKey: 's-20260930-q3a00001', title: 'Revise the labour section', agent: 'claude', workspaceId: REPORT, askedAt: asked, model: 'claude-opus-5-5', permissionMode: 'acceptEdits', agentStarted: false },
  { sessionKey: 's-20260928-q3a00002', title: 'Check the September close', agent: 'codex', workspaceId: REPORT, askedAt: asked, model: null, permissionMode: 'plan', agentStarted: false },
  { sessionKey: 's-20261001-bld00001', title: 'Wire the Results view', agent: 'claude', workspaceId: BUILD, askedAt: asked, model: null, permissionMode: null, agentStarted: false },
  { sessionKey: 's-20260930-bld00002', title: 'Helper: run the smoke suite', agent: 'claude', workspaceId: BUILD, askedAt: asked, model: null, permissionMode: 'auto', agentStarted: true },
];

async function openOffer(page: Page) {
  const panel = await openWaiting(page);
  await panel.getByRole('button', { name: /Resume 1 interrupted chat/ }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toBeVisible();
  return dialog;
}

test('the resume card ticks each chat, says what it runs with, and resumes only the ticked ones', async ({ page }) => {
  await page.goto('/?mock=1&demo=1&scenario=interrupted');
  await expect(page.locator('.home-project-row').first()).toBeVisible();
  await page.evaluate(o => { localStorage.setItem('mockInterruptedChats', JSON.stringify(o)); localStorage.removeItem('mockResumedChats'); }, offer);

  const dialog = await openOffer(page);
  await expect(dialog.getByRole('heading', { name: 'Resume 3 interrupted chats?' })).toBeVisible();
  // Cancel holds focus: each resume is a billed message.
  await expect(dialog.getByRole('button', { name: 'Not now' })).toBeFocused();
  const ticks = dialog.getByRole('checkbox');
  await expect(ticks).toHaveCount(3);
  for (let i = 0; i < 3; i++) await expect(ticks.nth(i)).toBeChecked();
  // Each row names the agent, model and permission mode its resume runs with.
  await expect(dialog.locator('.confirm-check').filter({ hasText: 'Revise the labour section' })).toContainText('Claude · claude-opus-5-5 · Accept edits');
  await expect(dialog.locator('.confirm-check').filter({ hasText: 'Check the September close' })).toContainText('Codex · default model · Read only');
  // The agent-started chat is listed apart: Open, nothing to tick.
  await expect(dialog.getByRole('heading', { name: '1 agent-started chat also stopped' })).toBeVisible();
  const helper = dialog.locator('.confirm-aside .confirm-check').filter({ hasText: 'Helper: run the smoke suite' });
  await expect(helper.getByRole('button', { name: 'Open Helper: run the smoke suite' })).toBeVisible();
  await expect(helper.getByRole('checkbox')).toHaveCount(0);

  // Untick the third; Resume ticked sends two.
  await dialog.locator('.confirm-check').filter({ hasText: 'Wire the Results view' }).getByRole('checkbox').uncheck();
  await dialog.getByRole('button', { name: 'Resume ticked' }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByText('Resumed 2 chats. 1 more stays in Needs you to resume later.').first()).toBeVisible();
  const resumed = await page.evaluate(() => JSON.parse(localStorage.getItem('mockResumedChats') ?? '[]') as string[]);
  expect(resumed).toEqual(['s-20260930-q3a00001', 's-20260928-q3a00002']);

  // The unticked chat stays offered.
  const again = await openOffer(page);
  await expect(again.getByRole('heading', { name: 'Resume 1 interrupted chat?' })).toBeVisible();
  await expect(again.getByRole('checkbox')).toHaveCount(1);
  await expect(again.locator('.confirm-check').first()).toContainText('Wire the Results view');
  await again.getByRole('button', { name: 'Not now' }).click();
  await expect(again).toHaveCount(0);
});

test('an agent-started chat opens from the card and is never resumed from it', async ({ page }) => {
  await page.goto('/?mock=1&demo=1&scenario=interrupted');
  await expect(page.locator('.home-project-row').first()).toBeVisible();
  await page.evaluate(o => { localStorage.setItem('mockInterruptedChats', JSON.stringify(o)); localStorage.removeItem('mockResumedChats'); }, offer);
  const dialog = await openOffer(page);
  // Keyboard: Tab from Cancel reaches the aside's Open, and Enter opens it (not the confirm).
  const open = dialog.getByRole('button', { name: 'Open Helper: run the smoke suite' });
  for (let i = 0; i < 8 && !(await open.evaluate(el => el === document.activeElement)); i++) await page.keyboard.press('Tab');
  await expect(open).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(dialog).toHaveCount(0);
  await expect(page.locator('.top-bar-crumbs')).toContainText('Results view for workspaces');
  const sent = await page.evaluate(() => (window as unknown as { __mockApiCalls?: Record<string, number> }).__mockApiCalls?.resume_interrupted_chats ?? 0);
  expect(sent, 'Open resumes nothing').toBe(0);
});
