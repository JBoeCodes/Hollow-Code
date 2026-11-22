import { openTasks } from './choose';
import { test, expect, type Locator, type Page } from './fixtures/test';
import { chooseNew, choose, filesPicker, openGlobalSearch, openProjectFiles, openWorkspaceResult, previewPane, resultReviewBar } from './choose';

// next-level P21: a sweep over the main surfaces with the browser's own
// accessibility tree (CDP), not a re-implementation of name computation.
// Every interactive control has a name, images have alt text, ids are
// unique, and there is one main landmark.
const INTERACTIVE = new Set(['button', 'link', 'textbox', 'searchbox', 'combobox', 'checkbox', 'radio', 'switch', 'tab', 'menuitem', 'option', 'slider', 'spinbutton', 'menuitemcheckbox', 'menuitemradio', 'treeitem']);

async function audit(page: Page, label: string) {
  const cdp = await page.context().newCDPSession(page);
  const { nodes } = await cdp.send('Accessibility.getFullAXTree') as { nodes: { ignored: boolean; role?: { value: string }; name?: { value: string }; backendDOMNodeId?: number }[] };
  const unnamed: string[] = [];
  let controls = 0;
  for (const n of nodes) {
    if (n.ignored || !n.role || !INTERACTIVE.has(n.role.value)) continue;
    controls++;
    if ((n.name?.value ?? '').trim()) continue;
    const { outerHTML } = n.backendDOMNodeId ? await cdp.send('DOM.resolveNode', { backendNodeId: n.backendDOMNodeId }).then(r => cdp.send('Runtime.callFunctionOn', { objectId: r.object.objectId!, functionDeclaration: 'function(){return {outerHTML:this.outerHTML.slice(0,160)}}', returnByValue: true })).then(r => r.result.value as { outerHTML: string }) : { outerHTML: '?' };
    unnamed.push(`${n.role.value}: ${outerHTML}`);
  }
  const dom = await page.evaluate(() => {
    const ids = new Map<string, number>();
    for (const el of document.querySelectorAll('[id]')) ids.set(el.id, (ids.get(el.id) ?? 0) + 1);
    const visible = (el: Element) => { const r = (el as HTMLElement).getBoundingClientRect(); return r.width > 0 && r.height > 0; };
    return {
      dupIds: [...ids].filter(([, n]) => n > 1).map(([id]) => id),
      imgNoAlt: [...document.querySelectorAll('img:not([alt])')].filter(visible).map(i => (i as HTMLImageElement).src.slice(0, 80)),
      mains: document.querySelectorAll('main, [role=main]').length,
    };
  });
  await cdp.detach();
  return { label, controls, unnamed, ...dom };
}

const rail = (page: Page) => page.getByRole('navigation', { name: 'Main' });
/** A destination has landed, so the audit reads it rather than a moment of it:
 *  its row is the current page, its heading is up, the page change's view
 *  transition is over, and nothing in the page is still loading. Each page is
 *  a lazy import whose Suspense fallback is App.tsx's PageLoading
 *  (`.page-loading`, built from `.ui-skeleton-bar`s, not the `Skeleton`
 *  primitive), and a panel inside a page loads behind ui.tsx's `Skeleton`
 *  (`.ui-skeleton`); a fixed wait could audit either. */
async function landed(page: Page, row: Locator, heading: string) {
  await expect(row).toHaveAttribute('aria-current', 'page');
  await expect(page.getByRole('heading', { name: heading, exact: true })).toBeVisible();
  await expect(page.locator('html')).not.toHaveClass(/\bis-transitioning\b/);
  await expect(page.locator('.canvas-split > .page :is(.page-loading, .ui-skeleton)')).toHaveCount(0);
}
const SCENES: [string, (page: Page) => Promise<void>][] = [
  ['Work', async () => {}],
  ['Workspace conversation', async page => { await page.getByRole('navigation', { name: 'Project navigation' }).getByRole('button', { name: /^Q3 regional operations review(,|$)/ }).click(); await page.locator('.chat-composer textarea').waitFor(); }],
  // The project's files in the Preview pane (COO rebuild L4: they were the
  // workspace panel's sections): WORKSPACE.md, a Result under its review
  // bar, and PLAN.md with its live boxes.
  ['Project files', async page => { await openProjectFiles(page); await expect(previewPane(page).getByRole('tab', { name: 'WORKSPACE.md', exact: true })).toHaveAttribute('aria-selected', 'true'); }],
  ['Result review', async page => { await openWorkspaceResult(page, 'local-q3-review', 'workspaces/local-q3-review/outputs/q3-regional-review.md'); await resultReviewBar(page).waitFor(); }],
  ['Plan', async page => { await choose(filesPicker(page), { label: 'PLAN.md' }); await expect(previewPane(page).getByRole('checkbox').first()).toBeEnabled(); }],
  ['Tasks', async page => { await previewPane(page).getByRole('button', { name: 'Close preview', exact: true }).click(); await openTasks(page); await page.locator('.issue-row').first().waitFor(); }],
  ['Library', async page => { const row = rail(page).getByRole('button', { name: /^Library/ }); await row.click(); await landed(page, row, 'Library'); }],
  ['Automations', async page => { const row = rail(page).getByRole('button', { name: /^Automations/ }); await row.click(); await landed(page, row, 'Automations'); }],
  ['Settings', async page => { const row = page.getByRole('navigation', { name: 'Shared tools' }).getByRole('button', { name: /^Settings/ }); await row.click(); await landed(page, row, 'Settings'); }],
  // All three searches for "plan" answered. Typing sets the list aria-busy at
  // once, and it leaves aria-busy when the vault and project-file searches
  // (debounced 180 ms) are done. The conversation search (also 180 ms,
  // CommandPalette.tsx) has no busy state of its own, so wait for its call:
  // the mock counts `search_sessions` inside the call and answers it in the
  // same task's microtasks (api.ts), so once a later evaluate sees the count
  // rise, its rows (if any) have rendered.
  ['Palette', async page => {
    const searches = () => page.evaluate(() => (window.__mockApiCalls ?? {})['search_sessions'] ?? 0);
    const before = await searches();
    await openGlobalSearch(page);
    await page.getByPlaceholder(/Find a conversation/).fill('plan');
    await expect.poll(searches, { message: 'the palette\'s conversation search for "plan" ran' }).toBeGreaterThan(before);
    await expect(page.locator('#palette-listbox')).toHaveAttribute('aria-busy', 'false');
  }],
  ['Add sheet', async page => { await page.keyboard.press('Escape'); await rail(page).getByRole('button', { name: /^Projects/ }).click(); await chooseNew(page, 'Add a task, note, link, file or recording…'); await page.getByRole('dialog', { name: 'Add' }).waitFor(); }],
];

test('the main surfaces have named controls, alt text, unique ids and one main', async ({ page }, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'the desktop composition carries every control');
  test.setTimeout(90_000);
  await page.goto('/?mock=1&demo=1');
  await rail(page).waitFor();
  const results = [];
  for (const [label, go] of SCENES) { await go(page); results.push(await audit(page, label)); }
  const problems = results.flatMap(r => [
    ...r.unnamed.map(u => `${r.label} · unnamed ${u}`),
    ...r.dupIds.map(id => `${r.label} · duplicate id ${id}`),
    ...r.imgNoAlt.map(src => `${r.label} · img without alt ${src}`),
    ...(r.mains === 1 ? [] : [`${r.label} · ${r.mains} main landmarks`]),
  ]);
  console.log(results.map(r => `${r.label}: ${r.controls} controls`).join('\n'));
  console.log(problems.join('\n'));
  // The sweep saw each surface (a broken navigation would audit one page ten times).
  for (const r of results) expect.soft(r.controls, r.label).toBeGreaterThan(5);
  expect(problems).toEqual([]);
});
