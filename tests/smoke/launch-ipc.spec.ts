import { expect, test, type Page } from './fixtures/test';

test.use({ timezoneId: 'America/New_York' });
const FRIDAY = '2026-10-30T16:00:00-04:00';

/**
 * Launch IPC budget (overnight 10-11/12 P3, absorbs B7 part 2): the calls the
 * window makes as it launches, from the mock's own tally, against today's
 * numbers. A new call at launch fails this until it is either deferred or the
 * budget is raised here on purpose, with the reason. The app judges repeats by
 * arguments itself (`src/launchIpc.ts`); the mock counts by command only.
 */
const launchCalls = async (page: Page, url: string, at = FRIDAY) => {
  // The weekly reminder reads only Friday afternoon through Monday morning.
  // Keep Date fixed without stopping the timers used to settle the launch.
  await page.clock.setFixedTime(new Date(at));
  await page.goto(url);
  await page.getByRole('navigation', { name: 'Main' }).waitFor();
  // Settled: the tally hasn't moved for 1.5 s (idle preloads included).
  await page.evaluate(async () => {
    const sum = () => Object.values(window.__mockApiCalls ?? {}).reduce((a, b) => a + b, 0);
    let last = sum(), since = performance.now();
    const until = performance.now() + 12_000;
    while (performance.now() - since < 1500 && performance.now() < until) {
      await new Promise(r => setTimeout(r, 100));
      const now = sum(); if (now !== last) { last = now; since = performance.now(); }
    }
  });
  return page.evaluate(() => ({ ...(window.__mockApiCalls ?? {}) }));
};
const total = (calls: Record<string, number>) => Object.values(calls).reduce((a, b) => a + b, 0);

test.beforeEach(({}, info) => { test.skip(info.project.name.includes('narrow'), 'one layout is enough for a call count'); });

// Weekly review was cut (overnight 11-08 S3): no launch reads it, even
// inside its old Friday-to-Monday reminder window.
for (const [day, at] of [
  ['Friday', FRIDAY],
  ['Tuesday', '2026-11-03T12:00:00-05:00'],
] as const) test(`a plain ${day} launch stays within its IPC budget`, async ({ page }) => {
  const calls = await launchCalls(page, '/?mock=1', at);
  // 2026-10-12: 32 calls over 28 commands; 12 of them push saved switches to
  // the backend, which keeps them in memory (store/runtime.ts).
  // 2026-10-21: two shared 128px reads restore app and Main agent identity.
  // Originals/editor state are excluded; mounting more indicators adds no calls.
  expect(calls.read_custom_icon).toBe(2);
  // Pending-install and quit reads restore their controls after a webview reload.
  // Rechecked against unchanged d3c47f0 on 2026-10-30: Home also reads
  // recently deleted projects and the weekly-review badge (38 calls / 33 commands).
  // Project main keys add fields to existing replies, not startup requests.
  expect(calls.trashed_workspaces).toBe(1);
  expect(calls.weekly_review ?? 0).toBe(0);
  // 2026-11-13: raised by one call and one command on purpose: #1358 keeps each
  // chat's own effort in Rust, and the frontend's per-chat effort cache loads it
  // once at startup (`chat_efforts_list`), 39 calls / 34 commands.
  // 2026-11-15: raised by one call and one command on purpose: M2's machine
  // strip reads `peer_machines` once at launch to know whether another Mac is
  // paired (it shows nothing until one is), 40 calls / 35 commands. That read
  // is the local record only (no `check`, no ssh); tunnels open only for a
  // paired Mac, after first paint. No existing launch read carries the flag.
  // Main was already over the old limit at e3ff9b222 (41 of 39) for its own reasons.
  // 2026-11-15 (browser fix batch FB): raised to 41 calls / 36 commands on purpose.
  // The only launch commands new since 11-13 besides `peer_machines` are U5 PR 2's
  // top-bar Update button reads, `update_get_state` and `update_last_result`
  // (the update it offers, and the last update's result note), each once and
  // each from the update client's memory, no network. Measured 41 / 36 on main 1359983f0.
  // 2026-11-17: raised by one call and one command on purpose: U3 (#1502) reads
  // `list_conversation_organization` once at launch, so the rail knows which
  // chats are pinned, snoozed or settled before it groups them. It read twice
  // (at the rail's mount and again after the first sessions read); the rail
  // now waits for that first sessions read. Measured 42 / 37 on main 69e47e2ed.
  // 2026-11-18 (browser fix batch b17): raised by one call and one command on
  // purpose: CC2 (#1592) pushes its saved code lookup switch at launch,
  // `set_code_context_mcp_enabled`, once, the same way `set_vault_mcp_enabled`
  // does (the backend keeps switches in memory, store/runtime.ts). It is the
  // only launch command new since 69e47e2ed. Measured 43 / 38 on main dce8428ec.
  // 2026-11-19 (browser fix batch 2): raised by one call and one command on
  // purpose: H6 (#1632) reads `first_run_status` once, 400 ms after the shell
  // paints (main.tsx, firstRunMount.tsx), to know whether a setup step is still
  // open; only then does the sheet's chunk load. It is the only launch command
  // new since dce8428ec. Measured 44 / 39 on main 219975224.
  expect(total(calls), JSON.stringify(calls)).toBeLessThanOrEqual(44);
  expect(Object.keys(calls).length).toBeLessThanOrEqual(39);
  // `set_agent_role` is once per role; nothing else runs more than twice.
  for (const [command, n] of Object.entries(calls)) expect(n, command).toBeLessThanOrEqual(command === 'set_agent_role' ? 4 : 2);
});

test('a large vault launch stays within its IPC budget', async ({ page }) => {
  // Measure the old reminder-window case explicitly as well as the vault size.
  const calls = await launchCalls(page, '/?mock=1&demo=large', FRIDAY);
  expect(calls.weekly_review ?? 0).toBe(0);
  // 2026-10-12: 43 calls over 37 commands (Home's calendar, tasks, commitments,
  // brief and digests for 200 workspaces).
  // 2026-10-13: the plan/activity badges (V6's one extra post-paint read) are retired; no signals read.
  expect(calls.workspace_signals ?? 0).toBe(0);
  expect(total(calls), JSON.stringify(calls)).toBeLessThanOrEqual(45);
  // 2026-11-19 (browser fix batch 2): one command more on purpose, H6's
  // `first_run_status` (#1632, see the plain launch above); calls stay within 45.
  expect(Object.keys(calls).length).toBeLessThanOrEqual(40);
});
