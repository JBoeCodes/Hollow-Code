/** Inert source extraction shared by export and the host-only layout audit. */
import { parseCanvasFence } from './fence';
import type { El } from './canvasModel';
import type { FlowDirection } from './kit/flow';

export interface CanvasFlow { label: string; text: string; direction: FlowDirection }
export function flowsInRoot(root: El, limit = 8): CanvasFlow[] {
  const found: CanvasFlow[] = [];
  const walk = (el: El) => {
    if (found.length >= limit || ['script', 'style', 'template'].includes(el.tagName.toLowerCase())) return;
    if (el.tagName.toLowerCase() === 'hc-flow') {
      found.push({ label: el.getAttribute('label') || `Flow ${found.length + 1}`, text: el.textContent || '',
        direction: el.getAttribute('direction')?.toLowerCase() === 'right' ? 'right' : 'down' });
    } else Array.from(el.children).forEach(walk);
  };
  walk(root);
  return found;
}

export function flowsIn(source: string, limit = 8): CanvasFlow[] {
  const parsed = parseCanvasFence(source, true);
  if (parsed.state !== 'ready') return [];
  // A template never upgrades custom elements or loads images. Keep that
  // stronger inert boundary instead of parsing into a live document.
  const template = document.createElement('template');
  template.innerHTML = parsed.inner;
  template.content.querySelectorAll('script,style,template').forEach(e => e.remove());
  return flowsInRoot({ tagName: 'root', children: template.content.children, getAttribute: () => null, textContent: null }, limit);
}
