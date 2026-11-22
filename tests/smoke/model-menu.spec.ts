import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

/**
 * The composer's model menu: every installed agent's models in one searchable
 * list, the current one checked; picking another agent's model switches the
 * agent; Effort sets this chat's own effort (Settings PR 7), never the app default.
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
  await page.locator('#settings-group-this-mac > summary').click(); // History & recovery: Advanced → This Mac
  await page.getByRole('button', { name: 'Open Library, History', exact: true }).click();
  await page.locator('.library-tabs').getByRole('button', { name: 'Conversations', exact: true }).click(); // Settings PR 4 cut the Conversation history button
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toBeVisible();
}

test('Enter on a focused model chooses that model instead of the mouse highlight', async ({ page }) => {
  await openChat(page);
  await page.getByRole('button', { name: /^Model:/ }).click();
  const target = page.getByRole('option', { name: /GPT-6-Sol/ }).first();
  await expect(target).toBeVisible();
  await page.getByRole('option', { name: /Claude default/ }).hover();
  await target.focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('button', { name: /^Model: Codex/ })).toBeVisible();
});

test('catalog failure has a keyboard Refresh outside the listbox and an Agent updates route', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockCodexCatalogError', 'once'));
  await openChat(page);
  const trigger = page.getByRole('button', { name: /^Model:/ });
  const before = await trigger.getAttribute('aria-label');
  await trigger.click();
  const health = page.getByRole('group', { name: 'Codex status', exact: true });
  await expect(health).toContainText('Synthetic Codex catalog unavailable');
  await expect(page.getByRole('listbox').getByRole('button', { name: 'Refresh', exact: true })).toHaveCount(0);
  await page.getByRole('listbox', { name: 'Codex models', exact: true }).getByRole('option').last().focus();
  await health.getByRole('button', { name: 'Refresh', exact: true }).focus();
  await page.keyboard.press('ArrowUp');
  await expect(health.getByRole('button', { name: 'Refresh', exact: true })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.locator('.model-menu-pop')).toBeVisible();
  await expect(trigger).toHaveAttribute('aria-label', before!);
  await expect(page.getByRole('option', { name: /GPT-6-Sol/ })).toBeVisible();
  await expect(health).not.toContainText('Synthetic Codex catalog unavailable');
  await health.getByRole('button', { name: 'Agent updates', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('heading', { name: 'Agent updates', exact: true })).toBeVisible();
});

for (const height of [480, 600]) test(`ordinary model search stays below the title bar at 720x${height}`, async ({ page }) => {
  await page.setViewportSize({ width: 720, height });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.addInitScript(height => {
    localStorage.setItem('hollow.studio.textScale', height === 600 ? '125' : '100');
    localStorage.setItem('mockCodexCatalogError', 'once');
  }, height);
  await openChat(page);
  await page.getByRole('button', { name: /^Model:/ }).click();
  const search = page.getByRole('textbox', { name: 'Search models' });
  await expect.poll(async () => (await search.boundingBox())?.y ?? -1).toBeGreaterThanOrEqual(44);
  await expect.poll(() => search.evaluate(el => {
    const box = el.getBoundingClientRect();
    return document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2) === el;
  })).toBe(true);
  await search.fill('codex');
  const refresh = page.getByRole('group', { name: 'Codex status', exact: true }).getByRole('button', { name: 'Refresh', exact: true });
  await refresh.focus();
  await expect(refresh).toBeInViewport();
});

test('model arrows start from focus even after another row is hovered', async ({ page }) => {
  await openChat(page);
  await page.getByRole('button', { name: /^Model:/ }).click();
  const models = page.getByRole('group', { name: 'Models', exact: true }).getByRole('option');
  await expect(models.nth(2)).toBeVisible();
  await models.nth(1).focus();
  await models.nth(0).hover();
  await page.keyboard.press('ArrowDown');
  await expect(models.nth(2)).toBeFocused();
});

// From the search box the arrows move a highlight and focus nothing, so a row
// below the list's edge stayed out of sight and Enter still chose it (a Codex
// read; the menu's height cap is what puts rows there).
test('ArrowDown from the search brings the highlighted model into view in a short window', async ({ page }) => {
  await page.setViewportSize({ width: 720, height: 480 });
  await openChat(page);
  await page.getByRole('button', { name: /^Model:/ }).click();
  const search = page.getByRole('textbox', { name: 'Search models' });
  await expect(search).toBeVisible();
  await search.focus();
  const rows = page.locator('.model-menu-list [role="option"]');
  const active = page.locator('.model-menu-row.active');
  const count = await rows.count();
  expect(count).toBeGreaterThan(6);
  for (let i = 0; i < count; i++) await page.keyboard.press('ArrowDown');
  await expect(active).toHaveCount(1);
  await expect(active).toBeInViewport({ ratio: 1 });
  // The list scrolled to get there (the nearest scrolling box of a row).
  const scrolled = () => rows.first().evaluate(el => {
    let box = el.parentElement;
    while (box && !/auto|scroll/.test(getComputedStyle(box).overflowY)) box = box.parentElement;
    return box ? box.scrollTop : -1;
  });
  expect(await scrolled()).toBeGreaterThan(0);
  // And back up: the first row comes back into view.
  for (let i = 0; i < count; i++) await page.keyboard.press('ArrowUp');
  await expect(rows.first()).toBeInViewport({ ratio: 1 });
});

test('Effort and Canvases arrow keys stay outside the model list', async ({ page }) => {
  await openChat(page);
  await page.getByRole('button', { name: /^Model:/ }).click();
  for (const setting of ['Effort', 'Canvases']) {
    await page.getByRole('button', { name: new RegExp(`^${setting}`) }).click();
    const list = page.getByRole('listbox', { name: setting === 'Effort' ? 'Claude effort' : 'Canvases in this chat' });
    const first = list.getByRole('option').first();
    await first.focus();
    // 2026-11-01: arrows move within this list (Home/End too), never into
    // the model list above it.
    await page.keyboard.press('ArrowDown');
    await expect(list.getByRole('option').nth(1)).toBeFocused();
    await page.keyboard.press('Home');
    await expect(first).toBeFocused();
    await page.keyboard.press('ArrowUp');
    await expect(first).toBeFocused();
    expect(await page.evaluate(() => !!document.activeElement?.closest('.model-menu-list'))).toBe(false);
  }
});

test('one menu lists every agent, switches provider and sets effort', async ({ page }, info) => {
  await openChat(page);
  const trigger = page.getByRole('button', { name: /^Model:/ });
  await trigger.click();

  const list = page.getByRole('group', { name: 'Models', exact: true });
  await expect(list.getByRole('group', { name: 'Claude', exact: true })).toBeVisible();
  await expect(list.getByRole('group', { name: 'Codex', exact: true })).toBeVisible();
  await expect(list.getByRole('group', { name: 'Grok', exact: true })).toBeVisible();
  await expect(list.getByRole('group', { name: 'Gemini' })).toHaveCount(0);
  await expect(list.getByRole('option', { selected: true })).toHaveCount(1);
  await page.waitForTimeout(400); // let the open animation finish
  const bg = await page.locator('.model-menu-pop').evaluate(el => getComputedStyle(el).backgroundColor);
  expect(bg).not.toBe('rgba(0, 0, 0, 0)');
  await info.attach('model-menu', { body: await page.screenshot(), contentType: 'image/png' });

  // Search narrows the list; another agent's model switches the agent.
  await page.getByRole('textbox', { name: 'Search models', exact: true }).fill('grok-4.6');
  await expect(list.getByRole('option')).toHaveCount(1);
  await list.getByRole('option', { name: /grok-4\.6/ }).click();
  await expect(trigger).toHaveAttribute('aria-label', /^Model: Grok · grok-4\.6 · Permissions: /);
  expect(await page.evaluate(() => localStorage.getItem('chatAgent'))).toBe('grok');

  // Back to Claude, then set its effort from the menu.
  await trigger.click();
  await page.getByRole('option', { name: /Opus/ }).click();
  await expect(trigger).toHaveAttribute('aria-label', /^Model: Claude · opus · Permissions: /);
  await trigger.click();
  await page.getByRole('button', { name: /^Effort/ }).click();
  await page.getByRole('listbox', { name: 'Claude effort' }).getByRole('option', { name: 'High', exact: true }).click();
  // Settings PR 7: the level is this chat's own, kept in localStorage; the
  // app default (set_agent_settings) is untouched.
  expect(await page.evaluate(() => (window as any).__mockApiCalls?.set_agent_settings ?? 0)).toBe(0);
  await expect(page.getByRole('button', { name: /^Effort/ })).toContainText('High');

  // Track U: Ultracode is the Effort list's last option, for this chat only.
  // It turns on without saving a level, shows on the trigger, and a saved
  // level turns it off again.
  const saves = () => page.evaluate(() => (window as any).__mockApiCalls?.set_agent_settings ?? 0);
  const savedBefore = await saves();
  // Picking a level leaves the menu open.
  await expect(page.locator('.model-menu-pop')).toBeVisible();
  await page.getByRole('button', { name: /^Effort/ }).click();
  const efforts = page.getByRole('listbox', { name: 'Claude effort' });
  await expect(efforts.getByRole('option')).toHaveText([/Default/, /Low/, /Medium/, /High/, /Extra high/, /Max/, /Ultracode/]);
  await efforts.getByRole('option', { name: /Ultracode/ }).click();
  // The conversation's own switch (P1-10): no global flag that follows every
  // new chat.
  expect(await page.evaluate(() => localStorage.getItem('chatUltracode'))).toBeNull();
  expect(await saves()).toBe(savedBefore);
  await expect(trigger).toContainText('Ultracode');
  await page.getByRole('button', { name: /^Effort/ }).click();
  await expect(page.getByRole('button', { name: /^Effort/ })).toContainText('Ultracode');
  await expect(efforts.getByRole('option', { selected: true })).toHaveText(/Ultracode/);
  await info.attach('ultracode-in-effort', { body: await page.screenshot(), contentType: 'image/png' });
  await page.screenshot({ path: process.env.ULTRACODE_SHOT ?? info.outputPath('ultracode.png') });
  await efforts.getByRole('option', { name: 'High', exact: true }).click();
  expect(await page.evaluate(() => localStorage.getItem('chatUltracode'))).toBeNull();
  await expect(trigger).not.toContainText('Ultracode');

  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});

test('failed status retains its labelled last-known value and retries without changing models', async ({ page }) => {
  await openChat(page);
  const trigger = page.getByRole('button', { name: /^Model:/ });
  await trigger.click();
  const health = page.getByRole('group', { name: 'Codex status', exact: true });
  await expect(health).toContainText('Update available');
  await page.keyboard.press('Escape');
  await expect(page.getByRole('textbox', { name: 'Search models' })).toHaveCount(0);
  await page.evaluate(async () => {
    const path = '/src/api.ts';
    const { api } = await import(/* @vite-ignore */ path);
    const original = api.agentCliStatus;
    let reject = true;
    api.agentCliStatus = (...args: unknown[]) => {
      if (reject) { reject = false; return Promise.reject(new Error('Synthetic status read failed')); }
      return original(...args);
    };
  });
  await trigger.click();
  await expect(page.locator('.model-menu-pop')).toContainText('last known value');
  await expect(health).toContainText('Update available');
  await page.getByRole('button', { name: 'Retry agent status', exact: true }).press('Space');
  await expect(page.getByRole('button', { name: 'Retry agent status', exact: true })).toHaveCount(0);
  await expect(trigger).toHaveAccessibleName(/^Model: Claude/);
});

test('failed turn exposes Agent updates without resending or consuming the draft', async ({ page }) => {
  await openChat(page);
  await page.evaluate(async () => {
    const path = '/src/store/chatRuntime.ts';
    const s = await import(/* @vite-ignore */ path);
    s.setChatTurns([{ role: 'user', text: 'Original request', tools: [] },
      { role: 'assistant', text: 'Codex ended the turn without saying why.', status: 'error', tools: [] }]);
  });
  await page.getByRole('textbox', { name: 'Chat message' }).fill('Unsent draft');
  await page.getByRole('group', { name: 'The last message failed' }).getByRole('button', { name: 'Agent updates', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Agent updates', exact: true })).toBeVisible();
  expect(await page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBe(0);
  expect(await page.evaluate(async () => {
    const path = '/src/store/chatRuntime.ts'; return (await import(/* @vite-ignore */ path)).chatInput();
  })).toBe('Unsent draft');
});

for (const action of ['Send', 'Continue']) test(`${action} unavailable notice retains the draft and opens Agent updates`, async ({ page }) => {
  await openChat(page);
  await page.evaluate(async action => {
    const path = '/src/store/chatRuntime.ts';
    const s = await import(/* @vite-ignore */ path);
    s.setAgentSettingsError('Synthetic agent unavailable');
    if (action === 'Continue') {
      s.setChatSessionId('native-synthetic');
      s.setChatTurns([{ role: 'user', text: 'Original request', tools: [] },
        { role: 'assistant', text: 'Interrupted', status: 'interrupted', tools: [] }]);
    }
  }, action);
  await page.getByRole('textbox', { name: 'Chat message' }).fill('Keep this draft');
  if (action === 'Send') await page.getByRole('textbox', { name: 'Chat message' }).press('Enter');
  else await page.getByRole('button', { name: 'Continue', exact: true }).click();
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toHaveValue('Keep this draft');
  await page.getByRole('button', { name: 'Agent updates', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Agent updates', exact: true })).toBeVisible();
  expect(await page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBe(0);
});

// 2026-11-01: Permissions moved into this menu. Its list keeps the keyboard
// the Select it replaced had (review: arrows didn't move, Esc dropped focus).
test('Permissions in the model menu: arrows move, Esc folds onto the row, then closes onto the button', async ({ page }) => {
  await openChat(page);
  const trigger = page.getByRole('button', { name: /^Model: / });
  await expect(trigger).toHaveAccessibleName(/ · Permissions: /);
  await trigger.click();
  const row = page.getByRole('button', { name: /^Permissions: / });
  await row.focus();
  await page.keyboard.press('Enter');
  const list = page.getByRole('listbox', { name: 'Permissions' });
  const options = list.getByRole('option');
  // Claude's own five (per-agent menus, 2026-11-12): Default, Accept edits, Plan, Auto, Bypass permissions.
  await expect(options).toHaveCount(5);
  await options.first().focus();
  await page.keyboard.press('ArrowDown');
  await expect(options.nth(1)).toBeFocused();
  await page.keyboard.press('End');
  await expect(options.last()).toBeFocused();
  await page.keyboard.press('Home');
  await expect(options.first()).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(list).toHaveCount(0);
  await expect(row).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(row).toHaveCount(0);
  await expect(trigger).toBeFocused();
});
