import { expect, test, type Page } from './fixtures/test';

// COO rebuild L6 (11-10): any installed agent may run the COO. Its model menu
// and slash commands offer every provider like any chat; nothing is disabled
// or refused for being another provider. (Was the P0-20 Claude-only rule.)

async function setup(page: Page) {
  await page.addInitScript(() => {
    localStorage.setItem('chatAgent', 'codex');
    localStorage.setItem('mockAgentSettings', JSON.stringify({ defaultAgent: 'codex', models: { codex: 'gpt-6-sol', claude: 'opus' } }));
    localStorage.setItem('mockWorkspaces', JSON.stringify([{ id: 'main', kind: 'main', title: 'COO', status: 'active', path: 'workspaces/main', cwd: '/Users/fixture/vault/workspaces/main', modified: new Date().toISOString() }]));
  });
  await page.goto('/?mock=1');
  await page.evaluate(async () => {
    const path = '/src/store.ts';
    const store = await import(/* @vite-ignore */ path);
    store.setChatAgentPersistent('codex');
  });
}
const orb = (page: Page) => page.locator('.top-bar').getByRole('button', { name: /^(COO|Back from the COO) ·/ });

for (const height of [480, 600]) test(`COO model search stays inside the window at Largest text, 720x${height}`, async ({ page }) => {
  await page.setViewportSize({ width: 720, height });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.addInitScript(() => localStorage.setItem('hollow.studio.textScale', '125'));
  await setup(page);
  await orb(page).click();
  await page.getByRole('button', { name: /^Model:/ }).click();
  const search = page.getByRole('textbox', { name: 'Search models' });
  await expect.poll(async () => (await search.boundingBox())?.y ?? -1).toBeGreaterThanOrEqual(44);
  await search.fill('codex');
  await expect.poll(async () => (await search.boundingBox())?.y ?? -1).toBeGreaterThanOrEqual(44);
  await expect(search).toBeInViewport();
  await expect(page.getByRole('option', { name: /Codex default/ })).toBeEnabled();
  await search.fill('opus');
  const claude = page.getByRole('option', { name: /Opus/ }).first();
  await claude.click();
  await expect(search).toHaveCount(0);
});

test('a fresh COO page keeps the chosen agent and offers every provider', async ({ page }) => {
  await setup(page);
  await orb(page).click();
  const trigger = page.getByRole('button', { name: /^Model:/ });
  await expect(trigger).toHaveAccessibleName(/^Model: Codex/);
  await trigger.click();
  await expect(page.getByRole('option', { name: /Codex default/ })).toBeEnabled();
  await expect(page.getByRole('option', { name: /Claude default/ })).toBeEnabled();
  await expect(page.getByText(/Claude only|requires Claude/)).toHaveCount(0);
  await page.getByRole('textbox', { name: 'Search models' }).fill('custom-v9');
  await expect(page.getByRole('option', { name: /with Codex/ })).toBeEnabled();
  await expect(page.getByRole('option', { name: /with Claude/ })).toBeEnabled();
});

test('COO slash commands switch provider and model like any chat', async ({ page }) => {
  await setup(page);
  await orb(page).click();
  const result = await page.evaluate(async () => {
    const storePath = '/src/store.ts', slashPath = '/src/components/chat/slashCommands.ts';
    const store = await import(/* @vite-ignore */ storePath);
    const slash = await import(/* @vite-ignore */ slashPath);
    store.setChatAgentPersistent('claude');
    const command = slash.createSlashCommands({ followSend() {} });
    command.handleLocalCommand('/agent codex');
    const agent = store.chatAgent();
    command.handleLocalCommand('/model gpt-6-sol');
    return { agent, model: store.chatModel() };
  });
  expect(result).toEqual({ agent: 'codex', model: 'gpt-6-sol' });
  expect(await page.evaluate(async () => {
    const path = '/src/store.ts';
    const store = await import(/* @vite-ignore */ path);
    return store.chatTurns.filter((t: { text: string }) => /Claude only/.test(t.text)).length;
  })).toBe(0);
});
