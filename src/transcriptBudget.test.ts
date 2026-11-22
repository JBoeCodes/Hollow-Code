import { describe, expect, it } from 'vitest';
// @ts-expect-error: a Node script without types
import { TRANSCRIPT_BUDGET } from '../scripts/perf-budgets.mjs';
import { TURN_WINDOW } from './conversationTools';
import { AUTO_STEP, LOAD_STEP, TURN_CAP } from './transcriptWindow';

// The transcript budget (scripts/perf-budgets.mjs, measured by scripts/perf-transcript.mjs) and the
// window constants that meet it must move together: raising TURN_CAP without raising the budget, or
// the budget without a measurement, fails here. The measured run itself is `npm run perf`.
describe('the transcript window meets its budget', () => {
  it('draws no more turns than the budget allows, with room for the turns on screen', () => {
    expect(TURN_CAP).toBeLessThanOrEqual(TRANSCRIPT_BUDGET.maxTurnsInDom);
    expect(TURN_CAP + AUTO_STEP).toBeLessThanOrEqual(TRANSCRIPT_BUDGET.maxTurnsInDom);
  });
  it('holds the DOM-node budget at the measured cost per turn', () => {
    // 4,726 nodes for 200 turns (23.6 a turn, perf-transcript, 2026-11-20) plus the page around them.
    const nodesPerTurn = 30;
    expect(TURN_CAP * nodesPerTurn).toBeLessThanOrEqual(TRANSCRIPT_BUDGET.maxDomNodes);
  });
  it('keeps the first window and one press inside the cap', () => {
    expect(TURN_WINDOW).toBe(LOAD_STEP);
    expect(TURN_WINDOW + LOAD_STEP).toBeLessThanOrEqual(TURN_CAP);
  });
  it('does not loosen without a reason: ceilings that a 1,000-turn unwindowed thread would pass are refused', () => {
    // Unwindowed, the thread drew 1,000 turns and 21,132 nodes (profile.md, before 0.3).
    expect(TRANSCRIPT_BUDGET.maxTurnsInDom).toBeLessThan(1000);
    expect(TRANSCRIPT_BUDGET.maxDomNodes).toBeLessThan(21132);
    expect(TRANSCRIPT_BUDGET.maxHeapOverHomeMB).toBeLessThan(59.3);
  });
});
