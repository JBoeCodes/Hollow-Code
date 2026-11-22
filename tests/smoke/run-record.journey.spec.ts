import { expect, test } from './fixtures/test';
import { showWorkspaceList } from './choose';

// The workspace's Activity timeline (and its retry's contrast check) went with
// the workspace panel in the COO rebuild L4 (2026-11-10): the record's hosts
// are the conversation and Automations.
for (const theme of ['light', 'dark']) for (const host of ['Chat', 'Automations']) {
  test(`${host} failed record uses the failure color in ${theme}`, async ({ page }) => {
    await page.addInitScript(theme => localStorage.setItem('theme', theme), theme);
    await seedRecord(page);
    const record = await recordHost(page, host);
    const expected = await record.evaluate(element => {
      const probe = document.createElement('span');
      probe.style.color = 'var(--error)';
      element.append(probe);
      const color = getComputedStyle(probe).color;
      probe.remove();
      return color;
    });
    const status = record.locator('.execution-receipt-summary > .toolcall-status');
    await expect(status).toHaveText('Failed');
    await expect(status).toHaveCSS('color', expected);
  });
}

test('the automation record retains only recall from its matching live run', async ({ page }) => {
  await seedRecord(page);
  await recordHost(page, 'Automations');
  const record = page.locator('.workflow-review > .run-record');
  const finish = async (runId: string) => {
    await page.evaluate(({ wire, runId }) => {
      const emit = (window as unknown as { __mockEmit: (name: string, payload: unknown) => number }).__mockEmit;
      emit('workflow-event', {
        type: 'started', slug: 'morning-brief', runId, queueId: 21,
        at: '2026-10-25T12:00:00Z', source: 'manual', reason: 'run now',
        recall: [{ path: 'learnings/status.md', targetPath: '*', subject: 'Status', statement: 'Keep status updates direct.', score: 8 }],
      });
      emit('workflow-event', {
        type: 'done', slug: 'morning-brief', runId, queueId: 21,
        status: 'success', output: '', durationSec: 1, error: null,
        nextRunAt: null, evidenceError: null,
        // Deliberately keep the newest log's wire: a different live context
        // must not supply recall to it, regardless of the done payload.
        receipt: { ...wire, runId: 'run-workflow-1', historyRunId: 'run-workflow-1', source: 'workflow', sourceId: 'morning-brief' },
      });
    }, { wire: sharedWire, runId });
    await expect(record).toBeVisible();
    const disclosure = record.locator('.execution-receipt');
    if (await disclosure.getAttribute('open') === null) await disclosure.locator(':scope > summary').click();
  };
  await finish('run-workflow-1');
  await expect(record.getByRole('list', { name: 'Recall used' })).toContainText('Keep status updates direct.');
  await finish('different-run');
  await expect(record.getByRole('list', { name: 'Recall used' })).toHaveCount(0);
});

test('the newest automation without evidence never borrows an older run record', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockRunRecordNoLatest', '1'));
  await page.goto('/?mock=1');
  await recordHost(page, 'Automations');
  const review = page.locator('.workflow-review');
  await expect(review.locator(':scope > .workflow-outcome')).toContainText('Hollow has no record of this run');
  await expect(review.locator(':scope > .run-record')).toHaveCount(0);
  await expect(review.locator('.workflow-history-row .execution-receipt')).toHaveCount(1);
});

test('historical Chat records have unique headings and only the latest failed message can retry', async ({ page }) => {
  await seedRecord(page);
  await page.evaluate(() => {
    const conversations = JSON.parse(localStorage.getItem('mockWorkspaceConversations') ?? '{}');
    const c = conversations['s-20260930-q3a00001'];
    const previous = c.turns[1];
    c.turns.push({ ...c.turns[0], id: 'second-user', text: 'A later request' }, {
      ...previous, id: 'second-assistant', runId: 'run-r1b', text: 'The later request failed.',
      receipt: { ...previous.receipt, id: 'rcpt-r1b', runId: 'run-r1b', historyRunId: 'run-r1b' },
    });
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify(conversations));
  });
  await page.goto('/?mock=1');
  await recordHost(page, 'Chat');
  const records = page.locator('.chat-main .run-record');
  await expect(records).toHaveCount(2);
  await expect(records.first().getByRole('button', { name: 'Retry', exact: true })).toHaveCount(0);
  await expect(records.last().getByRole('button', { name: 'Retry', exact: true })).toBeVisible();
  const headings = await records.locator('.workflow-outcome h3').evaluateAll(nodes => nodes.map(n => n.id));
  expect(headings).toHaveLength(2);
  expect(new Set(headings).size).toBe(2);
});

test('the automation run record contains one What happened head and copyable checks', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Automations', exact: true }).click();
  await page.locator('.workflow-row').filter({ hasText: 'Morning Brief' }).getByRole('button', { name: 'Open Morning Brief' }).click();
  const review = page.locator('.workflow-review');
  const record = review.locator('.run-record').first();
  await expect(record.getByRole('heading', { name: 'What happened' })).toBeVisible();
  await record.locator('.execution-receipt-summary').click();
  await expect(record.getByRole('button', { name: 'Copy command', exact: true }).first()).toBeVisible();
  await expect(record.getByRole('button', { name: 'Preview src/lib.rs', exact: true })).toBeVisible();
  await expect(review.locator(':scope > .execution-receipt')).toHaveCount(0);
});

const sharedWire = {
  id: 'rcpt-r1a', runId: 'run-r1a', source: 'chat', sourceId: 's-20260930-q3a00001',
  startedAt: '2026-10-25T10:00:00Z', finishedAt: '2026-10-25T10:01:00Z',
  status: 'error', verificationState: 'failedAfterChange', historyRunId: 'run-r1a',
  files: [{ path: 'workspaces/local-q3-review/outputs/q3-regional-review.md', changeType: 'modify', attribution: 'confirmed', tool: 'Edit' }],
  checks: [{ command: 'npm test ' + 'x'.repeat(311) + '…', kind: 'test', outcome: 'failed', sequence: 4, excerpt: 'The shared fixture failed its check.' }],
  failures: [{ label: 'Bash', sequence: 4, excerpt: 'The shared fixture failed its check.' }],
  outsideEffects: { recorded: [], unknown: [{ kind: 'shell', label: 'Shell commands', count: 1, examples: ['npm test'] }] },
  cites: { checked: 3, notOpened: 3, items: [
    { path: 'notes/same.md', target: '/recorded/repo/notes/same.md', line: 42, end: 44, status: 'notOpened' },
    { path: 'notes/same.md', target: '/recorded/vault/notes/same.md', line: 17, status: 'notOpened' },
    { path: 'legacy.md', line: 9, status: 'unresolved' },
  ] },
};

async function seedRecord(page: import('./fixtures/test').Page) {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'clipboard', { value: { writeText: async (text: string) => { (window as unknown as { recordCopied: string }).recordCopied = text; } }, configurable: true });
  });
  await page.goto('/?mock=1&demo=1');
  await expect(page.locator('.home-project-row').first()).toBeVisible();
  await page.evaluate(wire => {
    localStorage.setItem('mockRunRecord', JSON.stringify(wire));
    const conversations = JSON.parse(localStorage.getItem('mockWorkspaceConversations') ?? '{}');
    const c = conversations['s-20260930-q3a00001'];
    c.turns = c.turns.slice(0, 2);
    Object.assign(c.turns[1], { receipt: wire, runId: wire.runId, status: 'error', error: 'Check failed', text: 'The run failed its check. [Another file](/recorded/repo/normal.md)' });
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify(conversations));
    const text = Array.from({ length: 100 }, (_, i) => 'Recorded line ' + (i + 1)).join('\n');
    localStorage.setItem('mockPreviewFiles', JSON.stringify({
      '/recorded/repo/notes/same.md': { text, delay: 800 },
      '/recorded/vault/notes/same.md': { text, delay: 30 },
      '/recorded/repo/normal.md': { text: '# Regular preview', delay: 500 },
    }));
  }, sharedWire);
  await page.goto('/?mock=1');
}

async function recordHost(page: import('./fixtures/test').Page, host: string) {
  if (host === 'Automations') {
    await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Automations', exact: true }).click();
    await page.locator('.workflow-row').filter({ hasText: 'Morning Brief' }).getByRole('button', { name: 'Open Morning Brief' }).click();
    return page.locator('.workflow-review > .run-record').first();
  }
  await showWorkspaceList(page);
  await page.locator('.work-card').filter({ hasText: 'Q3 regional operations review' }).click();
  await expect(page.locator('.top-bar-crumbs')).toContainText('Q3 regional operations review');
  // The conversation whose run the record is (its Activity row opened it
  // until the COO rebuild L4).
  await page.evaluate(async () => {
    const path = '/src/store/workspaces.ts';
    await (await import(/* @vite-ignore */ path)).openSessionByKey('s-20260930-q3a00001');
  });
  return page.locator('.chat-main .run-record').first();
}

for (const host of ['Chat', 'Automations']) {
  test(host + ' renders the common record with exact command copy and full failure detail', async ({ page }) => {
    await seedRecord(page);
    const record = await recordHost(page, host);
    await expect(record.getByRole('heading', { name: 'What happened' })).toBeVisible();
    await expect(record.locator('.workflow-outcome')).toContainText('1 vault file');
    const labels = await record.locator('.workflow-outcome dt').allTextContents();
    expect(labels.filter(label => label !== 'Cost')).toEqual(['Changed', 'Checks', 'Outside its files', 'Not visible to Hollow']);
    const disclosure = record.locator('.execution-receipt');
    if (await disclosure.getAttribute('open') === null) await disclosure.locator(':scope > summary').click();
    await expect(record.locator('pre').filter({ hasText: 'The shared fixture failed its check.' }).first()).toBeVisible();
    await expect(record.locator('summary').filter({ hasText: /^Timeline$/ })).toBeVisible();
    await record.getByRole('button', { name: 'Copy recorded command (cut short)' }).click();
    expect(await page.evaluate(() => (window as unknown as { recordCopied: string }).recordCopied)).toBe(sharedWire.checks[0].command);
    await expect(record.locator('.execution-receipt-path code')).toHaveCount(1);
    if (host === 'Chat') {
      const input = page.getByRole('textbox', { name: 'Chat message' });
      await input.fill('Keep my existing draft');
      const sends = await page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0);
      await record.getByRole('button', { name: 'Ask to rerun', exact: true }).click();
      await expect(input).toHaveValue(new RegExp('^Keep my existing draft\\n\\nReview and rerun this recorded check'));
      expect(await input.inputValue()).toContain(sharedWire.checks[0].command);
      expect(await page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBe(sends);
      await expect(page.getByRole('button', { name: 'Retry', exact: true })).toHaveCount(1);
      await expect(record.getByRole('button', { name: 'Retry', exact: true })).toBeVisible();
    } else {
      await expect(record.getByRole('button', { name: 'Ask to rerun', exact: true })).toHaveCount(0);
      if (host === 'Automations') {
        await record.getByRole('button', { name: `Open ${sharedWire.files[0].path} in History & recovery`, exact: true }).click();
        await expect(page.locator('.history-recovery').first()).toBeVisible();
      }
    }
  });
}

test('recorded cite targets preserve line ranges and the newer file wins a delayed load', async ({ page }) => {
  await seedRecord(page);
  await recordHost(page, 'Chat');
  const record = page.locator('.chat-main .execution-receipt').first();
  await record.locator('.execution-receipt-summary').click();
  await record.locator('.execution-receipt-cites summary').click();
  await expect(record.getByRole('button', { name: /legacy.md/ })).toHaveCount(0);
  const a = record.getByRole('button', { name: 'notes/same.md:42-44 · not opened', exact: true });
  const b = record.getByRole('button', { name: 'notes/same.md:17 · not opened', exact: true });
  const returnToRecord = async () => {
    // At the native minimum the Preview pane takes the conversation's place
    // (run 15 P1) and × brings it back. It mounts lazily: wait for it first.
    const pane = page.getByRole('complementary', { name: 'Preview', exact: true });
    await expect(pane).toBeVisible();
    if (!(await record.isVisible())) await pane.getByRole('button', { name: 'Close preview', exact: true }).click();
    if (await record.getAttribute('open') === null) await record.locator('.execution-receipt-summary').click();
    const cites = record.locator('.execution-receipt-cites');
    if (await cites.getAttribute('open') === null) await cites.locator('summary').click();
  };
  await a.click();
  await returnToRecord();
  await b.click();
  const mark = page.locator('.receipt-source-target');
  await expect(mark).toHaveAttribute('data-line', '17');
  await expect(mark).toBeInViewport();
  await page.waitForTimeout(1000);
  await expect(mark).toHaveAttribute('data-line', '17');
  await expect(page.getByRole('complementary', { name: 'Preview' }).getByRole('tab', { selected: true })).toHaveAttribute('title', /recorded\/vault\/notes\/same.md/);
  await returnToRecord();
  await a.click();
  await expect(mark).toHaveAttribute('data-line', '42');
  await expect(mark).toHaveAttribute('data-end', '44');
  await expect(mark).toHaveText('Recorded line 42\nRecorded line 43\nRecorded line 44');
  await expect(mark).toBeInViewport();
  await returnToRecord();
  await page.locator('.chat-main').getByRole('link', { name: 'Open Another file, /recorded/repo/normal.md', exact: true }).click();
  await expect(page.locator('.chat-preview-loading')).toBeVisible();
  await expect(page.locator('.chat-preview').getByRole('heading', { name: 'Regular preview' })).toBeVisible();
  await expect(page.locator('.chat-preview-toggle').getByRole('button', { name: 'Preview', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(mark).toHaveCount(0);
});

test('a delayed cite cannot restore a highlight after switching workspaces', async ({ page }) => {
  await seedRecord(page);
  await recordHost(page, 'Chat');
  await page.evaluate(() => {
    const files = JSON.parse(localStorage.getItem('mockPreviewFiles') ?? '{}');
    files['/recorded/repo/notes/same.md'].delay = 3000;
    localStorage.setItem('mockPreviewFiles', JSON.stringify(files));
  });
  const record = page.locator('.chat-main .execution-receipt').first();
  await record.locator('.execution-receipt-summary').click();
  await record.locator('.execution-receipt-cites summary').click();
  await record.getByRole('button', { name: 'notes/same.md:42-44 · not opened', exact: true }).click();
  await expect(page.locator('.chat-preview-loading')).toBeVisible();
  // Run 15 P1: the app-wide Preview pane keeps a file and its cited lines
  // across workspaces; × is how a person leaves a cite.
  const pane = page.getByRole('complementary', { name: 'Preview', exact: true });
  await pane.getByRole('button', { name: 'Close preview', exact: true }).click();
  await expect(pane).toHaveCount(0);
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true }).click();
  await showWorkspaceList(page);
  await page.locator('.work-card').filter({ hasNotText: 'Q3 regional operations review' }).first().click();
  await page.waitForTimeout(3200);
  // The late read neither reopens the pane nor marks lines.
  await expect(pane).toHaveCount(0);
  await expect(page.locator('.receipt-source-target')).toHaveCount(0);
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true }).click();
  await recordHost(page, 'Chat');
  const returned = page.locator('.chat-main .execution-receipt').first();
  if (await returned.getAttribute('open') === null) await returned.locator('.execution-receipt-summary').click();
  const cites = returned.locator('.execution-receipt-cites');
  if (await cites.getAttribute('open') === null) await cites.locator('summary').click();
  await cites.getByRole('button', { name: 'notes/same.md:17 · not opened', exact: true }).click();
  await expect(page.locator('.receipt-source-target')).toHaveAttribute('data-line', '17');
  await expect(page.getByRole('complementary', { name: 'Preview' }).getByRole('tab', { selected: true })).toHaveAttribute('title', /recorded\/vault\/notes\/same.md/);
});
