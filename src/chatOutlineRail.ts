/**
 * The chat outline rail (overnight 11-12, E9): one dash per prompt in the
 * chat's left gutter, as in ChatGPT. Pure, unit-tested;
 * components/ChatOutlineRail.tsx owns the UI.
 */

/** Most dashes the strip draws; a longer chat samples its prompts evenly. */
export const RAIL_MAX_DASHES = 40;

/** Which outline positions get a dash: every one up to `cap`, else `cap`
 *  evenly spaced ones that always keep the first and the last prompt. */
export function railDashes(count: number, cap = RAIL_MAX_DASHES): number[] {
  if (count <= 0) return [];
  if (count <= cap) return Array.from({ length: count }, (_, i) => i);
  if (cap < 2) return [0];
  const out: number[] = [];
  for (let k = 0; k < cap; k++) {
    const at = Math.round((k * (count - 1)) / (cap - 1));
    if (out[out.length - 1] !== at) out.push(at);
  }
  return out;
}

/** The dash that stands for outline position `pos`: the last dash at or
 *  before it (a sampled strip marks the stretch you are in). -1 for none. */
export function dashFor(dashes: readonly number[], pos: number): number {
  if (pos < 0) return -1;
  let best = -1;
  for (let i = 0; i < dashes.length && dashes[i] <= pos; i++) best = i;
  return best;
}

/** The prompt (a turn index) whose part of the conversation is in view.
 *  `prompts` are every prompt's turn index in order; `tops` the rendered
 *  prompts' tops against the scroller; `line` the reading line (a third
 *  down the view, or its bottom once scrolled to the end). The last prompt
 *  above the line wins; with none above it, the prompt before the first
 *  rendered one (the render window starts mid-conversation), else the
 *  first. -1 when nothing is rendered or there are no prompts. */
export function currentPrompt(prompts: readonly number[], tops: readonly { index: number; top: number }[], line: number): number {
  if (!prompts.length || !tops.length) return -1;
  let above = -1, firstRendered = Infinity;
  for (const t of tops) {
    if (t.top <= line && t.index > above) above = t.index;
    if (t.index < firstRendered) firstRendered = t.index;
  }
  if (above >= 0) return above;
  let before = -1;
  for (const p of prompts) if (p < firstRendered) before = p;
  return before >= 0 ? before : prompts[0];
}

/** The strip's one tab stop (#1322 re-review): the dash keyboard focus last
 *  moved to while it stays in the strip, else the current prompt's, else the
 *  first; -1 with no dashes. Kept inside `count`, so a strip that re-rendered
 *  shorter never leaves no stop. */
export function railTabStop(count: number, current: number, focused: number | null): number {
  if (count <= 0) return -1;
  const at = focused ?? current;
  return Math.min(count - 1, Math.max(0, at));
}
