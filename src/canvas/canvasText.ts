/**
 * Read as text (plan L3 X9, 2026-10-14; review U canvases): a canvas's words
 * without its frame, for anyone the drawn canvas doesn't serve (a screen
 * reader meeting a chart, a canvas whose scripts can't run, a reader who
 * wants to copy a figure). Every host's ⋯ menu offers it
 * (`components/HtmlArtifact.tsx`), which shows the result in the page as
 * text only, never as markup.
 *
 * It reuses the canvas model walk (`canvasModel.ts`), which
 * says stats and charts as data tables and steps as lists. This adapter also
 * reads hc-checklist marks as list items, hc-flow links in words ("Plan → Build:
 * when ready") instead of Mermaid, and images as their alt text.
 *
 * Pure apart from `readAsText`, which parses with the page's DOMParser (an
 * inert document: nothing in it runs or loads).
 */
import { canvasModel, type El, type Item } from './canvasModel';
import { parseFlow } from './kit/flow';
import { checklistRows } from './kit/checklist';

export type TextBlock =
  | { kind: 'heading'; level: 2 | 3; text: string }
  | { kind: 'text'; text: string }
  | { kind: 'list'; ordered: boolean; items: string[] }
  | { kind: 'table'; caption: string; head: string[] | null; rows: string[][] }
  | { kind: 'pre'; label: string; text: string };

/** Most blocks shown; a longer canvas says how many more there are. */
export const TEXT_BLOCKS_MAX = 400;

const SKIP = new Set(['script', 'style', 'template']);
const words = (s: string | null | undefined) => (s ?? '').replace(/\s+/g, ' ').trim();

type DomLike = { nodeType: number; nodeName: string; nodeValue: string | null; childNodes?: ArrayLike<DomLike> };

/** Text without what scripts and styles carry, when the element is a real
 *  DOM node; a plain test object's own text otherwise. */
function cleanText(el: El): string | null {
  const nodes = (el as unknown as DomLike).childNodes;
  if (!nodes) return el.textContent;
  let out = '';
  for (const n of Array.from(nodes)) {
    if (n.nodeType === 3) out += n.nodeValue ?? '';
    else if (n.nodeType === 1 && !SKIP.has(n.nodeName.toLowerCase())) out += cleanText(n as unknown as El) ?? '';
  }
  return out;
}

const plain = (tagName: string, text: string, children: El[] = []): El => ({
  tagName, getAttribute: () => null, children, textContent: text,
});

/** An `hc-flow` as a heading (its label) and a list of its links in words. */
function flowAsList(el: El): El {
  const flow = parseFlow(el.textContent ?? '');
  const name = new Map(flow.nodes.map(n => [n.id, n.label]));
  const linked = new Set(flow.edges.flatMap(e => [e.from, e.to]));
  const lines = [
    ...flow.edges.map(e => `${name.get(e.from) ?? e.from} → ${name.get(e.to) ?? e.to}${e.label ? `: ${e.label}` : ''}`),
    ...flow.nodes.filter(n => !linked.has(n.id)).map(n => n.label),
  ];
  if (flow.droppedNodes || flow.droppedLinks) lines.push(`${flow.droppedNodes} nodes and ${flow.droppedLinks} links not drawn`);
  const label = words(el.getAttribute('label')) || 'Diagram';
  return plain('SECTION', '', [plain('H3', label), plain('UL', '', lines.map(l => plain('LI', l)))]);
}

/** The element as the walk should read it. */
function adapt(el: El): El {
  const tag = el.tagName.toLowerCase();
  if (tag === 'hc-flow') return flowAsList(el);
  if (tag === 'hc-checklist') {
    const children = Array.from(el.children).filter(c => !SKIP.has(c.tagName.toLowerCase()));
    const listItems = (nodes: El[]): El[] => nodes.flatMap(c => [
      ...(c.tagName.toLowerCase() === 'li' ? [c] : []),
      ...listItems(Array.from(c.children).filter(n => !SKIP.has(n.tagName.toLowerCase()))),
    ]);
    const li = listItems(children);
    const candidates = li.length ? li : children;
    const { rows, repeated } = checklistRows(candidates.length ? candidates.map(c => cleanText(c) ?? '') : (cleanText(el) ?? '').split('\n'));
    const label = words(el.getAttribute('label')) || 'Checklist';
    return plain('SECTION', '', [plain('H3', label), plain('UL', '', rows.map(r => plain('LI', `${r.done ? 'Done' : 'Not done'}: ${r.label}`))),
      ...(repeated ? [plain('P', `${repeated} repeated rows shown once; first mark kept.`)] : [])]);
  }
  if (tag === 'img') { const alt = words(el.getAttribute('alt')); return plain('P', alt ? `Image: ${alt}` : ''); }
  return {
    tagName: el.tagName,
    getAttribute: (n: string) => el.getAttribute(n),
    get children() { return Array.from(el.children).map(adapt); },
    get textContent() { return cleanText(el); },
  };
}

const TONE: Record<string, string> = { info: 'Note', success: 'Done', warning: 'Warning', danger: 'Problem', neutral: '' };

function toBlocks(items: Item[]): TextBlock[] {
  const out: TextBlock[] = [];
  const listItem = (ordered: boolean, text: string) => {
    const last = out[out.length - 1];
    if (last?.kind === 'list' && last.ordered === ordered) last.items.push(text);
    else out.push({ kind: 'list', ordered, items: [text] });
  };
  for (const it of items) {
    switch (it.kind) {
      case 'heading': if (it.text) out.push({ kind: 'heading', level: it.level, text: it.text }); break;
      // The model walk introduces a chart's table "in Hollow"; here it is.
      case 'paragraph': case 'quote': if (it.text) out.push({ kind: 'text', text: it.text.replace(/ in Hollow; its data:$/, '; its data:') }); break;
      case 'callout': {
        const lead = [TONE[it.tone], it.title].filter(Boolean).join(': ');
        const text = lead && it.text ? `${lead}. ${it.text}` : lead || it.text;
        if (text) out.push({ kind: 'text', text });
        break;
      }
      case 'bullet': listItem(!!it.numbered, it.text); break;
      case 'todo': listItem(false, `${it.done ? 'Done' : 'Not done'}: ${it.text}`); break;
      case 'table': {
        if (!it.rows.length) break;
        out.push({ kind: 'table', caption: it.label, head: it.header ? it.rows[0] : null, rows: it.header ? it.rows.slice(1) : it.rows });
        break;
      }
      case 'code': if (it.text.trim()) out.push({ kind: 'pre', label: it.label, text: it.text.replace(/^\n+|\s+$/g, '') }); break;
      case 'toggle': if (it.text) out.push({ kind: 'heading', level: 3, text: it.text }); out.push(...toBlocks(it.items)); break;
      case 'divider': break;
    }
  }
  return out;
}

/** A canvas root (`<hc-canvas>`, or an html fence's body) as text blocks,
 *  in order, at most `TEXT_BLOCKS_MAX` plus a line saying how many more. */
export function canvasText(root: El): TextBlock[] {
  const all = toBlocks(canvasModel(adapt(root)).items);
  if (all.length <= TEXT_BLOCKS_MAX) return all;
  return [...all.slice(0, TEXT_BLOCKS_MAX), { kind: 'text', text: `${all.length - TEXT_BLOCKS_MAX} more parts not shown. View source has everything.` }];
}

/** The stored HTML (a canvas fence body or an html fence) as text blocks.
 *  Parsed into an inert document: no script runs, nothing loads. */
export function readAsText(source: string): TextBlock[] {
  const doc = new DOMParser().parseFromString(source, 'text/html');
  const root = doc.querySelector('hc-canvas') ?? doc.body;
  return root ? canvasText(root as unknown as El) : [];
}
