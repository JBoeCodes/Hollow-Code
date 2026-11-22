/** Flow animation has no JS frame loop. Geometry is sampled once on first
 * visibility; the browser animates transforms. Every animated element has
 * its own visibility gate, including elements clipped by the flow viewport. */
import { motionAllowed } from './chartEntrance';
export function flowMotion(host: HTMLElement, nodes: SVGElement[], links: SVGElement[]): () => void {
    if (typeof IntersectionObserver === 'undefined' || !Element.prototype.animate)
        return () => { };
    const abort = new AbortController(), media = matchMedia('(prefers-reduced-motion: reduce)');
    type Item = {
        el: SVGElement;
        target: SVGElement;
        visible: boolean;
        entered: boolean;
        hover: boolean;
        focus: boolean;
        animation?: Animation;
        frames?: Keyframe[];
    };
    const items: Item[] = [...nodes, ...links].map(el => ({ el, target: el.querySelector<SVGElement>('.hc-flow-route') || el, visible: false, entered: false, hover: false, focus: false }));
    let dead = false;
    const settle = (item: Item) => { item.animation?.cancel(); item.animation = undefined; };
    const refresh = () => {
        if (dead || !host.isConnected)
            return;
        let particles = 0;
        const ordered = [...items].sort((a, b) => Number(b.hover || b.focus) - Number(a.hover || a.focus));
        for (const item of ordered) {
            const link = item.el.classList.contains('hc-flow-link');
            const dot = item.el.querySelector<SVGElement>('.hc-flow-particle');
            const requested = link ? item.el.dataset.active === 'true' || item.hover || item.focus : !item.entered;
            const wanted = requested && (!link || !item.visible || particles++ < 8);
            if (!motionAllowed() || !wanted) {
                settle(item);
                if (dot)
                    dot.style.opacity = '0';
                if (!link && (document.documentElement.dataset.hcMotion === 'reduce' || media.matches))
                    item.entered = true;
                continue;
            }
            if (!item.visible || document.hidden) {
                item.animation?.pause();
                continue;
            }
            if (!item.animation) {
                if (link) {
                    const path = item.el.querySelector<SVGPathElement>('.hc-flow-route');
                    if (!path || !dot)
                        continue;
                    if (!item.frames) {
                        const length = path.getTotalLength();
                        const count = Math.min(240, Math.max(16, Math.ceil(length / 6)));
                        item.frames = Array.from({ length: count + 1 }, (_, i) => { const p = path.getPointAtLength(length * i / count); return { transform: `translate(${p.x}px, ${p.y}px)`, offset: i / count }; });
                    }
                    dot.style.opacity = '1';
                    item.animation = dot.animate(item.frames, { id: 'hc-kit', duration: 2400, iterations: Infinity, easing: 'linear' });
                }
                else {
                    item.animation = item.el.animate([{ transform: 'translateY(5px)', opacity: 0.65 }, { transform: 'none', opacity: 1 }], { id: 'hc-kit', duration: 420, easing: 'cubic-bezier(.2,.7,.2,1)' });
                    item.animation.onfinish = () => { item.entered = true; settle(item); };
                }
            }
            else
                item.animation.play();
        }
    };
    const observer = new IntersectionObserver(entries => { for (const e of entries) {
        const item = items.find(i => i.target === e.target);
        if (item)
            item.visible = e.isIntersecting;
    } refresh(); });
    for (const item of items) {
        observer.observe(item.target);
        if (links.includes(item.el))
            for (const [event, key, value] of [['pointerenter', 'hover', true], ['pointerleave', 'hover', false], ['focus', 'focus', true], ['blur', 'focus', false]] as const) {
                item.el.addEventListener(event, () => { item[key] = value; refresh(); }, { signal: abort.signal });
            }
    }
    const root = new MutationObserver(refresh);
    root.observe(document.documentElement, { attributes: true, attributeFilter: ['data-hc-motion'] });
    document.addEventListener('visibilitychange', refresh, { signal: abort.signal });
    media.addEventListener('change', refresh, { signal: abort.signal });
    addEventListener('hollow-init', refresh, { signal: abort.signal });
    return () => { dead = true; observer.disconnect(); root.disconnect(); abort.abort(); for (const item of items) {
        if (item.animation)
            item.animation.onfinish = null;
        settle(item);
    } };
}
