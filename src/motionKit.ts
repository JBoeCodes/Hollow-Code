import { motionQuery, motionReduced } from './motion';

/** Read the panel setting's published token. No second preference or default. */
export function kitDuration(element: HTMLElement): number {
  const doc = element.ownerDocument;
  if (motionReduced(doc) || doc.hidden) return 0;
  const value = doc.defaultView?.getComputedStyle(element).getPropertyValue('--panel-duration').trim() ?? '';
  const milliseconds = Number.parseFloat(value) * (value.endsWith('ms') ? 1 : 1000);
  return Number.isFinite(milliseconds) ? Math.max(0, Math.min(400, milliseconds)) : 0;
}

/** Changes to either motion preference cancel owned animation immediately.
 * Offscreen cues stay still; a missing visibility API is a static fallback. */
export function observeKitMotion(element: HTMLElement, change: (duration: number) => void): () => void {
  const doc = element.ownerDocument, view = doc.defaultView;
  let visible = false;
  let stopped = false;
  const update = () => change(visible ? kitDuration(element) : 0);
  const media = motionQuery(view ?? undefined);
  const forced = view?.matchMedia?.('(forced-colors: active)');
  const refresh = () => {
    if (stopped) return;
    element.dataset.motionPlaying = visible && kitDuration(element) > 0 && !forced?.matches ? 'true' : 'false';
    update();
  };
  const intersection = typeof IntersectionObserver === 'undefined' ? undefined : new IntersectionObserver(entries => {
    visible = entries.some(entry => entry.isIntersecting);
    refresh();
  });
  intersection?.observe(element);
  const mutation = typeof MutationObserver === 'undefined' ? undefined : new MutationObserver(refresh);
  mutation?.observe(doc.documentElement, { attributes: true });
  media?.addEventListener('change', refresh);
  forced?.addEventListener('change', refresh);
  doc.addEventListener('visibilitychange', refresh);
  refresh();
  return () => {
    stopped = true;
    intersection?.disconnect(); mutation?.disconnect();
    media?.removeEventListener('change', refresh);
    forced?.removeEventListener('change', refresh);
    doc.removeEventListener('visibilitychange', refresh);
    delete element.dataset.motionPlaying;
  };
}

/** Measured dimensions, independent of Solid so lifecycle contracts can use
 * deterministic resize and animation drivers. The natural size is restored
 * after completion, and stale completions cannot clear a newer animation. */
export function measureMotionSize(outer: HTMLElement, inner: HTMLElement, axis: 'height' | 'width'): () => void {
  let previous: number | undefined;
  let animation: Animation | undefined;
  let duration = 0;
  let disposed = false;
  const natural = () => Math.ceil(inner.getBoundingClientRect()[axis]);
  const clear = () => {
    const old = animation; animation = undefined; old?.cancel();
    outer.style.removeProperty(axis);
    outer.style.removeProperty('overflow');
    outer.style.removeProperty('overflow-clip-margin');
  };
  const resize = () => {
    if (disposed) return;
    const next = natural();
    if (previous === undefined) { previous = next; return; }
    if (next === previous) return;
    const from = animation ? outer.getBoundingClientRect()[axis] : previous;
    previous = next;
    clear();
    if (!duration || !outer.animate || from === next) return;
    outer.style.setProperty(axis, `${next}px`);
    outer.style.setProperty('overflow', 'clip');
    // Keep the shared 2px outline plus its offset visible while sizing.
    outer.style.setProperty('overflow-clip-margin', 'calc(var(--focus-offset, 2px) + 2px)');
    const easing = outer.ownerDocument.defaultView?.getComputedStyle(outer).getPropertyValue('--ease-decel').trim() || 'ease-out';
    let current: Animation;
    try {
      current = outer.animate([{ [axis]: `${from}px` }, { [axis]: `${next}px` }], { duration: axis === 'width' ? duration * .75 : duration, easing });
    } catch {
      // Invalid custom easing or an unavailable animation engine must never
      // strand content behind an explicit size or clipped focus ring.
      clear(); return;
    }
    animation = current;
    current.onfinish = () => { if (animation === current) clear(); };
    current.oncancel = () => { if (animation === current) clear(); };
  };
  const stopPolicy = observeKitMotion(outer, value => {
    // Entering the viewport or re-enabling motion starts from current layout,
    // not an old hidden measurement or a partially completed transition.
    if (!value || !duration) { clear(); previous = natural(); }
    duration = value;
  });
  const observer = typeof ResizeObserver === 'undefined' ? undefined : new ResizeObserver(resize);
  observer?.observe(inner);
  resize();
  return () => { disposed = true; observer?.disconnect(); stopPolicy(); clear(); };
}
