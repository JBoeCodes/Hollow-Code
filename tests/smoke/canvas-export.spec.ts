import { expect, test, type Page } from './fixtures/test';
import { ROUTES } from '../../src/routeLedger';
import { home, walk } from './routeWalk';
import { previewPane, resultReviewBar } from './choose';

const route = ROUTES.find(r => r.id === 'canvas.save-image')!;
test('export names the operation and opens its saved file in the Preview pane', async ({ page }) => {
  await home(page, route);
  await walk(page, route.walk.slice(0, -1));
  await page.getByRole('menuitem', { name: 'Export canvas…', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Export canvas', exact: true });
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(dialog.getByRole('status')).toContainText('Saved outputs/exports/');
  // The export IPC mock returns a receipt; model the file written by the
  // native exporter so the Preview pane can read it.
  await page.evaluate(async () => {
    const { api } = await import('/src/api.ts');
    const request = JSON.parse(localStorage.getItem('mockCanvasLastExport')!);
    const ws = (await api.listWorkspaces()).find(w => w.id === request.workspaceId)!;
    const notes = JSON.parse(localStorage.getItem('mockWorkspaceNotes') ?? '{}');
    notes[`${ws.path}/outputs/exports/release-status-v1-doc.png`] = 'mock exported image';
    localStorage.setItem('mockWorkspaceNotes', JSON.stringify(notes));
  });
  await dialog.getByRole('button', { name: 'Show the file', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  // Its workspace opens with the file in the Preview pane, under its review bar.
  await expect(previewPane(page).getByRole('tab', { name: 'release-status-v1-doc.png', exact: true })).toHaveAttribute('aria-selected', 'true');
  await expect(resultReviewBar(page)).toBeVisible();
});
async function openExport(page: Page, source = '<hc-canvas id="release-status" title="Release status"><p>Build 2</p></hc-canvas>') {
  await home(page, route);
  await page.evaluate(async source => {
    const { api } = await import('/src/api.ts');
    const workspaces = await api.listWorkspaces();
    const ws = workspaces.find(w => !w.id.startsWith('_') && w.kind !== 'build')!;
    const c = await api.readCanvas({ workspaceId: ws.id }, 'release-status');
    await api.recordCanvas({ workspaceId: ws.id, sessionKey: 's-ledger' }, source, c!.sha256);
    const latest = await api.readCanvas({ workspaceId: ws.id }, 'release-status');
    await api.saveCanvasState({ workspaceId: ws.id }, 'release-status', { tab: 'Details' }, latest!.sha256);
  }, source);
  await walk(page, route.walk);
  const dialog = page.getByRole('dialog', { name: 'Export canvas', exact: true });
  await expect(dialog).toBeFocused();
  return dialog;
}
test('a gallery canvas exports its selected revision and options', async ({ page }) => {
  const dialog = await openExport(page);
  await dialog.getByRole('combobox', { name: 'Look', exact: true }).click();
  await page.getByRole('option', { name: 'Light', exact: true }).click();
  await dialog.getByRole('checkbox', { name: 'Also copy to clipboard (PNG)' }).check();
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(dialog.getByRole('status')).toContainText('Saved outputs/exports/release-status-v2-doc.png');
  await expect(dialog.getByRole('status')).toContainText('Copied to clipboard');
  const request = await page.evaluate(() => JSON.parse(localStorage.getItem('mockCanvasLastExport')!));
  expect(request).toMatchObject({ revision: 2, format: 'png', preset: 'doc', look: 'light', copy: true, target: 'file', scheme: 'light' });
  expect(request.source).toContain('Build 2');
  expect(request.state).toEqual({ tab: 'Details' });
  expect(await page.evaluate(() => window.__mockApiCalls?.canvas_export)).toBe(1);
  await page.screenshot({ path: `${process.env.HOLLOW_SCRATCH_DIR ?? test.info().outputDir}/canvas-export-success.png` });
  await dialog.getByRole('combobox', { name: 'Format', exact: true }).click();
  await page.getByRole('option', { name: 'PDF document', exact: true }).click();
  await expect(dialog.getByRole('status')).toContainText('2× pixels');
});

for (const look of ['Light', 'Dark']) test(`exports one selected static flow as SVG in ${look}`, async ({ page }) => {
  const source = '<hc-canvas id="release-status" title="Release status"><hc-flow label="First">a: Start\nb: Finish\na -> b</hc-flow><hc-flow label="Second" direction="right">x: Review &amp; approve {decision, success}\ny: Ship\nx -> y: Ready</hc-flow><script>window.svgExportMustNotRun=true</script></hc-canvas>';
  const dialog = await openExport(page, source);
  await dialog.getByRole('combobox', { name: 'Format', exact: true }).click();
  await page.getByRole('option', { name: 'SVG flow', exact: true }).click();
  await dialog.getByRole('combobox', { name: 'Flow', exact: true }).click();
  await page.getByRole('option', { name: '2. Second', exact: true }).click();
  await dialog.getByRole('combobox', { name: 'Look', exact: true }).click();
  await page.getByRole('option', { name: look, exact: true }).click();
  await expect(dialog.getByRole('checkbox')).toBeDisabled();
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(dialog.getByRole('status')).toContainText(`release-status-v2-doc${look === 'Dark' ? '-dark' : ''}-flow-2.svg`);
  const request = await page.evaluate(() => JSON.parse(localStorage.getItem('mockCanvasLastExport')!));
  expect(request).toMatchObject({ format: 'svg', flowIndex: 2, copy: false, source });
  expect(request.svg).toContain('Review &amp; approve');
  expect(request.svg).not.toContain('svgExportMustNotRun');
  expect(request.svg).not.toContain('>First<');
  const parsed = await page.evaluate(svg => {
    const doc = new DOMParser().parseFromString(svg, 'image/svg+xml');
    return { errors: doc.querySelectorAll('parsererror').length, nodes: doc.querySelectorAll('text').length };
  }, request.svg);
  expect(parsed.errors).toBe(0);
  expect(parsed.nodes).toBeGreaterThan(2);
  await page.screenshot({ path: `${process.env.HOLLOW_SCRATCH_DIR ?? test.info().outputDir}/flow-svg-${look.toLowerCase()}.png` });
  // Verify the actual generated file renders without the app or kit.
  await page.goto(`data:image/svg+xml,${encodeURIComponent(request.svg)}`);
  await expect(page.locator('svg')).toBeVisible();
  await page.screenshot({ path: `${process.env.HOLLOW_SCRATCH_DIR ?? test.info().outputDir}/flow-svg-file-${look.toLowerCase()}.png` });
});

test('offers no SVG for non-flows or flow-looking script/template text', async ({ page }) => {
  const dialog = await openExport(page, '<hc-canvas id="release-status" title="Release status"><script>const example = "<hc-flow>a -> b</hc-flow>"</script><template><hc-flow>a -> b</hc-flow></template><p>No diagram</p></hc-canvas>');
  await dialog.getByRole('combobox', { name: 'Format', exact: true }).click();
  await expect(page.getByRole('option', { name: 'SVG flow', exact: true })).toHaveCount(0);
  const extracted = await page.evaluate(async () => {
    const { canvasFlows } = await import('/src/canvas/flowSvg.ts');
    return canvasFlows('<hc-canvas id="safe" title="Safe"><hc-flow label="A">a: &lt;img src=https://invalid.example/leak&gt;</hc-flow><script>throw 1</script></hc-canvas>');
  });
  expect(extracted).toEqual([{ label: 'A', text: 'a: <img src=https://invalid.example/leak>', direction: 'down' }]);
});
test('a failed export says why and can retry as PDF', async ({ page }) => {
  const dialog = await openExport(page);
  await page.evaluate(() => localStorage.setItem('mockCanvasExportError', 'The canvas did not finish drawing within 15 seconds. Nothing was saved.'));
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(dialog.getByRole('alert')).toContainText('Nothing was saved');
  await expect(dialog.getByRole('status')).toHaveCount(0);
  await page.evaluate(() => localStorage.removeItem('mockCanvasExportError'));
  await dialog.getByRole('combobox', { name: 'Format', exact: true }).click();
  await page.getByRole('option', { name: 'PDF document', exact: true }).click();
  await expect(dialog.getByRole('checkbox')).toBeDisabled();
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(dialog.getByRole('status')).toContainText('.pdf');
  await expect(dialog.getByRole('alert')).toHaveCount(0);
});

test('older revisions export their own body and Escape restores menu focus', async ({ page }) => {
  const dialog = await openExport(page);
  await expect(dialog).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Canvas actions', exact: true })).toBeFocused();
  await page.getByRole('button', { name: 'v2', exact: true }).click();
  await page.getByRole('group', { name: 'Versions of Release status' }).getByRole('button', { name: /^v1/ }).click();
  await page.getByRole('button', { name: 'Canvas actions', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Export canvas…', exact: true }).click();
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(dialog.getByRole('status')).toContainText('release-status-v1-doc');
  const request = await page.evaluate(() => JSON.parse(localStorage.getItem('mockCanvasLastExport')!));
  expect(request.revision).toBe(1);
  expect(request.source).toContain('Build 1');
  expect(request.state).toBeNull();
});

for (const theme of ['light', 'dark', 'glass']) test(`export dialog ${theme} and current frame state`, async ({ page }) => {
  await page.addInitScript(t => {
    localStorage.setItem('theme', t === 'light' ? 'light' : 'dark');
    localStorage.setItem('glass', t === 'glass' ? 'on' : 'off');
  }, theme);
  const dialog = await openExport(page);
  await page.keyboard.press('Escape');
  const frame = page.frameLocator('.result-canvas iframe');
  await frame.locator('body').click();
  await frame.locator('body').evaluate(() => window.parent.postMessage({ __hc: 1, kind: 'state', canvasId: 'release-status', state: { tab: 'Changed' } }, '*'));
  await expect.poll(() => page.evaluate(async () => {
    const { lastCanvas } = await import('/src/store/lastCanvas.ts');
    return lastCanvas()?.ref?.state;
  })).toEqual({ tab: 'Changed' });
  await page.getByRole('button', { name: 'Canvas actions', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Export canvas…', exact: true }).click();
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(dialog.getByRole('status')).toContainText('Saved outputs/exports/');
  const request = await page.evaluate(() => JSON.parse(localStorage.getItem('mockCanvasLastExport')!));
  expect(request.state).toEqual({ tab: 'Changed' });
  expect(request.look).toBe('match');
  expect(request.scheme).toBe(theme === 'light' ? 'light' : 'dark');
  await page.screenshot({ path: `${process.env.HOLLOW_SCRATCH_DIR ?? test.info().outputDir}/canvas-export-${theme}.png` });
});

test('gallery comparison exports each side with only the current saved state', async ({ page }) => {
  const dialog = await openExport(page);
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Compare versions', exact: true }).click();
  const sides = page.locator('.cg-compare figure');
  await expect(sides).toHaveCount(2);
  for (const index of [0, 1]) {
    await sides.nth(index).getByRole('button', { name: 'Canvas actions', exact: true }).click();
    await sides.nth(index).getByRole('menuitem', { name: 'Export canvas…', exact: true }).click();
    await dialog.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(dialog.getByRole('status')).toContainText(`release-status-v${index + 1}-doc`);
    const request = await page.evaluate(() => JSON.parse(localStorage.getItem('mockCanvasLastExport')!));
    expect(request.state).toEqual(index ? { tab: 'Details' } : null);
    expect(request.source).toContain(`Build ${index + 1}`);
    await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  }
});

for (const custom of [false, true]) test(`storeless capture host renders chart and forty-node flow before ready (custom=${custom})`, async ({ page }) => {
  let document = '';
  const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
  const nodes = Array.from({ length: 40 }, (_, i) => `n${i}: Step ${i + 1}`).join('\n');
  const links = Array.from({ length: 39 }, (_, i) => `n${i} -> n${i + 1}`).join('\n');
  const source = `<hc-canvas id="capture-test" title="Capture test"><hc-chart type="bar" label="Painted chart">A: 10\nB: 25</hc-chart><hc-flow label="Forty steps">${nodes}\n${links}</hc-flow><p id="saved"></p><script>window.firstCanvasColor=getComputedStyle(document.documentElement).getPropertyValue('--app-canvas');addEventListener('hollow-init',()=>document.querySelector('#saved').textContent=hollowCanvas.get('tab'))</script></hc-canvas>`;
  await page.exposeFunction('captureInvoke', (command: string, args: Record<string, unknown>) => {
    calls.push({ command, args });
    if (command === 'canvas_capture_job') return { width: 800, request: { source, format: 'png', preset: 'doc', look: custom ? 'match' : 'light', scheme: 'light', theme: custom ? [['--app-canvas', '#fef0d0'], ['--app-ink', '#112233'], ['--app-surface', '#fef0d0'], ['--app-primary', '</style><script>window.badTheme=true</script>']] : [], state: { tab: 'Remembered view' } } };
    if (command === 'artifact_put') { document = (args.parts as string[]).join('capture-test-nonce'); return 'capture-document'; }
    if (command === 'canvas_capture_ready' || command === 'canvas_capture_failed') return null;
    throw new Error(`Unexpected storeless command: ${command}`);
  });
  await page.addInitScript(() => {
    if (window !== window.top) return;
    (window as any).__TAURI_INTERNALS__ = {
      invoke: (command: string, args: Record<string, unknown>) => (window as any).captureInvoke(command, args),
      convertFileSrc: () => `${location.origin}/capture-document`,
    };
  });
  await page.route('**/capture-document', route => route.fulfill({ body: document, contentType: 'text/html' }));
  await page.goto('/?window=canvas-capture');
  await expect.poll(() => calls.filter(c => c.command === 'canvas_capture_ready').length).toBe(1);
  expect(calls.some(c => c.command === 'canvas_capture_failed')).toBe(false);
  expect(calls.find(c => c.command === 'artifact_put')?.args.mode).toBe('canvas');
  expect(calls.find(c => c.command === 'canvas_capture_ready')?.args.height).toBeGreaterThan(100);
  await expect(page.frameLocator('iframe').locator('#saved')).toHaveText('Remembered view');
  if (custom) {
    expect(await page.frameLocator('iframe').locator('html').evaluate(() => (window as any).firstCanvasColor)).toBe('#fef0d0');
    expect(document).not.toContain('window.badTheme');
    await expect(page.locator('body')).toHaveCSS('background-color', 'rgb(254, 240, 208)');
  }
  const painted = await page.frameLocator('iframe').locator('hc-chart canvas').evaluate((node: HTMLCanvasElement) => {
    const data = node.getContext('2d')!.getImageData(0, 0, node.width, node.height).data;
    return data.some((n, i) => i % 4 === 3 && n > 0);
  });
  expect(painted).toBe(true);
  await expect(page.frameLocator('iframe').locator('hc-flow svg')).toContainText('Step 40');
  await page.screenshot({ path: `${process.env.HOLLOW_SCRATCH_DIR ?? test.info().outputDir}/canvas-capture-host.png`, fullPage: true });
});

const paper = { name: 'Paper / A', source: '', accent: '#aaaaaa', foreground: '#999999', background: '#fef0d0', isDark: false, colors: Array(16).fill(null), cursor: null, selectionForeground: null, selectionBackground: null, success: null, warning: null, error: null };
for (const format of ['PNG image', 'PDF document', 'SVG flow']) test(`workspace look exports full palette as ${format}`, async ({ page }) => {
  await page.addInitScript(theme => localStorage.setItem('mockCustomThemes', JSON.stringify([theme])), paper);
  const source = '<hc-canvas id="release-status" title="Release status"><hc-flow label="Release">a -> b</hc-flow></hc-canvas>';
  const dialog = await openExport(page, source);
  await page.evaluate(async () => {
    const { api } = await import('/src/api.ts');
    const { canvasExportTarget } = await import('/src/store/canvasExport.ts');
    const { refreshWorkspaces } = await import('/src/store/workspaces.ts');
    await api.setWorkspaceCanvasTheme(canvasExportTarget()!.ref!.workspaceId, 'Paper / A');
    await refreshWorkspaces();
  });
  await dialog.getByRole('combobox', { name: 'Look', exact: true }).click();
  await page.getByRole('option', { name: "This workspace's look", exact: true }).click();
  await expect(dialog.getByText(/contrast adjustments/)).toBeVisible();
  await dialog.getByRole('combobox', { name: 'Format', exact: true }).click();
  await page.getByRole('option', { name: format, exact: true }).click();
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(dialog.getByRole('status')).toContainText('-paper-a');
  const request = await page.evaluate(() => JSON.parse(localStorage.getItem('mockCanvasLastExport')!));
  expect(request).toMatchObject({ look: 'match', themeName: 'Paper / A', scheme: 'light', source, state: { tab: 'Details' } });
  expect(request.theme.length).toBeGreaterThan(40);
  expect(Object.fromEntries(request.theme)['--app-canvas']).toBe(paper.background);
  expect(Object.fromEntries(request.theme)['--app-ink']).not.toBe(paper.foreground);
  if (format === 'SVG flow') expect(request.svg).toContain('rgba(254,240,208,1)');
  await page.screenshot({ path: `${process.env.HOLLOW_SCRATCH_DIR ?? test.info().outputDir}/r16b-${format.slice(0,3)}.png` });
});

test('custom export choice does not change the workspace; missing theme refuses and app fallback works', async ({ page }) => {
  await page.addInitScript(theme => localStorage.setItem('mockCustomThemes', JSON.stringify([theme])), paper);
  const dialog = await openExport(page);
  await dialog.getByRole('combobox', { name: 'Look', exact: true }).click();
  await page.getByRole('option', { name: paper.name, exact: true }).click();
  await expect(dialog.getByRole('button', { name: 'Save', exact: true })).toBeEnabled();
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(dialog.getByRole('status')).toContainText('-paper-a.png');
  expect(await page.evaluate(async () => {
    const { canvasExportTarget } = await import('/src/store/canvasExport.ts');
    const { api } = await import('/src/api.ts');
    return (await api.listWorkspaces()).find(w => w.id === canvasExportTarget()!.ref!.workspaceId)?.canvasTheme;
  })).toBeFalsy();
  // A file disappearing after preview must be caught by the fresh save read.
  await page.evaluate(() => localStorage.setItem('mockCustomThemes', '[]'));
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(dialog.getByRole('alert')).toBeVisible();
  await page.evaluate(async () => { const s = await import('/src/store/theme.ts'); await s.refetchCustomThemes(); });
  await expect(dialog.getByRole('status')).toContainText('unavailable on this Mac');
  await expect(dialog.getByRole('button', { name: 'Save', exact: true })).toBeDisabled();
  await dialog.getByRole('combobox', { name: 'Look', exact: true }).click();
  await page.getByRole('option', { name: 'App theme', exact: true }).click();
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(dialog.getByRole('status')).toContainText('Saved outputs/exports/');
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('mockCanvasLastExport')!).themeName)).toBeUndefined();
});
