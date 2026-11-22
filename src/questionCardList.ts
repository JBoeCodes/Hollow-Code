/**
 * The question-card helpers the chat store needs on every event: keeping the
 * shown list and deciding where a failed answer goes. Split from
 * questionCards.ts (overnight 12 A0) so the store, which is in the App chunk,
 * doesn't pull the card form's validation and answer building in with it;
 * questionCards.ts re-exports these.
 *
 * Type-only imports: the node vitest suite loads this module directly.
 */
import type { ChatInteraction } from './types';

/** The notice when an answer turn could not be sent and its answers went to
 *  the composer instead of back onto the card. */
export const ANSWERS_IN_COMPOSER_NOTICE = 'Your answers are in the composer; send them as a message.';

/** What to do when an answer turn could not be sent: put the card back when
 *  it reopened in the conversation on screen (since 2026-11-04 a card can
 *  always be sent, into its own session or as a message), so the answers are
 *  live in one place only; otherwise (it didn't reopen, or the chat was reset
 *  to another conversation meanwhile) they go to the composer, so they are
 *  never lost. */
export function answerSendFailureAction(input: { reopenOk: boolean; sameChat: boolean }): 'restoreCard' | 'toComposer' {
  return input.reopenOk && input.sameChat ? 'restoreCard' : 'toComposer';
}

/** Whether a card is still shown (pending or stale). */
export function isOpenCard(card: Pick<ChatInteraction, 'status'>): boolean {
  return card.status === 'pending' || card.status === 'stale';
}

/** A finished turn never drops an open card. Closed cards still leave.
 *  Returns the same array when every card is still open. */
export function keepOpenCards(list: ChatInteraction[]): ChatInteraction[] {
  const open = list.filter(isOpenCard);
  return open.length === list.length ? list : open;
}

/** Apply one card update to the shown list for the current conversation:
 *  an open card is inserted or replaced, a closed one removed, another
 *  conversation's card ignored unless it is in `chain`. Keeps creation order. */
export function applyInteractionUpdate(list: ChatInteraction[], card: ChatInteraction, currentKey: string | null, chain: ReadonlySet<string> = new Set()): ChatInteraction[] {
  // `chain`: the chats before it in its chain, whose cards show here too
  // (one Main conversation, 2026-11-07).
  if (!currentKey || (card.sessionKey !== currentKey && !chain.has(card.sessionKey))) return list;
  const rest = list.filter(c => c.id !== card.id);
  if (!isOpenCard(card)) return rest.length === list.length ? list : rest;
  const at = list.findIndex(c => c.id === card.id);
  if (at >= 0) {
    const next = [...list];
    next[at] = card;
    return next;
  }
  return [...list, card];
}
