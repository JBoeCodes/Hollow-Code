import { showWorkspaceList } from './choose';
import AxeBuilder from '@axe-core/playwright';
import { expect, test, type FrameLocator, type Locator, type Page, type TestInfo } from './fixtures/test';
import { choose, chooseNew, filesPicker, openProjectFiles, openWorkspaceResult, previewPane } from './choose';

/**
 * Canvas C6 P2: the accessibility pass over every canvas surface
 * (docs/history/CANVAS_C6.md, Lane P). axe (the C2 K3 setup) on the host and inside
 * the frames, with zero serious or critical violations, and keyboard-only
 * journeys:
 *
 * - an inline chat canvas and its chrome: the version menu, "Back to",
 *   Expand, and "Ask to fix" on a broken canvas;
 * - an ask-canvas in a workspace chat: multi, range, rank and matrix by
 *   keyboard, Send, the message box by Tab, Enter, and the locked state;
 * - all six panel sections in light, dark and glass at narrow width: the
 *   pill row by arrows, every tab stop in the section with a visible focus
 *   ring, and reduced motion.
 *
 * Moderate and minor findings are listed (an annotation on the test) and
 * held to the documented list in `MODERATE_OK`, so a new one fails here.
 * VoiceOver itself is not driven: see the checklist in CANVAS_C6.md.
 */

type AxeResults = Awaited<ReturnType<AxeBuilder['analyze']>>;
const serious = (r: AxeResults) => r.violations
  .filter(v => v.impact === 'serious' || v.impact === 'critical')
  .map(v => `${v.id} (${v.impact}): ${v.nodes.slice(0, 4).map(n => `${n.target.join(' >>> ')} ${n.failureSummary?.split('\n').slice(1, 2).join('') ?? ''}`).join(' | ')}`);
/** Moderate rules left on purpose, and only inside a canvas frame
 *  (CANVAS_C6.md, Lane P): they grade a frame's document as if it were a
 *  page. A canvas is part of a reply; a `main` landmark or an `h1` in each
 *  frame would add a landmark and a top-level heading per canvas to
 *  VoiceOver's rotor, and its title is already the host's label. */
const FRAME_PAGE_RULES = new Set(['landmark-one-main', 'page-has-heading-one', 'region']);
type Violation = AxeResults['violations'][number];
const allowed = (v: Violation) => FRAME_PAGE_RULES.has(v.id) && v.impact === 'moderate' && v.nodes.every(n => n.target.length > 1);
const lesser = (r: AxeResults) => r.violations.filter(v => v.impact !== 'serious' && v.impact !== 'critical');
const explain = (v: Violation) => `${v.id} (${v.impact}): ${v.nodes.slice(0, 3).map(n => n.target.join(' >>> ')).join(' | ')}`;
const reachedFrame = (r: AxeResults) => [...r.passes, ...r.incomplete].some(x => x.nodes.some(n => n.target.length > 1));

async function axe(page: Page, info: TestInfo, label: string, include: string, { frame = false } = {}) {
  const r = await new AxeBuilder({ page }).include(include).analyze();
  expect(serious(r), `${label}: serious or critical`).toEqual([]);
  const rest = lesser(r);
  if (rest.length) info.annotations.push({ type: 'axe-moderate', description: `${label}: ${rest.map(v => v.id).join(', ')}` });
  expect(rest.filter(v => !allowed(v)).map(explain), `${label}: moderate or minor outside the frame-page rules`).toEqual([]);
  if (frame) expect(reachedFrame(r), `${label}: axe reached inside the frame`).toBe(true);
  return r;
}

/** The focused element in the page or in a canvas frame inside it,
 *  described, with whether a focus indicator is drawn (outline or
 *  box-shadow). Canvas frames are cross-origin (sandboxed, no same origin),
 *  so a frame's focus is read from inside it. */
type Focus = { where: string; name: string; tag: string; ring: boolean; inFrame: boolean; path: string };
const describeFocus = () => {
  const el = document.activeElement as HTMLElement | null;
  if (!el || el === document.body) return { name: '', tag: '', ring: false, path: '', title: '' };
  const cs = getComputedStyle(el);
  const outline = cs.outlineStyle !== 'none' && parseFloat(cs.outlineWidth) > 0;
  const shadow = cs.boxShadow !== 'none' && cs.boxShadow !== '';
  // The message box draws its ring on the composer around it (:focus-within).
  const box = el.closest('.chat-composer');
  const within = !!box && getComputedStyle(box).boxShadow !== 'none';
  const labelled = el.getAttribute('aria-labelledby')?.split(/\s+/).map(id => document.getElementById(id)?.textContent ?? '').join(' ');
  const name = (el.getAttribute('aria-label') || labelled || (el as HTMLInputElement).labels?.[0]?.textContent || el.textContent || '').trim().replace(/\s+/g, ' ').slice(0, 60);
  const path: string[] = [];
  for (let n: Element | null = el; n && path.length < 4; n = n.parentElement) path.unshift(n.id ? `#${n.id}` : `${n.tagName.toLowerCase()}${n.classList[0] ? `.${n.classList[0]}` : ''}`);
  return { name, tag: el.tagName, ring: outline || shadow || within, path: path.join(' > '), title: el.tagName === 'IFRAME' ? (el as HTMLIFrameElement).title : '' };
};
async function focused(page: Page): Promise<Focus> {
  const top = await page.evaluate(describeFocus);
  if (top.tag !== 'IFRAME') return { where: '', name: top.name, tag: top.tag, ring: top.ring, inFrame: false, path: top.path };
  for (const f of page.frames()) {
    if (f === page.mainFrame()) continue;
    if (!await f.evaluate(() => document.hasFocus()).catch(() => false)) continue;
    const inner = await f.evaluate(describeFocus).catch(() => null);
    if (inner) return { where: top.title, name: inner.name, tag: inner.tag, ring: inner.ring, inFrame: true, path: inner.path };
  }
  return { where: top.title, name: '', tag: '', ring: false, inFrame: true, path: '' };
}

/** Press Tab until `done` holds for the focused element (at most `max`
 *  presses), checking every stop on the way has a focus ring. */
async function tabUntil(page: Page, done: (f: Focus) => boolean, max = 40, key = 'Tab') {
  const stops: Focus[] = [];
  for (let i = 0; i < max; i++) {
    await page.keyboard.press(key);
    const f = await focused(page);
    stops.push(f);
    if (done(f)) return stops;
  }
  throw new Error(`not reached in ${max} presses: ${stops.map(s => `${s.where}|${s.tag}|${s.name}`).join(' ; ')}`);
}
const noRing = (stops: Focus[]) => stops.filter(s => s.tag && !s.ring).map(s => `${s.where ? `${s.where} > ` : ''}${s.path} "${s.name}"`);

// ─── Chat canvases ──────────────────────────────────────────────────────

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
const composer = (page: Page) => page.getByRole('textbox', { name: 'Chat message' });

async function openWorkspace(page: Page, theme: 'light' | 'dark' = 'light') {
  await page.addInitScript(t => { localStorage.setItem('theme', t); localStorage.setItem('glass', 'off'); }, theme);
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Produce a checked release report.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Release');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Release');
}

/** The person sends `prompt` (the composer's own send), and a turn answers `text`. */
async function turn(page: Page, prompt: string, runId: string, text: string, { send = true } = {}) {
  if (send) {
    const sends = await calls(page, 'chat_send');
    await composer(page).fill(prompt);
    await composer(page).press('Meta+Enter');
    await expect.poll(() => calls(page, 'chat_send')).toBe(sends + 1);
  }
  await emit(page, { type: 'runStarted', asked: prompt, runId, recall: [] });
  if (text.includes('```hollow-canvas\n') && !text.includes('id="Release Status"')) {
    await emit(page, { type: 'assistantDelta', text });
    await expect(page.locator('.work-conversation .chat-msg').last().locator('.chat-canvas iframe')).toBeVisible();
  } else {
    await emit(page, { type: 'assistantText', text });
  }
  await emit(page, { type: 'done', sessionId: null });
}

const statusCanvas = (build: number) => `<hc-canvas id="release-status" title="Release status">
<p>Build ${build}</p>
<hc-stats><hc-stat label="Bed" value="44"></hc-stat><hc-stat label="Fail" value="0" tone="success"></hc-stat></hc-stats>
<hc-tabs label="Views">
<section label="Summary"><p>29 checks passed</p></section>
<section label="Lanes"><hc-table label="Lane hours">Lane | Hours
Alpha | 3
Beta | 12</hc-table></section>
</hc-tabs>
<hc-details summary="Why it matters"><p>Install waits on the bed.</p></hc-details>
</hc-canvas>`;
const fence = (body: string) => `Here it is.\n\n\`\`\`hollow-canvas\n${body}\n\`\`\``;

test.describe('chat canvases', () => {
  test.beforeEach(async ({ page }, info) => {
    test.skip(info.project.name.includes('narrow'), 'the conversation and side-panel split are desktop-only; the canvas chrome is the same');
    await installEventBridge(page);
  });

  for (const theme of ['light', 'dark'] as const) {
    test(`axe in ${theme}: an inline canvas with three revisions, its version menu and a broken canvas`, async ({ page }, info) => {
      await openWorkspace(page, theme);
      for (const n of [1, 2, 3]) await turn(page, `Status ${n}?`, `run-a11y-${n}`, fence(statusCanvas(n)));
      const last = page.locator('.work-conversation .chat-canvas').last();
      await expect(page.frameLocator('.work-conversation iframe[title="Canvas: Release status"] >> nth=2').getByText('Build 3')).toBeVisible();
      await last.getByRole('button', { name: /^v3/ }).click();
      await expect(last.getByRole('group', { name: 'Versions of Release status' })).toBeVisible();
      await turn(page, 'Draw the bad one', 'run-a11y-bad', 'Status:\n\n```hollow-canvas\n<hc-canvas id="Release Status" title="Release">\n<p>x</p>\n</hc-canvas>\n```');
      await expect(page.getByRole('group', { name: 'Broken canvas: Release' })).toBeVisible();
      await axe(page, info, `chat ${theme}`, '.work-conversation', { frame: true });
    });
  }

  test('keyboard only: the version menu, Back to, Expand and Ask to fix', async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await openWorkspace(page);
    for (const n of [1, 2, 3]) await turn(page, `Status ${n}?`, `run-key-${n}`, fence(statusCanvas(n)));
    const third = page.locator('.work-conversation .chat-canvas').last();
    const frame = third.frameLocator('iframe[title="Canvas: Release status"]');
    await expect(frame.getByText('Build 3')).toBeVisible();
    const toggle = third.locator('button.chat-artifact-version');

    // From the composer, Shift+Tab walks back up the reply into the canvas.
    await composer(page).focus();
    const back = await tabUntil(page, f => f.name.startsWith('v3') && !f.inFrame, 60, 'Shift+Tab');
    expect(noRing(back), 'every stop on the way shows focus').toEqual([]);
    await expect(toggle).toBeFocused();

    // Enter opens the versions; Tab reaches them; Enter on v1 shows it, and
    // focus stays on the version button (it was lost to the page before).
    await page.keyboard.press('Enter');
    await expect(toggle).toHaveAttribute('aria-expanded', 'true');
    const versions = third.getByRole('group', { name: 'Versions of Release status' });
    await page.keyboard.press('Tab');
    await expect(versions.getByRole('button', { name: 'v3 current' })).toBeFocused();
    await page.keyboard.press('Tab');
    await page.keyboard.press('Tab');
    await expect(versions.getByRole('button', { name: 'v1' })).toBeFocused();
    await page.keyboard.press('Enter');
    await expect(third).toHaveAttribute('data-revision', '1');
    await expect(frame.getByText('Build 1')).toBeVisible();
    await expect(toggle).toBeFocused();
    // The change is said in the canvas's live region (the note under the frame is drawn anew).
    const said = third.locator('.chat-canvas-live[role="status"]');
    await expect(said).toHaveText('Showing v1, an earlier version, read-only.');

    // Escape closes the list from inside it and gives focus back to the button.
    await page.keyboard.press('Enter');
    await page.keyboard.press('Tab');
    await expect(versions.getByRole('button', { name: 'v3 current' })).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(versions).toHaveCount(0);
    await expect(toggle).toBeFocused();

    // Tab on, through the chrome and the frame, to "Back to v3"; Enter goes back.
    const forward = await tabUntil(page, f => f.name === 'Back to v3', 60);
    expect(noRing(forward), 'every stop on the way shows focus').toEqual([]);
    expect(forward.some(f => f.inFrame), 'Tab went through the canvas frame').toBe(true);
    await page.keyboard.press('Enter');
    await expect(third).toHaveAttribute('data-revision', '3');
    await expect(frame.getByText('Build 3')).toBeVisible();
    await expect(toggle).toBeFocused();
    await expect(said).toHaveText('Showing v3, this reply’s version.');

    // Expand, by keyboard: the same canvas in the side panel.
    await tabUntil(page, f => f.name === 'Expand' && !f.inFrame, 10);
    await page.keyboard.press('Enter');
    const pane = page.getByRole('complementary', { name: 'Preview' });
    await expect(pane.getByRole('tab', { selected: true })).toHaveAttribute('title', /^Canvas\n/);
    await expect(pane.locator('iframe[title="Canvas: Release status"]')).toBeVisible();

    // A broken canvas: Ask to fix by keyboard fills the composer and sends nothing.
    await turn(page, 'Draw the bad one', 'run-key-bad', 'Status:\n\n```hollow-canvas\n<hc-canvas id="Release Status" title="Release">\n<p>x</p>\n</hc-canvas>\n```');
    const broken = page.getByRole('group', { name: 'Broken canvas: Release' });
    await expect(broken).toBeVisible();
    const sends = await calls(page, 'chat_send');
    await composer(page).focus();
    const toFix = await tabUntil(page, f => f.name.startsWith('Ask to fix'), 30, 'Shift+Tab');
    expect(noRing(toFix)).toEqual([]);
    await page.keyboard.press('Enter');
    await expect(composer(page)).toHaveValue(/^The canvas "Release" \(id Release Status\) is broken/);
    await expect(composer(page)).toBeFocused();
    expect(await calls(page, 'chat_send')).toBe(sends);
  });

  const ASK = `<hc-canvas id="release-plan" title="Release plan" ask>
<hc-ask title="Release plan" send="Send answers">
<hc-multi name="checks" label="Which checks must pass first?">
Bed rows
Bundle budget
</hc-multi>
<hc-range name="budget" label="Kit budget" min="16" max="32" step="2" value="24" unit="KB"></hc-range>
<hc-rank name="order" label="Build order">
Inputs
Draft to the composer
Lock after sending
</hc-rank>
<hc-matrix name="fit" label="Rate each option" options="Low | Medium | High">
Speed
Risk
</hc-matrix>
</hc-ask>
</hc-canvas>`;

  test('keyboard only: an ask-canvas\'s multi, range, rank and matrix, Send, the message box and the lock; axe before and after', async ({ page }, info) => {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await openWorkspace(page);
    await turn(page, 'What should we ship?', 'run-ask-1', `A few questions.\n\n\`\`\`hollow-canvas\n${ASK}\n\`\`\``);
    const canvas = page.locator('.work-conversation .chat-canvas[data-canvas-id="release-plan"]');
    const frame: FrameLocator = canvas.frameLocator('iframe[title="Canvas: Release plan"]');
    const form: Locator = frame.getByRole('form', { name: 'Release plan' });
    // The Send button is Hollow's, under the frame (question cards, 2026-10-14).
    const send = canvas.getByRole('group', { name: 'Send this canvas’s answers' }).getByRole('button', { name: 'Send answers', exact: true });
    await expect(send).toBeVisible();
    await expect(form.getByRole('button', { name: 'Send answers' })).toHaveCount(0);
    await expect(canvas.locator('[data-ask-state="waiting"]')).toHaveText('Waiting on your answer');
    await axe(page, info, 'ask-canvas open', '.work-conversation', { frame: true });

    // Into the canvas from the composer, backwards: Shift+Tab reaches its Send first.
    await composer(page).focus();
    const into = await tabUntil(page, f => !f.inFrame && f.name === 'Send answers', 30, 'Shift+Tab');
    expect(noRing(into)).toEqual([]);
    // Then from its first field, forwards.
    await form.getByRole('checkbox', { name: 'Bed rows' }).focus();
    // Multi: Space checks the focused box; Tab to the next and check it too.
    await page.keyboard.press('Space');
    await page.keyboard.press('Tab');
    await expect(form.getByRole('checkbox', { name: 'Bundle budget' })).toBeFocused();
    await page.keyboard.press('Space');
    await expect(form.getByRole('checkbox', { name: 'Bed rows' })).toBeChecked();
    await expect(form.getByRole('checkbox', { name: 'Bundle budget' })).toBeChecked();
    // Range: the slider, by arrows (24 → 28), said with its unit.
    await page.keyboard.press('Tab');
    const slider = form.getByRole('slider');
    await expect(slider).toBeFocused();
    await page.keyboard.press('ArrowRight');
    await page.keyboard.press('ArrowRight');
    await expect(slider).toHaveAttribute('aria-valuetext', '28 KB');
    // Rank: Tab past the number box to the first item, then Down moves it.
    const stopsToRank = await tabUntil(page, f => f.inFrame && f.name.includes('Inputs'), 4);
    expect(noRing(stopsToRank)).toEqual([]);
    await page.keyboard.press('ArrowDown');
    await expect(frame.locator('hc-rank [aria-live="polite"]')).toHaveText('Inputs: position 2 of 3');
    // Matrix: Tab into each row's radio group; arrows pick within it.
    const stopsToMatrix = await tabUntil(page, f => f.inFrame && f.name === 'Speed: Low', 8);
    expect(noRing(stopsToMatrix)).toEqual([]);
    await page.keyboard.press('ArrowRight');
    await expect(form.getByRole('radio', { name: 'Speed: Medium' })).toBeChecked();
    await page.keyboard.press('Tab');
    await expect(form.getByRole('radio', { name: 'Risk: Low' })).toBeFocused();
    await page.keyboard.press('Space');
    await expect(form.getByRole('radio', { name: 'Risk: Low' })).toBeChecked();
    // Tab leaves the frame and reaches Hollow's Send; Enter sends the answers.
    const sends = await calls(page, 'chat_send');
    const out = await tabUntil(page, f => !f.inFrame && f.name === 'Send answers', 20);
    expect(noRing(out)).toEqual([]);
    await page.keyboard.press('Enter');
    await expect.poll(() => calls(page, 'chat_send')).toBe(sends + 1);
    const prompt = String((await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}'))).prompt);
    const text = prompt.slice(prompt.indexOf('Answered canvas "release-plan":'));
    expect(text).toMatch(/^Answered canvas "release-plan":/);
    expect(text).toMatch(/"order":\["Draft to the composer","Inputs","Lock after sending"\]/);
    expect(text).toMatch(/"budget":28/);
    expect(text).toMatch(/"fit":\{"Speed":"Medium","Risk":"Low"\}/);
    await expect(composer(page)).toHaveValue('');
    await turn(page, text, 'run-ask-2', 'Thanks, noted.', { send: false });

    // Locked: the fields are inert, Send is gone, the answer is shown, and the host says so.
    await expect(form.getByRole('status')).toHaveText('Answered. You sent these answers.');
    await expect(canvas.locator('.chat-ask-send')).toHaveCount(0);
    await expect(canvas.locator('[data-ask-state="answered"]')).toHaveText('Answered. You sent this canvas’s answers.');
    await expect(canvas.locator('[data-ask-state="answered"]')).toHaveAttribute('role', 'status');
    await axe(page, info, 'ask-canvas locked', '.work-conversation', { frame: true });
  });
});

// ─── Project files ──────────────────────────────────────────────────────
// The workspace panel's six sections went in the COO rebuild L4 (2026-11-10):
// the project's notes and its Results open as files in the Preview pane.

const NOTES = ['WORKSPACE.md', 'PLAN.md', 'SOURCES.md'] as const;

/** Glass leaves the page transparent over macOS's own material
 *  (NSVisualEffectView, behind the window); a browser has white there,
 *  which would grade text over a backdrop the app never has. So under glass
 *  the test paints an opaque stand-in for the material behind the page: a
 *  lighter grey than the dark material (about #1e1e1e–#2b2b2b) and a darker
 *  grey than the light one (about #ececec), each the less legible side for
 *  that theme's ink. The real material over a
 *  real wallpaper is not measured here (CANVAS_C6.md). */
const MATERIAL = { dark: '#3a3a3c', light: '#d4d4d6' };

async function openDemo(page: Page, theme: 'light' | 'dark', glass: boolean) {
  await page.addInitScript(([t, g, m]) => {
    localStorage.setItem('theme', t); localStorage.setItem('glass', g ? 'on' : 'off');
    if (g) document.addEventListener('DOMContentLoaded', () => { const st = document.createElement('style'); st.textContent = `html { background: ${m} !important; }`; document.head.append(st); });
  }, [theme, glass, MATERIAL[theme]] as const);
  await page.goto('/?mock=1&demo=1');
  await expect(page.locator('.home-project-row').first()).toBeVisible();
  await page.goto('/?mock=1');
  if (!(await page.locator('.work-card').first().isVisible())) await page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true }).click();
  await showWorkspaceList(page);
  await page.locator('.work-card').filter({ hasText: 'Q3 regional operations review' }).click();
  if (glass) await expect(page.locator('html')).toHaveAttribute('data-glass', 'on');
  else await expect(page.locator('html')).not.toHaveAttribute('data-glass', /.*/);
}

/** Wait for the Preview pane's file to settle (no skeleton or spinner). */
async function settledPane(page: Page) {
  const pane = previewPane(page);
  await expect(pane).toBeVisible();
  await expect(pane.locator('.ui-skeleton, .chat-preview-loading, [aria-busy="true"]')).toHaveCount(0, { timeout: 10_000 });
  return pane;
}

test.describe('project files at narrow width', () => {
  test.beforeEach(({}, info) => { test.skip(!info.project.name.includes('narrow'), 'narrow width is the check'); });

  for (const [theme, glass] of [['light', false], ['dark', false], ['dark', true], ['light', true]] as const) {
    const name = `${glass ? 'glass ' : ''}${theme}`;
    test(`axe in ${name}: the project's notes and a Result under its review bar, no serious or critical violations`, async ({ page }, info) => {
      test.setTimeout(90_000);
      await openDemo(page, theme, glass);
      await openProjectFiles(page);
      for (const note of NOTES) {
        await choose(filesPicker(page), { label: note });
        await expect(previewPane(page).getByRole('tab', { name: note, exact: true })).toHaveAttribute('aria-selected', 'true');
        await settledPane(page);
        await axe(page, info, `${note} (${name})`, '#preview-pane');
      }
      await openWorkspaceResult(page, 'local-q3-review', 'workspaces/local-q3-review/outputs/q3-regional-review.md');
      await expect(previewPane(page).getByRole('region', { name: 'Result review', exact: true })).toBeVisible();
      await settledPane(page);
      await axe(page, info, `Result review (${name})`, '#preview-pane');
    });
  }
});
