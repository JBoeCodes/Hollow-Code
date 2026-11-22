/** Host-only, estimated geometry. Never imported by the kit or the prompt. */
import { layoutFlow, parseFlow, type FlowDirection } from './kit/flow';
import { edgePath } from './kit/flowPath';

export interface FlowFinding { kind: 'crosses-node' | 'labels-overlap' | 'label-on-node' | 'node-cut' | 'link-cut' | 'not-routed' | 'label-listed' | 'not-shown' | 'more'; text: string }
const cache = new Map<string, readonly FlowFinding[]>();
export function autoAuditFlow(text: string): boolean {
  const f = parseFlow(text);
  return f.nodes.length + f.droppedNodes <= 30 && f.edges.length + f.droppedLinks <= 60;
}
type Rect = { x: number; y: number; w: number; h: number };
const overlaps = (a: Rect, b: Rect) => a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y;
/** Sample the renderer's exact M/L/Q/C paths, not a second routing model. */
function samples(path: string): number[][] {
  const out: number[][] = [];
  let start = [0, 0];
  for (const match of path.matchAll(/([MLQC])([^MLQC]*)/g)) {
    const p = match[2].trim().split(/[ ,]+/).map(Number), command = match[1];
    if (command === 'M') { start = p; continue; }
    const end = p.slice(-2);
    for (let i = 1; i < 20; i++) {
      const t = i / 20, u = 1 - t;
      out.push([0, 1].map(j => command === 'C'
        ? u ** 3 * start[j] + 3 * u * u * t * p[j] + 3 * u * t * t * p[j + 2] + t ** 3 * end[j]
        : command === 'Q' ? u * u * start[j] + 2 * u * t * p[j] + t * t * end[j]
          : u * start[j] + t * end[j]));
    }
    start = end;
  }
  return out;
}

export function auditFlow(text: string, direction: FlowDirection = 'down'): readonly FlowFinding[] {
  const key = `${direction}\0${text}`, known = cache.get(key);
  if (known) return known;
  const flow = parseFlow(text), layout = layoutFlow(flow, direction), findings: FlowFinding[] = [];
  let count = 0;
  const add = (kind: FlowFinding['kind'], text: string) => { count++; if (findings.length < 20) findings.push({ kind, text }); };
  const name = (id: string) => layout.nodes.find(n => n.id === id)?.label || id;
  for (const node of layout.nodes) if (node.lines.join(' ') !== node.label.replace(/\s+/g, ' ').trim())
    add('node-cut', `Node "${node.label}" may have a shortened label.`);
  const labels: Array<Rect & { name: string }> = [];
  for (const edge of layout.edges) {
    const title = `${name(edge.from)} → ${name(edge.to)}`;
    if (edge.route && !edge.route.length) {
      add('not-routed', `Link "${title}" could not be drawn; kept in the connection list.`);
      continue;
    }
    const points = samples(edgePath(edge, direction === 'right'));
    for (const node of layout.nodes) {
      if (node.id === edge.from || node.id === edge.to) continue;
      if (points.some(([x, y]) => x > node.x + .01 && x < node.x + node.w - .01 && y > node.y + .01 && y < node.y + node.h - .01))
        add('crosses-node', `Link "${title}" may run through "${node.label}".`);
    }
    if (edge.label && !edge.text) add('label-listed', `Label for "${title}" is kept in the connection list.`);
    if (edge.text.endsWith('…')) add('link-cut', `Label for "${title}" may be shortened.`);
    if (!edge.text) continue;
    const rect = { x: edge.lx - edge.lw / 2, y: edge.ly - 9, w: edge.lw, h: 18, name: title };
    for (const previous of labels) if (overlaps(rect, previous)) add('labels-overlap', `Labels for "${previous.name}" and "${title}" may overlap.`);
    for (const node of layout.nodes) if (node.id !== edge.from && node.id !== edge.to && overlaps(rect, node))
      add('label-on-node', `Label for "${title}" may cover "${node.label}".`);
    labels.push(rect);
  }
  if (flow.droppedNodes || flow.droppedLinks) add('not-shown', [flow.droppedNodes && `${flow.droppedNodes} nodes`, flow.droppedLinks && `${flow.droppedLinks} links`].filter(Boolean).join(' and ') + ' are not shown.');
  if (count > 20) findings.push({ kind: 'more', text: `And ${count - 20} more layout problems.` });
  if (cache.size >= 64) cache.delete(cache.keys().next().value!);
  const result = Object.freeze(findings.map(f => Object.freeze(f)));
  cache.set(key, result);
  return result;
}
