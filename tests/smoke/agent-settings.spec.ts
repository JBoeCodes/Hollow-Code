import { expect, test, type Page } from './fixtures/test';
import { choose, openAdvancedGroup, optionLabels, optionValues } from './choose';

/** A Codex-only computer proves that app defaults and common surfaces do not
 * require Claude. State is persisted across page reloads at the mock IPC seam. */
async function installAgentFixtures(page: Page, failSave = false, failLoad = false) {
  await page.route('**/src/mockApi.ts*', async route => {
    const response = await route.fetch();
    const original = await response.text();
    if (!original.includes('export async function mockInvoke(')) throw new Error('Agent fixture seam missing');
    await route.fulfill({ response, body: `${original}
{
  const baseAgentMock = mockInvoke;
  mockInvoke = async (command, args) => {
    if (command === 'chat_send') localStorage.setItem('mockLastChatArgs', JSON.stringify(args));
    if (command === 'agent_available') return ['codex'];
    if (command === 'agent_descriptors') return [
      { id: 'claude', label: 'Claude', available: false, capabilities: { modelSelection: true, sessionResume: true, toolRestrictions: true, ownedMcp: true, structuredOutput: true, effortLevels: ['low', 'medium', 'high', 'xhigh', 'max'] } },
      { id: 'codex', label: 'Codex', available: true, capabilities: { modelSelection: true, sessionResume: true, toolRestrictions: false, ownedMcp: true, structuredOutput: true, effortLevels: ['low', 'medium', 'high', 'xhigh', 'max'] } },
      { id: 'antigravity', label: 'Antigravity', available: false, capabilities: { modelSelection: true, sessionResume: false, toolRestrictions: false, ownedMcp: false, structuredOutput: false, effortLevels: ['low', 'medium', 'high', 'xhigh', 'max'] } }
    ];
    if (command === 'get_agent_settings' && ${failLoad} && localStorage.getItem('allowAgentSettingsLoad') !== 'true') throw new Error('Fixture settings unreadable');
    if (command === 'get_agent_settings') return JSON.parse(localStorage.getItem('agentSettingsFixture') ?? '{"defaultAgent":"codex","models":{"codex":"account-model-v1"}}');
    if (command === 'set_agent_settings') {
      if (${failSave}) throw new Error('Fixture disk unavailable');
      localStorage.setItem('agentSettingsFixture', JSON.stringify(args.settings));
      return args.settings;
    }
    if (command === 'write_workflow') {
      localStorage.setItem('workflowSettingsFixture', JSON.stringify(args));
      const detail = await baseAgentMock('read_workflow', { slug: args.slug });
      return { ...detail.workflow, ...args.input };
    }
    return baseAgentMock(command, args);
  };
}
` });
  });
}

async function openSettings(page: Page, section = 'models', group = 'this-mac') {
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), section);
  } else {
    // Twelve sections since Settings PR 3: the History & recovery link and the
    // runtime rows sit in Advanced's This Mac group.
    const name = ({ models: 'Models', chat: 'Chat', connectors: 'Connectors', advanced: 'Advanced' } as Record<string, string>)[section];
    await page.locator('.settings-index').getByRole('button', { name, exact: true }).click();
  }
  if (section === 'advanced') await openAdvancedGroup(page, group);
}

/** Models lists the agents that aren't installed behind one closed fold (Settings S5b). */
async function showNotInstalled(page: Page) {
  const fold = page.locator('.agent-defaults-others');
  if (await fold.getAttribute('open') === null) await fold.locator(':scope > summary').click();
}

/** Each agent's watchdog and deadline sit in Advanced, Run limits, with their own Save (Settings S5b). */
const openRunLimits = (page: Page) => openSettings(page, 'advanced', 'run-limits');
const saveRunLimits = async (page: Page) => {
  await page.getByRole('button', { name: 'Save run limits', exact: true }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Run limits saved.' })).toBeVisible();
};

test('Settings says when the installed Codex is not verified for tool-free helpers, with the fallback', async ({ page }) => {
  // Agent effectiveness F9 (wave 4 · V1): the sentence comes from the
  // backend probe (codex_probe), so this mocks an unverified version.
  await page.route('**/src/mockApi.ts*', async route => {
    const response = await route.fetch();
    const original = await response.text();
    if (!original.includes('export async function mockInvoke(')) throw new Error('Agent fixture seam missing');
    await route.fulfill({ response, body: `${original}
{
  const baseCodexMock = mockInvoke;
  mockInvoke = async (command, args) => {
    if (command === 'codex_helper_status') return { status: 'unverified', version: 'codex-cli 0.160.0', verifiedVersions: ['codex-cli 0.146.0', 'codex-cli 0.153.4'], served: 'probed', message: "Codex helpers don't run on codex-cli 0.160.0: Hollow hasn't verified its tool-free mode on this version (verified: 0.146.0, 0.153.4). Nothing ran, and Hollow doesn't switch agents by itself: choose Claude Code as the default agent for helpers, or install a verified Codex." };
    return baseCodexMock(command, args);
  };
}
` });
  });
  await page.goto('/?mock=1');
  await openSettings(page, 'advanced'); // the Codex helpers readout sits under Runtime, in This Mac
  const row = page.locator('.settings-row').filter({ has: page.locator('.settings-row-k', { hasText: 'Codex helpers' }) });
  await expect(row).toContainText('codex-cli 0.160.0');
  await expect(row).toContainText('Nothing ran');
  await expect(row).toContainText('choose Claude Code as the default agent for helpers');
});

test('app-wide agent and arbitrary model defaults save and survive a reload', async ({ page }) => {
  await installAgentFixtures(page);
  await page.goto('/?mock=1');
  await openSettings(page);
  await expect(page.getByLabel('Default agent', { exact: true })).toHaveAttribute('data-value', 'codex');
  // A saved ID the catalog doesn't list shows as a custom ID, unchanged.
  await expect(page.getByLabel('Codex model', { exact: true })).toHaveAttribute('data-value', '__custom__');
  await expect(page.getByLabel('Codex model custom ID', { exact: true })).toHaveValue('account-model-v1');
  await expect(page.getByRole('button', { name: 'Save agent defaults', exact: true })).toBeDisabled();

  await page.getByLabel('Codex model custom ID', { exact: true }).fill('custom-model-2026-09');
  // Only Codex is installed here: Claude's model is behind Not installed.
  await showNotInstalled(page);
  await choose(page.getByLabel('Claude model', { exact: true }), '__custom__');
  await page.getByLabel('Claude model custom ID', { exact: true }).fill('another-provider-model');
  await page.getByRole('button', { name: 'Save agent defaults', exact: true }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Agent defaults saved.' })).toBeVisible();
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('agentSettingsFixture')!))).toEqual({
    defaultAgent: 'codex', models: { codex: 'custom-model-2026-09', claude: 'another-provider-model' },
  });

  await page.reload();
  await openSettings(page);
  await expect(page.getByLabel('Default agent', { exact: true })).toHaveAttribute('data-value', 'codex');
  await expect(page.getByLabel('Codex model custom ID', { exact: true })).toHaveValue('custom-model-2026-09');
  await choose(page.getByLabel('Default agent', { exact: true }), '');
  await choose(page.getByLabel('Codex model', { exact: true }), '');
  await page.getByRole('button', { name: 'Save agent defaults', exact: true }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Agent defaults saved.' })).toBeVisible();
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('agentSettingsFixture')!))).toEqual({
    defaultAgent: null, models: { claude: 'another-provider-model' },
  });
});

test('Settings saves a per-agent effort (Paperclip PR 14a)', async ({ page }) => {
  await installAgentFixtures(page);
  await page.goto('/?mock=1');
  await openSettings(page);
  // COO rebuild L7: the per-agent levels show in Fixed effort (Automatic is the CFO's).
  await expect(page.getByLabel('Claude effort', { exact: true })).toHaveCount(0);
  await choose(page.getByLabel('Effort', { exact: true }), 'fixed');
  const claudeEffort = page.getByLabel('Claude effort', { exact: true });
  await expect(claudeEffort).toHaveAttribute('data-value', '');
  expect(await optionLabels(claudeEffort)).toEqual(['Runtime default', 'low', 'medium', 'high', 'xhigh', 'max']);
  await expect(page.getByLabel('Codex effort', { exact: true })).toBeVisible();
  // 2026-10-02: every agent offers every level (each run maps it per model).
  expect(await optionLabels(page.getByLabel('Antigravity effort', { exact: true }))).toEqual(['Runtime default', 'low', 'medium', 'high', 'xhigh', 'max']);

  // Effort saves on change, like the composer's Effort list (P0-19): no Save.
  await choose(claudeEffort, 'high');
  await expect(page.getByRole('status').filter({ hasText: 'Claude effort saved: high.' })).toBeVisible();
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('agentSettingsFixture')!))).toEqual({
    defaultAgent: 'codex', models: { codex: 'account-model-v1' }, efforts: { claude: 'high' },
  });
  await expect(claudeEffort).toHaveAttribute('data-value', 'high');
  await expect(page.getByRole('button', { name: 'Save agent defaults', exact: true })).toBeDisabled();

  // Back to the runtime default: `efforts` leaves the payload again.
  await choose(claudeEffort, '');
  await expect(page.getByRole('status').filter({ hasText: 'Claude effort saved: runtime default.' })).toBeVisible();
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('agentSettingsFixture')!))).toEqual({
    defaultAgent: 'codex', models: { codex: 'account-model-v1' },
  });
});

test('Settings saves the Codex no-output watchdog without resetting other fields (Paperclip PR 15b)', async ({ page }) => {
  await installAgentFixtures(page);
  await page.goto('/?mock=1');
  // The effort saves on change in Models; the watchdog's save must keep it.
  await openSettings(page);
  await choose(page.getByLabel('Effort', { exact: true }), 'fixed');
  await choose(page.getByLabel('Claude effort', { exact: true }), 'high');
  await expect(page.getByRole('status').filter({ hasText: 'Claude effort saved: high.' })).toBeVisible();
  await openRunLimits(page);
  const minutes = page.getByLabel('Stop a silent Codex run after', { exact: true });
  await expect(minutes).toHaveValue('30');
  await expect(page.getByRole('button', { name: 'Save run limits', exact: true })).toBeDisabled();

  await minutes.fill('45');
  await minutes.press('Tab');
  await saveRunLimits(page);
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('agentSettingsFixture')!))).toEqual({
    defaultAgent: 'codex', models: { codex: 'account-model-v1' }, efforts: { claude: 'high' }, codexNoOutputTimeoutMin: 45,
  });

  // Off is saved as null (explicitly disabled), and the effort survives.
  await page.getByLabel('Codex watchdog off', { exact: true }).check();
  await expect(minutes).toBeDisabled();
  await saveRunLimits(page);
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('agentSettingsFixture')!))).toEqual({
    defaultAgent: 'codex', models: { codex: 'account-model-v1' }, efforts: { claude: 'high' }, codexNoOutputTimeoutMin: null,
  });

  // Back on at the default: the field leaves the payload again.
  await page.getByLabel('Codex watchdog off', { exact: true }).uncheck();
  await expect(minutes).toHaveValue('30');
  await saveRunLimits(page);
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('agentSettingsFixture')!))).toEqual({
    defaultAgent: 'codex', models: { codex: 'account-model-v1' }, efforts: { claude: 'high' },
  });
});

test('every agent has a watchdog and an optional deadline in Settings (merged plan slice 3)', async ({ page }) => {
  await installAgentFixtures(page);
  await page.goto('/?mock=1');
  await openRunLimits(page);
  for (const label of ['Claude', 'Codex', 'Antigravity', 'Grok']) {
    await expect(page.getByLabel(`Stop a silent ${label} run after`, { exact: true })).toHaveValue('30');
    await expect(page.getByLabel(`End a ${label} run after`, { exact: true })).toHaveValue('');
  }
  await page.getByLabel('Claude watchdog off', { exact: true }).check();
  await expect(page.getByLabel('Stop a silent Claude run after', { exact: true })).toBeDisabled();
  const deadline = page.getByLabel('End a Grok run after', { exact: true });
  await deadline.fill('20');
  await deadline.press('Tab');
  await saveRunLimits(page);
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('agentSettingsFixture')!))).toEqual({
    defaultAgent: 'codex', models: { codex: 'account-model-v1' }, noOutputTimeoutMin: { claude: null }, deadlineMin: { grok: 20 },
  });

  // Clearing the deadline removes it; Codex's own field is untouched.
  await deadline.fill('');
  await deadline.press('Tab');
  await saveRunLimits(page);
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('agentSettingsFixture')!))).toEqual({
    defaultAgent: 'codex', models: { codex: 'account-model-v1' }, noOutputTimeoutMin: { claude: null },
  });
});

test('a failed settings save keeps the unsaved model editable and reports the error', async ({ page }) => {
  await installAgentFixtures(page, true);
  await page.goto('/?mock=1');
  await openSettings(page);
  await page.getByLabel('Codex model custom ID', { exact: true }).fill('unsaved-model');
  await page.getByRole('button', { name: 'Save agent defaults', exact: true }).click();
  await expect(page.getByRole('alert').filter({ hasText: 'Could not save agent defaults:' })).toContainText('Fixture disk unavailable');
  await expect(page.getByLabel('Codex model custom ID', { exact: true })).toHaveValue('unsaved-model');
  await expect(page.getByRole('button', { name: 'Save agent defaults', exact: true })).toBeEnabled();
  expect(await page.evaluate(() => localStorage.getItem('agentSettingsFixture'))).toBeNull();
});

test('capability doctor accepts a Codex-only computer without requiring Claude', async ({ page }) => {
  await installAgentFixtures(page);
  await page.goto('/?mock=1');
  await openSettings(page, 'advanced');
  const runtimeCheck = page.locator('.capability-doctor-row').filter({ hasText: 'Agent runtime' });
  await expect(runtimeCheck).toHaveAttribute('data-state', 'live');
  await expect(runtimeCheck).toContainText('Available to Hollow: codex');
  // The chip says it in words, not the state's id (night ST10 follow-up).
  await expect(runtimeCheck.locator('.settings-state-chip')).toHaveText('working');
  await expect(runtimeCheck).not.toContainText('Claude');
});


test('unreadable agent settings leave a retryable form rather than hiding the page', async ({ page }) => {
  await installAgentFixtures(page, false, true);
  await page.goto('/?mock=1');
  await openSettings(page);
  await expect(page.getByRole('alert').filter({ hasText: 'Could not load agent defaults:' })).toContainText('Fixture settings unreadable');
  await expect(page.getByLabel('Default agent', { exact: true })).toBeDisabled();
  await page.evaluate(() => localStorage.setItem('allowAgentSettingsLoad', 'true'));
  await page.getByRole('button', { name: 'Retry agent settings', exact: true }).click();
  await expect(page.getByLabel('Default agent', { exact: true })).toBeEnabled();
  await expect(page.getByLabel('Default agent', { exact: true })).toHaveAttribute('data-value', 'codex');
});

test('safer permissions start a new native session while preserving conversation context', async ({ page }) => {
  await installAgentFixtures(page);
  await page.goto('/?mock=1');
  await openSettings(page, 'advanced'); // History & recovery is a link row in Advanced → This Mac (Settings PR 4)
  await page.getByRole('button', { name: 'Open Library, History', exact: true }).click();
  await page.locator('.library-tabs').getByRole('button', { name: 'Conversations', exact: true }).click();
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toBeVisible();
  await page.evaluate(async moduleUrl => {
    const runtime = await import(moduleUrl);
    await runtime.refreshAgentSettings();
    runtime.setChatAgent('codex');
    runtime.setChatPermissionMode('bypassPermissions');
    runtime.setChatSessionIdPersistent('codex-bypass-session');
    runtime.setChatAgentSessionKeyPersistent('hollow-permission-history');
    runtime.pushChatTurn({ role: 'user', text: 'Keep the launch context available.', tools: [] });
    runtime.pushChatTurn({ role: 'assistant', agent: 'codex', text: 'The launch decision is recorded.', tools: [] });
  }, '/src/store/runtime.ts');
  // 2026-11-01: Permissions is a row of the model menu; its button names the mode.
  // Overnight 11-12 (C3): the words show on the composer's access chip.
  // Per-agent menus (2026-11-12): a Codex chat names Codex's own modes.
  const modelButton = page.getByRole('button', { name: /^Model: Codex · .* · Permissions: Full access \(unrestricted\)$/ });
  const accessChip = page.getByRole('button', { name: 'Access: Full access (unrestricted)', exact: true });
  await expect(accessChip).toContainText('Full access');
  await accessChip.click();
  await expect(page.getByRole('menuitem', { name: 'Full access · current', exact: true })).toBeDisabled();
  await expect(page.getByRole('menuitem', { name: 'Read only', exact: true })).toBeEnabled();
  await page.keyboard.press('Escape');
  await modelButton.click();
  await page.getByRole('button', { name: /^Permissions: Full access/ }).click();
  await expect(page.getByRole('group', { name: 'Permission settings' })).toBeVisible();
  await page.getByRole('listbox', { name: 'Permissions' }).getByRole('option', { name: 'Read only', exact: true }).click();
  await expect(modelButton).toHaveCount(0);
  await expect(page.getByRole('button', { name: /^Model: Codex · .* · Permissions: Read only$/ })).toBeVisible();
  await expect.poll(() => page.evaluate(() => localStorage.getItem('chatSessionId'))).toBeNull();
  await expect.poll(() => page.evaluate(() => new Map(JSON.parse(localStorage.getItem('hollow.chat.modes.v1') ?? '[]')).get('hollow-permission-history'))).toBe('plan');
  expect(await page.evaluate(() => localStorage.getItem('chatAgentSessionKey'))).toBe('hollow-permission-history');
  await expect(page.locator('.chat-main .turn.assistant')).toContainText('The launch decision is recorded.');
  await page.keyboard.press('Escape');
  await page.getByRole('textbox', { name: 'Chat message' }).fill('Explain the launch decision.');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  // The key stays and the native id goes, so the backend's context compiler
  // (agent effectiveness F4) rebuilds the conversation from the session
  // store for the new native session; the composer sends only its request.
  await expect.poll(() => page.evaluate(() => localStorage.getItem('mockLastChatPrompt'))).toContain('Explain the launch decision.');
  const sent = await page.evaluate(() => localStorage.getItem('mockLastChatPrompt') ?? '');
  expect(sent).not.toContain('<conversation-context>');
  expect(sent).not.toContain('The launch decision is recorded.');
  const args = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}'));
  expect(args.sessionKey).toBe('hollow-permission-history');
  expect(args.sessionId ?? null).toBeNull();
  expect(args.permissionMode).toBe('plan');
});

test('the new-chats permission mode leaves the chat on screen alone, and the Danger row is gone (P0-19; Settings PR 6, C6)', async ({ page }) => {
  await installAgentFixtures(page);
  await page.goto('/?mock=1');
  await expect(page.getByRole('button', { name: /Settings/ }).first()).toBeVisible();
  await page.evaluate(async moduleUrl => {
    const runtime = await import(moduleUrl);
    runtime.setChatPermissionMode('bypassPermissions');
    runtime.setChatSessionIdPersistent('bypass-native');
    runtime.setChatBusy(true);
  }, '/src/store/runtime.ts');
  await openSettings(page, 'chat');
  // Settings no longer acts on the chat on screen: the composer's permission menu and /permissions do.
  await expect(page.getByRole('button', { name: 'Revert to Edits only', exact: true })).toHaveCount(0);
  await expect(page.locator('.settings-row-danger')).toHaveCount(0);
  // One per agent since 2026-11-12, each in that agent's words.
  await choose(page.getByRole('combobox', { name: 'Permission mode for new Claude chats', exact: true }), 'plan');
  await expect(page.getByRole('status').filter({ hasText: 'New Claude chats start in Plan.' })).toBeVisible();
  expect(await page.evaluate(async moduleUrl => (await import(moduleUrl)).chatPermissionMode(), '/src/store/runtime.ts')).toBe('bypassPermissions');
  expect(await page.evaluate(() => localStorage.getItem('chatSessionId'))).toBe('bypass-native');
});

test('each agent lists its models, and picking one saves its exact ID', async ({ page }) => {
  await installAgentFixtures(page);
  await page.goto('/?mock=1');
  await openSettings(page);
  const codex = page.getByLabel('Codex model', { exact: true });
  expect(await optionLabels(codex)).toEqual(expect.arrayContaining(['Runtime default', 'GPT-6-Sol', 'GPT-6-Astra', 'Custom model ID…']));
  expect(await optionValues(codex)).toEqual(expect.arrayContaining(['gpt-6-sol', 'gpt-6-astra']));
  await showNotInstalled(page);
  expect(await optionLabels(page.getByLabel('Claude model', { exact: true }))).toContain('Opus 5.5');
  await choose(codex, 'gpt-6-astra');
  await expect(page.getByLabel('Codex model custom ID', { exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Save agent defaults', exact: true }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Agent defaults saved.' })).toBeVisible();
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('agentSettingsFixture')!).models)).toEqual({ codex: 'gpt-6-astra' });
});
