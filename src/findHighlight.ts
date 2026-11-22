/**
 * Find in this conversation, marked in the text (overnight 2026-11-04, lane
 * C, C4). The find bar (⌘F) outlined the turn holding the current match but
 * left the words themselves unmarked, so a long reply hid where the match
 * was. Now every match in the rendered transcript is marked, and those in
 * the current turn more strongly, as a browser's or ChatGPT's find does.
 *
 * It paints with the CSS Custom Highlight API, so the rendered Markdown is
 * never touched (no wrapping spans to undo, nothing for streaming text or a
 * canvas to trip over). Where the API is missing the turn outline alone
 * stays, as before.
 *
 * Only the containers that hold a turn's searched text are walked
 * (`FIND_TEXT`): reply and narration Markdown, code blocks, your message's
 * words and a wake note's. Labels, tool rows, footers and controls are never
 * marked, since `findInTurns` doesn't search them. The current turn is
 * marked first, so a common query in a long thread can't use up the cap
 * before the reply you are on.
 *
 * `matchOffsets` is pure; the painters read the DOM they are handed.
 */

/** Every match in the rendered turns. */
export const FIND_ALL = 'chat-find';
/** The matches in the turn the find bar is on. */
export const FIND_CURRENT = 'chat-find-current';

/** The elements holding a turn's searched text. */
export const FIND_TEXT = '.md-view, .chat-codeblock-pre, .chat-user-chip .text, .chat-host-note .text';
/** Inside a turn but not its text: tool rows and the run record. */
const NOT_TEXT = '.toolcalls, .toolcall, .chat-tool-ledger, .run-record';

/** At most this many marks besides the current turn's, and at most this
 *  many in the current turn. */
export const MAX_MARKS = 2000;

/** Where `query` starts in `text`: case-insensitive, non-overlapping. Under
 *  2 characters finds nothing, as `findInTurns`. Empty when lower-casing
 *  changes the text's or the query's length (a few non-Latin letters),
 *  since the offsets would then point at the wrong characters. */
export function matchOffsets(text: string, query: string): number[] {
  const trimmed = query.trim();
  const q = trimmed.toLowerCase();
  if (q.length < 2 || q.length !== trimmed.length || text.length < q.length) return [];
  const hay = text.toLowerCase();
  if (hay.length !== text.length) return [];
  const out: number[] = [];
  for (let at = hay.indexOf(q); at !== -1; at = hay.indexOf(q, at + q.length)) out.push(at);
  return out;
}

/** Two hit lists with the same turns, in order. */
export function sameHits(a: readonly number[], b: readonly number[]): boolean {
  return a.length === b.length && a.every((v, i) => v === b[i]);
}

/** Whether a DOM change can move the marks: it touched searched text or
 *  added some (a footer's clock ticking every second can't). */
export function touchesFindText(records: readonly MutationRecord[]): boolean {
  for (const r of records) {
    const el = r.target.nodeType === 1 ? r.target as Element : r.target.parentElement;
    if (el?.closest(FIND_TEXT)) return true;
    for (const n of r.addedNodes) {
      if (n.nodeType === 1 && ((n as Element).matches(FIND_TEXT) || (n as Element).querySelector(FIND_TEXT))) return true;
    }
  }
  return false;
}

type Registry = { set(name: string, value: unknown): unknown; delete(name: string): boolean };
type HighlightCtor = new (...ranges: Range[]) => unknown;

function api(): { registry: Registry; Highlight: HighlightCtor } | null {
  const g = globalThis as { CSS?: { highlights?: Registry }; Highlight?: HighlightCtor };
  return g.CSS?.highlights && typeof g.Highlight === 'function' ? { registry: g.CSS.highlights, Highlight: g.Highlight } : null;
}

/** Removes both marks. */
export function clearFindMarks(): void {
  const a = api();
  if (!a) return;
  a.registry.delete(FIND_ALL);
  a.registry.delete(FIND_CURRENT);
}

/** Ranges of `query` in the text nodes under `box`, at most `room` more. */
function rangesIn(box: Element, query: string, room: number, into: Range[]) {
  const len = query.trim().length;
  const doc = box.ownerDocument;
  const walker = doc.createTreeWalker(box, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node && into.length < room; node = walker.nextNode()) {
    const text = node.nodeValue ?? '';
    // An image placeholder's Load button sits inside the Markdown; not text.
    if (text.length < len || node.parentElement?.closest('button')) continue;
    for (const at of matchOffsets(text, query).slice(0, room - into.length)) {
      const range = doc.createRange();
      range.setStart(node, at);
      range.setEnd(node, at + len);
      into.push(range);
    }
  }
}

/** Marks `query` in the turns under `root` (each text's turn is its
 *  innermost `[data-turn-index]`), the `current` turn's matches first and
 *  apart. Returns how many it marked (0 without the API). */
export function paintFindMarks(root: Element | null | undefined, query: string, current: number | null): number {
  const a = api();
  if (!a) return 0;
  clearFindMarks();
  if (!root || query.trim().length < 2) return 0;
  const now: Element[] = [];
  const rest: Element[] = [];
  for (const box of root.querySelectorAll(FIND_TEXT)) {
    // Walked once: a container inside another (code in Markdown) is the outer one's.
    if (box.closest(NOT_TEXT) || box.parentElement?.closest(FIND_TEXT)) continue;
    const owner = box.closest('[data-turn-index]');
    // System notices aren't searched (findInTurns skips them).
    if (!owner || owner.closest('.turn.system')) continue;
    (current != null && owner.getAttribute('data-turn-index') === String(current) ? now : rest).push(box);
  }
  const mine: Range[] = [];
  const all: Range[] = [];
  for (const box of now) rangesIn(box, query, MAX_MARKS, mine);
  for (const box of rest) { if (all.length >= MAX_MARKS) break; rangesIn(box, query, MAX_MARKS, all); }
  if (mine.length) a.registry.set(FIND_CURRENT, new a.Highlight(...mine));
  if (all.length) a.registry.set(FIND_ALL, new a.Highlight(...all));
  return mine.length + all.length;
}
