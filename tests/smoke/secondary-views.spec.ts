/**
 * One page title; purpose belongs to the tab's tooltip and accessible
 * description. Data coverage stays visible. Clicks only, against demo data.
 */
import { expect, test, type Page } from './fixtures/test';
import { REVIEW_VIEWS, WORK_VIEWS } from '../../src/store/destinations';
import { ROUTES } from '../../src/routeLedger';
import { home, walk } from './routeWalk';
import { navigateWaiting } from './waiting';

test.beforeEach(({}, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'the journeys cover small windows');
});

/** What each data view says about its source, freshness or coverage. */
const COVERAGE: Record<string, RegExp> = {
  map: /workspaces? in \d+ clusters?.*Open a workspace from its satellite or List row/,
  activity: /Costs can arrive at turn end/,
};

async function demo(page: Page) {
  await page.goto('/?mock=1&demo=1&missions=1');
  await expect(page.getByRole('region', { name: 'Projects' })).toBeVisible();
}

test('Work’s views say what they are for, what they draw on, and the way back', async ({ page }) => {
  await demo(page);
  const tabs = page.getByRole('navigation', { name: 'Projects views' });
  await expect(page.getByRole('region', { name: 'Projects' }).getByRole('button', { name: 'Map', exact: true })).toHaveCount(0);
  for (const v of WORK_VIEWS) {
    const tab = tabs.getByRole('button', { name: v.label, exact: true });
    await tab.click();
    await expect(tab).toHaveAttribute('title', v.hint);
    await expect(tab).toHaveAccessibleDescription(v.hint);
    await expect(page.getByLabel('What this view is for')).toHaveCount(0);
    await expect(page.locator('.canvas-split > .page h1').filter({ visible: true })).toHaveCount(1);
    if (COVERAGE[v.id]) await expect(page.locator('.your-work').getByText(COVERAGE[v.id]).first()).toBeVisible();
    // The way back to the overview is the first tab, always on screen.
    await expect(tabs.getByRole('button', { name: 'Home', exact: true })).toBeVisible();
  }
});

test('Review’s views say what they are for, with the way back to everything waiting', async ({ page }) => {
  await demo(page);
  // The full view behind the bell's See all needs you (U3).
  await navigateWaiting(page);
  const filters = page.getByRole('navigation', { name: 'Waiting filters' });
  for (const v of REVIEW_VIEWS) {
    const tab = filters.getByRole('button', { name: new RegExp(`^${v.label}`) });
    await tab.click();
    await expect(tab).toHaveAttribute('title', v.hint);
    await expect(tab).toHaveAttribute('aria-description', v.hint);
    await expect(page.getByLabel('What this view is for')).toHaveCount(0);
    await expect(page.locator('.canvas-split > .page h1').filter({ visible: true })).toHaveCount(1);
    await expect(filters.getByRole('button', { name: /^All( \d+)?$/ })).toBeVisible();
  }
});

for (const [id, name, header] of [
  ['work.activity', 'Running', '.mc-head'],
  ['library.canvases', 'Results', null],
] as const) {
  test(`${name} keeps its named region without a repeated heading`, async ({ page }) => {
    await page.setViewportSize({ width: 720, height: 600 });
    await page.addInitScript(() => {
      localStorage.setItem('hollow.studio.textScale', '125');
      localStorage.setItem('hollow.studio.motion', 'reduced');
    });
    const route = ROUTES.find(r => r.id === id)!;
    await home(page, route);
    await walk(page, route.walk);
    const region = page.getByRole('region', { name, exact: true });
    await expect(region).toBeVisible();
    await expect(region.getByRole('heading', { name, exact: true })).toHaveCount(0);
    await expect(page.locator('.canvas-split > .page h1').filter({ visible: true })).toHaveCount(1);
    await expect(page.locator('.triage-keys, .mc-keys')).toHaveCount(0);
    if (header) {
      const bounds = await region.locator(header).evaluate(el => {
        const children = [...el.children].map(c => c.getBoundingClientRect()).filter(r => r.width && r.height);
        return { overlap: Math.min(...children.map(r => r.bottom)) - Math.max(...children.map(r => r.top)), right: Math.max(...children.map(r => r.right)), edge: el.getBoundingClientRect().right };
      });
      expect(bounds.overlap, 'header children share one row').toBeGreaterThan(0);
      expect(bounds.right, 'header children stay inside the header').toBeLessThanOrEqual(bounds.edge + 1);
    }
    if (name === 'Commitments' || name === 'Results') {
      const search = region.getByRole('searchbox');
      expect((await search.boundingBox())!.width, 'search keeps usable width').toBeGreaterThanOrEqual(120);
    }
  });
}
