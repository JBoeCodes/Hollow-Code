/**
 * Whether a chat turn shows the streaming caret (frontier F5).
 *
 * `partial` alone is not enough: a native Claude run splits one reply into a
 * turn per tool boundary, and every one of them stays `partial` until the run
 * ends. So the caret goes only on the last turn, only when no tool call follows
 * its text, and only while text is still arriving (`typing`: a delta landed in
 * the last `CARET_IDLE_MS`). A run waiting on a tool or a background check
 * shows no caret.
 */
export const CARET_IDLE_MS = 1200;

export function showsCaret(
  turn: { role: string; partial?: boolean; tools?: unknown[] },
  isLast: boolean,
  typing: boolean,
): boolean {
  return turn.role === 'assistant' && !!turn.partial && isLast && !(turn.tools?.length) && typing;
}
