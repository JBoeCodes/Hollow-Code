import type { RunnerGate, RunnerOwner, RunnerStatus } from './types';
import { fmtTime } from './utils';

/** The remote runner's state in words (overnight N1, 2026-11-02): Settings
 *  → Devices → Remote runner shows it (RunnerStatusBlock.tsx). */

/** Who the runner belongs to, in words (remote runners 2, 2026-11-03). */
export function ownerSentence(owner: RunnerOwner): string {
  const day = owner.recordedAt ? owner.recordedAt.slice(0, 10) : '';
  const keys = owner.keys == null ? '' : ` ${owner.keys} SSH ${owner.keys === 1 ? 'key opens' : 'keys open'} its runner account.`;
  switch (owner.state) {
    case 'you': return `Owner: this Hollow${day ? ` (recorded ${day})` : ''}.${keys}`;
    case 'other': return `Owner: another Hollow${day ? ` (recorded ${day})` : ''}.${keys}`;
    default: return `No owner recorded there.${keys}`;
  }
}

/** A check older than this is made again when Settings opens. */
export const RUNNER_STATUS_STALE_MS = 5 * 60_000;

export function runnerStateLabel(state: RunnerStatus['state']): string {
  switch (state) {
    case 'ready': return 'Answering';
    case 'refusing': return 'Not taking work';
    case 'unreachable': return 'Not answering';
    case 'error': return 'Check failed';
    case 'unchecked': return 'Not checked yet';
    default: return 'No runner set up';
  }
}

/** Whether the last answer is missing or old enough to check again. */
export function runnerStatusStale(status: RunnerStatus, now = Date.now()): boolean {
  if (!status.host) return false;
  if (!status.checkedAt) return true;
  const at = Date.parse(status.checkedAt);
  return !Number.isFinite(at) || now - at > RUNNER_STATUS_STALE_MS;
}

const FACT_LABELS: Record<string, string> = {
  free: 'Free disk',
  mem: 'Memory free',
  rust: 'Rust',
  node: 'Node',
  browser: 'Browser',
};

/** The preflight line's `key=value` pairs as labelled facts, in its order
 *  (`host` is the machine's name, shown on its own). A line with none is
 *  shown whole. */
export function preflightFacts(line: string | null): string[] {
  if (!line) return [];
  const facts: string[] = [];
  for (const token of line.split(/\s+/)) {
    const eq = token.indexOf('=');
    if (eq <= 0) continue;
    const key = token.slice(0, eq);
    let value = token.slice(eq + 1).replace(/[,;]$/, '');
    if (!value || key === 'host') continue;
    if (key === 'free') value = value.replace(/^(\d+(?:\.\d+)?)([KMGT]i?B)$/, '$1 $2');
    facts.push(`${FACT_LABELS[key] ?? key} ${value}`);
  }
  return facts.length ? facts : [line];
}

function duration(seconds: number | null): string {
  if (seconds === null || !Number.isFinite(seconds)) return '';
  const whole = Math.round(seconds);
  if (whole < 60) return `${whole} s`;
  const min = Math.floor(whole / 60), sec = whole % 60;
  return min < 10 && sec ? `${min} min ${sec} s` : `${Math.round(whole / 60)} min`;
}

/** One sentence on the runner's newest gate run. */
export function gateSentence(gate: RunnerGate | null, running: boolean): string {
  if (!gate) return running ? 'A gate is running there now.' : 'No gate has run there yet.';
  const what = [gate.checkout, gate.commit ? `at ${gate.commit.slice(0, 8)}` : ''].filter(Boolean).join(' ');
  const forWhat = what ? ` for ${what}` : '';
  if (!gate.end) {
    return running
      ? `A gate is running there now${forWhat}, started ${fmtTime(gate.start)}.`
      : `Its last gate${forWhat} started ${fmtTime(gate.start)} and didn't finish (stopped before it could).`;
  }
  const took = duration(gate.seconds);
  const verdict = gate.ok
    ? `passed ${fmtTime(gate.end)}${took ? ` in ${took}` : ''}`
    : `failed ${fmtTime(gate.end)}${gate.failedStep ? ` at ${gate.failedStep}` : ''}`;
  const now = running ? ' Another gate is running there now.' : '';
  return `Last gate there${forWhat}: ${verdict}.${now}`;
}

const CAP_LABELS: Record<string, string> = {
  rust: 'Rust',
  node: 'Node',
  uv: 'uv',
  python3: 'Python',
  postgres: 'Postgres',
  playwright: 'Playwright',
};

/** Smart routing 3: the runner's tools from its last report
 *  (`rust=1.96.0 … postgres= … slots=0/2 cargo=free`) in one sentence, what
 *  it has then what it lacks. gcloud, slots and the Rust lock are left out:
 *  gcloud has no sign-in there whatever its report says. */
export function capsSentence(line: string): string {
  const has: string[] = [];
  const lacks: string[] = [];
  for (const token of line.split(/\s+/)) {
    const eq = token.indexOf('=');
    if (eq <= 0) continue;
    const key = token.slice(0, eq);
    const label = CAP_LABELS[key];
    if (!label) continue;
    const value = token.slice(eq + 1);
    if (value) has.push(`${label} ${value}`);
    else lacks.push(label);
  }
  if (!has.length && !lacks.length) return line;
  const parts = [has.length ? `Has ${has.join(', ')}` : 'Has none of the tools Hollow looks for'];
  if (lacks.length) parts.push(`no ${lacks.join(' or ')}, so commands needing ${lacks.length === 1 ? 'it' : 'them'} run here`);
  return `${parts.join('; ')}.`;
}

/** Where a routing decision ran, in words: the runner set up here
 *  (`runner`), else the host it named (`--on`, an earlier runner). */
export function routeWhere(where: string, runner?: string | null): string {
  if (where === 'here') return 'This Mac';
  return !runner || where === runner ? 'Runner' : where;
}

/** A decision's reason, led by the command when the reason doesn't already
 *  start with it ("asked for with --remote" names no command). */
export function routeReason(label: string, why: string): string {
  return !label || why.startsWith(label) ? why : `${label}: ${why}`;
}

/** When it was checked and how long that took. */
export function checkedSentence(status: RunnerStatus): string {
  if (!status.checkedAt) return '';
  const took = status.millis === null ? '' : ` (${(status.millis / 1000).toFixed(1)} s)`;
  return `checked ${fmtTime(status.checkedAt)}${took}`;
}
