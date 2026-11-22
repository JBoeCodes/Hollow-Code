import { expect, test, type Page } from './fixtures/test';
import { chooseNew } from './choose';

async function bridge(page: Page) {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (event: unknown) => void>(), listeners = new Map<string, number[]>(); let next = 1;
    const w = window as unknown as Record<string, unknown>;
    w.__TAURI_INTERNALS__ = {
      transformCallback(callback: (event: unknown) => void) { const id = next++; callbacks.set(id, callback); return id; },
      invoke(command: string, args: { event?: string; handler?: number }) {
        if (command === 'plugin:event|listen' && args?.event && args.handler) {
          listeners.set(args.event, [...(listeners.get(args.event) ?? []), args.handler]); return Promise.resolve(next++);
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
const emit = (page: Page, payload: Record<string, unknown>) => page.evaluate(e => {
  (window as unknown as { __emitTauriEvent: (n: string, p: unknown) => void }).__emitTauriEvent('chat-event', e);
}, payload);
const flow = 'flowchart LR\na[Start] -->|yes| b{Ready?}\nb --> c([Finish])';
async function reply(page: Page, text: string, finish = true) {
  const composer = page.getByLabel('Chat message');
  const before = await page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0);
  await composer.fill('Show the flow'); await composer.press('Meta+Enter');
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBe(before + 1);
  await emit(page, { type: 'runStarted', asked: 'Show the flow', runId: 'run-mermaid', recall: [] });
  await emit(page, { type: 'assistantDelta', text });
  if (finish) await emit(page, { type: 'done', sessionId: null });
}
test.beforeEach(async ({ page }) => {
  await bridge(page); await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Draw a checked diagram.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Diagrams');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Diagrams');
});
for (const theme of ['light', 'dark']) test(`draws Mermaid and preserves source (${theme})`, async ({ page }, info) => {
  await page.evaluate(t => { document.documentElement.dataset.theme = t; }, theme);
  await reply(page, `Here is the flow.\n\n\`\`\`mermaid\n${flow}\n\`\`\``);
  const artifact = page.locator('.work-conversation .chat-artifact').last();
  await expect(artifact.locator('.chat-artifact-badge')).toHaveText('MERMAID');
  await expect(artifact).toHaveAttribute('data-sandbox', 'offline');
  await expect(artifact.frameLocator('iframe').locator('hc-flow svg')).toBeVisible();
  await expect(artifact.frameLocator('iframe').locator('hc-flow svg')).toContainText('Start');
  await expect(page.locator('.work-conversation .chat-canvas-note')).toContainText('Drawn from Mermaid:');
  await artifact.screenshot({ path: info.outputPath(`mermaid-${theme}.png`) });
  await artifact.locator('summary').click();
  await expect(page.getByRole('menuitem', { name: /Run scripts|Allow internet/ })).toHaveCount(0);
  await page.evaluate(() => { navigator.clipboard.writeText = async text => { (window as unknown as { copied: string }).copied = text; }; });
  await page.getByRole('menuitem', { name: 'Copy source', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { copied?: string }).copied)).toBe(flow);
  await artifact.locator('summary').click();
  await page.getByRole('menuitem', { name: 'View source', exact: true }).click();
  await expect(artifact.locator('pre')).toHaveText(flow);
  await expect(artifact.locator('pre')).toBeFocused();
  await artifact.locator('summary').click();
  await page.getByRole('menuitem', { name: 'Read as text', exact: true }).click();
  await expect(artifact.getByRole('region', { name: 'Text of this diagram' })).toContainText('Start → Ready?: yes');
});
test('unsupported diagrams remain source with a reason', async ({ page }) => {
  await reply(page, '```mermaid\nsequenceDiagram\nAlice->>Bob: Hi\n```');
  await expect(page.locator('.work-conversation .chat-canvas-note')).toHaveText('Not drawn: sequence diagram. Hollow draws Mermaid flowcharts.');
  await expect(page.locator('.work-conversation .chat-msg pre')).toContainText('Alice->>Bob: Hi');
  await expect(page.locator('.work-conversation .chat-msg iframe')).toHaveCount(0);
});
test('streaming keeps code until the reply ends', async ({ page }) => {
  await reply(page, `\`\`\`mermaid\n${flow}\n\`\`\``, false);
  await expect(page.locator('.work-conversation .chat-msg pre')).toContainText('flowchart LR');
  await expect(page.locator('.work-conversation .chat-msg iframe')).toHaveCount(0);
  await emit(page, { type: 'done', sessionId: null });
  await expect(page.locator('.work-conversation .chat-artifact iframe')).toHaveCount(1);
});
test('a Mermaid file opens as a diagram and keeps its Source view', async ({ page }) => {
  await reply(page, 'Open `route.mmd`.');
  // Nothing opens beside the conversation until the file link is clicked
  // (run 15 P1: the side panel is gone; the Preview pane opens on a click).
  const pane = page.getByRole('complementary', { name: 'Preview' });
  await expect(pane).toHaveCount(0);
  await page.locator('.work-conversation code.file-ref').last().click();
  await expect(pane.getByRole('tab', { selected: true })).toHaveAttribute('title', /^Diagram\b/);
  await expect(pane.frameLocator('iframe').locator('hc-flow svg')).toContainText('Finish');
  await pane.getByRole('button', { name: 'Source', exact: true }).click();
  await expect(pane.locator('pre.chat-preview-source')).toHaveText('flowchart LR\nstart[Start] --> finish[Finish]');
});

for (const surface of ['reply', 'file']) test(`a cold ${surface} chunk preserves the conversation and draft`, async ({ page }) => {
  let release!: () => void, requested = false;
  const ready = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/MermaidDiagram.tsx*', async route => { requested = true; await ready; await route.continue(); });
  try {
    if (surface === 'reply') await reply(page, `\`\`\`mermaid\n${flow}\n\`\`\``);
    else {
      await reply(page, 'Open `route.mmd`.');
      await page.locator('.work-conversation code.file-ref').last().click();
    }
    await expect.poll(() => requested).toBe(true);
    const fallback = surface === 'reply' ? page.locator('.work-conversation .chat-msg pre') : page.locator('.chat-preview-source');
    await expect(fallback).toContainText('flowchart LR');
    const covered = surface === 'file' && (page.viewportSize()?.width ?? 1440) <= 1100;
    if (covered) {
      await expect(page.locator('.work-conversation')).toBeAttached();
      await expect(page.locator('.work-conversation')).toBeHidden();
      await page.getByRole('button', { name: 'Close preview', exact: true }).click();
    }
    await expect(page.getByRole('region', { name: 'Workspace conversation', exact: true })).toBeVisible();
    const composer = page.getByLabel('Chat message');
    await expect(composer).toBeVisible();
    await composer.fill('Keep this draft while the diagram loads');
    // ⌘\ brings the Preview pane's last item back.
    if (covered) await page.keyboard.press('Meta+Backslash');
    release();
    await expect(page.locator(surface === 'reply' ? '.work-conversation .chat-artifact iframe' : '.chat-preview-body iframe')).toHaveCount(1);
    if (covered) await page.getByRole('button', { name: 'Close preview', exact: true }).click();
    await expect(composer).toBeVisible();
    await expect(composer).toHaveValue('Keep this draft while the diagram loads');
  } finally { release(); }
});
