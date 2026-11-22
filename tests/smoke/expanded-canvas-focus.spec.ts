import { chooseNew } from './choose';
import { expect, test, type Page } from './fixtures/test';
import AxeBuilder from '@axe-core/playwright';
import { join } from 'node:path';

/** Tauri event delivery on top of the `?mock=1` IPC (as canvas-inline.spec.ts). */
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
    w.__emitTauriEvent = (event: string, payload: unknown) => {
      for (const id of listeners.get(event) ?? []) callbacks.get(id)?.({ event, id, payload });
    };
  });
}

const emit = (page: Page, payload: Record<string, unknown>) => page.evaluate(event => {
  (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void }).__emitTauriEvent('chat-event', event);
}, payload);
const calls = (page: Page, command: string) => page.evaluate(c => window.__mockApiCalls?.[c] ?? 0, command);


async function openWorkspace(page: Page) {
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Produce a checked release report.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Release');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Release');
  // As in WebKit (the app's web view): no browser scroll anchoring to keep a
  // canvas in place on its own, so these tests see Hollow's re-anchoring.
  await page.addStyleTag({ content: '.preview-visual, .preview-visual * { overflow-anchor: none !important; }' });
}

async function ask(page: Page, prompt: string, runId: string) {
  const sends = await calls(page, 'chat_send');
  const composer = page.getByLabel('Chat message');
  await composer.fill(prompt);
  await composer.press('Meta+Enter');
  await expect.poll(() => calls(page, 'chat_send')).toBe(sends + 1);
  await emit(page, { type: 'runStarted', asked: prompt, runId, recall: [] });
}


const tallCanvas = '```hollow-canvas\n<hc-canvas id="expanded" title="Expanded report" height="1400"><main><h1>Expanded report</h1>' + Array.from({length: 60}, (_, i) => `<p>Section ${i + 1}: checks and next steps.</p>`).join('') + '</main></hc-canvas>\n```';
const pane = (page: Page) => page.getByRole('complementary', { name: 'Preview' });
const back = (page: Page) => pane(page).getByRole('button', { name: 'Back to canvas: Expanded report', exact: true });
// Read the real scroll owner, so the regression also runs on the old layout.
const scroll = (page: Page, top?: number) => pane(page).locator('.preview-visual').evaluate((root, to) => {
  const body = root.querySelector<HTMLElement>('.side-visual-body')!;
  const el = getComputedStyle(body).overflowY === 'auto' ? body : root as HTMLElement;
  if (to !== undefined) el.scrollTop = to;
  return el.scrollTop;
}, top);
async function expand(page: Page) {
  await openWorkspace(page);
  await ask(page, 'Show the report', 'run-expanded-1');
  await emit(page, { type: 'assistantText', text: tallCanvas });
  await emit(page, { type: 'done', sessionId: null });
  await page.locator('.work-conversation').getByRole('button', { name: 'Expand', exact: true }).click();
  await expect(pane(page).locator('iframe[title="Canvas: Expanded report"]')).toBeVisible();
  await expect.poll(() => pane(page).locator('iframe').evaluate(el => el.getBoundingClientRect().height)).toBeGreaterThan(1000);
}
test.beforeEach(async ({page}) => { await installEventBridge(page); await page.emulateMedia({reducedMotion:'reduce'}); });

test('expanded tall canvas has a keyboard return chip and keeps its selector visible', async ({page}) => {
  await expand(page);
  await scroll(page, 800);
  await expect(back(page)).toBeVisible();
  // The pane's bar stays in view while only the canvas scrolls (the
  // side panel's "Conversation visuals" shelf went with it, run 15 P1).
  await expect(pane(page).getByRole('button', {name:'Close preview', exact:true})).toBeInViewport();
  await back(page).focus(); await page.keyboard.press('Enter');
  await expect.poll(() => scroll(page)).toBeLessThan(3);
  await expect(back(page)).toHaveCount(0);
  await expect(pane(page).getByRole('region', {name:'Expanded canvas: Expanded report', exact:true})).toBeFocused();
});

test('late height changes keep a reading position and unrelated turns keep the expanded iframe', async ({page}, info) => {
  test.skip(info.project.name.includes('narrow'), 'simultaneous conversation and pane needs desktop');
  await expand(page);
  // Setting the position can land short while the frame is still settling its height (the browser
  // clamps scrollTop to what the content allows then, 669 here in about half the runs, and nothing
  // later moves it back), so set it until it holds at 700.
  await expect.poll(() => scroll(page, 700)).toBe(700);
  await expect(back(page)).toBeVisible();
  await pane(page).locator('iframe').evaluate(el => { el.dataset.sameFrame = 'yes'; el.style.height = '1800px'; });
  await expect.poll(() => scroll(page)).toBe(700);
  await ask(page, 'Another report', 'run-expanded-2');
  await emit(page, {type:'assistantDelta', text: '```hollow-canvas\n<hc-canvas id="new-report" title="Other report"><p>New</p></hc-canvas>\n```'});
  await emit(page, {type:'done', sessionId:null});
  await expect(pane(page).locator('iframe')).toHaveAttribute('data-same-frame', 'yes');
  await expect.poll(() => scroll(page)).toBe(700);
  await expect(back(page)).toBeVisible();
});

for (const theme of ['light','dark','glass']) test(`expanded canvas return in ${theme}`, async ({page}, info) => {
  await page.addInitScript(theme => { localStorage.setItem('theme', theme === 'glass' ? 'dark' : theme); localStorage.setItem('glass', theme === 'glass' ? 'on' : 'off'); }, theme);
  await expand(page);
  await expect(page.locator('html')).toHaveAttribute('data-theme', theme === 'light' ? 'light' : 'dark');
  if (theme === 'glass') { await expect(page.locator('html')).toHaveAttribute('data-glass','on'); await page.addStyleTag({content:'html[data-glass] { background:var(--canvas); }'}); }
  await scroll(page, 700); await expect(back(page)).toBeVisible();
  const audit = await new AxeBuilder({page}).include('#preview-pane').analyze();
  expect(audit.violations).toEqual([]);
  if (process.env.Y2_CAPTURE_DIR) await pane(page).screenshot({path:join(process.env.Y2_CAPTURE_DIR, `y2-${theme}-${info.project.name}.png`)});
});

for (const mode of ['system', 'setting', 'full']) test(`return motion uses ${mode} preference`, async ({page}) => {
  await page.emulateMedia({reducedMotion:mode === 'system' ? 'reduce' : 'no-preference'});
  await expand(page);
  await page.evaluate(mode => { document.documentElement.dataset.motion = mode === 'setting' ? 'reduced' : 'full'; }, mode);
  await scroll(page, 700); await expect(back(page)).toBeVisible();
  await pane(page).locator('.side-canvas-scroll').evaluate(el => {
    const original = el.scrollTo.bind(el);
    el.scrollTo = ((options: ScrollToOptions) => { el.setAttribute('data-behavior', options.behavior ?? 'auto'); original(options); }) as typeof el.scrollTo;
  });
  await back(page).click();
  await expect.poll(() => scroll(page)).toBeLessThan(3);
  if (mode === 'full') await expect(pane(page).locator('.side-canvas-scroll')).toHaveAttribute('data-behavior','smooth');
  else await expect(pane(page).locator('.side-canvas-scroll')).not.toHaveAttribute('data-behavior','smooth');
});

test('Escape dismisses without moving the reading position or saving another revision', async ({page}) => {
  await expand(page);
  const saved = await calls(page,'record_canvas');
  await expect.poll(() => scroll(page, 700)).toBe(700); // until it holds: see the late-height test above
  await expect(back(page)).toBeVisible();
  await back(page).focus(); await page.keyboard.press('Escape');
  await expect(back(page)).toHaveCount(0);
  expect(await scroll(page)).toBe(700);
  await expect(pane(page).getByRole('region', {name:'Expanded canvas: Expanded report', exact:true})).toBeFocused();
  expect(await calls(page,'record_canvas')).toBe(saved);
});

test('a small keyboard scroll after return releases the anchor before a late resize', async ({page}) => {
  await expand(page);
  await scroll(page, 700); await back(page).focus(); await page.keyboard.press('Enter');
  await expect.poll(() => scroll(page)).toBeLessThan(3);
  await page.waitForTimeout(800); // the return's smooth-settle guard has finished
  await page.keyboard.press('ArrowDown');
  await expect.poll(() => scroll(page)).toBeGreaterThan(20);
  await page.waitForTimeout(200);
  const at = await scroll(page);
  expect(at).toBeLessThan(48);
  await pane(page).locator('.side-canvas-scroll > div').evaluate(el => (el as HTMLElement).style.paddingBottom = '30px');
  await page.waitForTimeout(200);
  expect(await scroll(page)).toBe(at);
});

test('a focused return chip hands focus to the canvas when manual scrolling hides it', async ({page}) => {
  await expand(page); await scroll(page, 700); await expect(back(page)).toBeVisible();
  await back(page).focus(); await scroll(page, 0);
  await expect(back(page)).toHaveCount(0);
  await expect(pane(page).getByRole('region',{name:'Expanded canvas: Expanded report',exact:true})).toBeFocused();
});
