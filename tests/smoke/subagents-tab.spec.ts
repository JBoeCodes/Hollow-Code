/**
 * The Subagents tab (COO rebuild L3): a chat's Subagents chip counts what it
 * started and opens the Preview pane's Subagents tab; the tab lists Active
 * (running, then queued with its place) and Done, each row its title with
 * when it ended (live elapsed while it runs) at the right and model, effort,
 * time and tokens under it, durations unspaced like the chat's ("4m 12s",
 * "16h ago"); a row opens that subagent's session in the same tab,
 * with the CFO's choice under its header, Stop (asks first), a composer that queues behind a running turn, and
 * a back arrow to the list. A "Started a subagent" line in the transcript
 * opens the tab on that subagent. Against the demo mock with
 * `subagentsFixture.ts` (two active, three done).
 */
import { expect, test, type Page } from './fixtures/test';
import { RUNNING, SUBAGENTS_AT, seedSubagents } from './subagentsFixture';

test.skip(({ viewport }) => (viewport?.width ?? 1000) <= 860, 'the chat and the Preview pane side by side');

const calls = (page: Page, name: string) => page.evaluate(n => window.__mockApiCalls?.[n] ?? 0, name);

/** The page's time, held still (timers keep running), so elapsed is exact. */
const at = (seconds: number) => new Date(Date.parse(SUBAGENTS_AT) + seconds * 1000);

async function openQ3(page: Page, older = 0) {
  await page.clock.setFixedTime(at(0));
  await seedSubagents(page, older);
  await page.goto('/?mock=1&demo=1');
  await expect(page.getByRole('region', { name: 'Projects' })).toBeVisible();
  await page.getByRole('button', { name: 'Open Q3 regional operations review', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Q3 regional operations review');
  await page.locator('.side-card-toggle').filter({ hasText: 'Subagents' }).click();
}

function tab(page: Page) {
  return page.getByRole('complementary', { name: 'Preview' }).getByRole('tabpanel', { name: 'Subagents' });
}

test('the chip and the tab count, tick and open a subagent', async ({ page }) => {
  await openQ3(page);
  // The chip: three distinct agents' glyphs and the counts.
  const chip = page.getByRole('button', { name: /^Subagents:/ });
  await expect(chip).toHaveAccessibleName('Subagents: 1 running, 1 queued, 3 done');
  await expect(chip.locator('.agent-mark')).toHaveCount(4);
  // Glyphs then the counts, like ChatGPT's popover line; the name is in the label.
  await expect(chip.locator('[aria-live="polite"]')).toHaveText('1 running, 1 queued, 3 done');
  await chip.click();
  const list = tab(page);
  await expect(list.getByRole('heading', { name: 'Active · 2' })).toBeVisible();
  await expect(list.locator('.subagents-done > summary')).toBeVisible();
  // Running first, its live working time at the right and model, effort
  // and tokens under the title; queued second with its place; done rows say
  // when they ended at the right and how long they worked underneath.
  const rows = list.getByRole('listitem');
  await expect(rows.nth(0).locator('.subagents-row-title')).toHaveText('Check the labour figures');
  await expect(rows.nth(0).locator('.subagents-row-when')).toHaveText('4m 12s');
  await expect(rows.nth(0).locator('button')).toHaveAttribute('title', /Sonnet 5.5 · Medium/);
  await expect(rows.nth(0).locator('.subagents-cost')).toHaveCount(0);
  await expect(list.locator('.subagents-done')).not.toHaveAttribute('open');
  await list.locator('.subagents-done > summary').click();
  await expect(rows.nth(1)).toContainText('Draft the appendix table');
  await expect(rows.nth(1).locator('.subagents-row-when')).toHaveText('Queued, 2nd');
  await expect(rows.nth(2)).toContainText('Company review');
  await expect(rows.nth(2).locator('.subagents-row-when')).toHaveText('32s');
  await expect(rows.nth(3).locator('button')).toHaveAttribute('title', /1.2M tokens/);
  await expect(rows.nth(4).locator('.subagents-row-when')).toHaveText('32s');
  // The wall span and the cached tokens are the row's tooltip.
  await expect(list.getByRole('button', { name: /^Check the labour figures/ })).toHaveAttribute('title', /worked 4m 12s over 5m 0s\n150K in \(90K cached\) · 32K out/);
  // Working time ticks while it runs; a done row's doesn't.
  await page.clock.setFixedTime(at(10));
  await expect(rows.nth(0).locator('.subagents-row-when')).toHaveText('4m 22s');
  await expect(rows.nth(2).locator('.subagents-row-when')).toContainText('32s');
  // Open it: back arrow, title, header line (live), its issue, the transcript.
  await list.getByRole('button', { name: /^Check the labour figures/ }).click();
  const session = page.getByRole('region', { name: 'Subagent Check the labour figures' });
  await expect(session.getByRole('heading', { name: 'Check the labour figures' })).toBeVisible();
  await expect(session.getByRole('button', { name: 'Back to subagents', exact: true })).toBeFocused();
  await expect(session.locator('.subagents-session-meta')).toContainText('Sonnet 5.5 · Medium · 4m 22s · 182K tokens');
  await expect(session.locator('.subagents-issue')).toHaveText('ENG-301');
  await page.clock.setFixedTime(at(15));
  await expect(session.locator('.subagents-session-meta')).toContainText('4m 27s');
  // The CFO's choice for its latest turn (lane 2's run manifest decision).
  await expect(session.locator('.subagents-session-cfo')).toHaveText('CFO: Claude · Sonnet 5.5 · Medium · checks figures against a source');
  await expect(session).toContainText('Check the labour figures in section 3 against the September close.');
  await expect(session).toContainText('Worked for 32s');
  await expect(session.getByRole('button', { name: 'Copy reply', exact: true })).toBeVisible();
  // The latest reply shows its row at rest (opacity, so toBeVisible alone can't tell).
  await expect(session.locator('.subagents-reply-foot').last()).toHaveClass(/\blatest\b/);
  await expect(session.locator('.subagents-foot-actions').last()).toHaveCSS('opacity', '1');
  await expect(session).toContainText('Section 3 matches the close except one overtime figure');
  await expect(session.getByRole('button', { name: 'Show change in history', exact: true })).toBeVisible();
  // Back: the list again, the opened row focused.
  await session.getByRole('button', { name: 'Back to subagents', exact: true }).click();
  await expect(list.getByRole('button', { name: /^Check the labour figures/ })).toBeFocused();
  await expect(list.getByRole('heading', { name: 'Active · 2' })).toBeVisible();
});

test('a message to a running subagent queues, and Stop asks first', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockChatHold', '1'));
  await openQ3(page);
  await page.getByRole('button', { name: /^Subagents:/ }).click();
  await tab(page).getByRole('button', { name: /^Check the labour figures/ }).click();
  const session = page.getByRole('region', { name: 'Subagent Check the labour figures' });
  // The composer sends to its conversation, queued behind the running turn.
  const box = session.getByRole('textbox', { name: 'Message this subagent' });
  await expect(box).toBeEnabled();
  await box.fill('Also check the overtime table.');
  await session.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(session.getByRole('status').filter({ hasText: 'Queued' })).toBeVisible();
  await expect(box).toHaveValue('');
  const args = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}'));
  expect(args).toMatchObject({ prompt: 'Also check the overtime table.', sessionKey: RUNNING, after: 'queue', agent: 'claude' });
  // Stop ends its turn, so it asks first; Cancel stops nothing.
  await session.getByRole('button', { name: 'Stop', exact: true }).click();
  const confirm = page.getByRole('dialog', { name: 'Stop “Check the labour figures”?' });
  await expect(confirm).toContainText('The chat that started it sees it was stopped');
  await confirm.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(confirm).toBeHidden();
  expect(await calls(page, 'chat_cancel')).toBe(0);
  await session.getByRole('button', { name: 'Stop', exact: true }).click();
  await confirm.getByRole('button', { name: 'Stop', exact: true }).click();
  await expect.poll(() => page.evaluate(() => localStorage.getItem('mockCancelledChatKey'))).toBe(RUNNING);
  // Back to the list.
  await session.getByRole('button', { name: 'Back to subagents', exact: true }).click();
  await expect(tab(page).locator('.subagents-done > summary')).toBeVisible();
  // A queued one has no turn to stop yet.
  await tab(page).getByRole('button', { name: /^Draft the appendix table/ }).click();
  const queued = page.getByRole('region', { name: 'Subagent Draft the appendix table' });
  await expect(queued.locator('.subagents-session-meta')).toContainText('Queued, 2nd');
  await expect(queued.getByRole('button', { name: 'Stop', exact: true })).toHaveCount(0);
});

test('a Started a subagent line opens the tab on that subagent', async ({ page }) => {
  // Tauri event delivery on top of the mock IPC (as chat-video.spec.ts).
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
  const emit = (payload: Record<string, unknown>) => page.evaluate(event => {
    (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void }).__emitTauriEvent('chat-event', event);
  }, payload);
  await openQ3(page);
  const composer = page.getByLabel('Chat message');
  const sent = await calls(page, 'chat_send');
  await composer.fill('Check the labour figures with a helper');
  await composer.press('Meta+Enter');
  await expect.poll(() => calls(page, 'chat_send')).toBeGreaterThan(sent);
  await emit({ type: 'runStarted', asked: 'Check the labour figures with a helper', runId: 'run-start-sub', recall: [] });
  await emit({ type: 'toolUse', id: 'start-1', name: 'mcp__hollow_app__start_session', input: { workspace: 'local-q3-review', prompt: 'Check the labour figures in section 3 against the September close.', agent: 'claude' } });
  await emit({ type: 'toolResult', id: 'start-1', content: JSON.stringify({ sessionKey: RUNNING, queuePosition: 0 }), isError: false });
  // The call is one line, named by the subagent's title, with its state.
  const line = page.getByRole('button', { name: /^Started a subagent: Check the labour figures/ });
  await expect(line).toBeVisible();
  await expect(line).toContainText('Running');
  await line.click();
  await expect(page.getByRole('region', { name: 'Subagent Check the labour figures' }).getByRole('heading', { name: 'Check the labour figures' })).toBeVisible();
});

test('Done is the current conversation; Show older pages in the rest of the chain', async ({ page }) => {
  await openQ3(page, 60);
  // The card counts the current conversation, then yields to the pane.
  await expect(page.getByRole('button', { name: /^Subagents:/ })).toHaveAccessibleName('Subagents: 1 running, 1 queued, 3 done');
  await page.getByRole('button', { name: /^Subagents:/ }).click();
  const list = tab(page);
  await expect(page.locator('#side-panel-card')).toHaveCount(0);
  await expect(list.locator('.subagents-done > summary')).toBeVisible();
  await expect(list.getByText('Older subagent 1', { exact: true })).toHaveCount(0);
  await list.locator('.subagents-done > summary').click();
  // A visible control, in batches of 25, gone once none are left.
  const more = list.getByRole('button', { name: /^Show older/ });
  await expect(more).toHaveText('Show older (60)');
  await more.click();
  await expect(list.getByRole('heading', { name: 'Older · 25' })).toBeVisible();
  await expect(more).toHaveText('Show older (35)');
  await more.click();
  await more.click();
  await expect(list.getByRole('heading', { name: 'Older · 60' })).toBeVisible();
  await expect(more).toHaveCount(0);
});
