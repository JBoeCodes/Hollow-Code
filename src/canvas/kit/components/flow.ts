/** `<hc-flow label direction>`: a diagram of boxes and arrows from plain
 *  text (one node or link per line; see `../flow.ts`), laid out by the kit.
 *  It replaces whiteboards (2026-10-07). The drawing is one SVG named by its
 *  label; a visually hidden list says the same in words (each node, its
 *  shape and tone, and where its links go), so VoiceOver reads the diagram
 *  rather than its geometry. A tone is shown by colour and by its icon. The
 *  colours are CSS, so a theme change restyles it without a redraw. */
import { edgePath } from '../flowPath';
import { flowMotion } from '../flowMotion';
import { flowViewport } from '../flowViewport';
import { KIT_TONES } from '../manifest';
import { layoutFlow, parseFlow, type FlowDirection, type PlacedNode } from '../flow';
import { attr, h, part, source, uid } from '../registry';
import { TextureElement } from '../textureElement';
import { stipple } from '../material';

const SVG = 'http://www.w3.org/2000/svg';

function s(tag: string, attrs: Record<string, string | number>, text?: string): SVGElement {
  const el = document.createElementNS(SVG, tag);
  for (const k of Object.keys(attrs)) el.setAttribute(k, String(attrs[k]));
  if (text != null) el.textContent = text;
  return el;
}

function shape(n: PlacedNode): SVGElement {
  const { x, y, w, hh } = { x: n.x, y: n.y, w: n.w, hh: n.h };
  if (n.shape === 'decision') {
    return s('polygon', { points: `${x + w / 2},${y} ${x + w},${y + hh / 2} ${x + w / 2},${y + hh} ${x},${y + hh / 2}` });
  }
  const r = n.shape === 'start' || n.shape === 'end' ? hh / 2 : n.shape === 'note' ? 2 : 6;
  return s('rect', { x, y, width: w, height: hh, rx: r });
}

/** Live flows that asked for `right`: on a resize or `hc-reflow` (a tab or
 *  details opening) each draws again only if its direction would change. */
const wideFlows = new Set<HcFlow>();
let wired = false;
function wire(): void {
  if (wired) return;
  wired = true;
  let pending = false;
  const check = () => {
    if (pending) return;
    pending = true;
    requestAnimationFrame(() => {
      pending = false;
      wideFlows.forEach(f => f._recheck());
    });
  };
  addEventListener('resize', check);
  addEventListener('hc-reflow', check);
  addEventListener('toggle', check, true);
}

export class HcFlow extends TextureElement {
  static get observedAttributes(): string[] { return ['label', 'direction', 'interactive']; }
  private _down = false;
  private _motion?: () => void;
  private _measure: { text: string; right: number; down: number } | null = null;
  disconnectedCallback(): void {
    super.disconnectedCallback();
    this._motion?.();
    this._motion = undefined;
    wideFlows.delete(this);
  }

  /** Draws again when the room now calls for the other direction. */
  _recheck(): void {
    if (!this.isConnected) { wideFlows.delete(this); return; }
    if (this._wantsDown() !== this._down) this.render();
  }

  /** Whether a `right` flow should run down in the room it has now. */
  private _wantsDown(): boolean {
    const text = source(this).textContent || '';
    if (this._measure?.text !== text) {
      const flow = parseFlow(text);
      this._measure = {
        text,
        right: layoutFlow(flow, 'right', false).width,
        down: layoutFlow(flow, 'down', false).width,
      };
    }
    const room = this.getBoundingClientRect().width || document.documentElement.clientWidth || 720;
    return this._measure.right * 0.8 > room && this._measure.down < this._measure.right;
  }

  protected render(): void {
    this._motion?.();
    this.clearParts();
    wideFlows.delete(this);
    const label = attr(this, 'label');
    const direction: FlowDirection = attr(this, 'direction').toLowerCase() === 'right' ? 'right' : 'down';
    const flow = parseFlow(source(this).textContent || '');
    // 10-11 B2: `right` is a preference. When running across would shrink
    // the text past 80% in this width and running down shrinks it less,
    // the diagram runs down.
    this._down = direction === 'right' && this._wantsDown();
    if (direction === 'right') { wideFlows.add(this); wire(); }
    const layout = layoutFlow(flow, this._down ? 'down' : direction);
    const across = direction === 'right' && !this._down;
    const parts: HTMLElement[] = [];
    const nodes: SVGElement[] = [], links: SVGElement[] = [];
    if (label) parts.push(part('div', 'hc-flow-label', label));
    const box = part('div', 'hc-flow-box');
    box.tabIndex = -1;
    const svg = s('svg', {
      class: 'hc-flow-svg', viewBox: `0 0 ${layout.width} ${layout.height}`,
      width: layout.width, height: layout.height, role: 'group', 'aria-label': label || 'Diagram',
      style: `--hc-flow-min:${Math.round(layout.width * 0.8)}px`,
    });
    const arrow = uid('hc-flow-arrow');
    const defs = s('defs', {});
    const marker = s('marker', { id: arrow, viewBox: '0 0 10 10', refX: 9, refY: 5, markerWidth: 7, markerHeight: 7, orient: 'auto-start-reverse' });
    marker.append(s('path', { d: 'M0,0 L10,5 L0,10 z', class: 'hc-flow-head' }));
    defs.append(marker);
    svg.append(defs);
    for (const g of layout.groups) {
      const el = s('g', { class: 'hc-flow-group', 'aria-hidden':'true' });
      svg.append(el);
      const border = s('rect', { x: g.x, y: g.y, width: g.w, height: g.h, rx: 10 });
      el.append(border); stipple(el, border, 'hc-flow-dots');
      el.append(s('text', { x: g.x + 10, y: g.y + 15 }, g.text));
      svg.append(el);
    }
    // Links, then nodes, then link labels on top, so a node never hides one.
    const tags: SVGElement[] = [];
    for (const e of layout.edges) {
      if (e.route && !e.route.length) continue;
      const name = `${flow.nodes.find(n=>n.id===e.from)!.label} → ${flow.nodes.find(n=>n.id===e.to)!.label}${e.label ? ': '+e.label : ''}${e.active ? ' (active)' : ''}`;
      const el = s('g', { class: 'hc-flow-link', tabindex: links.length ? -1 : 0, role: 'img', 'aria-label': name, 'data-active': String(!!e.active) });
      const d=edgePath(e,across);
      el.append(s('path', { d, class:'hc-flow-hit', 'aria-hidden':'true' }));
      el.append(s('path', { d, class:'hc-flow-route', 'marker-end': `url(#${arrow})` }));
      el.append(s('circle', { r:3, cx:0, cy:0, class:'hc-flow-particle', 'aria-hidden':'true', style:'opacity:0' }));
      links.push(el);
      svg.append(el);
      if (e.text) {
        const tag = s('g', { class: 'hc-flow-linklabel', 'aria-hidden':'true' });
        tag.append(s('rect', { x: e.lx - e.lw / 2, y: e.ly - 9, width: e.lw, height: 18, rx: 4, class: 'hc-flow-tag' }));
        tag.append(s('text', { x: e.lx, y: e.ly + 4, 'text-anchor': 'middle' }, e.text));
        tags.push(tag);
      }
    }
    for (const n of layout.nodes) {
      const el = s('g', { class: 'hc-flow-node', 'aria-hidden':'true', 'data-shape': n.shape, 'data-tone': n.tone });
      svg.append(el);
      const outline = shape(n);
      el.append(outline); stipple(el, outline, 'hc-flow-dots');
      const icon = n.tone === 'neutral' ? '' : `${KIT_TONES[n.tone].icon} `;
      const top = n.y + n.h / 2 - ((n.lines.length - 1) * 16) / 2 + 5;
      const text = s('text', { x: n.x + n.w / 2, y: top, 'text-anchor': 'middle' });
      n.lines.forEach((line, i) => {
        text.append(s('tspan', { x: n.x + n.w / 2, dy: i ? 16 : 0 }, i === 0 ? icon + line : line));
      });
      el.append(text);
      nodes.push(el);
      svg.append(el);
    }
    svg.append(...tags);
    links.forEach((el,i)=>{
      el.addEventListener('focus',()=>links.forEach(link=>link.setAttribute('tabindex',link===el?'0':'-1')));
      el.addEventListener('keydown',raw=>{
        const event=raw as KeyboardEvent;
        const target=event.key==='ArrowRight'||event.key==='ArrowDown'?links[(i+1)%links.length]:event.key==='ArrowLeft'||event.key==='ArrowUp'?links[(i+links.length-1)%links.length]:null;
        if(target){event.preventDefault();(target as SVGElement & {focus:()=>void}).focus();}
        if(event.key==='Escape'){event.preventDefault();box.focus();}
      });
    });
    box.append(svg);
    if (this.hasAttribute('interactive') || layout.width > 700 || layout.height > 560) flowViewport(box,svg,layout.width,layout.height,label);
    parts.push(box);
    // The same diagram in words, for screen readers.
    const words = part('ul', 'hc-sr', null, { 'aria-label': `${label || 'Diagram'} as text` });
    for (const n of flow.nodes) {
      const bits = [n.label];
      if (n.shape !== 'box') bits.push(`(${n.shape})`);
      if (n.tone !== 'neutral') bits.push(`(${KIT_TONES[n.tone].word})`);
      if (n.group) bits.push(`in ${n.group}`);
      const links = flow.edges.filter(e => e.from === n.id).map(e => {
        const to = flow.nodes.find(m => m.id === e.to)!.label;
        return e.label ? `${to} (${e.label})` : to;
      });
      if (links.length) bits.push(`→ ${links.join(', ')}`);
      words.append(h('li', '', bits.join(' ')));
    }
    const more = [
      flow.droppedNodes ? `${flow.droppedNodes} ${flow.droppedNodes === 1 ? 'node' : 'nodes'}` : '',
      flow.droppedLinks ? `${flow.droppedLinks} ${flow.droppedLinks === 1 ? 'link' : 'links'}` : '',
    ].filter(Boolean).join(' and ');
    if (more) words.append(h('li', '', `${more} not shown`));
    parts.push(words);
    if (more) parts.push(part('div', 'hc-flow-more', `${more} not shown`, { 'aria-hidden': 'true' }));
    const blocked=layout.edges.filter(e=>e.route && !e.route.length).length;
    const hiddenLabels=layout.edges.filter(e=>e.label && !e.text && e.route?.length).length;
    if(blocked || hiddenLabels){
      const notices = [blocked ? `${blocked} ${blocked===1 ? 'link could' : 'links could'} not be routed.` : '', hiddenLabels ? `${hiddenLabels} ${hiddenLabels===1 ? 'link label is' : 'link labels are'} in the connection list to keep the diagram clear.` : ''].filter(Boolean);
      parts.push(part('div','hc-flow-more',notices.join(' ')+' Read all connections below.'));
      words.classList.remove('hc-sr');
      const details=part('details','hc-flow-fallback');
      details.append(h('summary','','Diagram connections'),words);
      parts.splice(parts.indexOf(words),1,details);
    }
    this.prepend(...parts);
    this._motion = flowMotion(this, nodes, links);
  }
}
