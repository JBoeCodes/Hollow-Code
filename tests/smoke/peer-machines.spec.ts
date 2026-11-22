import { test, expect, type Page } from './fixtures/test';
import type { MockPeers } from '../../src/mockPeers';

declare global { interface Window { __mockPeers?: MockPeers } }

// M2 PR 4, the peer link's UI (`docs/reference/peers.md`, "The UI";
// `src/mockPeers.ts` plays the other Mac): the machine strip and its states,
// a chat on another Mac read live (appends, send, stop, busy, its card
// answered at its home, a canvas placeholder), and Settings, Devices, Macs.
// The mock's live object `window.__mockPeers` stands in for the other Mac;
// `window.__mockEmit` sends what its hub would.

test.beforeEach(({}, info) => {
  // The phone-width sidebar is an overlay; the route ledger walks the strip at
  // the native minimum window and Largest text (route-ledger.journey.spec.ts).
  test.skip(info.project.name === 'chromium-narrow', 'the strip lives in the sidebar; small windows are the ledger walk’s');
});

const MINI = 'm-8d21c4be90f17a3e';
const ago = (min: number) => new Date(Date.now() - min * 60_000).toISOString();

type Seed = Record<string, unknown>;
async function start(page: Page, seed: Seed | 'demo' | null, url = '/?mock=1&demo=1') {
  await page.addInitScript(s => {
    if (s == null) localStorage.removeItem('mockPeers');
    else localStorage.setItem('mockPeers', typeof s === 'string' ? s : JSON.stringify(s));
  }, seed);
  await page.goto(url);
  await expect(page.locator('.app')).toBeVisible();
}

const sidebar = (page: Page) => page.getByRole('complementary', { name: 'Main navigation' });
const strip = (page: Page) => sidebar(page).getByRole('group', { name: 'Macs' });
const emit = (page: Page, name: string, payload: unknown) => page.evaluate(([n, p]) => (window as unknown as { __mockEmit: (n: string, p: unknown) => void }).__mockEmit(n, p), [name, payload] as const);
/** Change the other Mac, then say so as its hub would. */
async function changeMac(page: Page, machineId: string, patch: Record<string, unknown>) {
  await page.evaluate(([id, p]) => {
    const s = window.__mockPeers!;
    Object.assign(s.machines.find(m => m.machineId === id)!, p);
    (window as unknown as { __mockEmit: (n: string, p: unknown) => void }).__mockEmit('peer-machines-changed', { thisMac: s.thisMac, machines: s.machines.map(m => ({ ...m })), suggestedAlias: s.suggestedAlias });
  }, [machineId, patch] as const);
}

test('the strip shows nothing until another Mac is paired', async ({ page }) => {
  await start(page, null);
  await expect(page.getByRole('region', { name: 'Projects' })).toBeVisible();
  await expect(strip(page)).toHaveCount(0);
});

test('the strip shows each Mac’s state, its popover, Retry, and remote quick chats with a badge', async ({ page }, info) => {
  await start(page, {
    preset: 'demo',
    machines: [
      { machineId: MINI, slug: 'build-mac', name: 'Mac mini', sshHosts: ['runner-host', 'runner-host-lan'], remotePort: 47680, pairedAt: ago(3000), lastSeenAt: ago(1), lastVersion: '0.5.17 (abc1234)', state: 'online', message: null, checkedAt: ago(1), alias: 'runner-host' },
      { machineId: 'm-studio', slug: 'studio', name: 'Studio', sshHosts: ['studio'], remotePort: 47680, pairedAt: ago(3000), lastSeenAt: ago(4), lastVersion: null, state: 'offline', message: 'ssh: Could not resolve hostname studio', checkedAt: ago(1), alias: null },
      { machineId: 'm-old', slug: null, name: 'Old Mac', sshHosts: ['old'], remotePort: 47680, pairedAt: ago(3000), lastSeenAt: null, lastVersion: null, state: 'pairing-needed', message: null, checkedAt: ago(1), alias: null },
      { machineId: 'm-new', slug: null, name: 'New Mac', sshHosts: ['new'], remotePort: 47680, pairedAt: ago(5), lastSeenAt: null, lastVersion: null, state: 'offline', message: null, checkedAt: null, alias: null },
    ],
    onCheck: { 'm-studio': 'online' },
  });
  const rows = strip(page);
  await expect(rows.getByLabel('Alex’s MacBook Pro, This Mac')).toBeVisible();
  await expect(rows.getByRole('button', { name: 'Mac mini, 1 running' })).toBeVisible();
  await expect(rows.getByRole('button', { name: 'Studio, Offline · 4 min ago' })).toBeVisible();
  await expect(rows.getByRole('button', { name: 'Old Mac, Pair again' })).toBeVisible();
  // Before its first call a Mac is offline with no check: not checked yet.
  await expect(rows.getByRole('button', { name: 'New Mac, Not checked yet' })).toBeVisible();
  await rows.screenshot({ path: info.outputPath('strip.png') });

  // The online Mac's popover: its chats, newest first, the waiting one badged.
  await rows.getByRole('button', { name: 'Mac mini, 1 running' }).click();
  const pop = page.getByRole('dialog', { name: 'Chats on Mac mini' });
  await expect(pop.getByText('Online, last seen 1 min ago')).toBeVisible();
  const chats = pop.getByRole('list', { name: 'Chats on Mac mini' }).getByRole('button');
  await expect(chats).toHaveCount(3);
  await expect(chats.nth(0)).toContainText('Nightly build on the mini');
  await expect(chats.nth(2)).toContainText('Clean up the runner disk');
  await expect(pop.getByRole('img', { name: 'On Mac mini, waiting for your answer' })).toBeVisible();
  await pop.screenshot({ path: info.outputPath('popover-online.png') });
  await page.keyboard.press('Escape');
  await expect(pop).toHaveCount(0);

  // An offline Mac: ssh's sentence and Retry, which asks it again.
  await rows.getByRole('button', { name: 'Studio, Offline · 4 min ago' }).click();
  const studio = page.getByRole('dialog', { name: 'Chats on Studio' });
  await expect(studio.getByText('Studio is offline, last seen 4 min ago.')).toBeVisible();
  await expect(studio.getByText('ssh: Could not resolve hostname studio')).toBeVisible();
  await studio.screenshot({ path: info.outputPath('popover-offline.png') });
  await studio.getByRole('button', { name: 'Retry' }).click();
  await expect(rows.getByRole('button', { name: 'Studio, Online' })).toBeVisible();

  // The Mac mini goes offline: its rows stay, stale and dimmed.
  await page.keyboard.press('Escape');
  await changeMac(page, MINI, { state: 'offline', lastSeenAt: ago(2) });
  await expect(rows.getByRole('button', { name: /^Mac mini, Offline · \d min ago$/ })).toBeVisible();
  await rows.getByRole('button', { name: /^Mac mini, Offline/ }).click();
  const stale = page.getByRole('dialog', { name: 'Chats on Mac mini' });
  await expect(stale.locator('.machine-popover-chats.is-stale')).toBeVisible();
  await expect(stale.getByRole('button', { name: /Nightly build on the mini/ })).toBeVisible();
  await page.keyboard.press('Escape');

  // A remote quick chat sits under Quick chats with its Mac's badge.
  await changeMac(page, MINI, { state: 'online' });
  await page.evaluate(id => {
    const chat = window.__mockPeers!.chats[id][2];
    chat.session = { ...chat.session, workspace: { id: 'quick-chats', title: 'Chats', kind: 'quick' } };
  }, MINI);
  await rows.getByRole('button', { name: /^Mac mini, / }).click();
  await page.getByRole('dialog', { name: 'Chats on Mac mini' }).getByRole('button', { name: 'Close' }).click();
  await page.evaluate(async id => { (await import('/src/store/peerSessions.ts')).refreshPeerSessions(id); }, MINI);
  const quick = sidebar(page).getByRole('group', { name: 'Quick chats' });
  await expect(quick.getByRole('button', { name: /^Clean up the runner disk/ })).toBeVisible();
  await expect(quick.getByRole('img', { name: 'On Mac mini' })).toBeVisible();
});

test('a chat on another Mac: read live, send, stop and answer its card', async ({ page }, info) => {
  await start(page, 'demo');
  const rows = strip(page);
  await rows.getByRole('button', { name: 'Mac mini, 1 running' }).click();
  await page.getByRole('dialog', { name: 'Chats on Mac mini' }).getByRole('button', { name: /^Nightly build on the mini/ }).click();
  const view = page.getByRole('region', { name: 'Chat on Mac mini' });
  await expect(view.getByRole('heading', { name: 'Nightly build on the mini' })).toBeVisible();
  await expect(view.getByText('Live from Mac mini')).toBeVisible();
  await expect(view.getByText('Mode: Edits only')).toBeVisible();
  await expect(view.getByText('Run the gate on main and tell me what fails.')).toBeVisible();
  await expect(view.getByText('Running the gate on main.')).toBeVisible();

  // Appends follow the live view held; one for another generation is dropped.
  await emit(page, 'peer-chat-event', { machineId: MINI, key: 's-mini-build', kind: 'append', data: { ord: 2, gen: 1, text: 'Three tests failed.' } });
  await expect(view.getByText('Running the gate on main. Three tests failed.')).toBeVisible();
  await emit(page, 'peer-chat-event', { machineId: MINI, key: 's-mini-build', kind: 'append', data: { ord: 2, gen: 9, text: ' DROPPED' } });
  await emit(page, 'peer-chat-event', { machineId: MINI, key: 's-mini-build', kind: 'append', data: { ord: 2, gen: 1, text: ' All in peer tests.' } });
  await expect(view.getByText('Running the gate on main. Three tests failed. All in peer tests.')).toBeVisible();
  await expect(view.getByText(/DROPPED/)).toHaveCount(0);
  await view.screenshot({ path: info.outputPath('remote-chat-live.png') });

  // Send: at its home; the chat shows it and runs.
  const box = view.getByRole('textbox', { name: 'Message' });
  await box.fill('Also run clippy.');
  await view.getByRole('button', { name: 'Send' }).click();
  await expect(view.locator('.turn.user').getByText('Also run clippy.')).toBeVisible();
  await expect(box).toHaveValue('');
  expect(await page.evaluate(() => window.__mockPeers!.sent)).toEqual([{ machineId: MINI, key: 's-mini-build', text: 'Also run clippy.' }]);

  // Busy at its home: the draft stays and the composer says why.
  await page.evaluate(() => { window.__mockPeers!.busyKeys = ['s-mini-build']; });
  await box.fill('One more thing.');
  await box.press('Enter');
  await expect(view.getByText('Mac mini is busy with this chat. Your words are still here; send again in a moment.')).toBeVisible();
  await expect(box).toHaveValue('One more thing.');

  // Stop: at its home; Stop goes once the turn ends.
  await view.getByRole('button', { name: 'Stop on Mac mini' }).click();
  await expect(view.getByRole('button', { name: 'Stop on Mac mini' })).toHaveCount(0);

  // Offline: Send is disabled with the reason; Retry reopens it.
  await emit(page, 'peer-chat-event', { machineId: MINI, key: 's-mini-build', kind: 'disconnected', data: { message: 'The tunnel closed.', retryInMs: 5000 } });
  await expect(view.getByRole('status').filter({ hasText: "Can't reach Mac mini. The tunnel closed." }).first()).toBeVisible();
  await expect(view.getByRole('button', { name: 'Send' })).toBeDisabled();
  await view.getByRole('button', { name: 'Retry' }).click();
  await expect(view.getByRole('button', { name: 'Retry' })).toHaveCount(0);

  // Close drops it; the card chat opens from the popover.
  await view.getByRole('button', { name: 'Close this chat' }).click();
  await expect(view).toHaveCount(0);
  await rows.getByRole('button', { name: /^Mac mini, / }).click();
  await page.getByRole('dialog', { name: 'Chats on Mac mini' }).getByRole('button', { name: /^Pick a release name/ }).click();
  await expect(view.getByText('Which name should the release use?')).toBeVisible();
  await view.getByText('Aurora', { exact: true }).click();
  await view.screenshot({ path: info.outputPath('remote-chat-card.png') });
  await view.getByRole('button', { name: /^Submit/ }).click();
  await expect(view.getByText('Answered: aurora')).toBeVisible();
  await expect(view.getByText('Which name should the release use?')).toHaveCount(0);
  expect(await page.evaluate(() => window.__mockPeers!.answers)).toEqual([{ machineId: MINI, key: 's-mini-card', questionId: 'q-release', answers: { name: 'aurora' } }]);

  // A canvas in a reply is a placeholder: canvases stay on their Mac.
  await view.getByRole('button', { name: 'Close this chat' }).click();
  await rows.getByRole('button', { name: /^Mac mini, / }).click();
  await page.getByRole('dialog', { name: 'Chats on Mac mini' }).getByRole('button', { name: /^Clean up the runner disk/ }).click();
  await expect(view.getByText('Canvas on Mac mini: open it there.')).toBeVisible();
  await expect(view.getByText('The biggest folder is the cargo target.')).toBeVisible();
  await view.screenshot({ path: info.outputPath('remote-chat-canvas.png') });

  // Going elsewhere closes it.
  await page.getByRole('navigation', { name: 'Shared tools' }).getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(view).toHaveCount(0);
});

test('Settings, Macs: pair a Mac, Pair again from the strip, let another Mac connect', async ({ page }, info) => {
  await page.addInitScript(() => localStorage.setItem('mockPhoneAccess', JSON.stringify({
    machines: [{ deviceId: 'mc-1a2b3c4d', machineId: 'm-studio', name: 'Studio', pairedAt: '2026-11-14T10:00:00Z', lastSeenAt: '2026-11-15T09:00:00Z' }],
  })));
  await start(page, { pairFail: 'ssh: connect to host runner-host port 22: Operation timed out' });
  await expect(strip(page)).toHaveCount(0);
  await page.evaluate(async () => (await import('/src/store/runtime.ts')).requestOpenSettingsSection('devices'));
  await page.getByRole('navigation', { name: 'Shared tools' }).getByRole('button', { name: 'Settings', exact: true }).click();
  const panel = page.locator('.machines-panel');
  await expect(panel.getByRole('heading', { name: 'Macs', exact: true })).toBeVisible();
  await expect(panel.getByText('None paired')).toBeVisible();

  await panel.getByRole('button', { name: 'Pair a Mac' }).click();
  const form = panel.getByRole('form', { name: 'Pair a Mac' });
  await expect(form.getByRole('textbox', { name: 'SSH alias' })).toHaveValue('runner-host');
  await form.getByRole('button', { name: 'Pair' }).click();
  // The failure shows as an alert (plain words, ssh's own under Details); the form stays.
  await expect(panel.getByRole('alert')).toBeVisible();
  await form.getByRole('button', { name: 'Pair' }).click();
  await expect(panel.getByText('Paired with Mac mini.')).toBeVisible();
  await expect(panel.getByRole('alert')).toHaveCount(0);
  const paired = panel.getByRole('list', { name: 'Paired Macs' });
  await expect(paired.getByText('Mac mini', { exact: true })).toBeVisible();
  // Settings height: each Mac's details and aliases sit in its closed fold, named by its summary.
  await expect(paired.locator('details[open]')).toHaveCount(0);
  await paired.getByText(/^Details and SSH aliases · 1 alias$/).click();
  await expect(paired.getByRole('list', { name: 'SSH aliases for Mac mini, in the order tried' })).toContainText('runner-host');
  // The strip appears once a Mac is paired.
  await expect(strip(page).getByRole('button', { name: /^Mac mini, / })).toBeVisible();

  // The other side: a code for another Mac, and the Macs paired to this one.
  // The server side is one closed fold; its summary says how many Macs open this one.
  const server = panel.getByRole('group', { name: 'Macs that open this Mac’s chats' });
  await expect(server).not.toHaveAttribute('open', '');
  await server.getByText('Let another Mac connect · 1 paired to this one', { exact: true }).click();
  await server.getByRole('button', { name: 'Let another Mac connect' }).click();
  await expect(server.locator('summary')).toHaveText(/^Let another Mac connect · code open until /);
  await expect(server.locator('code')).toHaveText('M4CX-7RQ2');
  await panel.screenshot({ path: info.outputPath('settings-macs.png') });
  await server.getByRole('button', { name: 'Cancel' }).click();
  await expect(server.getByRole('button', { name: 'Let another Mac connect' })).toBeVisible();
  await server.getByRole('button', { name: 'Revoke Studio' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Revoke' }).click();
  await expect(server.getByText('No Mac opens this Mac’s chats.')).toBeVisible();

  // Pairing needed: the strip's Pair again opens Pair a Mac with the alias.
  await changeMac(page, MINI, { state: 'pairing-needed' });
  await strip(page).getByRole('button', { name: 'Mac mini, Pair again' }).click();
  await expect(panel.getByRole('form', { name: 'Pair a Mac' }).getByRole('textbox', { name: 'SSH alias' })).toHaveValue('runner-host');

  // Forget: the strip goes with the last Mac.
  await paired.getByRole('button', { name: 'Forget Mac mini' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Forget' }).click();
  await expect(panel.getByText('None paired')).toBeVisible();
  await expect(strip(page)).toHaveCount(0);
});

// ── M5 UI: moving a chat (`docs/reference/peers.md`, "Moving a chat") ────

const QUICK = 'quick-chats';
const TRIP = 's-20261115-qqqq0001';
const REPORT = 's-20261115-qqqq0002';
const qcwd = (id: string) => `/Users/fixture/vault/workspaces/${id}`;
/** This Mac with two quick chats, and the Mac mini demo (`peers`). */
async function startWithQuickChats(page: Page, peers: Seed) {
  await page.addInitScript(([seedPeers, trip, report, cwd]) => {
    if (sessionStorage.getItem('moveSeeded')) return;
    sessionStorage.setItem('moveSeeded', '1');
    const at = (min: number) => new Date(Date.now() - min * 60_000).toISOString();
    const chat = (key: string, title: string, min: number) => ({ key, title, agent: 'claude', harnessSessionId: 'sess-native', cwd, createdAt: at(min), lastActivityAt: at(min), endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'native' });
    const sessions = [chat(trip, 'Plan the trip', 5), chat(report, 'Old report', 60)];
    localStorage.setItem('mockWorkspaces', JSON.stringify([{ id: 'quick-chats', title: 'Quick chats', status: 'active', path: 'workspaces/quick-chats', cwd, modified: at(5), kind: 'quick' }]));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify(sessions));
    const conversations: Record<string, unknown> = {};
    for (const s of sessions) conversations[s.key] = { summary: s, eventsRetained: true, turns: [
      { id: `${s.key}-u`, role: 'user', text: `Asked in ${s.title}`, tools: [], agent: 'claude', runId: `run-${s.key}`, startedAt: s.createdAt, transcriptIncomplete: false },
      { id: `${s.key}-a`, role: 'assistant', text: `Answer for ${s.title}`, tools: [], agent: 'claude', runId: `run-${s.key}`, startedAt: s.createdAt, status: 'success', transcriptIncomplete: false },
    ] };
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify(conversations));
    localStorage.setItem('mockPeers', JSON.stringify(seedPeers));
  }, [peers, TRIP, REPORT, qcwd(QUICK)] as const);
  await page.goto('/?mock=1');
  await expect(page.locator('.app')).toBeVisible();
  await expect(strip(page)).toBeVisible();
}
/** Toasts sit over the composer's area: hidden in the notice's screenshots. */
const QUIET = '.snackbar-stack { display: none !important; }';
const quickChats = (page: Page) => sidebar(page).getByRole('group', { name: 'Quick chats' });
const notice = (page: Page) => page.getByRole('status', { name: 'Where this chat runs' });
async function openQuick(page: Page, title: string) {
  // A row badged moved is named with its badge ("Old report Moved to …").
  await quickChats(page).getByRole('button', { name: new RegExp(`^${title}`) }).first().click();
  await expect(page.locator('.chat-main')).toContainText(`Answer for ${title}`);
}
async function conversationMenu(page: Page) {
  await page.getByRole('group', { name: 'Conversation', exact: true }).getByRole('button', { name: 'Actions for this conversation', exact: true }).click();
  return page.locator('.ctx-menu');
}

test('Move to runner moves a chat, which then lists once at its new Mac, badged moved', async ({ page }, info) => {
  await startWithQuickChats(page, { preset: 'demo', titles: { [TRIP]: 'Plan the trip' } });
  await openQuick(page, 'Plan the trip');
  let menu = await conversationMenu(page);
  await menu.getByRole('menuitem', { name: 'Move to runner', exact: true }).click();
  // It asks first, saying where it goes and whose logins pay.
  const ask = page.getByRole('dialog', { name: 'Move “Plan the trip” to Mac mini?' });
  await expect(ask).toContainText('Mac mini’s own agent logins pay for its turns');
  await ask.screenshot({ path: info.outputPath('move-confirm.png') });
  await ask.getByRole('button', { name: 'Move', exact: true }).click();
  await expect(page.getByText('Moved to Mac mini; it keeps going there.')).toBeVisible();
  expect(await page.evaluate(() => window.__mockPeers!.moves)).toEqual([{ command: 'chat_move_to_runner', key: 's-20261115-qqqq0001' }]);
  // Its copy here says where it lives now, with Open there.
  await expect(notice(page)).toContainText('Moved to Mac mini. It keeps going there; this copy is read-only.');
  await expect(notice(page).getByRole('button', { name: 'Open on Mac mini' })).toBeVisible();
  // One row, at its new home, badged moved; this Mac's row is hidden.
  const rows = quickChats(page).locator(`[data-session-key="${TRIP}"]`);
  await expect(rows).toHaveCount(1);
  await expect(rows.getByRole('img', { name: 'On Mac mini, moved there from another Mac' })).toBeVisible();
  await expect(rows).toContainText('mini · moved');
  await page.locator('.chat-move-notice').screenshot({ path: info.outputPath('move-notice-moved.png'), style: QUIET });
  await quickChats(page).screenshot({ path: info.outputPath('quick-chats-moved.png') });
  // A moved chat can't move again.
  await page.keyboard.press('Escape');
  menu = await conversationMenu(page);
  await expect(menu.getByRole('menuitem', { name: 'Move to runner (moved)', exact: true })).toBeDisabled();
  await page.keyboard.press('Escape');
  // A refusal says why, and nothing moves.
  await openQuick(page, 'Old report');
  await page.evaluate(() => { window.__mockPeers!.nextMove = { outcome: 'refused', message: 'Mac mini doesn’t have this chat’s project yet. Sync the vault there first.' }; });
  menu = await conversationMenu(page);
  await menu.getByRole('menuitem', { name: 'Move to runner', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Move', exact: true }).click();
  await expect(page.getByText('Mac mini doesn’t have this chat’s project yet. Sync the vault there first.')).toBeVisible();
  await expect(notice(page)).toHaveCount(0);
});

test('a moving chat: Keep it here, Release here behind its confirm, and Forget says what it releases', async ({ page }, info) => {
  const at = new Date().toISOString();
  await startWithQuickChats(page, { preset: 'demo', homes: [
    { key: TRIP, state: 'leaving', peer: MINI, at, waiting: false },
    { key: REPORT, state: 'away', peer: MINI, at, waiting: false },
  ] });
  await openQuick(page, 'Plan the trip');
  await expect(notice(page)).toContainText('Moving to Mac mini. It runs on neither Mac until Mac mini answers.');
  await page.locator('.chat-move-notice').screenshot({ path: info.outputPath('move-notice-moving.png'), style: QUIET });
  // The other Mac doesn't answer: Keep it here waits for it.
  await page.evaluate(id => { window.__mockPeers!.machines.find(m => m.machineId === id)!.state = 'offline'; }, MINI);
  await notice(page).getByRole('button', { name: 'Keep it here' }).click();
  await expect(notice(page)).toContainText('Waiting for Mac mini to answer.');
  await page.locator('.chat-move-notice').screenshot({ path: info.outputPath('move-notice-waiting.png'), style: QUIET });
  // Release here asks first and says Mac mini may keep a copy; Cancel changes nothing.
  await notice(page).getByRole('button', { name: 'Release here…' }).click();
  const release = page.getByRole('dialog', { name: 'Release “Plan the trip” here?' });
  await expect(release).toContainText('If Mac mini already took it, Mac mini keeps its copy too');
  await release.screenshot({ path: info.outputPath('release-confirm.png') });
  await release.getByRole('button', { name: 'Cancel' }).click();
  await expect(notice(page)).toContainText('Waiting for Mac mini to answer.');
  await notice(page).getByRole('button', { name: 'Release here…' }).click();
  await page.getByRole('dialog', { name: 'Release “Plan the trip” here?' }).getByRole('button', { name: 'Release here' }).click();
  await expect(page.getByText('Released: the chat runs on this Mac again.')).toBeVisible();
  await expect(notice(page)).toHaveCount(0);
  expect(await page.evaluate(() => window.__mockPeers!.moves!.map(m => m.command))).toEqual(['chat_move_keep_here', 'chat_move_release_here']);

  // Forget says which moving chats it releases and that a moved one stays there.
  await page.evaluate(([key, peer]) => { window.__mockPeers!.homes!.push({ key, state: 'leaving', peer, at: new Date().toISOString(), waiting: false }); }, [TRIP, MINI] as const);
  await page.evaluate(async () => (await import('/src/store/runtime.ts')).requestOpenSettingsSection('devices'));
  await page.getByRole('navigation', { name: 'Shared tools' }).getByRole('button', { name: 'Settings', exact: true }).click();
  await page.locator('.machines-panel').getByRole('button', { name: 'Forget Mac mini' }).click();
  const forget = page.getByRole('dialog', { name: 'Forget Mac mini?' });
  await expect(forget).toContainText('One chat is still moving there: “Plan the trip”. It runs on this Mac again; Mac mini may keep a copy if it took it.');
  await expect(forget).toContainText('One chat that moved there stays there, shown here as moved to a Mac you forgot.');
  await forget.screenshot({ path: info.outputPath('forget-confirm.png') });
  await forget.getByRole('button', { name: 'Forget' }).click();
  await expect(strip(page)).toHaveCount(0);
  expect(await page.evaluate(() => window.__mockPeers!.homes!.map(h => [h.key, h.state]))).toEqual([['s-20261115-qqqq0002', 'away']]);
  // The chat that moved there shows here as moved to a Mac you forgot.
  await openQuick(page, 'Old report');
  await expect(notice(page)).toContainText('Moved to a Mac you forgot. It runs there; pair that Mac again to open it here.');
  await expect(quickChats(page).locator(`[data-session-key="${REPORT}"]`).getByRole('img', { name: 'Moved to a Mac you forgot' })).toBeVisible();
  await page.locator('.chat-move-notice').screenshot({ path: info.outputPath('move-notice-forgotten.png'), style: QUIET });
});

test('an offline copy: its badge and kept reason, and Continue here offline on the moved chat', async ({ page }, info) => {
  const at = new Date().toISOString();
  const why = 'One of its offline turns is larger than a sync carries (16 MiB), so it stays on this Mac.';
  // M5 offline UI: "Old report" moved to the mini; "Plan the trip" is its
  // offline copy, kept here because a turn was too large to sync.
  await startWithQuickChats(page, { preset: 'demo',
    homes: [{ key: REPORT, state: 'away', peer: MINI, at, waiting: false }],
    offline: [{ offlineKey: TRIP, key: REPORT, peer: MINI, state: 'kept', note: why, partly: false }] });
  await openQuick(page, 'Plan the trip');
  await expect(notice(page).getByRole('img', { name: 'Offline copy of a chat that moved to another Mac' })).toBeVisible();
  await expect(notice(page)).toContainText(`Kept on this Mac: ${why} It is an ordinary chat here now.`);
  await expect(notice(page).getByRole('button', { name: 'Open the original' })).toBeVisible();
  await page.locator('.chat-move-notice').screenshot({ path: info.outputPath('offline-notice-kept.png'), style: QUIET });
  // The moved chat says where its copy went and offers a new one.
  await openQuick(page, 'Old report');
  await expect(notice(page)).toContainText(`Your offline copy stayed on this Mac: ${why}`);
  await notice(page).getByRole('button', { name: 'Continue here offline…' }).click();
  const ask = page.getByRole('dialog', { name: 'Continue “Old report” here offline?' });
  await expect(ask).toContainText('the copy’s new turns sync there');
  await ask.screenshot({ path: info.outputPath('offline-continue-confirm.png') });
  await ask.getByRole('button', { name: 'Continue here', exact: true }).click();
  await expect(page.getByText('“Old report” continues here offline.')).toBeVisible();
  expect(await page.evaluate(() => window.__mockPeers!.offline!.map(r => [r.key, r.state]))).toEqual([
    ['s-20261115-qqqq0002', 'kept'], ['s-20261115-qqqq0002', 'open'],
  ]);
});

test('Settings, Macs: the sleep move is off until turned on, and saves either way', async ({ page }, info) => {
  await start(page, 'demo');
  await page.evaluate(async () => (await import('/src/store/runtime.ts')).requestOpenSettingsSection('devices'));
  await page.getByRole('navigation', { name: 'Shared tools' }).getByRole('button', { name: 'Settings', exact: true }).click();
  const panel = page.locator('.machines-panel');
  const sleep = panel.getByRole('switch', { name: 'Move running chats to the runner when this Mac sleeps' });
  await expect(sleep).toHaveAttribute('aria-checked', 'false');
  await expect(panel.getByText('Off. When on, running chats go to Mac mini, and its logins pay for them.')).toBeVisible();
  await panel.locator('.machines-move-row').screenshot({ path: info.outputPath('settings-move-on-sleep.png') });
  const saved = () => page.evaluate(() => JSON.parse(localStorage.getItem('mockAgentSettings') ?? '{}').moveOnSleep);
  await sleep.click();
  await expect(sleep).toHaveAttribute('aria-checked', 'true');
  await expect(panel.getByText('On: running chats go to Mac mini when this Mac sleeps; its logins pay.')).toBeVisible();
  expect(await saved()).toBe(true);
  // Off is sent too: a save without it would keep the old choice.
  await sleep.click();
  await expect(sleep).toHaveAttribute('aria-checked', 'false');
  expect(await saved()).toBe(false);
});
