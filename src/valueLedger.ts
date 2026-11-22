/** AI value ledger v1 (master plan #6): the shapes `value_summary` and
 *  `get_baselines` return, and how Settings, Usage, Value words them. */

export const WORK_KINDS = ['research', 'document', 'code', 'data', 'admin', 'meeting'] as const;
export type WorkKind = (typeof WORK_KINDS)[number];

export interface ValueTotals {
  runs: number;
  accepted: number;
  costUsd: number;
  unknownCostRuns: number;
  savedMinutes: number;
  savedHoursEstimate: number;
  /** Accepted runs whose work kind has no baseline (overnight A12). */
  unbaselinedAccepted?: number;
  /** Accepted runs of unknown work kind: no baseline can be set for them. */
  unknownKindAccepted?: number;
}

/** G4: whether hours saved show at all (only once a run is accepted), how
 *  many runs they come from, how each was accepted and the estimate's basis. */
export interface HoursSaved {
  shown: boolean;
  acceptedRuns: number;
  byIssueDone: number;
  byAcceptedResult: number;
  hoursEstimate: number | null;
  basis: string;
}

export interface ValueSummary {
  month: string;
  totals: ValueTotals;
  groups: { key: Record<string, string>; totals: ValueTotals }[];
  hoursSaved?: HoursSaved;
}

/** G4: hours saved stay hidden until some run is accepted (an accepted
 *  Result, or its tracker issue reached Done after the run started). */
export function hoursShown(t: ValueTotals): boolean {
  return t.accepted > 0;
}

export interface Baseline { minutes: number; note?: string; setAt: string }
export interface Baselines {
  schemaVersion: number;
  baselines: Partial<Record<WorkKind, Baseline>>;
  runOverrides: Record<string, { minutes: number; setAt: string }>;
}

/** The cost line: known dollars, and the runs with no cost said apart,
 *  never folded in as $0. With no known dollars beside them, only the
 *  unknown runs show: "$0.00 + …" read as a price (overnight S5). */
export function costLine(t: ValueTotals): string {
  const known = `$${t.costUsd.toFixed(2)}`;
  if (t.unknownCostRuns === 0) return known;
  const unknown = `${t.unknownCostRuns} run${t.unknownCostRuns === 1 ? '' : 's'} with unknown cost`;
  // Under half a cent rounds to "$0.00": shown only with the unknown runs.
  return t.costUsd >= 0.005 ? `${known} + ${unknown}` : unknown;
}

/** Hours saved, always labelled an estimate with its count and basis;
 *  null (hidden) until a run is accepted (G4); with no baseline set, says
 *  so, and never shows 0 h for accepted work that has none (overnight A12). */
export function savedLine(t: ValueTotals, anyBaseline: boolean): string | null {
  if (!hoursShown(t)) return null;
  const runs = `${t.accepted} accepted run${t.accepted === 1 ? '' : 's'}`;
  if (!anyBaseline) return `Set a baseline below to estimate hours saved (${runs})`;
  const unset = t.unbaselinedAccepted ?? 0;
  const unknown = t.unknownKindAccepted ?? 0;
  if (unset + unknown === t.accepted) return `Hours saved: baseline not set for this work (${runs})`;
  const missing = [unset > 0 ? `${unset} without a baseline` : '', unknown > 0 ? `${unknown} of unknown kind` : ''].filter(Boolean).map(m => `; ${m}`).join('');
  return `~${t.savedHoursEstimate.toFixed(1)} h saved from ${runs} (estimate from your baselines${missing})`;
}

/** One row's "Saved (estimate)" cell: "baseline not set" rather than 0 h
 *  when every accepted run in it lacks one. */
export function savedCell(t: ValueTotals): string {
  const unset = t.unbaselinedAccepted ?? 0;
  const unknown = t.unknownKindAccepted ?? 0;
  if (t.accepted > 0 && unknown === t.accepted) return 'work kind unknown';
  if (t.accepted > 0 && unset + unknown === t.accepted) return 'baseline not set';
  const hours = `~${t.savedHoursEstimate.toFixed(1)} h`;
  return unset + unknown > 0 ? `${hours} + ${unset + unknown} not set` : hours;
}

/** The editor's column beside a baseline: when it was set. */
export function baselineSetOn(b: { setAt: string } | undefined): string {
  return b ? `set ${b.setAt.slice(0, 10)}` : 'not set';
}
