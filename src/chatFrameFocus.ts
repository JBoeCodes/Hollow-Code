/**
 * A click inside a transcript frame is the person's own interaction (H10,
 * 2026-10-15). Canvases and HTML artifacts render in iframes sandboxed
 * `allow-scripts` without same-origin, so none of their pointer, wheel or key
 * events reach the transcript's intent listeners (chatScrollController), and
 * nothing can listen inside them. What the page does see: focus moving into
 * the frame blurs this window and leaves `document.activeElement` on the
 * `<iframe>`. That counts as intent only for a frame inside this transcript's
 * scroller: not the side panel's copy of a canvas, nor any other frame.
 *
 * Pure so the node Vitest suite covers it; the controller passes
 * `document.activeElement` and its scroller.
 */
export function isTranscriptFrameFocus<N extends { tagName: string }>(
  active: N | null | undefined,
  scroller: { contains(node: N): boolean } | null | undefined,
): boolean {
  return !!active && !!scroller && active.tagName === 'IFRAME' && scroller.contains(active);
}
