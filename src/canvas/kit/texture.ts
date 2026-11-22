/** Named, static presentation only. Never changes data, answers or view state. */
export type CanvasTexture = 'plain' | 'dither' | 'ascii';
export const TEXTURE_VETO = '(prefers-reduced-motion: reduce), (prefers-reduced-transparency: reduce), (prefers-contrast: more), (forced-colors: active)';

export function requestedTexture(value: string | null): CanvasTexture {
  const v = value?.trim().toLowerCase();
  return !v || v === 'dither' ? 'dither' : v === 'ascii' ? 'ascii' : 'plain';
}

/** The runtime owns this frozen getter. A request in markup cannot grant
 * permission. Missing/older hosts fail closed; the nearest explicit request
 * wins, including unknown values (plain). The host forwards its app policy;
 * frame media queries independently veto it. */
export function textureOf(el: Element): CanvasTexture {
  const host = (window as unknown as { hollowCanvas?: { readonly textureAllowed?: boolean } }).hollowCanvas;
  if (host?.textureAllowed !== true) return 'plain';
  try { if (matchMedia(TEXTURE_VETO).matches) return 'plain'; } catch { return 'plain'; }
  const request = el.closest('[texture]');
  const value = request?.getAttribute('texture') ?? null;
  if (value?.trim().toLowerCase() === 'plain' && request?.matches('hc-canvas,[data-hc-texture-root]')) return 'dither';
  return requestedTexture(value);
}

const listeners = new Set<() => void>();
let stop: (() => void) | undefined;
/** One observer/media subscription per frame, present only while a supported
 * graphic is connected. Attribute changes never rebuild controls or answers. */
export function watchTextures(callback: () => void): () => void {
  if (!stop) {
    const changed = () => listeners.forEach(fn => fn());
    const cleanups: Array<() => void> = [];
    const cleanup = () => { for (const fn of cleanups) { try { fn(); } catch { /* best effort */ } } };
    try {
      const media = matchMedia(TEXTURE_VETO);
      const observer = new MutationObserver(changed);
      cleanups.push(() => observer.disconnect());
      observer.observe(document.documentElement, { attributes: true, subtree: true, attributeFilter: ['texture'] });
      cleanups.push(() => removeEventListener('hollow-texture', changed));
      addEventListener('hollow-texture', changed);
      cleanups.push(() => media.removeEventListener('change', changed));
      media.addEventListener('change', changed);
      stop = cleanup;
    } catch { cleanup(); return () => {}; }
  }
  listeners.add(callback);
  return () => { listeners.delete(callback); if (!listeners.size) { stop?.(); stop = undefined; } };
}
