import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

/** Overnight N1 (2026-11-02): Settings → Devices → Runs on (Models until S5b) shows the
 *  Mac mini runner's state (mockApi's `runner_status`, `mockRemoteRunner`
 *  names the runner, `mockRunnerStatus` replaces the answer). */
async function openAgents(page: Page) {
  await page.goto('/?mock=1');
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), 'devices');
  } else {
    await page.locator('.settings-index').getByRole('button', { name: 'Devices', exact: true }).click();
  }
}

test('Runs on shows the runner answering, its preflight and last gate, and Check now asks again', async ({ page }) => {
  await page.addInitScript(() => { localStorage.setItem('mockRemoteRunner', 'runner-host'); });
  await openAgents(page);
  const block = page.getByRole('group', { name: 'Remote runner runner-host' });
  // Never checked: opening Settings checks.
  await expect(block).toContainText('Answering');
  // S5b-3: the preflight, last gate and owner sit in a fold that is closed at rest.
  await block.locator('details.runner-status-more').evaluate(el => { (el as HTMLDetailsElement).open = true; });
  await expect(block.getByRole('list', { name: 'Its preflight' })).toContainText('Free disk 243 GB');
  await expect(block).toContainText('Memory free 92%');
  await expect(block).toContainText('build-mac');
  await expect(block).toContainText('Running there now: 1 remote session.');
  await expect(block).toContainText(/Last gate there for ri27-mm5 at 4c5517b3: passed \d+m ago in 2 min 31 s\./);
  // The check on open took 1.1 s in the mock; Check now's own takes 1.2 s.
  await expect(block).toContainText(/checked \d+s ago \(1\.1 s\)/);
  await block.getByRole('button', { name: 'Check now' }).click();
  await expect(block).toContainText(/checked \d+s ago \(1\.2 s\)/);
  await block.scrollIntoViewIfNeeded();
  await page.screenshot({ path: test.info().outputPath('runner-status-ready.png') });
});

test('a runner that does not answer says what to check, and shows no stale facts', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('mockRemoteRunner', 'runner-host');
    localStorage.setItem('mockRunnerStatus', JSON.stringify({
      state: 'unreachable', checkedAt: new Date().toISOString(), millis: 10012,
      // An earlier answer's facts must not show beside "Not answering".
      preflight: 'ok host=build-mac free=243GB mem=92%', machine: 'build-mac', running: 1,
      lastGate: { start: new Date().toISOString(), end: new Date().toISOString(), seconds: 151, ok: true, exitCode: 0, failedStep: null, commit: '4c5517b3', checkout: 'ri27-mm5' },
      message: "runner-host didn't answer: ssh: connect to host runner-host port 22: Operation timed out. It may be off, asleep or restarting, or the network between the two is down. After a restart it waits at the FileVault unlock screen until someone signs in on it; nothing runs there until then.",
    }));
  });
  await openAgents(page);
  const block = page.getByRole('group', { name: 'Remote runner runner-host' });
  await expect(block).toContainText('Not answering');
  await expect(block).toContainText('FileVault unlock screen');
  await expect(block).not.toContainText('Free disk');
  await expect(block).not.toContainText('Last gate');
  await block.scrollIntoViewIfNeeded();
  await page.screenshot({ path: test.info().outputPath('runner-status-unreachable.png') });
});

test('Builds and tests shows where the latest commands ran and what the runner has', async ({ page }) => {
  await page.addInitScript(() => { localStorage.setItem('mockRemoteRunner', 'runner-host'); });
  await openAgents(page);
  await expect(page.locator('#builds-on')).toBeVisible();
  // S5b-3: the decisions sit in a fold that is closed at rest.
  await page.locator('details.route-decisions-fold').evaluate(el => { (el as HTMLDetailsElement).open = true; });
  const block = page.getByRole('group', { name: 'Where builds and tests ran' });
  await expect(block).toContainText('runner-host: Has Rust 1.96.0, Node 26.3.1, uv 0.13.5, Python 3.14.6, Postgres 17.11, Playwright chromium-1243.');
  const decisions = block.getByRole('list', { name: 'Recent decisions' });
  await expect(decisions.getByRole('listitem')).toHaveCount(3);
  await expect(decisions.getByRole('listitem').first()).toContainText('Runner · gate, usually 3 min; this Mac 72% free');
  await expect(decisions).toContainText('This Mac · npm run tauri:rebuild: asked for with --here');
  await block.scrollIntoViewIfNeeded();
  await page.screenshot({ path: test.info().outputPath('builds-and-tests.png') });
});

test('no runner set up: no status block, and nothing to route until Set up', async ({ page }) => {
  await openAgents(page);
  // Overnight 11-07: the rows that choose what runs on a runner appear once one is set.
  await expect(page.getByRole('button', { name: 'Set up', exact: true })).toBeVisible();
  await expect(page.locator('#runs-on')).toHaveCount(0);
  await expect(page.locator('#builds-on')).toHaveCount(0);
  await expect(page.locator('#app-builds-on')).toHaveCount(0);
  await expect(page.locator('#unattended-runs-on')).toHaveCount(0);
  await expect(page.locator('.runner-github')).toHaveCount(0);
  await expect(page.locator('.runner-status')).toHaveCount(0);
});
