import { describe, expect, it } from 'vitest';
import type { GraphNode, Notification, ReviewInboxItem, VaultItem, VaultTree } from './types';
import { filterWorkspaceTree, isWorkspaceGraphNode, isWorkspaceNotification, isWorkspaceReviewItem, isWorkspaceVaultItem, suggestionContext, SUGGESTION_CONTEXTS, WORKSPACE_REVIEW_KINDS } from './workspaceContent';
import { KIND_META, matchesLens } from './vaultKinds';

function review(overrides: Partial<ReviewInboxItem> = {}): ReviewInboxItem {
  return {
    id: 'task-1', kind: 'meeting_task', source: 'meeting', sourcePath: 'calendar/review.md',
    risk: 'local_write', status: 'pending', payload: { title: 'Update the project brief' },
    createdAt: '2026-09-06', updatedAt: '2026-09-06', fingerprint: 'task-1',
    snoozedUntil: null, error: null, ...overrides,
  };
}

describe('workspace review boundaries', () => {
  it('places every supported suggestion in one contextual home', () => {
    const expected = {
      heartbeat_task: 'tasks', meeting_task: 'tasks', agent_task: 'tasks', commitment: 'tasks',
      project_activation: 'projects', goal: 'projects', learning: 'learning',
      workflow_permissions: 'automations', workflow_failure_streak: 'automationHealth', budget_pause: 'budgets',
      gardener: 'tidy', memory_recheck: 'tidy',
      eval_candidate: 'evals',
    };
    for (const [kind, context] of Object.entries(expected)) {
      expect(suggestionContext(review({ kind: kind as ReviewInboxItem['kind'] }))).toBe(context);
    }
    const routedKinds = Object.values(SUGGESTION_CONTEXTS).flatMap(context => context.kinds);
    expect(new Set(routedKinds).size).toBe(routedKinds.length);
    expect([...routedKinds].sort()).toEqual(Object.keys(WORKSPACE_REVIEW_KINDS).sort());
  });

  it('never assigns retired messaging records to a contextual suggestions panel', () => {
    for (const item of [
      review({ kind: 'heartbeat_send' }), review({ kind: 'reply_draft', status: 'accepted' }),
      review({ kind: 'meeting_followup' }), review({ sourcePath: 'emails/old.md' }),
      review({ risk: 'external_send' }), review({ source: 'inbox' }),
    ]) expect(suggestionContext(item)).toBeNull();
  });

  it('routes workflow permission reviews to Automations', () => {
    const permission = review({
      kind: 'workflow_permissions', source: 'workflow', sourcePath: 'workflows/close-my-day.md',
      payload: { slug: 'close-my-day', title: 'Close my day' },
    });
    expect(isWorkspaceReviewItem(permission)).toBe(true);
    expect(suggestionContext(permission)).toBe('automations');
  });

  it('routes failure-streak cards to Failing automations, apart from permission reviews', () => {
    const streak = review({
      kind: 'workflow_failure_streak', source: 'workflow', sourcePath: 'workflows/nightly-report.md',
      fingerprint: 'workflow-failure-streak:nightly-report:run-3',
      payload: { slug: 'nightly-report', title: 'Nightly report', count: 3, lastError: 'claude exited 1' },
    });
    expect(isWorkspaceReviewItem(streak)).toBe(true);
    expect(suggestionContext(streak)).toBe('automationHealth');
    expect(SUGGESTION_CONTEXTS.automationHealth.title).toBe('Failing automations');
  });

  it('routes budget pauses to their own Automations panel (Paperclip PR 8a)', () => {
    const pause = review({
      kind: 'budget_pause', source: 'budget', sourcePath: null,
      payload: { budgetId: 'b1', scopeLabel: 'all usage' },
    });
    expect(isWorkspaceReviewItem(pause)).toBe(true);
    expect(suggestionContext(pause)).toBe('budgets');
    expect(SUGGESTION_CONTEXTS.budgets.title).toBe('Budget pauses');
  });

  it('keeps local Calendar tasks and chat learning, including their history', () => {
    expect(isWorkspaceReviewItem(review())).toBe(true);
    expect(isWorkspaceReviewItem(review({ kind: 'learning', source: 'chat', sourcePath: null, status: 'accepted' }))).toBe(true);
  });

  it('hides persisted sends and drafts regardless of status or a stale risk label', () => {
    expect(isWorkspaceReviewItem(review({ kind: 'heartbeat_send', risk: 'external_send' }))).toBe(false);
    expect(isWorkspaceReviewItem(review({ kind: 'reply_draft', status: 'accepted' }))).toBe(false);
    expect(isWorkspaceReviewItem(review({ kind: 'meeting_followup', status: 'snoozed' }))).toBe(false);
    expect(isWorkspaceReviewItem(review({ risk: 'external_send' }))).toBe(false);
  });

  it('excludes local proposals derived from retired message sources', () => {
    expect(isWorkspaceReviewItem(review({ kind: 'commitment', source: 'inbox', sourcePath: null }))).toBe(false);
    expect(isWorkspaceReviewItem(review({ sourcePath: 'vault:emails/2026-07-follow-up.md' }))).toBe(false);
    expect(isWorkspaceReviewItem(review({ sourcePath: 'C:\\vault\\Teams\\thread.md' }))).toBe(false);
  });

  it('does not admit unknown persisted kinds through inherited object keys', () => {
    expect(isWorkspaceReviewItem(review({ kind: 'constructor' as ReviewInboxItem['kind'] }))).toBe(false);
  });
});

describe('workspace notifications', () => {
  const notification: Notification = {
    id: '1', title: 'Meeting starts soon', body: '', kind: 'info', source: 'calendar',
    actionLabel: 'Open meeting', actionUrl: 'vault:calendar/review.md', created: '2026-09-06', read: false,
  };

  it('keeps Calendar reminders and local task updates', () => {
    expect(isWorkspaceNotification(notification)).toBe(true);
    expect(isWorkspaceNotification({ ...notification, source: 'tasks', actionUrl: 'vault:tasks/review.md' })).toBe(true);
  });

  it('hides retired notification sources and compose links', () => {
    expect(isWorkspaceNotification({ ...notification, source: 'email-aging' })).toBe(false);
    expect(isWorkspaceNotification({ ...notification, source: 'Slack' })).toBe(false);
    expect(isWorkspaceNotification({ ...notification, actionUrl: 'page:inbox' })).toBe(false);
    expect(isWorkspaceNotification({ ...notification, actionUrl: 'mailto:person@example.com' })).toBe(false);
  });
});

describe('workspace knowledge views', () => {
  it('excludes message kinds even outside legacy folders, preserving ordinary notes', () => {
    const local = { kind: 'note', path: 'notes/email-protocol.md', hidden: false } as VaultItem;
    const legacy = { ...local, kind: 'email', path: 'imports/message.md' } as VaultItem;
    expect(isWorkspaceVaultItem(local)).toBe(true);
    expect(isWorkspaceVaultItem(legacy)).toBe(false);
    expect(matchesLens(legacy, 'all')).toBe(false);
    expect(Object.keys(KIND_META)).not.toEqual(expect.arrayContaining(['email', 'chat']));
    expect(isWorkspaceGraphNode({ id: legacy.path, kind: 'email' } as GraphNode)).toBe(false);
    expect(isWorkspaceGraphNode({ id: 'calendar/teams-meeting.md', kind: 'meeting' } as GraphNode)).toBe(true);
  });

  it('removes retired tree branches and known message files without altering note contents', () => {
    const note = { kind: 'file', path: 'notes/work.md', content: '# Email protocol\n\nKeep this authored note.' } as const;
    const tree = {
      emails: { kind: 'folder', children: { 'old.md': note } },
      notes: { kind: 'folder', children: { 'work.md': note, 'old-import.md': note } },
      calendar: { kind: 'folder', children: { 'review.md': note } },
    } as unknown as VaultTree;
    const filtered = filterWorkspaceTree(tree, '', new Set(['notes/old-import.md']));
    expect(Object.keys(filtered)).toEqual(['notes', 'calendar']);
    expect(filtered.notes).toEqual({ kind: 'folder', children: { 'work.md': note } });
    expect(tree.emails).toBeDefined();
  });
});
