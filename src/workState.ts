/**
 * Shared state vocabulary (next-level P01/P04/P05/P20). One word per state,
 * the same everywhere: the rail, a conversation, Results, Your work and a
 * build's delivery record. The audit's rule is that created, checked,
 * accepted, published and installed mean different things, so each family
 * stays separate and nothing maps one onto another. Pure, unit-tested.
 */

export type Tone = 'ok' | 'warn' | 'error' | 'busy' | 'neutral' | 'off' | 'accent';

export interface StateWord { label: string; tone: Tone; /** One sentence for a tooltip or screen reader. */ hint: string }

// ── Runs: what an agent turn is doing right now.
export type RunStage = 'queued' | 'working' | 'waiting' | 'retrying' | 'stopped' | 'interrupted' | 'failed' | 'finished';
export const RUN_STAGE: Record<RunStage, StateWord> = {
  queued: { label: 'Queued', tone: 'neutral', hint: 'Waiting for a free slot; it starts on its own.' },
  working: { label: 'Working', tone: 'busy', hint: 'The agent is running.' },
  waiting: { label: 'Waiting for you', tone: 'warn', hint: 'A question or approval needs your answer before it can continue.' },
  retrying: { label: 'Retrying', tone: 'busy', hint: 'The last attempt failed and Hollow is trying again.' },
  stopped: { label: 'Stopped', tone: 'off', hint: 'You stopped this run.' },
  interrupted: { label: 'Interrupted', tone: 'warn', hint: 'The run ended before it finished (quit, restart or crash). Continue picks it up.' },
  failed: { label: 'Failed', tone: 'error', hint: 'The run ended with an error.' },
  finished: { label: 'Finished', tone: 'neutral', hint: 'The run finished. Finished is not the same as checked or accepted.' },
};

/** A run's stage from the evidence we have. `status` is a receipt/turn status
 *  (`success`, `error`, `cancelled`, `interrupted`, …). Live signals win. */
export function runStage(input: { running?: boolean; queued?: boolean; waiting?: boolean; retrying?: boolean; status?: string | null; stoppedByUser?: boolean }): RunStage | null {
  if (input.waiting) return 'waiting';
  if (input.retrying) return 'retrying';
  if (input.running) return 'working';
  if (input.queued) return 'queued';
  const s = (input.status ?? '').toLowerCase();
  if (!s) return null;
  if (s === 'cancelled' || s === 'canceled' || s === 'stopped') return input.stoppedByUser === false ? 'interrupted' : 'stopped';
  if (s === 'interrupted' || s === 'abandoned' || s === 'orphaned') return 'interrupted';
  if (s === 'error' || s === 'failed' || s === 'failure') return 'failed';
  if (s === 'success' || s === 'ok' || s === 'done' || s === 'finished') return 'finished';
  return null;
}

// ── Checks: what verification recorded. Missing checks are never zero failures.
export type CheckState = 'passed' | 'failed' | 'partial' | 'notChecked';
export const CHECK_STATE: Record<CheckState, StateWord> = {
  passed: { label: 'Checks passed', tone: 'ok', hint: 'Every recorded check passed.' },
  failed: { label: 'Checks failed', tone: 'error', hint: 'At least one recorded check failed.' },
  partial: { label: 'Partly checked', tone: 'warn', hint: 'Some checks ran; others have no result.' },
  notChecked: { label: 'Not checked', tone: 'off', hint: 'No check results were recorded. That is not the same as passing.' },
};

export function checkState(input: { failed?: number | null; total?: number | null; unknown?: number | null } | null | undefined): CheckState {
  if (!input || input.total == null || input.total === 0) return 'notChecked';
  if ((input.failed ?? 0) > 0) return 'failed';
  if ((input.unknown ?? 0) > 0) return 'partial';
  return 'passed';
}

/** Checks in words, one phrase for Overview, the recap, Activity, Results
 *  and its cards (P1-50): "1 of 3 checks failed", "2 checks passed". */
export function checkPhrase(input: Parameters<typeof checkState>[0]): string {
  const state = checkState(input);
  if (state === 'notChecked' || !input) return CHECK_STATE.notChecked.label;
  const total = input.total!, failed = input.failed ?? 0, unknown = Math.max(0, Math.min(input.unknown ?? 0, total - failed));
  const checks = `${total} check${total === 1 ? '' : 's'}`;
  const open = unknown ? `, ${unknown} without a result` : '';
  if (state === 'failed') return `${failed} of ${checks} failed${open}`;
  if (state === 'partial') return `${total - unknown} of ${checks} passed${open}`;
  return `${checks} passed`;
}

// ── Review: the user's own verdict on a result. Independent of checks.
export type ReviewState = 'draft' | 'ready' | 'accepted' | 'changed';
export const REVIEW_STATE: Record<ReviewState, StateWord> = {
  draft: { label: 'Draft', tone: 'neutral', hint: 'Not marked ready for review.' },
  ready: { label: 'Ready for review', tone: 'accent', hint: 'Marked ready; nobody has accepted it yet.' },
  accepted: { label: 'Accepted', tone: 'ok', hint: 'You accepted this exact version.' },
  changed: { label: 'Changed since accepted', tone: 'warn', hint: 'The file changed after you accepted it. The new version needs its own review.' },
};

// ── Delivery: where a build's change has got to. Each step needs its own evidence.
export type DeliveryStep = 'local' | 'committed' | 'published' | 'merged' | 'built' | 'scheduled' | 'installed';
export const DELIVERY_STEP: Record<DeliveryStep, StateWord> = {
  local: { label: 'Local changes', tone: 'neutral', hint: 'Changed in the worktree, not committed.' },
  committed: { label: 'Committed', tone: 'neutral', hint: 'Committed on the branch, not pushed.' },
  published: { label: 'Pull request open', tone: 'accent', hint: 'Pushed with an open pull request.' },
  merged: { label: 'Merged', tone: 'ok', hint: 'The pull request is merged.' },
  built: { label: 'Built', tone: 'ok', hint: 'An app was built from this revision; it is not installed.' },
  scheduled: { label: 'Install scheduled', tone: 'warn', hint: 'The build waits in the Update button until chats finish. It is pending until the restarted app confirms the install.' },
  installed: { label: 'Installed', tone: 'ok', hint: 'The installer and the running app both confirm this revision.' },
};

// ── Designed screen states (P01): one look for every page's non-ready state.
export type SurfaceState = 'loading' | 'empty' | 'filtered' | 'offline' | 'partial' | 'blocked' | 'failed' | 'conflicted' | 'stale';
