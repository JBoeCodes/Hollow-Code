// A saved conversation's turns as a transcript (overnight 10-11 B3): the
// mapping from `read_agent_conversation` to `ChatTurn`s used by the
// workspace open (store/workspaces.ts). Conversation history keeps its own copy,
// with its incomplete-transcript and error notices.
import { executionReceiptFromWire } from './executionReceipt';
import type { AgentConversation } from './types';
import type { ChatTurn } from './chatTurnTypes';
import { steerRowsOf } from './chatSteer';
import { savedTurnFiles } from './turnFiles';

/** A saved user turn's send time (`startedAt`, when its run began) for
 *  `ChatTurn.at` (overnight 2026-11-04 C3); undefined for a reply, or a
 *  time that doesn't parse, so the message shows no time rather than a
 *  wrong one. Conversation history's own mapping uses it too. */
export function savedUserAt(turn: { role: string; startedAt?: string | null }): string | undefined {
  if (turn.role !== 'user' || !turn.startedAt) return undefined;
  return Number.isFinite(Date.parse(turn.startedAt)) ? turn.startedAt : undefined;
}

/** A saved turn's time for `ChatTurn.at`: a user turn's send time
 *  (`savedUserAt`), a reply's end (`endedAt`, when its run finished;
 *  overnight 11-12, C8), so the reply row says when it finished even when
 *  no receipt came back. Undefined when unknown or unparseable. */
export function savedTurnAt(turn: { role: string; startedAt?: string | null; endedAt?: string | null; status?: string | null }): string | undefined {
  if (turn.role === 'user') return savedUserAt(turn);
  // Only a run that really ended: an interrupted turn's `endedAt` is the
  // time the next launch recovered it, not when the reply stopped.
  if (turn.status && !FINISHED.has(turn.status)) return undefined;
  return turn.endedAt && Number.isFinite(Date.parse(turn.endedAt)) ? turn.endedAt : undefined;
}

/** The statuses whose saved `endedAt` is when the reply finished. */
const FINISHED = new Set(['success', 'error', 'cancelled']);

/** A live reply's finish time once `status` is marked (`markReply`): the
 *  first mark's time is kept, except that a success replaces an earlier
 *  error's (a reply that failed, then ran on and succeeded, finished at the
 *  success). */
export function markedReplyAt(turn: { status?: string | null; at?: string }, status: 'success' | 'error', now: string): string {
  return !turn.at || (status === 'success' && turn.status === 'error') ? now : turn.at;
}

/** A saved turn's words and, for a user turn, the files it was sent with
 *  (B1a, ENG-104): the store's `files` when it sent them, else chips from a
 *  trailing `<attached-files>` block in the text, which leaves the bubble
 *  either way. Conversation history's own mapping uses it too. */
export function savedUserText(turn: { role: string; text: string; files?: ChatTurn['files'] }): Pick<ChatTurn, 'text' | 'files'> {
  if (turn.role !== 'user') return { text: turn.text };
  const parsed = savedTurnFiles(turn.text);
  return turn.files?.length ? { text: parsed.text, files: turn.files } : parsed;
}

export function turnsFromConversation(conversation: Pick<AgentConversation, 'turns'>): ChatTurn[] {
  // A turn's steers (mid-turn steering) show before its answer: what the
  // person said while it worked.
  return conversation.turns.flatMap(t => [...steerRowsOf(t), {
    role: t.role, ...savedUserText(t), tools: t.tools.map(tool => ({ ...tool, result: tool.result ?? undefined })), agent: t.agent,
    receipt: t.receipt ? executionReceiptFromWire(t.receipt) : undefined, status: t.status,
    resultKind: t.resultKind ?? t.receipt?.resultKind ?? null, stoppedBeforeSpawn: t.stoppedBeforeSpawn ?? false, runId: t.runId ?? undefined,
    at: savedTurnAt(t),
    reasoning: t.reasoning || undefined,
  }]);
}
