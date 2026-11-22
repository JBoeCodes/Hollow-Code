// Pure helpers for automation permission reviews (Paperclip PR 5). No store
// or Tauri import, so the node vitest suite can load it.
import type { ReviewInboxItem, RunFlag, Workflow, WorkflowPermissionDiffEntry, WorkflowPermissionReviewPayload } from './types';
import { errMsg } from './utils';

/** Mirrors Rust `workflow_permissions::REFUSAL_PREFIX`. */
export const WORKFLOW_REVIEW_REFUSAL_PREFIX = 'Automation paused for review:';
/** Mirrors Rust `review_queue::SUPERSEDED_PREFIX`. */
export const REVIEW_SUPERSEDED_PREFIX = 'Superseded:';

/** True when a thrown value (Tauri rejects with plain strings) is a refusal to
 *  run an automation whose settings changed outside the editor. */
export function isPermissionRefusal(error: unknown): boolean {
  if (error === null || error === undefined) return false;
  return errMsg(error).trimStart().startsWith(WORKFLOW_REVIEW_REFUSAL_PREFIX);
}

/** The next-run text for an automation the backend's permission check is
 *  refusing (`Workflow.permissionHold`, from the same rule the scheduler
 *  applies: a pending review and a user's rejection both hold it), or null
 *  when it may run. The review queue alone can't tell: a rejection stays in
 *  the queue after the file is restored to its approved version. */
export function permissionHoldText(hold: Workflow['permissionHold'] | null): string | null {
  if (hold === 'review') return 'paused until its settings change is approved';
  if (hold === 'unavailable') return 'paused: automation permission records are unavailable';
  return null;
}

const RUN_FLAG_LABELS: Record<RunFlag['kind'], string> = {
  workflow: 'Automation settings',
  workflowSettings: 'Automation notifications',
  operatorProfile: 'Operator profile',
  agentInstructions: 'Agent instructions',
  agentConfig: 'Agent configuration',
  stateFile: 'Hollow state file',
  historyManifest: 'History record',
  sharedDoc: 'Shared workspace file',
};

type FlaggedRun = { flags?: RunFlag[] | null };

/** A run's protected changes: every flag but a shared doc written over. */
export function protectedFlags(run: FlaggedRun): RunFlag[] {
  return (run.flags ?? []).filter(flag => flag.kind !== 'sharedDoc');
}

/** Shared workspace docs the run wrote over from an older copy (merged back
 *  or kept as a conflict copy; concurrency PR 2, 2026-10-20). */
export function sharedDocFlags(run: FlaggedRun): RunFlag[] {
  return (run.flags ?? []).filter(flag => flag.kind === 'sharedDoc');
}

/** One line per protected change in a run: kind · path · change · detail. */
export function runFlagLine(flag: RunFlag): string {
  const parts = [RUN_FLAG_LABELS[flag.kind] ?? flag.kind, flag.path];
  if (flag.changeType) parts.push(flag.changeType);
  if (flag.detail) parts.push(flag.detail);
  return parts.join(' · ');
}

/** Status chip text. A rejection Hollow made because the subject changed again
 *  reads 'superseded', never as the user's own rejection. */
export function reviewStatusLabel(item: Pick<ReviewInboxItem, 'status' | 'error'>): string {
  if (item.status === 'pending') return 'Suggested';
  if (item.status === 'rejected' && (item.error ?? '').startsWith(REVIEW_SUPERSEDED_PREFIX)) return 'superseded';
  return item.status;
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every(entry => typeof entry === 'string');
}

function nullableString(value: unknown): value is string | null {
  return value === null || typeof value === 'string';
}

function diffEntry(value: unknown): WorkflowPermissionDiffEntry | null {
  if (!value || typeof value !== 'object') return null;
  const entry = value as Record<string, unknown>;
  if (typeof entry.field !== 'string') return null;
  if (entry.kind === 'scalar' && nullableString(entry.before) && nullableString(entry.after)) {
    return { field: entry.field, kind: 'scalar', before: entry.before, after: entry.after };
  }
  if (entry.kind === 'list' && isStringArray(entry.added) && isStringArray(entry.removed)) {
    return { field: entry.field, kind: 'list', added: entry.added, removed: entry.removed, reordered: entry.reordered === true };
  }
  if (entry.kind === 'text' && entry.field === 'prompt' && nullableString(entry.before) && typeof entry.after === 'string') {
    return { field: 'prompt', kind: 'text', before: entry.before, after: entry.after, truncated: entry.truncated === true };
  }
  return null;
}

/** Defensive parse of a `workflow_permissions` payload (the queue lives in the
 *  agent-writable vault; the backend re-derives it before approving). */
export function permissionPayload(item: Pick<ReviewInboxItem, 'kind' | 'payload'>): WorkflowPermissionReviewPayload | null {
  if (item.kind !== 'workflow_permissions') return null;
  const p = item.payload as Record<string, unknown>;
  if (typeof p.slug !== 'string' || typeof p.title !== 'string') return null;
  const diff = Array.isArray(p.diff)
    ? p.diff.map(diffEntry).filter((entry): entry is WorkflowPermissionDiffEntry => entry !== null)
    : [];
  return {
    slug: p.slug,
    title: p.title,
    path: typeof p.path === 'string' ? p.path : `workflows/${p.slug}.md`,
    observedDigest: typeof p.observedDigest === 'string' ? p.observedDigest : '',
    approvedDigest: typeof p.approvedDigest === 'string' ? p.approvedDigest : null,
    changedFields: isStringArray(p.changedFields) ? p.changedFields : [],
    escalation: p.escalation === true,
    summary: typeof p.summary === 'string' ? p.summary : '',
    diff,
  };
}

function label(field: string): string {
  return field ? field[0].toUpperCase() + field.slice(1) : field;
}

/** Plain-text confirmation body: one line per diff entry, every added and
 *  removed tool listed in full. */
export function permissionConfirmBody(payload: WorkflowPermissionReviewPayload): string {
  const lines: string[] = [];
  if (payload.approvedDigest === null) {
    lines.push('This automation file has no current approval: it is new, or it was recreated after being deleted.');
  }
  for (const entry of payload.diff) {
    if (entry.kind === 'scalar') {
      lines.push(`${label(entry.field)}: ${entry.before ?? '(none)'} → ${entry.after ?? '(none)'}`);
    } else if (entry.kind === 'list') {
      const parts = [
        ...entry.added.map(value => `+${value}`),
        ...entry.removed.map(value => `−${value}`),
      ];
      lines.push(`${label(entry.field)}: ${parts.length ? parts.join(', ') : 'reordered'}`);
    } else {
      lines.push(entry.before === null ? 'Prompt: new or unavailable for comparison' : 'Prompt: changed (see the card for the excerpt)');
    }
  }
  if (payload.escalation) {
    lines.push('', 'This grants the automation more access than it had.');
  }
  lines.push('', 'Hollow will run the automation with these settings from now on.');
  return lines.join('\n');
}

// ── Editor changes that need approval (next-level P17) ─────────────────────

/** The stamped fields as the automation editor holds them (Rust
 *  `workflow_permissions::PermissionFields`, minus the prompt digest, which
 *  the editor compares as text). */
export interface StampedFields {
  permissionMode: string;
  allowedTools: readonly string[];
  disallowedTools: readonly string[];
  cwd: string | null;
  agent: string | null;
  model: string | null;
  triggers: readonly string[];
  schedule: string;
  prompt: string;
  /** The owner Mac (M4, `runs_on`) as saved or as the save will write it;
   *  absent where the caller doesn't edit it. */
  runsOn?: string | null;
  /** The precheck command (OC13) as saved or as the save will write it;
   *  absent where the caller doesn't edit it. */
  precheck?: string | null;
}

export interface StampedChange {
  /** The backend's label (`changed_fields`): `permission mode`, `allowed tools`, … */
  field: string;
  /** One line for the review list. */
  text: string;
}

export interface StampedChanges {
  changes: StampedChange[];
  /** True when the change grants more access (Rust
   *  `PermissionFields::escalation`). */
  widens: boolean;
}

/** plan < default < acceptEdits < auto < bypassPermissions; unknown ranks
 *  highest (mirrors Rust `mode_rank`). */
export function modeRank(mode: string): number {
  const ranks: Record<string, number> = { plan: 0, default: 1, acceptEdits: 2, auto: 3, bypassPermissions: 4 };
  return ranks[(mode ?? '').trim() || 'acceptEdits'] ?? 5;
}

const listText = (list: readonly string[]) => (list.length ? list.join(', ') : 'none');
const valueText = (value: string | null) => (value && value.trim() ? value : 'default');

/** What an editor save would change in the automation's approved settings,
 *  and whether it widens access, by the backend's own rules
 *  (`changed_fields` / `escalation`). An editor save of a file that matched
 *  its approval approves the new settings, so these are what the user is
 *  approving. An empty allowlist means every tool: adding the first entries
 *  restricts, clearing a non-empty list widens. */
export function stampedChanges(before: StampedFields, after: StampedFields, macName: (owner: string | null) => string = o => o || 'no owner'): StampedChanges {
  const mode = (m: string) => (m ?? '').trim() || 'acceptEdits';
  const same = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((v, i) => v === b[i]);
  const gained = (a: readonly string[], b: readonly string[]) => b.some(t => !a.includes(t));
  const changes: StampedChange[] = [];
  if (mode(before.permissionMode) !== mode(after.permissionMode)) {
    changes.push({ field: 'permission mode', text: `Permission mode: ${mode(before.permissionMode)} → ${mode(after.permissionMode)}` });
  }
  if (!same(before.allowedTools, after.allowedTools)) {
    const added = after.allowedTools.filter(t => !before.allowedTools.includes(t));
    const removed = before.allowedTools.filter(t => !after.allowedTools.includes(t));
    const text = after.allowedTools.length === 0
      ? 'Allowed tools: cleared, so every tool the permission mode allows'
      : before.allowedTools.length === 0
        ? `Allowed tools: limited to ${listText(after.allowedTools)}`
        : `Allowed tools: ${[...added.map(t => `+${t}`), ...removed.map(t => `−${t}`)].join(', ') || 'reordered'}`;
    changes.push({ field: 'allowed tools', text });
  }
  if (!same(before.disallowedTools, after.disallowedTools)) {
    const added = after.disallowedTools.filter(t => !before.disallowedTools.includes(t));
    const removed = before.disallowedTools.filter(t => !after.disallowedTools.includes(t));
    changes.push({ field: 'disallowed tools', text: `Disallowed tools: ${[...added.map(t => `+${t}`), ...removed.map(t => `−${t}`)].join(', ') || 'reordered'}` });
  }
  if ((before.cwd ?? null) !== (after.cwd ?? null)) {
    changes.push({ field: 'working folder', text: `Working folder: ${before.cwd || 'the vault'} → ${after.cwd || 'the vault'}` });
  }
  if ((before.agent ?? null) !== (after.agent ?? null)) {
    changes.push({ field: 'agent', text: `Agent: ${valueText(before.agent)} → ${valueText(after.agent)}` });
  }
  if ((before.model ?? null) !== (after.model ?? null)) {
    changes.push({ field: 'model', text: `Model: ${valueText(before.model)} → ${valueText(after.model)}` });
  }
  if (!same(before.triggers, after.triggers)) {
    changes.push({ field: 'triggers', text: `Triggers: ${listText(before.triggers)} → ${listText(after.triggers)}` });
  }
  if (before.schedule !== after.schedule) {
    changes.push({ field: 'schedule', text: `Schedule: ${before.schedule} → ${after.schedule}` });
  }
  if (before.prompt.trim() !== after.prompt.trim()) {
    changes.push({ field: 'prompt', text: 'Prompt: changed' });
  }
  const ownerBefore = before.runsOn ?? null;
  const ownerAfter = after.runsOn ?? null;
  if (ownerBefore !== ownerAfter) {
    changes.push({ field: 'runs on', text: `Runs on: ${macName(ownerBefore)} → ${macName(ownerAfter)}` });
  }
  const precheckBefore = (before.precheck ?? '').trim();
  const precheckAfter = (after.precheck ?? '').trim();
  if (precheckBefore !== precheckAfter) {
    changes.push({ field: 'precheck', text: `Precheck: ${precheckBefore || 'none'} → ${precheckAfter || 'none'}` });
  }
  const widens = modeRank(after.permissionMode) > modeRank(before.permissionMode)
    || (before.allowedTools.length > 0 && (after.allowedTools.length === 0 || gained(before.allowedTools, after.allowedTools)))
    || before.disallowedTools.some(t => !after.disallowedTools.includes(t))
    || (before.cwd ?? null) !== (after.cwd ?? null)
    || (before.agent ?? null) !== (after.agent ?? null)
    || gained(before.triggers, after.triggers)
    // Gaining or changing an owner Mac (Rust: an observed `runs_on` that
    // differs): the Macs hold different logins.
    || (ownerAfter !== null && ownerAfter !== ownerBefore)
    // Setting or changing a precheck runs a command as the user (Rust: an
    // observed `precheck` that differs). Clearing one is not widening.
    || (precheckAfter !== '' && precheckAfter !== precheckBefore);
  return { changes, widens };
}

/** The confirmation body for an editor save that widens access. */
export function stampedConfirmBody(changes: StampedChanges): string {
  return [
    ...changes.changes.map(c => c.text),
    '',
    'Saving approves these settings: scheduled and triggered runs use them from now on, without asking again.',
  ].join('\n');
}
