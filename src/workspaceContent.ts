import type { GraphNode, Notification, ReviewInboxItem, VaultTree } from './types';

export const WORKSPACE_REVIEW_KINDS = {
  learning: 'Learning',
  commitment: 'Commitment',
  project_activation: 'Project brief',
  goal: 'Goal',
  heartbeat_task: 'Suggested task',
  meeting_task: 'Meeting task',
  workflow_permissions: 'Automation permissions',
  workflow_failure_streak: 'Failing automation',
  budget_pause: 'Budget pause',
  // Suggested by a chat or automation through hollow_tasks propose_tasks
  // (Paperclip PR 12); accepting creates the task.
  agent_task: 'Agent-proposed task',
  // The gardener's nightly tidy-up (master plan #19): Apply ticked applies the
  // ticked lines only.
  gardener: 'Tidy up',
  // An extracted decision or fact about a person (master plan F3), with its
  // recording timestamp or conversation turn.
  record: 'Decision or fact',
  // "Is 'Ana' Ana Ruiz?" (master plan #3): only the person links a name.
  person_alias: 'Who is this?',
  // A companion device and the Mac both changed one task field: Use the
  // device's value, or keep the Mac's (companion protocol v1).
  companion_conflict: 'Changed on two devices',
  // A companion op sent more than 14 days ago: Apply it, or Discard it
  // (companion protocol, "Old ops ask first").
  companion_old_op: 'Old change from a device',
  // A stale memory or home file (ME2), offered once: Rechecked closes it, and
  // the person re-dates the file.
  memory_recheck: 'Recheck memory',
  // NV7: a chat that missed a skill; accepting needs the reviewer's own
  // expected skill and writes a proposed eval fixture.
  eval_candidate: 'Eval candidate',
} as const;

export const WORKSPACE_REVIEW_SOURCES = {
  activation: 'Project scan', heartbeat: 'Check-in', meeting: 'Meeting', chat: 'Chat', agent: 'Agent', workflow: 'Automation',
  budget: 'Budget', gardener: 'Gardener', companion: 'Companion device', memory: 'Memory upkeep',
} as const;

export type WorkspaceReviewItem = ReviewInboxItem & {
  kind: keyof typeof WORKSPACE_REVIEW_KINDS;
  source: keyof typeof WORKSPACE_REVIEW_SOURCES;
};

export type SuggestionContext = 'tasks' | 'projects' | 'learning' | 'automations' | 'automationHealth' | 'budgets' | 'tidy' | 'records' | 'evals';

export const SUGGESTION_CONTEXTS = {
  tasks: { title: 'Task suggestions', kinds: ['heartbeat_task', 'meeting_task', 'agent_task', 'commitment', 'companion_conflict', 'companion_old_op'] },
  projects: { title: 'Project suggestions', kinds: ['project_activation', 'goal'] },
  learning: { title: 'Learning suggestions', kinds: ['learning'] },
  // Automation settings changed outside the editor, paused until approved.
  automations: { title: 'Permission changes', kinds: ['workflow_permissions'] },
  // Automations that failed three times in a row (Paperclip PR 9): Pause
  // schedule or Dismiss, sticky until the automation succeeds again.
  automationHealth: { title: 'Failing automations', kinds: ['workflow_failure_streak'] },
  // Hard-stop usage budgets holding automations (Paperclip PR 8a): raise and
  // resume, or keep paused. Not permission changes, so a panel of their own.
  budgets: { title: 'Budget pauses', kinds: ['budget_pause'] },
  // The gardener's one nightly card (master plan #19).
  // Plus stale memory and home files to recheck (ME2): upkeep, not learning
  // (the learning context opens Library Learnings, which can't decide them).
  tidy: { title: 'Tidy up', kinds: ['gardener', 'memory_recheck'] },
  // Extracted from recordings and chats (master plan F3).
  records: { title: 'Decisions and facts', kinds: ['record', 'person_alias'] },
  // Chats that missed a skill, filed as eval cases (NV7); linked from Skills.
  evals: { title: 'Eval candidates', kinds: ['eval_candidate'] },
} as const;

export function suggestionContext(item: ReviewInboxItem): SuggestionContext | null {
  if (!isWorkspaceReviewItem(item)) return null;
  for (const [context, { kinds }] of Object.entries(SUGGESTION_CONTEXTS)) {
    if ((kinds as readonly string[]).includes(item.kind)) return context as SuggestionContext;
  }
  return null;
}

/** Legacy persisted records must not reintroduce retired message surfaces. */
export function isRetiredMessageSource(source: string | null | undefined): boolean {
  const value = (source ?? '').replaceAll('\\', '/').replace(/^vault:/i, '');
  return /^(?:page:)?(?:email|emails|teams|slack|inbox)(?:$|[-/:])/i.test(value)
    || /(?:^|\/)(?:emails|teams|slack)(?:\/|$)/i.test(value)
    || /^mailto:/i.test(value);
}

export function isWorkspaceReviewItem(item: ReviewInboxItem): item is WorkspaceReviewItem {
  return Object.hasOwn(WORKSPACE_REVIEW_KINDS, item.kind)
    && Object.hasOwn(WORKSPACE_REVIEW_SOURCES, item.source)
    && item.risk === 'local_write'
    && !isRetiredMessageSource(item.sourcePath);
}

export function isWorkspaceNotification(item: Notification): boolean {
  return !isRetiredMessageSource(item.source) && !isRetiredMessageSource(item.actionUrl);
}

export function isWorkspaceVaultItem(item: { kind?: string; path: string }): boolean {
  return item.kind !== 'email' && item.kind !== 'chat' && !isRetiredMessageSource(item.path);
}

export function isWorkspaceGraphNode(node: GraphNode): boolean {
  return isWorkspaceVaultItem({ kind: node.kind, path: node.id });
}

export function filterWorkspaceTree(tree: VaultTree, base = '', excludedPaths = new Set<string>()): VaultTree {
  const result: VaultTree = {};
  for (const [name, node] of Object.entries(tree)) {
    const path = base ? `${base}/${name}` : name;
    if (isRetiredMessageSource(path) || excludedPaths.has(path)) continue;
    result[name] = node.kind === 'folder'
      ? { ...node, children: filterWorkspaceTree(node.children, path, excludedPaths) }
      : node;
  }
  return result;
}
