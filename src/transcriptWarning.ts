import type { ChatTurn } from './chatTurnTypes';
import { cancelledNotice } from './chatStop';

/** Only explicit host warnings qualify. Ordinary notices and assistant prose
 * retain their existing presentation. Exact text is the cause identity. */
export function transcriptWarning(turns: readonly ChatTurn[], index: number): { title: string; count: number } | null {
  const turn = turns[index];
  const hostError = turn?.role === 'assistant' && !turn.tools.length && /^\(error: [\s\S]*\)$/.test(turn.text);
  // A stop's own notice ("Stopped. Hollow could not confirm...") is neutral, not a host warning, though its words match.
  if (turn?.role === 'system' && turn.text === cancelledNotice('unconfirmed')) return null;
  if (!turn || turn.notice || (!hostError && (turn.role !== 'system' || !/\b(warning|failed|failure|not sent|could not|couldn't|limit|capped)\b/i.test(turn.text)))) return null;
  let start = index;
  while (start > 0 && turns[start - 1].role !== 'user') start--;
  let end = index + 1;
  while (end < turns.length && turns[end].role !== 'user') end++;
  const same = (t: ChatTurn) => t.role === turn.role && !t.notice && t.text === turn.text;
  if (turns.slice(start, index).some(same)) return { title: '', count: 0 };
  const first = turn.text.trim().split('\n')[0];
  return { title: first.length > 120 ? `${first.slice(0, 119)}…` : first, count: turns.slice(index, end).filter(same).length };
}
