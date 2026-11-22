/**
 * "Compare versions in the gallery" (plan L3 X7, 2026-10-14; review U
 * canvases). A reply, the side panel and Results have no room for two
 * frames side by side, so their ⋯ menu opens the gallery (Library → Results
 * → Canvases) on that canvas with Compare on, and the gallery offers a way
 * back. The gallery's own route (`store/libraryMode.ts`) publishes this
 * request only after navigation is accepted. It is taken once by the
 * gallery when it opens. No signal: the open itself (`openCanvasGallery`)
 * is what the gallery reacts to.
 */

export interface GalleryCanvasRequest {
  workspaceId: string;
  /** The canvas's id in its store. */
  id: string;
  /** Open with Compare versions on. */
  compare: boolean;
  /** Where the person came from: the gallery's Back button. */
  back: { label: string; go: () => void } | null;
}

let pending: GalleryCanvasRequest | null = null;

/** Only the accepted Library route publishes a request; an ordinary open clears it. */
export function requestGalleryCanvas(request: GalleryCanvasRequest | null) { pending = request; }

/** The request, once: the gallery takes it as it opens. */
export function takeGalleryCanvas(): GalleryCanvasRequest | null {
  const r = pending;
  pending = null;
  return r;
}
