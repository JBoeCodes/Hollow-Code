import { showWorkspaceList } from './choose';
import { expect, test, type Page } from './fixtures/test';
import { choose, openAdvancedGroup } from './choose';

async function openSection(page: Page, id: string, label: RegExp) {
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), id);
  } else {
    await page.locator('.settings-index').getByRole('button', { name: label }).click();
  }
}

test.beforeEach(async ({ page }) => { await page.goto('/?mock=1'); });

test('service setup preserves a draft and saves GitHub only on request', async ({ page }) => {
  await openSection(page, 'connectors', /^Connectors/);
  await expect(page.getByRole('heading', { name: 'Services', exact: true })).toBeVisible();
  // Hollow's own Notion setup went with Notion sync (2026-11-10).
  await expect(page.getByRole('heading', { name: 'Notion setup', exact: true })).toHaveCount(0);
  // The catalog and the form are sheets (Settings PR 5): the form hides the page behind it, so
  // a draft can't be replaced by accident; Cancel closes it and focus returns to its opener.
  const addRow = page.getByRole('button', { name: 'Add a connector…', exact: true });
  await addRow.click();
  await page.getByRole('button', { name: 'Custom connector' }).click();
  await page.getByLabel('Connector name').fill('Keep this draft');
  await page.getByRole('dialog', { name: 'Add a custom connector' }).getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(page.getByLabel('Connector name')).toHaveCount(0);
  await expect(addRow).toBeFocused();
  await page.getByRole('button', { name: 'Set up GitHub', exact: true }).click();
  await expect(page.getByLabel('Connector URL')).toHaveValue('https://api.githubcopilot.com/mcp/readonly');
  await expect(page.getByRole('button', { name: 'GitHub settings', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Save connector', exact: true }).click();
  const row = page.locator('.cx-row').filter({ has: page.getByRole('button', { name: 'GitHub settings', exact: true }) });
  await expect(row.getByRole('radio', { name: 'Only when relevant', exact: true })).toHaveAttribute('aria-checked', 'true');
  await page.getByRole('button', { name: 'Manage GitHub', exact: true }).click();
  await expect(row).toHaveAttribute('data-open', 'true');
  await expect(page.getByRole('button', { name: 'GitHub settings', exact: true })).toHaveCount(1);
});

test('the connectors library adds, shares and imports connectors in one place', async ({ page }) => {
  await openSection(page, 'connectors', /^Connectors/);
  await expect(page.getByRole('heading', { name: 'Connectors', exact: true, level: 2 })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Your connectors' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Add connectors' })).toBeVisible();
  // Built-in tools live here too, as switches.
  await expect(page.getByText('Built into Hollow', { exact: true })).toBeVisible();
  await expect(page.getByRole('switch', { name: 'Task tools' })).toBeVisible();

  // A custom connector, from the catalog sheet.
  await page.getByRole('button', { name: 'Add a connector…', exact: true }).click();
  await page.getByRole('button', { name: /Custom connector/ }).click();
  await page.getByLabel('Connector name').fill('Superhuman Mail');
  await page.getByLabel('Connector description').fill('Read, search and draft email');
  await page.getByRole('radio', { name: 'Local command' }).click();
  await page.getByLabel('Connector command').fill('npx');
  await page.getByLabel('Connector arguments').fill('-y\nsuperhuman-mcp');
  await page.getByRole('button', { name: 'Add variable' }).click();
  await page.getByLabel('Name', { exact: true }).fill('API_KEY');
  await page.getByLabel('Value', { exact: true }).fill('secret-value');
  await page.getByRole('button', { name: 'Save connector' }).click();

  const row = page.locator('.cx-row').filter({ hasText: 'Superhuman Mail' });
  await expect(row).toHaveAttribute('data-open', 'true');
  await expect(row).toContainText('npx -y superhuman-mcp');
  await expect(row).toContainText('API_KEY');
  await expect(row).not.toContainText('secret-value');
  await expect(row).toContainText('Claude and Codex · chats');
  // A local command Hollow can't probe is configured, never a green "On"
  // (next-level P00: configuration is not evidence of access).
  await expect(row.locator('.cx-pill')).toHaveText('Configured, not checked');

  await row.getByRole('checkbox', { name: 'Codex' }).uncheck();
  await expect(row).toContainText('Claude · chats');
  await row.getByRole('radio', { name: 'Only when relevant' }).click();
  await expect(row).toContainText('only when relevant');
  await row.getByRole('switch', { name: 'Turn off Superhuman Mail' }).click();
  await expect(row.locator('.cx-pill')).toHaveText('Off');
  const saved = await page.evaluate(() => JSON.parse(localStorage.getItem('mockConnectors') ?? '[]'));
  expect(saved[0]).toMatchObject({ id: 'superhuman-mail', agents: ['claude'], mode: 'relevant', envKeys: ['API_KEY'], enabled: false });

  // A connector configured inside Claude Code is offered in the catalog sheet.
  await page.getByRole('button', { name: 'Add a connector…', exact: true }).click();
  await page.locator('.cx-gallery').getByRole('button', { name: 'Add linear' }).click();
  await expect(page.locator('.cx-row').filter({ hasText: 'linear' })).toContainText('from Claude Code');
  await expect(page.locator('.cx-gallery').getByRole('button', { name: 'Add linear' })).toHaveCount(0);

  // claude.ai connectors: Claude only, with live sign-in status and a Sign in action.
  await page.getByRole('button', { name: 'Add a connector…', exact: true }).click();
  await expect(page.locator('.cx-tile').filter({ hasText: 'Microsoft 365' })).toContainText('Sign in to it on claude.ai');
  await page.locator('.cx-gallery').getByRole('button', { name: 'Add Microsoft 365' }).click();
  const m365 = page.locator('.cx-row').filter({ hasText: 'Microsoft 365' });
  await expect(m365.locator('.cx-pill')).toHaveText('Needs sign-in');
  await expect(m365.getByRole('button', { name: 'Sign in' })).toBeVisible();
  await expect(m365).toContainText('Re-authentication is required');
  await expect(m365.getByRole('checkbox', { name: 'Codex' })).toHaveCount(0);
  await expect(page.locator('.cx-banner:not([data-kind])')).toContainText('1 connector needs signing in');
  // Whose connectors they are: the banner and the row name Claude Code's login.
  await expect(page.locator('.cx-banner:not([data-kind])')).toContainText('you@example.com in Example Team');
  await expect(m365).toContainText('Connect it on claude.ai as you@example.com in Example Team');
  await page.getByRole('button', { name: 'Add a connector…', exact: true }).click();
  await page.locator('.cx-gallery').getByRole('button', { name: 'Add Superhuman Mail', exact: true }).click();
  await expect(page.locator('.cx-row').filter({ hasText: 'mcp.mail.superhuman.com' }).locator('.cx-pill')).toHaveText('Available');
  // Every row shows an icon tile (an image or a letter).
  await expect(page.locator('.cx-row .cx-icon').first()).toBeVisible();
});

test('built-in Microsoft 365 runs on Hollow\'s own sign-in', async ({ page }) => {
  await page.evaluate(() => { localStorage.setItem('mockBuiltins', '1'); localStorage.removeItem('mockM365'); });
  await page.reload();
  await openSection(page, 'connectors', /^Connectors/);
  // Hollow's own Notion connector is gone (2026-11-10); only Microsoft 365 is built in here.
  await expect(page.locator('.cx-row').filter({ hasText: 'Built into Hollow' }).filter({ hasText: 'Notion' })).toHaveCount(0);
  const m365 = page.locator('.cx-row').filter({ hasText: 'Microsoft 365' });
  await expect(m365.locator('.cx-pill')).toHaveText('Needs sign-in');
  await m365.getByRole('button', { name: 'Sign in', exact: true }).click();
  // The sign-in Hollow just completed is its own check, with both scopes.
  await expect(m365.locator('.cx-pill')).toHaveText('Available');
  await expect(m365).toContainText('Signed in to Microsoft as you@example.com');
  // Built into Hollow: no address, no Edit or Remove; sign out instead.
  await m365.getByRole('button', { name: 'Microsoft 365 settings' }).click();
  await expect(m365).toContainText('Built into Hollow');
  await expect(m365.getByRole('button', { name: 'Remove' })).toHaveCount(0);
  await expect(m365.getByRole('button', { name: 'Edit' })).toHaveCount(0);
  await expect(m365.getByRole('checkbox', { name: 'Codex' })).toBeChecked();
  await m365.getByRole('button', { name: 'Sign out' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Sign out' }).click();
  await expect(m365.locator('.cx-pill')).toHaveText('Needs sign-in');
});

// The prioritizer lives in Advanced → Search & indexes (a closed group since Settings PR 3).
async function openAdvanced(page: Page) {
  await openSection(page, 'advanced', /^Advanced/);
  await openAdvancedGroup(page, 'indexes');
}

test('System Advanced holds the prioritizer, and the index has no legacy labels', async ({ page }) => {
  await openAdvanced(page);
  await expect(page.getByRole('heading', { name: 'Context prioritizer', exact: true })).toBeVisible();
  await expect(page.getByText(/kept 8 of 8 passages/)).toBeVisible();
  // The key is managed in Settings → Accounts; the switches stay here.
  await expect(page.getByText(/No Jev key yet/)).toBeVisible();
  await page.getByRole('button', { name: 'Manage key' }).click();
  await page.getByLabel('Jev API key').fill('ts-test-key');
  await page.getByRole('button', { name: 'Save key' }).click();
  await expect(page.locator('[data-provider="jev"]').getByRole('button', { name: 'Test' })).toBeVisible();
  await openAdvanced(page);
  await expect(page.getByText(/Jev key saved/)).toBeVisible();
  await page.getByRole('switch', { name: 'Use Jev to rank context' }).click();
  await expect(page.getByText('Jev is on for new requests.')).toBeVisible();
  if ((page.viewportSize()?.width ?? 1000) > 860) {
    await expect(page.locator('.settings-index')).not.toContainText('legacy');
    // Twelve sections since Settings PR 3. Search & indexes is inside Advanced, not a section.
    await expect(page.locator('.settings-index')).toContainText('Personalization');
    await expect(page.locator('.settings-index')).not.toContainText('Search & indexes');
  }
});

test('workspace rows show the project app icon', async ({ page }) => {
  await page.evaluate(() => {
    localStorage.setItem('mockWorkspaces', JSON.stringify([{ id: 'local-aaaaaaaaaaaaaaaaaaaaaaaa', title: 'Hollow', status: 'active', path: 'workspaces/local-aaaaaaaaaaaaaaaaaaaaaaaa', cwd: '/v/workspaces/local-aaaaaaaaaaaaaaaaaaaaaaaa', modified: new Date().toISOString() }]));
  });
  await page.reload();
  await showWorkspaceList(page);
  const card = page.locator('.work-card').filter({ hasText: 'Hollow' });
  await expect(card.locator('.project-icon-art')).toBeVisible();
  // Unique project names keep their origin in the row's tooltip.
  await expect(card).toHaveAttribute('title', /Project folder · example-user/);
});

test('Superhuman Mail adds on Hollow’s sign-in in one click (10-11 B4)', async ({ page }) => {
  await openSection(page, 'connectors', /^Connectors/);
  await page.getByRole('button', { name: 'Add a connector…', exact: true }).click();
  const add = page.locator('.cx-gallery').getByRole('button', { name: 'Add Superhuman Mail on Hollow’s sign-in' });
  await add.click();
  await expect(page.getByText('Remote server, on Hollow’s own sign-in').first()).toBeVisible();
  await expect(add).toHaveCount(0);
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});
