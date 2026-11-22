import { describe, expect, it } from 'vitest';
import {
  HANDOVER_TOKENS, billingLabel, cacheRows, contextSummary, contextWindow, handoverOffered, messageContextSplit, largestRows, percentUsed, planLevel, planUsage,
  popoverEndpoint, resetText, selfHostedEndpoint, toolUseRows, turnRows,
} from './chatUsage';
import type { ClaudeAccountsView, ClaudeQuota, ContextUse, TurnUsage } from './types';

describe('contextWindow', () => {
  it('knows Claude windows and bumps to 1M when a turn already used more', () => {
    expect(contextWindow('claude', 'opus', 50_000)).toBe(200_000);
    expect(contextWindow('claude', 'opus[1m]', 50_000)).toBe(1_000_000);
    expect(contextWindow('claude', 'opus', 450_000)).toBe(1_000_000);
    expect(contextWindow('codex', 'gpt-5', 10)).toBeNull();
  });

  it('uses the window a Codex turn reported (overnight 9 A10)', () => {
    const window = contextWindow('codex', 'gpt-6-astra', 231_046, 258_400);
    expect(window).toBe(258_400);
    const ring = contextSummary(231_046, window);
    expect(ring.label).toBe('231k/258.4k');
    expect(ring.fraction).toBeCloseTo(0.894, 3);
    // Past 75%: the ring offers "Continue in a new conversation".
    expect(handoverOffered(231_046, window)).toBe(true);
    expect(handoverOffered(150_000, window)).toBe(false);
    // A reported window wins over the model's inferred one; none keeps the old answer.
    expect(contextWindow('claude', 'opus', 50_000, 258_400)).toBe(258_400);
    expect(contextWindow('codex', 'gpt-6-astra', 10, null)).toBeNull();
  });
});

describe('contextSummary', () => {
  it('labels used/window, or tokens alone without a window', () => {
    expect(contextSummary(941_500, 1_000_000)).toEqual({ label: '941.5k/1M', fraction: 0.9415 });
    expect(contextSummary(1_200, null)).toEqual({ label: '1.2k tokens', fraction: null });
  });
});

describe('plan windows', () => {
  it.each([
    [0, '0% used'], [0.5, '0.5% used'], [9.96, '10.0% used'],
    [23.5, '24% used'], [37.42, '37% used'], [99.6, '100% used'], [101, '101% used'],
  ])('shares plan-label precision for %s percent', (value, label) => {
    expect(percentUsed(value as number)).toBe(label);
  });

  // P1-27: the composer and Settings read one level rule for a plan window.
  it('levels a window the same way on every surface', () => {
    expect([0, 79.9, 80, 99.9, 100, 140].map(planLevel)).toEqual(['ok', 'ok', 'high', 'high', 'over', 'over']);
  });

  it('formats a reset time and skips an unknown one', () => {
    expect(resetText('2026-09-27T00:50:00Z')).toMatch(/^Resets Sep 2[67], 2026, /);
    expect(resetText(null)).toBeNull();
    expect(resetText('nope')).toBeNull();
  });

  const quota: ClaudeQuota = {
    source: 'stream',
    observedAt: '2026-09-26T12:00:00Z',
    windows: [
      { id: 'five_hour', label: '5-hour', usedPercent: 27, resetsAt: null, source: 'stream', observedAt: '' },
      { id: 'seven_day', label: 'Weekly', usedPercent: 34, resetsAt: null, source: 'stream', observedAt: '' },
    ],
  };

  it('uses the login quota when no added account runs', () => {
    const p = planUsage(null, quota)!;
    expect(p.account).toBeNull();
    expect(p.windows.map(w => w.label)).toEqual(['5h limit', '7d limit']);
    expect(planUsage(null, null)).toBeNull();
  });

  it('prefers the added account new runs use', () => {
    const view: ClaudeAccountsView = {
      routing: 'single', pinned: 'acct-1', ceilingPercent: 90, failover: true, nextAccount: 'acct-1', error: null,
      accounts: [{
        id: 'acct-1', label: 'Work', enabled: true, status: 'ok', note: null, lastUsedAt: null, createdAt: null,
        usage: { fiveHour: { usedPercent: 10, resetsAt: null }, sevenDay: null, status: null, overageDisabledReason: null, source: 'probe', observedAt: '' },
      }],
    };
    const p = planUsage(view, quota)!;
    expect(p.account).toBe('Work');
    expect(p.windows).toEqual([{ id: 'five_hour', label: '5h limit', usedPercent: 10, resetsAt: null }]);
  });
});

describe('turnRows', () => {
  it('lists only the reported counts', () => {
    const u: TurnUsage = { costUsd: null, costStatus: 'unpriced', billing: 'unknown', inputTokens: 1200, cachedInputTokens: null, outputTokens: 30 };
    expect(turnRows(u)).toEqual([{ label: 'Input', value: '1.2k' }, { label: 'Output', value: '30' }]);
    expect(turnRows(null)).toEqual([]);
  });
});

describe('handoverOffered (repo features R6)', () => {
  it('offers at 75% of a known window, or past the hand-over point on 1M', () => {
    expect(HANDOVER_TOKENS).toBe(350_000);
    expect(handoverOffered(150_000, 200_000)).toBe(true);
    expect(handoverOffered(149_000, 200_000)).toBe(false);
    expect(handoverOffered(349_000, 1_000_000)).toBe(false);
    expect(handoverOffered(350_000, 1_000_000)).toBe(true);
    expect(handoverOffered(900_000, null)).toBe(false);
  });
});

describe('context use rows (repo features R6)', () => {
  const use: ContextUse = {
    windowStartedAt: null, windowTurns: 2, measuredTurns: 2, atLeast: false,
    byTool: [{ tool: 'Read', bytes: 48_000, calls: 3 }, { tool: 'mcp__hollow_vault__read_source', bytes: 400, calls: 1 }, { tool: 'other', bytes: 4, calls: 1 }],
    largest: [{ turn: 2, tool: 'Read', bytes: 40_000, target: 'chat.rs' }, { turn: 1, tool: 'Task', bytes: 2_000, target: null }],
    cache: [
      { turn: 1, inputTokens: 40_000, cachedInputTokens: 0, cachedShare: 0, cold: true, cause: 'newSession' },
      { turn: 2, inputTokens: 1_000, cachedInputTokens: 39_000, cachedShare: 0.975, cold: false, cause: null },
      { turn: 3, inputTokens: 50_000, cachedInputTokens: 0, cachedShare: 0, cold: true, cause: 'idle' },
      { turn: 4, inputTokens: null, cachedInputTokens: null, cachedShare: null, cold: false, cause: null },
    ],
    compactions: 0,
  };

  it('estimates tokens from bytes and names tools without their server', () => {
    expect(toolUseRows(use)).toEqual([
      { label: 'Read', value: '≈12k tokens est. · 3 calls' },
      { label: 'read_source', value: '≈100 tokens est. · 1 call' },
      { label: 'Other', value: '≈1 tokens est. · 1 call' },
    ]);
    expect(largestRows(use)).toEqual([
      { label: 'Turn 2 · Read · chat.rs', value: '≈10k tokens est.' },
      { label: 'Turn 1 · Task', value: '≈500 tokens est.' },
    ]);
    expect(toolUseRows(null)).toEqual([]);
  });

  it('marks cold turns with their cause and skips unreported ones', () => {
    expect(cacheRows(use)).toEqual([
      { label: 'Turn 1', value: '0% cached · cold: new session' },
      { label: 'Turn 2', value: '98% cached' },
      { label: 'Turn 3', value: '0% cached · cold: idle over an hour' },
    ]);
  });
});

describe('billingLabel (Q8)', () => {
  it('names GPU-time billing for a self-hosted run', () => {
    expect(billingLabel('self-hosted')).toBe('Billed by GPU time');
    expect(billingLabel('subscription')).toBe('Subscription');
    expect(billingLabel('unknown')).toBeNull();
  });
});

describe('selfHostedEndpoint (Q8 review)', () => {
  it('names the endpoint the popover shows instead of the Claude account and plan', () => {
    expect(selfHostedEndpoint({ billing: 'self-hosted', endpoint: 'Qwen3.8 27B' })).toBe('Qwen3.8 27B');
    expect(selfHostedEndpoint({ billing: 'self-hosted', endpoint: ' ' })).toBe('Self-hosted model');
    expect(selfHostedEndpoint({ billing: 'self-hosted' })).toBe('Self-hosted model');
    expect(selfHostedEndpoint({ billing: 'subscription', endpoint: 'ignored' })).toBeNull();
    expect(selfHostedEndpoint(null)).toBeNull();
  });

  it('keeps the last endpoint while a turn runs, and knows a self-hosted model id before any turn', () => {
    const qwen = { billing: 'self-hosted' as const, endpoint: 'Qwen3.8 27B' };
    expect(popoverEndpoint(qwen, 'opus', null)).toBe('Qwen3.8 27B');
    expect(popoverEndpoint({ billing: 'subscription' }, '', 'Qwen3.8 27B')).toBeNull();
    // Running: the turn's usage was cleared at runStarted.
    expect(popoverEndpoint(null, '', 'Qwen3.8 27B')).toBe('Qwen3.8 27B');
    expect(popoverEndpoint(null, 'self-hosted:qwen', 'Qwen3.8 27B')).toBe('Qwen3.8 27B');
    expect(popoverEndpoint(null, 'self-hosted:qwen', null)).toBe('Self-hosted model');
    expect(popoverEndpoint(null, 'opus', 'Qwen3.8 27B')).toBeNull();
    expect(popoverEndpoint(null, '', null)).toBeNull();
  });
});


describe('messageContextSplit', () => {
  const split = (entries: [string, number][]) => messageContextSplit({ layers: entries.map(([name, bytes]) => ({ name, bytes, sha256: '' })) });
  it('groups only recorded text and apportions rounding to 100 percent', () => {
    expect(split([['request', 72], ['workspace', 4], ['operatorProfile', 1], ['policy', 23]])).toEqual([
      { label: 'Conversation', value: '72%' }, { label: 'Project notes', value: '4%' },
      { label: 'Your profile', value: '1%' }, { label: 'Instructions', value: '23%' },
    ]);
    expect(split([['request', 1], ['workspace', 1], ['operatorUpdate', 1]]).map(r => r.value)).toEqual(['34%', '33%', '33%', '0%']);
  });
  it('counts the shared project docs as project notes', () => {
    expect(split([['request', 50], ['workspaceShared', 50]]).map(r => r.value)).toEqual(['50%', '50%', '0%', '0%']);
  });
  it('does not invent a split without measurements', () => {
    expect(messageContextSplit(null)).toEqual([]);
    expect(split([])).toEqual([]);
    expect(split([['request', 0]])).toEqual([]);
    expect(split([['request', -1]])).toEqual([]);
  });
  it('keeps small nonzero shares distinguishable from absent layers', () => {
    expect(split([['request', 999], ['operatorProfile', 1]])[2].value).toBe('<1%');
  });
  it('keeps cache reads out of Last turn token rows', () => {
    expect(turnRows({ inputTokens: 7400, outputTokens: 1100, cachedInputTokens: 767200 } as TurnUsage)).toEqual([{ label: 'Input', value: '7.4k' }, { label: 'Output', value: '1.1k' }]);
  });
});

