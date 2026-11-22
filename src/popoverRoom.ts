/**
 * How tall a popover that opens upward from a composer control may be
 * (overnight 2026-11-04, lane C, C2). The model menu's popover had a fixed
 * budget (`100dvh - 208px`) that fit the workspace chat, but Library's
 * Conversations tab puts the vault chat under the page heading, the tabs
 * and the conversation header inside a pane that clips, so at 720x480 the
 * popover's search box sat above the pane, cut off. The room is measured
 * when it opens instead: up to the nearest ancestor that clips, never above
 * the 44px title bar, less a margin.
 *
 * `roomAbove` is pure; `clipTopsAbove` reads the DOM it is handed.
 */

/** The title bar's height: nothing opens over it. */
export const TITLE_BAR = 44;
/** Never shorter than the search and a row or two (it scrolls inside). */
export const MIN_ROOM = 120;

/** Height available above `triggerTop`, below the lowest clipping top. */
export function roomAbove(triggerTop: number, clipTops: readonly number[], margin = 12): number {
  const ceiling = Math.max(TITLE_BAR, ...clipTops);
  return Math.max(0, Math.round(triggerTop - ceiling - margin));
}

/** The tops of `el`'s ancestors that clip their overflow vertically. */
export function clipTopsAbove(el: Element): number[] {
  const tops: number[] = [];
  const view = el.ownerDocument.defaultView;
  if (!view) return tops;
  for (let p = el.parentElement; p && p !== el.ownerDocument.body; p = p.parentElement) {
    if (view.getComputedStyle(p).overflowY !== 'visible') tops.push(p.getBoundingClientRect().top);
  }
  return tops;
}
