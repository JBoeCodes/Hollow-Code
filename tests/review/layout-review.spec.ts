import { expect, test, type Page } from '@playwright/test';
import { chooseNew, openProjectFiles, previewPane } from '../smoke/choose';
import { seedSubagents } from '../smoke/subagentsFixture';

/**
 * Product-quality review fixtures (R01–R03, H05/H06/H09). Each test asserts a
 * measurable layout/interaction property and saves a screenshot under
 * test-results/review/ for human review. Runs against mock data (?mock=1).
 * Browser evidence only: native WebKit and live integrations are separate.
 */
async function createWorkspace(page: Page, name: string) {
  await chooseNew(page, 'New workspace');
  await page.getByRole('textbox', { name: 'Desired result' }).fill(`Produce a checked ${name} report.`);
  await page.getByRole('textbox', { name: 'Workspace name' }).fill(name);
  await page.getByRole('button', { name: 'Create workspace', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText(name);
}
const shot = (page: Page, name: string) => page.screenshot({ path: `test-results/review/${name}.png`, fullPage: false });
const nav = (page: Page, name: string) => page.getByLabel('Main navigation').getByRole('button', { name: new RegExp(`^${name}`) }).first().click();

async function openSettings(page: Page, section: string) {
  await nav(page, 'Settings');
  await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: section, exact: true }).click();
}

/** Width of the widest visible content box inside `sel`, relative to the pane. */
async function fill(page: Page, sel: string) {
  return page.evaluate(s => {
    const el = document.querySelector(s) as HTMLElement | null;
    if (!el) return { pane: 0, used: 0 };
    const pane = el.getBoundingClientRect();
    let right = pane.left, left = pane.right;
    for (const c of el.querySelectorAll<HTMLElement>('section, form, .settings-row, .skills-layout, .skills-list, .skills-detail, .settings-subhead')) {
      const r = c.getBoundingClientRect(); if (!r.width) continue;
      right = Math.max(right, r.right); left = Math.min(left, r.left);
    }
    return { pane: pane.width, used: right - left };
  }, sel);
}

for (const width of [1280, 1440, 1920]) {
  test(`settings sections use the page at ${width}px (R02)`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/?mock=1');
    // Skills left Connectors for Library in Settings PR 4.
    for (const section of ['Connectors', 'Background work', 'General']) {
      await openSettings(page, section);
      await page.waitForTimeout(120);
      const f = await fill(page, '.settings-body');
      expect(f.used / f.pane, `${section} uses ${Math.round(f.used)} of ${Math.round(f.pane)}px`).toBeGreaterThan(0.85);
      await shot(page, `settings-${section.toLowerCase().replace(' ', '-')}-${width}`);
    }
  });
}

test('the side panel is the Preview pane: Files, Browser and Subagents tabs (frontier F7, COO rebuild L3/L4)', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await seedSubagents(page);
  await page.goto('/?mock=1&demo=1');
  await page.getByRole('button', { name: 'Open Q3 regional operations review', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Q3 regional operations review');
  // The workspace panel, its section row and its Workspace / Logs strip are gone.
  await expect(page.getByRole('tablist', { name: 'Side panel view' })).toHaveCount(0);
  await expect(page.locator('#workspace-context, .ws-pill')).toHaveCount(0);
  // Files: the top bar's Project files opens WORKSPACE.md.
  await openProjectFiles(page);
  const pane = previewPane(page);
  const strip = pane.getByRole('tablist', { name: 'Preview tabs' });
  await expect(strip.getByRole('tab', { name: 'WORKSPACE.md', exact: true })).toHaveAttribute('aria-selected', 'true');
  await shot(page, 'preview-files');
  // Browser: a new web tab opens the start page.
  await pane.getByRole('button', { name: 'New tab', exact: true }).click();
  await pane.getByRole('button', { name: /^Browser/ }).click();
  await expect(pane.locator('.browser-start')).toBeVisible();
  await shot(page, 'preview-browser');
  // Subagents: the chat's chip opens its tab.
  await page.getByRole('button', { name: /^Subagents:/ }).click();
  await expect(strip.getByRole('tab', { name: /^Subagents/ })).toHaveAttribute('aria-selected', 'true');
  await expect(pane.getByRole('tabpanel', { name: 'Subagents' })).toBeVisible();
  expect(await strip.getByRole('tab').count()).toBeGreaterThanOrEqual(3);
  const h = await strip.getByRole('tab').first().evaluate(el => el.getBoundingClientRect().height);
  expect(h).toBeGreaterThanOrEqual(20);
  expect(h).toBeLessThanOrEqual(48);
  await shot(page, 'preview-subagents');
});

test('chat and context prose share one type scale (R03)', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/?mock=1');
  const size = (sel: string) => page.evaluate(s => { const el = document.querySelector(s); return el ? getComputedStyle(el).fontSize + '/' + getComputedStyle(el).lineHeight : null; }, sel);
  // Resolve the tokens on synthetic probes inside each pane so the check does
  // not depend on which fixture conversation is open.
  await createWorkspace(page, 'Type review');
  // The project's files open in the Preview pane (the workspace panel went in COO rebuild L4).
  await openProjectFiles(page);
  await page.evaluate(() => {
    for (const [host, cls] of [['.work-conversation', 'md-view'], ['.preview-pane-body', 'viewer-rendered']] as const) {
      const d = document.createElement('div'); d.className = `${cls} probe-${cls}`; d.textContent = 'Probe paragraph';
      document.querySelector(host)?.appendChild(d);
    }
  });
  const chat = await size('.probe-md-view');
  const context = await size('.probe-viewer-rendered');
  expect(chat).not.toBeNull();
  expect(context).toBe(chat);
  // Resizing the panel reflows without scaling type.
  await page.setViewportSize({ width: 1920, height: 900 });
  expect(await size('.probe-viewer-rendered')).toBe(context);
});

test('settings search finds motion and jumps to it (H06)', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/?mock=1');
  await nav(page, 'Settings');
  await page.getByLabel('Find a setting').fill('motion');
  const control = page.getByRole('button', { name: /^Motion/ });
  await expect(control).toBeVisible();
  await shot(page, 'settings-search-motion');
  await control.click();
  await expect(page.locator('.settings-found')).toBeVisible();
});

test('a build workspace needs an intentional repository (H05)', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/?mock=1');
  await chooseNew(page, 'New workspace');
  await page.getByLabel('Desired result').fill('Fix the menus');
  await page.getByRole('radio', { name: /Build in a repository/ }).click();
  const repo = page.getByRole('combobox', { name: 'Repository' });
  await expect(repo).toHaveAttribute('data-value', '');
  await expect(page.getByRole('button', { name: 'Create workspace' })).toBeDisabled();
  await repo.click();
  await expect(page.locator('.hs-pop [role=option]')).toHaveCount(3);
  await expect(page.locator('.hs-pop')).toContainText('linked worktree');
  await shot(page, 'build-repository-picker');
  await page.locator('.hs-pop [role=option]').nth(1).click();
  await expect(page.getByRole('button', { name: 'Create workspace' })).toBeEnabled();
});

test('one task editor from the palette and the Tasks page (H02)', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/?mock=1');
  await nav(page, 'Tasks');
  await page.getByRole('button', { name: 'New task' }).click();
  const panel = page.locator('.task-detail .task-editor-form');
  await expect(panel.getByRole('radiogroup', { name: 'Priority' })).toBeVisible();
  await expect(panel.getByRole('button', { name: 'Create task' })).toBeDisabled();
  await shot(page, 'task-editor-panel');
  await page.keyboard.press('ControlOrMeta+k');
  await page.locator('.palette-input').fill('New task');
  await page.locator('.palette-row', { hasText: 'New task' }).first().click();
  const sheet = page.locator('.task-editor.modal-content .task-editor-form');
  await expect(sheet.getByRole('radiogroup', { name: 'Priority' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Close task editor' })).toBeVisible();
  const labels = async (root: typeof panel) => root.locator('.task-field > span:first-child').allTextContents();
  expect(await labels(sheet)).toEqual(await labels(panel));
  await shot(page, 'task-editor-sheet');
});

test('⌘1, the rail and the palette all mean the Work overview, and drafts survive (H01)', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/?mock=1');
  await createWorkspace(page, 'Nav check');
  const overview = page.getByRole('heading', { name: 'Projects', level: 1, exact: true });
  await page.getByLabel('Chat message').fill('Unsent thought');
  // Shortcut
  await page.getByLabel('Chat message').blur();
  await page.keyboard.press('ControlOrMeta+1');
  await expect(overview).toBeVisible();
  await page.locator('.work-card', { hasText: 'Nav check' }).click();
  await expect(page.getByLabel('Chat message')).toHaveValue('Unsent thought');
  // Rail
  await nav(page, 'Projects');
  await expect(overview).toBeVisible();
  await page.locator('.work-card', { hasText: 'Nav check' }).click();
  // Palette
  await page.keyboard.press('ControlOrMeta+k');
  await page.locator('.palette-input').fill('Projects');
  await page.locator('.palette-row').filter({ has: page.locator('.palette-row-label').filter({ hasText: /^Projects$/ }) }).click();
  await expect(overview).toBeVisible();
  await page.locator('.work-card', { hasText: 'Nav check' }).click();
  await expect(page.getByLabel('Chat message')).toHaveValue('Unsent thought');
});

test('Home is one dense list, with no resume card or recent strip (H07, frontier F1)', async ({ page }) => {
  for (const width of [1280, 1920]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/?mock=1');
    await createWorkspace(page, `Home check A ${width}`);
    await nav(page, 'Projects');
    await createWorkspace(page, `Home check B ${width}`);
    await nav(page, 'Projects');
    await expect(page.locator('.studio-resume, .studio-recent, .studio-hero, .studio-first-run')).toHaveCount(0);
    await expect(page.locator('.work-list .work-card', { hasText: `Home check A ${width}` })).toHaveCount(1);
    await expect(page.locator('.work-list .work-card', { hasText: `Home check B ${width}` })).toHaveCount(1);
    await expect(page.locator('.work-list')).not.toContainText('No conversation yet');
    await expect(page.locator('.work-list')).not.toContainText('Room for your next idea');
    const f = await fill(page, '.studio-home');
    expect(f.used / f.pane).toBeGreaterThan(0.85);
    await shot(page, `home-${width}`);
  }
});

test('Tasks board uses the width; Agenda week is gone (H11, overnight 11-08 S3)', async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 900 });
  await page.goto('/?mock=1');
  await nav(page, 'Tasks');
  await page.getByRole('group', { name: 'Task layout' }).getByRole('button', { name: 'Board' }).click();
  const cols = page.locator('.task-board > section');
  await expect(cols.first()).toBeVisible();
  const boardWidth = await page.locator('.task-board').evaluate(el => el.getBoundingClientRect().width);
  const used = await cols.evaluateAll(els => els.reduce((w, e) => w + e.getBoundingClientRect().width, 0));
  expect(used / boardWidth).toBeGreaterThan(0.9);
  await shot(page, 'tasks-board-1920');
  await expect(page.getByRole('group', { name: 'Task layout' }).getByRole('button', { name: 'Agenda', exact: true })).toHaveCount(0);
  await expect(page.locator('.agenda-week-day')).toHaveCount(0);
});

test('connected-project coverage is one actionable line with details (H03)', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript(() => {
    const files = [{ path: '/Hollow/Projects/Launch/README.md', relative: 'Launch/README.md', projectId: 'local-demo', size: 64, readable: true },
      ...Array.from({ length: 6 }, (_, i) => ({ path: `/Hollow/Projects/Mono/generated/f${i}.md`, relative: `Mono/generated/f${i}.md`, projectId: 'local-mono', size: 10, readable: true })),
      { path: '/Hollow/Projects/Mono/deck.key', relative: 'Mono/deck.key', projectId: 'local-mono', size: 10, readable: false }];
    localStorage.setItem('mockDirectories', JSON.stringify({ version: 1, pendingRestart: false, homePath: '/Hollow', projectsPath: '/Hollow/Projects', vaultPath: '/Hollow/Vault', refreshedAt: '2026-09-28T00:00:00Z',
      projects: [{ id: 'local-demo', title: 'Launch', path: '/Hollow/Projects/Launch' }, { id: 'local-mono', title: 'Mono', path: '/Hollow/Projects/Mono' }], files,
      warnings: ['Mono: only the first 2500 files are indexed; add .hollowignore rules to choose which.', 'permission denied: /Hollow/Projects/Locked'],
      coverage: { projects: [{ id: 'local-demo', title: 'Launch', indexed: 1, found: 1, capped: false, folders: [] }, { id: 'local-mono', title: 'Mono', indexed: 2500, found: 2500, capped: true, folders: [{ relative: 'Mono/generated', files: 2100 }, { relative: 'Mono/src', files: 400 }] }], scanLimited: false, maxFiles: 20000, maxProjectFiles: 2500, excluded: [] } }));
  });
  await page.goto('/?mock=1');
  await nav(page, 'Library');
  await page.getByRole('navigation', { name: 'Library view' }).getByRole('button', { name: 'Connected projects' }).click();
  const cov = page.getByRole('region', { name: 'Source coverage' });
  await expect(cov).toContainText('Coverage incomplete: 1 of 2 projects has more than 2,500 files');
  await expect(page.locator('.work-error')).toHaveCount(0); // no stack of warning cards
  await expect(cov).toContainText('permission denied'); // other problems stay visible
  await expect(page.locator('.library-tag')).toContainText('needs reader');
  await shot(page, 'library-coverage');
  await cov.getByRole('button', { name: 'Leave out' }).first().click();
  await expect(page.getByText(/Nothing inside the folder changes/)).toBeVisible();
  await page.getByRole('button', { name: 'Leave folder out' }).click();
  await expect(cov).toContainText('Left out by you');
  await expect(cov.getByRole('button', { name: 'Include again' })).toBeVisible();
});

test('Skills is one catalog with ownership and duplicate precedence (H13)', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/?mock=1');
  await openSettings(page, 'Connectors'); // Skills are in Library, Skills since Settings PR 4
  await page.getByRole('button', { name: 'Open Library, Skills', exact: true }).click();
  await expect(page.locator('.card-shelf, .skill-card')).toHaveCount(0); // no second presentation
  await expect(page.getByRole('heading', { name: 'Skills', exact: true })).toHaveCount(1); // one heading
  const rows = page.locator('.skills-row');
  await expect(rows.first()).toBeVisible();
  await rows.first().click();
  await expect(page.getByRole('region', { name: 'Skill details' })).toContainText(/Shared|Installed for|Plugin|Bundled/);
  await shot(page, 'skills-catalog');
});

test('reduced motion: panel views and rows render without animation', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/?mock=1');
  await createWorkspace(page, 'Motion check');
  await openProjectFiles(page);
  const anim = await page.locator('.preview-pane-body > *').first().evaluate(el => getComputedStyle(el).animationName);
  expect(anim).toBe('none');
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.evaluate(() => { document.documentElement.dataset.motion = 'reduced'; });
  await nav(page, 'Projects');
  const row = await page.locator('.studio-work-row').first().evaluate(el => getComputedStyle(el).animationName);
  expect(row).toBe('none');
});

test('keys typed in an open menu stay in the menu; Tab returns to the trigger; radio arrows', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/?mock=1');
  await nav(page, 'Tasks');
  // A page-level shortcut listener must not see typeahead keys from the popover.
  await page.evaluate(() => { (window as any).__leaked = []; window.addEventListener('keydown', e => (window as any).__leaked.push(e.key)); });
  const sort = page.getByRole('combobox', { name: 'Filter by project' });
  await sort.click();
  await expect(page.locator('.hs-pop')).toBeVisible();
  await page.keyboard.press('j');
  await page.keyboard.press('a');
  expect(await page.evaluate(() => (window as any).__leaked)).toEqual([]);
  await page.keyboard.press('Tab');
  await expect(page.locator('.hs-pop')).toHaveCount(0);
  await expect(sort).not.toBeFocused(); // default Tab moved on from the trigger
  expect(await page.evaluate(() => document.activeElement !== document.body)).toBe(true);
  // Priority radio group: one Tab stop, arrows select.
  await page.getByRole('button', { name: 'New task' }).click();
  const group = page.locator('.task-detail').getByRole('radiogroup', { name: 'Priority' });
  await group.getByRole('radio', { name: 'No priority' }).focus();
  await page.keyboard.press('ArrowRight');
  await expect(group.getByRole('radio', { name: 'High priority (P1)' })).toHaveAttribute('aria-checked', 'true');
  await expect(group.getByRole('radio', { name: 'High priority (P1)' })).toBeFocused();
  await expect(group.locator('[tabindex="0"]')).toHaveCount(1);
});
