import { describe, expect, it } from 'vitest';
import { TEXT_BLOCKS_MAX, canvasText } from './canvasText';
import type { El } from './canvasModel';

// A tiny element tree: what the walk reads of a DOM element (as canvasModel.test.ts).
function el(tagName: string, attrs: Record<string, string> = {}, ...children: Array<El | string>): El {
  const kids = children.filter((c): c is El => typeof c !== 'string');
  const text = () => children.map(c => (typeof c === 'string' ? c : c.textContent ?? '')).join('');
  return { tagName: tagName.toUpperCase(), getAttribute: (n) => (n in attrs ? attrs[n] : null), children: kids, get textContent() { return text(); } };
}

// Plan L3 X9 (2026-10-14): Read as text, the canvas's words without its frame.
describe('a canvas read as text', () => {
  it('reads authored checklist rows as distinct items with their marks', () => {
    const checklist = el('hc-checklist', { label: 'Checks' }, '[ ] First\n[x] Second\n[ ] Third\n[ ] Fourth\n[ ] Fifth');
    expect(canvasText(el('hc-canvas', {}, checklist))).toEqual([
      { kind: 'heading', level: 3, text: 'Checks' },
      { kind: 'list', ordered: false, items: ['Not done: First', 'Done: Second', 'Not done: Third', 'Not done: Fourth', 'Not done: Fifth'] },
    ]);
  });
  const canvas = el('hc-canvas', { id: 'release', title: 'Release slips to Friday' },
    el('h2', {}, 'Where it stands'),
    el('p', {}, 'Two checks are open.'),
    el('hc-stats', {},
      el('hc-stat', { label: 'Checks', value: '41', delta: '+3', tone: 'success' }),
      el('hc-stat', { label: 'Open bugs', value: '2' })),
    el('hc-callout', { tone: 'warning', title: 'Heads up' }, 'Install waits for the bed.'),
    el('hc-chart', { type: 'bar', label: 'Hours by lane', unit: 'h' }, 'Lane A: 12\nLane B: 30'),
    el('hc-steps', { current: '2' }, el('div', { label: 'Merge', status: 'done' }), el('div', { label: 'Install' })),
    el('ul', {}, el('li', {}, 'First'), el('li', {}, 'Second')),
    el('hc-flow', { label: 'Path' }, 'plan: Plan\nbuild: Build\nship: Ship\nplan -> build: when ready'),
    el('img', { alt: 'The release board', src: 'x.png' }),
    el('script', {}, 'alert(1)'),
  );
  const blocks = canvasText(canvas);

  it('keeps headings, text, lists and tables in order', () => {
    expect(blocks.map(b => b.kind)).toEqual([
      'heading', 'text', 'table', 'text', 'heading', 'text', 'table', 'list', 'heading', 'list', 'text',
    ]);
    expect(blocks[0]).toEqual({ kind: 'heading', level: 2, text: 'Where it stands' });
    expect(blocks[1]).toEqual({ kind: 'text', text: 'Two checks are open.' });
  });

  it('a chart reads as its data table, never only as a picture', () => {
    expect(blocks[4]).toEqual({ kind: 'heading', level: 3, text: 'Hours by lane' });
    expect(blocks[5]).toEqual({ kind: 'text', text: 'A bar chart; its data:' });
    expect(blocks[6]).toEqual({ kind: 'table', caption: '', head: ['Item', 'Value (h)'], rows: [['Lane A', '12'], ['Lane B', '30']] });
  });

  it('stats are a table; a tone is said in words, not only by colour', () => {
    expect(blocks[2]).toMatchObject({ kind: 'table', head: ['Metric', 'Value', 'Note'], rows: [['Checks', '41', 'Done: +3'], ['Open bugs', '2', '']] });
    expect(blocks[3]).toEqual({ kind: 'text', text: 'Warning: Heads up. Install waits for the bed.' });
  });

  it('steps and plain lists merge into one list each; done is said', () => {
    expect(blocks[7]).toEqual({ kind: 'list', ordered: false, items: ['Done: Merge', 'Not done: Install (now)', 'First', 'Second'] });
  });

  it('a diagram reads as its links in words, an image as its alt text, and scripts as nothing', () => {
    expect(blocks[8]).toEqual({ kind: 'heading', level: 3, text: 'Path' });
    expect(blocks[9]).toEqual({ kind: 'list', ordered: false, items: ['Plan → Build: when ready', 'Ship'] });
    expect(blocks[10]).toEqual({ kind: 'text', text: 'Image: The release board' });
    expect(JSON.stringify(blocks)).not.toContain('alert');
  });

  it('script and style text inside a paragraph is left out (a real DOM node)', () => {
    // The DOM shape `readAsText` passes: child nodes with their kinds.
    const text = (s: string) => ({ nodeType: 3, nodeName: '#text', nodeValue: s });
    const script = { nodeType: 1, nodeName: 'SCRIPT', nodeValue: null, childNodes: [text('steal()')] };
    const p = { ...el('p', {}, 'Visible words. steal()'), childNodes: [text('Visible words. '), script] };
    expect(canvasText(el('hc-canvas', {}, p as El))).toEqual([{ kind: 'text', text: 'Visible words.' }]);
  });

  it('a very long canvas is cut with a line that says so', () => {
    const many = el('hc-canvas', {}, ...Array.from({ length: TEXT_BLOCKS_MAX + 5 }, (_, i) => el('p', {}, `Line ${i}`)));
    const out = canvasText(many);
    expect(out).toHaveLength(TEXT_BLOCKS_MAX + 1);
    expect(out[TEXT_BLOCKS_MAX]).toEqual({ kind: 'text', text: '5 more parts not shown. View source has everything.' });
  });
});
