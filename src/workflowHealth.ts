// Pure helpers for failure-streak cards (Paperclip PR 9, 9a-2): Automations →
// Failing automations. No store or Tauri import, so the node vitest suite can
// load it.
import type { ReviewInboxItem, Workflow } from './types';
import { STREAK_ACTION_SCHEME, streakActionSlug } from './streakAction';

// Re-exported from a tiny module so the launch path (store/runtime) doesn't
// pull the rest of this file into the App chunk (overnight P3).
export { STREAK_ACTION_SCHEME, streakActionSlug };

/** Mirrors Rust `workflow_hygiene::STREAK_KIND`. */
export const STREAK_KIND = 'workflow_failure_streak';

/** Payload of a `workflow_failure_streak` card (Rust
 *  `workflow_hygiene::record_outcome`). */
export interface StreakPayload {
  slug: string;
  title: string;
  count: number;
  firstFailedAt: string;
  lastError: string;
  /** The schedule when the card opened; stale after a routine toggle, so
   *  Pause reads the live workflow instead ({@link canPauseStreak}). */
  schedule: string;
  triggers: string[];
}

export function isStreakItem(item: Pick<ReviewInboxItem, 'kind'>): boolean {
  return item.kind === STREAK_KIND;
}

/** Strict parse: null unless every field has its wire type and the slug and
 *  title are non-empty and the count is a positive integer. */
export function streakPayload(item: Pick<ReviewInboxItem, 'kind' | 'payload'>): StreakPayload | null {
  if (!isStreakItem(item)) return null;
  const p = item.payload as Record<string, unknown> | null | undefined;
  if (!p || typeof p !== 'object') return null;
  const { slug, title, count, firstFailedAt, lastError, schedule, triggers } = p;
  if (typeof slug !== 'string' || !slug) return null;
  if (typeof title !== 'string' || !title) return null;
  if (typeof count !== 'number' || !Number.isInteger(count) || count < 1) return null;
  if (typeof firstFailedAt !== 'string' || typeof lastError !== 'string') return null;
  if (typeof schedule !== 'string') return null;
  if (!Array.isArray(triggers) || !triggers.every(t => typeof t === 'string')) return null;
  return { slug, title, count, firstFailedAt, lastError, schedule, triggers };
}

/** The card's one-line summary: `Failed N times in a row · <last error>`. */
export function streakSummary(payload: StreakPayload | null): string {
  if (!payload) return 'This automation keeps failing.';
  const base = `Failed ${payload.count} times in a row`;
  const error = payload.lastError.trim();
  return error ? `${base} · ${error}` : base;
}


/** Whether Pause schedule has anything to do: the LIVE workflow (from the
 *  workflows list, not the card's enqueue-time payload) still has a
 *  schedule. A missing workflow cannot be paused either. */
export function canPauseStreak(payload: StreakPayload | null, workflows: readonly Workflow[] | undefined): boolean {
  if (!payload) return false;
  const live = workflows?.find(w => w.slug === payload.slug);
  if (!live) return false;
  return live.schedule.trim() !== '' && live.schedule !== 'manual';
}

// ── Notification policy (the editor's `notify` select) ─────────────────────

/** Mirrors Rust `vault_workflows::NOTIFY_VALUES`, in select order. */
export const NOTIFY_OPTIONS = ['always', 'failures', 'never'] as const;
export type NotifyOption = typeof NOTIFY_OPTIONS[number];

export const NOTIFY_LABELS: Record<NotifyOption, string> = {
  always: 'Every run',
  failures: 'Only failures',
  never: 'Never',
};

/** The select value for a file's `notify`: the backend reads null, an empty
 *  value or anything unrecognised as `always` (trimmed, case-insensitive). */
export function notifySelectValue(raw: string | null | undefined): NotifyOption {
  const value = (raw ?? '').trim().toLowerCase();
  return (NOTIFY_OPTIONS as readonly string[]).includes(value) ? value as NotifyOption : 'always';
}

/** What a save sends as `WorkflowInput.notify`: only a value the user
 *  changed. Sending the hydrated value back would turn a hand-typed
 *  unrecognised value into a removal on every unrelated save. */
export function notifyForSave(current: NotifyOption, hydrated: NotifyOption): NotifyOption | undefined {
  return current === hydrated ? undefined : current;
}

/** The live run log's first line: the run's source and reason when the
 *  backend sent them (`Run started · trigger: file changed: notes/a.md`). */
export function startedLogText(source: string | undefined, reason: string | undefined): string {
  const origin = (source ?? '').trim();
  const why = (reason ?? '').trim();
  if (!origin) return 'Run started';
  if (!why || (origin === 'manual' && why === 'run now') || (origin === 'cron' && why === 'scheduled')) {
    return `Run started · ${origin === 'manual' ? 'run now' : origin === 'cron' ? 'scheduled' : origin}`;
  }
  return `Run started · ${origin}: ${why}`;
}

/** The live run log's line for a run that moved off its agent's budget
 *  ceiling (merged plan decision 6), so the move shows while the run is live
 *  and not only on the receipt: `Moved to Codex · Claude usage reached …`.
 *  Null when the run kept its own agent. */
export function switchedLogText(
  switched: { from: string; to: string; reason: string } | null | undefined,
): string | null {
  if (!switched) return null;
  const reason = switched.reason.trim();
  const line = `Moved from ${switched.from} to ${switched.to} at a budget ceiling`;
  return reason ? `${line} · ${reason}` : line;
}

// ── Skip when nothing changed (Paperclip PR 9b) ────────────────────────────

/** The watch globs typed in the editor: comma- or newline-separated,
 *  trimmed, empties dropped (the file stores a comma-list). */
export function parseWatch(text: string): string[] {
  return text.split(/[,\n]/).map(glob => glob.trim()).filter(Boolean);
}

/** True when `cwd` names exactly one workspace folder (`…/workspaces/<id>`),
 *  where an empty watch defaults to `workspaces/<id>/**`. Mirrors the
 *  backend's rule on the text only; the save validates the real path. */
export function isWorkspaceCwd(cwd: string): boolean {
  return /(^|\/)workspaces\/[^/]+\/?$/.test(cwd.trim());
}

/** The watch field's placeholder for a working folder. */
export function watchPlaceholder(cwd: string): string {
  if (isWorkspaceCwd(cwd)) return 'workspaces/<id>/** (default)';
  if (cwd.trim()) return 'required: vault globs (notes/**), or absolute globs under a folder outside the vault';
  return 'required, e.g. notes/**';
}

/** What a save sends for the quiet-skip fields: only what the user changed,
 *  so an unrelated save never re-validates (or clears) a hand-set value. */
export function quietForSave(
  skip: boolean,
  hydratedSkip: boolean,
  watchText: string,
  hydratedWatch: readonly string[],
): { skipIfQuiet?: boolean; watch?: string[] } {
  const out: { skipIfQuiet?: boolean; watch?: string[] } = {};
  if (skip !== hydratedSkip) out.skipIfQuiet = skip;
  const watch = parseWatch(watchText);
  if (watch.join('\n') !== hydratedWatch.join('\n')) out.watch = watch;
  return out;
}

/** What a save sends for the precheck command (OC13): only a change, so an
 *  unrelated save never rewrites it. Undefined leaves the saved one; '' removes it. */
export function precheckForSave(current: string, hydrated: string): string | undefined {
  const next = current.trim();
  return next === hydrated.trim() ? undefined : next;
}

// ── Run review (next-level P17) ─────────────────────────────────────────────

/** The one thing to do about a failed or paused automation. `retry` points at
 *  the page's single Run now; the others open the place the fix lives. */
export type FailureFixAction = 'retry' | 'permissions' | 'budgets' | 'accounts' | 'edit' | 'none';

export interface FailureFix {
  /** What went wrong, in one plain sentence. */
  reason: string;
  /** What to do about it. */
  fix: string;
  action: FailureFixAction;
  /** A button label for actions that open somewhere; null for `retry`/`none`. */
  actionLabel: string | null;
}

// A bare "401" isn't enough (a hash, "1401ms", a connector's own 401): only a
// status-prefixed one, or the agents' own sign-in wording (Rust
// `result_kind` reads the same envelopes).
const AUTH_RE = /\b(?:api error|status|http)\W{0,3}401\b|invalid authentication|failed to authenticate|authentication_error|not logged in|please run \/login|token has expired|invalid api key/i;

/** True when the error or stderr reads as an agent sign-in failure. */
export function isAuthFailureText(error?: string | null, tail?: string | null): boolean {
  return AUTH_RE.test(`${error ?? ''}\n${tail ?? ''}`);
}

/** The actionable reason for an automation that can't run or whose latest
 *  run failed, or null when there is nothing to fix. Order: a permission
 *  hold, then a budget hold (both stop every run), then the latest failure.
 *  A budget stop is not a failure and is handled by its own notice. */
export function failureFix(input: {
  permissionHold?: Workflow['permissionHold'];
  budgetHold?: string | null;
  lastStatus: string | null;
  lastError: string | null;
  errorTail?: string | null;
  agentName?: string;
  failedCheck?: string | null;
  /** What the failed run's receipt proves it changed (`runOutcome`): an
   *  error alone never establishes `none` (plan completion L2). Unknown
   *  when not given. */
  changedAnything?: 'none' | 'some' | 'unknown';
  /** "What happened" is on the page to point at (default true). */
  outcomeShown?: boolean;
}): FailureFix | null {
  if (input.permissionHold === 'review') {
    return {
      reason: 'Its settings changed outside the automation editor, so it is paused until you review the change.',
      fix: 'Review the change in Permission changes, then approve or reject it.',
      action: 'permissions', actionLabel: 'Review permission changes',
    };
  }
  if (input.permissionHold === 'unavailable') {
    return {
      reason: 'Hollow can’t read its automation permission records, so every automation is paused.',
      fix: 'Repair or move the permission records, then restart Hollow.',
      action: 'none', actionLabel: null,
    };
  }
  if (input.budgetHold) {
    return {
      reason: input.budgetHold,
      fix: 'Raise or resume the budget in Budget pauses, or wait for its window to reset.',
      action: 'budgets', actionLabel: 'Review budget pauses',
    };
  }
  if (input.lastStatus !== 'error') return null;
  const error = (input.lastError ?? '').trim();
  const agent = input.agentName || 'the agent';
  const shown = input.outcomeShown !== false;
  if (isAuthFailureText(error, input.errorTail)) {
    // A sign-in error can arrive after the agent already did things, so
    // only the receipt decides whether anything changed (V03).
    const reason = input.changedAnything === 'none'
      ? `${agent} could not sign in. Its run record shows the run changed nothing before it stopped.`
      : input.changedAnything === 'some'
        ? `${agent} could not sign in, after the run had already changed something: see What happened below.`
        : `${agent} could not sign in. Hollow can’t tell from the error whether the run changed anything first${shown ? ': see What happened below' : ''}.`;
    return {
      reason,
      fix: shown
        ? `Sign in through ${agent}’s app or command line, then retry the way What happened suggests. Scheduled runs try again at their next time.`
        : `Sign in through ${agent}’s app or command line, then check its results before Run now. Scheduled runs try again at their next time.`,
      action: 'accounts', actionLabel: 'Open Accounts',
    };
  }
  if (/^working folder /i.test(error)) {
    return {
      reason: `The ${error.charAt(0).toLowerCase()}${error.slice(1)}${error.endsWith('.') ? '' : '.'}`,
      fix: 'Choose a folder that exists in the editor’s Inputs & scope, or leave it empty to use the vault.',
      action: 'edit', actionLabel: 'Edit inputs & scope',
    };
  }
  if (/not installed|not found on path|no such file or directory.*(claude|codex|agy|grok)/i.test(error)) {
    return {
      reason: `${agent} isn’t available on this Mac: ${error}`,
      fix: 'Install it, or choose another agent in the editor.',
      action: 'edit', actionLabel: 'Choose another agent',
    };
  }
  if (input.failedCheck) {
    return {
      reason: `A check failed: ${input.failedCheck}.`,
      fix: 'Read the run record below for what the check printed, fix the cause, then Run now.',
      action: 'retry', actionLabel: null,
    };
  }
  return {
    reason: error || 'The last run ended with an error.',
    fix: shown
      ? 'What happened below says what it already did and which retry is safe. If it fails the same way again, open the logs.'
      : 'Check its results and logs below for what it already did before you Run now.',
    action: 'retry', actionLabel: null,
  };
}

/** The latest run's stage and checks, in the shared vocabulary
 *  (`workState.RUN_STAGE` / `CHECK_STATE`) inputs. */
export function receiptCheckCounts(checks: readonly { outcome: string }[] | null | undefined): { failed: number; total: number; unknown: number } | null {
  if (!checks) return null;
  let failed = 0, unknown = 0;
  for (const c of checks) {
    if (c.outcome === 'failed') failed++;
    else if (c.outcome !== 'passed') unknown++;
  }
  return { failed, total: checks.length, unknown };
}
