import { expect, it } from 'vitest';
import { proposalRequest, proposalUsable } from './planProposal';
import type { ChatTurn } from './chatTurnTypes';

it('only the completed latest proposal can prepare a follow-up', () => {
  const plan: ChatTurn = { role: 'assistant', text: 'Plan', proposedPlan: 'Plan', tools: [], runId: 'r-plan' };
  expect(proposalUsable([plan], 0, false, false)).toBe(true);
  expect(proposalUsable([plan], 0, true, false)).toBe(false);
  expect(proposalUsable([plan], 0, false, true)).toBe(false);
  expect(proposalUsable([{ ...plan, partial: true }], 0, false, false)).toBe(false);
  expect(proposalUsable([{ ...plan, proposedPlan: undefined }], 0, false, false)).toBe(false);
  expect(proposalUsable([plan, { role: 'user', text: 'Later request', tools: [] }], 0, false, false)).toBe(false);
  expect(proposalRequest(plan, 'accept')).toContain('r-plan');
  expect(proposalRequest(plan, 'refine')).toContain('My changes:');
});
