// One app-side motion policy. No store imports: safe for small UI helpers.
const mediaByWindow = new WeakMap<Window, MediaQueryList>();
const currentWindow = () => typeof window === 'undefined' ? undefined : window;

/** Live system query, also used by views that must react to a change. */
export function motionQuery(view: Window | undefined = currentWindow()): MediaQueryList | undefined {
  if (!view?.matchMedia) return undefined;
  let media = mediaByWindow.get(view);
  if (!media) {
    media = view.matchMedia('(prefers-reduced-motion: reduce)');
    mediaByWindow.set(view, media);
  }
  return media;
}

/** Either the app setting or system preference reduces motion. Read live;
 * an injected document uses its own window (view-transition tests/frames). */
export function motionReduced(doc: Document | undefined = typeof document === 'undefined' ? undefined : document): boolean {
  return doc?.documentElement.dataset.motion === 'reduced'
    || !!motionQuery(doc?.defaultView ?? currentWindow())?.matches;
}
