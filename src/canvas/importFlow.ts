/** Bounded, view-only Mermaid flowchart import. No Mermaid runtime, DOM or
 * execution: styling/click instructions are counted, never interpreted.
 * This host-only module is loaded with MermaidDiagram, not with the kit. */
import { FLOW_SHAPES, parseFlow, type FlowShape } from './kit/flow';
import { KIT_TONES } from './kit/manifest';

type Node = { id: string; label: string; shape: FlowShape | 'round'; group: string; depth: number };
type Edge = { from: string; to: string; label: string };
export type ImportFlow = { ok: true; flow: string; direction: 'down' | 'right'; title: string; note: string;
  counts: { shapes: number; nodes: number; links: number; droppedNodes: number; droppedLinks: number } }
  | { ok: false; kind: string; reason: string };

const escapeHtml = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
function label(raw: string): string {
  const quoted = raw.trim().replace(/^"([\s\S]*)"$/, '$1');
  const plain = quoted.startsWith('`') && quoted.endsWith('`')
    ? quoted.slice(1, -1).replace(/(\*\*|\*)(.+?)\1/g, '$2').replace(/(^|\s)(__|_)(.+?)\2(?=\s|$)/g, '$1$3') : quoted;
  return plain.replace(/<br\s*\/?\s*>/gi, ' ').replace(/(^|\s)fa[bsrl]?:fa-[\w-]+/g, '$1')
    .replace(/#(quot|lt|gt|amp|\d+);|&(quot|lt|gt|amp);/g, (_, a: string, b: string) => {
      const key = a || b;
      if (/^\d+$/.test(key)) { const n = Number(key); return n > 0 && n <= 0x10ffff ? String.fromCodePoint(n) : ''; }
      return ({ quot: '"', lt: '<', gt: '>', amp: '&' } as Record<string, string>)[key];
    }).replace(/[\s\u0000-\u001f]+/g, ' ').trim();
}
// Keep syntax-looking label endings literal in hc-flow's line grammar.
function literal(s: string): string {
  return s.replace(/\{([^{}]*)\}\s*$/, (all, words: string) => {
    const ws = words.toLowerCase().split(/[\s,]+/).filter(Boolean);
    return ws.length && ws.every(w => [...FLOW_SHAPES, ...Object.keys(KIT_TONES), 'active'].includes(w)) ? `(${words})` : all;
  });
}

/** Split only outside quoted labels and shape brackets. Comments never
 * swallow a quoted URL or label. The input/line bound is checked first. */
function statements(text: string): string[] {
  const out: string[] = []; let current = '', quote = '', depth = 0;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (quote) { current += c; if (c === quote && text[i - 1] !== '\\') quote = ''; continue; }
    if (c === '"' || c === '`') { quote = c; current += c; continue; }
    if (c === '%' && text[i + 1] === '%') { while (i < text.length && text[i] !== '\n') i++; }
    else if ('[({'.includes(c)) depth++;
    else if ('])}'.includes(c)) depth = Math.max(0, depth - 1);
    if (text[i] === '\n' || (c === ';' && depth === 0)) { if (current.trim()) out.push(current.trim()); current = ''; depth = 0; }
    else current += text[i] ?? '';
  }
  if (current.trim()) out.push(current.trim());
  return out;
}

const SHAPES: Array<[string, string, Node['shape']]> = [
  ['(((', ')))', 'round'], ['([', '])', 'round'], ['[[', ']]', 'box'], ['[(', ')]', 'box'],
  ['((', '))', 'round'], ['{{', '}}', 'decision'], ['[/', '/]', 'box'], ['[\\', '\\]', 'box'],
  ['[/', '\\]', 'box'], ['[\\', '/]', 'box'], ['[', ']', 'box'], ['(', ')', 'box'],
  ['{', '}', 'decision'], ['>', ']', 'note'],
];
function endQuoteAware(text: string, start: number, close: string): number {
  let quote = '';
  for (let i = start; i < text.length; i++) {
    const c = text[i];
    if (quote) { if (c === quote && text[i - 1] !== '\\') quote = ''; }
    else if (c === '"' || c === '`') quote = c;
    else if (text.startsWith(close, i)) return i;
  }
  return -1;
}

export function importMermaid(input: string): ImportFlow {
  const fail = (kind: string, reason: string): ImportFlow => ({ ok: false, kind, reason });
  if (input.length > 65536 || new TextEncoder().encode(input).length > 65536 || input.replace(/\r?\n$/, '').split('\n').length > 2000)
    return fail('large diagram', 'The limit is 64 KB and 2,000 lines.');
  let title = 'Mermaid flowchart';
  const text = input.replace(/^\s*---\r?\n([\s\S]*?)\r?\n---\s*(?:\r?\n|$)/, (_, yaml: string) => {
    const t = /^title:\s*(.*)$/m.exec(yaml); if (t) title = label(t[1]).replace(/^'(.*)'$/, '$1'); return '';
  });
  const lines = statements(text);
  const header = /^(?:flowchart|graph)(?=[ \t]|$)(?:[ \t]+(TB|TD|BT|LR|RL)\b)?[ \t]*(.*)$/i.exec(lines.shift() ?? '');
  if (!header) {
    const first = text.replace(/^\s*%%[^\n]*(?:\n|$)/gm, '').trim().split(/\s/)[0] || 'empty diagram';
    const kinds: Record<string, string> = { sequenceDiagram: 'sequence diagram', classDiagram: 'class diagram', stateDiagram: 'state diagram', 'stateDiagram-v2': 'state diagram', erDiagram: 'entity relationship diagram', gitGraph: 'Git graph', gantt: 'Gantt chart', pie: 'pie chart', journey: 'user journey', mindmap: 'mind map' };
    return fail(kinds[first] ?? first, 'Hollow draws Mermaid flowcharts.');
  }
  if (header[2]) lines.unshift(header[2]);
  const orientation = header[1] ?? 'TD';
  const direction = /LR|RL/i.test(orientation) ? 'right' : 'down';
  const nodes = new Map<string, Node>(), edges: Edge[] = [], groups = new Map<string, string>();
  const stack: string[] = [], omitted = new Map<string, number>();
  let shapes = 0, excessEdges = 0;
  const omit = (what: string, n = 1) => omitted.set(what, (omitted.get(what) ?? 0) + n);
  function nodeAt(s: string): { id: string; rest: string } | null {
    const m = /^((?:[\p{L}\p{N}_]|-(?![-.]))+)/u.exec(s.trimStart()); if (!m) return null;
    const id = m[1]; let rest = s.trimStart().slice(id.length).trimStart();
    let content: string | null = null, shape: Node['shape'] = 'box';
    if (rest.startsWith('@{')) {
      const end = endQuoteAware(rest, 2, '}'); if (end < 0) return null;
      const attrs = rest.slice(2, end);
      const name = /\bshape\s*:\s*([\w-]+)/.exec(attrs)?.[1];
      const value = /\blabel\s*:\s*("(?:\\.|[^"\\])*"|[^,}]+)/.exec(attrs)?.[1];
      content = value ?? id;
      shape = ['diam', 'diamond', 'decision', 'question'].includes(name ?? '') ? 'decision'
        : ['circle', 'dbl-circ', 'stadium', 'sm-circ', 'fr-circ'].includes(name ?? '') ? 'round'
        : name === 'flag' ? 'note' : 'box';
      rest = rest.slice(end + 1).trimStart();
    } else {
      for (const [open, close, kind] of SHAPES) {
        if (!rest.startsWith(open)) continue;
        const end = endQuoteAware(rest, open.length, close); if (end < 0) continue;
        content = rest.slice(open.length, end); shape = kind; rest = rest.slice(end + close.length).trimStart(); break;
      }
    }
    if (content !== null) shapes++;
    const old = nodes.get(id);
    const deeper = stack.length > (old?.depth ?? 0);
    const group = deeper ? stack.at(-1)! : old?.group ?? '', depth = deeper ? stack.length : old?.depth ?? 0;
    if (!old || content !== null) nodes.set(id, { id, label: content === null ? id : label(content), shape: content === null ? old?.shape ?? 'box' : shape, group, depth });
    else if (deeper) { old.group = group; old.depth = depth; }
    rest = rest.replace(/^:::[\w-]+\s*/, '');
    return { id, rest };
  }
  function ends(s: string): { ids: string[]; rest: string } | null {
    const first = nodeAt(s); if (!first) return null;
    const ids = [first.id]; let rest = first.rest;
    while (/^&\s*/.test(rest)) {
      const next = nodeAt(rest.replace(/^&\s*/, '')); if (!next) break;
      ids.push(next.id); rest = next.rest;
    }
    return { ids, rest };
  }
  function arrow(s: string): { rest: string; label: string; both: boolean; hidden: boolean } | null {
    s = s.replace(/^[\w-]+@(?=[<.=-])/, '');
    const direct = /^(?:<[-=]{2,}>|[-=]{2,}[>ox]?|-\.[.\-]*->|~{3,})/.exec(s);
    if (direct) {
      let rest = s.slice(direct[0].length).trimStart(), value = '';
      if (rest.startsWith('|')) {
        const end = endQuoteAware(rest, 1, '|'); if (end < 0) return null;
        value = label(rest.slice(1, end)); rest = rest.slice(end + 1).trimStart();
      }
      // `-- text -->` starts with a bare two-character stroke.
      if (!['--', '=='].includes(direct[0])) return { rest, label: value, both: direct[0].startsWith('<'), hidden: direct[0].startsWith('~') };
    }
    const labelled = /^(?:--\s+(.+?)\s+--+>|-\.\s+(.+?)\s+\.->|==\s+(.+?)\s+==+>)\s*/.exec(s);
    return labelled ? { rest: s.slice(labelled[0].length), label: label(labelled[1] ?? labelled[2] ?? labelled[3]), both: false, hidden: false } : null;
  }
  for (const line of lines) {
    if (/^(classDef|class|style|linkStyle|click|direction)\b/.test(line)) { omit(line.startsWith('click') ? 'click handler' : line.startsWith('direction') ? 'direction line' : 'style line'); continue; }
    if (/^subgraph\s/.test(line)) {
      const raw = line.replace(/^subgraph\s+/, '');
      const explicit = /^([\w-]+)\s*\[([\s\S]*)\]$/.exec(raw);
      const id = explicit?.[1] ?? raw, name = label(explicit?.[2] ?? raw);
      groups.set(id, name); if (stack.length) omit('nested group'); stack.push(name); continue;
    }
    if (line === 'end') { stack.pop(); continue; }
    let a = ends(line); if (!a) { omit('unrecognized statement'); continue; }
    while (a.rest) {
      const link = arrow(a.rest); const b = link && ends(link.rest);
      if (!link || !b) { omit('unrecognized statement'); break; }
      if (link.hidden) omit('invisible link', a.ids.length * b.ids.length);
      else {
        const directions = link.both ? 2 : 1, total = a.ids.length * b.ids.length * directions;
        const take = Math.min(total, 4000 - edges.length);
        for (let i = 0; i < take; i++) {
          const pair = Math.floor(i / directions), from = a.ids[Math.floor(pair / b.ids.length)], to = b.ids[pair % b.ids.length];
          edges.push(i % directions ? { from: to, to: from, label: literal(link.label) } : { from, to, label: literal(link.label) });
        }
        excessEdges += total - take;
      }
      a = b;
    }
  }
  for (const id of groups.keys()) nodes.delete(id);
  const keptEdges = edges.filter(e => { const drop = groups.has(e.from) || groups.has(e.to); if (drop) omit('subgraph link'); return !drop; });
  if (!nodes.size) return fail('empty flowchart', 'No readable nodes were found.');
  const ids = new Map<string, string>(), used = new Set<string>();
  for (const id of nodes.keys()) {
    const base = id.replace(/[^A-Za-z0-9_-]/g, '_') || 'node'; let safe = base, n = 1;
    while (used.has(safe)) safe = `${base}_${n++}`;
    used.add(safe); ids.set(id, safe);
  }
  const output: string[] = []; let group = '';
  for (const n of nodes.values()) {
    if (n.group !== group) { group = n.group; output.push(`[${group.replace(/->/g, '→')}]`); }
    const shape = n.shape !== 'round' ? n.shape : !keptEdges.some(e => e.to === n.id) ? 'start' : !keptEdges.some(e => e.from === n.id) ? 'end' : 'box';
    output.push(`${ids.get(n.id)}: ${literal(n.label)} {${shape}}`);
  }
  output.push('[]', ...keptEdges.map(e => `${ids.get(e.from)} -> ${ids.get(e.to)}${e.label ? `: ${e.label}` : ''}`));
  const flow = output.join('\n'), parsed = parseFlow(flow);
  const left = [...omitted].map(([name, n]) => `${n} ${name}${n === 1 ? '' : 's'}`);
  let note = `Drawn from Mermaid: ${shapes} shapes → ${parsed.nodes.length} nodes, ${parsed.edges.length} links.`;
  if (left.length) note += ` Left out: ${left.join(', ')}.`;
  if (parsed.droppedNodes) note += ` ${parsed.droppedNodes} node${parsed.droppedNodes === 1 ? '' : 's'} not shown; Hollow draws at most 60.`;
  if (parsed.droppedLinks + excessEdges) note += ` ${parsed.droppedLinks + excessEdges} link${parsed.droppedLinks + excessEdges === 1 ? '' : 's'} not shown; Hollow draws at most 120.`;
  if (/BT|RL/i.test(orientation)) note += ` Direction ${orientation.toUpperCase()} drawn ${direction === 'right' ? 'left-right' : 'top-down'}.`;
  return { ok: true, flow, direction, title, note, counts: { shapes, nodes: parsed.nodes.length, links: parsed.edges.length, droppedNodes: parsed.droppedNodes, droppedLinks: parsed.droppedLinks + excessEdges } };
}

export function flowHtml(r: Extract<ImportFlow, { ok: true }>): string {
  return `<hc-flow label="${escapeHtml(r.title)}" direction="${r.direction}">${escapeHtml(r.flow)}</hc-flow>`;
}
