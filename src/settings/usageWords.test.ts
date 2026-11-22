// Settings, Usage's small word helpers (Settings PR 8) that `usageInDetail`
// leaves out: token totals, the cost kind a bucket holds, the agent tab
// names and the budget period words.
import { describe, expect, it } from 'vitest';
import { allTokens, costKind } from './usageDetail';
import { agentTabLabel, agentTabs, USAGE_PERIODS } from './usageTabs';
import { budgetPeriodWords } from './budgetMeters';
import type { UsageBucket } from '../types';

const bucket = (key: string, fields: Partial<UsageBucket> = {}): UsageBucket => ({
  key, label: key, runs: 1, metered: 1, partial: 0, unknown: 0, notStarted: 0, inProgress: 0, unpriced: 0,
  inputTokens: 0, cachedInputTokens: 0, outputTokens: 0, reportedCostUsd: null, estimatedCostUsd: null, ...fields,
});

describe('a usage bucket in numbers', () => {
  it('counts input, cached input and output once each', () => {
    expect(allTokens(bucket('claude', { inputTokens: 100, cachedInputTokens: 300, outputTokens: 50 }))).toBe(450);
    expect(allTokens(bucket('antigravity'))).toBe(0);
  });

  it('says which kind of cost it holds: reported, estimated, both or none (a zero cost is still a cost)', () => {
    expect(costKind(bucket('claude', { reportedCostUsd: 2 }))).toBe('reported');
    expect(costKind(bucket('codex', { estimatedCostUsd: 0.4 }))).toBe('estimated');
    expect(costKind(bucket('claude', { reportedCostUsd: 2, estimatedCostUsd: 1 }))).toBe('both');
    expect(costKind(bucket('claude', { reportedCostUsd: 0 }))).toBe('reported');
    expect(costKind(bucket('antigravity'))).toBe('none');
  });
});

describe('Usage tab and period names', () => {
  it('names an agent tab, and keeps an unknown agent as recorded', () => {
    expect(agentTabLabel('self-hosted')).toBe('Self-hosted');
    expect(agentTabLabel('claude')).toBe('Claude');
    expect(agentTabLabel('acme')).toBe('acme');
  });

  it('puts an unknown agent after the known ones, alphabetically, under its own bucket label', () => {
    const tabs = agentTabs([bucket('zeta', { label: 'Zeta' }), bucket('acme', { label: 'Acme' }), bucket('codex')]);
    expect(tabs).toEqual([{ key: 'codex', label: 'Codex' }, { key: 'acme', label: 'Acme' }, { key: 'zeta', label: 'Zeta' }]);
    expect(agentTabs(undefined)).toEqual([]);
  });

  it('offers the four periods in order, and the budget words match the three budget windows', () => {
    expect(USAGE_PERIODS.map(p => p.id)).toEqual(['day', 'week', 'month', 'last30']);
    expect(budgetPeriodWords('day')).toBe('Today');
    expect(budgetPeriodWords('week')).toBe('This week');
    expect(budgetPeriodWords('month')).toBe('This month');
    // The budget words are the first three period labels.
    expect(['day', 'week', 'month'].map(id => USAGE_PERIODS.find(p => p.id === id)!.label)).toEqual(['Today', 'This week', 'This month']);
  });
});
