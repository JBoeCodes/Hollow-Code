/**
 * Settings, Usage's tabs (Settings PR 8): Limits first (Usage opens there),
 * then All agents, one tab per agent with runs in the period, and Budgets.
 * Pure apart from one signal: a Settings jump to a control on another tab
 * asks for that tab, and the panel switches to it when it mounts or at once.
 */
import { createSignal } from 'solid-js';
import type { UsageBucket, UsageWindow } from '../types';

export type UsageTab = 'limits' | 'all' | 'budgets' | `agent:${string}`;

export const USAGE_PERIODS: readonly { id: UsageWindow; label: string }[] = [
  { id: 'day', label: 'Today' },
  { id: 'week', label: 'This week' },
  { id: 'month', label: 'This month' },
  { id: 'last30', label: 'Last 30 days' },
];

/** The agent tabs' order and names. */
const AGENT_ORDER = ['claude', 'codex', 'grok', 'antigravity', 'self-hosted'];
const AGENT_LABELS: Record<string, string> = {
  claude: 'Claude', codex: 'Codex', grok: 'Grok', antigravity: 'Antigravity', 'self-hosted': 'Self-hosted',
};

/** One tab per agent with runs in the period, in a fixed order. */
export function agentTabs(agents: readonly UsageBucket[] | undefined): { key: string; label: string }[] {
  const rank = (key: string) => { const at = AGENT_ORDER.indexOf(key); return at < 0 ? AGENT_ORDER.length : at; };
  return [...(agents ?? [])]
    .filter(a => a.runs > 0)
    .sort((a, b) => rank(a.key) - rank(b.key) || a.key.localeCompare(b.key))
    .map(a => ({ key: a.key, label: AGENT_LABELS[a.key] ?? a.label }));
}

export function agentTabLabel(key: string): string {
  return AGENT_LABELS[key] ?? key;
}

/** Controls that live on a tab other than Limits, by their visible label. */
const TAB_OF_LABEL: Readonly<Record<string, UsageTab>> = {
  'Usage by day': 'all',
  'Value this month': 'all',
  'By model': 'all',
  'By where': 'all',
  Outcomes: 'all',
  'All budgets': 'budgets',
  'Add budget': 'budgets',
  'Reserve for your own prompts': 'budgets',
  'Extra usage for unattended work': 'budgets',
  'Budget for each subagent': 'budgets',
  'Project budgets': 'budgets',
};

/** The tab a Settings jump to `label` lands on (Limits when it names none). */
export function usageTabFor(label?: string): UsageTab {
  return (label && TAB_OF_LABEL[label]) || 'limits';
}

/** A jump's tab, waiting for the Usage panel (consumed once read). */
export const [usageTabRequest, requestUsageTab] = createSignal<UsageTab | null>(null);
