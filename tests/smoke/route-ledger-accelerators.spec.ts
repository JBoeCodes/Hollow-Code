/**
 * The route ledger's accelerators land where its walks do (plan completion
 * L4, 2026-10-13; matrix N7/N10). Every palette row, `g` key and ⌘ key of a
 * shared action (src/actionDefs.ts) is run from the row's starting point and
 * must show the row's landmark (or the landing the row names for it): an
 * alias leads to the same experience, through the one dispatcher. Actions
 * that change or send something (write, run, external) are not run; a
 * conditional one is checked for the words it says when it can't run.
 * Desktop only: the walks themselves run at every size.
 */
import { expect, test, type Page } from './fixtures/test';
import { openGlobalSearch } from './choose';
import { ROUTES, accelLanding, type Route } from '../../src/routeLedger';
import { ACTION_DEFS, actionLabel } from '../../src/actionDefs';
import { GO_TARGET_BY_ID } from '../../src/keyboardLeader';
import { COMMAND_BY_ID } from '../../src/shortcuts';
import { NATIVE_ONLY, escape, fromWorkspace, harmless, home, landmark, walk } from './routeWalk';

/** Palette rows whose label says their state (CommandPalette.tsx PALETTE_OWN). */
const OWN_WORDS: Record<string, { query: string; row: RegExp }> = {
  'act:toggle-side-pane': { query: 'preview pane', row: /^(Hide|Show) preview pane/ },
};

/** The Preview pane's condition (run 15 P1): the route's own walk opens an
 *  item and Close hides it, so the toggle has something to bring back. */
async function previewToBringBack(page: Page, r: Route) {
  await walk(page, r.walk.slice(fromWorkspace(r).length));
  const pane = page.getByRole('complementary', { name: 'Preview' });
  await pane.getByRole('button', { name: 'Close preview', exact: true }).click();
  await expect(pane).toHaveCount(0);
}

test.beforeEach(({}, info) => {
  test.skip(info.project.name !== 'chromium-desktop', 'accelerators are checked once, at desktop; the walks run at every size');
});

test.describe('route ledger accelerators land where the walk does', () => {
  for (const r of ROUTES) {
    // A conditional action is run where its condition holds: in the row's
    // workspace for the workspace conditions; nothing is being sent at rest.
    const holds = new Set(['can-leave', ...(fromWorkspace(r).length ? ['workspace-open', 'side-pane', 'conversation-agent', 'plan-open'] : [])]);
    const defs = ACTION_DEFS.filter(d => d.route === r.id && harmless(d) && (!d.when || holds.has(d.when)) && !NATIVE_ONLY[d.id]);
    for (const d of defs.filter(d => d.palette && !d.paletteIn)) {
      test(`palette ${d.palette} → ${r.id}`, async ({ page }) => {
        // Clipboard permission is not part of route discovery. Keep the
        // browser fixture local and still verify the exact copied payload.
        if (d.id === 'copy-build') await page.addInitScript(() => {
          Object.defineProperty(navigator, 'clipboard', { configurable:true, value: {
            writeText:async (text:string) => { (window as unknown as { routeCopiedBuild:string }).routeCopiedBuild = text; },
          } });
        });
        await home(page, r);
        await walk(page, fromWorkspace(r));
        if (d.when === 'side-pane') await previewToBringBack(page, r);
        // The palette from the sidebar's search entry (the top bar's Search anything went in U1).
        const palette = await openGlobalSearch(page);
        // A row that names its state (PALETTE_OWN labels) is found by its words.
        const own = OWN_WORDS[d.palette!];
        await palette.getByRole('combobox', { name: 'Search' }).fill(own?.query ?? actionLabel(d));
        await palette.getByRole('option', { name: own?.row ?? new RegExp(`^${escape(actionLabel(d))}`) }).first().click();
        await expect(landmark(page, accelLanding(r, d.palette!))).toBeVisible();
        if (d.id === 'copy-build') expect(await page.evaluate(() => (window as unknown as { routeCopiedBuild:string }).routeCopiedBuild))
          .toBe('Hollow 0.5.3 · commit f1x7ure · built fixture · source mock…');
      });
    }
    for (const d of defs.filter(d => d.go)) {
      test(`go ${d.go} → ${r.id}`, async ({ page }) => {
        await home(page, r);
        await walk(page, fromWorkspace(r));
        await page.locator('body').click({ position: { x: 1, y: 1 } });
        await page.keyboard.press('g');
        await page.keyboard.press(GO_TARGET_BY_ID[d.go!].key);
        await expect(landmark(page, accelLanding(r, d.go!))).toBeVisible();
      });
    }
    for (const d of defs.filter(d => d.key && COMMAND_BY_ID[d.key]?.default.startsWith('CmdOrCtrl+') && !COMMAND_BY_ID[d.key].global)) {
      test(`key ${d.key} → ${r.id}`, async ({ page }) => {
        await home(page, r);
        await walk(page, fromWorkspace(r));
        if (d.when === 'side-pane') await previewToBringBack(page, r);
        await page.locator('body').click({ position: { x: 1, y: 1 } });
        await page.keyboard.press(COMMAND_BY_ID[d.key!].default.replace('CmdOrCtrl', 'Meta'));
        await expect(landmark(page, accelLanding(r, d.key!))).toBeVisible();
      });
    }
  }
});

test.describe('a shared action that can’t run says why', () => {
  // actionDefs.ts CONDITIONS: the words a menu item or key gets instead of
  // silently doing nothing.
  for (const [id, chord, words] of [
    ['go-back', 'Meta+[', 'Nothing to go back to yet.'],
    ['toggle-side-pane', 'Meta+\\', 'a file or a link first'],
  ] as const) {
    test(`${id} with its condition unmet`, async ({ page }) => {
      await page.goto('/?mock=1&demo=1');
      await expect(page.getByRole('region', { name: 'Projects' })).toBeVisible();
      await page.locator('body').click({ position: { x: 1, y: 1 } });
      await page.keyboard.press(chord);
      await expect(page.getByText(words).first()).toBeVisible();
    });
  }
});
