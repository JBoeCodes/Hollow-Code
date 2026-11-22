import { describe, expect, it } from 'vitest';
import type { ReviewInboxItem, Workflow } from './types';
import {
  canPauseStreak, failureFix, isAuthFailureText, isStreakItem, receiptCheckCounts, isWorkspaceCwd, NOTIFY_LABELS, NOTIFY_OPTIONS, notifyForSave, notifySelectValue,
  parseWatch, precheckForSave, quietForSave, startedLogText, switchedLogText, STREAK_ACTION_SCHEME, streakActionSlug, streakPayload, streakSummary,
  watchPlaceholder,
} from './workflowHealth';

function card(payload: Record<string, unknown> = {}, overrides: Partial<ReviewInboxItem> = {}): ReviewInboxItem {
  return {
    id: 'streak-1', kind: 'workflow_failure_streak', source: 'workflow', sourcePath: 'workflows/nightly-report.md',
    risk: 'local_write', status: 'pending', createdAt: '2026-09-25', updatedAt: '2026-09-25',
    fingerprint: 'workflow-failure-streak:nightly-report:run-3', snoozedUntil: null, error: null,
    payload: {
      slug: 'nightly-report', title: 'Nightly report', count: 3, firstFailedAt: '2026-09-25T01:00:00Z',
      lastError: 'claude exited 1', schedule: '0 0 7 * * * *', triggers: [], ...payload,
    },
    ...overrides,
  };
}

function workflow(slug: string, schedule: string): Workflow {
  return { slug, schedule } as Workflow;
}

describe('failure-streak cards', () => {
  it('parses the payload strictly', () => {
    const parsed = streakPayload(card());
    expect(parsed).toEqual({
      slug: 'nightly-report', title: 'Nightly report', count: 3, firstFailedAt: '2026-09-25T01:00:00Z',
      lastError: 'claude exited 1', schedule: '0 0 7 * * * *', triggers: [],
    });
    for (const broken of [
      { slug: '' }, { title: 7 }, { count: 0 }, { count: 2.5 }, { count: '3' },
      { lastError: null }, { schedule: undefined }, { triggers: 'task-overdue' }, { triggers: [1] },
    ]) expect(streakPayload(card(broken))).toBeNull();
    expect(streakPayload(card({}, { kind: 'workflow_permissions' }))).toBeNull();
    expect(isStreakItem(card())).toBe(true);
  });

  it('summarises the count and the last error', () => {
    expect(streakSummary(streakPayload(card()))).toBe('Failed 3 times in a row · claude exited 1');
    expect(streakSummary(streakPayload(card({ lastError: '  ' })))).toBe('Failed 3 times in a row');
    expect(streakSummary(null)).toBe('This automation keeps failing.');
  });

  it('pauses only a live workflow that still has a schedule', () => {
    const payload = streakPayload(card());
    expect(canPauseStreak(payload, [workflow('nightly-report', '0 0 7 * * * *')])).toBe(true);
    // The live schedule wins over the card's stale one.
    expect(canPauseStreak(payload, [workflow('nightly-report', 'manual')])).toBe(false);
    expect(canPauseStreak(streakPayload(card({ schedule: 'manual' })), [workflow('nightly-report', '0 0 9 * * * *')])).toBe(true);
    expect(canPauseStreak(payload, [workflow('other', '0 0 7 * * * *')])).toBe(false);
    expect(canPauseStreak(payload, undefined)).toBe(false);
    expect(canPauseStreak(null, [workflow('nightly-report', '0 0 7 * * * *')])).toBe(false);
  });

  it('reads the streak notification action', () => {
    expect(streakActionSlug(`${STREAK_ACTION_SCHEME}nightly-report`)).toBe('nightly-report');
    expect(streakActionSlug('workflow:nightly-report')).toBeNull();
    expect(streakActionSlug(STREAK_ACTION_SCHEME)).toBeNull();
    expect(streakActionSlug(null)).toBeNull();
  });
});

describe('the notify select', () => {
  it('hydrates like the backend reads it', () => {
    expect(notifySelectValue(null)).toBe('always');
    expect(notifySelectValue('')).toBe('always');
    expect(notifySelectValue(' Failures ')).toBe('failures');
    expect(notifySelectValue('NEVER')).toBe('never');
    expect(notifySelectValue('sometimes')).toBe('always');
    expect(NOTIFY_OPTIONS.map(o => NOTIFY_LABELS[o])).toEqual(['Every run', 'Only failures', 'Never']);
  });

  it('sends only a value the user changed', () => {
    expect(notifyForSave('always', 'always')).toBeUndefined();
    expect(notifyForSave('never', 'never')).toBeUndefined();
    expect(notifyForSave('never', 'always')).toBe('never');
    expect(notifyForSave('always', 'failures')).toBe('always');
  });

  it('labels the start of a live run with its origin', () => {
    expect(startedLogText(undefined, undefined)).toBe('Run started');
    expect(startedLogText('manual', 'run now')).toBe('Run started · run now');
    expect(startedLogText('cron', 'scheduled')).toBe('Run started · scheduled');
    expect(startedLogText('trigger', 'file changed: notes/a.md')).toBe('Run started · trigger: file changed: notes/a.md');
    expect(startedLogText('retry', '')).toBe('Run started · retry');
  });

  it('says when a run moved off its agent budget ceiling (decision 6)', () => {
    expect(switchedLogText(null)).toBeNull();
    expect(switchedLogText(undefined)).toBeNull();
    expect(switchedLogText({ from: 'Claude', to: 'Codex', reason: 'Claude usage reached 100% of its daily budget.' }))
      .toBe('Moved from Claude to Codex at a budget ceiling · Claude usage reached 100% of its daily budget.');
    expect(switchedLogText({ from: 'Claude', to: 'Codex', reason: ' ' }))
      .toBe('Moved from Claude to Codex at a budget ceiling');
  });
});

describe('skip when nothing changed (Paperclip PR 9b)', () => {
  it('parses watch globs from commas and lines', () => {
    expect(parseWatch(' notes/**, projects/*.md\n\n  inbox/* ')).toEqual(['notes/**', 'projects/*.md', 'inbox/*']);
    expect(parseWatch('')).toEqual([]);
  });

  it('knows a workspace folder and says what the watch needs', () => {
    expect(isWorkspaceCwd('/vault/workspaces/launch-plan')).toBe(true);
    expect(isWorkspaceCwd('/vault/workspaces/launch-plan/')).toBe(true);
    expect(isWorkspaceCwd('/vault/workspaces/launch-plan/outputs')).toBe(false);
    expect(isWorkspaceCwd('')).toBe(false);
    expect(watchPlaceholder('/vault/workspaces/launch-plan')).toBe('workspaces/<id>/** (default)');
    expect(watchPlaceholder('')).toBe('required, e.g. notes/**');
    expect(watchPlaceholder('/code/app')).toContain('absolute globs under a folder outside the vault');
  });

  it('sends only what the user changed', () => {
    expect(quietForSave(false, false, '', [])).toEqual({});
    expect(quietForSave(true, true, 'notes/**', ['notes/**'])).toEqual({});
    expect(quietForSave(true, false, 'notes/**', ['notes/**'])).toEqual({ skipIfQuiet: true });
    expect(quietForSave(true, true, 'notes/**, a/*', ['notes/**'])).toEqual({ watch: ['notes/**', 'a/*'] });
    expect(quietForSave(false, true, '', ['notes/**'])).toEqual({ skipIfQuiet: false, watch: [] });
  });

  it('sends the precheck only when it changed (OC13); empty removes it', () => {
    expect(precheckForSave('', '')).toBeUndefined();
    expect(precheckForSave('  exit 1 ', 'exit 1')).toBeUndefined();
    expect(precheckForSave('exit 1', '')).toBe('exit 1');
    expect(precheckForSave(' test -d x ', 'exit 1')).toBe('test -d x');
    expect(precheckForSave('', 'exit 1')).toBe('');
  });
});

describe('failureFix (next-level P17)', () => {
  const failed = { lastStatus: 'error', lastError: 'claude exited 1', agentName: 'Claude' };

  it('puts holds before the latest failure, each with one fix', () => {
    expect(failureFix({ ...failed, permissionHold: 'review', budgetHold: 'Budget paused: x' })!.action).toBe('permissions');
    expect(failureFix({ ...failed, permissionHold: 'unavailable' })!.action).toBe('none');
    expect(failureFix({ ...failed, budgetHold: 'Budget paused: daily tokens' })).toMatchObject({ reason: 'Budget paused: daily tokens', action: 'budgets' });
    expect(failureFix({ lastStatus: 'success', lastError: null })).toBeNull();
  });

  it('reads sign-in, folder, agent and check failures', () => {
    expect(failureFix({ ...failed, errorTail: 'API Error: 401 Invalid authentication' })).toMatchObject({ action: 'accounts', actionLabel: 'Open Accounts' });
    const folder = failureFix({ ...failed, lastError: 'working folder /x does not exist or is not a folder' })!;
    expect(folder.reason).toBe('The working folder /x does not exist or is not a folder.');
    expect(folder.action).toBe('edit');
    expect(failureFix({ ...failed, lastError: 'codex is not installed' })!.action).toBe('edit');
    expect(failureFix({ ...failed, failedCheck: 'npm test' })).toMatchObject({ reason: 'A check failed: npm test.', action: 'retry', actionLabel: null });
    expect(failureFix(failed)).toMatchObject({ reason: 'claude exited 1', action: 'retry' });
    expect(failureFix({ ...failed, lastError: null })!.reason).toBe('The last run ended with an error.');
  });

  // Plan completion L2 (V03): an authentication error never proves the run
  // did nothing; only its receipt can say so.
  it('never reads a sign-in failure as "nothing happened" without a receipt saying so', () => {
    const auth = { ...failed, errorTail: 'API Error: 401 Invalid authentication' };
    for (const fix of [failureFix(auth)!, failureFix({ ...auth, changedAnything: 'unknown' })!]) {
      expect(fix.reason).not.toMatch(/before doing any work|didn.t do anything|nothing happened/i);
      expect(fix.reason).toContain('can’t tell');
    }
    expect(failureFix({ ...auth, changedAnything: 'some' })!.reason).toContain('already changed something');
    expect(failureFix({ ...auth, changedAnything: 'none' })!.reason).toContain('run record shows the run changed nothing');
    expect(failureFix(failed)!.fix).not.toMatch(/^Run now to try again/);
    // With no run to explain on the page, the fix doesn't point at one.
    expect(failureFix({ ...auth, outcomeShown: false })!.reason).not.toContain('What happened');
    expect(failureFix({ ...failed, outcomeShown: false })!.fix).not.toContain('What happened');
    // A bare 401 inside a hash or a duration is not a sign-in failure.
    expect(isAuthFailureText('wrote 1401ms; sha a401bc', null)).toBe(false);
    expect(isAuthFailureText('API Error: 401 {"type":"authentication_error"}', null)).toBe(true);
    expect(isAuthFailureText(null, 'OAuth token has expired. Please run /login')).toBe(true);
  });

  it('counts receipt checks without treating missing ones as passed', () => {
    expect(receiptCheckCounts(null)).toBeNull();
    expect(receiptCheckCounts([])).toEqual({ failed: 0, total: 0, unknown: 0 });
    expect(receiptCheckCounts([{ outcome: 'passed' }, { outcome: 'failed' }, { outcome: 'unknown' }])).toEqual({ failed: 1, total: 3, unknown: 1 });
  });
});
