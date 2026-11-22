// A run's outcome, for the automation run review (plan completion L2,
// assignment item 3): its state in one shared vocabulary, what it changed,
// the results it touched, its checks and known cost, what it did beyond its
// files (recorded outside effects) and what Hollow can't see, and which
// retry is safe. Pure and unit-tested; `components/RunOutcome.tsx` renders it.
//
// Rules the review asked for:
// - An error string never proves nothing happened: "changed nothing" needs a
//   receipt with no file, repository or outside change AND no unknown calls.
// - A retry matches the recorded effects: when something outside already
//   happened, the offered retry is the guarded one (`run_workflow` with
//   `retryOf`), which Hollow's own executors enforce; what it can't block is
//   named, never hidden.
// - Unknown cost stays unknown, never $0.
import { isOwnReceiptFile, type EvidenceRepoChangesWire, type OutsideEffectsWire } from './executionReceipt';
import type { ManifestCostRollup } from './generated/ipc';
import { CHECK_STATE, RUN_STAGE, checkState, type StateWord } from './workState';

// ── State ──────────────────────────────────────────────────────────────────

/** Where an automation, runbook or night item stands. One word each, the
 *  same on every operational surface. */
export type OpsState =
  | 'never' | 'preview' | 'queued' | 'running' | 'waiting' | 'blocked'
  | 'failed' | 'interrupted' | 'paused' | 'stopped' | 'budgetStopped' | 'readyForReview' | 'finished';

export const OPS_STATE: Record<OpsState, StateWord> = {
  never: { label: 'Never run', tone: 'off', hint: 'It has not run yet.' },
  preview: { label: 'Preview', tone: 'neutral', hint: 'Showing what a run would do. Nothing runs and nothing is spent.' },
  queued: { label: 'Queued', tone: 'neutral', hint: 'Waiting its turn: another run holds the vault, or it waits for its time. It starts on its own; nothing has happened yet.' },
  // The app-wide word for a live run (`RUN_STAGE.working`).
  running: { ...RUN_STAGE.working },
  waiting: { ...RUN_STAGE.waiting },
  blocked: { label: 'Blocked', tone: 'warn', hint: 'It can’t run until something is fixed: a permission review, a budget pause, a sign-in or a setting. The fix is shown with it.' },
  failed: { ...RUN_STAGE.failed, hint: 'The run ended with an error. What it did before that is listed with it.' },
  interrupted: { ...RUN_STAGE.interrupted, hint: 'The run ended before it finished (quit, restart or crash). What it did before that is listed with it.' },
  paused: { label: 'Paused', tone: 'warn', hint: 'Yielded for requested work. Continuing requires confirmed cleanup, usage records and current permission. Stop pending work prevents continuation.' },
  stopped: { ...RUN_STAGE.stopped },
  budgetStopped: { label: 'Stopped', tone: 'warn', hint: 'A usage budget stopped this run. That is not a failure of the automation.' },
  readyForReview: { label: 'Ready for review', tone: 'accent', hint: 'It finished and produced something to look at. Finished is not checked or accepted.' },
  finished: { label: 'Finished', tone: 'neutral', hint: 'It finished without producing a result file. Finished is not the same as checked.' },
};

/** An automation's state, from the strongest live signal down. */
export function automationState(input: {
  running?: boolean;
  waiting?: boolean;
  queued?: boolean;
  blocked?: boolean;
  lastStatus?: string | null;
  budgetStop?: boolean;
  /** The latest run wrote or touched a result someone can open. */
  produced?: boolean;
}): OpsState {
  if (input.running) return 'running';
  if (input.waiting) return 'waiting';
  if (input.queued) return 'queued';
  if (input.blocked) return 'blocked';
  switch ((input.lastStatus ?? '').toLowerCase()) {
    case '': return 'never';
    case 'running': return 'running';
    case 'error': return input.budgetStop ? 'budgetStopped' : 'failed';
    case 'cancelled': return 'stopped';
    case 'paused': return 'paused';
    case 'interrupted': return 'interrupted';
    case 'success': return input.produced ? 'readyForReview' : 'finished';
    // An unrecognised status claims nothing (the old status line showed none).
    default: return 'never';
  }
}

/** One run-history row's status in the status pill's words (P1-04): the
 *  stored `success` / `error` / `cancelled` / `interrupted` / `running`, and a
 *  budget stop says Stopped as the pill does; any other status stays as
 *  written, first letter capitalised, rather than claiming one. */
export function historyStatusLabel(status: string, budgetStop = false): string {
  const s = status.toLowerCase();
  if (['success', 'error', 'cancelled', 'paused', 'interrupted', 'running'].includes(s)) return OPS_STATE[automationState({ lastStatus: s, budgetStop })].label;
  return status.charAt(0).toUpperCase() + status.slice(1);
}

const NIGHT_OPS: Record<string, OpsState> = {
  queued: 'queued', running: 'running', waiting: 'waiting', done: 'readyForReview',
  failed: 'failed', stopped: 'stopped', refused: 'blocked', removed: 'stopped',
};
const RUNBOOK_OPS: Record<string, OpsState> = {
  running: 'running', waiting: 'waiting', passed: 'readyForReview', unverified: 'finished',
  failed: 'failed', stopped: 'stopped', interrupted: 'failed',
};

/** The shared explanation for a Tonight / Last night item's status. */
export function nightStateHint(status: string): string {
  if (status === 'removed') return 'Removed before it started: it won’t run, and nothing happened.';
  if (status === 'refused') return 'Tonight refused it: the overnight budget is $0, or it asked for more than the overnight cap. Nothing ran.';
  if (status === 'done') return 'It ran overnight and finished. Review its steps and the morning report; finished is not checked or accepted.';
  const state = NIGHT_OPS[status];
  return state ? OPS_STATE[state].hint : 'Unknown state.';
}

/** The shared explanation for a runbook run's or step's status. */
export function runbookStateHint(status: string | undefined): string {
  if (!status) return OPS_STATE.never.hint;
  if (status === 'passed') return 'Every step ran and its check passed. Review the run records before relying on it.';
  if (status === 'unverified') return 'It finished, but a step had no check to prove it worked.';
  if (status === 'interrupted') return 'It ended before finishing (quit, restart or crash). Steps marked done happened; later ones didn’t.';
  const state = RUNBOOK_OPS[status];
  return state ? OPS_STATE[state].hint : 'Unknown state.';
}

/** Where a night item's stop or refusal is fixed, in its own row (plan
 *  completion L2): the overnight cap lives in Settings, Usage, Budgets
 *  (P1-07), a sign-in in Accounts. Null when its note names no setting. */
export function nightRecovery(item: { status: string; note?: string | null }): { label: string; target: 'settingsBudgets' | 'accounts' } | null {
  if (!['refused', 'stopped', 'failed'].includes(item.status)) return null;
  const note = (item.note ?? '').toLowerCase();
  // An item that asked for more than the cap can't be fixed by the cap (it is
  // at most $40): no button rather than the wrong one.
  if (/more than the .* overnight cap/.test(note)) return null;
  if (/overnight budget is \$0|overnight stop|cost wasn.t reported/.test(note)) return { label: 'Change the overnight cap', target: 'settingsBudgets' };
  if (/\b401\b|\bsign(?:ed)? in\b|\blog(?:ged)? ?in\b|authenticat/.test(note)) return { label: 'Open Accounts', target: 'accounts' };
  return null;
}

// ── What happened ──────────────────────────────────────────────────────────

/** Shared facts, accepted directly from a wire or normalized receipt. Run
 * status and retry authority belong to the host, never to verification. */
export interface RunReceiptFacts {
  files: readonly { path: string; changeType?: string; attribution?: string }[];
  checks: readonly { outcome: string }[];
  repoChanges?: EvidenceRepoChangesWire[];
  outsideEffects?: OutsideEffectsWire | null;
  historyRecoverable?: boolean | null;
  historyRunId?: string | null;
  pullRequests?: readonly { action: string; number: number }[] | null;
}

export interface RunOutcomeInput {
  /** The reviewed run's log status (`success` | `error` | `cancelled` |
   *  `interrupted`). */
  status: string | null;
  /** That run's receipt; null when none was recorded (unknown, not none). */
  receipt: RunReceiptFacts | null;
  /** The automation's declared result files (`outputGlob`), if any. */
  outputGlob?: string | null;
  /** The run's cost roll-up; undefined while loading, null when unreadable. */
  cost?: Pick<ManifestCostRollup, 'ownUsd' | 'totalUsd' | 'knownUsd' | 'unknown' | 'gpuTime'> | null;
  /** The run failed because the agent couldn't sign in. */
  authFailure?: boolean;
  /** A permission or budget hold stops every run until it is fixed. */
  blocked?: boolean;
  /** A usage budget ended the run (not a failure). */
  budgetStop?: boolean;
}

export type EffectOutcome = 'done' | 'failed' | 'uncertain';

export interface OutcomeEffect {
  label: string;
  outcome: EffectOutcome;
  outcomeLabel: string;
  /** Hollow blocks repeating it in a guarded retry. */
  guarded: boolean;
}

export interface OutcomeUnknown {
  label: string;
  count: number;
  examples: string[];
}

export type RetryKind = 'none' | 'fixFirst' | 'safe' | 'currentFiles' | 'guarded' | 'checkFirst';

export interface RetryAdvice {
  kind: RetryKind;
  /** The retry button's label; null when no retry is offered here. */
  label: string | null;
  /** One or two sentences: why this retry, and what it won't do. */
  detail: string;
  /** Pass to `run_workflow` as `retryOf` (the guarded retry), else null. */
  retryOf: string | null;
}

/** Did the run change anything? `unknown` unless a receipt proves it. */
export type ChangedAnything = 'none' | 'some' | 'unknown';

export interface RunOutcome {
  changedAnything: ChangedAnything;
  /** "1 vault file (History can undo it) · 2 files in app", or a sentence. */
  changed: string;
  /** Result files this run wrote or changed (vault paths). */
  outputs: string[];
  /** Whether `outputs` are the declared results or just what it wrote. */
  outputsDeclared: boolean;
  checks: StateWord;
  /** "$0.42", "$0.42 known · 1 cost unknown", "Not known", "Reading…". */
  cost: string;
  effects: OutcomeEffect[];
  /** Recorded effects beyond the receipt's list. */
  moreEffects: number;
  /** Whether the receipt recorded outside effects at all (older ones
   *  didn't): an empty list then means unknown, not none. */
  outsideRecorded: boolean;
  unknown: OutcomeUnknown[];
  /** One sentence on how complete this record is. */
  coverage: string;
  retry: RetryAdvice;
}

const OUTCOME_LABELS: Record<EffectOutcome, string> = {
  done: 'done',
  failed: 'failed, nothing sent',
  uncertain: 'may have happened',
};

/** `Shell commands` → `shell command` for one (Rust `run_effects::unknown_label`). */
export function unknownLabel(label: string, count: number): string {
  const lower = label.toLowerCase();
  if (count !== 1) return lower;
  const space = lower.indexOf(' ');
  return space > 0 && lower.endsWith('s') ? lower.slice(0, -1) : lower;
}

function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}

/** `briefings/*.html`-style glob → RegExp (`*` within a segment, `**`
 *  across). Comma-separated globs match any. */
export function globMatcher(glob: string | null | undefined): ((path: string) => boolean) | null {
  const parts = (glob ?? '').split(',').map(g => g.trim()).filter(Boolean);
  if (!parts.length) return null;
  const res = parts.map(g => new RegExp(`^${g
    .replace(/[.+^${}()|[\]\\]/g, '\\$&')
    .replace(/\*\*\/?/g, '\u0000')
    .replace(/\*/g, '[^/]*')
    .replace(/\?/g, '[^/]')
    .replace(/\u0000/g, '(?:.*/)?')
    .replace(/\(\?:\.\*\/\)\?$/, '.*')}$`));
  return (path: string) => res.some(re => re.test(path));
}

/** The files a run produced: its declared results when named, else what it
 *  wrote that a person can open (not its own workflow file or dot-files). */
export function runOutputs(receipt: Pick<RunReceiptFacts, 'files'> | null, outputGlob?: string | null): { paths: string[]; declared: boolean } {
  if (!receipt) return { paths: [], declared: !!outputGlob?.trim() };
  const written = receipt.files
    .filter(f => f.changeType !== 'delete' && isOwnReceiptFile(f))
    .map(f => f.path)
    .filter(p => !p.startsWith('workflows/') && !p.split('/').some(seg => seg.startsWith('.')));
  const match = globMatcher(outputGlob);
  if (match) return { paths: written.filter(match), declared: true };
  return { paths: written, declared: false };
}

/** A roll-up's cost words when part of it ran on a self-hosted model (Q8):
 *  GPU time, never $0, with what else is known or unreported; null when none
 *  of it did. */
export function gpuCostText(cost: Pick<ManifestCostRollup, 'knownUsd' | 'unknown' | 'gpuTime'>): string | null {
  if (!cost.gpuTime) return null;
  const known = cost.knownUsd > 0 ? `$${cost.knownUsd.toFixed(2)} known, plus GPU time` : 'Billed by GPU time';
  return cost.unknown ? `${known} · ${plural(cost.unknown, 'cost')} not reported` : known;
}

/** The flight recorder's run cost, as a suffix of its step count line. */
export function runCostSuffix(cost: Pick<ManifestCostRollup, 'totalUsd' | 'knownUsd' | 'unknown' | 'gpuTime'> | null | undefined): string {
  if (cost?.totalUsd != null) return ` · run cost $${cost.totalUsd.toFixed(4)}`;
  const gpu = cost ? gpuCostText(cost) : null;
  return gpu ? ` · run cost: ${gpu.charAt(0).toLowerCase()}${gpu.slice(1)}` : ' · run cost unknown';
}

function costText(cost: RunOutcomeInput['cost']): string {
  if (cost === undefined) return 'Reading…';
  if (cost === null) return 'Not known (no usage record could be read for this run)';
  const gpu = gpuCostText(cost);
  if (gpu) return gpu;
  const total = cost.totalUsd ?? cost.ownUsd;
  if (total != null && cost.unknown === 0) return total > 0 && total < 0.005 ? 'under $0.01' : `$${total.toFixed(2)}`;
  if (cost.knownUsd > 0) return `$${cost.knownUsd.toFixed(2)} known · ${plural(cost.unknown, 'cost')} not reported`;
  return 'Not known (its agent or model reports no cost)';
}

function effectsOf(wire: OutsideEffectsWire | null | undefined): { effects: OutcomeEffect[]; unknown: OutcomeUnknown[]; more: number } {
  const effects = (wire?.recorded ?? []).map((e): OutcomeEffect => {
    const outcome: EffectOutcome = e.outcome === 'done' || e.outcome === 'failed' ? e.outcome : 'uncertain';
    return { label: e.label, outcome, outcomeLabel: OUTCOME_LABELS[outcome], guarded: !!e.key };
  });
  const unknown = (wire?.unknown ?? []).filter(u => u.count > 0).map(u => ({ label: u.label, count: u.count, examples: u.examples ?? [] }));
  return { effects, unknown, more: wire?.moreRecorded ?? 0 };
}

/** The outcome of one run. */
export function runOutcome(input: RunOutcomeInput): RunOutcome {
  const r = input.receipt;
  // Older receipts carry no `outsideEffects`: unknown, never none.
  const recorded = r?.outsideEffects != null;
  const { effects, unknown, more } = effectsOf(r?.outsideEffects);
  const happened = effects.filter(e => e.outcome !== 'failed');
  const files = r?.files.filter(isOwnReceiptFile).length ?? 0;
  const repoFiles = (r?.repoChanges ?? []).reduce((n, repo) => n + (repo.totalFiles || 0), 0);
  const repoUnrecorded = (r?.repoChanges ?? []).some(repo => repo.status !== 'captured');
  const outputs = runOutputs(r, input.outputGlob);
  const counts = r ? { failed: r.checks.filter(c => c.outcome === 'failed').length, total: r.checks.length, unknown: r.checks.filter(c => c.outcome !== 'passed' && c.outcome !== 'failed').length } : null;

  let changedAnything: ChangedAnything;
  if (!r) changedAnything = 'unknown';
  else if (files > 0 || repoFiles > 0 || happened.length > 0 || more > 0 || (r.pullRequests ?? []).length > 0) changedAnything = 'some';
  else if (!recorded || unknown.length > 0 || repoUnrecorded) changedAnything = 'unknown';
  else changedAnything = 'none';

  const parts: string[] = [];
  if (files > 0) {
    const undo = r!.historyRecoverable === false ? 'History can’t undo all of them' : files === 1 ? 'History can undo it' : 'History can undo them';
    parts.push(`${plural(files, 'vault file')} (${undo})`);
  }
  for (const repo of r?.repoChanges ?? []) {
    if (repo.totalFiles > 0) parts.push(`${plural(repo.totalFiles, 'file')} in ${repo.label}`);
    else if (repo.status !== 'captured') parts.push(`${repo.label}: not recorded`);
  }
  for (const pr of r?.pullRequests ?? []) parts.push(`${pr.action === 'merged' ? 'merged' : 'opened'} PR #${pr.number}`);
  const changed = !r
    ? 'Unknown: Hollow has no record of this run.'
    : parts.length ? parts.join(' · ')
      : 'No file changes.';

  let coverage: string;
  if (!r) coverage = 'Hollow has no record of this run, so it can’t say what it changed or did.';
  else if (!recorded) coverage = 'This run record is from before Hollow recorded what runs do beyond their files, so that part is unknown.';
  else if (unknown.length) coverage = `Hollow can’t see what ${plural(unknown.reduce((n, u) => n + u.count, 0), 'call')} did (${unknown.map(u => unknownLabel(u.label, u.count)).join(', ')}), so this record may be incomplete.`;
  else if (repoUnrecorded) coverage = 'A repository’s changes weren’t recorded, so this record may be incomplete.';
  else coverage = 'Every call it made is accounted for: file changes through History, outside actions as listed.';

  return {
    changedAnything,
    changed,
    outputs: outputs.paths,
    outputsDeclared: outputs.declared,
    checks: CHECK_STATE[checkState(counts)],
    cost: costText(input.cost),
    effects,
    moreEffects: more,
    outsideRecorded: recorded,
    unknown,
    coverage,
    retry: retryAdvice(input, { recorded, files, repoFiles, repoUnrecorded, prs: (r?.pullRequests ?? []).length, happened, unknown, more }),
  };
}

function retryAdvice(
  input: RunOutcomeInput,
  facts: { recorded: boolean; files: number; repoFiles: number; repoUnrecorded: boolean; prs: number; happened: OutcomeEffect[]; unknown: OutcomeUnknown[]; more: number },
): RetryAdvice {
  const status = (input.status ?? '').toLowerCase();
  if (status !== 'error' && status !== 'cancelled' && status !== 'interrupted') return { kind: 'none', label: null, detail: '', retryOf: null };
  if (input.budgetStop) return { kind: 'fixFirst', label: null, detail: 'Resume or raise the budget first; a run now would stop at the same limit.', retryOf: null };
  if (input.blocked) return { kind: 'fixFirst', label: null, detail: 'Fix what pauses it first; until then Run now is refused.', retryOf: null };
  const r = input.receipt;
  // The guarded retry is looked up by the History run id (`find_by_history_run`).
  const runId = r?.historyRunId || null;
  const guardedCount = facts.happened.filter(e => e.guarded).length;
  const openCount = facts.happened.length - guardedCount + facts.more;
  const signIn = input.authFailure ? 'Sign in first. Then: ' : '';
  if (!r) {
    return {
      kind: 'checkFirst', label: null, retryOf: null,
      detail: `${signIn}Hollow has no record of what this run did, so it can’t tell whether running again repeats something. Check its results and logs, then use Run now.`,
    };
  }
  if (facts.happened.length || facts.more || facts.prs) {
    // A PR the receipt lists is outside the Mac even on an older receipt.
    const things = Math.max(facts.happened.length + facts.more, facts.prs);
    const blocked = guardedCount ? `Hollow refuses the identical call${guardedCount === 1 ? '' : 's'} for ${guardedCount === 1 ? 'the one' : `the ${guardedCount}`} it runs itself` : 'Hollow can’t block any of them itself';
    const others = Math.max(openCount, things - guardedCount);
    const open = others ? `; check ${others === 1 ? 'the other one' : `the other ${others}`} before running again` : '';
    return {
      kind: 'guarded', label: 'Run again without repeating', retryOf: runId,
      detail: `${signIn}It already did ${plural(things, 'thing')} outside its files. ${blocked}${open}. The retry is told what already happened.`,
    };
  }
  if (facts.repoUnrecorded) {
    return {
      kind: 'checkFirst', label: null, retryOf: null,
      detail: `${signIn}A repository’s changes weren’t recorded, so Hollow can’t tell whether running again repeats something. Check the repository first, then use Run now.`,
    };
  }
  if (!facts.recorded) {
    return {
      kind: 'checkFirst', label: null, retryOf: null,
      detail: `${signIn}Hollow can’t tell whether running again repeats something it did. Check its results first, then use Run now.`,
    };
  }
  if (facts.unknown.length) {
    return {
      kind: 'checkFirst', label: 'Run again without repeating', retryOf: runId,
      detail: `${signIn}It ran calls Hollow can’t see into, so they may have changed something. The retry is told what they were; check them first if they could act outside this Mac.`,
    };
  }
  if (facts.files || facts.repoFiles) {
    return {
      kind: 'currentFiles', label: null, retryOf: null,
      detail: `${signIn}Nothing happened outside its files. Run now starts from the files as they are now, including what this run wrote; undo them in History first for a clean start.`,
    };
  }
  return {
    kind: 'safe', label: null, retryOf: null,
    detail: `${signIn}It changed nothing Hollow records and every call is accounted for, so Run now is safe to repeat.`,
  };
}
