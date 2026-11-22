import { openTasks } from './choose';
import { expect, test, type Locator, type Page } from './fixtures/test';
import { navigateWaiting, waitingBell } from './waiting';

/**
 * Clipping regressions, asserted by measuring the DOM rather than by eye.
 *
 * These are the failures code reading cannot find: a flex child with
 * `flex-basis: 0%` and `overflow: visible` is crushed below its content width
 * and spills the remainder over its neighbour, while the CSS still looks
 * perfectly reasonable in the file.
 */

/** Elements whose content is wider than their box, i.e. silently cut off. */
async function clippedWithin(page: Page, root: string): Promise<string[]> {
  return page.evaluate((sel) => {
    const host = document.querySelector(sel);
    if (!host) return [`__missing__:${sel}`];
    const nodes = [host, ...host.querySelectorAll('*')] as HTMLElement[];
    return nodes
      .filter((el) => {
        const style = getComputedStyle(el);
        // Only flag elements that cannot scroll and do not hide the excess:
        // those are the ones that actually spill over their neighbours.
        if (style.overflowX !== 'visible') return false;
        return el.scrollWidth > el.clientWidth + 1 && el.clientWidth > 0;
      })
      .map((el) => `${el.className || el.tagName} (${el.scrollWidth} > ${el.clientWidth})`);
  }, root);
}

test('page headings have a clear title and no decorative progress indicator', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/?mock=1');
  await page.getByLabel('Main navigation').getByRole('button', { name: /Settings/ }).click();
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
  // No generated decoration on the header (the old underline/progress bar
  // was a ::before/::after). A flex container's empty pseudo-element computes
  // as display:block in Chromium, so assert on its content, not its display.
  const header = '.page-header:not(.page-retained .page-header)';
  const decor = await page.locator(header).evaluate(el => [getComputedStyle(el, '::before').content, getComputedStyle(el, '::after').content]);
  for (const content of decor) expect(['none', 'normal']).toContain(content);
  expect(await clippedWithin(page, header)).toEqual([]);
});

test('no page scrolls horizontally at 900px', async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 900 });
  await page.goto('/?mock=1');
  await expect(page.getByLabel('Main navigation')).toBeVisible();

  // The destinations the old name list meant (Today, Tasks, Vault; Calendar
  // is gone), clicked by their rail rows. The old `getByRole('button', /name/)
  // .first()` matched whatever button came first, or nothing, so it could not
  // say which page it had opened, let alone wait for it.
  // Needs you (was Review), Automations and Settings too (EH8, review H15).
  // Needs you has no rail row since U3: the top bar's bell, then its See all
  // needs you; Settings sits in the rail's bottom "Shared tools" nav, not in Main.
  const rail = page.getByRole('navigation', { name: 'Main' });
  const shared = page.getByRole('navigation', { name: 'Shared tools' });
  const pages: Array<[string, Locator]> = [
    ['Projects', rail.getByRole('button', { name: 'Projects', exact: true })],
    ['Needs you', waitingBell(page)],
    ['Tasks', page.locator('.home-view-actions').getByRole('button', { name: 'Tasks', exact: true })],
    ['Library', rail.getByRole('button', { name: 'Library', exact: true })],
    ['Automations', rail.getByRole('button', { name: 'Automations', exact: true })],
    ['Settings', shared.getByRole('button', { name: 'Settings', exact: true })],
  ];
  for (const [name, row] of pages) {
    if (name === 'Tasks') await openTasks(page); else if (name === 'Needs you') await navigateWaiting(page); else await row.click();
    // Wait for the real page, not the moment after the click: the row is the
    // current page, the view transition (`is-transitioning` on the root) is
    // over, the lazy page's Suspense fallback (App.tsx PageLoading,
    // `.page-loading`) has gone, and the page's own heading is up. Only then
    // is the overflow read a reading of the page itself.
    if (name !== 'Tasks' && name !== 'Needs you') await expect(row).toHaveAttribute('aria-current', 'page');
    await expect(page.locator('html')).not.toHaveClass(/\bis-transitioning\b/);
    // Skeletons too: Tasks shows one ("Loading tasks") until its data lands.
    await expect(page.locator('.canvas-split > .page :is(.page-loading, .ui-skeleton)')).toHaveCount(0);
    await expect(page.locator('.canvas-split > .page').getByRole('heading', { level: 1, name, exact: true })).toBeVisible();
    // The page has landed: one reading, not a poll that could pass on a
    // moment before content arrives.
    const scrolls = await page.evaluate(() => {
      const d = document.documentElement;
      return d.scrollWidth > d.clientWidth + 1;
    });
    expect(scrolls, `${name} scrolls horizontally at 900px`).toBe(false);
  }
});

/**
 * Dangling `animation-name`s (a name with no `@keyframes`) are checked at build
 * time in scripts/check-css-tokens.mjs, not here. A browser-side stylesheet walk
 * cannot see them: when an `animation` shorthand contains a `var()` — and every
 * one in this codebase uses `var(--t-*)` for its duration — the value is stored
 * as a pending substitution and `rule.style.animationName` reads back empty, so
 * the walk finds nothing to complain about and passes with the bug present.
 */

/**
 * Quick capture parsed the same line twice on every commit: once for the live
 * preview, then again in submit(). The preview was deliberately display-only so
 * a stale or in-flight parse could never change what got created: the right
 * instinct, paid for with a duplicate IPC. It now reuses the preview's parse only
 * when it was made from this exact trimmed string, so the safety property is
 * enforced by identity instead of by hoping the timing worked out. Since S5
 * (2026-11-03) quick capture is the Add sheet set to a task, which keeps the rule.
 */
test('quick capture reuses its own parse, but never a stale one', async ({ page }) => {
  await page.goto('/?mock=1');
  const openQuickCapture = async () => {
    await page.evaluate(async url => { (await import(url)).runMenuAction('quick-capture'); }, '/src/appMenu.ts');
    const sheet = page.getByRole('dialog', { name: 'Add', exact: true });
    await expect(sheet).toBeVisible();
    return { input: sheet.getByRole('textbox', { name: 'What to add' }), add: sheet.getByRole('button', { name: 'Add task', exact: true }) };
  };

  const parses = () => page.evaluate(() => (window.__mockApiCalls ?? {})['parse_quick_add'] ?? 0);
  // Creates so far. Monotonic (the test's reset() clears only the call counts),
  // so "the submit finished" is a state to wait for, not a moment to catch.
  const created = () => page.evaluate(() => ((window as unknown as { __mockCreatedTasks?: unknown[] }).__mockCreatedTasks ?? []).length);
  const reset = () => page.evaluate(() => { window.__mockApiCalls = {}; });

  // Type, let the debounce settle, submit: the parse is reused, not repeated.
  // The mock answers a parse within the call's own task, so once the count
  // reads 1 the preview (and the string it was parsed from) is in place.
  let sheet = await openQuickCapture();
  await reset();
  await sheet.input.fill('write the report p1 #Work');
  await expect.poll(parses, { message: 'the preview parse should have run once' }).toBe(1);
  await sheet.add.click();
  // save() parses (if it must) before it creates, so once the create has
  // landed the count is final.
  await expect.poll(created).toBe(1);
  expect(await parses(), 'submit re-parsed a line it already had').toBe(1);
  await expect(page.getByRole('dialog', { name: 'Add', exact: true })).toHaveCount(0);

  // Edit past a settled preview and submit immediately: the stale parse must be
  // refused and a fresh one taken, or the task is created from the old text.
  sheet = await openQuickCapture();
  await expect(sheet.input).toHaveValue('');
  await reset();
  await sheet.input.fill('a p1');
  await expect.poll(parses, { message: 'the preview of "a p1" should have run' }).toBe(1);
  await sheet.input.fill('b p2');
  await sheet.add.click();
  await expect.poll(created).toBe(2);
  expect(await parses(), 'a stale preview was reused for a line the user edited').toBe(2);
  const made = await page.evaluate(() => ((window as unknown as { __mockCreatedTasks?: { title: string }[] }).__mockCreatedTasks ?? []).map(t => t.title));
  expect(made.at(-1), 'the task came from the edited line').toBe('b p2');
});

/**
 * The cold-boot cascade ran on two clocks that never agreed: `.is-booting` was
 * added and its 900ms strip timer armed BEFORE `import('./App')` started, while
 * the CSS chain it gates cannot begin until that render lands and paints. Most
 * of the budget went to loading the app's largest chunk, so the launch animation
 * depended on whether that chunk was warm in disk cache.
 *
 * Now one `BOOT_MS` drives both, and the strip is anchored on first paint.
 */

/**
 * Data rows must not carry an entrance animation. Solid's `<For>` reconciles by
 * `===`, so a list fed fresh IPC objects rebuilds every row on every refetch and
 * the entrance replays — the flash is the animation faithfully announcing a
 * rebuild. Lists keyed on module constants or strings reuse their DOM and keep
 * theirs, which is why this test asserts both directions.
 */

/**
 * Work the app used to do and throw away. None of this is visible in the DOM —
 * "this command ran once, not twice" is only assertable against the mock's
 * per-command invoke counter.
 */

/**
 * Banking the hearth must stop EVERY ambient animation, not the nine selectors
 * someone remembered to list. The old mechanism was a hand-maintained
 * `body.is-away` list covering 11 of 50 infinite animations, so the Calendar
 * now-line and now-dot, the Vault drift dot and four of the five arch-breathe
 * hosts all kept running in an unfocused window.
 *
 * Asserted as an outcome over whatever is on screen, so it cannot go stale as
 * pages change: with `is-away` set, nothing infinite may still be running.
 */

/**
 * The hearth banks ambient motion on window blur. The old guard returned early
 * whenever `document.activeElement` was an IFRAME, which is a DOM-focus fact,
 * not an OS one — it stays on the <iframe> after a later Cmd+Tab, so once you
 * had opened an email the app's only animation-pause mechanism never fired
 * again. `document.hasFocus()` is the right discriminator.
 *
 * This pins the logic. Whether WKWebView reports `hasFocus()` correctly for a
 * frontmost-other-app and for an in-app iframe can only be confirmed in an
 * installed .app.
 */
test('ambient motion banks on real focus loss, not on iframe focus', async ({ page }) => {
  await page.goto('/?mock=1');
  await expect(page.getByLabel('Main navigation')).toBeVisible();

  // The blur handler banks twice: at 0 ms, and again at 400 ms in case
  // WKWebView reports focus stale (App.tsx). A timer queued after both, with a
  // later deadline, fires after both, so the answer read then is final for
  // this blur, not a sample. (The old 60 ms read ran before the 400 ms one.)
  // COUPLED: 401 here must stay above the 400 in App.tsx's `handleBlur`
  // (`setTimeout(bank, 400)`, in the ambient-motion onMount). Raise that delay
  // and this read lands before the second bank again.
  const blurWith = (hasFocus: boolean) => page.evaluate(async (hf) => {
    Object.defineProperty(document, 'hasFocus', { value: () => hf, configurable: true });
    window.dispatchEvent(new Event('blur'));
    await new Promise(resolve => setTimeout(resolve, 401));
    return document.body.classList.contains('is-away');
  }, hasFocus);

  // Focus still ours (an in-app iframe took DOM focus): keep breathing.
  expect(await blurWith(true), 'iframe focus must not bank the fire').toBe(false);

  // Real system focus loss: bank.
  expect(await blurWith(false), 'losing system focus must bank the fire').toBe(true);

  // And it must un-latch. Returning to a window whose focus sits inside an
  // iframe may never fire `focus` on the parent, so a poll recovers it.
  await page.evaluate(() => {
    Object.defineProperty(document, 'hasFocus', { value: () => true, configurable: true });
  });
  await expect
    .poll(() => page.evaluate(() => document.body.classList.contains('is-away')), { timeout: 4000 })
    .toBe(false);
});

