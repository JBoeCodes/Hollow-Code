/**
 * Long messages of yours fold behind "Show more" (overnight 2026-11-04, lane
 * C, a C3 follow-up), as in the ChatGPT and Claude desktop apps: a pasted
 * 200-line prompt used to fill the whole transcript. `components/ChatBody.tsx`
 * shows a folded message's first `FOLDED_LINES` lines, faded at the foot, with
 * a Show more button under them; `styles/chat-workspace.css` holds the height
 * and the fade.
 *
 * The fold is decided from the text, never measured, so it is there from the
 * first paint: no measure-then-collapse jump and no ResizeObserver. Only the
 * words fold; images and the queued badge stay as they are. Pure.
 */
import { visibleUserText } from './canvas/askAnswer';

/** A message with more lines than this folds. A folded one shows
 *  `FOLDED_LINES`, so one folded by its lines always has at least four more
 *  under Show more. */
export const FOLD_LINES = 12;

/** The lines a folded message shows: the `9lh` in styles/chat-workspace.css. */
export const FOLDED_LINES = 9;

/**
 * A message longer than this many characters folds too: a long paragraph
 * wraps past `FOLD_LINES` with few line breaks of its own. The bubble is at
 * most 85% of the chat column, in 13px Inter at about 6.5px a character:
 * roughly 65 characters a line in the default 1,180px window beside the
 * sidebar and the conversation list, and about 140 in a maximized 1,728px
 * one. 1,500 characters wrap to about 11 lines in the wide window, still
 * past the nine a fold shows, and to over 20 in the default one. A lower
 * number would fold messages a wide window shows in under nine lines, where
 * Show more reveals nothing; a higher one leaves long paragraphs unfolded
 * in the default window.
 *
 * Both numbers count line breaks and characters, never the lines a message
 * wraps to, so the fold is only as good as that guess: on a very wide
 * display a message just past 1,500 characters in few lines can fit in
 * nine and still fold, and CJK text, whose characters are about twice as
 * wide, wraps to about twice the lines before it folds.
 */
export const FOLD_CHARS = 1500;

/** Whether the visible words of a message of yours fold behind Show more:
 *  more than `FOLD_LINES` lines (counted by line breaks: how a line wraps
 *  isn't known here) or `FOLD_CHARS` characters, not counting trailing
 *  blank lines and spaces. A `\r\n` is one line break, as `\n` is; blank
 *  text never folds. */
export function foldsUserText(text: string | null | undefined): boolean {
  const words = (text ?? '').trimEnd();
  if (!words) return false;
  if (words.length > FOLD_CHARS) return true;
  let lines = 1;
  for (let at = words.indexOf('\n'); at !== -1 && lines <= FOLD_LINES; at = words.indexOf('\n', at + 1)) lines++;
  return lines > FOLD_LINES;
}

/** While Find in the conversation (⌘F) is open with a query it searches
 *  (two characters or more, the floor `findInTurns` uses), every folded
 *  message shows whole, so no match is hidden under a fold. Closing Find
 *  folds them again, except the one it was on (`findKeepsOpen`); clearing
 *  the query folds them all. */
export function findShowsWhole(findOpen: boolean, query: string): boolean {
  return findOpen && query.trim().length >= 2;
}

/** As Find closes, the turn it was on (`hits[at]`) when that is a long
 *  message of yours, which then stays open: closing Find would otherwise
 *  fold away the match it had just taken you to. Null for anything else
 *  (a reply, a short message, no match). */
export function findKeepsOpen(turns: readonly { role: string; text: string }[], hits: readonly number[], at: number): number | null {
  const index = hits[at];
  const turn = index == null ? undefined : turns[index];
  return turn?.role === 'user' && foldsUserText(visibleUserText(turn.text)) ? index : null;
}
