import { expect, test, type Page } from './fixtures/test';

/**
 * Settings PR 3 (2026-11-12, the settings redesign plan 3.2): twelve
 * sections in one list. The seven before them and every id they had
 * absorbed send their deep links to their new homes; Chat, Accounts and
 * Keyboard shortcuts are sections again; Advanced holds closed groups
 * (This Mac leads with the capability doctor), and a search or jump into
 * one opens it. Votes and Routing are gone (overnight 11-08 S5).
 */

const index = (page: Page) => page.getByRole('navigation', { name: 'Settings sections' });

async function openSettings(page: Page) {
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).waitFor();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page.locator('.settings-page')).toBeVisible();
}

/** The runtime module is imported once and kept on window: a dynamic import()
 *  awaited while the page is busy can be collected ("Promise was collected",
 *  reported as a destroyed context; docs/reference/quirks.md). */
async function request(page: Page, id: string) {
  await page.evaluate(async section => {
    const w = window as unknown as { __settingsRuntime?: { requestOpenSettingsSection: (id: string) => void } };
    w.__settingsRuntime ??= await import('/src/store/runtime.ts');
    w.__settingsRuntime.requestOpenSettingsSection(section);
  }, id);
}

test('the index lists eighteen sections, with no group caps (PR 3)', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await openSettings(page);
  const sections = index(page).locator('.settings-index-item:not(.settings-index-control)');
  await expect(sections).toHaveText(['General', 'Chat', 'Models', 'Accounts', 'Personalization', 'Connectors', 'Sources', 'Background work', 'Devices', 'Usage', 'Keyboard shortcuts', 'Diffs', 'Browser', 'Worktrees', 'Archived', 'Updates', 'About', 'Advanced']);
  await expect(index(page).locator('.settings-index-cap')).toHaveCount(0);
  // A fresh Settings opens on the first section.
  await expect(page.locator('.settings-section .settings-section-title').first()).toHaveText('General');
  for (const gone of ['Interface', 'Agents', 'Connections', 'Knowledge', 'Automation', 'System', 'Response feedback', 'Context prioritizer', 'Search by meaning', 'Memory', 'Companion devices', 'Hollow folder', 'Skills', 'Search & indexes', 'History & recovery']) {
    await expect(index(page).getByRole('button', { name: gone, exact: true })).toHaveCount(0);
  }
});

test('every retired section id lands on its new home', async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await openSettings(page);
  const homes: [string, string, string, boolean][] = [
    ['feedback', 'Usage', 'Budgets', false],
    ['context', 'Advanced', 'Context prioritizer', true],
    ['vaultsearch', 'Advanced', 'Search by meaning', true],
    ['learning', 'Personalization', 'Memory', false],
    ['doctor', 'Advanced', 'Capability doctor', true],
    ['heartbeat', 'Background work', 'Silent heartbeat', false],
    ['startup', 'General', 'Run at login', false],
    ['debug', 'Advanced', 'Debug logs', true],
  ];
  for (const [id, home, control, inAdvanced] of homes) {
    await page.evaluate(() => document.querySelectorAll('.settings-found').forEach(el => el.classList.remove('settings-found')));
    await request(page, id);
    await expect(page.locator('.settings-section .settings-section-title').first(), id).toHaveText(home);
    await expect(page.locator('.settings-found').first(), id).toContainText(control);
    // A control in Advanced is inside a closed group, and the jump opened it.
    // Anything else is never a card inside a fold (About you's adaptation map links to Memory too).
    // Counted at once: the mark fades after 1.8 s, so a retrying check would pass anyway.
    if (inAdvanced) await expect(page.locator('details.settings-group').filter({ has: page.locator('.settings-found') }).first(), id).toHaveAttribute('open', '');
    else expect(await page.locator('details.settings-fold .settings-found').count(), id).toBe(0);
  }
});

test('an old section or part id opens its new section, or its part with the part’s heading focused (PR 3)', async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await openSettings(page);
  // Ids that are sections now open them with the section's title focused.
  const sections: [string, string][] = [
    ['shortcuts', 'Keyboard shortcuts'], ['chat', 'Chat'], ['accounts', 'Accounts'], ['companions', 'Devices'],
    ['interface', 'General'], ['defaults', 'Models'], ['operator', 'Personalization'], ['automation', 'Background work'], ['system', 'Advanced'],
  ];
  for (const [id, home] of sections) {
    await request(page, id);
    const title = page.locator('.settings-section .settings-section-title').first();
    await expect(title, id).toHaveText(home);
    await expect(title, id).toBeFocused();
  }
  const parts: [string, string, string][] = [
    ['selfhosted', 'Models', 'Self-hosted models'],
    ['indexes', 'Advanced', 'Search & indexes'],
    ['vault', 'Sources', 'Hollow folder'],
  ];
  for (const [id, home, part] of parts) {
    await request(page, id);
    await expect(page.locator('.settings-section .settings-section-title').first(), id).toHaveText(home);
    const heading = page.locator(`#settings-${id} > .settings-part-head .settings-subhead-title`);
    await expect(heading, id).toHaveText(part);
    await expect(heading, id).toBeFocused();
    await expect(heading, id).toBeInViewport();
  }
});

test('old ids and labels of what moved to Library and Automations open those pages (PR 4)', async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await openSettings(page);
  const libraryTab = page.getByRole('navigation', { name: 'Library view' });
  for (const [id, tab] of [['skills', 'Skills'], ['history', 'History']] as const) {
    await request(page, id);
    await expect(libraryTab.getByRole('button', { name: tab, exact: true }), id).toHaveAttribute('aria-pressed', 'true');
    await openSettings(page);
  }
  await page.evaluate(async () => (await import('/src/store/runtime.ts')).requestOpenSettingsSection('automation', 'Automation windows'));
  await expect(page.getByRole('list', { name: 'Automation windows' })).toBeVisible();
});

test('"retitle" finds Chat, which holds Enter sends, the new-chat permission mode and chats at once', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await openSettings(page);
  await page.getByRole('searchbox', { name: 'Find a setting' }).fill('retitle');
  await expect(index(page).getByRole('button', { name: 'Chat', exact: true })).toBeVisible();
  await expect(index(page).getByRole('button', { name: 'Retitle new chats', exact: true })).toBeVisible();
  await page.getByRole('searchbox', { name: 'Find a setting' }).fill('');
  await index(page).getByRole('button', { name: 'Chat', exact: true }).click();
  const chat = page.locator('#settings-chat');
  for (const row of ['Canvases', 'Enter sends a message', 'Retitle new chats', 'Permission mode for new chats', 'Chats at once']) {
    await expect(chat.locator('.settings-row-k').filter({ hasText: row }).first(), row).toBeVisible();
  }
  // Overall and per workspace share the one row (Settings PR 5); each input keeps its name.
  await expect(chat.getByLabel('Chats running at once', { exact: true })).toBeVisible();
  await expect(chat.getByLabel('Chats at once in one workspace', { exact: true })).toBeVisible();
  // The third limit (2026-11-12): subagents one chat runs at once, default 6.
  await expect(chat.getByLabel('Subagents running at once from one chat', { exact: true })).toHaveValue('6');
  await expect(chat.getByRole('button', { name: 'Chats at once', exact: true })).toHaveCount(0);
});

test('Advanced opens with every group closed; This Mac leads with the doctor and Warn under; search opens a group', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await openSettings(page);
  await index(page).getByRole('button', { name: 'Advanced', exact: true }).click();
  const advanced = page.locator('#settings-advanced');
  const groups = advanced.locator('details.settings-group');
  await expect(groups.locator(':scope > summary .settings-subhead-title')).toHaveText(['This Mac', 'Search & indexes', 'Hollow’s own model calls', 'Run limits', 'App builds and web backend', 'Settings file', 'Diagnostics']);
  for (const open of await groups.evaluateAll(els => els.map(el => (el as HTMLDetailsElement).open))) expect(open).toBe(false);
  const thisMac = advanced.locator('#settings-group-this-mac');
  await thisMac.locator(':scope > summary').click();
  await expect(thisMac.getByRole('heading', { name: 'Capability doctor', exact: true })).toBeVisible();
  await expect(thisMac.getByRole('textbox', { name: 'Warn under' })).toBeVisible();
  const readouts = advanced.locator('#settings-group-diagnostics');
  await expect(readouts).not.toHaveAttribute('open', '');
  // Search opens the fold.
  await page.getByRole('searchbox', { name: 'Find a setting' }).fill('launch calls');
  await index(page).getByRole('button', { name: 'Launch calls', exact: true }).click();
  await expect(readouts).toHaveAttribute('open', '');
  // A readout with no row of its own lands on itself and opens the fold, not on the fold.
  await readouts.locator(':scope > summary').click();
  await expect(readouts).not.toHaveAttribute('open', '');
  // Startup warm-up was cut (Settings PR 6, C8): its old name still finds Diagnostics.
  await page.getByRole('searchbox', { name: 'Find a setting' }).fill('warm up');
  await expect(index(page).getByRole('button', { name: 'Startup warm-up', exact: true })).toHaveCount(0);
  await expect(index(page).getByRole('button', { name: 'Diagnostics', exact: true })).toBeVisible();
});

test('Manage key lands on the Jev row; both Jev switches link to the key in Accounts', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await openSettings(page);
  await index(page).getByRole('button', { name: 'Advanced', exact: true }).click();
  await page.locator('#settings-indexes > summary').click();
  const section = page.locator('#settings-indexes');
  // Settings PR 7: each switch says it needs a key and links to the Jev row.
  await expect(section.getByText('Needs a Jev key:')).toHaveCount(2);
  await expect(section.getByRole('button', { name: 'add in Accounts', exact: true })).toHaveCount(2);
  await section.getByRole('button', { name: 'Manage key', exact: true }).click();
  await expect(page.locator('.settings-found').first()).toContainText('Jev (TypeSafe)');
});

test('Personalization no longer has the adaptation map (Settings PR 6, C11)', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await openSettings(page);
  await index(page).getByRole('button', { name: 'Personalization', exact: true }).click();
  await expect(page.locator('#settings-personalization')).toBeVisible();
  await expect(page.locator('#settings-personalization details.personalization-fold')).toHaveCount(0);
  await expect(page.getByText('How Hollow adapts to you')).toHaveCount(0);
});

test('Usage keeps Budgets and has no Routing or Votes', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await openSettings(page);
  await index(page).getByRole('button', { name: 'Usage', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Budgets', exact: true })).toBeVisible();
  // Value this month is on the All agents tab (Settings PR 8).
  await page.getByRole('tab', { name: 'All agents', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Value this month', exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Routing', exact: true })).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'Votes', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Useful', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Not useful', exact: true })).toHaveCount(0);
});

test('the model menu’s Edit opens Settings → Chat', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await openSettings(page);
  await index(page).getByRole('button', { name: 'Advanced', exact: true }).click();
  await page.locator('#settings-group-this-mac > summary').click();
  await page.getByRole('button', { name: 'Open Library, History', exact: true }).click();
  await page.locator('.library-tabs').getByRole('button', { name: 'Conversations', exact: true }).click(); // Settings PR 4 cut the Conversation history button
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toBeVisible();
  await page.locator('.chat-main .model-menu-trigger').first().click();
  await page.locator('.model-menu-edit').click();
  await expect(page.locator('.settings-section .settings-section-title').first()).toHaveText('Chat');
  await expect(page.locator('#settings-chat .settings-section-title')).toBeFocused();
});
