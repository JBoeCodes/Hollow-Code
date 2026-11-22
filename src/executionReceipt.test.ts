import { describe, expect, it } from 'vitest';
import {
  pullRequestLabel,
  canvasesLine,
  compactionsLine,
  effortLine,
  switchLine,
  executionReceiptStateText,
  executionReceiptFromWire,
  isOwnReceiptFile,
  receiptHasRepoEvidence,
  type EvidenceRepoChangesWire,
  formatPhaseDuration,
  phaseTimingRows,
  receiptRecoveryNote,
  unbackedClaimLabel,
  receiptAttentionLabel,
  type EvidenceReceiptWire,
  MAX_RECEIPT_EXCERPT_CHARS,
} from './executionReceipt';
import { citeItemLine, citesLine } from './receiptCites';

const wireReceipt = (over: Partial<EvidenceReceiptWire> = {}): EvidenceReceiptWire => ({
  id: 'receipt-1',
  runId: 'run-1',
  source: 'chat',
  sourceId: 'session-1',
  startedAt: '2026-07-13T12:00:00Z',
  finishedAt: '2026-07-13T12:00:02Z',
  status: 'success',
  verificationState: 'verifiedAfterChange',
  files: [{
    path: 'src/example.ts',
    changeType: 'modified',
    attribution: 'confirmed',
    tool: 'Edit',
  }],
  checks: [{
    command: 'npm test',
    kind: 'test',
    outcome: 'passed',
    sequence: 4,
    excerpt: '11 tests passed',
  }],
  failures: [],
  historyRunId: 'run-1',
  ...over,
});

describe('executionReceiptFromWire', () => {
  it('keeps the exact recorded command, including its stored truncation marker', () => {
    for (const command of ['npm test ' + 'x'.repeat(291), 'x'.repeat(320) + '…']) {
      const receipt = executionReceiptFromWire(wireReceipt({ checks: [
        { command, kind: 'test', outcome: 'failed', sequence: 1, excerpt: null },
      ] }));
      expect(receipt.checks[0].command).toBe(command);
    }
  });
  it.each([
    ['verifiedAfterChange', 'verified'],
    ['failedAfterChange', 'failed'],
    ['staleAfterChange', 'stale'],
    ['notVerified', 'unverified'],
    ['unknownOrder', 'unverified'],
    ['noChanges', 'no_changes'],
    ['futureState', 'unverified'],
  ] as const)('maps backend state %s conservatively to %s', (verificationState, expected) => {
    const receipt = executionReceiptFromWire(wireReceipt({ verificationState }));
    expect(receipt.status).toBe(expected);
  });

  // Live coverage the removed TS classifier's tests used to give (H22).
  it('caps stored check and failure excerpts and the scheduled wake, ending with an ellipsis', () => {
    const long = 'x'.repeat(MAX_RECEIPT_EXCERPT_CHARS + 200);
    const receipt = executionReceiptFromWire(wireReceipt({
      checks: [{ command: 'npm test', kind: 'test', outcome: 'failed', sequence: 4, excerpt: long }],
      failures: [{ label: 'Bash', sequence: 5, excerpt: long }],
      scheduledWake: long,
    }));
    for (const text of [receipt.checks[0].excerpt, receipt.failures[0].excerpt, receipt.scheduledWake]) {
      expect(text).toHaveLength(MAX_RECEIPT_EXCERPT_CHARS);
      expect(text?.endsWith('…')).toBe(true);
    }
    const short = executionReceiptFromWire(wireReceipt());
    expect(short.checks[0].excerpt).toBe('11 tests passed');
  });

  it.each([
    ['verifiedAfterChange', 'The final observed changes were followed by successful observed checks.'],
    ['staleAfterChange', 'Checks ran before the final observed change, but not afterward.'],
    ['unknownOrder', 'Changes and checks were observed, but their order was unavailable; verification cannot be established.'],
  ] as const)('a %s receipt with attributed files says so plainly', (verificationState, text) => {
    expect(executionReceiptStateText(executionReceiptFromWire(wireReceipt({ verificationState })))).toBe(text);
  });

  it('preserves durable paths, checks, failures, and backend metadata', () => {
    const receipt = executionReceiptFromWire(wireReceipt({
      files: [{
        path: 'vault/tasks/today.md',
        changeType: 'created',
        attribution: 'observed',
        tool: null,
      }],
      verificationState: 'unknownOrder',
      checks: [{
        command: 'custom verify',
        kind: 'future-check-kind',
        outcome: 'future-outcome',
        sequence: 9,
        excerpt: 'output',
      }],
      failures: [{ label: 'Bash', sequence: 10, excerpt: 'permission denied' }],
    }));

    expect(receipt).toMatchObject({
      id: 'receipt-1',
      runId: 'run-1',
      source: 'chat',
      sourceId: 'session-1',
      historyRunId: 'run-1',
      ordering: 'unknown',
      unattributedFileCount: 1,
    });
    expect(receipt.files[0]).toMatchObject({
      path: 'vault/tasks/today.md',
      changeType: 'created',
      attribution: 'observed',
      tool: 'filesystem observation',
    });
    expect(receipt.checks[0]).toMatchObject({ kind: 'check', outcome: 'unknown' });
    expect(receipt.failures[0]).toMatchObject({ tool: 'Bash', excerpt: 'permission denied' });
    expect(executionReceiptStateText(receipt)).toContain('filesystem history');
  });

  it('labels another chat\'s file plainly and never counts it as this run\'s (chat concurrency F1)', () => {
    const receipt = executionReceiptFromWire(wireReceipt({
      verificationState: 'noChanges',
      files: [{
        path: 'workspaces/w1/report.md',
        changeType: 'modify',
        attribution: 'shared',
        tool: null,
      }],
    }));
    expect(receipt.files[0]).toMatchObject({ attribution: 'shared', tool: 'changed by another chat' });
    expect(receipt.unattributedFileCount).toBe(0);
    // Not this run's: chips and the change count leave it out.
    expect(receipt.files.filter(isOwnReceiptFile)).toEqual([]);
    expect(isOwnReceiptFile({ attribution: 'observed' })).toBe(true);
  });

  it('passes the result kind through, and omits it when the receipt has none (Paperclip PR 14a)', () => {
    const quota = executionReceiptFromWire(wireReceipt({
      status: 'error',
      resultKind: 'providerQuota',
      failures: [{ label: 'result', sequence: 5, excerpt: 'Usage limit reached.' }],
    }));
    expect(quota.resultKind).toBe('providerQuota');
    expect(quota.failures[0]).toMatchObject({ tool: 'result', excerpt: 'Usage limit reached.' });
    expect('resultKind' in executionReceiptFromWire(wireReceipt({}))).toBe(false);
    expect('resultKind' in executionReceiptFromWire(wireReceipt({ resultKind: null }))).toBe(false);
  });

  it('lists first stdout and first text apart from any first line (agent effectiveness F11)', () => {
    const rows = phaseTimingRows({ spawnMs: 900, firstOutputMs: 40, firstStdoutMs: 300, firstTextMs: 1_500, turnMs: 9_000 });
    expect(rows.map((r) => r.label)).toEqual(['Preparation', 'First output (any line)', 'First program output', 'First text', 'Agent turn']);
    expect(rows[3].value).toBe('1.5s');
  });

  it('lists when the model started before the first text and never the tool count as a time (OB08a)', () => {
    const rows = phaseTimingRows({ firstStdoutMs: 470, firstModelMs: 2_200, firstTextMs: 17_800, toolsBeforeText: 6 });
    expect(rows.map((r) => r.label)).toEqual(['First program output', 'Model started', 'First text']);
    expect(rows[1].value).toBe('2.2s');
  });

  it('phase timings survive the wire and format (Paperclip PR 15c)', () => {
    const timed = executionReceiptFromWire(wireReceipt({
      phaseTimings: { queueWaitMs: 12, spawnMs: 2_300, turnMs: 125_000, evidenceMs: 3 },
    }));
    expect(timed.phaseTimings).toEqual({ queueWaitMs: 12, spawnMs: 2_300, turnMs: 125_000, evidenceMs: 3 });
    expect(phaseTimingRows(timed.phaseTimings)).toEqual([
      { key: 'queueWaitMs', label: 'Queue wait', value: '12ms' },
      { key: 'spawnMs', label: 'Preparation', value: '2.3s' },
      { key: 'turnMs', label: 'Agent turn', value: '2m 5s' },
      { key: 'evidenceMs', label: 'Receipt', value: '3ms' },
    ]);
    expect('phaseTimings' in executionReceiptFromWire(wireReceipt({}))).toBe(false);
    expect('phaseTimings' in executionReceiptFromWire(wireReceipt({ phaseTimings: null }))).toBe(false);
    expect(phaseTimingRows(undefined)).toEqual([]);
    expect(phaseTimingRows({})).toEqual([]);

    expect(formatPhaseDuration(0)).toBe('0ms');
    expect(formatPhaseDuration(999)).toBe('999ms');
    expect(formatPhaseDuration(5_000)).toBe('5s');
    expect(formatPhaseDuration(59_999)).toBe('59.9s');
    expect(formatPhaseDuration(60_000)).toBe('1m 0s');
    expect(formatPhaseDuration(3_840_000)).toBe('1h 4m');
  });

  it('shows a failed run even when the backend reports no file changes', () => {
    const receipt = executionReceiptFromWire(wireReceipt({
      status: 'error',
      verificationState: 'noChanges',
      files: [],
      checks: [],
      failures: [{ label: 'run', sequence: 1, excerpt: 'Run finished with status error.' }],
    }));

    expect(receipt.status).toBe('failed');
    expect(executionReceiptStateText(receipt)).toContain('no completed file changes');
  });

  it('attaches recall only when the runStarted event has the exact run id', () => {
    const recall = [{
      path: 'learning/team.md',
      targetPath: '*',
      subject: 'communication',
      statement: 'Prefer direct status updates.',
      score: 8,
    }];

    expect(executionReceiptFromWire(wireReceipt(), { runId: 'run-1', recall }).recall).toEqual(recall);
    expect(executionReceiptFromWire(wireReceipt(), { runId: 'different-run', recall }).recall).toBeUndefined();
  });

  it('keeps a recalled learning\'s lifecycle label (F7)', () => {
    const recall = [
      {
        path: 'learnings/cadence-new.md', targetPath: 'projects/Kestrel.md', subject: 'reporting',
        statement: 'Every second Tuesday.', score: 10, lifecycle: 'current: supersedes learnings/cadence-old.md',
      },
      {
        path: 'learnings/cadence-old.md', targetPath: 'projects/Kestrel.md', subject: 'reporting',
        statement: 'Every Friday.', score: 20,
        lifecycle: 'historical: superseded by learnings/cadence-new.md (current since 2026-09-15)',
      },
    ];
    const kept = executionReceiptFromWire(wireReceipt(), { runId: 'run-1', recall }).recall;
    expect(kept?.map(item => item.lifecycle)).toEqual(recall.map(item => item.lifecycle));
  });
});

const repo = (over: Partial<EvidenceRepoChangesWire> = {}): EvidenceRepoChangesWire => ({
  root: '/work/app',
  label: 'app',
  target: 'linkedProject',
  status: 'captured',
  headBefore: 'a'.repeat(40),
  headAfter: 'b'.repeat(40),
  files: [
    {
      path: 'src/lib.rs', changeType: 'modify', status: ' M', untracked: false, gitlink: false,
      additions: 3, deletions: 1, binary: false, oversized: false, patchTruncated: false,
      withheld: false, dirtyBefore: false, attribution: 'observed', tool: null,
    },
    {
      path: 'notes.txt', changeType: 'create', status: '??', untracked: true, gitlink: false,
      additions: 1, deletions: 0, binary: false, oversized: false, patchTruncated: false,
      withheld: false, dirtyBefore: false, attribution: 'confirmed', tool: 'Write',
    },
  ],
  totalFiles: 2,
  truncated: false,
  ...over,
});

describe('executionReceiptFromWire: repo-only receipts are never no_changes', () => {
  it('maps a repo-only noChanges receipt to unverified and counts its rows', () => {
    const wire = wireReceipt({ verificationState: 'noChanges', files: [], checks: [], repoChanges: [repo()] });
    expect(receiptHasRepoEvidence(wire)).toBe(true);
    const receipt = executionReceiptFromWire(wire);
    expect(receipt.status).toBe('unverified');
    expect(receipt.repoFileCount).toBe(2);
    expect(receipt.unattributedFileCount).toBe(1);
    expect(receipt.repoChanges?.[0].files.map(file => file.path)).toEqual(['src/lib.rs', 'notes.txt']);
  });

  it('gives an unrecorded repository its own wording', () => {
    const unavailable = repo({ status: 'unavailable', error: 'git timed out', files: [], totalFiles: 0 });
    const receipt = executionReceiptFromWire(wireReceipt({
      verificationState: 'noChanges', files: [], checks: [], repoChanges: [unavailable],
    }));
    expect(receipt.status).toBe('unverified');
    expect(executionReceiptStateText(receipt)).toContain('Repository changes');
  });

  it('keeps a legacy receipt without repository changes as no_changes', () => {
    const receipt = executionReceiptFromWire(wireReceipt({ verificationState: 'noChanges', files: [], checks: [] }));
    expect(receipt.status).toBe('no_changes');
    expect(receipt.repoChanges).toBeUndefined();
    expect(receipt.repoFileCount).toBeUndefined();
    const empty = executionReceiptFromWire(wireReceipt({
      verificationState: 'noChanges', files: [], checks: [], repoChanges: [repo({ files: [], totalFiles: 0 })],
    }));
    expect(empty.status).toBe('no_changes');
  });

  it('does not call a failed repo-changing run changeless', () => {
    const receipt = executionReceiptFromWire(wireReceipt({
      status: 'error', verificationState: 'unknownOrder', files: [], checks: [], repoChanges: [repo()],
      failures: [{ label: 'run', sequence: 1, excerpt: 'Run finished with status error.' }],
    }));
    expect(receipt.status).toBe('failed');
    expect(executionReceiptStateText(receipt)).not.toContain('no completed file changes');
  });
});

describe('requested → sent effort (merged plan Track U)', () => {
  it('reads the receipt effort and words the mapping', () => {
    const r = executionReceiptFromWire(wireReceipt({ effort: { requested: 'ultracode', sent: 'xhigh', note: 'no ultracode here' } }));
    expect(r.effort).toEqual({ requested: 'ultracode', sent: 'xhigh', note: 'no ultracode here' });
    expect(effortLine(r.effort!)).toBe('ultracode → xhigh');
    expect(effortLine({ requested: 'high', sent: 'high' })).toBe('high');
    expect(effortLine({ requested: 'max', sent: null })).toBe('max → none');
    expect(executionReceiptFromWire(wireReceipt()).effort).toBeUndefined();
  });
});

describe('moves at a ceiling (merged plan decision 6)', () => {
  it('keeps the actual account choice and rejection evidence in attempt order', () => {
    const accountChoices = ['Admin: 96% headroom; most room', 'Admin: quota rejection contradicts the usage meter', 'ferumbas: 86% headroom; most room'];
    expect(executionReceiptFromWire(wireReceipt({ accountChoices })).accountChoices).toEqual(accountChoices);
    expect(executionReceiptFromWire(wireReceipt()).accountChoices).toBeUndefined();
  });
  it('keeps the receipt switches in order and words each one', () => {
    const switches = [
      { kind: 'agent' as const, from: 'Claude', to: 'Codex', reason: 'Budget paused: Claude usage reached its daily token limit.' },
      { kind: 'account' as const, from: 'Admin', to: 'Tech', reason: 'Admin hit its limit' },
    ];
    const r = executionReceiptFromWire(wireReceipt({ switches }));
    expect(r.switches).toEqual(switches);
    expect(switchLine(r.switches![0])).toBe('Claude → Codex');
    expect(executionReceiptFromWire(wireReceipt()).switches).toBeUndefined();
    expect(executionReceiptFromWire(wireReceipt({ switches: [] })).switches).toBeUndefined();
  });
});

describe('recoverability on receipts (merged plan slice 4, decision 5)', () => {
  const file = (path: string, notRecoverable?: boolean) => ({
    path, changeType: 'modify', attribution: 'observed', tool: null, ...(notRecoverable ? { notRecoverable } : {}),
  });
  it('says a run is not fully recoverable when one file is not', () => {
    const receipt = executionReceiptFromWire(wireReceipt({
      files: [file('notes/a.md'), file('notes/big.bin', true)],
      historyRecoverable: false,
    }));
    expect(receipt.historyRecoverable).toBe(false);
    expect(receipt.files.map(f => !!f.notRecoverable)).toEqual([false, true]);
    expect(receiptRecoveryNote(receipt)).toMatch(/^Not fully recoverable: .*one changed file/);
  });
  it('claims nothing when every file is recoverable or nothing is known', () => {
    expect(receiptRecoveryNote(executionReceiptFromWire(wireReceipt({ files: [file('a.md')], historyRecoverable: true })))).toBeNull();
    expect(receiptRecoveryNote(executionReceiptFromWire(wireReceipt({ files: [file('a.md')] })))).toBeNull();
    expect(receiptRecoveryNote({ files: [], historyRecoverable: false })).toMatch(/Not fully recoverable/);
  });
});

describe('canvas count and bytes per reply (Canvas C3 wave 2 C4)', () => {
  it('reads the receipt canvases and words their cost', () => {
    const r = executionReceiptFromWire(wireReceipt({ canvases: { count: 2, bytes: 14_540 } }));
    expect(r.canvases).toEqual({ count: 2, bytes: 14_540 });
    expect(canvasesLine(r.canvases)).toBe('2 canvases · 14 KB');
    expect(canvasesLine({ count: 1, bytes: 2_458 })).toBe('1 canvas · 2.4 KB');
    expect(canvasesLine({ count: 1, bytes: 300 })).toBe('1 canvas · 300 B');
    expect(canvasesLine({ count: 0, bytes: 0 })).toBeNull();
    expect(canvasesLine(undefined)).toBeNull();
    expect(canvasesLine(null)).toBeNull();
  });

  it('leaves canvases out when the reply drew none, and on older receipts', () => {
    expect(executionReceiptFromWire(wireReceipt()).canvases).toBeUndefined();
    expect(executionReceiptFromWire(wireReceipt({ canvases: null })).canvases).toBeUndefined();
    expect(executionReceiptFromWire(wireReceipt({ canvases: { count: 0, bytes: 0 } })).canvases).toBeUndefined();
  });
});

describe('the Cites row (repo features R5)', () => {
  const cites = {
    checked: 9,
    seen: 6,
    fileOnly: 1,
    unknown: 1,
    pastEnd: 1,
    items: [
      { path: 'src/a.rs', line: 12, end: 20, status: 'fileOnly' },
      { path: 'src/b.rs', line: 3, status: 'unknown' },
      { path: 'docs/c.md', line: 400, status: 'pastEnd' },
    ],
  };

  it('maps the wire cites and words every part that is not zero', () => {
    const r = executionReceiptFromWire(wireReceipt({ cites }));
    expect(r.cites).toEqual(cites);
    expect(citesLine(r.cites)).toBe(
      '9 checked · 6 seen · 1 outside the lines read · 1 not seen here (a subagent or teammate may have read it) · 1 past end of file',
    );
    expect(citesLine({ checked: 2, notOpened: 1, missing: 1, earlier: 0 })).toBe('2 checked · 1 not opened · 1 no such file');
    expect(r.cites?.items?.map(citeItemLine)).toEqual([
      'src/a.rs:12-20 · outside the lines read',
      'src/b.rs:3 · not seen here (a subagent or teammate may have read it)',
      'docs/c.md:400 · past end of file',
    ]);
  });

  it('shows no row when nothing was checked, and on older receipts', () => {
    expect(executionReceiptFromWire(wireReceipt()).cites).toBeUndefined();
    expect(executionReceiptFromWire(wireReceipt({ cites: null })).cites).toBeUndefined();
    expect(executionReceiptFromWire(wireReceipt({ cites: { checked: 0 } })).cites).toBeUndefined();
    expect(citesLine(undefined)).toBeNull();
    expect(citesLine({ checked: 0 })).toBeNull();
  });
});

describe('compactions on the receipt (repo features R1)', () => {
  it('passes the count through and words it; none, 0 and older receipts leave it out', () => {
    const r = executionReceiptFromWire(wireReceipt({ compactions: 2 }));
    expect(r.compactions).toBe(2);
    expect(compactionsLine(r.compactions)).toBe('2 times');
    expect(compactionsLine(1)).toBe('1 time');
    expect(executionReceiptFromWire(wireReceipt()).compactions).toBeUndefined();
    expect(executionReceiptFromWire(wireReceipt({ compactions: 0 })).compactions).toBeUndefined();
    expect(compactionsLine(undefined)).toBeNull();
    expect(compactionsLine(0)).toBeNull();
  });
});

describe('a scheduled wake-up on the receipt (overnight C3)', () => {
  it('carries the line when present and nothing otherwise', () => {
    const r = executionReceiptFromWire(wireReceipt({ scheduledWake: 'Hollow wakes this chat at 03:10: re-run the gate' }));
    expect(r.scheduledWake).toBe('Hollow wakes this chat at 03:10: re-run the gate');
    expect(executionReceiptFromWire(wireReceipt({})).scheduledWake).toBeUndefined();
    expect(executionReceiptFromWire(wireReceipt({ scheduledWake: '   ' })).scheduledWake).toBeUndefined();
    expect(executionReceiptFromWire(wireReceipt({ scheduledWake: null })).scheduledWake).toBeUndefined();
  });

  it('carries the pull requests a turn merged or opened, linking only to GitHub (overnight F8)', () => {
    const r = executionReceiptFromWire(wireReceipt({ pullRequests: [
      { number: 453, repo: 'example-org/hollow-code', action: 'merged', url: 'https://github.com/example-org/hollow-code/pull/453' },
      { number: 12, repo: null, action: 'opened', url: 'javascript:alert(1)' },
    ] }));
    expect(r.pullRequests?.map(pullRequestLabel)).toEqual(['Merged #453 in example-org/hollow-code', 'Opened #12']);
    expect(r.pullRequests?.[0].url).toBe('https://github.com/example-org/hollow-code/pull/453');
    expect(r.pullRequests?.[1].url, 'never a non-GitHub link').toBeNull();
    expect(executionReceiptFromWire(wireReceipt({})).pullRequests).toBeUndefined();
    expect(executionReceiptFromWire(wireReceipt({ pullRequests: [] })).pullRequests).toBeUndefined();
  });
});

describe('an unbacked pass claim on the verification row (overnight A8)', () => {
  it('chips a claim with no check after the last edit, or after a failed check', () => {
    const noCheck = executionReceiptFromWire(wireReceipt({ verification: { claim: 'passed', claimWithoutCheck: true } }));
    expect(noCheck.unbackedClaim).toBe('noCheck');
    expect(unbackedClaimLabel(noCheck.unbackedClaim)).toBe('Says it passes · no check after its last edit');
    const afterFail = executionReceiptFromWire(wireReceipt({ verification: { claim: 'passed', claimAfterFail: true } }));
    expect(unbackedClaimLabel(afterFail.unbackedClaim)).toBe('Says it passes · its last check failed');
    const both = executionReceiptFromWire(wireReceipt({ verification: { claim: 'passed', claimWithoutCheck: true, claimAfterFail: true } }));
    expect(both.unbackedClaim, 'a failed check is the stronger fact').toBe('afterFail');
  });

  it('shows nothing for a backed claim, no claim or an older receipt', () => {
    for (const verification of [undefined, null, { claim: 'passed' }, { claim: 'none' }]) {
      const r = executionReceiptFromWire(wireReceipt({ verification }));
      expect(r.unbackedClaim).toBeUndefined();
      expect(unbackedClaimLabel(r.unbackedClaim)).toBeNull();
    }
  });
});

describe("the reply footer's check chip (P0-23)", () => {
  const label = (over: Partial<EvidenceReceiptWire>) => receiptAttentionLabel(executionReceiptFromWire(wireReceipt(over)));

  it('a verified receipt, no changes or no receipt gets no chip', () => {
    expect(label({})).toBeNull();
    expect(label({ verificationState: 'noChanges', files: [], checks: [] })).toBeNull();
    expect(receiptAttentionLabel(undefined)).toBeNull();
    // Cites that resolve (seen, or only outside the lines read) aren't a problem.
    expect(label({ cites: { checked: 3, seen: 2, fileOnly: 1, items: [] } })).toBeNull();
  });

  it('an unbacked pass claim wins, in the Run record\'s own words', () => {
    expect(label({ verificationState: 'staleAfterChange', verification: { claim: 'passed', claimWithoutCheck: true } }))
      .toBe('Says it passes · no check after its last edit');
    expect(label({ verification: { claim: 'passed', claimAfterFail: true } })).toBe('Says it passes · its last check failed');
  });

  it('stale checks say so, and an unverified change with no claim stays quiet', () => {
    expect(label({ verificationState: 'staleAfterChange' })).toBe('Checks out of date');
    expect(label({ verificationState: 'notVerified' })).toBeNull();
    expect(label({ verificationState: 'unknownOrder' })).toBeNull();
  });

  it('says Not verified only for an unconfirmed done or fixed claim', () => {
    expect(label({ verificationState: 'notVerified', verification: { claim: 'done', claimWithoutCheck: true } })).toBe('Not verified');
    expect(label({ verificationState: 'failedAfterChange', verification: { claim: 'fixed', claimAfterFail: true } })).toBe('Not verified');
    expect(label({ verificationState: 'verifiedAfterChange', verification: { claim: 'done' } })).toBeNull();
    expect(label({ verificationState: 'notVerified', verification: { claim: 'none' } })).toBeNull();
  });

  it("counts the cites that don't resolve: past the end, no such file, unresolved name", () => {
    expect(label({ cites: { checked: 4, seen: 2, pastEnd: 1, missing: 1, items: [] } })).toBe("2 cites don't resolve");
    expect(label({ cites: { checked: 2, seen: 1, unresolved: 1, items: [] } })).toBe("1 cite doesn't resolve");
  });
});
