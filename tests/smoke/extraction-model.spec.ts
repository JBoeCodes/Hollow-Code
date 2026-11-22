// Settings → Advanced → Hollow’s own model calls → Extraction model (overnight A11), against
// the mock: empty is the default (Haiku 5.5 on Claude); a chosen id saves,
// survives a reload, and "Use the default" clears it. An agent's change
// (`app-changed`, overnight H6) shows in an open Settings without reopening it,
// and the person's save wins over a re-read still in flight (H6 follow-up).
import { expect, test, type Page } from './fixtures/test';
import { choose, openAdvancedGroup } from './choose';

async function openDefaults(page: Page) {
  await page.getByRole('button', { name: /Settings/ }).first().click();
  // Extraction model sits in Advanced's closed model-calls group (Settings PR 3).
  await openSection(page, 'advanced', /^Advanced$/);
  await openAdvancedGroup(page, 'model-calls');
}

async function openSection(page: Page, id: string, name: RegExp) {
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), id);
  } else {
    await page.locator('.settings-index').getByRole('button', { name }).click();
  }
}

/** What `app_broker::execute` emits after an agent's action; how many listened. */
async function agentChanged(page: Page, action: string, area: string) {
  return page.evaluate(([action, area]) => (window as unknown as { __mockEmit: (e: string, p: unknown) => number }).__mockEmit('app-changed', { action, area }), [action, area]);
}

test('the extraction model saves, survives a reload and goes back to the default', async ({ page }) => {
  await page.goto('/?mock=1');
  await openDefaults(page);
  const input = page.getByLabel('Extraction model', { exact: true });
  await expect(input).toHaveValue('');
  const row = page.locator('.settings-row').filter({ has: input });
  await expect(row).toContainText('Now: Claude asks claude-haiku-5-5, the default');

  await input.fill('claude-sonnet-5-5');
  await input.press('Enter');
  await input.blur();
  await expect(row).toContainText('Now: Claude asks claude-sonnet-5-5 (chosen here)');
  expect(await page.evaluate(() => localStorage.getItem('mockExtractionModel'))).toBe('claude-sonnet-5-5');

  await page.reload();
  await openDefaults(page);
  await expect(page.getByLabel('Extraction model', { exact: true })).toHaveValue('claude-sonnet-5-5');
  await page.getByRole('button', { name: 'Use the default' }).click();
  await expect(page.getByLabel('Extraction model', { exact: true })).toHaveValue('');
  expect(await page.evaluate(() => localStorage.getItem('mockExtractionModel'))).toBe('');
  expect(await page.evaluate(() => (window as unknown as { __mockApiMisses?: string[] }).__mockApiMisses ?? [])).toEqual([]);
});

test('an agent changing the extraction model or Learning suggestions shows in open Settings (overnight H6)', async ({ page }) => {
  await page.goto('/?mock=1');
  await openDefaults(page);
  const input = page.getByLabel('Extraction model', { exact: true });
  await expect(input).toHaveValue('');

  // An agent sets it through Hollow app tools: the backend saves, then emits app-changed.
  await page.evaluate(() => localStorage.setItem('mockExtractionModel', 'claude-sonnet-5-5'));
  expect(await agentChanged(page, 'settings.extraction.set_extraction_settings', 'settings.extraction')).toBeGreaterThan(0);
  await expect(input).toHaveValue('claude-sonnet-5-5');
  await expect(page.locator('.settings-row').filter({ has: input })).toContainText('Now: Claude asks claude-sonnet-5-5 (chosen here)');

  // Retitle new chats lives in Chat (a section again since Settings PR 3)
  // and follows the same action there.
  await openSection(page, 'chat', /^Chat$/);
  const retitle = page.getByRole('checkbox', { name: 'Retitle new chats' });
  await expect(retitle).not.toBeChecked();
  await page.evaluate(() => localStorage.setItem('mockRetitleChats', '1'));
  expect(await agentChanged(page, 'settings.extraction.set_extraction_settings', 'settings.extraction')).toBeGreaterThan(0);
  await expect(retitle).toBeChecked();

  // `learning` was Memory's own section; it lands in Personalization now.
  await openSection(page, 'personalization', /^Personalization$/);
  const suggestions = page.getByRole('group', { name: 'How many learnings to suggest', exact: true });
  await expect(suggestions.getByRole('button', { name: 'Important only' })).toHaveAttribute('aria-pressed', 'true');
  await page.evaluate(() => localStorage.setItem('mockLearningSuggestions', 'all'));
  await agentChanged(page, 'learning.set_learning_suggestions', 'learning');
  await expect(suggestions.getByRole('button', { name: 'All', exact: true })).toHaveAttribute('aria-pressed', 'true');
  // change_setting learning.suggestions arrives as settings.change.
  await page.evaluate(() => localStorage.setItem('mockLearningSuggestions', 'off'));
  await agentChanged(page, 'settings.change', 'settings');
  await expect(suggestions.getByRole('button', { name: 'Off', exact: true })).toHaveAttribute('aria-pressed', 'true');
  expect(await page.evaluate(() => (window as unknown as { __mockApiMisses?: string[] }).__mockApiMisses ?? [])).toEqual([]);
});

test('a save wins over an agent-triggered re-read that answers after it (H6 follow-up)', async ({ page }) => {
  type Held = { command: string; release: Promise<unknown>; delivered?: boolean };
  type W = { __mockHoldOnce?: Held; __held?: Held; __releaseHeld?: () => void; __mockApiCalls?: Record<string, number> };
  await page.goto('/?mock=1');
  await openDefaults(page);
  const input = page.getByLabel('Extraction model', { exact: true });
  const row = page.locator('.settings-row').filter({ has: input });
  await expect(input).toHaveValue('');
  const readsBefore = await page.evaluate(() => (window as unknown as W).__mockApiCalls?.get_extraction_settings ?? 0);

  // An agent sets Opus; the re-read it triggers reads that and is held.
  await page.evaluate(() => {
    const w = window as unknown as W;
    let release!: () => void;
    const held: Held = { command: 'get_extraction_settings', release: new Promise<void>(r => { release = r; }) };
    w.__mockHoldOnce = held; w.__held = held; w.__releaseHeld = release;
    localStorage.setItem('mockExtractionModel', 'claude-opus-5-5');
  });
  await agentChanged(page, 'settings.extraction.set_extraction_settings', 'settings.extraction');
  await expect.poll(() => page.evaluate(() => (window as unknown as W).__mockApiCalls?.get_extraction_settings ?? 0)).toBeGreaterThan(readsBefore);

  // Meanwhile the person saves Sonnet.
  await input.fill('claude-sonnet-5-5');
  await input.press('Enter');
  await input.blur();
  await expect(row).toContainText('Now: Claude asks claude-sonnet-5-5 (chosen here)');

  // The held re-read answers Opus after the save: the saved value stays.
  await page.evaluate(() => (window as unknown as W).__releaseHeld!());
  await expect.poll(() => page.evaluate(() => (window as unknown as W).__held?.delivered === true)).toBe(true);
  await expect(input).toHaveValue('claude-sonnet-5-5');
  await expect(row).toContainText('Now: Claude asks claude-sonnet-5-5 (chosen here)');
  expect(await page.evaluate(() => localStorage.getItem('mockExtractionModel'))).toBe('claude-sonnet-5-5');
});
