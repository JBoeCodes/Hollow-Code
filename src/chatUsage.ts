/**
 * Pure helpers for the composer's usage popover (components/ChatUsageMenu.tsx):
 * context-window size, the hand-over point, plan-window rows and labels, and
 * the Context section's tool-result and cache rows. No store or Tauri import,
 * so the node vitest suite loads it.
 */
import type { ClaudeAccountsView, ClaudeQuota, ContextCacheTurn, ContextUse, TurnUsage, UsageBilling, RunManifest } from './types';
import { formatTokens } from './usageFormat';

// The quota opt-in's localStorage key lives in store/prefs.ts (QUOTA_OPT_IN_KEY):
// that is its only reader, and keeping it there keeps this module out of the
// shell chunk (2026-11-15, the bundle budget).

const ONE_MILLION = 1_000_000;

/**
 * The model's context window in tokens, or null when Hollow can't tell.
 * Claude: 1M for a `[1m]` model, else 200k (bumped to 1M when a turn already
 * used more, which only a 1M session can). A window the agent reported
 * itself (`reported`: Codex's, from its rollout, overnight 9 A10) wins.
 * Other agents don't say.
 */
/** Where Antigravity's prepaid credit balance is shown: agy doesn't report it, so
 *  the usage menu links here instead of inventing a number (2026-11-20). */
export const ANTIGRAVITY_CREDITS_URL = 'https://ai.studio/projects';

export function contextWindow(agent: string, model: string | null | undefined, used: number | null, reported?: number | null): number | null {
  if (reported != null && reported > 0) return reported;
  if (agent !== 'claude') return null;
  if ((model ?? '').toLowerCase().includes('[1m]')) return ONE_MILLION;
  if (used != null && used > 200_000) return ONE_MILLION;
  return 200_000;
}

export interface ContextSummary {
  /** "142.3k/200k", or "142.3k tokens" without a known window. */
  label: string;
  /** 0-1, or null without a known window. */
  fraction: number | null;
}

export function contextSummary(used: number, window: number | null): ContextSummary {
  if (!window) return { label: `${formatTokens(used)} tokens`, fraction: null };
  return { label: `${formatTokens(used)}/${formatTokens(window)}`, fraction: Math.min(Math.max(used / window, 0), 1) };
}

/** The hand-over point on a 1M window; mirrors `context_gauge::ADVISORY_TOKENS`. */
export const HANDOVER_TOKENS = 350_000;

/** Offer "Continue in a new conversation" (repo features R6): a known window
 *  at least 75% full, or past the hand-over point on a 1M window. */
export function handoverOffered(used: number, window: number | null): boolean {
  if (!window) return false;
  return used / window >= 0.75 || (window >= ONE_MILLION && used >= HANDOVER_TOKENS);
}

/** Shared plan label; retain overage and small fractions, round larger values. */
export function percentUsed(n: number): string {
  const shown = n >= 10 || Number.isInteger(n) ? String(Math.round(n)) : n.toFixed(1);
  return `${shown}% used`;
}

/** A plan window's level, the same in the composer and Settings (P1-27):
 *  high from 80% used, over at 100% (overage). */
export function planLevel(usedPercent: number): 'ok' | 'high' | 'over' {
  if (usedPercent >= 100) return 'over';
  if (usedPercent >= 80) return 'high';
  return 'ok';
}

/** "Resets Sep 26, 2026, 8:50 PM", or null when unknown. */
export function resetText(iso: string | null | undefined): string | null {
  if (!iso) return null;
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return null;
  const text = at.toLocaleString('en-US', {
    month: 'short', day: 'numeric', year: 'numeric', hour: 'numeric', minute: '2-digit',
  });
  return `Resets ${text}`;
}

export interface PlanWindow {
  id: string;
  label: string;
  usedPercent: number;
  resetsAt: string | null;
}

export interface PlanUsage {
  /** Whose plan: an added account's label, or null for Claude Code's login. */
  account: string | null;
  windows: PlanWindow[];
}

const WINDOW_LABELS: Record<string, string> = { five_hour: '5h limit', seven_day: '7d limit' };

/**
 * The plan windows to show. An added Claude account that new runs use wins
 * (its own probe/stream readings); otherwise the login's quota, when read.
 */
export function planUsage(accounts: ClaudeAccountsView | null, quota: ClaudeQuota | null): PlanUsage | null {
  const next = accounts?.nextAccount ? accounts.accounts.find(a => a.id === accounts.nextAccount) : undefined;
  if (next) {
    const windows: PlanWindow[] = [];
    if (next.usage?.fiveHour) windows.push({ id: 'five_hour', label: WINDOW_LABELS.five_hour, ...next.usage.fiveHour });
    if (next.usage?.sevenDay) windows.push({ id: 'seven_day', label: WINDOW_LABELS.seven_day, ...next.usage.sevenDay });
    return { account: next.label, windows };
  }
  if (!quota) return null;
  return {
    account: null,
    windows: quota.windows.map(w => ({
      id: w.id,
      label: WINDOW_LABELS[w.id] ?? w.label,
      usedPercent: w.usedPercent,
      resetsAt: w.resetsAt,
    })),
  };
}

export function billingLabel(billing: UsageBilling | null | undefined): string | null {
  switch (billing) {
    case 'subscription': return 'Subscription';
    case 'api': return 'API key';
    case 'self-hosted': return 'Billed by GPU time';
    default: return null;
  }
}

/**
 * The self-hosted endpoint a turn ran on (Q8), by name, or null for any other
 * turn. The usage popover shows it in place of the Claude account and plan,
 * which such a turn never uses.
 */
export function selfHostedEndpoint(usage: Pick<TurnUsage, 'billing' | 'endpoint'> | null | undefined): string | null {
  if (usage?.billing !== 'self-hosted') return null;
  return usage.endpoint?.trim() || 'Self-hosted model';
}

/** A chat model id naming a self-hosted entry (Q5: `self-hosted:<entry id>`). */
export const SELF_HOSTED_MODEL_PREFIX = 'self-hosted:';

/**
 * The endpoint the usage popover names (Q8 review). The last turn's usage
 * decides; while a turn runs its usage is cleared, so the conversation's
 * last self-hosted endpoint (`remembered`) stays until the next usage event,
 * unless the chat's model now names another model. A self-hosted model id
 * is known to be one before any turn.
 */
export function popoverEndpoint(
  usage: Pick<TurnUsage, 'billing' | 'endpoint'> | null | undefined,
  model: string | null | undefined,
  remembered: string | null,
): string | null {
  if (usage) return selfHostedEndpoint(usage);
  if (model?.startsWith(SELF_HOSTED_MODEL_PREFIX)) return remembered ?? 'Self-hosted model';
  return model ? null : remembered;
}

export interface TurnRow {
  label: string;
  value: string;
}

/** The last turn's token rows (only the reported ones). */
export function turnRows(u: TurnUsage | null | undefined): TurnRow[] {
  if (!u) return [];
  const rows: TurnRow[] = [];
  if (u.inputTokens != null) rows.push({ label: 'Input', value: formatTokens(u.inputTokens) });
  if (u.outputTokens != null) rows.push({ label: 'Output', value: formatTokens(u.outputTokens) });
  return rows;
}

// Repo features R6: "What's using it" and "Cache" from `conversation_context_use`.
// Token figures are estimates: bytes/4.

/** "≈12.3k tokens est." from a byte count. */
function estimate(bytes: number): string {
  return `≈${formatTokens(Math.round(bytes / 4))} tokens est.`;
}

/** `mcp__hollow_vault__read_source` → `read_source`; `other` → `Other`. */
function toolName(tool: string): string {
  return tool === 'other' ? 'Other' : tool.split('__').pop() || tool;
}

/** Tool results by tool since the agent session began, largest first. */
export function toolUseRows(u: ContextUse | null | undefined): TurnRow[] {
  return (u?.byTool ?? []).map(t => ({
    label: toolName(t.tool),
    value: `${estimate(t.bytes)} · ${t.calls} call${t.calls === 1 ? '' : 's'}`,
  }));
}

/** The largest single results with their turn and file or program. */
export function largestRows(u: ContextUse | null | undefined): TurnRow[] {
  return (u?.largest ?? []).map(l => ({
    label: `Turn ${l.turn} · ${toolName(l.tool)}${l.target ? ` · ${l.target}` : ''}`,
    value: estimate(l.bytes),
  }));
}

const CAUSES: Record<NonNullable<ContextCacheTurn['cause']>, string> = {
  newSession: 'new session',
  idle: 'idle over an hour',
  model: 'model changed',
  compacted: 'after a compaction',
  unknown: 'cause unknown',
};

/** Each recent turn's cached share (reported ones only); a cold one says why. */
export function cacheRows(u: ContextUse | null | undefined): TurnRow[] {
  return (u?.cache ?? []).filter(c => c.cachedShare != null).map(c => ({
    label: `Turn ${c.turn}`,
    value: `${Math.round((c.cachedShare ?? 0) * 100)}% cached${c.cold ? ` · cold: ${CAUSES[c.cause ?? 'unknown']}` : ''}`,
  }));
}

/** Shares of the latest recorded prompt, not a claim about retained context.
 * Provider history, hidden prompts and tool schemas are not measured here. */
export function messageContextSplit(manifest: Pick<RunManifest, 'layers'> | null | undefined): TurnRow[] {
  if (!manifest?.layers.length) return [];
  const sizes = [0, 0, 0, 0];
  for (const layer of manifest.layers) {
    if (!Number.isFinite(layer.bytes) || layer.bytes < 0) return [];
    const i = ['request', 'conversationContext', 'taskState', 'attachedImages', 'attachedFiles', 'sharedPage', 'pageContext'].includes(layer.name) ? 0
      : ['workspace', 'workspaceShared', 'retrievalEvidence', 'recall'].includes(layer.name) ? 1
        : ['operatorProfile', 'operatorUpdate'].includes(layer.name) ? 2 : 3;
    sizes[i] += layer.bytes;
  }
  const total = sizes.reduce((a, b) => a + b, 0);
  if (!total) return [];
  const shares = sizes.map(n => 100 * n / total);
  const rounded = shares.map(Math.floor);
  const order = shares.map((n, i) => ({ i, fraction: n - rounded[i] })).sort((a, b) => b.fraction - a.fraction);
  const remainder = 100 - rounded.reduce((a, b) => a + b, 0);
  for (let i = 0; i < remainder; i++) rounded[order[i].i]++;
  return ['Conversation', 'Project notes', 'Your profile', 'Instructions'].map((label, i) => ({ label, value: sizes[i] > 0 && rounded[i] === 0 ? '<1%' : `${rounded[i]}%` }));
}
