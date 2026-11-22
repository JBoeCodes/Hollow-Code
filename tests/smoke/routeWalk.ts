/**
 * Walking the route ledger (plan completion L4): find a target by role and
 * accessible name, click a walk, start from the demo Home with a row's seed.
 * Shared by route-ledger.journey.spec.ts and route-ledger-accelerators.spec.ts.
 */
import { expect, type Locator, type Page } from './fixtures/test';
import type { Route, Seed, Target } from '../../src/routeLedger';
import type { ActionDef } from '../../src/actionDefs';
import { seedSubagents } from './subagentsFixture';

export const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** The visible element a target names. */
export function locate(page: Page, x: Target): Locator {
  const scope: Page | Locator = x.in ? locate(page, x.in) : page;
  const found = x.role === 'text' ? scope.getByText(x.name, { exact: true })
    : x.role === 'label' ? scope.getByLabel(x.name, { exact: true })
    : scope.getByRole(x.role as Parameters<Page['getByRole']>[0], x.match === 'prefix' ? { name: new RegExp(`^${escape(x.name)}`) }
      : x.match === 'contains' ? { name: new RegExp(escape(x.name)) } : { name: x.name, exact: true });
  return found.filter({ visible: true }).first();
}

/** In the sidebar: the rail's destinations, its workspace tree, Settings. */
const SIDEBAR = new Set(['Main', 'Project navigation', 'Shared tools', 'Main navigation']);
const inSidebar = (x: Target): boolean => (!!x.in && (SIDEBAR.has(x.in.name) || inSidebar(x.in)));

/** Whether `target` shows up within `ms` (a page change may still be animating). */
async function shows(target: Locator, ms: number): Promise<boolean> {
  return target.waitFor({ state: 'visible', timeout: ms }).then(() => true, () => false);
}

/** The target, shown. A small window's sidebar can be hidden behind
 *  "Show sidebar" (a click, like any other on the path); says whether it
 *  had to open it. */
export async function reach(page: Page, x: Target): Promise<{ target: Locator; opened: boolean }> {
  const target = locate(page, x);
  if (await shows(target, x.orElse ? 2500 : 3000) || !inSidebar(x)) return { target, opened: false };
  const show = page.getByRole('button', { name: 'Show sidebar', exact: true });
  if (await show.isVisible()) { await show.click(); return { target, opened: true }; }
  return { target, opened: false };
}

/** States the demo Home lacks: set before the app loads, or (`after`)
 *  through the mock API once it has. */
export const SEEDS: Record<Seed, ((page: Page) => Promise<void>) | { after: (page: Page) => Promise<void> }> = {
  'explicit-appearance': page => page.addInitScript(() => localStorage.setItem('theme', 'light')),
  'conflict-copies': page => page.addInitScript(() => {
    localStorage.setItem('mockNeedsYouExtra', JSON.stringify([
      { id: 'copies:vault', kind: 'copies', sourceId: 'vault', title: '2 conflict copies in the vault', detail: 'notes/Ideas (1).md (same as the original); notes/Plan 2.md', state: 'pending', at: null, workspaceId: null, sessionKey: null, path: null, category: null, link: null },
    ]));
  }),
  // A context sync clash on the personal root (CX6c): its Waiting on you
  // item and the mock's clash card behind it.
  'sync-clash': page => page.addInitScript(() => {
    localStorage.setItem('mockSyncClash', '1');
    localStorage.setItem('mockNeedsYouExtra', JSON.stringify([
      { id: 'clash:personal:me/Identity.md', kind: 'clash', sourceId: 'personal:me/Identity.md', title: 'Two Macs changed Identity.md', detail: "This Mac kept its version of me/Identity.md; studio-mac's is beside it as me/Identity (from studio-mac).md.", state: 'pending', at: '2026-11-15T09:00:00Z', workspaceId: null, sessionKey: null, path: 'me/Identity.md', category: 'context-sync', link: null },
    ]));
  }),
  // An issue the tracker changed after you edited it (the mock tracker's seed).
  'tracker-conflict': page => page.addInitScript(() => {
    localStorage.setItem('mockTracker', JSON.stringify({ conflicts: { 'i-32': [{ field: 'priority', local: '1', remote: '3', at: '2026-11-11T09:00:00Z' }] } }));
  }),
  // Another Mac paired and online, with its chats (mockPeers.ts's demo, M2).
  'paired-mac': page => page.addInitScript(() => localStorage.setItem('mockPeers', 'demo')),
  // The same, and Move to runner's next answer is "waiting" (M5): the chat is then moving.
  'moving-chat': page => page.addInitScript(() => localStorage.setItem('mockPeers', JSON.stringify({ preset: 'demo', nextMove: { outcome: 'waiting', message: 'Mac mini hasn’t answered. The chat runs on neither Mac until it does.' } }))),
  // The demo's Q3 conversations started two active and three done subagents.
  subagents: seedSubagents,
  // A saved canvas in the first demo workspace (as J6 makes one).
  'saved-canvas': { after: page => page.evaluate(async () => {
    const path = '/src/api.ts', { api } = await import(/* @vite-ignore */ path);
    const ws = (await api.listWorkspaces()).find((w: { kind?: string; id: string }) => !w.id.startsWith('_') && w.kind !== 'build') as { id: string };
    await api.recordCanvas({ workspaceId: ws.id, sessionKey: 's-ledger' }, '<hc-canvas id="release-status" title="Release status">\n  <p>Build 1</p>\n</hc-canvas>', null);
  }) },
};

export async function home(page: Page, r: Route) {
  const seed = r.seed ? SEEDS[r.seed] : null;
  if (typeof seed === 'function') await seed(page);
  await page.goto('/?mock=1&demo=1');
  await expect(page.getByRole('region', { name: 'Projects' })).toBeVisible();
  if (seed && typeof seed !== 'function') await seed.after(page);
}

export async function walk(page: Page, steps: readonly Target[]) {
  for (const [i, step] of steps.entries()) {
    const { target, opened } = await reach(page, step);
    if (step.orElse && !(await target.isVisible())) { await walk(page, step.orElse); continue; }
    if (step.fill != null) await target.fill(step.fill);
    else await target.click();
    // An overlay sidebar opened for this step is closed again before a step
    // outside it, as a person would: a destination closes it itself (it
    // slides away), anything else by its own Close. Not under a menu the
    // step just opened.
    const next = steps[i + 1];
    if (opened && next && !inSidebar(next) && !['menuitem', 'option'].includes(next.role)) {
      const close = page.getByRole('complementary', { name: 'Main navigation' }).getByRole('button', { name: 'Close sidebar', exact: true });
      await close.waitFor({ state: 'hidden', timeout: 1500 }).catch(async () => { if (await close.isVisible()) await close.click(); });
      await expect(close).toBeHidden();
    }
  }
}

/** A landmark, or its small-window form. */
export function landmark(page: Page, x: Target): Locator {
  return x.orElse?.length ? locate(page, x).or(locate(page, x.orElse[0])).first() : locate(page, x);
}

/** The walk's opening of a workspace, which its accelerators start from too. */
export const fromWorkspace = (r: Route) => r.walk[0]?.role === 'button' && /^Open /.test(r.walk[0].name) ? [r.walk[0]] : [];

/** An action whose run only shows something: safe to run here. */
export const harmless = (d: ActionDef) => d.effect === 'navigate' || d.effect === 'view' || d.effect === 'draft';
/** Accelerators whose landing a browser can't show, with why (native checks). */
export const NATIVE_ONLY: Record<string, string> = { dictate: 'needs the microphone and macOS speech recognition' };

