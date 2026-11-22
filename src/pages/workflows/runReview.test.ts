// The Automations run review's plain helpers (plan completion L2, item 6;
// P0-13 for the Run now confirm): which run the review explains, where and
// by whom a run works in words, what a refused run opens, and what the row's
// Run now asks first. The controller itself (`createRunReview`) needs a
// browser, so only the exported helpers are covered here.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { RunLogEntry, Workflow } from '../../types';

const h = vi.hoisted(() => ({
  refreshReviewInbox: vi.fn(), requestSuggestions: vi.fn(), showConfirm: vi.fn(async () => true),
  workspaces: [] as { cwd: string; title: string }[], defaultAgent: '', available: [] as string[],
}));
vi.mock('../../api', () => ({ api: {} }));
vi.mock('../../nativeEvents', () => ({ listen: vi.fn() }));
vi.mock('../../scopedListen', () => ({ scopedListen: vi.fn() }));
vi.mock('../../store/runQueue', () => ({ runQueue: () => [] }));
vi.mock('../../store/workspaces', () => ({ projectWorkspaces: () => h.workspaces }));
vi.mock('../../reviewInbox', () => ({ refreshReviewInbox: h.refreshReviewInbox, requestSuggestions: h.requestSuggestions }));
vi.mock('../../store', () => ({
  showConfirm: h.showConfirm,
  AGENT_LABELS: { claude: 'Claude', codex: 'Codex' },
  agentsAvailable: () => h.available,
  agentSettings: () => ({ defaultAgent: h.defaultAgent }),
}));

import { confirmRunNow, newestRun, noteRunFailure, workflowAgentName, workflowTarget } from './runReview';

const run = (at: string, status: RunLogEntry['status'], note = ''): RunLogEntry =>
  ({ at, status, durationSec: 1, note, runId: null, errorTail: null, exitKind: null });
const wf = (over: Partial<Workflow> = {}) => ({ title: 'Nightly', cwd: '', agent: '', ...over }) as Workflow;

beforeEach(() => {
  vi.clearAllMocks();
  h.workspaces = []; h.defaultAgent = ''; h.available = [];
});

describe('the run the review explains', () => {
  it('is the newest run that started, whatever order the log is in', () => {
    const log = [run('2026-11-12T01:00:00Z', 'success', 'old'), run('2026-11-13T01:00:00Z', 'error', 'newest'), run('2026-11-12T09:00:00Z', 'success', 'mid')];
    expect(newestRun(log)?.note).toBe('newest');
    expect(log[0].note).toBe('old');
  });

  it('skips skipped and deferred rows, which describe an occurrence that never ran', () => {
    const log = [run('2026-11-13T03:00:00Z', 'skipped'), run('2026-11-13T02:00:00Z', 'deferred'), run('2026-11-13T01:00:00Z', 'cancelled', 'real')];
    expect(newestRun(log)?.note).toBe('real');
  });

  it('counts interrupted and paused runs as runs', () => {
    expect(newestRun([run('2026-11-13T01:00:00Z', 'interrupted')])?.status).toBe('interrupted');
    expect(newestRun([run('2026-11-13T01:00:00Z', 'paused')])?.status).toBe('paused');
  });

  it('is null for no log, an empty log or a log of rows that never ran', () => {
    expect(newestRun(undefined)).toBeNull();
    expect(newestRun([])).toBeNull();
    expect(newestRun([run('2026-11-13T01:00:00Z', 'skipped')])).toBeNull();
  });
});

describe('where a run works and which agent runs it', () => {
  it('names the workspace whose folder it is, else the folder, else the vault', () => {
    h.workspaces = [{ cwd: '/p/atlas', title: 'Atlas' }];
    expect(workflowTarget(wf({ cwd: '/p/atlas' }))).toBe('the Atlas workspace');
    expect(workflowTarget(wf({ cwd: '/p/other' }))).toBe('/p/other');
    expect(workflowTarget(wf({ cwd: '' }))).toBe('the vault');
  });

  it('uses the automation’s own agent, else the default, else the first available, else a plain noun', () => {
    h.defaultAgent = 'codex'; h.available = ['claude'];
    expect(workflowAgentName(wf({ agent: 'claude' }))).toBe('Claude');
    expect(workflowAgentName(wf({ agent: '' }))).toBe('Codex');
    h.defaultAgent = '';
    expect(workflowAgentName(wf({ agent: '' }))).toBe('Claude');
    h.available = [];
    expect(workflowAgentName(wf({ agent: '' }))).toBe('The agent');
  });

  it('keeps an agent it has no label for as recorded', () => {
    expect(workflowAgentName(wf({ agent: 'acme' }))).toBe('acme');
  });
});

describe('a refused run', () => {
  it('always refreshes the review queue, and opens nothing for an ordinary failure', () => {
    noteRunFailure(new Error('spawn failed'));
    expect(h.refreshReviewInbox).toHaveBeenCalledTimes(1);
    expect(h.requestSuggestions).not.toHaveBeenCalled();
  });

  it('opens Waiting on you on the automation’s permission changes when that is why', () => {
    noteRunFailure('Automation paused for review: its tools changed');
    expect(h.requestSuggestions).toHaveBeenCalledWith('automations');
  });

  it('opens it on the budget pauses when a usage budget held the run', () => {
    noteRunFailure(new Error('Budget paused: daily limit reached'));
    expect(h.requestSuggestions).toHaveBeenCalledWith('budgets');
    expect(h.requestSuggestions).toHaveBeenCalledTimes(1);
  });

  it('handles an empty rejection without throwing', () => {
    noteRunFailure(undefined);
    expect(h.refreshReviewInbox).toHaveBeenCalledTimes(1);
    expect(h.requestSuggestions).not.toHaveBeenCalled();
  });
});

describe('Run now from the list asks first', () => {
  it('names the automation, the agent and where it works, and focuses Cancel', async () => {
    h.workspaces = [{ cwd: '/p/atlas', title: 'Atlas' }];
    expect(await confirmRunNow(wf({ title: 'Nightly digest', cwd: '/p/atlas', agent: 'codex' }))).toBe(true);
    expect(h.showConfirm).toHaveBeenCalledWith({
      title: 'Run “Nightly digest” now?',
      body: 'A real run: Codex works in the Atlas workspace and uses model allowance.',
      confirmLabel: 'Run now', focusCancel: true,
    });
  });

  it('calls an untitled automation by that name and passes a refusal back', async () => {
    h.showConfirm.mockResolvedValueOnce(false);
    h.available = ['claude'];
    expect(await confirmRunNow(wf({ title: '' }))).toBe(false);
    expect(h.showConfirm).toHaveBeenCalledWith(expect.objectContaining({
      title: 'Run “untitled automation” now?',
      body: 'A real run: Claude works in the vault and uses model allowance.',
    }));
  });
});
