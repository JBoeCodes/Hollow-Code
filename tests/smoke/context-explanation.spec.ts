import { expect, test } from './fixtures/test';
import { choose } from './choose';

test('Conversation history names the kind of conversation and explains it', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await page.getByLabel('Main navigation').getByRole('button', { name: /^Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) await choose(page.getByRole('combobox', { name: 'Settings section' }), 'advanced');
  else await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Advanced', exact: true }).click();
  await page.locator('#settings-group-this-mac > summary').click(); // History & recovery: Advanced → This Mac
  await page.getByRole('button', { name: 'Open Library, History', exact: true }).click();
  await page.locator('.library-tabs').getByRole('button', { name: 'Conversations', exact: true }).click(); // Settings PR 4 cut the Conversation history button
  // Library → Conversations since S4: Library is the page (h1), Conversation history its view (h2).
  await expect(page.getByRole('heading', { name: 'Conversation history', level: 2 })).toBeVisible();
  const heading = page.locator('.chat-conversation-heading');
  await expect(heading).toBeVisible();
  await expect(heading.locator('> span')).toHaveText(/(Workspace conversation|Conversation kept with the COO|COO conversation|Vault conversation)$/);
  await heading.getByText('About this conversation').click();
  await expect(heading.locator('.chat-conversation-about')).toContainText('Kept in');
  await expect(heading.locator('.chat-conversation-about')).toContainText('Continues');
});
