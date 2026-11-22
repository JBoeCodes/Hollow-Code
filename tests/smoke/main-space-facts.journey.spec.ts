import { expect, test } from './fixtures/test';

async function main(page: import('@playwright/test').Page, demo = true) {
  await page.addInitScript(() => localStorage.setItem('mockMainAgentStatus', JSON.stringify({
    spentTodayUsd: 0, askedTodayUsd: 0, wakeSkipped: null, dailyCapUsd: 100,
    checkIns: false, lastCheckIn: null, nextCheckIn: null, checkInsToday: 0,
    suggestions: { answered: 3, taken: 2, declinedForGood: 1, takenShare: 2 / 3 }, actOnOwn: true,
  })));
  await page.goto(demo ? '/?mock=1&demo=1' : '/?mock=1');
  await page.locator('.top-bar').getByRole('button', { name: /^(COO|Back from the COO) ·/ }).click();
  await expect(page.locator('.work-conversation')).toBeVisible();
}

/** What the store holds as `actOnOwn` (the day's text is Settings → Background work → COO now). */
async function status(page: import('@playwright/test').Page, actOnOwn: boolean) {
  await expect.poll(() => page.evaluate(async () => {
    const p = '/src/store/mainAgent.ts';
    return (await import(/* @vite-ignore */ p)).mainAgentStatus()?.actOnOwn;
  })).toBe(actOnOwn);
}

/** The top bar's Actions for COO (11-11: it holds what the removed header band's More held). */
async function more(page: import('@playwright/test').Page, item: string | RegExp) {
  await page.locator('.top-bar').getByRole('button', { name: 'Actions for COO', exact: true }).click();
  return page.getByRole('menuitem', typeof item === 'string' ? { name: item, exact: true } : { name: item });
}

test('the COO page has no header band and its chat starts at the top', async ({ page }) => {
  await main(page);
  await expect(page.locator('.main-space')).toHaveCount(0);
  await expect(page.locator('.main-space-head')).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'COO', exact: true, level: 2 })).toHaveCount(0);
  await expect(page.locator('.work-conversation').getByRole('button', { name: 'More for the COO', exact: true })).toHaveCount(0);
  // Its actions are in the top bar's menu.
  for (const name of [/^Actions it took on its own ·/, 'Change hard stop', 'Turn check-ins on in Settings', 'How check-ins run']) {
    await expect(await more(page, name)).toBeVisible();
    await page.keyboard.press('Escape');
  }
  const composer = await page.locator('.work-conversation .chat-composer').boundingBox();
  expect(composer).not.toBeNull();
  expect(composer!.y + composer!.height).toBeLessThanOrEqual(page.viewportSize()!.height);
});

test('Main More routes action recovery', async ({ page }) => {
  await main(page, false);
  await expect(await more(page, 'Actions it took on its own · 0 recent')).toBeVisible();
  await page.keyboard.press('Escape');
  await page.evaluate(async () => {
    const apiPath = '/src/api.ts', storePath = '/src/store/mainAgent.ts';
    const { api } = await import(/* @vite-ignore */ apiPath);
    const store = await import(/* @vite-ignore */ storePath);
    api.mainAgentActivity = async () => { throw new Error('synthetic actions failure'); };
    store.refreshMainAgentChildren(0);
  });
  await expect.poll(async () => page.evaluate(async () => {
    const p = '/src/store/mainAgent.ts';
    return (await import(/* @vite-ignore */ p)).mainAgentActsRead();
  })).toBe('error');
  await (await more(page, 'Actions it took on its own · Unavailable')).click();
  const acts = page.getByRole('region', { name: 'COO actions', exact: true });
  await expect(acts.getByRole('button', { name: 'Retry actions' })).toBeVisible();
  await expect(acts).toContainText('Actions are unavailable.');
});

test('Main action history keeps dates and full summaries at its single destination', async ({ page }) => {
  await page.route('**/src/api.ts', async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: `${await response.text()}\napi.mainAgentActivity = async () => [{at:'2026-10-27T05:30:00Z',action:'task.create',summary:'A complete recorded action summary.'}];` });
  });
  await main(page);
  await (await more(page, 'Actions it took on its own · 1 recent')).click();
  const acts = page.getByRole('region', { name: 'COO actions', exact:true });
  await expect(acts).toContainText('A complete recorded action summary.');
  await expect(acts.locator('time')).toHaveAttribute('datetime', '2026-10-27T05:30:00Z');
});

test('Main Actions reveals its destination and gives the disclosure keyboard focus', async ({ page }) => {
  await main(page);
  await (await more(page, /^Actions it took on its own ·/)).click();
  const summary = page.locator('.main-action-history > summary');
  await expect(summary).toBeVisible();
  if (process.env.HOLLOW_A9_CAPTURE) await page.screenshot({ path: process.env.HOLLOW_A9_CAPTURE });
  await expect(summary).toBeFocused();
  const box = await summary.boundingBox();
  expect(box!.y).toBeGreaterThanOrEqual(0);
  expect(box!.y + box!.height).toBeLessThanOrEqual(page.viewportSize()!.height);
  await page.keyboard.press('Tab');
  await expect(page.getByRole('region', { name: 'COO actions', exact: true })).toBeFocused();
});

test('missing standing orders offer defaults only after a click', async ({ page }) => {
  await page.route('**/src/api.ts', async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: `${await response.text()}\nconst readMainWorkspace = api.readWorkspace; api.readWorkspace = async (...args) => { const d = await readMainWorkspace(...args); if (args[0] === 'main' && !window.createdStandingOrders) d.files = d.files.filter(f => f.name !== 'STANDING_ORDERS.md'); return d; }; window.createdStandingOrders = 0; const createDefaultOrders = api.createMainStandingOrders; api.createMainStandingOrders = async () => { window.createdStandingOrders++; return createDefaultOrders(); };` });
  });
  await main(page);
  const create = await more(page, 'Create default standing orders');
  await expect(create).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as { createdStandingOrders: number }).createdStandingOrders)).toBe(0);
  await create.click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { createdStandingOrders: number }).createdStandingOrders)).toBe(1);
  // Once the file exists, More edits it instead.
  await expect(await more(page, 'Edit standing orders')).toBeVisible();
  await expect(page.getByRole('menuitem', { name: 'Create default standing orders', exact: true })).toHaveCount(0);
});

test('focus refreshes the Main status without a queue event', async ({ page }) => {
  await main(page);
  await expect.poll(() => page.evaluate(async () => {
    const p = '/src/store/mainAgent.ts';
    return (await import(/* @vite-ignore */ p)).mainAgentStatus()?.actOnOwn;
  })).toBe(true);
  await page.evaluate(async () => {
    const p = '/src/api.ts';
    const { api } = await import(/* @vite-ignore */ p);
    api.mainAgentChildren = async () => { throw new Error('synthetic children failure'); };
    const status = JSON.parse(localStorage.getItem('mockMainAgentStatus')!);
    localStorage.setItem('mockMainAgentStatus', JSON.stringify({ ...status, actOnOwn: false }));
    window.dispatchEvent(new Event('focus'));
  });
  await expect.poll(() => page.evaluate(async () => {
    const p = '/src/store/mainAgent.ts';
    return (await import(/* @vite-ignore */ p)).mainAgentStatus()?.actOnOwn;
  })).toBe(false);
});

test('a failed refresh keeps the last status and an older response cannot replace a newer one', async ({ page }) => {
  await main(page);
  await status(page, true);
  await page.evaluate(async () => {
    const apiPath = '/src/api.ts', storePath = '/src/store/mainAgent.ts';
    const { api } = await import(/* @vite-ignore */ apiPath);
    const store = await import(/* @vite-ignore */ storePath);
    api.mainAgentStatus = async () => { throw new Error('synthetic refresh failure'); };
    await store.refreshMainAgentStatus();
  });
  await expect.poll(() => page.evaluate(async () => { const p = '/src/store/mainAgent.ts'; return (await import(/* @vite-ignore */ p)).mainAgentStatusError(); })).toContain('synthetic refresh failure');
  await status(page, true);
  await page.evaluate(async () => {
    const apiPath = '/src/api.ts', storePath = '/src/store/mainAgent.ts';
    const { api } = await import(/* @vite-ignore */ apiPath);
    const store = await import(/* @vite-ignore */ storePath);
    const original = store.mainAgentStatus();
    let release;
    api.mainAgentStatus = () => new Promise(resolve => { release = resolve; });
    const older = store.refreshMainAgentStatus();
    api.mainAgentStatus = async () => ({ ...original, actOnOwn: false });
    await store.refreshMainAgentStatus();
    release(original);
    await older;
  });
  await status(page, false);
});

test('a saved Main setting refreshes its status', async ({ page }) => {
  await main(page, false);
  await status(page, true);
  const calls = await page.evaluate(async () => {
    const path = '/src/api.ts';
    const { api } = await import(/* @vite-ignore */ path);
    const old = JSON.parse(localStorage.getItem('mockMainAgentStatus')!);
    localStorage.setItem('mockMainAgentSettings', JSON.stringify({ checkIns: false, intervalMin: 30, startHour: 8, endHour: 18, weekdaysOnly: true, dailyCapUsd: 100, actOnOwn: true, swarmMax: 25, swarmModel: 'claude-haiku-5-5', swarmConcurrency: 6 }));
    const settings = await api.getMainAgentSettings();
    let reads = 0;
    api.mainAgentStatus = async () => { reads++; return { ...old, actOnOwn: false }; };
    await api.setMainAgentSettings({ ...settings, actOnOwn: false });
    return reads;
  });
  expect(calls).toBe(1);
  await status(page, false);
});

test('the next check-in refreshes once without a queue or focus event', async ({ page }) => {
  await main(page, false);
  await status(page, true);
  await page.clock.install();
  await page.evaluate(async () => {
    const apiPath = '/src/api.ts', storePath = '/src/store/mainAgent.ts';
    const { api } = await import(/* @vite-ignore */ apiPath);
    const store = await import(/* @vite-ignore */ storePath);
    const old = store.mainAgentStatus();
    (window as any).timerStatusReads = 0;
    api.mainAgentStatus = async () => { (window as any).timerStatusReads++; return { ...old, actOnOwn: false, nextCheckIn: null }; };
    store.setMainAgentStatus({ ...old, nextCheckIn: new Date(Date.now() + 1000).toISOString() });
  });
  expect(await page.evaluate(() => (window as any).timerStatusReads)).toBe(0);
  await page.clock.fastForward(2000);
  await status(page, false);
  expect(await page.evaluate(() => (window as any).timerStatusReads)).toBe(1);
  await page.clock.fastForward(10000);
  expect(await page.evaluate(() => (window as any).timerStatusReads)).toBe(1);
});
