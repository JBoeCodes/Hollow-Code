/** `hc-flow`'s model: parse its text into nodes and links, then lay them out
 *  in layers (Canvas kit, 2026-10-07, replacing whiteboards). Pure, no DOM,
 *  so Vitest covers it directly; `components/flow.ts` draws the result.
 *
 *  **Text (kit@1).** One item per line, readable without the script:
 *    - `id: Label {decision, success}` a node. Trailing braces hold a shape
 *      (`start`, `end`, `decision`, `note`) and/or a tone; braces holding
 *      any other word stay in the label (`GET /users/{id}`). A bare
 *      `id {decision}` adds words to a node without changing its label;
 *    - `a -> b` a link, `a -> b: yes` a labelled one, `a -> b -> c` a chain.
 *      The label is everything after the first `:` that follows the first
 *      `->`, so it may hold `->` or `:` itself; a line whose first `:` comes
 *      before its first `->` is a node (`step: Click -> Save`). Each end may
 *      carry braces too (`a {start} -> b {danger}`). An end that names no
 *      node by id or label becomes a node of its own, and a later
 *      `id: Label` naming that label takes it over. Repeated links merge,
 *      their labels joined with ` / `;
 *    - `[Backend]` starts a group: the nodes after it, until the next
 *      `[…]` or a bare `[]`, are drawn inside one labelled box.
 *
 *  **Layout.** Back links are reversed for layering (so a cycle can't hang
 *  it), each node goes one layer past its deepest parent, and a few
 *  barycenter sweeps order each layer to cut crossings. With groups, each
 *  group and each ungrouped node takes a lane free over its layers, so a
 *  group's box holds only its members, boxes never overlap, and groups that
 *  follow each other share a lane. Link labels
 *  and loops count toward the drawing's size. Deterministic for the same
 *  text. */
import { routeFlow, type Point } from './flowRoute';
import { KIT_TONES, type KitTone } from './manifest';

export const FLOW_SHAPES = ['box', 'start', 'end', 'decision', 'note'] as const;
export type FlowShape = typeof FLOW_SHAPES[number];
export type FlowDirection = 'down' | 'right';

export interface FlowNode { id: string; label: string; shape: FlowShape; tone: KitTone; group: string }
export interface FlowEdge { from: string; to: string; label: string; active?: boolean }
export interface Flow { nodes: FlowNode[]; edges: FlowEdge[]; groups: string[]; droppedNodes: number; droppedLinks: number }

/** Bounds, so a long text stays drawable. */
export const MAX_NODES = 60;
export const MAX_EDGES = 120;
const MAX_LABEL = 120;
const MAX_LINK_LABEL = 40;

const clip = (s: string, n: number) => (s.length > n ? `${s.slice(0, n - 1)}…` : s);

const isWord = (w: string) => (FLOW_SHAPES as readonly string[]).includes(w) || Object.prototype.hasOwnProperty.call(KIT_TONES, w);

/** `text {a, b}` → the text and its words, when every word is a shape or a
 *  tone; otherwise the braces are part of the text. */
function braces(text: string): { body: string; words: string[] } {
  const m = /\{([^{}]*)\}\s*$/.exec(text);
  if (m) {
    const words = m[1].split(/[\s,]+/).map(w => w.toLowerCase()).filter(Boolean);
    if (words.length && words.every(isWord)) return { body: text.slice(0, m.index).trim(), words };
  }
  return { body: text.trim(), words: [] };
}

export function parseFlow(text: string): Flow {
  const nodes: FlowNode[] = [];
  const byId = new Map<string, FlowNode>();
  /** Nodes made by a link end, not yet defined by a node line. */
  const implicit = new Set<FlowNode>();
  const edges: FlowEdge[] = [];
  const groups: string[] = [];
  const dropped = new Set<string>();
  let droppedLinks = 0;
  let group = '';
  const apply = (n: FlowNode, words: string[]) => {
    for (const w of words) {
      if ((FLOW_SHAPES as readonly string[]).includes(w)) n.shape = w as FlowShape;
      else if (w !== 'neutral') n.tone = w as KitTone;
    }
  };
  const create = (id: string, label: string): FlowNode | null => {
    if (nodes.length >= MAX_NODES) { dropped.add(id); return null; }
    const n: FlowNode = { id, label: clip(label || id, MAX_LABEL), shape: 'box', tone: 'neutral', group };
    nodes.push(n);
    byId.set(id, n);
    return n;
  };
  /** A node line: define `id` (or take over the link-made node with that label). */
  const define = (id: string, label: string | null, words: string[]) => {
    let n = byId.get(id);
    if (!n && label) {
      const made = nodes.find(m => implicit.has(m) && m.id === m.label && m.label === label);
      if (made) {
        byId.delete(made.id);
        for (const e of edges) { if (e.from === made.id) e.from = id; if (e.to === made.id) e.to = id; }
        made.id = id;
        made.group = group;
        byId.set(id, made);
        n = made;
      }
    }
    if (!n) n = create(id, label || id) ?? undefined;
    if (!n) return;
    implicit.delete(n);
    if (label) n.label = clip(label, MAX_LABEL);
    apply(n, words);
  };
  /** A link end: a node by id, then by label, else a new one. Undefined for
   *  an empty end; null for a node left out by the limit. */
  const end = (raw: string): string | null | undefined => {
    const { body, words } = braces(raw);
    if (!body) return undefined;
    let n = byId.get(body) || nodes.find(m => m.label === body);
    if (!n) {
      if (dropped.has(body)) return null;
      n = create(body, body) ?? undefined;
      if (!n) return null;
      implicit.add(n);
    }
    apply(n, words);
    return n.id;
  };
  const link = (from: string, to: string, label: string, active: boolean) => {
    const same = edges.find(e => e.from === from && e.to === to);
    if (same) {
      if (active) same.active = true;
      if (label && !same.label.split(' / ').includes(label)) same.label = clip(same.label ? `${same.label} / ${label}` : label, MAX_LABEL);
      return;
    }
    if (edges.length >= MAX_EDGES) { droppedLinks += 1; return; }
    edges.push({ from, to, label: clip(label, MAX_LABEL), ...(active ? { active: true } : {}) });
  };
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) continue;
    const head = line.includes('->') ? null : /^\[(.*)\]$/.exec(line);
    if (head) {
      group = head[1].trim();
      if (group && !groups.includes(group)) groups.push(group);
      continue;
    }
    const arrow = line.indexOf('->');
    const colon = line.indexOf(':');
    if (arrow > -1 && (colon < 0 || colon > arrow)) {
      const active = /\s*\{active\}\s*$/.test(line);
      const edgeLine = active ? line.replace(/\s*\{active\}\s*$/, '') : line;
      const after = edgeLine.indexOf(':', arrow);
      const chain = after < 0 ? edgeLine : edgeLine.slice(0, after);
      const label = after < 0 ? '' : edgeLine.slice(after + 1).trim();
      const ids = chain.split('->').map(end);
      for (let i = 0; i + 1 < ids.length; i++) {
        const from = ids[i], to = ids[i + 1];
        if (from === undefined || to === undefined) continue;
        if (from === null || to === null) { droppedLinks += 1; continue; }
        link(from, to, i + 2 === ids.length ? label : '', active);
      }
      continue;
    }
    const { body, words } = braces(line);
    if (colon > 0 && body.includes(':')) {
      const c = body.indexOf(':');
      define(body.slice(0, c).trim(), body.slice(c + 1).trim() || null, words);
    } else if (body) {
      const named = byId.get(body) || nodes.find(m => m.label === body);
      if (named) apply(named, words);
      else define(body, body, words);
    }
  }
  return {
    nodes, edges,
    groups: groups.filter(g => nodes.some(n => n.group === g)),
    droppedNodes: dropped.size, droppedLinks,
  };
}

export interface PlacedNode extends FlowNode { layer: number; x: number; y: number; w: number; h: number; lines: string[] }
export interface PlacedEdge extends FlowEdge {
  x1: number; y1: number; x2: number; y2: number;
  /** The points a forward link passes through: its source's exit, one per
   *  layer it skips (so it goes round the nodes there), its target's entry. */
  points: Array<[number, number]>;
  /** Obstacle-safe orthogonal route (rounded by the renderer). Empty means unreachable. */
  route?: Point[];
  /** A link to the same or an earlier layer; `self` a link to itself. */
  back: boolean; self: boolean;
  /** How far a back link or loop bulges out, and where its label sits. */
  bulge: number; lx: number; ly: number; lw: number; text: string;
}
export interface PlacedGroup { name: string; x: number; y: number; w: number; h: number; text: string }
export interface FlowLayout { nodes: PlacedNode[]; edges: PlacedEdge[]; groups: PlacedGroup[]; width: number; height: number }

const LINE_CHARS = 22;
const MAX_LINES = 3;
/** Rough glyph widths at the kit's 13px (nodes) and 11px (link labels). */
const CHAR_W = 7;
const LINK_CHAR_W = 6.4;
/** Group names are 11px semibold. */
const GROUP_CHAR_W = 6.6;
const GROUP_NAME_MAX = 240;
const linkText = (label: string) => clip(label, MAX_LINK_LABEL);
const linkWidth = (text: string) => (text ? text.length * LINK_CHAR_W + 10 : 0);
const nameWidth = (name: string) => name.length * GROUP_CHAR_W + 20;

/** A label wrapped into at most three short lines. */
export function wrapLabel(label: string): string[] {
  const lines: string[] = [];
  let cur = '';
  for (const word of label.split(/\s+/).filter(Boolean)) {
    const w = clip(word, LINE_CHARS);
    if (!cur) cur = w;
    else if (cur.length + 1 + w.length <= LINE_CHARS) cur += ` ${w}`;
    else { lines.push(cur); cur = w; }
  }
  if (cur) lines.push(cur);
  if (lines.length > MAX_LINES) {
    const kept = lines.slice(0, MAX_LINES);
    kept[MAX_LINES - 1] = clip(`${kept[MAX_LINES - 1]} …`, LINE_CHARS + 2);
    return kept;
  }
  return lines.length ? lines : [''];
}

function size(n: FlowNode, lines: string[]): { w: number; h: number } {
  const chars = Math.max(...lines.map(l => l.length), 4) + (n.tone === 'neutral' ? 0 : 2);
  const textW = chars * CHAR_W;
  const textH = lines.length * 16;
  // A diamond holds its text box when w/2 and h/2 are each twice the box's.
  if (n.shape === 'decision') return { w: Math.max(120, textW * 2 + 8), h: Math.max(64, textH * 2 + 8) };
  return { w: Math.min(200, Math.max(84, textW + 28)), h: textH + 20 };
}

const MARGIN = 16;
const GROUP_PAD = 12;
const GROUP_HEAD = 18;
const BULGE = 40;

export function layoutFlow(flow: Flow, direction: FlowDirection = 'down', routing = true): FlowLayout {
  const real = flow.nodes.length;
  const ids = flow.nodes.map(n => n.id);
  const index = new Map(ids.map((id, i) => [id, i] as const));
  const out: number[][] = ids.map(() => []);
  for (const e of flow.edges) {
    const a = index.get(e.from)!, b = index.get(e.to)!;
    if (a !== b) out[a].push(b);
  }
  // Back links (to a node still on the DFS stack) are reversed for layering.
  const state = ids.map(() => 0);
  const dag: number[][] = ids.map(() => []);
  const visit = (u: number) => {
    const stack: Array<[number, number]> = [[u, 0]];
    state[u] = 1;
    while (stack.length) {
      const top = stack[stack.length - 1];
      const [v, i] = top;
      if (i < out[v].length) {
        top[1] += 1;
        const w = out[v][i];
        if (state[w] === 1) dag[w].push(v);
        else {
          dag[v].push(w);
          if (state[w] === 0) { state[w] = 1; stack.push([w, 0]); }
        }
      } else { state[v] = 2; stack.pop(); }
    }
  };
  ids.forEach((_, i) => { if (state[i] === 0) visit(i); });
  // Longest path from the sources (Kahn's order over the DAG).
  const indeg = ids.map(() => 0);
  dag.forEach(ws => ws.forEach(w => { indeg[w] += 1; }));
  const layer = ids.map(() => 0);
  const queue = ids.map((_, i) => i).filter(i => indeg[i] === 0);
  for (let q = 0; q < queue.length; q++) {
    const v = queue[q];
    for (const w of dag[v]) {
      layer[w] = Math.max(layer[w], layer[v] + 1);
      if (--indeg[w] === 0) queue.push(w);
    }
  }
  // Vertices: the nodes, then one virtual point per layer a forward link
  // skips, so ordering and spacing route it round the nodes there.
  const vLayer = [...layer];
  const up: number[][] = ids.map(() => []);
  const down: number[][] = ids.map(() => []);
  const join = (u: number, v: number) => { down[u].push(v); up[v].push(u); };
  const via: number[][] = flow.edges.map(() => []);
  flow.edges.forEach((e, k) => {
    const a = index.get(e.from)!, b = index.get(e.to)!;
    if (a === b || layer[a] === layer[b]) return;
    const [u, v] = layer[a] < layer[b] ? [a, b] : [b, a];
    if (layer[b] - layer[a] < 2) { join(u, v); return; }
    let prev = a;
    for (let l = layer[a] + 1; l < layer[b]; l++) {
      const d = vLayer.length;
      vLayer.push(l);
      up.push([]);
      down.push([]);
      via[k].push(d);
      join(prev, d);
      prev = d;
    }
    join(prev, b);
  });
  const count = vLayer.length;
  const verts = Array.from({ length: count }, (_, v) => v);
  const depth = Math.max(0, ...layer) + 1;
  const layers: number[][] = Array.from({ length: depth }, () => []);
  verts.forEach(v => layers[vLayer[v]].push(v));
  // Barycenter ordering within each layer; a group's members stay together.
  const grouped = flow.groups.length > 0;
  const groupOf = (v: number) => (v < real ? flow.nodes[v].group : '');
  const pos = verts.map(() => 0);
  const place = (l: number[]) => l.forEach((v, i) => { pos[v] = i; });
  layers.forEach(place);
  const order = (l: number[], near: number[][]) => {
    const bary = new Map<number, number>();
    for (const v of l) {
      const ns = near[v];
      bary.set(v, ns.length ? ns.reduce((sum, n) => sum + pos[n], 0) / ns.length : pos[v]);
    }
    l.sort((a, b) => bary.get(a)! - bary.get(b)! || pos[a] - pos[b]);
    const first = new Map<string, number>();
    l.forEach((v, i) => { const g = groupOf(v); if (g && !first.has(g)) first.set(g, i); });
    const ranked = l.map((v, i) => ({ v, r: groupOf(v) ? first.get(groupOf(v))! : i, i }));
    ranked.sort((a, b) => a.r - b.r || a.i - b.i);
    ranked.forEach(({ v }, i) => { l[i] = v; });
    place(l);
  };
  for (let sweep = 0; sweep < 4; sweep++) {
    for (let i = 1; i < depth; i++) order(layers[i], up);
    for (let i = depth - 2; i >= 0; i--) order(layers[i], down);
  }
  // Lanes (only with groups): each group, and each other vertex, is an item
  // over a range of layers; an item takes the first lane free for its whole
  // range, so a lane holds one item per layer. A group's box then holds only
  // its members, and boxes never overlap; groups that follow each other
  // share a lane. Without groups every vertex of a layer shares one lane.
  interface Item { members: number[]; from: number; to: number; boxed: boolean; key: number }
  const items: Item[] = [];
  const laneOf = verts.map(() => 0);
  let laneCount = 1;
  const boxedLane: boolean[] = [false];
  if (grouped) {
    for (const g of flow.groups) {
      const ms = verts.filter(v => groupOf(v) === g);
      const from = Math.min(...ms.map(v => vLayer[v]));
      const to = Math.max(...ms.map(v => vLayer[v]));
      const key = Math.min(...ms.filter(v => vLayer[v] === from).map(v => pos[v]));
      items.push({ members: ms, from, to, boxed: true, key });
    }
    verts.forEach(v => { if (!groupOf(v)) items.push({ members: [v], from: vLayer[v], to: vLayer[v], boxed: false, key: pos[v] }); });
    items.sort((a, b) => a.from - b.from || a.key - b.key);
    const lastTo: number[] = [];
    boxedLane.length = 0;
    for (const it of items) {
      let k = lastTo.findIndex(t => t < it.from);
      if (k < 0) { k = lastTo.length; lastTo.push(-1); boxedLane.push(false); }
      lastTo[k] = it.to;
      if (it.boxed) boxedLane[k] = true;
      for (const v of it.members) laneOf[v] = k;
    }
    laneCount = lastTo.length;
  }
  // Sizes, then coordinates. "Along" runs from layer to layer (down the page,
  // or across it); "across" runs within a layer. A virtual point is a thin
  // slot a link passes through.
  const placed: PlacedNode[] = flow.nodes.map((n, i) => {
    const lines = wrapLabel(n.label);
    return { ...n, layer: layer[i], x: 0, y: 0, ...size(n, lines), lines };
  });
  const box = verts.map(v => (v < real ? placed[v] : { x: 0, y: 0, w: 16, h: 16 }));
  const right = direction === 'right';
  const breadth = (v: number) => (right ? box[v].h : box[v].w);
  const thickness = (v: number) => (v < real ? (right ? box[v].w : box[v].h) : 0);
  const gapIn = right ? 24 : 28;
  // Room a group box needs on each side of its members, within a layer.
  const before = right ? GROUP_PAD + GROUP_HEAD : GROUP_PAD;
  const after = GROUP_PAD;
  const laneGap = grouped ? 20 : 0;
  // The gap after each layer: in right mode wide enough for the labels of
  // the forward links leaving it (a label's first spot is that gap).
  const gapAfter = layers.map((_, li) => {
    if (!right) return 60;
    const widest = Math.max(0, ...flow.edges
      .filter(e => layer[index.get(e.from)!] === li && layer[index.get(e.to)!] > li)
      .map(e => linkWidth(linkText(e.label))));
    return Math.max(80, widest + 24);
  });
  const lanes = Array.from({ length: laneCount }, (_, k) => k);
  const inLane = (l: number[], k: number) => l.filter(v => laneOf[v] === k);
  const span = (vs: number[]) => vs.reduce((sum, v) => sum + breadth(v), 0) + Math.max(0, vs.length - 1) * gapIn;
  const laneSpan = lanes.map(k => Math.max(0, ...layers.map(l => span(inLane(l, k)))));
  // Down the page a group's name runs across its lane: make room for it.
  if (grouped && !right) {
    for (const g of flow.groups) {
      const k = laneOf[verts.find(v => groupOf(v) === g)!];
      laneSpan[k] = Math.max(laneSpan[k], Math.min(nameWidth(g), GROUP_NAME_MAX) - before - after);
    }
  }
  const laneStart: number[] = [];
  let cursor = 0;
  for (const k of lanes) {
    cursor += boxedLane[k] ? before : 0;
    laneStart.push(cursor);
    cursor += laneSpan[k] + (boxedLane[k] ? after : 0) + laneGap;
  }
  let along = grouped ? (right ? GROUP_PAD : GROUP_PAD + GROUP_HEAD) : 0;
  const layerStart: number[] = [];
  const layerThick: number[] = [];
  layers.forEach((l, li) => {
    const thick = Math.max(0, ...l.map(thickness));
    layerStart.push(along);
    layerThick.push(thick);
    for (const k of lanes) {
      const members = inLane(l, k);
      let at = laneStart[k] + (laneSpan[k] - span(members)) / 2;
      for (const v of members) {
        const p = box[v];
        if (right) { p.x = along + (thick - (v < real ? p.w : 0)) / 2; p.y = at; at += p.h + gapIn; }
        else { p.y = along + (thick - (v < real ? p.h : 0)) / 2; p.x = at; at += p.w + gapIn; }
      }
    }
    along += thick + gapAfter[li];
  });
  const groups: PlacedGroup[] = flow.groups.map(name => {
    const ms = placed.filter(p => p.group === name);
    let x = Math.min(...ms.map(p => p.x)) - GROUP_PAD;
    const y = Math.min(...ms.map(p => p.y)) - GROUP_PAD - GROUP_HEAD;
    let w = Math.max(...ms.map(p => p.x + p.w)) + GROUP_PAD - x;
    if (!right) {
      // Widen to the name, within the room its lane kept (centred on the members).
      const k = laneOf[index.get(ms[0].id)!];
      const room = laneSpan[k] + before + after;
      const want = Math.min(nameWidth(name), GROUP_NAME_MAX, room);
      if (want > w) { x -= (want - w) / 2; w = want; }
    }
    const fits = Math.max(1, Math.floor((w - 20) / GROUP_CHAR_W));
    return { name, x, y, w, h: Math.max(...ms.map(p => p.y + p.h)) + GROUP_PAD - y, text: clip(name, fits) };
  });
  /** The cross-axis position of a forward link's drawn curve at `at` along
   *  it. Each stretch is the renderer's cubic with both control points at
   *  its middle along the link: along it moves by 1.5t − 1.5t² + t³ (always
   *  rising, so bisection finds t), across by 3t² − 2t³. */
  const onPath = (points: Array<[number, number]>, at: number): number => {
    const al = (p: [number, number]) => (right ? p[0] : p[1]);
    const cr = (p: [number, number]) => (right ? p[1] : p[0]);
    let i = 0;
    while (i < points.length - 2 && al(points[i + 1]) < at) i++;
    const p = points[i], q = points[i + 1];
    const u = al(q) === al(p) ? 0.5 : Math.min(1, Math.max(0, (at - al(p)) / (al(q) - al(p))));
    let lo = 0, hi = 1;
    for (let n = 0; n < 30; n++) {
      const t = (lo + hi) / 2;
      if (1.5 * t - 1.5 * t * t + t * t * t < u) lo = t; else hi = t;
    }
    const t = (lo + hi) / 2;
    return cr(p) + (cr(q) - cr(p)) * t * t * (3 - 2 * t);
  };
  const edges: PlacedEdge[] = flow.edges.map((e, k) => {
    const a = placed[index.get(e.from)!], b = placed[index.get(e.to)!];
    const self = a === b;
    const back = self || b.layer <= a.layer;
    const text = linkText(e.label);
    const lw = linkWidth(text);
    let pts: { x1: number; y1: number; x2: number; y2: number };
    if (self) {
      pts = right
        ? { x1: a.x + a.w * 0.35, y1: a.y + a.h, x2: a.x + a.w * 0.65, y2: a.y + a.h }
        : { x1: a.x + a.w, y1: a.y + a.h * 0.3, x2: a.x + a.w, y2: a.y + a.h * 0.7 };
    } else if (right) {
      pts = back
        ? { x1: a.x + a.w / 2, y1: a.y + a.h, x2: b.x + b.w / 2, y2: b.y + b.h }
        : { x1: a.x + a.w, y1: a.y + a.h / 2, x2: b.x, y2: b.y + b.h / 2 };
    } else {
      pts = back
        ? { x1: a.x + a.w, y1: a.y + a.h / 2, x2: b.x + b.w, y2: b.y + b.h / 2 }
        : { x1: a.x + a.w / 2, y1: a.y + a.h, x2: b.x + b.w / 2, y2: b.y };
    }
    const points: Array<[number, number]> = [[pts.x1, pts.y1]];
    if (!back) for (const d of via[k]) points.push([box[d].x + box[d].w / 2, box[d].y + box[d].h / 2]);
    points.push([pts.x2, pts.y2]);
    const bulge = back ? (self ? 28 : BULGE) : 0;
    const far = bulge * 0.75;
    // Where the label may go, best first; it takes the first spot that
    // covers no node (labels draw above nodes). A forward link: the gap just
    // after its source, then the gap just before its target, then its
    // middle. A back link or loop: past the far point of its bulge, beside
    // its source, then its target, then halfway.
    const spots: Array<[number, number]> = [];
    if (back) {
      const outer = (right ? Math.max(pts.y1, pts.y2) : Math.max(pts.x1, pts.x2)) + far;
      for (const t of [0, 1, 0.5]) {
        const at = right ? pts.x1 + (pts.x2 - pts.x1) * t : pts.y1 + (pts.y2 - pts.y1) * t;
        spots.push(right ? [at, outer + 5] : [outer + lw / 2 - 4, at]);
      }
    } else {
      const gapMid = (li: number) => layerStart[li] + layerThick[li] + gapAfter[li] / 2;
      const s1 = right ? pts.x1 : pts.y1, s2 = right ? pts.x2 : pts.y2;
      for (const at of [gapMid(a.layer), gapMid(b.layer - 1), (s1 + s2) / 2]) {
        const cross = onPath(points, at);
        spots.push(right ? [at, cross] : [cross, at]);
      }
    }
    const clear = ([x, y]: [number, number]) => placed.every(n =>
      x + lw / 2 <= n.x || n.x + n.w <= x - lw / 2 || y + 9 <= n.y || n.y + n.h <= y - 9);
    const [lx, ly] = text ? spots.find(clear) ?? spots[0] : spots[spots.length - 1];
    return { ...e, ...pts, points, back, self, bulge, lx, ly, lw, text };
  });
  // Group-aware routes treat unrelated groups as solid obstacles. Endpoint
  // groups are permeable, so links can enter/leave at their border; their
  // other member nodes still remain obstacles.
  const routeBudget = { left: 100000 };
  const labelBoxes: { x: number; y: number; w: number; h: number }[] = [];
  if (routing && groups.length) for (const e of edges) {
    const a = placed[index.get(e.from)!], b = placed[index.get(e.to)!];
    const obstacles = [...groups.filter(g => g.name !== a.group && g.name !== b.group),
      ...placed];
    if (e.self) {
      const p: Point = [e.x1 + (right ? 0 : 12), e.y1 + (right ? 12 : 0)];
      const q: Point = [e.x2 + (right ? 0 : 12), e.y2 + (right ? 12 : 0)];
      const middle = routeFlow(p, q, obstacles, routeBudget);
      e.route = middle.length ? [[e.x1,e.y1], ...middle, [e.x2,e.y2]] : [];
    } else {
      const start: Point = [e.x1 + (right && !e.back || !right && e.back ? 12 : 0), e.y1 + (right && e.back || !right && !e.back ? 12 : 0)];
      const end: Point = [e.x2 + (right && !e.back ? -12 : !right && e.back ? 12 : 0), e.y2 + (!right && !e.back ? -12 : right && e.back ? 12 : 0)];
      const middle = routeFlow(start,end,obstacles,routeBudget);
      e.route = middle.length ? [[e.x1,e.y1], ...middle, [e.x2,e.y2]] : [];
    }
    if (e.text && e.route.length > 1) {
      const segments = e.route.slice(1).map((p,i) => ({ p, q: e.route![i], length: Math.hypot(p[0]-e.route![i][0],p[1]-e.route![i][1]) })).sort((a,b)=>b.length-a.length);
      const spots = segments.flatMap(({p,q}) => [0.5,0.25,0.75].map(t => ({x:q[0]+(p[0]-q[0])*t,y:q[1]+(p[1]-q[1])*t})));
      const occupied = [...placed, ...groups.filter(g => g.name !== a.group && g.name !== b.group), ...groups.map(g => ({...g,h:22})), ...labelBoxes];
      const clear = spots.find(p => occupied.every(n =>
        p.x+e.lw/2+2 <= n.x || p.x-e.lw/2-2 >= n.x+n.w || p.y+11 <= n.y || p.y-11 >= n.y+n.h));
      if (clear) { e.lx=clear.x; e.ly=clear.y; labelBoxes.push({x:e.lx-e.lw/2,y:e.ly-9,w:e.lw,h:18}); }
      else e.text = ''; // The full connection stays in its accessible name and text list.
    }
  }
  // Bounds of everything drawn, then shift it all inside the margin.
  const xs: number[] = [], ys: number[] = [];
  for (const p of placed) { xs.push(p.x, p.x + p.w); ys.push(p.y, p.y + p.h); }
  for (const g of groups) { xs.push(g.x, g.x + g.w); ys.push(g.y, g.y + g.h); }
  for (const e of edges) {
    for (const [x, y] of [...e.points, ...(e.route ?? [])]) { xs.push(x); ys.push(y); }
    if (e.text) { xs.push(e.lx - e.lw / 2, e.lx + e.lw / 2); ys.push(e.ly - 9, e.ly + 9); }
    if (e.back) {
      if (right) ys.push(Math.max(e.y1, e.y2) + e.bulge);
      else xs.push(Math.max(e.x1, e.x2) + e.bulge);
    }
  }
  const minX = Math.min(0, ...xs), minY = Math.min(0, ...ys);
  const dx = MARGIN - minX, dy = MARGIN - minY;
  for (const p of placed) { p.x += dx; p.y += dy; }
  for (const g of groups) { g.x += dx; g.y += dy; }
  for (const e of edges) {
    e.x1 += dx; e.x2 += dx; e.lx += dx; e.y1 += dy; e.y2 += dy; e.ly += dy;
    e.points = e.points.map(([x, y]) => [x + dx, y + dy]);
    if (e.route) e.route = e.route.map(([x,y]) => [x+dx,y+dy]);
  }
  return {
    nodes: placed, edges, groups,
    width: Math.ceil(Math.max(0, ...xs) + dx + MARGIN),
    height: Math.ceil(Math.max(0, ...ys) + dy + MARGIN),
  };
}
