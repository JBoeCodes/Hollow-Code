/**
 * Settings, Usage's agent tabs (Settings PR 8): the words under Totals.
 * Pure. Costs follow `usageFormat` (reported and estimated never mixed,
 * never "$0" for a missing cost).
 */
import type { UsageBucket } from '../types';
import { formatUsd } from '../usageFormat';

export function allTokens(b: UsageBucket): number {
  return b.inputTokens + b.cachedInputTokens + b.outputTokens;
}

/** Cached input as a share of all input, "62%", or null with no input. */
export function cacheShare(b: UsageBucket): string | null {
  const input = b.inputTokens + b.cachedInputTokens;
  return input > 0 ? `${Math.round((b.cachedInputTokens / input) * 100)}%` : null;
}

/** Which kind of cost the bucket holds. */
export function costKind(b: UsageBucket): 'reported' | 'estimated' | 'both' | 'none' {
  const r = b.reportedCostUsd != null, e = b.estimatedCostUsd != null;
  return r && e ? 'both' : r ? 'reported' : e ? 'estimated' : 'none';
}

/** Known cost per metered run, or null when nothing is known. */
export function averageCost(b: UsageBucket): string | null {
  const known = (b.reportedCostUsd ?? 0) + (b.estimatedCostUsd ?? 0);
  const runs = b.metered + b.partial;
  return known > 0 && runs > 0 ? formatUsd(known / runs) : null;
}

/** The counts that are not plain metered runs, only when present (the
 *  words Usage has used since Paperclip PR 6). */
export function notMetered(b: UsageBucket): string[] {
  const out: string[] = [];
  if (b.partial > 0) out.push(`${b.partial} partial (incomplete totals)`);
  if (b.unknown > 0) out.push(`${b.unknown} with no usable usage figures`);
  if (b.notStarted > 0) out.push(`${b.notStarted} didn't start`);
  if (b.inProgress > 0) out.push(`${b.inProgress} in progress`);
  if (b.unpriced > 0) out.push(`${b.unpriced} with tokens but no price`);
  if (b.gpuTime) out.push(`${b.gpuTime} billed by GPU time (self-hosted)`);
  return out;
}

/** The one line an agent tab adds under Totals. */
export function agentNote(agent: string | null | undefined): string | null {
  if (agent === 'antigravity') return 'Antigravity reports no tokens or cost: its tab counts runs and outcomes only.';
  if (agent === 'self-hosted') return 'A self-hosted model is billed by its GPU time, not per token: its runs have tokens and no cost.';
  if (agent === 'codex') return 'Codex reports tokens but no price: with its model in System, Advanced, Prices, its cost is estimated.';
  if (agent === 'claude') return 'On a subscription login Claude Code reports a cost at API list prices: an estimate, not a bill.';
  return null;
}

/** Agents whose CLIs report no plan limits (Limits tab). */
export const NO_PLAN_LIMITS = ['Grok', 'Antigravity'] as const;

/** The summary a tab shows: only one read for that tab's agent (overnight
 *  11-13 S3). While a new agent tab loads, `latest` still holds the last
 *  tab's summary, which must not show under the new tab's name. Another
 *  period for the same agent keeps showing until the new one comes. */
export function figuresFor<T extends { agent?: string | null }>(summary: T | undefined, agent: string | null): T | undefined {
  return summary && (summary.agent ?? null) === agent ? summary : undefined;
}
