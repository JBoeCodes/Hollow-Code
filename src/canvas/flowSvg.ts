/** Standalone, static flow drawings. Never serializes agent markup or runs it. */
import { layoutFlow, parseFlow } from './kit/flow';
import { edgePath } from './kit/flowPath';
import { KIT_TONES } from './kit/manifest';
import tokens from '../styles/tokens.css?raw';

export type ExportFlow = import('./canvasFlows').CanvasFlow;
import { flowsIn } from './canvasFlows';
const esc = (s: string) => s.replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/g, '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&apos;' }[c]!));

/** Template contents are inert: no custom-element upgrade, script or image load. */
export function canvasFlows(source: string): ExportFlow[] {
  return flowsIn(source, Infinity);
}

const roles = { ink: 'ink', canvas: 'canvas', surface: 'surface-card', soft: 'surface-soft', line: 'hairline', muted: 'muted', primary: 'primary', info: 'accent-teal', success: 'success', warning: 'warning', danger: 'error' } as const;
export type FlowColors = Record<keyof typeof roles, string>;
/** Read stock values from the same source as capture, avoiding a second palette. */
export function flowColors(scheme: 'light' | 'dark', pairs: Array<[string, string]> = []): FlowColors {
  const css = tokens.replace(/\/\*[\s\S]*?\*\//g, '');
  const declarations = (css.match(/:root\s*\{([^}]+)\}/)?.[1] || '') + (scheme === 'dark' ? css.match(/:root\[data-theme="dark"\]\s*\{([^}]+)\}/)?.[1] || '' : '');
  const values = Object.fromEntries(Array.from(declarations.matchAll(/--([\w-]+)\s*:\s*([^;]+);/g), m => [m[1], m[2].trim()]));
  const supplied = Object.fromEntries(pairs);
  const appNames: Record<string, string> = { soft: 'surface-soft', line: 'hairline' };
  const safe = (v: string) => /^#[\da-f]{3,8}$/i.test(v) || /^rgba?\([\d.,%\s]+\)$/.test(v);
  return Object.fromEntries(Object.entries(roles).map(([role, token]) => {
    const value = supplied[`--app-${appNames[role] || role}`];
    // Only numeric colours enter the stylesheet. Resolve more complex custom
    // colours at the caller; unknown expressions use the stock role.
    return [role, value && safe(value) ? value : values[token]];
  })) as FlowColors;
}

/** Resolve CSS colour expressions (including color-mix) to portable sRGB. */
export function resolvedFlowTheme(pairs: Array<[string, string]>): Array<[string, string]> {
  const canvas = document.createElement('canvas'); canvas.width = canvas.height = 1;
  const ctx = canvas.getContext('2d');
  if (!ctx) return pairs;
  return pairs.filter(([, value]) => CSS.supports('color', value) && !/var\(|currentcolor/i.test(value)).map(([key, value]) => {
    ctx.clearRect(0, 0, 1, 1); ctx.fillStyle = value; ctx.fillRect(0, 0, 1, 1);
    const [r, g, b, a] = ctx.getImageData(0, 0, 1, 1).data;
    return [key, `rgba(${r},${g},${b},${a / 255})`];
  });
}

export function flowSvg(flow: ExportFlow, width: number, colors: FlowColors): { svg: string; warning: string | null } {
  const model = parseFlow(flow.text);
  if (!model.nodes.length) throw new Error('This flow has no nodes to export.');
  const right = layoutFlow(model, 'right', false), down = layoutFlow(model, 'down', false);
  const across = flow.direction === 'right' && !(right.width * .8 > width && down.width < right.width);
  const layout = layoutFlow(model, across ? 'right' : 'down');
  const titleHeight = 32, height = layout.height + titleHeight;
  const c = colors;
  // Colors come from flowColors, but the pure writer also rejects CSS injection.
  if (Object.values(c).some(v => !/^(#[\da-f]{3,8}|rgba?\([\d.,%\s]+\))$/i.test(v))) throw new Error('Invalid flow colours.');
  const style = `text{font-family:Arial,sans-serif;font-size:13px;fill:${c.ink}}.edge{fill:none;stroke:${c.muted};stroke-width:1.4}.active{stroke:${c.primary};stroke-width:2}.group{fill:${c.soft};stroke:${c.line}}.small{font-size:11px;fill:${c.muted}}.tag{fill:${c.surface};stroke:${c.line}}`;
  const warning = model.droppedNodes || model.droppedLinks ? `${model.droppedNodes} nodes and ${model.droppedLinks} links exceed the kit limits and are not drawn.` : null;
  const description = [...model.nodes.map(n => `${n.label} (${n.shape}, ${KIT_TONES[n.tone].word})${n.group ? ` in ${n.group}` : ''}`), ...model.edges.map(e => `${model.nodes.find(n => n.id === e.from)?.label} → ${model.nodes.find(n => n.id === e.to)?.label}${e.label ? `: ${e.label}` : ''}`), warning || ''].join('\n');
  const parts = [`<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${Math.ceil(height * width / layout.width)}" viewBox="0 0 ${layout.width} ${height}" role="img" aria-labelledby="title desc"><title id="title">${esc(flow.label)}</title><desc id="desc">${esc(description)}</desc><style>${style}</style>`,
    `<rect width="${layout.width}" height="${height}" fill="${c.canvas}"/><text x="12" y="22">${esc(flow.label)}</text><g transform="translate(0 ${titleHeight})"><defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="${c.muted}"/></marker></defs>`];
  for (const g of layout.groups) parts.push(`<rect class="group" x="${g.x}" y="${g.y}" width="${g.w}" height="${g.h}" rx="10"/><text class="small" x="${g.x + 10}" y="${g.y + 15}">${esc(g.text)}</text>`);
  const tags: string[] = [];
  let hidden = 0;
  for (const e of layout.edges) {
    if (e.route && !e.route.length) { hidden++; continue; }
    parts.push(`<path class="edge${e.active ? ' active' : ''}" d="${edgePath(e, across)}" marker-end="url(#arrow)"/>`);
    if (e.label && !e.text) hidden++;
    if (e.text) tags.push(`<rect class="tag" x="${e.lx - e.lw / 2}" y="${e.ly - 9}" width="${e.lw}" height="18" rx="4"/><text class="small" x="${e.lx}" y="${e.ly + 4}" text-anchor="middle">${esc(e.text)}</text>`);
  }
  for (const n of layout.nodes) {
    const tone = n.tone === 'neutral' ? c.muted : c[n.tone];
    const attrs = `fill="${n.shape === 'note' ? c.soft : c.surface}" stroke="${tone}" stroke-width="${n.tone === 'neutral' ? 1.25 : 2}"${n.shape === 'note' ? ' stroke-dasharray="4 3"' : ''}`;
    parts.push(n.shape === 'decision' ? `<polygon points="${n.x + n.w / 2},${n.y} ${n.x + n.w},${n.y + n.h / 2} ${n.x + n.w / 2},${n.y + n.h} ${n.x},${n.y + n.h / 2}" ${attrs}/>` : `<rect x="${n.x}" y="${n.y}" width="${n.w}" height="${n.h}" rx="${n.shape === 'start' || n.shape === 'end' ? n.h / 2 : n.shape === 'note' ? 2 : 6}" ${attrs}/>`);
    const y = n.y + n.h / 2 - ((n.lines.length - 1) * 16) / 2 + 5;
    parts.push(`<text x="${n.x + n.w / 2}" y="${y}" text-anchor="middle">${n.lines.map((line, i) => `<tspan x="${n.x + n.w / 2}" dy="${i ? 16 : 0}">${esc((i === 0 && n.tone !== 'neutral' ? KIT_TONES[n.tone].icon + ' ' : '') + line)}</tspan>`).join('')}</text>`);
  }
  parts.push(...tags, '</g></svg>');
  return { svg: parts.join(''), warning: [warning, hidden ? `${hidden} connections or labels could not be drawn; the SVG description retains them.` : ''].filter(Boolean).join(' ') || null };
}
