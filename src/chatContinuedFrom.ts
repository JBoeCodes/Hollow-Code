/**
 * The earlier conversations a new one continues from (agent effectiveness
 * F4, K1 review). The backend's context compiler bundles the conversation a
 * fresh native session needs from the session store, for the turn's own
 * session key. Two composer paths start a new key while the window still
 * shows an earlier conversation: Sessions -> Continue from summary, and the
 * reset when the working folder vanished (`recoverChatContext`). The send
 * names those keys (`chat_send`'s `continuedFrom`), oldest first, and the
 * compiler bundles their recorded turns ahead of the conversation's own.
 * In memory only, like the turns on screen: a new chat clears it, a
 * backgrounded conversation keeps its own.
 */

/** At most this many earlier conversations ride along (the backend keeps
 *  the same number, `context_compiler::MAX_CONTINUED_FROM`). */
export const MAX_CONTINUED_FROM = 8;

/** `current` with `key` appended once (moved to the end if present),
 *  keeping the newest `MAX_CONTINUED_FROM`. */
export function continuedFromAfter(current: readonly string[], key: string | null | undefined): string[] {
  if (!key) return [...current];
  return [...current.filter(k => k !== key), key].slice(-MAX_CONTINUED_FROM);
}
