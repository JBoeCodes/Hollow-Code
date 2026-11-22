// Settings help text, cut after its first sentence (overnight 11-07, lane D3).
// A section or sub-header intro of three or four sentences reads as a wall
// beside Claude's and ChatGPT's one-line descriptions, so the first sentence
// stays in view and the rest sits behind a visible "More" (`LeadText` in
// `settingsLayout.tsx`). Nothing is dropped: the rest stays in the page. Pure,
// so the node vitest suite can load it.

/** A help text longer than this is cut after its first sentence. */
export const LEAD_MAX = 150;
/** A first sentence shorter than this is not a lead worth cutting at. */
const LEAD_MIN = 40;
/**
 * The end of the text before a stop is an abbreviation or a one or two letter
 * word ("e.g", "i.e", "U.S", "vs", "Dr"): the stop is probably not a sentence
 * end, so the cut moves on to the next one. A real sentence that ends in a
 * short word ("... leave it on. Then ...") only keeps one more sentence in the
 * lead, which is harmless; a cut in the middle of a sentence is not.
 */
const ABBREVIATION_BEFORE_STOP = /(?:^|[\s(])(?:[A-Za-z]{1,2}|(?:[A-Za-z]\.){1,3}[A-Za-z])$/;

/**
 * `[lead, rest]` for a long help text: the lead is the first sentence (a full
 * stop followed by a space and a capital letter or a digit, at least
 * {@link LEAD_MIN} characters in, and not after an abbreviation such as
 * "e.g." or a one or two letter word), the rest is everything after it (the
 * match needs a capital or a digit after the stop, so the rest is never
 * empty). `null` when the text is no longer than `max` or has no sentence
 * break to cut at: such a text is shown whole.
 */
export function splitLead(text: string, max: number = LEAD_MAX): [lead: string, rest: string] | null {
  if (text.length <= max) return null;
  const sentenceEnd = /\.\s+(?=[A-Z0-9])/g;
  for (let m = sentenceEnd.exec(text); m; m = sentenceEnd.exec(text)) {
    const end = m.index + 1;
    if (end >= LEAD_MIN && !ABBREVIATION_BEFORE_STOP.test(text.slice(0, m.index))) {
      return [text.slice(0, end), text.slice(m.index + m[0].length)];
    }
  }
  return null;
}
