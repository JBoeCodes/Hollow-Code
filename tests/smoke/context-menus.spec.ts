import { showWorkspaceList } from './choose';
/**
 * No action is right-click only (review decision C; plan completion L4,
 * matrix N3). Each right-click menu in the app (src/routeLedger.ts
 * RIGHT_CLICK lists every `onContextMenu`) is opened, and every item it
 * offers is then found on that object's visible route: its ⋯ with the same
 * items, or the buttons and fields of its details. Clicks only, against the
 * demo mock; nothing is run.
 */
import { expect, test, type Locator, type Page } from './fixtures/test';

test.beforeEach(({}, info) => {
  test.skip(info.project.name !== 'chromium-desktop', 'right-click parity is a desktop check (small windows have no sidebar tree)');
});

const main = (page: Page) => page.getByRole('navigation', { name: 'Main' });
const tree = (page: Page) => page.getByRole('navigation', { name: 'Project navigation' });

/** The labels of the menu now open, then closed with Escape. */
async function openMenuLabels(page: Page): Promise<string[]> {
  const menu = page.getByRole('menu');
  await expect(menu.first()).toBeVisible();
  const labels = (await menu.first().getByRole('menuitem').allInnerTexts()).map(l => l.replace(/\s+/g, ' ').trim());
  await page.keyboard.press('Escape');
  await expect(menu).toHaveCount(0);
  return labels;
}
async function rightClick(page: Page, target: Locator) {
  await target.click({ button: 'right' });
  return openMenuLabels(page);
}
async function demo(page: Page) {
  await page.goto('/?mock=1&demo=1');
  await expect(page.getByRole('region', { name: 'Projects' })).toBeVisible();
}

test('right-click parity: workspace rows and the rail', async ({ page }) => {
  await demo(page);
  await showWorkspaceList(page);
  const list = page.getByRole('region', { name: 'Projects' });
  const rowMenu = await rightClick(page, list.getByRole('button', { name: 'Open Q3 regional operations review', exact: true }));
  await list.getByRole('button', { name: 'Actions for Q3 regional operations review' }).click();
  expect(await openMenuLabels(page)).toEqual(rowMenu);

  // The sidebar's project row: the same items (the row has no fold arrow
  // or ⋯ since 11-07; the ⋯ items are the project's ⋯ on Work and in the top
  // bar). Since L8 (11-10) it is the project's one row: nothing to fold, no
  // conversations under it, and a click opens the project's lead.
  const projectRow = tree(page).getByRole('button', { name: /^Q3 regional operations review(,|$)/ });
  expect(await rightClick(page, projectRow)).toEqual(rowMenu);
  await expect(projectRow).not.toHaveAttribute('aria-expanded');
  await expect(tree(page).locator('.work-session-row, .work-tree-sessions')).toHaveCount(0);
  await projectRow.click();
  await expect(page.locator('.work-header h1')).toHaveText('Q3 regional operations review');
  await expect(projectRow).toHaveAttribute('aria-current', 'true');
});

test('right-click parity: conversation history rows', async ({ page }) => {
  await demo(page);
  await page.getByRole('navigation', { name: 'Shared tools' }).getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Advanced', exact: true }).click();
  await page.locator('#settings-group-this-mac > summary').click(); // History & recovery: Advanced → This Mac
  await page.getByRole('button', { name: 'Open Library, History', exact: true }).click();
  await page.locator('.library-tabs').getByRole('button', { name: 'Conversations', exact: true }).click(); // Settings PR 4 cut the Conversation history button
  const row = page.locator('.session-row').first();
  const menu = await rightClick(page, row);
  expect(menu).toContain('Open conversation');
  await row.getByRole('button', { name: /^Actions for / }).click();
  expect(await openMenuLabels(page)).toEqual(menu);
});

test('right-click parity: automation rows', async ({ page }) => {
  await demo(page);
  await main(page).getByRole('button', { name: 'Automations', exact: true }).click();
  const row = page.locator('.workflow-row').first();
  expect(await rightClick(page, row)).toEqual(['Run now', 'Edit', 'Delete']);
  await row.click();
  await expect(page.getByRole('button', { name: 'Run now', exact: true }).first()).toBeVisible();
  // Edit sits in the automation's options menu since P1-26 (#925).
  await page.getByRole('button', { name: 'Automation options' }).first().click();
  await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Delete', exact: true })).toBeVisible();
});

test('right-click parity: skill rows', async ({ page }) => {
  await demo(page);
  await page.getByRole('navigation', { name: 'Shared tools' }).getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Connectors', exact: true }).click();
  // Skills are in Library, Skills since Settings PR 4; Connectors keeps a link row.
  await page.getByRole('button', { name: 'Open Library, Skills', exact: true }).click();
  const row = page.getByRole('button', { name: /^release-notes / });
  expect(await rightClick(page, row)).toEqual(['Use in a new workspace…', 'Open', 'Edit', 'Delete']);
  await row.click();
  const details = page.getByRole('region', { name: 'Skill details' });
  await expect(details.getByRole('button', { name: 'Use in a new workspace…', exact: true }).first()).toBeVisible();
  await expect(details.getByRole('button', { name: 'Edit skill' })).toBeVisible();
  await expect(details.getByRole('button', { name: 'Delete skill' })).toBeVisible();
});

test('right-click parity: a reply’s changed files', async ({ page }) => {
  await demo(page);
  await page.getByRole('button', { name: 'Open Q3 regional operations review', exact: true }).click();
  const chip = page.getByRole('list', { name: 'Changed files' }).getByRole('listitem').first().getByRole('button');
  // The demo's chip is a Result in outputs/: its click opens the Preview pane
  // (2026-11-04), and Review result (COO rebuild L4: it was Open in Results)
  // is on its menu, and on the reply's More actions.
  expect(await rightClick(page, chip)).toEqual(['Review result', 'Show change in history']);
  await page.getByRole('group', { name: 'Reply details' }).first().getByRole('button', { name: 'More actions' }).click();
  const more = await openMenuLabels(page);
  expect(more).toContain('Show this reply’s changes in history');
  expect(more).toContain('Review q3-regional-review.md');
});

test('right-click parity: chats on another Mac', async ({ page }) => {
  // M2 PR 4: a chat in the machine strip's popover (mockPeers.ts's demo Mac mini).
  await page.addInitScript(() => localStorage.setItem('mockPeers', 'demo'));
  await demo(page);
  await page.getByRole('group', { name: 'Macs' }).getByRole('button', { name: /^Mac mini, / }).click();
  const pop = page.getByRole('dialog', { name: 'Chats on Mac mini' });
  const row = pop.getByRole('button', { name: /^Nightly build on the mini/ });
  // Home-only actions are listed, disabled: they are the other Mac's to do.
  expect(await rightClick(page, row)).toEqual(['Open', 'Stop', 'Copy conversation ID', 'Rename…', 'Move to project…', 'Archive']);
  // Open is the row's click; Stop and Copy conversation ID are the open chat's.
  await row.click();
  const view = page.getByRole('region', { name: 'Chat on Mac mini' });
  await expect(view.getByRole('button', { name: 'Stop on Mac mini' })).toBeVisible();
  await expect(view.getByRole('button', { name: 'Copy conversation ID' })).toBeVisible();
});
