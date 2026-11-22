import { openWorkspaceResult, resultReviewBar } from './choose';
import { test, expect } from './fixtures/test';

// Consolidation D (2026-10-13): capabilities the palette or a key reached
// that also have a visible control where the need is (docs/ROUTE_LEDGER.md).
// Clicks only.

test('the composer’s Dictate, model and Stop are buttons', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  // An idle conversation, so the reply runs at once rather than queueing.
  await page.locator('.home-project-row').filter({ hasText: 'Q3 regional operations review' }).getByRole('button').first().click();
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await expect(composer).toBeVisible();
  await expect(page.getByRole('button', { name: 'Dictate', exact: true })).toBeVisible();

  await page.getByRole('button', { name: /^Model: / }).click();
  await expect(page.getByRole('group', { name: 'Models', exact: true })).toBeVisible();
  await page.getByRole('button', { name: /^Model: / }).click();
  await expect(page.getByRole('group', { name: 'Models', exact: true })).toHaveCount(0);

  await composer.fill('Summarise the plan');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  // Stop is a visible, enabled button while a turn runs or waits; that it
  // cancels is chat-workspace.spec.ts's "explicit send and stop".
  const stop = page.getByRole('button', { name: 'Stop', exact: true });
  await expect(stop).toBeEnabled();
  await stop.click();
});

test('a result menu exports and does not pressure-test', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await page.getByRole('button', { name: 'Open Q3 regional operations review', exact: true }).click();
  // The Result in the Preview pane under its review bar (COO rebuild L4).
  await openWorkspaceResult(page, 'local-q3-review', 'workspaces/local-q3-review/outputs/q3-regional-review.md');
  const bar = resultReviewBar(page);
  await expect(bar.locator('.result-states')).toBeVisible();
  await bar.locator('summary[aria-label="More result actions"]').click();
  await expect(page.getByRole('menuitem', { name: 'Personas for pressure tests…' })).toHaveCount(0);
  await expect(page.getByRole('menuitem', { name: 'Pressure-test this draft…' })).toHaveCount(0);
  await expect(page.getByRole('menuitem', { name: 'Present' })).toHaveCount(0);
  await expect(page.getByRole('menuitem', { name: 'Export as Markdown…' })).toBeVisible();
  await expect(page.getByRole('menuitem', { name: 'Earlier versions (History & recovery)' })).toBeVisible();
});

test('the composer’s + menu attaches files and lists every command, and runs Claude Code’s own lists read-only', async ({ page }) => {
  // Plan completion L4 (review V02, matrix N2): /mcp and /plugin, and every
  // other local command, by a click; typing "/" stays the accelerator.
  // 2026-11-01: the menu is the composer's +, and Attach files… leads it
  // (C5, 2026-11-04: any file, not only an image).
  await page.goto('/?mock=1&demo=1');
  await page.locator('.home-project-row').filter({ hasText: 'Q3 regional operations review' }).getByRole('button').first().click();
  const conversation = page.getByRole('region', { name: 'Workspace conversation' });
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  // One button for what you add: no separate Attach or Commands button.
  await expect(conversation.getByRole('button', { name: 'Attach image', exact: true })).toHaveCount(0);
  await expect(conversation.getByRole('button', { name: 'Commands', exact: true })).toHaveCount(0);
  await conversation.getByRole('button', { name: 'Attach and commands', exact: true }).click();
  const menu = page.getByRole('menu');
  await expect(menu.getByRole('menuitem').first()).toHaveText('Attach files…');
  // Attach files… opens the composer's own file picker, for any type.
  const picked = page.waitForEvent('filechooser');
  await menu.getByRole('menuitem', { name: 'Attach files…' }).click();
  const chooser = await picked;
  expect(chooser.isMultiple()).toBe(true);
  expect(await chooser.element().getAttribute('accept')).toBeNull();
  await conversation.getByRole('button', { name: 'Attach and commands', exact: true }).click();
  for (const name of ['Claude Code’s MCP servers (its own list)', 'Claude Code’s plugins (its own list)', 'All commands, with Claude Code’s skills…']) {
    await expect(menu.getByRole('menuitem', { name })).toBeVisible();
  }
  for (const cmd of ['/new', '/cwd', '/agent', '/model', '/permissions', '/stop', '/help']) {
    await expect(menu.getByRole('menuitem', { name: new RegExp(`^${cmd.replace('/', '\\/')} · `) })).toBeVisible();
  }
  // P1-27: /clear is /new's alias, typed only; /new is listed once.
  await expect(menu.getByRole('menuitem', { name: /^\/clear · / })).toHaveCount(0);
  // A command goes into the message box; nothing is sent.
  await menu.getByRole('menuitem', { name: /^\/model · / }).click();
  await expect(composer).toHaveValue('/model ');
  // A draft is never replaced: the menu says so and leaves it (review fix).
  await composer.fill('A long message I am still writing');
  await conversation.getByRole('button', { name: 'Attach and commands', exact: true }).click();
  await page.getByRole('menu').getByRole('menuitem', { name: /^\/help · / }).click();
  await expect(composer).toHaveValue('A long message I am still writing');
  await expect(page.getByText('Your message stays: a command goes in an empty message box. Send or clear it first.').first()).toBeVisible();
  await composer.fill('');
  // Claude Code's MCP servers: listed at once, read-only, said to be its own.
  const before = await page.evaluate(() => (window as unknown as { __mockApiCalls?: Record<string, number> }).__mockApiCalls?.chat_send ?? 0);
  await conversation.getByRole('button', { name: 'Attach and commands', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Claude Code’s MCP servers (its own list)' }).click();
  await expect(conversation.getByText('$ claude mcp list')).toBeVisible();
  await expect(conversation.getByText(/notion-mcp-server/)).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as { __mockApiCalls?: Record<string, number> }).__mockApiCalls?.chat_send ?? 0)).toBe(before);
  // All commands: the slash list, the same as typing "/".
  await conversation.getByRole('button', { name: 'Attach and commands', exact: true }).click();
  await page.getByRole('menuitem', { name: 'All commands, with Claude Code’s skills…' }).click();
  await expect(composer).toHaveValue('/');
  await expect(page.locator('.slash-popup')).toBeVisible();
});
