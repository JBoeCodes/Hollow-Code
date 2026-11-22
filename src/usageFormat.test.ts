import { describe, expect, it } from 'vitest';
import { bucketCost, costChip, formatTokens, formatUsd, nightItemCost, SURFACE_LABELS } from './usageFormat';
import type { TurnUsage } from './types';
// `?raw` reads the sources as text without executing them (they import the
// store and Tauri), so this stays a pure node test.
import CHAT_BODY from './components/ChatBody.tsx?raw';
import USAGE_MENU from './components/ChatUsageMenu.tsx?raw';
import CHAT_RUNTIME from './store/chatRuntime.ts?raw';

/** "$0" or "$0.00" as a whole figure; not "$0.42" and not "<$0.01". */
const ZERO_DOLLARS = /\$0(?:\.0+)?(?![.\d])/;

function turn(fields: Partial<TurnUsage>): TurnUsage {
  return {
    costUsd: null,
    costStatus: 'unknown',
    billing: 'unknown',
    inputTokens: null,
    cachedInputTokens: null,
    outputTokens: null,
    ...fields,
  };
}

describe('usage labels never show $0 and keep estimates labelled', () => {
  it('names the provider and source of each estimate', () => {
    const usage = turn({ costStatus: 'estimated', billing: 'subscription', costUsd: 0.42 });
    expect(costChip(usage, 'codex')?.title).toContain("Hollow's estimate from your price for the model");
    expect(costChip(usage, 'codex')?.title).not.toContain('Claude');
    expect(costChip(usage, 'claude')?.title).toContain("Claude Code's estimate");
    expect(costChip(usage, 'grok')?.title).toContain("The agent's estimate");
  });

  it('matches only whole-zero figures with the guard regex', () => {
    expect(ZERO_DOLLARS.test('$0')).toBe(true);
    expect(ZERO_DOLLARS.test('$0.00')).toBe(true);
    expect(ZERO_DOLLARS.test('$0.42')).toBe(false);
    expect(ZERO_DOLLARS.test('<$0.01')).toBe(false);
  });

  it('hides the chip when there is nothing honest to show', () => {
    expect(costChip(null)).toBeNull();
    expect(costChip(turn({ costStatus: 'reported', costUsd: 0 }))).toBeNull();
    expect(costChip(turn({ costStatus: 'estimated', costUsd: 0 }))).toBeNull();
    expect(costChip(turn({ costStatus: 'unpriced' }))).toBeNull();
    expect(costChip(turn({ costStatus: 'partial' }))).toBeNull();
    expect(costChip(turn({ costStatus: 'unknown', costUsd: 1, inputTokens: 5 }))).toBeNull();
    expect(formatUsd(0)).toBeNull();
    expect(formatUsd(null)).toBeNull();
    expect(formatUsd(Number.NaN)).toBeNull();
  });

  it('renders tiny costs as <$0.01, reported plainly and estimates with ≈ and est.', () => {
    expect(formatUsd(0.004)).toBe('<$0.01');
    const reported = costChip(turn({ costStatus: 'reported', billing: 'api', costUsd: 0.42 }))!;
    expect(reported.label).toBe('$0.42');
    expect(reported.label).not.toContain('≈');
    const estimated = costChip(turn({ costStatus: 'estimated', billing: 'subscription', costUsd: 0.1556084 }), 'claude')!;
    expect(estimated.label.startsWith('≈')).toBe(true);
    expect(estimated.label.endsWith('est.')).toBe(true);
    expect(estimated.title).toContain('list prices');
    const unknownBilling = costChip(turn({ costStatus: 'estimated', billing: 'unknown', costUsd: 0.004 }), 'claude')!;
    expect(unknownBilling.label).toBe('≈<$0.01 est.');
    expect(unknownBilling.title).toContain("couldn't tell");
  });

  it('shows token counts for unpriced and partial turns', () => {
    const unpriced = costChip(turn({ costStatus: 'unpriced', inputTokens: 11_849, cachedInputTokens: 11_904, outputTokens: 5 }))!;
    expect(unpriced.label).toBe('23.8k tokens');
    const partial = costChip(turn({ costStatus: 'partial', inputTokens: 18_119, cachedInputTokens: 42_412, outputTokens: 77 }))!;
    expect(partial.label).toBe('60.6k tokens (partial)');
    expect(formatTokens(999)).toBe('999');
    expect(formatTokens(1_200)).toBe('1.2k');
    expect(formatTokens(3_400_000)).toBe('3.4M');
  });

  it('never produces a $0 label for any status or billing', () => {
    const statuses: TurnUsage['costStatus'][] = ['reported', 'estimated', 'unpriced', 'partial', 'unknown'];
    const billings: TurnUsage['billing'][] = ['api', 'subscription', 'unknown'];
    for (const costStatus of statuses) {
      for (const billing of billings) {
        for (const costUsd of [null, 0, 0.0001, 0.004, 0.005, 0.42, 12]) {
          const chip = costChip(turn({ costStatus, billing, costUsd, inputTokens: 1, outputTokens: 1 }));
          if (chip) expect(chip.label).not.toMatch(ZERO_DOLLARS);
        }
      }
    }
    for (const reportedCostUsd of [null, 0, 0.001, 0.42]) {
      for (const estimatedCostUsd of [null, 0, 0.003, 3.1]) {
        expect(bucketCost({ reportedCostUsd, estimatedCostUsd })).not.toMatch(ZERO_DOLLARS);
      }
    }
  });

  it('keeps reported and estimated bucket costs apart', () => {
    expect(bucketCost({ reportedCostUsd: 0.42, estimatedCostUsd: 3.1 })).toBe('$0.42 reported · ≈$3.10 estimated');
    expect(bucketCost({ reportedCostUsd: null, estimatedCostUsd: 3.1 })).toBe('≈$3.10 estimated');
    expect(bucketCost({ reportedCostUsd: null, estimatedCostUsd: null })).toBe('—');
    expect(bucketCost({ reportedCostUsd: 0, estimatedCostUsd: 0 })).toBe('—');
    expect(SURFACE_LABELS.delegate).toBe('Teammates');
  });

  it('says a self-hosted bucket is billed by GPU time, never $0 or a dash (Q8 review)', () => {
    expect(bucketCost({ reportedCostUsd: null, estimatedCostUsd: null, gpuTime: 2 })).toBe('Billed by GPU time');
    expect(bucketCost({ reportedCostUsd: 0.42, estimatedCostUsd: null, gpuTime: 1 })).toBe('$0.42 reported · plus GPU time');
    expect(bucketCost({ reportedCostUsd: 0, estimatedCostUsd: 0, gpuTime: 1 })).not.toMatch(ZERO_DOLLARS);
  });

  it('names GPU time on a partial self-hosted turn and on a night item (Q8 second review)', () => {
    const cut = costChip(turn({ costStatus: 'partial', billing: 'self-hosted', inputTokens: 700, endpoint: 'Qwen3.8 27B' }), 'claude');
    expect(cut?.label).toBe(`${formatTokens(700)} tokens (partial)`);
    expect(cut?.title).toContain('Billed by GPU time on Qwen3.8 27B');
    expect(costChip(turn({ costStatus: 'partial', billing: 'subscription', inputTokens: 700 }))?.title).not.toContain('GPU');
    expect(nightItemCost({ costUsd: null, gpuTime: true })).toBe('Billed by GPU time');
    expect(nightItemCost({ costUsd: 0.25, gpuTime: true })).toBe('Recorded cost $0.25, plus GPU time');
    expect(nightItemCost({ costUsd: 0.25 })).toBe('Recorded cost $0.25');
    expect(nightItemCost({ costUsd: null })).toBe('Cost not reported');
    expect(nightItemCost({ costUsd: 0, gpuTime: true })).not.toMatch(ZERO_DOLLARS);
  });

  it('is what the chat composer and the chat event handler use', () => {
    // Overnight 11-04 (C2): the composer shows the cost in the usage ring
    // (its hover text and card), not as words on the bar.
    expect(USAGE_MENU).toContain('costChip(');
    expect(CHAT_BODY).not.toContain('costChip(');
    expect(CHAT_BODY).not.toMatch(/chatCostUsd|cost\S*\.toFixed\(2\)/);
    expect(CHAT_RUNTIME).toContain("case 'usage'");
    const started = CHAT_RUNTIME.indexOf("case 'runStarted'");
    expect(started).toBeGreaterThan(-1);
    const nextCase = CHAT_RUNTIME.indexOf('case ', started + 1);
    expect(CHAT_RUNTIME.slice(started, nextCase)).toContain('setChatTurnUsage(null)');
  });
});

describe('self-hosted models (Q8)', () => {
  it('shows tokens and "billed by GPU time" with the endpoint, never a price', () => {
    const usage = turn({
      costStatus: 'unpriced', billing: 'self-hosted', inputTokens: 163_722, outputTokens: 925, endpoint: 'Qwen3.8 27B',
    });
    const chip = costChip(usage, 'claude');
    expect(chip?.label).toBe(`${formatTokens(164_647)} tokens`);
    expect(chip?.title).toContain('Billed by GPU time on Qwen3.8 27B');
    expect(`${chip?.label} ${chip?.title}`).not.toMatch(ZERO_DOLLARS);
    expect(`${chip?.label}`).not.toContain('$');
    const nameless = costChip(turn({ costStatus: 'unpriced', billing: 'self-hosted', inputTokens: 10 }), 'claude');
    expect(nameless?.title).toContain('your self-hosted model');
  });
});
