import { openTasks } from './choose';
import { expect, test, type Page } from './fixtures/test';
import { openWaiting } from './waiting';
import { openLibraryView } from './libraryView';

/**
 * The fixture state matrix (plan completion L5, review V07 / roadmap B,
 * matrix V7a): each named whole-app scenario in src/mockScenarios.ts boots
 * the populated demo in that state, and the person can see the state where
 * they would look for it, reached by clicks. docs/FIXTURES.md maps every
 * state to its URL, its surface and the test below. The automatic pageErrors
 * fixture fails any of these on an uncaught page error.
 */

test.beforeEach(({}, info) => { test.skip(info.project.name !== 'chromium-desktop', 'the state matrix is width-independent; the journeys cover the small sizes'); });

const boot = (page: Page, scenario: string, demo = '1') => page.goto(`/?mock=1&demo=${demo}&scenario=${scenario}`);
const nav = (page: Page, name: string) => page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name, exact: true });

async function settingsSection(page: Page, name: RegExp) {
  await page.getByRole('button', { name: /Settings/ }).first().click();
  await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name }).click();
}

async function openReport(page: Page) {
  await page.getByRole('region', { name: 'Projects' }).getByRole('button', { name: /Q3 regional operations review/ }).first().click();
  await expect(page.locator('.top-bar-crumbs')).toContainText('Q3 regional operations review');
}

test('disconnected: each outside service says it is not connected, and where to fix it', async ({ page }) => {
  await boot(page, 'disconnected');
  // The review list names the source it couldn't read rather than looking empty.
  const panel = await openWaiting(page);
  await expect(panel.getByRole('status').filter({ hasText: /Couldn’t read Linear sync/ })).toBeVisible();
  // The companion backend: unreachable, said plainly, with the Tailscale hint for a tailnet name (B05).
  // Companion devices is in Devices since Settings PR 3.
  await settingsSection(page, /^Devices/);
  const backend = page.locator('.companion-backend');
  await expect(backend.locator('.companion-connection-state')).toHaveText("Can't reach the backend");
  await expect(backend).toContainText("Hollow couldn't reach hollow-companion.example.ts.net");
  await expect(backend).toContainText('check that Tailscale is running');
  // Microsoft 365: signed out, with the sign-in in Connectors.
  await settingsSection(page, /^Connectors/);
  await expect(page.getByText('Sign in to Microsoft once, here in Hollow.')).toBeVisible();
  // Agenda was cut (overnight 11-08 S3). Signing in stays on Connectors.
  await settingsSection(page, /^Advanced/);
  await expect(page.getByRole('heading', { name: 'Advanced', exact: true, level: 2 })).toBeVisible();
  await expect(backend).toHaveCount(0);
  await openTasks(page);
  await expect(page.locator('.agenda-source')).toHaveCount(0);
  await settingsSection(page, /^Connectors/);
  await expect(page.getByText('Sign in to Microsoft once, here in Hollow.')).toBeVisible();
});

test('interrupted: the restart offer lists the chat, the turn offers Continue, and nothing resends by itself', async ({ page }) => {
  await boot(page, 'interrupted');
  const panel = await openWaiting(page);
  const row = panel.getByRole('button', { name: /Resume 1 interrupted chat/ });
  await expect(row).toBeVisible();
  await expect(row).toContainText('Revise the labour section');
  await row.click();
  const confirm = page.getByRole('dialog');
  await expect(confirm).toContainText('Revise the labour section');
  await confirm.getByRole('button', { name: /Cancel|Not now|Keep/ }).first().click();
  // The interrupted turn itself: Continue is offered, never sent on its own.
  await nav(page, 'Projects').click();
  await openReport(page);
  // The follow-up queued before the quit is back in the composer, and the
  // notice saying so doesn't hide Continue (it did before the L5 fix).
  await expect(page.getByLabel('Chat message')).toHaveValue('Also add the overtime table to the appendix.');
  await expect(page.getByText(/A message you queued before Hollow quit was never sent/)).toBeVisible();
  await expect(page.getByRole('button', { name: 'Continue', exact: true })).toBeVisible();
  const sent = await page.evaluate(() => (window as unknown as { __mockApiCalls?: Record<string, number> }).__mockApiCalls?.resume_interrupted_chats ?? 0);
  expect(sent, 'nothing resumes without the person').toBe(0);
});

test('permission-required: a held automation and an approval wait, each saying why', async ({ page }) => {
  await boot(page, 'permission-required');
  const panel = await openWaiting(page);
  await expect(panel.getByRole('button', { name: /Allow “git push”/ })).toBeVisible();
  // The held automation says it is paused and why, where automations live.
  await nav(page, 'Automations').click();
  await expect(page.getByText(/paused until its settings change is approved/).first()).toBeVisible();
});

// The stale brief's notice chip went with the workspace panel's Overview
// (COO rebuild L4, 2026-11-10); the question still says it.
test('stale: an unusable question says it is out of date', async ({ page }) => {
  await boot(page, 'stale');
  const panel = await openWaiting(page);
  const question = panel.getByRole('button', { name: /Keep the old heading/ });
  await expect(question).toHaveAttribute('aria-disabled', 'true');
  await expect(question).toContainText('can no longer be answered');
});

test('partial-success: half-read sources are named, and what was read still shows', async ({ page }) => {
  await boot(page, 'partial-success');
  const panel = await openWaiting(page);
  await expect(panel.getByRole('status').filter({ hasText: 'Couldn’t read questions and Linear sync' })).toBeVisible();
  await expect(panel.getByRole('region', { name: 'Review items and proposals' })).toContainText('Release verification');
});

test('conflict: a two-writer task and iCloud copies wait as conflicts', async ({ page }) => {
  await boot(page, 'conflict');
  const panel = await openWaiting(page);
  await expect(panel.getByRole('button', { name: /Send the draft to Dana changed in Linear and here/ })).toBeVisible();
  await expect(panel.getByRole('button', { name: /2 conflict copies in the vault/ })).toBeVisible();
});

test('loading: slow reads show they are loading, then the content, with no empty-state flash', async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as unknown as { releaseLoadingProjects: () => void };
    window.__mockHoldOnce = { command: 'list_workspaces', release: new Promise<void>(resolve => { w.releaseLoadingProjects = resolve; }) };
  });
  await boot(page, 'loading');
  const resume = page.getByRole('region', { name: 'Projects' });
  const loading = page.getByRole('status').filter({ hasText: 'Loading your work…' });
  await expect(loading).toBeVisible();
  // The initial workspace read gates Home. Hold it until the page's
  // loading status is observed, then release it to reveal Projects.
  for (let i = 0; i < 6; i++) {
    await expect(page.getByText(/Nothing to (resume|pick up)|No workspaces yet|A place for your ongoing work/)).toHaveCount(0);
    await page.waitForTimeout(400);
  }
  await page.evaluate(() => (window as unknown as { releaseLoadingProjects: () => void }).releaseLoadingProjects());
  await expect(resume.locator('.home-project-row').first()).toBeVisible({ timeout: 15_000 });
  await expect(loading).toBeHidden();
  await expect(resume).toHaveAttribute('aria-busy', 'false');
});

test('unavailable: damaged stores say so instead of looking empty', async ({ page }) => {
  await boot(page, 'unavailable');
  await nav(page, 'Library').click();
  await openLibraryView(page, 'Results');
  await expect(page.getByText(/Results review state was damaged/).first()).toBeVisible();
});

test('scenarios combine, and an unknown name is reported rather than silently ignored', async ({ page }) => {
  const warnings: string[] = [];
  page.on('console', m => { if (m.type() === 'warning') warnings.push(m.text()); });
  await boot(page, 'stale,conflict,nonsense');
  const names = await page.evaluate(() => (window as unknown as { __mockScenario?: { names: string[] } }).__mockScenario?.names);
  expect(names).toEqual(['stale', 'conflict']);
  expect(warnings.join('\n')).toContain('mock scenario "nonsense" doesn’t exist'.replace('’', "'"));
  const panel = await openWaiting(page);
  await expect(panel.getByRole('button', { name: /Keep the old heading/ })).toBeVisible();
  await expect(panel.getByRole('button', { name: /2 conflict copies in the vault/ })).toBeVisible();
});

test('library-scale: 213 workspaces with same-named ones told apart, on the large fixture', async ({ page }) => {
  test.setTimeout(60_000);
  await page.goto('/?mock=1&demo=large&missions=1&scenario=library-scale');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true }).click();
  const list = page.getByRole('region', { name: 'Projects' });
  await list.getByRole('button', { name: 'All projects' }).click();
  await list.getByRole('checkbox', { name: 'Include archived' }).check();
  await list.getByRole('searchbox', { name: 'Find a project' }).fill('Q3 review');
  for (const origin of ['Project folder', 'Hollow workspace', 'Notion project', 'Build · feature/q3-review']) {
    await expect(list.getByRole('button', { name: new RegExp(`^Actions for Q3 review \\(${origin.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`) })).toBeVisible();
  }
  const panel = await openWaiting(page);
  await expect(panel.getByRole('region', { name: 'Approvals and questions' })).toContainText('Question 30 from a running chat');
});
