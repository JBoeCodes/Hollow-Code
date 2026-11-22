import { test, expect, type Page } from './fixtures/test';
import type { MockPeers } from '../../src/mockPeers';

declare global { interface Window { __mockPeers?: MockPeers } }

// Two Macs, the UI paths (`docs/reference/peers.md`, "The UI"; `src/mockPeers.ts`
// plays the other Mac): Cancel on the Pairing step, address entry, the route
// chip on a routed turn, Start the runner's COO, Library's list of another
// Mac's chats with its "On Mac mini" filter, and the notices when that Mac
// can't be reached. Written for the end browser batch; first run and fixed 2026-11-18.

test.beforeEach(({}, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'the sidebar and Library lists are desktop layouts');
});

const MINI = 'm-8d21c4be90f17a3e';
const ROUTED = 's-20261117-route001';
const cwd = '/Users/fixture/vault/workspaces/quick-chats';

// `demo=1` seeds the demo set over mockWorkspaces and its sessions, so a spec
// that seeds its own chats opens with plain `mock=1`; `main=1` adds the COO's
// workspace, without which the COO's status is never read.
async function open(page: Page, init: (arg: unknown) => void, arg: unknown, url = '/?mock=1&demo=1') {
  await page.addInitScript(init, arg);
  await page.goto(url);
  await expect(page.locator('.app')).toBeVisible();
}

async function openMacs(page: Page) {
  await page.evaluate(async () => (await import('/src/store/runtime.ts')).requestOpenSettingsSection('devices'));
  await page.getByRole('navigation', { name: 'Shared tools' }).getByRole('button', { name: 'Settings', exact: true }).click();
  const panel = page.locator('.machines-panel');
  await expect(panel.getByRole('heading', { name: 'Macs', exact: true })).toBeVisible();
  return panel;
}

test('Cancel on the Pairing step stops a pairing in flight and keeps nothing', async ({ page }, info) => {
  await open(page, () => localStorage.setItem('mockPeers', JSON.stringify({ pairHold: true })), null);
  const panel = await openMacs(page);
  await panel.getByRole('button', { name: 'Pair a Mac' }).click();
  const form = panel.getByRole('form', { name: 'Pair a Mac' });
  await form.getByRole('button', { name: 'Pair', exact: true }).click();
  // In flight: Pair is busy, and Cancel stays clickable.
  await expect(form.getByRole('button', { name: 'Pairing…' })).toBeDisabled();
  const cancel = form.getByRole('button', { name: 'Cancel' });
  await expect(cancel).toBeEnabled();
  await form.screenshot({ path: info.outputPath('pairing-in-flight.png') });
  await cancel.click();
  // Back where it was: the form is gone, nothing paired, no error shown.
  await expect(form).toHaveCount(0);
  await expect(panel.getByText('None paired')).toBeVisible();
  await expect(panel.getByRole('alert')).toHaveCount(0);
  expect(await page.evaluate(() => window.__mockPeers!.pairCalls)).toEqual(['runner-host', 'cancel']);
  // The machine strip, exactly: Devices' own "Macs that open this Mac's chats" stays.
  await expect(page.getByRole('group', { name: 'Macs', exact: true })).toHaveCount(0);
  // Cancel with nothing running just closes the form.
  await panel.getByRole('button', { name: 'Pair a Mac' }).click();
  await panel.getByRole('form', { name: 'Pair a Mac' }).getByRole('button', { name: 'Cancel' }).click();
  await expect(panel.getByRole('form', { name: 'Pair a Mac' })).toHaveCount(0);
  // And the next pairing works.
  await panel.getByRole('button', { name: 'Pair a Mac' }).click();
  await panel.getByRole('form', { name: 'Pair a Mac' }).getByRole('button', { name: 'Pair', exact: true }).click();
  await expect(panel.getByText('Paired with Mac mini.')).toBeVisible();
});

test('Pair a Mac takes a typed address, refuses junk in plain words, and sends host:port as typed', async ({ page }, info) => {
  await open(page, () => localStorage.setItem('mockPeers', JSON.stringify({ suggestedAlias: null })), null);
  const panel = await openMacs(page);
  await panel.getByRole('button', { name: 'Pair a Mac' }).click();
  const form = panel.getByRole('form', { name: 'Pair a Mac' });
  const box = form.getByRole('textbox', { name: 'SSH alias or address' });
  await expect(form.locator('xpath=following-sibling::p[1]')).toContainText('type its address instead');
  // Junk and this Mac's own loopback are refused before anything is asked of ssh.
  for (const [junk, why] of [
    ['https://example.com/pair', 'type its name, not a web address or a path'],
    ['127.0.0.1', 'that is this Mac, not another one'],
    ['localhost:47680', 'that is this Mac, not another one'],
    ['mini:99999', 'the port must be a number from 1 to 65535'],
  ] as const) {
    await box.fill(junk);
    await form.getByRole('button', { name: 'Pair', exact: true }).click();
    await expect(panel.getByRole('alert')).toContainText(why);
  }
  await panel.screenshot({ path: info.outputPath('pairing-refused.png') });
  expect(await page.evaluate(() => window.__mockPeers!.pairCalls ?? [])).toEqual([]);
  // A tailnet name with a port goes through and pairs.
  await box.fill('build-mac.tail0000.ts.net:47680');
  await form.getByRole('button', { name: 'Pair', exact: true }).click();
  await expect(panel.getByText('Paired with Mac mini.')).toBeVisible();
  await expect(panel.getByRole('alert')).toHaveCount(0);
  expect(await page.evaluate(() => window.__mockPeers!.pairCalls)).toEqual(['build-mac.tail0000.ts.net']);
});

test('the route chip on a routed turn opens that chat on that Mac', async ({ page }, info) => {
  await open(page, arg => {
    const [key, dir] = arg as [string, string];
    const at = new Date().toISOString(); // sleep-ok: a fixture timestamp, nothing waits on it
    const summary = { key, title: 'Ship the build', agent: 'claude', harnessSessionId: 'sess-native', cwd: dir, createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'native' };
    const receipt = JSON.stringify({ routed: true, to: 'coo', session: 's-mini-build', machine: { id: 'm-8d21c4be90f17a3e', name: 'Mac mini' } });
    localStorage.setItem('mockWorkspaces', JSON.stringify([{ id: 'quick-chats', title: 'Quick chats', status: 'active', path: 'workspaces/quick-chats', cwd: dir, modified: at, kind: 'quick' }]));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify([summary]));
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify({ [key]: { summary, eventsRetained: true, turns: [
      { id: `${key}-u`, role: 'user', text: 'Run the nightly build', tools: [], agent: 'claude', runId: 'run-r', startedAt: at, transcriptIncomplete: false },
      { id: `${key}-a`, role: 'assistant', text: 'Sent it to the runner.', tools: [{ id: 'tool-route', name: 'mcp__hollow_app__run_action', input: { id: 'sessions.route_to_runner' }, result: receipt, isError: false, status: 'done' }], agent: 'claude', runId: 'run-r', startedAt: at, status: 'success', transcriptIncomplete: false },
    ] } }));
    localStorage.setItem('mockPeers', 'demo');
  }, [ROUTED, cwd], '/?mock=1');
  await page.getByRole('complementary', { name: 'Main navigation' }).getByRole('group', { name: 'Quick chats' }).getByRole('button', { name: /^Ship the build/ }).first().click();
  await expect(page.locator('.chat-main')).toContainText('Sent it to the runner.');
  // A finished turn folds its steps into "Worked · 1 step" (frontier F5); the
  // chip sits in the route row's head inside it.
  await page.locator('.chat-main').getByRole('button', { name: /^Worked/ }).click();
  const chip = page.getByRole('button', { name: 'Sent to Mac mini’s COO: open that chat' });
  await expect(chip).toBeVisible();
  await chip.screenshot({ path: info.outputPath('route-chip.png') });
  await chip.click();
  const view = page.getByRole('region', { name: 'Chat on Mac mini' });
  await expect(view.getByRole('heading', { name: 'Nightly build on the mini' })).toBeVisible();
});

test('Settings, Devices: Start the runner’s COO appears only while its COO has no conversation, and starts it', async ({ page }, info) => {
  await open(page, () => {
    localStorage.setItem('mockPeers', 'demo');
    localStorage.setItem('mockMainAgentStatus', JSON.stringify({
      spentTodayUsd: 0, askedTodayUsd: 0, wakeSkipped: null, dailyCapUsd: 100, checkIns: false, lastCheckIn: null, nextCheckIn: null, checkInsToday: 0,
      suggestions: { answered: 0, taken: 0, declinedForGood: 0, takenShare: null }, actOnOwn: true, role: 'single',
      dispatcher: { dispatching: false, setting: 'auto', machineId: 'm-8d21c4be90f17a3e', machine: 'Mac mini', since: null, off: 'runner-not-started', whyOff: 'Mac mini’s runner COO has no conversation yet.' },
    }));
  }, null, '/?mock=1&demo=1&main=1');
  await openMacs(page);
  const row = page.locator('.settings-row').filter({ hasText: 'COO on this Mac' }).first();
  const start = row.getByRole('button', { name: 'Start Mac mini\'s COO' });
  await expect(start).toBeVisible();
  await row.screenshot({ path: info.outputPath('start-coo.png') });
  await start.click();
  await expect.poll(() => page.evaluate(() => window.__mockPeers!.cooStarted)).toEqual([MINI]);
});

test('Library, Conversations: another Mac’s chats under "On Mac mini", read-only with a notice while it is unreachable', async ({ page }, info) => {
  await open(page, () => localStorage.setItem('mockPeers', 'demo'), null);
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true }).click();
  await page.getByRole('navigation', { name: 'Library view' }).getByRole('button', { name: 'Conversations', exact: true }).click();
  const aside = page.getByRole('complementary', { name: 'Conversations' });
  // This Mac's list first; the Mac's name is the filter's own label, from the registry.
  await expect(aside.getByRole('button', { name: /^Open Nightly build on the mini on Mac mini/ })).toHaveCount(0);
  await aside.locator('.chat-history-sources').getByText('On Mac mini', { exact: true }).click();
  const chats = aside.getByRole('region', { name: 'Chats on Mac mini' });
  await expect(chats.getByRole('button', { name: /^Open Nightly build on the mini on Mac mini/ })).toBeVisible();
  await expect(chats.getByRole('button', { name: /^Open Pick a release name on Mac mini/ })).toBeVisible();
  await chats.screenshot({ path: info.outputPath('library-on-mac-mini.png') });
  // Back to Hollow's own list: the remote rows go.
  await aside.locator('.chat-history-sources').getByText('Hollow', { exact: true }).click();
  await expect(chats).toHaveCount(0);

  // Unreachable: the list stays, read-only, and says so with its actions.
  await aside.locator('.chat-history-sources').getByText('On Mac mini', { exact: true }).click();
  await page.evaluate(() => {
    const s = window.__mockPeers!;
    Object.assign(s.machines.find(m => m.machineId === 'm-8d21c4be90f17a3e')!, { state: 'offline', message: 'ssh: connect to host runner-host port 22: Operation timed out' });
    (window as unknown as { __mockEmit: (n: string, p: unknown) => void }).__mockEmit('peer-machines-changed', { thisMac: s.thisMac, machines: s.machines.map(m => ({ ...m })), suggestedAlias: s.suggestedAlias });
  });
  const status = chats.getByRole('status');
  await expect(status).toContainText('Mac mini');
  await expect(status.getByRole('button', { name: /Retry|Check now/ }).first()).toBeVisible();
  await chats.screenshot({ path: info.outputPath('library-mac-offline.png') });
});
