import { expect, test, type Page } from './fixtures/test';
import { choose, expectConversationWithPane, previewPane, resultReviewBar } from './choose';

/** The Conductor-style reply footer (components/TurnFooter.tsx): while the
 * agent works, the slot under the reply shows what it is doing and a live
 * clock; once the run is done it shows the duration and finish time, copy,
 * feedback, a menu, and the changed files as `name +a −d` chips. Events come
 * through a Tauri event bridge; commands go through the `?mock=1` IPC. */

function callCount(page: Page, command: string) {
  return page.evaluate(cmd => window.__mockApiCalls?.[cmd] ?? 0, command);
}

/** Tauri event delivery for `listen()`, as in chat-workspace.spec.ts:
 * commands still go through the mock IPC. */
async function installEventBridge(page: Page) {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (event: unknown) => void>();
    const listeners = new Map<string, number[]>();
    let next = 1;
    const w = window as unknown as Record<string, unknown>;
    w.__TAURI_INTERNALS__ = {
      transformCallback(callback: (event: unknown) => void) {
        const id = next++;
        callbacks.set(id, callback);
        return id;
      },
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

function emitChatEvent(page: Page, payload: Record<string, unknown>) {
  return page.evaluate(event => {
    (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void })
      .__emitTauriEvent('chat-event', event);
  }, payload);
}

function narrow(page: Page) {
  return (page.viewportSize()?.width ?? 1000) <= 860;
}

async function openSettingsSection(page: Page, id: string, label: RegExp) {
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  if (narrow(page)) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), id);
  } else {
    await page.locator('.settings-index').getByRole('button', { name: label }).click();
  }
}

async function showChat(page: Page) {
  await openSettingsSection(page, 'advanced', /^Advanced/);
  await page.locator('#settings-group-this-mac > summary').click(); // History & recovery: Advanced → This Mac
  await page.getByRole('button', { name: 'Open Library, History', exact: true }).click();
  await page.locator('.library-tabs').getByRole('button', { name: 'Conversations', exact: true }).click(); // Settings PR 4 cut the Conversation history button
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toBeVisible();
}

async function openChat(page: Page) {
  await page.goto('/?mock=1');
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await showChat(page);
}

async function openConversation(page: Page, _source: 'hollow', title: string) {
  const search = page.getByRole('textbox', { name: 'Search conversations' });
  if (!await search.isVisible()) {
    await page.getByRole('button', { name: 'Show conversations', exact: true }).click();
  }
  await expect(search).toBeVisible();
  // One list since run 15 P4 removed Claude history and its source switch.
  await page.locator('.chat-aside').getByRole('button', { name: `Open ${title}`, exact: true }).click();
}

const MOCK_KEY = 's-20260926-foot0001';

test('the working row becomes the reply footer with file chips', async ({ page }, info) => {
  await installEventBridge(page);
  await openChat(page);
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await composer.fill('Update the docs');
  await composer.press('Meta+Enter');
  await expect.poll(() => callCount(page, 'chat_send')).toBe(1);
  const clientTurnId = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}').clientTurnId);

  await emitChatEvent(page, { type: 'runStarted', runId: 'run-foot-1', sessionKey: MOCK_KEY, recall: [], clientTurnId });
  const live = page.locator('.chat-main .turn-footer.live');
  await expect(live).toBeVisible();
  await expect(live).toContainText('Thinking');
  await expect(live.locator('.turn-footer-time')).toHaveText(/^\d+s$/);
  await expect(live.locator('.chat-working-mark svg')).toHaveCount(1);

  await emitChatEvent(page, { type: 'toolUse', id: 'tu-1', name: 'Read', input: { file_path: '/vault/CLAUDE.md' }, clientTurnId });
  await expect(live).toContainText('CLAUDE.md');
  await page.screenshot({ path: info.outputPath('turn-footer-working.png') });
  await emitChatEvent(page, { type: 'toolResult', id: 'tu-1', content: 'ok', isError: false, clientTurnId });
  await emitChatEvent(page, { type: 'assistantText', text: 'The docs are updated. See `CLAUDE.md` and `docs/BUILD_LOG.md`.', clientTurnId });
  // "Worked for" runs from the receipt's start to the last tool result (C1),
  // which arrived a few hundred ms ago, so the span is 713 s less a moment:
  // the finish stamp is the same instant, and the line is matched to the second
  // within a small window.
  const startMs = Date.now() - 713_000;
  const started = new Date(startMs).toISOString();
  const finished = new Date(startMs + 713_000).toISOString();
  await emitChatEvent(page, {
    type: 'evidenceReceipt',
    clientTurnId,
    receipt: {
      id: 'receipt-foot-1', runId: 'run-foot-1', source: 'chat', sourceId: null, startedAt: started, finishedAt: finished,
      status: 'success', verificationState: 'notVerified', checks: [], failures: [], historyRunId: 'run-foot-1', sessionKey: MOCK_KEY,
      files: [
        { path: 'CLAUDE.md', changeType: 'modify', attribution: 'confirmed', tool: 'Edit', additions: 35, deletions: 10 },
        { path: 'docs/BUILD_LOG.md', changeType: 'modify', attribution: 'confirmed', tool: 'Edit', additions: 39, deletions: 0 },
        { path: 'docs/WORKSPACES.md', changeType: 'modify', attribution: 'confirmed', tool: 'Edit', additions: 12, deletions: 4 },
        { path: 'src/new.ts', changeType: 'create', attribution: 'observed', tool: null, additions: 20, deletions: 0 },
      ],
    },
  });
  await emitChatEvent(page, { type: 'done', sessionId: null, sessionKey: MOCK_KEY, clientTurnId });

  await expect(live).toHaveCount(0);
  const footer = page.locator('.chat-main .turn-footer');
  await expect(footer).toHaveCount(1);
  // Frontier F5: the run's step folds into "Worked for 11m 53s" (the step
  // count is read out, not shown) above the answer, so the footer shows only
  // when it finished.
  await expect(page.locator('.chat-main .chat-run-fold-head')).toHaveText(/^Worked for 11m 5[0-3]s · 1 step$/);
  await expect(footer.locator('.turn-footer-time')).toHaveText(/^\d{1,2}:\d{2}/);
  // Overnight 11-12 (C2): a folded reply's chips sit above its "Worked for"
  // line, not in the footer.
  const chat = page.locator('.chat-main');
  const chips = chat.getByRole('list', { name: 'Changed files' }).getByRole('listitem');
  await expect(chips).toHaveCount(3);
  await expect(footer.getByRole('list', { name: 'Changed files' })).toHaveCount(0);
  // EH4: a list of plain buttons, one in each listitem (a button with
  // role=listitem is no button to assistive tech), still reached by Tab.
  await expect(chat.getByRole('list', { name: 'Changed files' }).getByRole('button')).toHaveCount(3);
  await expect(chat.locator('.turn-chips [role="listitem"] > button.turn-chip')).toHaveCount(3);
  await expect(chat.locator('button[role="listitem"]')).toHaveCount(0);
  await expect(chips.nth(0)).toHaveText('CLAUDE.md+35−10');
  await expect(chips.nth(1)).toHaveText('BUILD_LOG.md+39−0');
  await expect(chips.nth(2)).toHaveText('+2 more+32−4');
  // The receipt stays folded until asked for.
  await expect(page.locator('.chat-main .execution-receipt')).toHaveCount(0);
  await page.screenshot({ path: info.outputPath('turn-footer-done.png') });
  await chips.nth(2).click();
  await expect(page.locator('.chat-main .execution-receipt')).toHaveCount(1);
  await expect(chips.nth(2).getByRole('button')).toHaveAttribute('aria-expanded', 'true');
  await chips.nth(2).click();
  await expect(page.locator('.chat-main .execution-receipt')).toHaveCount(0);
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});

test('a failed run keeps its receipt on screen', async ({ page }) => {
  await installEventBridge(page);
  await openChat(page);
  await emitChatEvent(page, { type: 'runStarted', runId: 'run-foot-2', sessionKey: MOCK_KEY, recall: [] });
  await emitChatEvent(page, { type: 'assistantText', text: 'The check failed.' });
  await emitChatEvent(page, {
    type: 'evidenceReceipt',
    receipt: {
      id: 'receipt-foot-2', runId: 'run-foot-2', source: 'chat', sourceId: null, startedAt: new Date().toISOString(),
      finishedAt: new Date().toISOString(), status: 'error', verificationState: 'failedAfterChange',
      files: [{ path: 'notes/a.md', changeType: 'modify', attribution: 'confirmed', tool: 'Edit' }],
      checks: [{ command: 'npm test', kind: 'test', outcome: 'failed', sequence: 3, excerpt: '1 failed' }],
      failures: [], historyRunId: 'run-foot-2', sessionKey: MOCK_KEY,
    },
  });
  await emitChatEvent(page, { type: 'done', sessionId: null, sessionKey: MOCK_KEY });
  await expect(page.locator('.chat-main .turn-footer')).toHaveCount(1);
  // No line counts were recorded: the chip shows the name only.
  await expect(page.locator('.chat-main .turn-chip')).toHaveText('a.md');
  await expect(page.locator('.chat-main .execution-receipt')).toHaveCount(1);
});

// P0-23: a reply can't look verified when its checks are stale or missing.
// The footer says so without opening anything; the chip opens the Run record.
async function finishReply(page: Page, runId: string, receipt: Record<string, unknown>) {
  await emitChatEvent(page, { type: 'runStarted', runId, sessionKey: MOCK_KEY, recall: [] });
  await emitChatEvent(page, { type: 'assistantText', text: 'Done: all tests pass.' });
  await emitChatEvent(page, {
    type: 'evidenceReceipt',
    receipt: {
      id: `receipt-${runId}`, runId, source: 'chat', sourceId: null, startedAt: new Date().toISOString(),
      finishedAt: new Date().toISOString(), status: 'success', verificationState: 'verifiedAfterChange',
      files: [{ path: 'src/a.ts', changeType: 'modify', attribution: 'confirmed', tool: 'Edit' }],
      checks: [{ command: 'npm test', kind: 'test', outcome: 'passed', sequence: 3, excerpt: 'ok' }],
      failures: [], historyRunId: runId, sessionKey: MOCK_KEY, ...receipt,
    },
  });
  await emitChatEvent(page, { type: 'done', sessionId: null, sessionKey: MOCK_KEY });
  await expect(page.locator('.chat-main .turn-footer:not(.live)')).toHaveCount(1);
}

test('a pass claim with no check after the last edit is named in the footer, and the chip opens the Run record', async ({ page }) => {
  await installEventBridge(page);
  await openChat(page);
  await finishReply(page, 'run-claim-1', { verificationState: 'staleAfterChange', verification: { claim: 'passed', claimWithoutCheck: true } });
  const footer = page.locator('.chat-main .turn-footer');
  const chip = footer.getByRole('button', { name: /Says it passes · no check after its last edit/ });
  await expect(chip).toBeVisible();
  await expect(page.locator('.chat-main .execution-receipt')).toHaveCount(0);
  await chip.click();
  const receipt = page.locator('.chat-main .execution-receipt');
  await expect(receipt).toHaveAttribute('open', '');
  await expect(receipt.locator('.execution-receipt-claim')).toHaveText('Says it passes · no check after its last edit');
  await expect(receipt.locator('summary.execution-receipt-summary')).toBeFocused();
  // A second click keeps it open: the chip shows the record, it doesn't toggle it.
  await chip.click();
  await expect(receipt).toHaveAttribute('open', '');
  await expect(receipt.locator('summary.execution-receipt-summary')).toBeFocused();
  // The shared wrapper must also let the chip reopen an existing disclosure.
  await receipt.locator('summary.execution-receipt-summary').click();
  await expect(receipt).not.toHaveAttribute('open', '');
  await chip.click();
  await expect(receipt).toHaveAttribute('open', '');
  await expect(receipt.locator('summary.execution-receipt-summary')).toBeFocused();
});

for (const [name, receipt, text] of [
  ['a failed last check', { verificationState: 'failedAfterChange', checks: [{ command: 'npm test', kind: 'test', outcome: 'failed', sequence: 3, excerpt: '1 failed' }], verification: { claim: 'passed', claimAfterFail: true } }, 'Says it passes · its last check failed'],
  ['stale checks', { verificationState: 'staleAfterChange' }, 'Checks out of date'],
  ['an unconfirmed done claim', { verificationState: 'notVerified', verification: { claim: 'done', claimWithoutCheck: true } }, 'Not verified'],
  ['cites that don\'t resolve', { cites: { checked: 3, seen: 1, pastEnd: 1, missing: 1, items: [{ path: 'src/a.ts', line: 900, status: 'pastEnd' }, { path: 'src/gone.ts', line: 1, status: 'missing' }] } }, '2 cites don\'t resolve'],
] as const) {
  test(`the footer names ${name} (P0-23)`, async ({ page }) => {
    await installEventBridge(page);
    await openChat(page);
    await finishReply(page, `run-${text.length}`, receipt);
    await expect(page.locator('.chat-main .turn-footer').getByRole('button', { name: text })).toBeVisible();
  });
}

test('an unverified change with no completion claim has no check chip', async ({ page }) => {
  await installEventBridge(page);
  await openChat(page);
  await finishReply(page, 'run-quiet-1', { verificationState: 'notVerified', checks: [], verification: { claim: 'none' } });
  await expect(page.locator('.chat-main .turn-footer .turn-footer-check')).toHaveCount(0);
});

test('a verified reply has no check chip (P0-23)', async ({ page }) => {
  await installEventBridge(page);
  await openChat(page);
  await finishReply(page, 'run-verified-1', {});
  await expect(page.locator('.chat-main .turn-footer .turn-footer-check')).toHaveCount(0);
});

// Matrix N8 (overnight 10-14/15 H3), revised 2026-11-04 (Alex: "any file
// I open that's created by the agent [should] open in the side pane if I
// click on it") and in the COO rebuild L4 (2026-11-10: the Results section
// went): a changed file in the workspace's outputs/ is a Result, and its chip
// opens it in the Preview pane under its review bar (Accept, Ask for a
// revision); the chip's right-click Review result does the same from its own
// workspace (`turnFooter.ts` `chipTarget`). The demo's Q3 conversation
// "Revise the labour section" changed outputs/q3-regional-review.md.
test('a changed file in outputs/ opens in the Preview pane with its review bar', async ({ page }, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'the Preview pane split is desktop-only');
  await page.goto('/?mock=1&demo=1');
  await expect(page.getByRole('region', { name: 'Projects' })).toBeVisible();
  await page.getByRole('button', { name: 'Open Q3 regional operations review', exact: true }).click();
  const chip = page.getByRole('list', { name: 'Changed files' }).getByRole('listitem').filter({ hasText: 'q3-regional-review.md' }).getByRole('button');
  await expect(chip).toHaveCount(1);
  await expect(chip).toHaveAttribute('title', /Preview$/);

  await chip.click();
  const pane = previewPane(page);
  await expect(pane.getByRole('tab', { selected: true })).toHaveText('q3-regional-review.md');
  const bar = resultReviewBar(page);
  await expect(bar).toBeVisible();
  await expect(bar.locator('.result-states')).toContainText('Draft');
  await expect(bar.getByRole('button', { name: 'Accept', exact: true })).toBeVisible();
  await expect(bar.getByRole('button', { name: 'Ask for a revision' })).toBeVisible();
  expect(await callCount(page, 'set_result_review')).toBe(0);
  await pane.getByRole('button', { name: 'Close preview', exact: true }).click();
  await expect(pane).toHaveCount(0);

  await chip.click({ button: 'right' });
  const menu = page.getByRole('menu');
  await expect(menu.getByRole('menuitem')).toHaveText(['Review result', 'Show change in history']);
  await menu.getByRole('menuitem', { name: 'Review result', exact: true }).click();
  await expect(pane.getByRole('tab', { selected: true })).toHaveText('q3-regional-review.md');
  await expect(bar).toBeVisible();
  // The conversation is beside the pane, or under it in a narrow window,
  // still the same one; Close preview brings it back alone, with the chip.
  await expectConversationWithPane(page);
  await pane.getByRole('button', { name: 'Close preview', exact: true }).click();
  await expect(chip).toBeVisible();
  expect(await callCount(page, 'set_result_review')).toBe(0);
});

// EH4 (review follow-ups to N8): the demo's Q3 conversation, with its first
// reply's receipt (and, for one test, its vault) changed, then reloaded
// without the demo so the mock reads them again (as revision-draft.spec.ts does).
const Q3 = 'workspaces/local-q3-review';
async function seedQ3(page: Page, change: { files?: unknown[]; deleted?: string }) {
  await page.goto('/?mock=1&demo=1');
  await expect(page.getByRole('region', { name: 'Projects' })).toBeVisible();
  await page.evaluate(({ files, deleted }) => {
    if (files) {
      const conversations = JSON.parse(localStorage.getItem('mockWorkspaceConversations') ?? '{}');
      const turn = conversations['s-20260930-q3a00001'].turns.find((t: { id: string }) => t.id === 'r1a-assistant');
      turn.receipt.files = files;
      localStorage.setItem('mockWorkspaceConversations', JSON.stringify(conversations));
    }
    if (deleted) {
      const notes = JSON.parse(localStorage.getItem('mockWorkspaceNotes') ?? '{}');
      delete notes[deleted];
      localStorage.setItem('mockWorkspaceNotes', JSON.stringify(notes));
    }
  }, change);
  await page.goto('/?mock=1');
  await expect(page.getByRole('region', { name: 'Projects' })).toBeVisible();
  await page.getByRole('button', { name: 'Open Q3 regional operations review', exact: true }).click();
}

const vaultFile = (path: string, additions: number, deletions: number) =>
  ({ path, changeType: 'modify', attribution: 'confirmed', tool: 'Edit', additions, deletions });

test('two changed files with one name say their folders, and run details open a Result in the Preview pane', async ({ page }, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'the side-panel split is desktop-only');
  await seedQ3(page, { files: [
    vaultFile(`${Q3}/outputs/q3-regional-review.md`, 14, 9),
    vaultFile(`${Q3}/outputs/drafts/q3-regional-review.md`, 3, 1),
    vaultFile(`${Q3}/sources/regional-pnl-q3.csv`, 2, 0),
    vaultFile(`${Q3}/outputs/decision-slides.md`, 4, 0),
  ] });
  const list = page.getByRole('list', { name: 'Changed files' });
  const chips = list.getByRole('listitem');
  await expect(chips).toHaveCount(3);
  await expect(chips.nth(0)).toHaveText('q3-regional-review.md (outputs)+14−9');
  await expect(chips.nth(1)).toHaveText('q3-regional-review.md (outputs/drafts)+3−1');
  await expect(chips.nth(2)).toHaveText('+2 more+6−0');
  // Distinct accessible names, one button per listitem.
  await expect(list.getByRole('button', { name: /q3-regional-review\.md\s*\(outputs\)/ })).toHaveCount(1);
  await expect(list.getByRole('button', { name: /q3-regional-review\.md\s*\(outputs\/drafts\)/ })).toHaveCount(1);

  // The reply's More actions tell the two apart too.
  await page.getByRole('group', { name: 'Reply details' }).first().getByRole('button', { name: 'More actions' }).click();
  const menu = page.getByRole('menu');
  await expect(menu.getByRole('menuitem', { name: 'Review q3-regional-review.md (outputs)', exact: true })).toHaveCount(1);
  await expect(menu.getByRole('menuitem', { name: 'Review q3-regional-review.md (outputs/drafts)', exact: true })).toHaveCount(1);
  await page.keyboard.press('Escape');
  await expect(menu).toHaveCount(0);

  // `+2 more` by the keyboard opens run details, where (2026-11-04) every
  // row opens its file in the Preview pane like its chip, with its History
  // beside it; a source's history is that button.
  const more = chips.nth(2).getByRole('button');
  await more.focus();
  await page.keyboard.press('Enter');
  await expect(more).toHaveAttribute('aria-expanded', 'true');
  const receipt = page.locator('.execution-receipt');
  await expect(receipt.getByRole('button', { name: `Open ${Q3}/sources/regional-pnl-q3.csv in History & recovery`, exact: true })).toHaveCount(1);
  await receipt.getByRole('button', { name: `Open ${Q3}/outputs/decision-slides.md`, exact: true }).click();
  await expect(page.getByRole('complementary', { name: 'Preview' }).getByRole('tab', { selected: true })).toHaveText('decision-slides.md');
  expect(await callCount(page, 'set_result_review')).toBe(0);
});

test('a Result deleted since its reply says so, with its history as the way on', async ({ page }, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'the side-panel split is desktop-only');
  await seedQ3(page, { deleted: `${Q3}/outputs/q3-regional-review.md` });
  // No folder holds it any more (2026-11-04: the chip opens the Preview pane,
  // and a file no place holds is said in words, with its history).
  await page.evaluate(() => localStorage.setItem('mockMissingFiles', JSON.stringify(['outputs/q3-regional-review.md'])));
  const chip = page.getByRole('list', { name: 'Changed files' }).getByRole('listitem').filter({ hasText: 'q3-regional-review.md' }).getByRole('button');
  await expect(chip).toHaveCount(1);
  await chip.click();
  const gone = page.locator('.snackbar').filter({ hasText: "Hollow couldn't find q3-regional-review.md" });
  await expect(gone).toBeVisible();
  await expect(page.getByRole('complementary', { name: 'Preview' })).toHaveCount(0);
  await gone.getByRole('button', { name: 'Show its history', exact: true }).click();
  await expect(page.locator('.library-reader .history-recovery')).toBeVisible();
});
