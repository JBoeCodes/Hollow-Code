import { expect, test, type Page } from './fixtures/test';
import { SETTINGS_SECTION_GROUPS } from '../../src/settingsSections';
import { SETTINGS_PAGE_ENTRIES, pageEntryWhere } from '../../src/settings/pageTargets';

/**
 * P1-21: the Settings shell. Every control the search lists lands in its
 * section; a section switch with unsaved changes asks first; a switch starts
 * at the top with the heading focused and is remembered for the session; the
 * search stays below 860 px and Enter opens its top hit.
 */

const index = (page: Page) => page.getByRole('navigation', { name: 'Settings sections' });
const search = (page: Page) => page.getByRole('searchbox', { name: 'Find a setting' });

async function openSettings(page: Page) {
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).waitFor();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page.locator('.settings-page')).toBeVisible();
}

test('every control the search lists is found in its own section', async ({ page }) => {
  test.setTimeout(300_000);
  // The states in which every row shows: the Main agent's settings (always
  // there in the app; the plain mock has none), Search by meaning on, a
  // connected companion backend and a remote runner.
  await page.addInitScript(() => {
    localStorage.setItem('mockMainAgentSettings', JSON.stringify({ checkIns: false, intervalMin: 30, startHour: 8, endHour: 18, weekdaysOnly: true, dailyCapUsd: 100, actOnOwn: true, swarmMax: 25, swarmModel: 'claude-haiku-5-5', swarmConcurrency: 6 }));
    localStorage.setItem('semanticSearchEnabled', 'true');
    // A remote runner: the rows that choose what runs on it (Runs on, Builds and tests, App builds,
    // Overnight and worker chats) show only once one is set up (overnight 11-07, lane D3).
    localStorage.setItem('mockRemoteRunner', 'runner-host');
    // COO on this Mac (M3a PR2) shows once a paired Mac runs a runner COO or while saved off; Move running
    // chats to the runner when this Mac sleeps (M5 UI) once the runner takes chats or while on. Both saved so here.
    localStorage.setItem('mockAgentSettings', JSON.stringify({ defaultAgent: null, models: {}, cooDispatcher: 'off', moveOnSleep: true }));
    localStorage.setItem('mockCompanionBackend', JSON.stringify({ url: 'https://hollow-companion.example.ts.net', connected: true, lastContactAt: null, lastError: null, mirrorError: null, mirroredTasks: 3, lastPushAt: null, mirroredDocs: 2, docsError: null, docsSupported: true, scope: 'personal', scopeError: null }));
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await openSettings(page);
  const all = SETTINGS_SECTION_GROUPS.flatMap(g => g.items.flatMap(s => (s.controls ?? []).map(c => ({ id: s.id, label: c.label }))));
  // One label, one control: a hit button is named by its label alone.
  const labels = all.map(c => c.label);
  expect(labels.filter((l, i) => labels.indexOf(l) !== i)).toEqual([]);
  const findsEach = async (controls: { id: string; label: string }[]) => {
    for (const { id, label } of controls) {
      // The previous jump's mark lasts 1.8 s; clear it so it can't pass for this one.
      await page.evaluate(() => document.querySelectorAll('.settings-found').forEach(el => el.classList.remove('settings-found')));
      await search(page).fill(label);
      const hit = index(page).getByRole('button', { name: label, exact: true }).and(page.locator('.settings-index-control'));
      await hit.click();
      // Marked by its text, or by the name a control carries only as its aria-label.
      const found = page.locator(`#settings-${id} .settings-found`);
      const named = `[data-settings-control="${label}"]`;
      await expect.soft(found.filter({ hasText: label }).or(found.and(page.locator(named))).or(found.filter({ has: page.locator(named) })), `${id}: "${label}"`).toHaveCount(1, { timeout: 4000 });
    }
  };
  // Updates (U5 PR 2): Update token and Connect Hollow show only while Hollow is not connected
  // to the update server, where Disconnect hides, so they are checked in that state below.
  const DISCONNECTED = ['Update token', 'Connect Hollow'];
  await findsEach(all.filter(c => !DISCONNECTED.includes(c.label)));
  // A setting whose home is another page: listed "in Library" / "in Automations",
  // and choosing it opens that page (Settings plan PR 2). Each entry is checked
  // in turn, coming back to Settings for the next.
  for (const entry of SETTINGS_PAGE_ENTRIES) {
    await search(page).fill(entry.label);
    const hit = index(page).locator(`[data-page-hit="${entry.label}"]`);
    await expect.soft(hit, `${entry.label} is listed`).toContainText(pageEntryWhere(entry));
    await hit.click();
    await expect.soft(page.locator('.settings-page'), `${entry.label} leaves Settings for ${entry.page}`).toBeHidden({ timeout: 4000 });
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await expect(page.locator('.settings-page')).toBeVisible();
  }
  // Not connected to the update server: the token field and Connect Hollow show.
  await page.evaluate(() => localStorage.setItem('mockUpdateConnected', '0'));
  await openSettings(page);
  await findsEach(all.filter(c => DISCONNECTED.includes(c.label)));
});

test('a hit inside the closed Advanced fold opens it and focuses the row', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await openSettings(page);
  await search(page).fill('Index my vault for semantic search');
  await index(page).getByRole('button', { name: 'Index my vault for semantic search', exact: true }).click();
  await expect(page.locator('#settings-indexes')).toHaveAttribute('open', '');
  await expect(page.getByRole('switch', { name: 'Index my vault for semantic search' })).toBeFocused();
});

test('a control that is not shown lands on the switch that shows it', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await openSettings(page);
  await search(page).fill('Indexing progress');
  await index(page).getByRole('button', { name: 'Indexing progress', exact: true }).click();
  await expect(page.locator('#settings-indexes .settings-found').filter({ hasText: 'Index my vault for semantic search' })).toHaveCount(1);
  await expect(page.getByRole('switch', { name: 'Index my vault for semantic search' })).toBeFocused();
});

test('Enter opens the top hit', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await openSettings(page);
  await search(page).fill('reduce motion');
  await search(page).press('Enter');
  await expect(page.locator('#settings-general .settings-found').filter({ hasText: 'Motion' })).toHaveCount(1);
});

test('a switch starts at the top, focuses the heading and is remembered for the session', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 700 });
  await openSettings(page);
  await index(page).getByRole('button', { name: 'Advanced', exact: true }).click();
  await expect(page.locator('#settings-advanced')).toBeVisible();
  // Its groups open, so there is something to scroll.
  for (const summary of await page.locator('#settings-advanced details.settings-group > summary').all()) await summary.click();
  await page.locator('.settings-body').evaluate(el => { el.scrollTop = 900; });
  await index(page).getByRole('button', { name: 'Usage', exact: true }).click();
  const heading = page.getByRole('heading', { level: 2, name: 'Usage' });
  await expect(heading).toBeFocused();
  expect(await page.locator('.settings-body').evaluate(el => el.scrollTop)).toBe(0);
  // Leaving Settings and coming back reopens Usage.
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true }).click();
  await expect(page.locator('.settings-page')).toHaveCount(0);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page.locator('#settings-usage')).toBeVisible();
  await expect(index(page).getByRole('button', { name: 'Usage', exact: true })).toHaveAttribute('aria-current', 'page');
});

test('an unsaved setting asks before switching, and Keep editing keeps it', async ({ page }) => {
  // Windows moved to Automations (PR 4); Advanced's Warn under is another edit that waits for Save.
  await page.setViewportSize({ width: 1440, height: 1000 });
  await openSettings(page);
  await index(page).getByRole('button', { name: 'Advanced', exact: true }).click();
  await page.locator('#settings-group-this-mac > summary').click();
  const warn = page.getByRole('textbox', { name: 'Warn under' });
  const before = await warn.inputValue();
  const edited = before === '77' ? '78' : '77';
  await warn.fill(edited);
  await index(page).getByRole('button', { name: 'General', exact: true }).click();
  const ask = page.getByRole('alertdialog', { name: 'Unsaved changes' });
  await expect(ask).toBeVisible();
  await expect(ask).toContainText('Disk space warning');
  await expect(page.locator('#settings-advanced')).toBeVisible();
  await ask.getByRole('button', { name: 'Keep editing' }).click();
  await expect(ask).toHaveCount(0);
  // Focus goes back to what asked to switch, not to the page.
  await expect(index(page).getByRole('button', { name: 'General', exact: true })).toBeFocused();
  await expect(warn).toHaveValue(edited);
  // Leaving on purpose drops the draft and opens the section asked for.
  await index(page).getByRole('button', { name: 'General', exact: true }).click();
  await page.getByRole('alertdialog', { name: 'Unsaved changes' }).getByRole('button', { name: /^Discard changes/ }).click();
  await expect(page.locator('#settings-general')).toBeVisible();
  await index(page).getByRole('button', { name: 'Advanced', exact: true }).click();
  await page.locator('#settings-group-this-mac > summary').click();
  await expect(page.getByRole('textbox', { name: 'Warn under' })).toHaveValue(before);
});

test('below 860 px the search stays and a control hit jumps', async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 700 });
  await openSettings(page);
  await expect(page.getByRole('combobox', { name: 'Settings section' })).toBeVisible();
  await expect(search(page)).toBeVisible();
  // Search sits above the section picker.
  const searchBox = await search(page).boundingBox();
  const picker = await page.getByRole('combobox', { name: 'Settings section' }).boundingBox();
  expect(searchBox!.y).toBeLessThan(picker!.y);
  await search(page).fill('motion');
  await index(page).getByRole('button', { name: 'Motion', exact: true }).click();
  await expect(page.locator('#settings-general .settings-found').filter({ hasText: 'Motion' })).toHaveCount(1);
});
