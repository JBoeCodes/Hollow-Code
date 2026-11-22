/**
 * Usage labels (Paperclip PR 6, gate D2). Pure: no store or Tauri import, so
 * the node vitest suite loads it and PR 8's budgets reuse it.
 *
 * Two rules every label here keeps:
 * - Never "$0" or "$0.00". No figure is not a free run: a missing or zero
 *   cost hides the chip, and anything under half a cent reads "<$0.01".
 * - A cost Claude Code reports on a subscription login (or a login Hollow
 *   could not classify) is a list-price estimate, and says so: "≈$x est.".
 */
import type { TurnUsage, UsageBucket } from './types';

/** 1234 → "1.2k", 3_400_000 → "3.4M". */
export function formatTokens(n: number): string {
  if (!Number.isFinite(n) || n < 0) return '0';
  if (n >= 1_000_000) return `${trimFixed(n / 1_000_000)}M`;
  if (n >= 1_000) return `${trimFixed(n / 1_000)}k`;
  return String(Math.round(n));
}

function trimFixed(value: number): string {
  return value.toFixed(1).replace(/\.0$/, '');
}

/** A cost worth showing, or null (missing, zero, negative or not a number). */
export function formatUsd(n: number | null | undefined): string | null {
  if (n == null || !Number.isFinite(n) || n <= 0) return null;
  if (n < 0.005) return '<$0.01';
  return `$${n.toFixed(2)}`;
}

function turnTokens(u: TurnUsage): number | null {
  const parts = [u.inputTokens, u.cachedInputTokens, u.outputTokens];
  if (parts.every(part => part == null)) return null;
  const total = parts.reduce<number>((sum, part) => sum + (part ?? 0), 0);
  return total > 0 ? total : null;
}

export interface CostChip {
  label: string;
  title: string;
}

/** The chat composer's chip for the last finished turn, or null to hide it. */
export function costChip(u: TurnUsage | null | undefined, agent = ''): CostChip | null {
  if (!u) return null;
  const tokens = turnTokens(u);
  const cost = formatUsd(u.costUsd);
  switch (u.costStatus) {
    case 'reported':
      return cost ? { label: cost, title: 'Cost reported by the agent for the last turn.' } : null;
    case 'estimated':
      if (!cost) return null;
      return {
        label: `≈${cost} est.`,
        title: agent === 'codex'
          ? "Hollow's estimate from your price for the model, for the last turn. It may not be what your plan charges."
          : agent !== 'claude' ? "The agent's estimate for the last turn. It may not be what your plan charges."
          : u.billing === 'subscription'
          ? "Claude Code's estimate at API list prices for the last turn. It may not be what your plan charges."
          : "Claude Code's estimate at API list prices for the last turn. Hollow couldn't tell how this account is billed.",
      };
    case 'unpriced':
      // A self-hosted model (Q8): its cost is GPU time, unknown here, never $0.
      if (u.billing === 'self-hosted') {
        return tokens != null
          ? {
            label: `${formatTokens(tokens)} tokens`,
            title: `Billed by GPU time on ${u.endpoint || 'your self-hosted model'}, not per token, so this turn has no price.`,
          }
          : null;
      }
      return tokens != null
        ? { label: `${formatTokens(tokens)} tokens`, title: 'No price was reported for this turn.' }
        : null;
    case 'partial':
      return tokens != null
        ? {
          label: `${formatTokens(tokens)} tokens (partial)`,
          title: "Totals are incomplete: the turn stopped early, or a resumed turn's totals could not be verified."
            // A self-hosted model (Q8): no price either way.
            + (u.billing === 'self-hosted' ? ` Billed by GPU time on ${u.endpoint || 'your self-hosted model'}, not per token.` : ''),
        }
        : null;
    default:
      return null;
  }
}

/** A night item's recorded cost (Runbooks' last night): a self-hosted
 *  model's part is billed by GPU time (Q8), never $0. */
export function nightItemCost(item: { costUsd?: number | null; gpuTime?: boolean | null }): string {
  const amount = formatUsd(item.costUsd);
  if (item.gpuTime) return amount ? `Recorded cost ${amount}, plus GPU time` : 'Billed by GPU time';
  return amount ? `Recorded cost ${amount}` : 'Cost not reported';
}

/** A bucket's cost: reported and estimated kept apart, or an em dash. */
export function bucketCost(b: Pick<UsageBucket, 'reportedCostUsd' | 'estimatedCostUsd' | 'gpuTime'>): string {
  const parts: string[] = [];
  const reported = formatUsd(b.reportedCostUsd);
  const estimated = formatUsd(b.estimatedCostUsd);
  if (reported) parts.push(`${reported} reported`);
  if (estimated) parts.push(`≈${estimated} estimated`);
  // A self-hosted model (Q8): its runs have no price, never $0.
  if (b.gpuTime) parts.push(parts.length ? 'plus GPU time' : 'Billed by GPU time');
  return parts.length ? parts.join(' · ') : '—';
}

/** Settings, Usage labels for each launch surface. */
export const SURFACE_LABELS: Record<string, string> = {
  chat: 'Chat',
  workflow: 'Automations',
  heartbeat: 'Heartbeat',
  oneshot: 'Assistants',
  delegate: 'Teammates',
  ingest: 'Imports',
};
