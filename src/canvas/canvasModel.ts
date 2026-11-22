/**
 * A canvas as a plain model (moved here on 2026-11-10 from the canvas page
 * export, removed with its writes). Pure.
 * `canvasModel` walks anything shaped like a DOM element (`El`; the app passes
 * a parsed `<hc-canvas>`) and says what the canvas holds: stats and charts as
 * data tables, steps as to-dos, an `hc-flow` as a Mermaid code block. Read as
 * text (`canvasText.ts`) is its reader; `mermaid` also serves the flow import
 * tests.
 */
import { parseRows, toChartData } from './kit/data';
import { parseFlow, type Flow } from './kit/flow';

/** The part of a DOM element the walk reads (tests pass plain objects). */
export interface El {
  tagName: string;
  getAttribute(name: string): string | null;
  readonly children: ArrayLike<El>;
  readonly textContent: string | null;
}

type Tone = 'info' | 'success' | 'warning' | 'danger' | 'neutral';
export type Item =
  | { kind: 'heading'; level: 2 | 3; text: string }
  | { kind: 'paragraph'; text: string }
  | { kind: 'callout'; tone: Tone; title: string; text: string }
  | { kind: 'table'; label: string; rows: string[][]; header: boolean }
  | { kind: 'todo'; text: string; done: boolean }
  | { kind: 'bullet'; text: string; numbered?: boolean }
  | { kind: 'code'; label: string; language: string; text: string }
  | { kind: 'quote'; text: string }
  | { kind: 'divider' }
  | { kind: 'toggle'; text: string; items: Item[] };

const words = (s: string | null | undefined) => (s ?? '').replace(/\s+/g, ' ').trim();
const tag = (el: El) => el.tagName.toLowerCase();
const kids = (el: El): El[] => Array.from(el.children);
const at = (el: El, name: string) => words(el.getAttribute(name));
const toneOf = (el: El): Tone => {
  const t = at(el, 'tone').toLowerCase();
  return t === 'info' || t === 'success' || t === 'warning' || t === 'danger' ? t : 'neutral';
};
/** A kit data element's own text: its rows, without upgraded parts. */
const sourceText = (el: El) => el.textContent ?? '';

/** The model of a canvas: its title, then its items in order. */
export function canvasModel(root: El): { title: string; items: Item[] } {
  return { title: at(root, 'title'), items: walk(kids(root)) };
}

function walk(els: El[]): Item[] {
  const out: Item[] = [];
  for (const el of els) out.push(...item(el));
  return out;
}

function item(el: El): Item[] {
  const t = tag(el);
  const label = at(el, 'label');
  const titled = (items: Item[]): Item[] => (label ? [{ kind: 'heading', level: 3, text: label }, ...items] : items);
  switch (t) {
    case 'script': case 'style': case 'template': return [];
    case 'hc-stats': {
      const rows = kids(el).filter(k => tag(k) === 'hc-stat').map(s => {
        const tone = toneOf(s);
        const mark = tone === 'neutral' ? '' : `${TONE_WORD[tone]}: `;
        return [at(s, 'label'), at(s, 'value'), `${mark}${at(s, 'delta')}`.replace(/: $/, '')];
      });
      if (!rows.length) return [];
      const hasNote = rows.some(r => r[2]);
      return [{ kind: 'table', label, header: true, rows: [hasNote ? ['Metric', 'Value', 'Note'] : ['Metric', 'Value'], ...rows.map(r => (hasNote ? r : r.slice(0, 2)))] }];
    }
    case 'hc-stat': return [{ kind: 'paragraph', text: [at(el, 'label'), at(el, 'value'), at(el, 'delta')].filter(Boolean).join(': ') }];
    case 'hc-callout': return [{ kind: 'callout', tone: toneOf(el), title: at(el, 'title'), text: words(el.textContent) }];
    case 'hc-steps': {
      const cur = Number(at(el, 'current')) || 0;
      return titled(kids(el).map((s, i) => {
        const status = at(s, 'status') || (cur ? (i + 1 < cur ? 'done' : i + 1 === cur ? 'current' : 'upcoming') : 'upcoming');
        const when = at(s, 'when');
        const detail = words(s.textContent);
        const text = [at(s, 'label') + (status === 'current' ? ' (now)' : status === 'blocked' ? ' (blocked)' : ''), when, detail].filter(Boolean).join(' · ');
        return { kind: 'todo' as const, text, done: status === 'done' };
      }));
    }
    case 'hc-table': {
      const table = kids(el).find(k => tag(k) === 'table');
      const rows = table ? tableRows(table) : parseRows(sourceText(el));
      return rows.length ? [{ kind: 'table', label, header: true, rows }] : [];
    }
    case 'hc-chart': case 'hc-bars': {
      const table = kids(el).find(k => tag(k) === 'table');
      const data = table
        ? toChartData(tableRows(table), splitNames(at(el, 'series')), 'Value', true)
        : toChartData(parseRows(sourceText(el)), splitNames(at(el, 'series')));
      const unit = at(el, 'unit');
      const head = [data.head || 'Item', ...data.series.map(s => (unit && data.series.length === 1 ? `${s.name} (${unit})` : s.name))];
      const rows = data.categories.map((c, i) => [c, ...data.series.map(s => s.cells[i] ?? '')]);
      if (!rows.length) return [];
      return [
        { kind: 'heading', level: 3, text: label || 'Chart' },
        { kind: 'paragraph', text: `${t === 'hc-bars' ? 'Bars' : `A ${at(el, 'type') || 'line'} chart`} in Hollow; its data:` },
        { kind: 'table', label: '', header: true, rows: [head, ...rows] },
      ];
    }
    case 'hc-flow': {
      const flow = parseFlow(sourceText(el));
      if (!flow.nodes.length) return [];
      return [{ kind: 'code', label: label || 'Diagram', language: 'mermaid', text: mermaid(flow, at(el, 'direction') === 'right' ? 'LR' : 'TD') }];
    }
    case 'hc-compare': return titled(kids(el).map(o => ({
      kind: 'bullet' as const,
      text: `${at(o, 'label')}${o.getAttribute('recommended') !== null ? ' (recommended)' : ''}: ${words(o.textContent)}`,
    })));
    case 'hc-progress': {
      const v = Number(at(el, 'value')) || 0, max = Number(at(el, 'max')) || 100;
      return [{ kind: 'paragraph', text: `${label || 'Progress'}: ${v} of ${max} (${Math.round((v / max) * 100)}%)` }];
    }
    case 'hc-details': return [{ kind: 'toggle', text: at(el, 'summary') || 'Details', items: block(kids(el), el) }];
    case 'hc-tabs': return titled(kids(el).flatMap(p => [{ kind: 'heading' as const, level: 3 as const, text: at(p, 'label') }, ...block(kids(p), p)]));
    case 'hc-code': return [{ kind: 'code', label: at(el, 'label'), language: codeLanguage(at(el, 'lang')), text: el.textContent ?? '' }];
    case 'hc-file': return [{ kind: 'paragraph', text: `${words(el.textContent) || 'File'} (${at(el, 'path')})` }];
    case 'hc-badge': return [{ kind: 'paragraph', text: words(el.textContent) }];
    case 'h1': case 'h2': return [{ kind: 'heading', level: 2, text: words(el.textContent) }];
    case 'h3': case 'h4': case 'h5': case 'h6': return [{ kind: 'heading', level: 3, text: words(el.textContent) }];
    case 'ul': case 'ol': return kids(el).filter(k => tag(k) === 'li').map(li => ({ kind: 'bullet' as const, text: words(li.textContent), numbered: t === 'ol' }));
    case 'table': { const rows = tableRows(el); return rows.length ? [{ kind: 'table', label: '', header: true, rows }] : []; }
    case 'pre': return [{ kind: 'code', label: '', language: 'plain text', text: el.textContent ?? '' }];
    case 'blockquote': return [{ kind: 'quote', text: words(el.textContent) }];
    case 'hr': return [{ kind: 'divider' }];
    case 'div': case 'section': case 'article': case 'main': case 'header': case 'footer': {
      const inner = kids(el);
      return inner.length ? block(inner, el) : textItem(el);
    }
    default: return textItem(el);
  }
}

/** A container's children, or its text when it has only inline content. */
function block(inner: El[], el: El): Item[] {
  const blocky = inner.some(k => BLOCK_TAGS.has(tag(k)) || tag(k).startsWith('hc-'));
  return blocky ? walk(inner) : textItem(el);
}
const BLOCK_TAGS = new Set(['div', 'section', 'article', 'main', 'header', 'footer', 'p', 'ul', 'ol', 'table', 'pre', 'blockquote', 'hr', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6']);
function textItem(el: El): Item[] {
  const text = words(el.textContent);
  return text ? [{ kind: 'paragraph', text }] : [];
}

function tableRows(table: El): string[][] {
  const rows: string[][] = [];
  const visit = (e: El) => {
    for (const k of kids(e)) {
      const t = tag(k);
      if (t === 'tr') rows.push(kids(k).filter(c => tag(c) === 'td' || tag(c) === 'th').map(c => words(c.textContent).replace(/\s*[↕↑↓]$/, '')));
      else if (t === 'thead' || t === 'tbody' || t === 'tfoot') visit(k);
    }
  };
  visit(table);
  return rows.filter(r => r.length);
}

function splitNames(s: string): string[] {
  return s ? s.split(',').map(x => x.trim()).filter(Boolean) : [];
}

const TONE_WORD: Record<Exclude<Tone, 'neutral'>, string> = { info: 'Info', success: 'Done', warning: 'Warning', danger: 'Problem' };

/** An `hc-flow` as Mermaid source. */
export function mermaid(flow: Flow, dir: 'TD' | 'LR'): string {
  const id = new Map(flow.nodes.map((n, i) => [n.id, `n${i}`]));
  const q = (s: string) => `"${s.replace(/"/g, '#quot;').replace(/</g, '#lt;').replace(/>/g, '#gt;')}"`;
  const node = (n: Flow['nodes'][number]) => {
    const l = q(n.label);
    const shape = n.shape === 'decision' ? `{${l}}` : n.shape === 'start' || n.shape === 'end' ? `([${l}])` : n.shape === 'note' ? `>${l}]` : `[${l}]`;
    return `  ${id.get(n.id)}${shape}`;
  };
  const lines = [`flowchart ${dir}`];
  const groups = [...new Set(flow.nodes.map(n => n.group))];
  groups.forEach((g, gi) => {
    const members = flow.nodes.filter(n => n.group === g);
    if (!g) { lines.push(...members.map(node)); return; }
    lines.push(`  subgraph g${gi}[${q(g)}]`, ...members.map(n => `  ${node(n)}`), '  end');
  });
  for (const e of flow.edges) {
    const a = id.get(e.from), b = id.get(e.to);
    if (a && b) lines.push(e.label ? `  ${a} -->|${q(e.label)}| ${b}` : `  ${a} --> ${b}`);
  }
  return lines.join('\n');
}

const LANGS = new Set(['javascript', 'typescript', 'python', 'rust', 'json', 'bash', 'shell', 'sql', 'html', 'css', 'markdown', 'yaml', 'go', 'java', 'swift', 'ruby', 'mermaid']);
/** A code language by its common name, or plain text when unknown. */
export function codeLanguage(lang: string): string {
  const l = lang.toLowerCase();
  const alias: Record<string, string> = { js: 'javascript', ts: 'typescript', py: 'python', rs: 'rust', sh: 'shell', zsh: 'shell', md: 'markdown', yml: 'yaml' };
  const name = alias[l] ?? l;
  return LANGS.has(name) ? name : 'plain text';
}

