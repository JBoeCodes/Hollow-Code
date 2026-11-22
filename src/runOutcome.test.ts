import { describe, expect, it } from 'vitest';
import { executionReceiptFromWire, type EvidenceReceiptWire } from './executionReceipt';
import { OPS_STATE, automationState, globMatcher, historyStatusLabel, nightRecovery, nightStateHint, runCostSuffix, runOutcome, runOutputs, runbookStateHint } from './runOutcome';

// P1-04: an automation's run history says each run's state in the same words
// as its status pill, not the stored "success" / "error".
describe('historyStatusLabel', () => {
  it('uses the OPS_STATE words and keeps an unknown status as written', () => {
    expect(historyStatusLabel('success')).toBe('Finished');
    expect(historyStatusLabel('error')).toBe(OPS_STATE.failed.label);
    expect(historyStatusLabel('cancelled')).toBe(OPS_STATE.stopped.label);
    expect(historyStatusLabel('paused')).toBe('Paused');
    expect(historyStatusLabel('interrupted')).toBe(OPS_STATE.interrupted.label);
    expect(historyStatusLabel('error', true)).toBe(OPS_STATE.budgetStopped.label);
    expect(historyStatusLabel('skipped')).toBe('Skipped');
  });
});

function receipt(over: Partial<EvidenceReceiptWire> = {}): EvidenceReceiptWire {
  return {
    id: 'r1', runId: 'run-1', source: 'workflow', sourceId: 'brief', startedAt: '', finishedAt: '',
    status: 'error', verificationState: 'noChanges', files: [], checks: [], failures: [], historyRunId: 'run-1',
    outsideEffects: {},
    ...over,
  };
}

const notion = { kind: 'connector', label: 'Notion · create page “Weekly”', tool: 'mcp__notion__notion-create-pages', outcome: 'done', key: 'k1', sequence: 2 };
const push = { kind: 'shell', label: 'git push · git push origin main', tool: 'Bash', outcome: 'done', sequence: 3 };
const shell = { kind: 'shell', label: 'Shell commands', count: 2, examples: ['python3 a.py'] };

it('shares summary facts across normalized Chat/Activity and automation receipts without granting retry', () => {
  const wire = receipt({ files: [{ path: 'outputs/report.md', changeType: 'create', attribution: 'confirmed', tool: 'Write' }],
    checks: [{ command: 'npm test', kind: 'test', outcome: 'failed', sequence: 1, excerpt: null }],
    outsideEffects: { recorded: [notion, push], unknown: [shell] } });
  const automation = runOutcome({ status: 'error', receipt: wire, cost: null });
  const normalized = runOutcome({ status: null, receipt: executionReceiptFromWire(wire), cost: null });
  expect({ ...normalized, retry: automation.retry }).toEqual(automation);
  expect(normalized.retry.kind).toBe('none');
  expect(automation.retry.kind).toBe('guarded');
});

describe('automationState (plan completion L2)', () => {
  it('takes the strongest live signal, then the last run', () => {
    expect(automationState({ running: true, blocked: true, lastStatus: 'error' })).toBe('running');
    expect(automationState({ queued: true, blocked: true })).toBe('queued');
    expect(automationState({ blocked: true, lastStatus: 'success' })).toBe('blocked');
    expect(automationState({ lastStatus: 'error' })).toBe('failed');
    expect(automationState({ lastStatus: 'error', budgetStop: true })).toBe('budgetStopped');
    expect(automationState({ lastStatus: 'success', produced: true })).toBe('readyForReview');
    expect(automationState({ lastStatus: 'success' })).toBe('finished');
    expect(automationState({ lastStatus: null })).toBe('never');
    // A crash or restart mid-run is not a finished run.
    expect(automationState({ lastStatus: 'interrupted' })).toBe('interrupted');
    expect(automationState({ lastStatus: 'paused' })).toBe('paused');
    expect(OPS_STATE.interrupted.label).toBe('Interrupted');
    expect(automationState({ lastStatus: 'skipped' })).toBe('never');
    expect(OPS_STATE.running.label).toBe('Working');
  });

  it('explains queued as nothing having happened yet, and finished as not checked', () => {
    expect(OPS_STATE.queued.hint).toContain('nothing has happened yet');
    expect(OPS_STATE.readyForReview.hint).toContain('not checked or accepted');
    expect(OPS_STATE.preview.hint).toContain('Nothing runs');
  });

  it('gives Tonight and runbook statuses the same explanations', () => {
    expect(nightStateHint('queued')).toBe(OPS_STATE.queued.hint);
    expect(nightStateHint('refused')).toContain('Nothing ran');
    expect(nightStateHint('removed')).toContain('nothing happened');
    expect(runbookStateHint('waiting')).toBe(OPS_STATE.waiting.hint);
    expect(runbookStateHint('interrupted')).toContain('later ones didn’t');
    expect(runbookStateHint(undefined)).toBe(OPS_STATE.never.hint);
  });
});

describe('nightRecovery', () => {
  it('sends a cap stop or refusal to the overnight cap and a sign-in failure to Accounts', () => {
    expect(nightRecovery({ status: 'stopped', note: 'Stopped at the overnight stop ($40.10 of $40.00 spent); what it did so far is kept.' })).toMatchObject({ target: 'settingsBudgets' });
    expect(nightRecovery({ status: 'refused', note: 'Refused: the overnight budget is $0.' })).toMatchObject({ label: 'Change the overnight cap' });
    expect(nightRecovery({ status: 'failed', note: 'Claude: 401 Invalid authentication' })).toMatchObject({ target: 'accounts' });
    expect(nightRecovery({ status: 'failed', note: 'claude exited 1' })).toBeNull();
    // Raising the cap can't fix an item that asked for more than it.
    expect(nightRecovery({ status: 'refused', note: 'Refused: it asks for $50.00, more than the $40.00 overnight cap.' })).toBeNull();
    expect(nightRecovery({ status: 'failed', note: 'catalog index failed' })).toBeNull();
    expect(nightRecovery({ status: 'done', note: 'overnight cap' })).toBeNull();
  });
});

describe('runOutcome (plan completion L2, V03)', () => {
  it('never claims nothing happened without a receipt', () => {
    const o = runOutcome({ status: 'error', receipt: null, authFailure: true });
    expect(o.changedAnything).toBe('unknown');
    expect(o.changed).toContain('Unknown');
    expect(o.coverage).toContain('no record of this run');
    expect(o.retry).toMatchObject({ kind: 'checkFirst', label: null, retryOf: null });
    expect(o.retry.detail.startsWith('Sign in first.')).toBe(true);
  });

  it('offers the guarded retry once something happened outside, naming what it can’t block', () => {
    const o = runOutcome({ status: 'error', receipt: receipt({ outsideEffects: { recorded: [notion, push] } }) });
    expect(o.changedAnything).toBe('some');
    expect(o.effects.map(e => [e.outcome, e.guarded])).toEqual([['done', true], ['done', false]]);
    expect(o.retry).toMatchObject({ kind: 'guarded', label: 'Run again without repeating', retryOf: 'run-1' });
    expect(o.retry.detail).toContain('Hollow refuses the identical call for the one it runs itself');
    expect(o.retry.detail).toContain('check the other one');
  });

  it('treats an uncertain effect as one that may have happened, and a failed one as not', () => {
    const uncertain = runOutcome({ status: 'error', receipt: receipt({ outsideEffects: { recorded: [{ ...notion, outcome: 'uncertain' }] } }) });
    expect(uncertain.effects[0].outcomeLabel).toBe('may have happened');
    expect(uncertain.retry.kind).toBe('guarded');
    const failed = runOutcome({ status: 'error', receipt: receipt({ outsideEffects: { recorded: [{ ...notion, outcome: 'failed' }] } }) });
    expect(failed.retry.kind).toBe('safe');
    expect(failed.changedAnything).toBe('none');
  });

  it('asks for a check when calls are unknown, and passes the run so the retry is told', () => {
    const o = runOutcome({ status: 'error', receipt: receipt({ outsideEffects: { unknown: [shell] } }) });
    expect(o.changedAnything).toBe('unknown');
    expect(o.coverage).toContain('can’t see what 2 calls did');
    expect(o.retry).toMatchObject({ kind: 'checkFirst', retryOf: 'run-1' });
  });

  it('says a retry starts from current files when only files changed', () => {
    const o = runOutcome({ status: 'error', receipt: receipt({ files: [{ path: 'briefs/a.html', changeType: 'create', attribution: 'confirmed', tool: 'Write' }], historyRecoverable: true }) });
    expect(o.changed).toBe('1 vault file (History can undo it)');
    expect(o.retry).toMatchObject({ kind: 'currentFiles', label: null, retryOf: null });
  });

  it('treats a listed PR or an unrecorded repository as never safe', () => {
    const pr = runOutcome({ status: 'error', receipt: receipt({ pullRequests: [{ number: 7, action: 'opened', repo: 'o/r', url: null }] }) });
    expect(pr.changedAnything).toBe('some');
    expect(pr.retry.kind).toBe('guarded');
    const repo = runOutcome({ status: 'error', receipt: receipt({ repoChanges: [{ root: '/r', label: 'app', target: 'cwd', status: 'error', headBefore: null, headAfter: null, files: [], totalFiles: 0, truncated: false } as never] }) });
    expect(repo.changedAnything).toBe('unknown');
    expect(repo.coverage).toContain('weren’t recorded');
    expect(repo.retry.kind).toBe('checkFirst');
  });

  it('says one shell command, not one shell commands', () => {
    const o = runOutcome({ status: 'error', receipt: receipt({ outsideEffects: { unknown: [{ ...shell, count: 1 }] } }) });
    expect(o.coverage).toContain('(shell command)');
  });

  it('calls a run that changed nothing and saw everything safe to repeat', () => {
    const o = runOutcome({ status: 'error', receipt: receipt() });
    expect(o.changedAnything).toBe('none');
    expect(o.retry.kind).toBe('safe');
    expect(o.coverage).toContain('Every call it made is accounted for');
  });

  it('reads an older receipt without outside effects as unknown, not none', () => {
    const o = runOutcome({ status: 'error', receipt: receipt({ outsideEffects: undefined }) });
    expect(o.changedAnything).toBe('unknown');
    expect(o.coverage).toContain('from before');
    expect(o.outsideRecorded).toBe(false);
    expect(o.retry).toMatchObject({ kind: 'checkFirst', label: null });
  });

  it('sends holds and budget stops to their fix first; a success needs no retry', () => {
    expect(runOutcome({ status: 'error', receipt: receipt(), blocked: true }).retry.kind).toBe('fixFirst');
    expect(runOutcome({ status: 'error', receipt: receipt(), budgetStop: true }).retry.detail).toContain('budget');
    expect(runOutcome({ status: 'success', receipt: receipt() }).retry).toMatchObject({ kind: 'none', label: null });
    expect(runOutcome({ status: 'interrupted', receipt: receipt({ outsideEffects: { recorded: [notion] } }) }).retry.kind).toBe('guarded');
  });

  it('keeps unknown cost unknown, never $0', () => {
    const r = receipt();
    expect(runOutcome({ status: 'success', receipt: r }).cost).toBe('Reading…');
    expect(runOutcome({ status: 'success', receipt: r, cost: null }).cost).toContain('no usage record could be read');
    expect(runOutcome({ status: 'success', receipt: r, cost: { ownUsd: 0.42, totalUsd: 0.42, knownUsd: 0.42, unknown: 0 } }).cost).toBe('$0.42');
    expect(runOutcome({ status: 'success', receipt: r, cost: { ownUsd: null, totalUsd: null, knownUsd: 0.1, unknown: 1 } }).cost).toBe('$0.10 known · 1 cost not reported');
    expect(runOutcome({ status: 'success', receipt: r, cost: { ownUsd: null, totalUsd: null, knownUsd: 0, unknown: 1 } }).cost).toContain('Not known');
    // Q8 review: a self-hosted model's part is GPU time, never $0 or "not known".
    expect(runOutcome({ status: 'success', receipt: r, cost: { ownUsd: null, totalUsd: null, knownUsd: 0, unknown: 0, gpuTime: 1 } }).cost).toBe('Billed by GPU time');
    expect(runOutcome({ status: 'success', receipt: r, cost: { ownUsd: 0.5, totalUsd: null, knownUsd: 0.75, unknown: 0, gpuTime: 2 } }).cost).toBe('$0.75 known, plus GPU time');
    expect(runOutcome({ status: 'success', receipt: r, cost: { ownUsd: null, totalUsd: null, knownUsd: 0, unknown: 1, gpuTime: 1 } }).cost).toBe('Billed by GPU time · 1 cost not reported');
  });

  it('gives the flight recorder the same GPU-time words (Q8 second review)', () => {
    expect(runCostSuffix({ totalUsd: 0.1234, knownUsd: 0.1234, unknown: 0 })).toBe(' · run cost $0.1234');
    expect(runCostSuffix({ totalUsd: null, knownUsd: 0, unknown: 1 })).toBe(' · run cost unknown');
    expect(runCostSuffix(null)).toBe(' · run cost unknown');
    expect(runCostSuffix({ totalUsd: null, knownUsd: 0, unknown: 0, gpuTime: 1 })).toBe(' · run cost: billed by GPU time');
    expect(runCostSuffix({ totalUsd: null, knownUsd: 0, unknown: 2, gpuTime: 1 })).toBe(' · run cost: billed by GPU time · 2 costs not reported');
    expect(runCostSuffix({ totalUsd: null, knownUsd: 0.5, unknown: 0, gpuTime: 1 })).toBe(' · run cost: $0.50 known, plus GPU time');
  });

  it('lists checks in the shared words, missing checks as not checked', () => {
    expect(runOutcome({ status: 'success', receipt: receipt() }).checks.label).toBe('Not checked');
    expect(runOutcome({ status: 'success', receipt: receipt({ checks: [{ command: 'npm test', kind: 'test', outcome: 'failed', sequence: 1, excerpt: '' }] }) }).checks.label).toBe('Checks failed');
  });
});

describe('runOutputs', () => {
  const r = receipt({ files: [
    { path: 'briefs/2026-07-07.html', changeType: 'create', attribution: 'confirmed', tool: 'Write' },
    { path: 'notes/scratch.md', changeType: 'modify', attribution: 'observed', tool: null },
    { path: 'workflows/brief.md', changeType: 'modify', attribution: 'observed', tool: null },
    { path: '.hollow/state.json', changeType: 'modify', attribution: 'observed', tool: null },
    { path: 'old.md', changeType: 'delete', attribution: 'observed', tool: null },
  ] });

  it('keeps the declared results when they are named', () => {
    expect(runOutputs(r, 'briefs/*.html')).toEqual({ paths: ['briefs/2026-07-07.html'], declared: true });
  });

  it('falls back to what it wrote, without its own file, dot-files or deletes', () => {
    expect(runOutputs(r, '')).toEqual({ paths: ['briefs/2026-07-07.html', 'notes/scratch.md'], declared: false });
    expect(runOutputs(null, 'x/*')).toEqual({ paths: [], declared: true });
  });

  it('never names another chat\'s file as this run\'s output, nor counts it (chat concurrency F1)', () => {
    const shared = receipt({ files: [{ path: 'outputs/report.md', changeType: 'modify', attribution: 'shared', tool: null }] });
    expect(runOutputs(shared, '')).toEqual({ paths: [], declared: false });
  });

  it('matches globs within and across folders', () => {
    const m = globMatcher('outputs/**/*.md, briefs/*.html')!;
    expect(m('outputs/a.md')).toBe(true);
    expect(m('outputs/x/y/a.md')).toBe(true);
    expect(m('briefs/a.html')).toBe(true);
    expect(m('briefs/x/a.html')).toBe(false);
    expect(globMatcher(' ')).toBeNull();
  });
});

it('priority pause remains distinct from success and never offers ordinary failure retry', () => {
  const paused = runOutcome({ status: 'paused', receipt: receipt({ status: 'paused' }), cost: null });
  expect(automationState({ lastStatus: 'paused', produced: true })).toBe('paused');
  expect(paused.retry.kind).toBe('none');
  expect(historyStatusLabel('paused')).toBe('Paused');
});
