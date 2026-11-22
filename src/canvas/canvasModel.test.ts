import { describe, expect, it } from 'vitest';
import { canvasModel, codeLanguage, mermaid, type El } from './canvasModel';
import { parseFlow } from './kit/flow';

// A tiny element tree: what the walk reads of a DOM element.
function el(tagName: string, attrs: Record<string, string> = {}, ...children: Array<El | string>): El {
  const kids = children.filter((c): c is El => typeof c !== 'string');
  const text = () => children.map(c => (typeof c === 'string' ? c : c.textContent ?? '')).join('');
  return { tagName: tagName.toUpperCase(), getAttribute: (n) => (n in attrs ? attrs[n] : null), children: kids, get textContent() { return text(); } };
}

describe('a canvas as a model', () => {
  const canvas = el('hc-canvas', { id: 'release', title: 'Release slips to Friday' },
    el('hc-stats', {},
      el('hc-stat', { label: 'Checks', value: '41', delta: '+3', tone: 'success' }),
      el('hc-stat', { label: 'Open bugs', value: '2', tone: 'warning' })),
    el('hc-callout', { tone: 'warning', title: 'Heads up' }, 'Install waits for the bed.'),
    el('hc-steps', { current: '2' }, el('div', { label: 'Merge', status: 'done' }), el('div', { label: 'Test bed' }, 'Running now'), el('div', { label: 'Install' })),
    el('hc-chart', { type: 'bar', label: 'Hours by lane', unit: 'h' }, 'Lane A: 12\nLane B: 30'),
    el('hc-flow', { label: 'Path', direction: 'right' }, '[Front]\napp: App "web"\n[]\napi: API {success}\napp -> api: calls'),
    el('hc-compare', {}, el('div', { label: 'Fly', recommended: '' }, 'Closer'), el('div', { label: 'Render' }, 'Simpler')),
    el('hc-details', { summary: 'Why' }, el('p', {}, 'Because.')),
    el('script', {}, 'alert(1)'),
  );
  const model = canvasModel(canvas);

  it('keeps the title and says each part in order, scripts dropped', () => {
    expect(model.title).toBe('Release slips to Friday');
    expect(model.items.map(i => i.kind)).toEqual([
      'table', 'callout', 'todo', 'todo', 'todo', 'heading', 'paragraph', 'table', 'code', 'bullet', 'bullet', 'toggle',
    ]);
    expect(JSON.stringify(model)).not.toContain('alert(1)');
  });
  it('stats are a table, with their tone in words', () => {
    expect(model.items[0]).toMatchObject({ kind: 'table', rows: [['Metric', 'Value', 'Note'], ['Checks', '41', 'Done: +3'], ['Open bugs', '2', 'Warning']] });
  });
  it('steps are to-dos: done ticked, the current one says so', () => {
    expect(model.items.slice(2, 5)).toEqual([
      { kind: 'todo', text: 'Merge', done: true },
      { kind: 'todo', text: 'Test bed (now) · Running now', done: false },
      { kind: 'todo', text: 'Install', done: false },
    ]);
  });
  it('a chart goes as its data; a flow as Mermaid', () => {
    expect(model.items[7]).toMatchObject({ kind: 'table', rows: [['Item', 'Value (h)'], ['Lane A', '12'], ['Lane B', '30']] });
    expect(model.items[8]).toMatchObject({ kind: 'code', language: 'mermaid', text: 'flowchart LR\n  subgraph g0["Front"]\n    n0["App #quot;web#quot;"]\n  end\n  n1["API"]\n  n0 -->|"calls"| n1' });
  });
  it('the recommended option says so', () => {
    expect(model.items[9]).toEqual({ kind: 'bullet', text: 'Fly (recommended): Closer' });
  });
});

describe('Mermaid and code languages', () => {
  it('shapes for start, decision and note; angle brackets stay text', () => {
    const m = mermaid(parseFlow('s: Go {start}\nd: OK? {decision}\nn: Mind {note}\ns -> d -> n'), 'TD');
    expect(m).toContain('n0(["Go"])');
    expect(m).toContain('n1{"OK?"}');
    expect(m).toContain('n2>"Mind"]');
    expect(mermaid(parseFlow('a: <img src=x>\nb: B\na -> b'), 'TD')).toContain('n0["#lt;img src=x#gt;"]');
  });
  it('code languages map to common names', () => {
    expect(codeLanguage('ts')).toBe('typescript');
    expect(codeLanguage('rs')).toBe('rust');
    expect(codeLanguage('brainfuck')).toBe('plain text');
  });
});
