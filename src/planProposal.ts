import type { ChatTurn } from './chatTurnTypes';

export function proposalUsable(turns: readonly ChatTurn[], at: number, busy: boolean, loading: boolean): boolean {
  const turn = turns[at];
  return !busy && !loading && !!turn && turn.role === 'assistant' && !turn.partial
    && !!turn.runId && !!turn.proposedPlan?.trim() && !turns.slice(at + 1).some(t => t.role === 'user');
}

export function proposalRequest(turn: ChatTurn, choice: 'accept' | 'refine'): string {
  return choice === 'accept'
    ? `I accept the proposed plan from run ${turn.runId}. Implement it when my selected permission mode allows changes.`
    : `Refine the proposed plan from run ${turn.runId}. My changes: `;
}
