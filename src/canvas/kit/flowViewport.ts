/** User-driven viewBox changes only: native pointer capture for drag, keyboard
 * pan and zoom, explicit Fit. Wheel zoom requires Control/Command so normal
 * page scrolling is never trapped. No document-level pointer listeners. */
import { h, uid } from './registry';
export function flowViewport(box: HTMLElement, svg: SVGElement, width: number, height: number, label: string): void {
    box.classList.add('hc-flow-interactive');
    box.setAttribute('role', 'region');
    box.setAttribute('aria-label', `${label || 'Flow'} diagram controls`);
    box.tabIndex = 0;
    const hint = h('span', 'hc-sr', 'Arrow keys pan; plus and minus zoom; Home fits. Drag to pan, or Control/Command and wheel to zoom.');
    hint.id = uid('hc-flow-help');
    box.setAttribute('aria-describedby', hint.id);
    const toolbar = h('div', 'hc-flow-toolbar', null, { 'role': 'group', 'aria-label': 'Diagram zoom' });
    const zoomOut = h('button', 'hc-btn', '−', { 'type': 'button', 'aria-label': 'Zoom out' });
    const zoomIn = h('button', 'hc-btn', '+', { 'type': 'button', 'aria-label': 'Zoom in' });
    const fit = h('button', 'hc-btn', 'Fit', { 'type': 'button', 'aria-label': 'Fit diagram' });
    const value = h('output', 'hc-flow-zoom', 'Zoom 100%', { 'aria-live': 'polite' });
    toolbar.append(zoomOut, value, zoomIn, fit);
    box.prepend(toolbar, hint);
    let scale = 1, x = 0, y = 0;
    const apply = () => {
        const w = width / scale, hh = height / scale;
        x = Math.max(-width * .1, Math.min(width - w + width * .1, x));
        y = Math.max(-height * .1, Math.min(height - hh + height * .1, y));
        svg.setAttribute('viewBox', `${x} ${y} ${w} ${hh}`);
        value.textContent = `Zoom ${Math.round(scale * 100)}%`;
        zoomOut.setAttribute('aria-disabled', String(scale <= 1));
        zoomIn.setAttribute('aria-disabled', String(scale >= 16));
    };
    const zoom = (factor: number) => { const next = Math.max(1, Math.min(16, scale * factor)); x += (width / scale - width / next) / 2; y += (height / scale - height / next) / 2; scale = next; apply(); };
    const reset = () => { scale = 1; x = 0; y = 0; apply(); };
    zoomOut.onclick = () => zoom(1 / 1.25);
    zoomIn.onclick = () => zoom(1.25);
    fit.onclick = reset;
    box.addEventListener('keydown', e => {
        if (e.target !== box && !['+', '=', '-'].includes(e.key))
            return;
        const d = 40 / scale;
        if (e.key === 'ArrowLeft')
            x -= d;
        else if (e.key === 'ArrowRight')
            x += d;
        else if (e.key === 'ArrowUp')
            y -= d;
        else if (e.key === 'ArrowDown')
            y += d;
        else if (e.key === '+' || e.key === '=') {
            e.preventDefault();
            zoom(1.25);
            return;
        }
        else if (e.key === '-') {
            e.preventDefault();
            zoom(1 / 1.25);
            return;
        }
        else if (e.key === 'Home') {
            e.preventDefault();
            reset();
            return;
        }
        else
            return;
        e.preventDefault();
        apply();
    });
    box.addEventListener('focusin', e => {
        if (!(e.target instanceof SVGElement) || !e.target.classList.contains('hc-flow-link'))
            return;
        const b = e.target.querySelector<SVGGraphicsElement>('.hc-flow-route')!.getBBox();
        if (b.x < x || b.y < y || b.x + b.width > x + width / scale || b.y + b.height > y + height / scale)
            reset();
    });
    let drag: {
        id: number;
        x: number;
        y: number;
        ox: number;
        oy: number;
        units: number;
    } | null = null;
    svg.addEventListener('pointerdown', raw => {
        const e = raw as PointerEvent;
        if (e.button !== 0)
            return;
        const r = svg.getBoundingClientRect();
        drag = { id: e.pointerId, x: e.clientX, y: e.clientY, ox: x, oy: y, units: Math.max(width / r.width, height / r.height) / scale };
        svg.setPointerCapture(e.pointerId);
        e.preventDefault();
        box.focus({ preventScroll: true });
    });
    svg.addEventListener('pointermove', raw => { const e = raw as PointerEvent; if (!drag || drag.id !== e.pointerId)
        return; x = drag.ox - (e.clientX - drag.x) * drag.units; y = drag.oy - (e.clientY - drag.y) * drag.units; apply(); });
    for (const name of ['pointerup', 'pointercancel', 'lostpointercapture'])
        svg.addEventListener(name, () => { drag = null; });
    svg.addEventListener('wheel', raw => { const e = raw as WheelEvent; if (!e.ctrlKey && !e.metaKey)
        return; e.preventDefault(); zoom(e.deltaY < 0 ? 1.1 : 1 / 1.1); }, { passive: false });
    apply();
}
