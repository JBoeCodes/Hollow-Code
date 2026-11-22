import { createRoot, onCleanup, type JSX } from 'solid-js';

/**
 * Renders `children` with no Suspense boundary above them (consolidation lane
 * B, 2026-10-13). A surface written as an overlay mounted by the shell
 * (OnceOpened: "no Suspense boundary, so the surface's own resources never
 * hold it offscreen") reads its resources directly and draws its own
 * loading and error lines. Moved into a page, it would sit under that page's
 * Suspense boundary, and every pending read (a first load, a Refresh) would
 * swap the whole view for the boundary's fallback. Rendering it in a root of
 * its own, with no inherited context, keeps the overlay-era behaviour; the
 * root is disposed with the parent. A lazy component inside renders nothing
 * until its chunk arrives (preload it to avoid the blank). The root has no
 * error handler either: put a `RecoverableView` (`suspends={false}`) inside
 * it, or an error leaves the area blank (run 13 FIX-EB).
 */
export function OutsideSuspense(props: { children: () => JSX.Element }) {
  let dispose: (() => void) | undefined;
  onCleanup(() => dispose?.());
  return createRoot(d => { dispose = d; return props.children(); }, null);
}
