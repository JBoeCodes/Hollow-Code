// Settings PR 8: Usage in detail. Spread usage and the Claude account
// controls save from Usage, Limits through the same calls they made from
// Agents, Accounts; budgets show as meters with warn and hard-stop marks;
// old deep links to Accounts land on Usage, Limits.
import { describe, expect, it, vi } from 'vitest';
import CLAUDE_LIMITS from './ClaudeLimits.tsx?raw';
import ACCOUNTS from './ClaudeAccountsPanel.tsx?raw';
import USAGE_PANEL from './UsagePanel.tsx?raw';
import USAGE_LIMITS from './UsageLimits.tsx?raw';
import USAGE_METER from './UsageMeter.tsx?raw';
import NAVIGATION from './controllers/settingsNavigation.ts?raw';
import { CEILINGS, ceilingChoices, spreadUsage, type ClaudeAccountsCalls } from './spreadUsageActions';
import { budgetMeter, limitBudgets } from './budgetMeters';
import { agentTabs, usageTabFor } from './usageTabs';
import { agentNote, averageCost, cacheShare, notMetered } from './usageDetail';
import { SETTINGS_SECTION_BY_ID, resolveMovedControl } from '../settingsSections';
import { resolveSettingsTarget } from '../settingsResolve';
import type { BudgetStatus, ClaudeAccountsView, UsageBucket } from '../types';

const view = {} as ClaudeAccountsView;
function calls(): ClaudeAccountsCalls & { log: unknown[][] } {
  const log: unknown[][] = [];
  const rec = (name: string) => vi.fn(async (...args: unknown[]) => { log.push([name, ...args]); return view; });
  return {
    log,
    setClaudeAccountRouting: rec('setClaudeAccountRouting'),
    updateClaudeAccount: rec('updateClaudeAccount'),
    refreshClaudeAccounts: rec('refreshClaudeAccounts'),
  } as ClaudeAccountsCalls & { log: unknown[][] };
}

const SOURCES: Record<string, string> = {
  './ClaudeLimits.tsx': CLAUDE_LIMITS, './ClaudeAccountsPanel.tsx': ACCOUNTS, './UsagePanel.tsx': USAGE_PANEL,
  './UsageLimits.tsx': USAGE_LIMITS, './UsageMeter.tsx': USAGE_METER, './controllers/settingsNavigation.ts': NAVIGATION,
};
const src = (path: string) => SOURCES[path];

describe('Automatic rotation controls save from Usage', () => {
  it('makes the same calls for each control', async () => {
    const c = calls();
    await spreadUsage.moveOnAt(c, 90);
    await spreadUsage.inRotation(c, 'acct-1', false);
    await spreadUsage.checkUsage(c);
    expect(c.log).toEqual([
      ['setClaudeAccountRouting', 'spread', null, 90],
      ['updateClaudeAccount', { id: 'acct-1', enabled: false }],
      ['refreshClaudeAccounts'],
    ]);
  });

  it('keeps the Move on at choices and adds a saved odd one in order', () => {
    expect(ceilingChoices(80)).toEqual(CEILINGS);
    expect(ceilingChoices(85)).toEqual([70, 80, 85, 90, 95]);
  });

  it('wires every control on Usage, Limits through those calls, and Accounts keeps none of them', () => {
    const limits = src('./ClaudeLimits.tsx');
    for (const call of ['spreadUsage.moveOnAt(api, Number(value))',
      'spreadUsage.inRotation(api, a.id, box.checked)', 'spreadUsage.checkUsage(api)']) {
      expect(limits, call).toContain(call);
    }
    for (const label of ['Automatic account rotation', 'Move on at', 'In rotation', 'Check usage', 'Read live from Claude']) {
      expect(limits, label).toContain(label);
    }
    const accounts = src('./ClaudeAccountsPanel.tsx');
    expect(accounts).not.toContain('setClaudeAccountRouting');
    expect(accounts).not.toContain('claude-acct-use');
    expect(accounts).not.toContain('updateClaudeAccount({ id: a.id, enabled');
    expect(accounts).toContain('Limits and automatic account rotation are in Usage.');
    expect(limits).not.toContain('>One account</button>');
    expect(limits).not.toContain('spreadUsage.resume(api');
    expect(limits).not.toContain('spreadUsage.routing(api');
    expect(src('./UsagePanel.tsx')).toContain("'limits'");
  });
});

function budget(fields: Partial<BudgetStatus>): BudgetStatus {
  return {
    id: 'b', scope: 'global', scopeLabel: 'All usage', metric: 'costUsd', window: 'day', limit: 10, warnPercent: 80,
    hardStop: true, windowKey: '2026-11-12', windowStart: '', windowEnd: '', observed: 8, percent: 80, state: 'warning',
    pause: null, runs: 3, unknownRuns: 0, partialRuns: 0, unpricedRuns: 0, cachedInputTokens: 0, estimated: false,
    enforceability: 'ok', enforceabilityReason: null, scopeMissing: false, suggestedLimit: null, ledgerError: null,
    ...fields,
  };
}

describe('budgets on Limits', () => {
  it('shows a daily budget at 80% with its warn mark and its hard stop', () => {
    const row = budgetMeter(budget({}));
    expect(row.level).toBe('warn');
    expect(row.warnAt).toBe(0.8);
    expect(row.fill).toBe(0.8);
    expect(row.hardStop).toBe(true);
    expect(row.percent).toBe(80);
    const limits = src('./UsageLimits.tsx');
    expect(limits).toContain("kind: 'warn' as const");
    expect(limits).toContain("kind: 'stop' as const");
    expect(src('./UsageMeter.tsx')).toContain('data-mark={mark.kind}');
  });

  it('is ok under the warn mark, over at the limit, unknown when unread', () => {
    expect(budgetMeter(budget({ percent: 50, state: 'ok' })).level).toBe('ok');
    expect(budgetMeter(budget({ percent: 130, state: 'over' }))).toMatchObject({ level: 'over', fill: 1, percent: 130 });
    expect(budgetMeter(budget({ percent: null, observed: null, state: 'unknown' }))).toMatchObject({ level: 'unknown', percent: null });
  });

  it('puts day and week budgets first, fullest first, and monthly ones below', () => {
    const { soon, monthly } = limitBudgets([
      budget({ id: 'm', window: 'month', percent: 99 }),
      budget({ id: 'd', window: 'day', percent: 20 }),
      budget({ id: 'w', window: 'week', percent: 60 }),
    ]);
    expect(soon.map(r => r.id)).toEqual(['w', 'd']);
    expect(monthly.map(r => r.id)).toEqual(['m']);
  });
});

describe('old deep links land on Usage, Limits', () => {
  it('sends accounts and its part to Usage for the controls that moved', () => {
    for (const label of ['Spread usage', 'Check usage', 'Move on at', 'In rotation', 'Resume on another account when one runs out']) {
      const current = ['Spread usage', 'Resume on another account when one runs out'].includes(label) ? 'Automatic account rotation' : label;
      for (const id of ['accounts', 'defaults']) {
        expect(resolveSettingsTarget(id, label), `${id} ${label}`).toMatchObject({ kind: 'section', section: 'usage', label: current });
      }
      expect(resolveMovedControl('accounts', label)).toBe('usage');
      expect(usageTabFor(label)).toBe('limits');
      expect(SETTINGS_SECTION_BY_ID.usage.controls?.map(c => c.label)).toContain(current);
    }
    // Everything else in Accounts stays put.
    expect(resolveSettingsTarget('accounts', 'Other providers')).toMatchObject({ section: 'accounts' });
    expect(resolveMovedControl('usage', 'Spread usage')).toBe('usage');
  });

  it('opens the tab a control sits on', () => {
    expect(usageTabFor('Budgets')).toBe('limits');
    expect(usageTabFor('Usage by day')).toBe('all');
    expect(usageTabFor('Value this month')).toBe('all');
    expect(usageTabFor('Reserve for your own prompts')).toBe('budgets');
    expect(usageTabFor(undefined)).toBe('limits');
    expect(src('./controllers/settingsNavigation.ts')).toContain('requestUsageTab(usageTabFor(label))');
  });
});

const bucket = (key: string, fields: Partial<UsageBucket> = {}): UsageBucket => ({
  key, label: key, runs: 1, metered: 1, partial: 0, unknown: 0, notStarted: 0, inProgress: 0, unpriced: 0,
  inputTokens: 0, cachedInputTokens: 0, outputTokens: 0, reportedCostUsd: null, estimatedCostUsd: null, ...fields,
});

describe('agent tabs', () => {
  it('lists only agents with runs, in a fixed order', () => {
    const tabs = agentTabs([bucket('self-hosted'), bucket('codex'), bucket('grok', { runs: 0 }), bucket('claude'), bucket('antigravity')]);
    expect(tabs.map(t => t.label)).toEqual(['Claude', 'Codex', 'Antigravity', 'Self-hosted']);
    expect(agentTabs(undefined)).toEqual([]);
  });

  it('words the totals without inventing a cost', () => {
    const b = bucket('claude', { runs: 4, metered: 2, unknown: 1, notStarted: 1, inputTokens: 100, cachedInputTokens: 300, reportedCostUsd: 1 });
    expect(cacheShare(b)).toBe('75%');
    expect(averageCost(b)).toBe('$0.50');
    expect(notMetered(b)).toEqual(['1 with no usable usage figures', "1 didn't start"]);
    expect(averageCost(bucket('antigravity', { unknown: 1, metered: 0 }))).toBeNull();
    expect(agentNote('antigravity')).toContain('reports no tokens');
    expect(agentNote(null)).toBeNull();
  });
});
