import { expect, test, type Page } from './fixtures/test';

/**
 * Canvas C6 P3: measurements, not checks. Skipped unless HOLLOW_MEASURE=1:
 *
 *   HOLLOW_MEASURE=1 npx playwright test tests/smoke/canvas-scale-measure.spec.ts --project=chromium-desktop --workers=1
 *
 * 1. First paint of a conversation with 30 canvas replies, against the same
 *    conversation with 30 plain replies: time from opening it until all 30
 *    replies are in the transcript, until the first canvas frame is live and
 *    sized, main-thread long tasks in the first 4 s, and live frames.
 * (A second measure, the workspace panel sections' re-read on one
 * `vault-changed`, went with the panel in the COO rebuild L4.)
 * Numbers print as `MEASURE …` lines; medians of five runs.
 */

test.skip(!process.env.HOLLOW_MEASURE, 'measurement only: HOLLOW_MEASURE=1');
test.beforeEach(({}, info) => { test.skip(info.project.name !== 'chromium-desktop', 'one viewport'); });
test.describe.configure({ timeout: 180_000 });

async function installEventBridge(page: Page) {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (event: unknown) => void>();
    const listeners = new Map<string, number[]>();
    let next = 1;
    const w = window as unknown as Record<string, unknown>;
    w.__TAURI_INTERNALS__ = {
      transformCallback(callback: (event: unknown) => void) { const id = next++; callbacks.set(id, callback); return id; },
      invoke(command: string, args: { event?: string; handler?: number }) {
        if (command === 'plugin:event|listen' && args?.event && args.handler) {
          listeners.set(args.event, [...(listeners.get(args.event) ?? []), args.handler]);
          return Promise.resolve(next++);
        }
        if (command === 'plugin:event|unlisten') return Promise.resolve();
        return Promise.reject(new Error(`not in Tauri: ${command}`));
      },
    };
    w.__emitTauriEvent = (event: string, payload: unknown) => { for (const id of listeners.get(event) ?? []) callbacks.get(id)?.({ event, id, payload }); };
  });
}

const median = (xs: number[]) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];
const N = 30;
const canvas = (n: number) => `\`\`\`hollow-canvas
<hc-canvas id="lane-${n}" title="Lane ${n}">
  <hc-tabs label="Views">
    <section label="Summary"><div style="height:200px">Lane ${n} summary</div></section>
    <section label="Detail"><div style="height:200px">Lane ${n} detail</div></section>
  </hc-tabs>
</hc-canvas>
\`\`\``;

async function seedConversation(page: Page, kind: 'canvas' | 'plain') {
  const at = '2026-10-05T09:00:00.000Z', key = 's-20261005-scale001', cwd = '/Users/fixture/vault/workspaces/alpha-1';
  const session = { key, title: 'Thirty replies', agent: 'claude', harnessSessionId: 'sess-native', cwd, createdAt: at, lastActivityAt: at, endedAt: null, turnCount: N, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'native' };
  const turns = Array.from({ length: N }, (_, i) => [
    { id: `${key}-u${i}`, role: 'user', text: `Show lane ${i + 1}`, tools: [], agent: 'claude', runId: `run-${i}`, startedAt: at, transcriptIncomplete: false },
    { id: `${key}-a${i}`, role: 'assistant', text: kind === 'canvas' ? `Lane ${i + 1}:\n\n${canvas(i + 1)}` : `Plain reply ${i + 1}\n\n| Lane | Summary |\n| --- | --- |\n| ${i + 1} | ok |`, tools: [], agent: 'claude', runId: `run-${i}`, startedAt: at, status: 'success', transcriptIncomplete: false },
  ]).flat();
  await page.addInitScript(([seed]) => {
    if (sessionStorage.getItem('scaleSeeded')) return;
    sessionStorage.setItem('scaleSeeded', '1');
    for (const [k, v] of Object.entries(seed as Record<string, unknown>)) localStorage.setItem(k, JSON.stringify(v));
  }, [{
    'hollow.railWorkspaces': ['alpha-1'],
    mockWorkspaces: [{ id: 'alpha-1', title: 'Alpha', status: 'active', path: 'workspaces/alpha-1', cwd, modified: at }],
    mockWorkspaceSessions: [session],
    mockWorkspaceConversations: { [key]: { summary: session, eventsRetained: true, turns } },
  }] as const);
}

async function openOnce(page: Page, kind: 'canvas' | 'plain') {
  await seedConversation(page, kind);
  await page.goto('/?mock=1');
  // One click opens it: Work's Open Alpha, its only (so newest) conversation
  // (the sidebar lists no conversations since L8, 11-10).
  const open = page.getByRole('button', { name: 'Open Alpha', exact: true });
  await expect(open).toBeVisible();
  await page.waitForTimeout(500);
  await page.evaluate(kind => {
    const m = { t0: performance.now(), long: 0, all: -1, frame: -1, frames: 0 };
    (window as unknown as { __m: typeof m }).__m = m;
    new PerformanceObserver(list => { for (const e of list.getEntries()) if (e.startTime >= m.t0 && e.startTime < m.t0 + 4000) m.long += e.duration; }).observe({ type: 'longtask' });
    const tick = () => {
      const t = performance.now() - m.t0;
      const root = document.querySelector('.work-conversation');
      if (root && m.all < 0 && (kind === 'canvas' ? root.querySelector(`.chat-canvas[data-canvas-id="lane-${30}"]`) : root.textContent?.includes('Plain reply 30'))) m.all = t;
      const live = root?.querySelectorAll<HTMLIFrameElement>('.chat-artifact-slot[data-frame-live="true"] iframe') ?? [];
      if (m.frame < 0 && [...live].some(f => f.getBoundingClientRect().height > 50)) m.frame = t;
      m.frames = live.length;
      if (t < 8000) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }, kind);
  await open.click();
  await page.waitForTimeout(kind === 'canvas' ? 6000 : 4500);
  return page.evaluate(() => (window as unknown as { __m: { all: number; frame: number; long: number; frames: number } }).__m);
}

for (const kind of ['plain', 'canvas'] as const) {
  test(`first paint: a conversation of ${N} ${kind} replies`, async ({ browser }) => {
    const runs = [];
    for (let i = 0; i < 5; i++) {
      const context = await browser.newContext({ viewport: { width: 1440, height: 920 } });
      const page = await context.newPage();
      await installEventBridge(page);
      runs.push(await openOnce(page, kind));
      await context.close();
    }
    const line = `MEASURE first-paint ${kind}: all ${N} replies in ${median(runs.map(r => r.all)).toFixed(0)} ms; first live frame ${kind === 'canvas' ? `${median(runs.map(r => r.frame)).toFixed(0)} ms` : 'n/a'}; long tasks in 4 s ${median(runs.map(r => r.long)).toFixed(0)} ms; live frames ${median(runs.map(r => r.frames))} (runs: ${JSON.stringify(runs.map(r => [Math.round(r.all), Math.round(r.frame), Math.round(r.long), r.frames]))})`;
    console.log(line);
    expect(runs.every(r => r.all >= 0)).toBe(true);
  });
}
