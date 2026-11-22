/**
 * Ask's scopes (consolidation D, 2026-10-13): the person picks where a
 * question goes. Each scope names exactly what context it uses and what
 * pressing the button does, so a question never lands in a different
 * conversation, with different context or permission, than the person
 * believes. Pure: Your work's Ask field (components/StudioHome.tsx) offers
 * them. The top bar's Ask ⌄ panel also offered "In this workspace"; it was
 * removed on 2026-10-21, and a workspace's own message box is that place.
 *
 * One Ask (2026-11-03): every question is a chat, so the two scopes that
 * opened their own overlay are gone: "Answer from my notes" (Ask your vault)
 * and "Plan it in Hollow" (Ask Hollow). A saved `notes` choice reads as a
 * quick chat and `hollow` as the Main agent (`store/ask.ts`). Quick chat's
 * Add to project starts the conversation in a workspace instead.
 *
 * L8 (11-10, Q1) sent New chat to the COO; since 2026-11-14 it starts a
 * quick chat again (with Add to project, that project's lead).
 */
export type AskScope = 'main' | 'quick';

export interface AskScopeInfo {
  id: AskScope;
  label: string;
  /** What context the question is asked with. */
  context: string;
  /** The button's words: what happens when it is pressed. */
  action: string;
  /** The field's placeholder: what Enter does in this scope (P1-11). */
  placeholder: string;
  /** The button's words while ⌥ is held, where ⌥-Enter only places the
   *  text (D2: the Main agent page's box). */
  placeAction?: string;
}

// Labels follow P1-03's glossary: "Do it" belongs to the Main agent's
// suggestions.
export const ASK_SCOPES: readonly AskScopeInfo[] = [
  { id: 'main', label: 'Across all my work', context: 'The COO, on its page: every workspace, your tasks and calendar. Enter sends; ⌥-Enter only puts it in its box', action: 'Ask the COO', placeAction: 'Put it in the box', placeholder: 'Ask the COO anything; Enter sends it on its page…' },
  { id: 'quick', label: 'New chat', context: 'A new quick chat outside any project, listed under Quick chats (with Add to project, for that project’s lead); your profile and memory still apply', action: 'Start the chat', placeholder: 'Ask anything; Enter starts a new quick chat…' },
];

/** The scope a fresh Ask starts on: the person's last choice, else a quick
 *  chat (what Home's Ask always did). */
export function defaultAskScope(last: AskScope | null): AskScope {
  return last ?? 'quick';
}

/** The longest text the command bar's Ask row takes. */
export const ASK_MAX_CHARS = 2000;

/** Whether a palette query reads as a sentence worth asking (overnight 10-11
 *  A2, the command bar's Ask row): three words or more (or a question of
 *  two), at most `ASK_MAX_CHARS`. A command name ("open workspace map") is a
 *  sentence too; the row then comes after the match. */
export function looksLikeSentence(query: string): boolean {
  const q = query.trim();
  if (!q || q.length > ASK_MAX_CHARS) return false;
  const words = q.split(/\s+/).filter(w => /[\p{L}\p{N}]/u.test(w));
  return words.length >= 3 || (q.endsWith('?') && words.length >= 2);
}

/** Where the Ask row goes among the palette's scored rows: first, unless the
 *  best row is a strong match (it is, or starts with, what was typed, or
 *  every word typed starts one of its words with at least one in its label:
 *  `paletteScore` 600 and up, P1-48), which then keeps Enter. */
export function askRowIndex(bestScore: number | undefined): number {
  return bestScore != null && bestScore >= 600 ? 1 : 0;
}
