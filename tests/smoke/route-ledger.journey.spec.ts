/**
 * The route ledger, walked (plan completion L4, 2026-10-13; matrix N5).
 * Every row of src/routeLedger.ts is clicked from the demo Home exactly as
 * its `walk` says, by role and accessible name, and its `lands` landmark must
 * then be on screen. Removing or renaming a route's control fails its row,
 * so the ledger records reachable capabilities, not test titles.
 *
 * A required spec: it runs at desktop, the native minimum window and
 * Largest text, and may not skip (scripts/check-required-specs.mjs). The
 * accelerators (palette, `g`, ⌘) are checked against the same landmarks by
 * route-ledger-accelerators.spec.ts.
 */
import { expect, test } from './fixtures/test';
import { ROUTES } from '../../src/routeLedger';
import { home, landmark, walk } from './routeWalk';

test.describe('route ledger walks', () => {
  for (const r of ROUTES) {
    test(`walk ${r.id}: ${r.path}`, async ({ page }) => {
      await home(page, r);
      await walk(page, r.walk);
      await expect(landmark(page, r.lands)).toBeVisible();
    });
  }
});

