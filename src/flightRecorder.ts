/** Flight recorder (master plan #15): a turn's steps (`run_steps`, the F6
 *  step spine) as a timeline. Pure, so the ordering and nesting are tested
 *  without the app. */

export type StepKind = 'model' | 'tool' | 'delegate' | 'write';

export interface StepFile { path: string; hash?: string | null; size?: number | null; state: string }

export interface RunStep {
  sessionKey: string;
  turnOrd: number;
  seq: number;
  runId: string | null;
  kind: StepKind;
  name: string | null;
  toolId: string | null;
  parentSeq: number | null;
  startedAt: string;
  endedAt: string | null;
  status: string;
  files: StepFile[];
  checkpoint: { tool?: string; input?: string; result?: string; status?: string } | null;
}

export interface StepsView {
  sessionKey: string;
  turn: number;
  runId: string | null;
  live: boolean;
  steps: RunStep[];
  firstError: number | null;
  fedBy: number[];
  accounts: string[];
}

export interface TimelineRow {
  step: RunStep;
  depth: number;
  label: string;
  duration: string;
  firstError: boolean;
  fedTheError: boolean;
}

/** The rows in timeline order: each step after its parent, children by
 *  sequence, one level deeper per delegate. A step whose parent is missing
 *  shows at the top level; a cycle can't loop. */
export function timelineRows(view: Pick<StepsView, 'steps' | 'firstError' | 'fedBy'>): TimelineRow[] {
  const bySeq = new Map(view.steps.map((s) => [s.seq, s]));
  const children = new Map<number | null, RunStep[]>();
  for (const s of view.steps) {
    const parent = s.parentSeq != null && bySeq.has(s.parentSeq) && s.parentSeq < s.seq ? s.parentSeq : null;
    const list = children.get(parent) ?? [];
    list.push(s);
    children.set(parent, list);
  }
  for (const list of children.values()) list.sort((a, b) => a.seq - b.seq);
  const fed = new Set(view.fedBy);
  const out: TimelineRow[] = [];
  const walk = (parent: number | null, depth: number) => {
    for (const s of children.get(parent) ?? []) {
      out.push({
        step: s,
        depth,
        label: stepLabel(s),
        duration: stepDuration(s),
        firstError: view.firstError === s.seq,
        fedTheError: fed.has(s.seq),
      });
      walk(s.seq, depth + 1);
    }
  };
  walk(null, 0);
  return out;
}

export function stepLabel(s: Pick<RunStep, 'kind' | 'name' | 'files'>): string {
  switch (s.kind) {
    case 'model': return 'Model';
    case 'delegate': return `Delegate · ${s.name ?? 'teammate'}`;
    case 'write': {
      const paths = s.files.map((f) => f.path.split('/').pop() ?? f.path);
      return `Write · ${paths.length ? paths.join(', ') : s.name ?? 'file'}`;
    }
    default: return `Tool · ${s.name ?? 'tool'}`;
  }
}

export function stepDuration(s: Pick<RunStep, 'startedAt' | 'endedAt'>): string {
  if (!s.endedAt) return '—';
  const ms = Date.parse(s.endedAt) - Date.parse(s.startedAt);
  if (!Number.isFinite(ms) || ms < 0) return '—';
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`;
}

/** Status words a person reads; an error-ish status is flagged. */
export function stepStatus(status: string): { text: string; bad: boolean } {
  switch (status) {
    case 'ok': return { text: 'done', bad: false };
    case 'running': return { text: 'running', bad: false };
    case 'unconfirmed': return { text: 'no result recorded', bad: false };
    case 'interrupted': return { text: 'interrupted', bad: true };
    case 'cancelled': return { text: 'stopped', bad: true };
    case 'paused': return { text: 'paused for requested work', bad: false };
    case 'error': return { text: 'failed', bad: true };
    default: return { text: status, bad: status !== 'success' };
  }
}
