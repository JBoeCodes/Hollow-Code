import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { chooseNew, openProjectFiles, previewPane } from '../smoke/choose';
import { dirname } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { openLibraryView } from '../smoke/libraryView';

/**
 * Frontier-UI review captures. Seeds a realistic session (three workspaces, a
 * finished agent turn with tool calls and a Markdown reply) and saves one
 * screenshot per primary surface under test-results/frontier/<label>/, so a
 * before/after pair can be compared by eye. Mock data only (?mock=1); the
 * native window chrome (traffic lights, vibrancy) is not visible here.
 * FRONTIER_LABEL names the capture set (default "current").
 */
const LABEL = process.env.FRONTIER_LABEL ?? 'current';
// FRONTIER_CHROME=overlay previews the macOS overlay title bar's layout.
const URL = process.env.FRONTIER_CHROME === 'overlay' ? '/?mock=1&chrome=overlay' : '/?mock=1';
// Each theme variant writes its own folder: <label>/light, <label>/dark, <label>/glass-dark.
// FRONTIER_THEMES=light,dark limits the set (default: all three).
const THEMES = (process.env.FRONTIER_THEMES ?? 'light,dark,glass-dark').split(',').map(t => t.trim()).filter(Boolean);
// FRONTIER_STYLES=1 also writes <name>.styles.json beside each screenshot: the
// computed style of every rendered element, keyed by its DOM path. Diffing two
// labels (scripts/style-diff.mjs) proves a stylesheet refactor changed nothing
// it did not mean to. Motion is reduced in that mode so no frame is mid-animation.
const STYLES = !!process.env.FRONTIER_STYLES;
const CLOCK = STYLES ? Date.parse('2026-09-30T09:41:00') : null;
const STYLE_PROPS = [
  'display', 'position', 'width', 'height', 'padding', 'margin', 'gap', 'grid-template-columns',
  'color', 'background-color', 'background-image', 'opacity', 'visibility',
  'border-top', 'border-right', 'border-bottom', 'border-left', 'border-radius', 'outline',
  'box-shadow', 'text-shadow', 'filter', 'backdrop-filter', 'transform',
  'font-family', 'font-size', 'font-weight', 'line-height', 'letter-spacing', 'text-transform',
  // Layout and behaviour a fold can silently flip (added for the shell.css fold, P2).
  'min-width', 'max-width', 'min-height', 'max-height', 'top', 'right', 'bottom', 'left', 'z-index',
  'flex', 'flex-direction', 'flex-wrap', 'align-items', 'align-self', 'align-content', 'justify-content',
  'justify-self', 'justify-items', 'grid-template-rows', 'grid-column', 'grid-row', 'grid-auto-flow',
  'grid-auto-columns', 'overflow-x', 'overflow-y', 'text-overflow', 'white-space', 'overflow-wrap',
  'text-align', 'font-style', 'cursor', 'transition', 'animation-name', 'clip', 'object-fit',
  'user-select', 'accent-color', 'outline-offset', 'list-style-type', 'list-style-position', 'resize',
  'scrollbar-width', 'pointer-events', 'overscroll-behavior-y', 'text-decoration-line', 'box-sizing',
];
const PSEUDO_PROPS = [
  'content', 'background-color', 'background-image', 'box-shadow', 'opacity', 'border-radius', 'color',
  'display', 'position', 'top', 'right', 'bottom', 'left', 'width', 'height', 'pointer-events',
];
// FRONTIER_WIDTHS=1200,900,800,560,collapsed (styles mode only) also dumps each
// surface at those viewport widths (`<name>@<w>.styles.json`) and, for
// `collapsed`, with the rail collapsed at 1440. A narrow width at or under
// 860px also dumps the rail opened over the page (`@<w>-rail`). The viewport
// and the rail are restored before the flow continues, so the 1440 captures
// are unchanged. Media queries and the collapsed rail are otherwise unseen.
const WIDTHS = (process.env.FRONTIER_WIDTHS ?? '').split(',').map(t => t.trim()).filter(Boolean);
// FRONTIER_PROBE=<file.json>: a JSON array of selectors. The 1440 and
// `collapsed` dumps also write `<name>.probe.json` mapping every selector to
// the elements it matches (indices into the dump's key order), so a
// refactor can check which rules the capture actually exercises and which
// selectors reach the same elements.
const PROBE: string[] = process.env.FRONTIER_PROBE ? JSON.parse(readFileSync(process.env.FRONTIER_PROBE, 'utf8')) : [];
async function dumpStyles(page: Page, file: string, withProbe = true) {
  const data = await page.evaluate(({ props, pseudoProps, probe }: { props: string[]; pseudoProps: string[]; probe: string[] }) => {
    const out: Record<string, Record<string, string>> = {};
    const index = new Map<Element, number>();
    const pathOf = (el: Element): string => {
      const parts: string[] = [];
      for (let e: Element | null = el; e && e !== document.documentElement; e = e.parentElement) {
        const parent: Element | null = e.parentElement;
        const idx = parent ? Array.prototype.indexOf.call(parent.children, e) : 0;
        const cls = typeof e.className === 'string' ? e.className.trim().split(/\s+/).filter(Boolean).slice(0, 3).join('.') : '';
        parts.unshift(`${e.tagName.toLowerCase()}${cls ? '.' + cls : ''}:${idx}`);
      }
      return parts.join('>');
    };
    for (const el of Array.from(document.body.querySelectorAll('*'))) {
      if (el.closest('svg') && el.tagName.toLowerCase() !== 'svg') continue;
      const r = (el as HTMLElement).getBoundingClientRect?.();
      if (!r || (r.width === 0 && r.height === 0)) continue;
      const cs = getComputedStyle(el);
      const rec: Record<string, string> = {};
      for (const p of props) rec[p] = cs.getPropertyValue(p);
      for (const pseudo of ['::before', '::after']) {
        const ps = getComputedStyle(el, pseudo);
        if (ps.content && ps.content !== 'none' && ps.content !== 'normal') {
          for (const p of pseudoProps) rec[`${pseudo}${p}`] = ps.getPropertyValue(p);
        }
      }
      index.set(el, index.size);
      out[pathOf(el)] = rec;
    }
    // Probe hits are indices into the dump's element order (the key order of
    // <name>.styles.json); elements without a box are not listed.
    const matched: Record<string, number[]> = {};
    for (const sel of probe) {
      try {
        const hits = Array.from(document.querySelectorAll(sel)).map(e => index.get(e)).filter((i): i is number => i !== undefined);
        if (hits.length) matched[sel] = hits;
      } catch { /* a selector this engine can't parse matches nothing */ }
    }
    return { out, matched };
  }, { props: STYLE_PROPS, pseudoProps: PSEUDO_PROPS, probe: withProbe ? PROBE : [] });
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, JSON.stringify(data.out));
  if (withProbe && PROBE.length) writeFileSync(file.replace(/\.styles\.json$/, '.probe.json'), JSON.stringify(data.matched));
}
const railCollapsed = (page: Page) => page.evaluate(() => !!document.querySelector('.work-rail.collapsed'));
// Clicked from script: a palette or modal backdrop must not swallow it, and
// focus stays where the flow left it.
const toggleRail = (page: Page) => page.evaluate(() => {
  (document.querySelector('.top-bar button[aria-label$="sidebar"]') as HTMLButtonElement | null)?.click();
});
const settle = (page: Page) => page.evaluate(() => new Promise<void>(r => requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(r, 60)))));
async function dumpWidths(page: Page, base: string) {
  const wasCollapsed = await railCollapsed(page);
  for (const token of WIDTHS) {
    if (token === 'collapsed') {
      await page.setViewportSize({ width: 1440, height: 900 });
      await settle(page);
      const open = !(await railCollapsed(page));
      if (open) { await toggleRail(page); await settle(page); }
      await dumpStyles(page, `${base}@collapsed.styles.json`);
      if (open) { await toggleRail(page); await settle(page); }
      continue;
    }
    const w = Number(token);
    await page.setViewportSize({ width: w, height: 900 });
    await settle(page);
    await dumpStyles(page, `${base}@${w}.styles.json`, false);
    if (w <= 860 && token === WIDTHS.find(t => Number(t) <= 860)) {
      const collapsedNow = await railCollapsed(page);
      if (collapsedNow) { await toggleRail(page); await settle(page); }
      await dumpStyles(page, `${base}@${w}-rail.styles.json`, false);
      if (collapsedNow) { await toggleRail(page); await settle(page); }
    }
  }
  await page.setViewportSize({ width: 1440, height: 900 });
  await settle(page);
  if ((await railCollapsed(page)) !== wasCollapsed) { await toggleRail(page); await settle(page); }
}
const shotFor = (variant: string) => async (page: Page, name: string) => {
  const base = `test-results/frontier/${LABEL}/${variant}/${name}`;
  // Styles mode parks the pointer off the page, so no dump depends on what a
  // resize happened to slide under it (hover is re-evaluated on a timer).
  if (STYLES) await page.mouse.move(-10, -10);
  await page.screenshot({ path: `${base}.png`, fullPage: false });
  if (STYLES) await dumpStyles(page, `${base}.styles.json`);
  if (STYLES && WIDTHS.length) await dumpWidths(page, base);
};

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
  (window as unknown as { __emitTauriEvent: (n: string, p: unknown) => void }).__emitTauriEvent('chat-event', event);
}, payload);

async function createWorkspace(page: Page, name: string, goal: string, onOpen?: () => Promise<void>) {
  await chooseNew(page, 'New workspace');
  if (onOpen) { await page.waitForTimeout(150); await onOpen(); }
  await page.getByRole('textbox', { name: 'Desired result' }).fill(goal);
  await page.getByRole('textbox', { name: 'Workspace name' }).fill(name);
  await page.getByRole('button', { name: 'Create workspace', exact: true }).click();
  await expect(page.locator('.work-header h1, [data-workspace-title]').first()).toHaveText(name);
}
const nav = async (page: Page, name: string) => {
  await page.getByLabel('Main navigation').getByRole('button', { name: new RegExp(`^${name}`) }).first().click();
  // Pages are lazy chunks: wait for the page to replace its loading state, so
  // a capture never depends on how fast the chunk arrived.
  await expect(page.locator('.page-loading')).toHaveCount(0, { timeout: 10_000 }).catch(() => {});
};

const REPLY = [
  'I traced the failure to the rail’s `selectWorkspace` path. It only navigated **after** a successful read, so a slow or failed read looked like a dead click.',
  '',
  '### What changed',
  '- Navigation happens first; the read fills in afterwards.',
  '- A failed read now shows a banner *and* a toast.',
  '- The connected-project button is no longer blocked by a running chat.',
  '',
  '```ts',
  'export function selectWorkspace(id: string) {',
  '  navigate({ page: "work", workspaceId: id });',
  '  void readWorkspace(id).catch(showOpenFailure);',
  '}',
  '```',
  '',
  'Tests: vitest 532 passed, tsc clean.',
].join('\n');

for (const variant of THEMES) {
  const width = 1440;
  test(`frontier capture at ${width}px (${variant})`, async ({ page }) => {
    const shot = shotFor(variant);
    if (STYLES) test.setTimeout(WIDTHS.length || PROBE.length ? 900_000 : 180_000);
    await page.setViewportSize({ width, height: 900 });
    if (STYLES) await page.emulateMedia({ reducedMotion: 'reduce' });
    await installEventBridge(page);
    // Theme is read from localStorage at module load (store/theme.ts), so set
    // it before any app script runs. Glass rides on a dark base.
    await page.addInitScript((v: string) => {
      try {
        localStorage.setItem('theme', v === 'light' ? 'light' : 'dark');
        localStorage.setItem('glass', v.startsWith('glass') ? 'on' : 'off');
      } catch { /* ignore */ }
    }, variant);
    // Styles mode pins the page clock (timers still run), so time labels such as
    // a reply's "12:48 AM" render the same width in every capture set.
    if (CLOCK) await page.clock.setFixedTime(CLOCK);
    await page.goto(URL);
    // A cold first boot on the static server can outlast the 5s default when
    // another worker is loading the app at the same time.
    await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true }).first()).toBeVisible({ timeout: 15_000 });
    // Prove the variant actually applied before shooting anything: the
    // polarity attribute, the glass overlay, and the rendered canvas colour
    // (a light canvas under "dark" would otherwise pass silently).
    const applied = await page.evaluate(() => {
      const el = document.documentElement;
      const rgb = getComputedStyle(document.querySelector('.canvas') ?? el).backgroundColor.match(/[\d.]+/g)?.map(Number) ?? [];
      const canvasHex = getComputedStyle(el).getPropertyValue('--canvas').trim();
      const hex = canvasHex.replace('#', '');
      const [r, g, b] = hex.length === 6 ? [0, 2, 4].map(i => parseInt(hex.slice(i, i + 2), 16)) : rgb;
      return { theme: el.dataset.theme, glass: el.getAttribute('data-glass'), luminance: (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255, canvasBg: rgb };
    });
    expect(applied.theme, `data-theme for ${variant}`).toBe(variant === 'light' ? 'light' : 'dark');
    expect(applied.glass === 'on', `data-glass for ${variant}`).toBe(variant.startsWith('glass'));
    if (variant === 'light') expect(applied.luminance).toBeGreaterThan(0.8);
    else expect(applied.luminance).toBeLessThan(0.2);
    if (!variant.startsWith('glass')) {
      // Opaque themes paint the canvas; its rendered colour must match the polarity.
      const [r, g, b] = applied.canvasBg;
      const lum = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
      if (variant === 'light') expect(lum).toBeGreaterThan(0.8); else expect(lum).toBeLessThan(0.2);
    }
    await shot(page, 'home-empty');

    await createWorkspace(page, 'Q4 parking revenue model', 'A revenue model for the Q4 board pack.', () => shot(page, 'new-workspace'));
    await nav(page, 'Projects');
    await createWorkspace(page, 'TipFlow tip reconciliation', 'Reconcile September TipFlow tips against HRIS.');
    await nav(page, 'Projects');
    await createWorkspace(page, 'Rail navigation fix', 'The rail opens workspaces from any page.');
    await shot(page, 'workspace-empty');

    const composer = page.getByRole('textbox', { name: 'Chat message' });
    await composer.fill('Why does clicking a workspace in the rail do nothing from Settings?');
    await composer.press('Meta+Enter');
    await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBeGreaterThan(0);
    const { clientTurnId, sessionKey } = await page.evaluate(() => {
      const a = JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}');
      return { clientTurnId: a.clientTurnId, sessionKey: a.sessionKey || 's-20260926-mock0001' };
    });
    await emit(page, { type: 'runStarted', runId: 'run-cap-1', sessionKey, recall: [], clientTurnId });
    await emit(page, { type: 'assistantText', text: 'Let me find where the rail opens a workspace.', clientTurnId });
    await emit(page, { type: 'toolUse', id: 't1', name: 'Grep', input: { pattern: 'selectWorkspace', path: 'src' }, clientTurnId });
    await emit(page, { type: 'toolResult', id: 't1', content: 'src/store.ts:412\nsrc/components/Rail.tsx:88', isError: false, clientTurnId });
    await emit(page, { type: 'turnBoundary', clientTurnId }); // native: one agent message per call
    await emit(page, { type: 'toolUse', id: 't2', name: 'Read', input: { file_path: 'src/store.ts' }, clientTurnId });
    await emit(page, { type: 'toolResult', id: 't2', content: 'ok', isError: false, clientTurnId });
    await emit(page, { type: 'turnBoundary', clientTurnId }); // native: one agent message per call
    await emit(page, { type: 'toolUse', id: 't3', name: 'Edit', input: { file_path: 'src/store.ts', old_string: 'await readWorkspace(id);\n  navigate(', new_string: 'navigate(' }, clientTurnId });
    await emit(page, { type: 'toolResult', id: 't3', content: 'ok', isError: false, clientTurnId });
    await emit(page, { type: 'turnBoundary', clientTurnId }); // native: one agent message per call
    await emit(page, { type: 'toolUse', id: 't4', name: 'Bash', input: { command: 'npx vitest run src/navigation.test.ts', description: 'Run navigation tests' }, clientTurnId });
    await page.waitForTimeout(150);
    await shot(page, 'chat-working');
    await emit(page, { type: 'toolResult', id: 't4', content: '✓ 12 passed', isError: false, clientTurnId });
    await emit(page, { type: 'turnBoundary', clientTurnId });
    // Stream the answer in two chunks: the first capture shows the caret.
    const cut = REPLY.indexOf('### What changed');
    await emit(page, { type: 'assistantDelta', text: REPLY.slice(0, cut).trimEnd(), clientTurnId });
    await page.waitForTimeout(150);
    await shot(page, 'chat-streaming');
    await emit(page, { type: 'assistantDelta', text: `\n\n${REPLY.slice(cut)}`, clientTurnId });
    const now = new Date(CLOCK ?? Date.now()).toISOString();
    await emit(page, {
      type: 'evidenceReceipt', clientTurnId,
      receipt: {
        id: 'r-cap-1', runId: 'run-cap-1', source: 'chat', sourceId: null, startedAt: new Date((CLOCK ?? Date.now()) - 94_000).toISOString(), finishedAt: now,
        status: 'success', verificationState: 'notVerified', checks: [], failures: [], historyRunId: 'run-cap-1', sessionKey,
        files: [
          { path: 'src/store.ts', changeType: 'modify', attribution: 'confirmed', tool: 'Edit', additions: 14, deletions: 6 },
          { path: 'src/components/Rail.tsx', changeType: 'modify', attribution: 'confirmed', tool: 'Edit', additions: 3, deletions: 2 },
        ],
      },
    });
    await emit(page, { type: 'done', sessionId: null, sessionKey, clientTurnId });
    await page.waitForTimeout(200);
    await shot(page, 'chat-done');
    // Frontier F5: the finished run's steps fold into one row; open it.
    const fold = page.locator('.chat-run-fold-head');
    await expect(fold).toHaveText(/Worked for 1m 3\ds · 4 steps/);
    await fold.click();
    await page.waitForTimeout(150);
    await shot(page, 'chat-done-expanded');
    // Scrolled up, the transcript offers "Jump to latest".
    await page.locator('.work-conversation .chat-scroll').evaluate(el => { el.scrollTop = 0; el.dispatchEvent(new Event('scroll')); });
    await expect(page.getByRole('button', { name: 'Jump to latest' })).toBeVisible();
    await shot(page, 'chat-jump-latest');
    await page.getByRole('button', { name: 'Jump to latest' }).click();
    await expect(page.getByRole('button', { name: 'Jump to latest' })).toHaveCount(0);
    await fold.click();
    // ⌘K in a conversation offers its actions (frontier F6).
    await page.keyboard.press('Meta+k');
    await page.waitForTimeout(150);
    await shot(page, 'palette-conversation');
    await page.keyboard.press('Escape');
    // Frontier F7, since COO rebuild L3/L4 (2026-11-10): the side panel is the
    // Preview pane, with Files, Browser and Subagents tabs (the workspace
    // panel, its sections and Logs are gone).
    await openProjectFiles(page);
    const pane = previewPane(page);
    await expect(pane.getByRole('tab', { name: 'WORKSPACE.md', exact: true })).toHaveAttribute('aria-selected', 'true');
    await page.waitForTimeout(150);
    await shot(page, 'side-pane-files');
    await pane.getByRole('button', { name: 'New tab', exact: true }).click();
    await pane.getByRole('button', { name: /^Browser/ }).click();
    await expect(pane.locator('.browser-start')).toBeVisible();
    await page.waitForTimeout(150);
    await shot(page, 'side-pane-browser');
    // A chat with no subagents shows no chip, so open its tab through the store.
    await page.evaluate(async parent => {
      const store = '/src/store/preview.ts';
      (await import(/* @vite-ignore */ store)).openPreview({ kind: 'subagents', parent });
    }, sessionKey);
    await expect(pane.getByRole('tabpanel', { name: 'Subagents' })).toBeVisible();
    await page.waitForTimeout(150);
    await shot(page, 'side-pane-subagents');
    await pane.getByRole('button', { name: 'Close preview', exact: true }).click();
    await expect(pane).toHaveCount(0);

    await nav(page, 'Projects');
    await page.waitForTimeout(150);
    await shot(page, 'home');
    await nav(page, 'Tasks');
    await page.waitForTimeout(150);
    await shot(page, 'tasks');
    await nav(page, 'Library');
    await page.waitForTimeout(150);
    await shot(page, 'library');
    await openLibraryView(page, 'Notes');
    // An open file (library-has-file) restyles the list and the reader. The
    // mock's default folder is empty, so browse all files first.
    const browseAll = page.getByRole('button', { name: 'Browse all files' });
    if (await browseAll.count()) await browseAll.first().click();
    await page.locator('.library-item').first().waitFor({ timeout: 3_000 }).catch(() => {});
    if (await page.locator('.library-item').count()) await shot(page, 'library-all');
    if (await page.locator('.library-item').count()) {
      await page.locator('.library-item').first().click();
      await page.waitForTimeout(250);
      await shot(page, 'library-file');
    }
    await nav(page, 'Automations');
    await page.waitForTimeout(150);
    await shot(page, 'automations');
    if (await page.locator('.workflow-row').count()) {
      await page.locator('.workflow-row').first().click();
      await page.waitForTimeout(250);
      await shot(page, 'automation-detail');
    }
    await nav(page, 'Settings');
    await page.waitForTimeout(150);
    await shot(page, 'settings');
    const section = (name: string) => page.getByRole('navigation', { name: 'Settings sections' })
      .getByRole('button', { name: new RegExp(`^${name}`) }).first().click();
    await section('General');
    await page.waitForTimeout(150);
    await shot(page, 'settings-interface');
    await section('Models');
    await page.waitForTimeout(150);
    await shot(page, 'settings-agents');
    await section('Connectors');
    await page.waitForTimeout(250);
    await shot(page, 'settings-connectors');
    await section('Personalization');
    await page.waitForTimeout(250);
    await shot(page, 'settings-about');
    // Skills are in Library, Skills since Settings PR 4: Connectors has a link
    // row (after the switch's own scroll to the top has run).
    await section('Connectors');
    await page.getByRole('button', { name: 'Open Library, Skills', exact: true }).click();
    await page.waitForTimeout(250);
    await shot(page, 'settings-skills');
    if (await page.locator('.skills-row').count()) {
      await page.locator('.skills-row').first().click();
      await page.waitForTimeout(250);
      await shot(page, 'settings-skill-detail');
    }
    await page.keyboard.press('Meta+k');
    await page.waitForTimeout(150);
    await shot(page, 'palette');
    await page.keyboard.press('Escape');

    await nav(page, 'Tasks');
    const layout = page.getByRole('group', { name: 'Task layout' });
    await layout.getByRole('button', { name: 'Board', exact: true }).click();
    await page.waitForTimeout(150);
    await shot(page, 'tasks-board');
    await expect(layout.getByRole('button', { name: 'Agenda', exact: true })).toHaveCount(0);
    await layout.getByRole('button', { name: 'List', exact: true }).click();
    await page.getByRole('button', { name: /New task/ }).first().click();
    await page.waitForTimeout(200);
    await shot(page, 'task-editor');
    await page.getByRole('button', { name: /^Open task:/ }).first().click();
    await page.waitForTimeout(200);
    await shot(page, 'task-detail');
  });
}
