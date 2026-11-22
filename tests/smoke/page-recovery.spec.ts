import { openTasks } from './choose';
import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';
import { showUsageTab } from './usageTab';
import { navigateWaiting } from './waiting';

for (const [name, file, exported, main, tab] of [
  ['Review', 'WaitingOnYouPanel', 'default', 'Review', 'All waiting'],
] as const) test(`isolated ${name} render errors retry locally with destination tabs intact`, async ({ page, pageErrors }) => {
  await page.route(`**/src/components/${file}.tsx*`, route => route.fulfill({ contentType: 'application/javascript', body: `
    export ${exported === 'default' ? 'default ' : ''}function ${file}() {
      window.__isolatedAttempts = (window.__isolatedAttempts || 0) + 1;
      if (window.__isolatedAttempts === 1) throw new Error('Synthetic ${name} isolated failure');
      const el = document.createElement('p'); el.textContent = 'Synthetic ${name} recovered'; return el;
    }` }));
  await page.goto('/?mock=1');
  const nav = page.getByRole('navigation', { name: 'Main', exact: true });
  // The full Needs you view (was Review) opens from the bell's See all (U3).
  if (main === 'Review') await navigateWaiting(page);
  else await nav.getByRole('button', { name: main, exact: true }).click();
  const root = page.locator('.canvas-split > .page');
  if (tab !== 'All waiting') await root.getByRole('button', { name: tab === 'Incoming' ? /^Incoming(?:\s|$)/ : tab, exact: tab !== 'Incoming' }).click();
  const alert = root.locator('.view-recovery');
  await expect(alert).toContainText(`${name} could not load`);
  await expect(alert).toContainText(`Synthetic ${name} isolated failure`);
  await expect(root.locator('.destination-views')).toBeVisible();
  await expect(nav).toBeVisible();
  await alert.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(root.getByText(`Synthetic ${name} recovered`, { exact: true })).toBeVisible();
  await expect(alert).toHaveCount(0);
  expect(pageErrors.seen.filter(e => e.includes(`Synthetic ${name} isolated`))).toEqual([]);
});

async function expectSettingsNavigation(page: Page) {
  await expect(page.locator('.settings-index')).toBeAttached();
  // settings-graph.css switches to the compact selector at max-width: 860px.
  const navigation = (page.viewportSize()?.width ?? 1440) <= 860
    ? page.getByRole('combobox', { name: 'Settings section', exact: true })
    : page.locator('.settings-index');
  await expect(navigation).toBeVisible();
}

/** Open section `id` and return it. */
async function settings(page: Page, id: string, name: RegExp) {
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  if ((page.viewportSize()?.width ?? 1440) <= 860) await choose(page.getByRole('combobox', { name: 'Settings section', exact: true }), id);
  else await page.locator('.settings-index').getByRole('button', { name }).click();
  return page.locator(`#settings-${id}`);
}

for (const way of ['reload', 'leave and return']) test(`a throwing Tasks page recovers through ${way} without losing the shell`, async ({ page, pageErrors }) => {
  await page.route('**/src/pages/Tasks.tsx*', route => route.fulfill({ contentType: 'application/javascript', body: `
    export function Tasks() {
      window.__renderAttempts = (window.__renderAttempts || 0) + 1;
      if (window.__renderAttempts === 1) throw new Error('Synthetic Tasks render failure');
      const el = document.createElement('p'); el.textContent = 'Synthetic Tasks recovered'; return el;
    }` }));
  await page.goto('/?mock=1');
  const nav = page.getByRole('navigation', { name: 'Main', exact: true });
  await openTasks(page);
  await expect(page.getByRole('alert')).toContainText('Tasks');
  await expect(page.getByRole('alert')).toContainText('Synthetic Tasks render failure');
  await expect(page.getByRole('button', { name: 'Copy details', exact: true })).toBeVisible();
  await page.evaluate(() => Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async (text: string) => { (window as any).__copiedDetails = text; } } }));
  await page.getByRole('button', { name: 'Copy details', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__copiedDetails)).toContain('Synthetic Tasks render failure');
  await expect(nav).toBeVisible();
  if (way === 'reload') await page.getByRole('button', { name: 'Reload page', exact: true }).click();
  else {
    await nav.getByRole('button', { name: 'Projects', exact: true }).click();
    await expect(page.getByRole('alert')).toHaveCount(0);
    await openTasks(page);
  }
  await expect(page.getByText('Synthetic Tasks recovered', { exact: true })).toBeVisible();
  expect(pageErrors.seen.filter(e => e.includes('Synthetic Tasks'))).toEqual([]);
});

test('a throwing lazy Settings section retries locally while its index stays mounted', async ({ page, pageErrors }) => {
  await page.route('**/src/settings/UsagePanel.tsx*', route => route.fulfill({ contentType: 'application/javascript', body: `
    export function UsagePanel() {
      window.__sectionAttempts = (window.__sectionAttempts || 0) + 1;
      if (window.__sectionAttempts === 1) throw new Error('Synthetic Usage render failure');
      const el = document.createElement('p'); el.textContent = 'Synthetic Usage recovered'; return el;
    }` }));
  await page.goto('/?mock=1');
  const section = await settings(page, 'usage', /^Usage/);
  await expect(section.getByRole('alert')).toContainText('Synthetic Usage render failure');
  await expectSettingsNavigation(page);
  await section.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(section.getByText('Synthetic Usage recovered')).toBeVisible();
  expect(pageErrors.seen.filter(e => e.includes('Synthetic Usage'))).toEqual([]);
});

for (const method of ['valueSummary', 'getValueBaselines', 'agentCliStatus', 'listHistoryRuns', 'historyStorageStatus']) {
  test(`Settings keeps other blocks and retries a failed ${method} read`, async ({ page, pageErrors }) => {
    await page.goto('/?mock=1');
    await page.evaluate(async method => {
      const path = '/src/api.ts'; const { api } = await import(/* @vite-ignore */ path);
      const original = api[method]; let fail = true;
      api[method] = (...args: unknown[]) => {
        if (fail) { fail = false; return Promise.reject(new Error(`Synthetic ${method} read failure`)); }
        return original(...args);
      };
    }, method);
    const agent = method === 'agentCliStatus';
    const history = method === 'listHistoryRuns' || method === 'historyStorageStatus';
    // Agent updates sit in Accounts and Value this month on Usage's All agents tab
    // (Settings PR 8); History & recovery is Library's History tab (overnight 11-12 L2).
    let section;
    if (history) {
      await page.getByRole('button', { name: 'Library', exact: true }).click();
      await page.getByRole('navigation', { name: 'Library view' }).getByRole('button', { name: 'History', exact: true }).click();
      section = page.locator('.history-recovery');
    } else {
      section = await settings(page, agent ? 'accounts' : 'usage', agent ? /^Accounts$/ : /^Usage$/);
      if (!agent) await showUsageTab(page, 'All agents');
    }
    await expect(section.getByText(new RegExp(`Synthetic ${method} read failure`))).toBeVisible();
    if (history) await expect(page.getByRole('navigation', { name: 'Library view' })).toBeVisible();
    else await expectSettingsNavigation(page);
    if (!agent && !history) await expect(section.getByRole('heading', { name: 'Value this month', exact: true })).toBeVisible();
    await section.getByRole('button', { name: 'Retry', exact: true }).click();
    await expect(section.getByText(new RegExp(`Synthetic ${method} read failure`))).toHaveCount(0);
    expect(pageErrors.seen.filter(e => e.includes(`Synthetic ${method}`))).toEqual([]);
  });
}

for (const method of ['listLearningProposals','listLearningContext']) test(`Learnings retries ${method} without losing its home`, async ({page,pageErrors}) => {
  await page.goto('/?mock=1');
  await page.evaluate(async method => {
    const path='/src/api.ts'; const {api}=await import(/* @vite-ignore */ path);
    const original=api[method]; let fail=true;
    api[method]=(...args:unknown[])=>{if(fail){fail=false;return Promise.reject(new Error(`Synthetic ${method} read failure`));}return original(...args);};
  },method);
  await page.getByRole('button',{name:'Library',exact:true}).click();
  await page.getByRole('navigation',{name:'Library view'}).getByRole('button',{name:'Learnings',exact:true}).click();
  const home=page.getByRole('region',{name:'Learnings',exact:true});
  await expect(home.getByText(new RegExp(`Synthetic ${method} read failure`))).toBeVisible();
  await expect(page.getByRole('navigation',{name:'Library view'})).toBeVisible();
  await home.getByRole('button',{name:'Try again',exact:true}).click();
  await expect(home.getByText(new RegExp(`Synthetic ${method} read failure`))).toHaveCount(0);
  expect(pageErrors.seen.filter(e=>e.includes(`Synthetic ${method}`))).toEqual([]);
});

test('Learning refetch keeps Library tabs and saved learning mounted', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('button', { name: 'Library', exact: true }).click();
  await page.getByRole('navigation', { name: 'Library view' }).getByRole('button', { name: 'Learnings', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Learnings', exact: true })).toBeVisible();
  await page.evaluate(async () => {
    const path = '/src/api.ts'; const { api } = await import(/* @vite-ignore */ path);
    const w = window as any; w.__index = document.querySelector('.library-tabs'); w.__releaseReads = [];
    for (const key of ['listLearningProposals', 'listLearningContext']) {
      const original = api[key];
      api[key] = (...args: unknown[]) => new Promise(resolve => w.__releaseReads.push(() => original(...args).then(resolve)));
    }
    const store = '/src/reviewInbox.ts'; const { refreshReviewInbox } = await import(/* @vite-ignore */ store);
    refreshReviewInbox();
  });
  await expect.poll(() => page.evaluate(() => (window as any).__releaseReads.length)).toBe(2);
  await expect(page.getByRole('navigation', { name: 'Library view' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Learnings', exact: true })).toBeVisible();
  expect(await page.evaluate(() => (window as any).__index === document.querySelector('.library-tabs'))).toBe(true);
  await page.evaluate(() => (window as any).__releaseReads.forEach((release: () => void) => release()));
});

test('History restore refetch keeps the Library tabs and history list mounted', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('button', { name: 'Library', exact: true }).click();
  await page.getByRole('navigation', { name: 'Library view' }).getByRole('button', { name: 'History', exact: true }).click();
  const section = page.locator('.history-recovery');
  await expect(section.locator('.history-run-row').first()).toBeVisible();
  await page.evaluate(async () => {
    const path = '/src/api.ts'; const { api } = await import(/* @vite-ignore */ path);
    const w = window as any; w.__index = document.querySelector('.library-tabs'); w.__releaseReads = [];
    for (const key of ['listHistoryRuns', 'historyStorageStatus']) {
      const original = api[key];
      api[key] = (...args: unknown[]) => new Promise(resolve => w.__releaseReads.push(() => original(...args).then(resolve)));
    }
  });
  await section.getByRole('button', { name: 'Restore entire run', exact: true }).click();
  await page.getByRole('dialog', { name: 'Restore this entire run?', exact: true }).getByRole('button', { name: 'Restore run', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__releaseReads.length)).toBe(2);
  await expect(page.getByRole('navigation', { name: 'Library view' })).toBeVisible();
  await expect(section.locator('.history-run-row').first()).toBeVisible();
  expect(await page.evaluate(() => (window as any).__index === document.querySelector('.library-tabs'))).toBe(true);
  await page.evaluate(() => (window as any).__releaseReads.forEach((release: () => void) => release()));
});

for (const method of ['usageSummary', 'listRunQueue']) test(`Activity retains its layout and retries ${method} failure`, async ({ page, pageErrors }) => {
  await page.goto('/?mock=1');
  await page.evaluate(async method => {
    const path = '/src/api.ts'; const { api } = await import(/* @vite-ignore */ path);
    const original = api[method]; (window as any).__allowActivityRead = false;
    (window as any).__activityReads = 0;
    api[method] = (...args: unknown[]) => {
      (window as any).__activityReads++;
      if (!(window as any).__allowActivityRead) return Promise.reject(new Error(`Synthetic ${method} read failure`));
      return original(...args);
    };
  }, method);
  await page.getByRole('navigation', { name: 'Projects views' }).getByRole('button', { name: 'Running', exact: true }).click();
  const activity = page.getByRole('region', { name: 'Running', exact: true });
  await expect(activity.getByRole('group', { name: 'Running layout' })).toBeVisible();
  if (method === 'usageSummary') await expect(activity).toContainText('— spent today');
  await expect(activity.getByRole('button', { name: method === 'usageSummary' ? 'Retry spend' : 'Retry run list', exact: true })).toBeVisible();
  const beforeRetry = await page.evaluate(() => { (window as any).__allowActivityRead = true; return (window as any).__activityReads; });
  await activity.getByRole('button', { name: method === 'usageSummary' ? 'Retry spend' : 'Retry run list', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__activityReads)).toBeGreaterThan(beforeRetry);
  await expect(activity.getByRole('button', { name: method === 'usageSummary' ? 'Retry spend' : 'Retry run list', exact: true })).toHaveCount(0);
  expect(pageErrors.seen.filter(e => e.includes(`Synthetic ${method}`))).toEqual([]);
});

// RF-909 (run 13 FIX-EB): Work's tabs render in a root of their own
// (OutsideSuspense), outside the page's error boundary, so a throwing view
// left its tab blank with no way back. Each now fails in place, worded, with
// Retry, while the tabs stay usable. Today, Weekly review and the map were
// cut (overnight 11-08 S3 and S4).
for (const [tab, file] of [['Running', 'MissionControl']] as const) test(`a throwing Work ${tab} tab says so and retries in place`, async ({ page, pageErrors }) => {
  await page.route(`**/src/components/${file}.tsx*`, route => route.fulfill({ contentType: 'application/javascript', body: `
    export function ${file}() {
      window.__tabAttempts = (window.__tabAttempts || 0) + 1;
      if (window.__tabAttempts === 1) throw new Error('Synthetic ${tab} render failure');
      const el = document.createElement('p'); el.textContent = 'Synthetic ${tab} recovered'; return el;
    }` }));
  await page.goto('/?mock=1');
  const views = page.getByRole('navigation', { name: 'Projects views' });
  await (tab === 'Map' ? page.getByRole('region', { name: 'Projects' }) : views).getByRole('button', { name: tab, exact: true }).click();
  const alert = page.locator('.your-work').getByRole('alert');
  await expect(alert).toContainText(`${tab} could not load`);
  await expect(alert).toContainText(`Synthetic ${tab} render failure`);
  await expect(views.getByRole('button', { name: 'Home', exact: true })).toBeVisible();
  await page.locator('.your-work').getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page.getByText(`Synthetic ${tab} recovered`, { exact: true })).toBeVisible();
  await expect(alert).toHaveCount(0);
  expect(pageErrors.seen.filter(e => e.includes(`Synthetic ${tab}`))).toEqual([]);
});

test('a Work tab whose module fails to load says so and the other tabs still work', async ({ page, pageErrors }) => {
  await page.route('**/src/components/MissionControl.tsx*', route => route.fulfill({ contentType: 'application/javascript', body: `throw new Error('Synthetic Running load failure');` }));
  await page.goto('/?mock=1');
  const views = page.getByRole('navigation', { name: 'Projects views' });
  await views.getByRole('button', { name: 'Running', exact: true }).click();
  const alert = page.locator('.your-work').getByRole('alert');
  await expect(alert).toContainText('Running could not load');
  await expect(alert).toContainText('Synthetic Running load failure');
  await views.getByRole('button', { name: 'Home', exact: true }).click();
  await expect(alert).toHaveCount(0);
  await expect(page.getByRole('region', { name: 'Projects', exact: true })).toBeVisible();
  expect(pageErrors.seen.filter(e => e.includes('Synthetic Running'))).toEqual([]);
});
