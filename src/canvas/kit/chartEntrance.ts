/** One short entrance over a cached data image. Only transform animates:
 *  no paint loop, no timer loop, no layout read per frame. The canvas keeps
 *  axes and exact labels still. Offscreen/hidden work is paused; teardown
 *  drops every observer, animation and listener. */
import { uid } from './registry';

export function motionAllowed(): boolean {
  const api = (window as unknown as { hollowCanvas?: { init: unknown } }).hollowCanvas;
  return !(api && !api.init) && document.documentElement.dataset.hcMotion !== 'reduce'
    && !matchMedia('(prefers-reduced-motion: reduce)').matches;
}
function s(tag: string, attrs: Record<string, string | number>): SVGElement {
  const el = document.createElementNS('http://www.w3.org/2000/svg', tag);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, String(v));
  return el;
}

export function chartEntrance(host: HTMLElement, plot: HTMLElement, snapshot: () => string, w: number, h: number,
  kind: string, base: number, start: () => void, finish: () => void): () => void {
  if (!motionAllowed() || typeof IntersectionObserver === 'undefined' || !Element.prototype.animate) return () => {};
  const make = () => {
  const image = snapshot();
  const layer = s('svg', { viewBox: `0 0 ${w} ${h}`, class: 'hc-chart-entrance', 'aria-hidden': 'true' });
  const defs = s('defs', {}); layer.append(defs);
  const targets: Array<{ el: SVGElement; from: string; delay: number; duration: number }> = [];
  const picture = () => s('image', { href: image, width: w, height: h });
  if (kind === 'donut') {
    // Two independently clipped halves sweep clockwise from twelve. Both
    // clips rotate; their fixed half bounds keep the other half untouched.
    for (let i = 0; i < 2; i++) {
      const fixedId = uid('hc-half'), movingId = uid('hc-sweep');
      const fixed = s('clipPath', { id: fixedId }), moving = s('clipPath', { id: movingId });
      fixed.append(s('rect', { x: i ? 0 : w / 2, y: 0, width: w / 2, height: h }));
      const rect = s('rect', { x: i ? -w : w / 2, y: -h, width: w * 1.5, height: h * 3 });
      rect.style.transformOrigin = `${w / 2}px ${h / 2}px`;
      moving.append(rect); defs.append(fixed, moving);
      const half = s('g', { 'clip-path': `url(#${fixedId})` }), sweep = s('g', { 'clip-path': `url(#${movingId})` });
      sweep.append(picture()); half.append(sweep); layer.append(half);
      targets.push({ el: rect, from: 'rotate(-180deg)', delay: i * 420, duration: 420 });
    }
  } else if (kind === 'bar') {
    const img = picture(); img.style.transformOrigin = `0px ${base}px`; layer.append(img);
    targets.push({ el: img, from: 'scaleY(0)', delay: 0, duration: 700 });
  } else {
    const id = uid('hc-trace'), clip = s('clipPath', { id });
    const rect = s('rect', { x: 0, y: 0, width: w, height: h });
    clip.append(rect); defs.append(clip);
    const img = picture(); img.setAttribute('clip-path', `url(#${id})`); layer.append(img);
    targets.push({ el: rect, from: 'scaleX(0)', delay: 0, duration: 800 });
  }
  return { layer, targets };
  };
  let layer: SVGElement | null = null;
  let active = false, visible = false, dead = false;
  let animations: Animation[] = [];
  const media = matchMedia('(prefers-reduced-motion: reduce)');
  const dispose = () => {
    if (dead) return;
    dead = true; observer.disconnect(); rootObserver.disconnect();
    document.removeEventListener('visibilitychange', refresh);
    media.removeEventListener('change', refresh);
    for (const a of animations) { a.onfinish = null; a.cancel(); }
    layer?.remove();
  };
  const complete = () => { dispose(); finish(); };
  const refresh = () => {
    if (dead) return;
    if (!host.isConnected || !motionAllowed()) { complete(); return; }
    if (!visible || document.hidden) { for (const a of animations) a.pause(); return; }
    if (!active) {
      active = true;
      const visual = make(); layer = visual.layer; start(); plot.append(layer);
      animations = visual.targets.map(t => t.el.animate([{ transform: t.from }, { transform: 'none' }],
        // `id` marks it as the kit's (transform only, inside the chart), which
        // the design check's measuring leaves out (ideas 10-27 #4).
        { id: 'hc-kit', duration: t.duration, delay: t.delay, easing: kind === 'donut' ? 'linear' : 'cubic-bezier(.2,.7,.2,1)', fill: 'both' }));
      animations[animations.length - 1].onfinish = complete;
    } else for (const a of animations) a.play();
  };
  const observer = new IntersectionObserver(entries => { visible = entries.some(e => e.isIntersecting); refresh(); });
  const rootObserver = new MutationObserver(refresh);
  rootObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['data-hc-motion'] });
  observer.observe(host);
  document.addEventListener('visibilitychange', refresh);
  media.addEventListener('change', refresh);
  return dispose;
}
