/**
 * The quiet actions under your own chat messages (overnight 2026-11-04,
 * lane C, C3): when each was sent, Copy, and Edit on the last one. Pure
 * (vitest loads it in node); `components/ChatBody.tsx` renders them. A
 * saved turn's send time is `chatHydrate.ts` `savedUserAt`, kept there so
 * the launch chunk doesn't load this module or `turnFooter.ts`.
 */
import { finishedText } from './turnFooter';
import { CONTINUE_PROMPT } from './chatContinue';
import { mainTurnKind } from './mainAgent';
import { readCanvasAnswers, visibleUserText } from './canvas/askAnswer';
import { isAgentContinuedPrompt } from './accountSwitchNotice';

/** A question card's answers start with this line, then one line per
 *  answer (Rust `interaction_schema::ANSWER_HEADER`; a vitest pins it). */
export const CARD_ANSWER_HEADER = 'Answered questions';
const CARD_ANSWERS = new RegExp(`^${CARD_ANSWER_HEADER}(?:\\n|$)`);

/** A user turn Hollow wrote, not the person: a Main agent wake, the Continue
 *  prompt, a question card's answers (resolved by the backend for its card),
 *  an ask-canvas's answers (they lock their canvas revision) or
 *  the continuation on another agent after one ran out of usage. Sent again
 *  as plain text, any of them would skip the checks it first went with (the
 *  continuation would go to the agent still picked), so Retry and Edit
 *  never offer one (C3 review). */
export function hostWrittenTurn(turn: { role: string; text: string }): boolean {
  return mainTurnKind(turn) === 'wake' || turn.text.trim() === CONTINUE_PROMPT
    || CARD_ANSWERS.test(turn.text) || readCanvasAnswers(turn.text).length > 0
    || isAgentContinuedPrompt(turn.text);
}

/** A message that went with more than its words: images, files (C5;
 *  `files`, not among `images`), a page shared from the Preview pane
 *  (`sharedPage`), or a text that still holds an attached-images,
 *  attached-files or shared-page block. Retry and Edit send only words, so
 *  they aren't offered for one (C3 review). A reopened conversation keeps
 *  only the words of each message, so it can't say. */
export function carriesAttachments(turn: { text: string; images?: readonly string[]; files?: readonly unknown[]; sharedPage?: boolean }): boolean {
  return !!turn.images?.length || !!turn.files?.length || !!turn.sharedPage || /<(?:attached-images|attached-files|shared-page)[\s>]/.test(turn.text);
}

/** Whether Retry or Edit can send this message again as it went: the
 *  person's own, words only. */
export function repeatable(turn: { role: string; text: string; images?: readonly string[]; files?: readonly unknown[]; sharedPage?: boolean }): boolean {
  return !hostWrittenTurn(turn) && !carriesAttachments(turn);
}

/** The fields of a chat turn Edit reads (a subset of `ChatTurn`). */
export interface EditTurn {
  role: 'user' | 'assistant' | 'system';
  text: string;
}

/** The index of the message Edit puts back in the box, or -1. Offered on
 *  your last message only, while nothing runs and nothing is queued (a
 *  queued message has its own Edit). Hollow can't rewrite history, so
 *  sending it again is a new message and the earlier answer stays.
 *  `skip` says a message can't be put back as it went (ChatBody passes
 *  `!repeatable`: one Hollow wrote, or one with images, files or a shared page). A
 *  message with no words (an image only) has nothing to put back. */
export function editIndex<T extends EditTurn>(turns: readonly T[], state: { busy: boolean; queued: boolean }, skip: (turn: T) => boolean = () => false): number {
  if (state.busy || state.queued) return -1;
  for (let i = turns.length - 1; i >= 0; i--) {
    if (turns[i].role !== 'user') continue;
    return !skip(turns[i]) && turns[i].text.trim() ? i : -1;
  }
  return -1;
}

/** What ↑ recalls in an empty box (overnight 11-07, D1; Claude's ↑ edits the
 *  last message of this chat): this conversation's own messages first,
 *  newest first, then the earlier prompts from every chat (`chatHistory`).
 *  Hollow's own turns (a wake, Continue, card answers) aren't yours to
 *  recall, and a message whose text still holds an attachment block can't
 *  go back as words, so both are skipped; a repeated message appears once,
 *  at its newest place. Read when ↑ or ↓ is pressed, never as a memo: it
 *  would read every turn on every streamed token. */
export function recallList(turns: readonly { role: string; text: string }[], earlier: readonly string[], cap = 200): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  const add = (raw: string) => {
    const text = raw.trim();
    if (!text || seen.has(text)) return;
    seen.add(text);
    out.push(text);
  };
  for (let i = turns.length - 1; i >= 0 && out.length < cap; i--) {
    const turn = turns[i];
    if (turn.role !== 'user' || hostWrittenTurn(turn) || /<(?:attached-images|attached-files|shared-page)[\s>]/.test(turn.text)) continue;
    add(visibleUserText(turn.text));
  }
  for (const text of earlier) {
    if (out.length >= cap) break;
    add(text);
  }
  return out;
}

/** Where a recall stands (overnight 11-07, D1, from a Codex read): the place ↑
 *  and ↓ keep is an index into a list rebuilt on every key press, so a turn
 *  that arrives or starts from the queue can shift it, and text put in the
 *  box another way (a Quote) leaves it pointing at a message the box no
 *  longer holds. The box's own text settles it: still the message at the
 *  place, the place stands; the same message elsewhere in the list, that
 *  place; anything else, null (not recalling: ↓ must not wipe what is there). */
export function recallAnchor(list: readonly string[], place: number | null, shown: string): number | null {
  if (place == null) return null;
  const flat = (s: string) => s.replace(/\r\n?/g, '\n').trim();
  const text = flat(shown);
  if (place >= 0 && place < list.length && flat(list[place]) === text) return place;
  const moved = list.findIndex(entry => flat(entry) === text);
  return moved >= 0 ? moved : null;
}

/** Whether the caret sits on the box's first or last shown line (ENG-93): a
 *  long message wraps, so ↑ on its second shown line must move the caret,
 *  not recall. `height` is the box's content height for a text at its own
 *  width (0 when nothing is laid out, as in node): the text before the caret
 *  is as tall as one line when the caret is on the first, as tall as the
 *  whole text when on the last. Without layout, the `\n` lines decide. */
export function caretEdges(value: string, caret: number, height: (text: string) => number): { atTop: boolean; atBottom: boolean } {
  const firstBreak = value.indexOf('\n');
  const byLines = { atTop: caret <= (firstBreak < 0 ? value.length : firstBreak), atBottom: caret >= value.lastIndexOf('\n') + 1 };
  const one = height('x');
  if (!one) return byLines;
  const before = height(value.slice(0, caret) || 'x');
  return { atTop: byLines.atTop && before <= one, atBottom: byLines.atBottom && before >= height(value || 'x') };
}

/** When a message was sent, as the reply footer says its finish time:
 *  `10:21 PM` today, `Nov 3, 10:21 PM` on an older day, with the full date
 *  and time for its tooltip. Null for a missing or bad time, so a message
 *  shows no time rather than a wrong one. */
export function sentTime(at: string | undefined, now = Date.now(), locale?: string): { label: string; title: string } | null {
  const label = finishedText(at, now, locale);
  if (!label) return null;
  const title = new Date(Date.parse(at!)).toLocaleString(locale, {
    weekday: 'long', year: 'numeric', month: 'long', day: 'numeric', hour: 'numeric', minute: '2-digit',
  });
  return { label, title: `Sent ${title}` };
}
