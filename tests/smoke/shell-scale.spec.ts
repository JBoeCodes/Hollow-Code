/**
 * The shell at a real library's size (review U "Shell and rail": the native
 * library had 213 workspaces; plan completion L4, matrix X14). 213
 * workspaces, with the same name in several places (a project folder, a
 * Hollow workspace, a Notion mirror, a build) and twice in the same place,
 * and titles far longer than a row. Work's Find, the top bar's Search,
 * the sidebar and Every workspace each tell the same-named ones apart; a
 * long title's row carries its full name and origin; Find can narrow by
 * where a workspace comes from. Clicks and typing only.
 */
import { expect, test, type Page } from './fixtures/test';
import { openGlobalSearch } from './choose';

test.beforeEach(({}, info) => {
  test.skip(info.project.name !== 'chromium-desktop', 'a desktop check of scale; J1 covers same-named workspaces at the small sizes');
});

const at = (m: number) => new Date(Date.now() - m * 60_000).toISOString();
const vault = (id: string) => `/Users/fixture/vault/workspaces/${id}`;
const ws = (id: string, title: string, cwd: string, minutesAgo: number, extra: Record<string, unknown> = {}) =>
  ({ id, title, status: 'active', path: `workspaces/${id}`, cwd, modified: at(minutesAgo), ...extra });
const LONG = 'Quarterly regional operations review for the Northeast, Mid-Atlantic and Southeast parking portfolios, with the lease renewals';
const WORKSPACES = [
  ws('local-q3-folder', 'Q3 review', '/Users/fixture/Projects/q3-review', 50),
  ws('q3-hollow', 'Q3 review', vault('q3-hollow'), 40),
  ws('notion-0123456789abcdef0123456789abcdef', 'Q3 review', vault('notion-q3'), 30),
  ws('q3-build', 'Q3 review', '/Users/fixture/Hollow/Worktrees/q3/q3-review', 20, { build: { repo: '/Users/fixture/Projects/q3', worktree: '/Users/fixture/Hollow/Worktrees/q3/q3-review', branch: 'feature/q3-review', base: 'origin/main', exists: true } }),
  // The same name twice in the same place: told apart by folder.
  ws('budget-east', 'Budget', vault('budget-east'), 60),
  ws('budget-west', 'Budget', vault('budget-west'), 70),
  ws('long-hollow', LONG, vault('long-hollow'), 15),
  ...Array.from({ length: 206 }, (_, i) => ws(`scale-${String(i).padStart(3, '0')}`, `Location ${100 + i} operating plan`, vault(`scale-${i}`), 100 + i, i % 9 === 0 ? { status: 'done' } : {})),
];

async function seed(page: Page) {
  await page.addInitScript(workspaces => {
    if (sessionStorage.getItem('scaleSeeded')) return;
    sessionStorage.setItem('scaleSeeded', '1');
    localStorage.removeItem('hollow.workspace');
    localStorage.setItem('mockWorkspaces', JSON.stringify(workspaces));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify([]));
    localStorage.setItem('hollow.railWorkspaces', JSON.stringify(['local-q3-folder', 'q3-hollow', 'notion-0123456789abcdef0123456789abcdef', 'q3-build', 'budget-east', 'budget-west', 'long-hollow']));
  }, WORKSPACES);
  await page.goto('/?mock=1');
}

test('213 workspaces: Find, Search, the sidebar and Every workspace tell same-named ones apart', async ({ page }) => {
  expect(WORKSPACES).toHaveLength(213);
  await seed(page);
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true }).click();
  const list = page.getByRole('region', { name: 'Projects' });
  await list.getByRole('button', { name: 'All projects' }).click();
  await list.getByRole('checkbox', { name: 'Include archived' }).check();
  await expect(list.locator('.work-card')).toHaveCount(213);
  await list.locator('.work-card').last().scrollIntoViewIfNeeded();
  await expect(list.locator('.work-card').last()).toBeVisible();


  // Find "Q3 review": four rows, each saying where it comes from.
  const find = list.getByRole('searchbox', { name: 'Find a project' });
  await find.fill('Q3 review');
  for (const origin of ['Project folder', 'Hollow workspace', 'Notion project', 'Build · feature/q3-review']) {
    await expect(list.getByRole('button', { name: new RegExp(`^Actions for Q3 review \\(${origin.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`) })).toBeVisible();
  }
  // …and narrows by where: "q3 notion" leaves the Notion mirror alone.
  await find.fill('q3 notion');
  await expect(list.locator('.work-card')).toHaveCount(1);
  await expect(list.getByRole('button', { name: 'Actions for Q3 review (Notion project)' })).toBeVisible();

  // Twice in the same place: the folder says which.
  await find.fill('Budget');
  await expect(list.getByRole('button', { name: 'Actions for Budget (Hollow workspace · budget-east)' })).toBeVisible();
  await expect(list.getByRole('button', { name: 'Actions for Budget (Hollow workspace · budget-west)' })).toBeVisible();

  // A long title: the row shows what fits; its tooltip has the whole name and origin.
  await find.fill('Quarterly regional');
  await expect(list.locator('.work-card').first()).toHaveAttribute('title', `${LONG} (Hollow workspace)`);

  // The sidebar's Search (the top bar's until U1): the same four, told apart in the jump list.
  const palette = await openGlobalSearch(page);
  await palette.getByRole('combobox', { name: 'Search' }).fill('Q3 review');
  // The four workspaces, not their Plan, Results, Brief and Sources rows
  // ("Q3 review › Plan", 10-25), which made this count 20.
  const rows = palette.getByRole('option', { name: /^Q3 review(?! ›)/ });
  await expect(rows).toHaveCount(4);
  const hints = (await rows.allInnerTexts()).map(t => t.replace(/\s+/g, ' '));
  for (const origin of ['Project folder', 'Hollow workspace', 'Notion project', 'Build · feature/q3-review']) {
    expect(hints.some(h => h.includes(origin)), origin).toBe(true);
  }
  await page.keyboard.press('Escape');

  // The sidebar: the same names, each with its origin.
  const rail = page.getByRole('navigation', { name: 'Project navigation' });
  for (const name of ['Q3 review, Notion project', 'Budget, Hollow workspace · budget-west']) {
    await expect(rail.getByRole('button', { name, exact: true })).toBeVisible();
  }
});

test('the visible All projects route expands Home without hiding pins or menus', async ({ page }) => {
  await seed(page);
  // The sidebar's Projects row, then the page's All projects (L1, 11-12:
  // the sidebar's own All projects row went).
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', {name: /^Projects/}).click();
  const list = page.getByRole('region', {name:'Projects'});
  await list.getByRole('button', {name:'All projects', exact: true}).click();
  await expect(list.getByRole('searchbox', {name:'Find a project'})).toBeFocused();
  // Archived ones are a check away (the sidebar row included them).
  await list.getByRole('checkbox', {name:'Include archived'}).check();
  await expect(list.locator('.work-card')).toHaveCount(213);
  await expect(list.getByRole('button', {name:'Show recent projects'})).toHaveAttribute('aria-expanded','true');
  await list.getByRole('button', {name:'Show recent projects'}).click();
  await expect(list.locator('.work-card')).toHaveCount(5);
  await list.getByRole('button', {name:'All projects'}).click();
  await list.getByRole('checkbox', {name:'Include archived'}).check();
  await list.getByRole('searchbox', {name:'Find a project'}).fill('Location');
  await expect(list.locator('.work-card')).toHaveCount(206);
  await expect(list.getByRole('button', {name:'Show recent projects'})).toHaveAttribute('aria-expanded', 'true');
  await list.getByRole('searchbox', {name:'Find a project'}).fill('');
  await expect(list.locator('.work-card')).toHaveCount(213);
  await expect(list.getByRole('button', {name:'Actions for Budget (Hollow workspace · budget-west)'})).toBeAttached();

});
