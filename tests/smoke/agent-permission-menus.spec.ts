import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

/**
 * Per-agent permission menus (2026-11-12): a chat's access chip and its model
 * menu's Permissions row list the chat's agent's own modes, by their own
 * names (`src/agentPermModes.ts`), with Hollow's own cards in one last line.
 */

async function openChat(page: Page) {
  await page.goto('/?mock=1');
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), 'advanced');
  } else {
    await page.locator('.settings-index').getByRole('button', { name: 'Advanced', exact: true }).click();
  }
  await page.locator('#settings-group-this-mac > summary').click();
  await page.getByRole('button', { name: 'Open Library, History', exact: true }).click();
  await page.locator('.library-tabs').getByRole('button', { name: 'Conversations', exact: true }).click();
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toBeVisible();
}

async function useAgent(page: Page, agent: string, mode: string) {
  await page.evaluate(async ([moduleUrl, a, m]) => {
    const runtime = await import(moduleUrl);
    await runtime.refreshAgentSettings();
    runtime.setChatAgent(a);
    runtime.setChatPermissionMode(m);
  }, ['/src/store/runtime.ts', agent, mode] as const);
}

const MENUS: [agent: string, mode: string, chip: string, items: string[], options: string[], last: RegExp][] = [
  ['claude', 'acceptEdits', 'Access: Accept edits',
    ['Default', 'Accept edits · current', 'Plan', 'Auto', 'Bypass permissions'],
    ['Default', 'Accept edits', 'Plan', 'Auto', 'Bypass permissions'], /^Hollow still asks on a card .*; Bypass permissions skips/],
  ['codex', 'default', 'Access: Ask for approval',
    ['Read only', 'Ask for approval · current', 'Approve for me', 'Full access'],
    ['Read only', 'Ask for approval', 'Approve for me', 'Full access'], /^Hollow still asks on a card .*; Full access skips/],
  ['grok', 'plan', 'Access: Plan',
    ['Default', 'Accept edits', 'Plan · current', 'Auto', 'Bypass permissions'],
    ['Default', 'Accept edits', 'Plan', 'Auto', 'Bypass permissions'], /^Hollow’s own cards can’t reach Grok/],
  ['antigravity', 'bypassPermissions', 'Access: Skip permissions (unrestricted)',
    ['Accept edits', 'Plan', 'Auto', 'Skip permissions · current'],
    ['Accept edits', 'Plan', 'Auto', 'Skip permissions'], /^Hollow’s own cards can’t reach Antigravity/],
];

for (const [agent, mode, chip, items, options, last] of MENUS) {
  test(`a ${agent} chat lists ${agent}'s own permission modes by name`, async ({ page }) => {
    await openChat(page);
    await useAgent(page, agent, mode);
    const access = page.getByRole('button', { name: chip, exact: true });
    await access.click();
    const rows = page.locator('.composer-access [role=menuitem]');
    const names = (await rows.allTextContents()).map(t => t.trim());
    // Each mode row, then the current mode's note, then the shared line.
    expect(names.slice(0, items.length).map(n => n.replace(/: .*$/, ''))).toEqual(items);
    expect(names).toHaveLength(items.length + 2);
    expect(names.at(-1)).toMatch(last);
    await page.keyboard.press('Escape');
    await page.getByRole('button', { name: /^Model:/ }).click();
    await page.getByRole('button', { name: /^Permissions: .*\. Change$/ }).click();
    const list = page.getByRole('listbox', { name: 'Permissions' });
    await expect(list.getByRole('option')).toHaveText(options);
  });
}

test('switching a chat to another agent moves its mode to the nearest one there and says so', async ({ page }) => {
  await openChat(page);
  await useAgent(page, 'claude', 'acceptEdits');
  await expect(page.getByRole('button', { name: 'Access: Accept edits', exact: true })).toBeVisible();
  await page.getByRole('button', { name: /^Model:/ }).click();
  await page.getByRole('option', { name: /GPT-6-Sol/ }).first().click();
  await expect(page.getByRole('button', { name: 'Access: Ask for approval', exact: true })).toBeVisible();
  await expect(page.getByText('Permissions: Accept edits on Claude is Ask for approval on Codex.').first()).toBeVisible();
  expect(await page.evaluate(async moduleUrl => (await import(moduleUrl)).chatPermissionMode(), '/src/store/runtime.ts')).toBe('default');
});

test('Settings sets each agent’s own mode for new chats', async ({ page }) => {
  await openChat(page);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  // Below the sidebar breakpoint the section index is a "Settings section" selector.
  const sectionIndex = page.locator('.settings-index').getByRole('button', { name: 'Chat', exact: true });
  const sectionSelect = page.getByRole('combobox', { name: 'Settings section', exact: true });
  await expect(sectionIndex.or(sectionSelect)).toBeVisible();
  if (await sectionIndex.isVisible()) await sectionIndex.click();
  else await choose(sectionSelect, { label: 'Chat' });
  // Nothing saved in this fresh window: Claude and Codex start in Auto.
  await expect(page.getByRole('combobox', { name: 'Permission mode for new Claude chats', exact: true })).toContainText('Auto');
  await expect(page.getByRole('combobox', { name: 'Permission mode for new Codex chats', exact: true })).toContainText('Approve for me');
  await expect(page.getByRole('combobox', { name: 'Permission mode for new Grok chats', exact: true })).toContainText('Accept edits');
  await choose(page.getByRole('combobox', { name: 'Permission mode for new Codex chats', exact: true }), 'plan');
  await expect(page.getByRole('status').filter({ hasText: 'New Codex chats start in Read only.' })).toBeVisible();
  expect(await page.evaluate(() => localStorage.getItem('newCodexPermissionMode'))).toBe('plan');
});
