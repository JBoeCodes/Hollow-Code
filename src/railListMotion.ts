import { observeKitMotion } from './motionKit';

interface Position { element: HTMLElement; top: number; left: number }

/** One measured list across all shelves, keyed by conversation identity.
 * Solid may recreate a row when it changes shelves. Only its real destination
 * animates, so no cloned buttons, ids or stale focus targets remain behind. */
export function observeRailListMotion(root: HTMLElement): () => void {
  let previous = new Map<string, Position>();
  const animations = new Map<HTMLElement, Animation>();
  let duration = 0, stopped = false;
  const cancel = () => {
    const owned = [...animations.values()]; animations.clear();
    for (const animation of owned) animation.cancel();
  };
  const positions = () => {
    const result = new Map<string, Position>();
    for (const element of root.querySelectorAll<HTMLElement>('[data-rail-motion-key]')) {
      if (!element.getClientRects().length) continue;
      const key = element.dataset.railMotionKey;
      if (!key || result.has(key)) continue;
      const rect = element.getBoundingClientRect();
      // Content coordinates keep ordinary scrolling out of list motion.
      result.set(key, { element, top: rect.top + root.scrollTop, left: rect.left + root.scrollLeft });
    }
    return result;
  };
  const update = () => {
    if (stopped) return;
    // Finish our transforms before reading natural positions. The latest
    // canonical positions remain the next reordering's origin.
    cancel();
    const next = positions();
    if (duration) for (const [key, row] of next) {
      const old = previous.get(key);
      const x = old ? old.left - row.left : 0, y = old ? old.top - row.top : 0;
      if (old && Math.abs(x) < 1 && Math.abs(y) < 1) continue;
      if (!row.element.animate) continue;
      const frames: Keyframe[] = old
        ? [{ transform: `translate(${x}px, ${y}px)` }, { transform: 'translate(0, 0)' }]
        : [{ opacity: 0 }, { opacity: 1 }];
      try {
        const animation = row.element.animate(frames, { duration, easing: 'ease-out' });
        animations.set(row.element, animation);
        const done = () => { if (animations.get(row.element) === animation) animations.delete(row.element); };
        animation.onfinish = done; animation.oncancel = done;
      } catch { /* Static layout remains usable without an animation engine. */ }
    }
    previous = next;
  };
  const policy = observeKitMotion(root, value => {
    if (!value || !duration) { cancel(); previous = positions(); }
    duration = value;
  });
  previous = positions();
  const observer = typeof MutationObserver === 'undefined' ? undefined : new MutationObserver(update);
  // Timers change text, not the row list. Ignore character and class updates.
  observer?.observe(root, { childList: true, subtree: true });
  return () => { stopped = true; observer?.disconnect(); policy(); cancel(); previous.clear(); };
}
