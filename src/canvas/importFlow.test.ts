import { describe, expect, it } from 'vitest';
import { importMermaid, flowHtml } from './importFlow';
import { parseFlow } from './kit/flow';
import { mermaid } from './canvasModel';
import { chatArtifactKind } from '../chatArtifactKind';
import { fileRefFromCode } from '../chatFileLinks';
import guide from '../../src-tauri/src/canvas_design_prompt.md?raw';

function converted(text: string) {
  const result = importMermaid(text);
  if (!result.ok) throw new Error(result.reason);
  return { result, parsed: parseFlow(result.flow) };
}
function semantic(text: string) {
  const f = parseFlow(text), name = new Map(f.nodes.map(n => [n.id, n.label]));
  return { nodes: f.nodes.map(n => [n.label, n.group, n.shape]).sort(), edges: f.edges.map(e => [name.get(e.from), name.get(e.to), e.label]).sort() };
}
describe('Mermaid flowchart import', () => {
  it('round trips every design-guide flow without importing its tones', () => {
    for (const m of guide.matchAll(/<hc-flow[^>]*>([\s\S]*?)<\/hc-flow>/g)) {
      expect(semantic(converted(mermaid(parseFlow(m[1]), 'LR')).result.flow)).toEqual(semantic(m[1]));
    }
  });
  it.each([
    '[A]\na1\na2\n[B]\nb1\nb2\n[C]\nc1\nc2\n[]\na1 -> b2\nb1 -> c2\nc1 -> a2\na2 -> a1',
    '[Outer]\na\nb\n[Middle]\nc\nd\n[]\na -> c -> d -> b\na -> b: shortcut\nb -> a: return',
    '[A]\na\n[B]\nb\n[C]\nc\n[]\na -> b -> c\nc -> a: retry',
    '[A]\na\nb\n[B]\nc\nd\n[]\na -> a: loop\nb -> b\nc -> c\na -> c -> b -> d',
    'a: Begin {start}\nb: Ready? {decision}\nc: Read this {note}\nd: Done {end}\na -> b -> c -> d: yes',
  ])('round trips exported node labels, shapes, groups and edges', source => {
    expect(semantic(converted(mermaid(parseFlow(source), 'TD')).result.flow)).toEqual(semantic(source));
  });
  it.each(['[box]', '(rounded)', '([stadium])', '[[subroutine]]', '[(database)]', '((circle))', '(((double circle)))', '>note]', '{decision}', '{{hexagon}}', '[/slant/]', '[\\slant\\]', '[/slant\\]', '[\\slant/]'])('reads shape %s', shape => {
    expect(converted(`flowchart TD\na${shape}`).parsed.nodes).toHaveLength(1);
    expect(converted(`flowchart TD\na${shape}`).result.note).not.toContain('unrecognized');
  });
  it('reads modern shapes, literal brace endings, decoded labels and markdown', () => {
    const { parsed } = converted('flowchart TD\na@{ shape: diamond, label: "Pick #quot;this#quot; &amp; #35;" }\nb["`fa:fa-home First<br>line {danger}`"]:::thing\na --> b');
    expect(parsed.nodes.map(n => [n.label, n.shape, n.tone])).toEqual([
      ['Pick "this" & #', 'decision', 'neutral'], ['First line (danger)', 'box', 'neutral'],
    ]);
  });
  it('reads all arrow forms, labels, fanout, chains and both directions', () => {
    const { parsed, result } = converted('graph LR\nA & B --> C & D -->|"next"| E\nE <--> F\nF -- yes --> G\nG -. no .-> H\nH == fast ==> I\nI --o J\nJ --x K\nK -.-> L\nL ----> M\nM e1@--> N\nN --- O\nO ==> P\nP ~~~ Q');
    expect(result.direction).toBe('right');
    expect(parsed.edges).toHaveLength(18);
    expect(parsed.edges.find(e => e.from === 'F' && e.to === 'G')?.label).toBe('yes');
    expect(parsed.edges.find(e => e.from === 'G')?.label).toBe('no');
    expect(parsed.edges.filter(e => e.to === 'E').map(e => e.label)).toEqual(['next', 'next', '']);
    expect(result.note).toContain('1 invisible link');
  });
  it('distinguishes compact arrows from hyphens within identifiers', () => {
    const { parsed } = converted('flowchart LR; step-one-->step-two-.->last; last<-->step-one');
    expect(parsed.nodes.map(n => n.id)).toEqual(['step-one', 'step-two', 'last']);
    expect(parsed.edges).toHaveLength(4);
  });
  it('keeps inner groups and counts styles, handlers, direction and subgraph links', () => {
    const { result, parsed } = converted('flowchart TD\nsubgraph outer [Outside]\na\nsubgraph inner [Inside]\nb\nend\nc\nend\nouter --> b\nclassDef x fill:red\nclass a x\nstyle a fill:blue\nlinkStyle 0 color:red\nclick a "https://x"\ndirection LR');
    expect(parsed.nodes.map(n => [n.label, n.group])).toEqual([['a', 'Outside'], ['b', 'Inside'], ['c', 'Outside']]);
    expect(parsed.edges).toHaveLength(0);
    for (const s of ['1 nested group', '4 style lines', '1 click handler', '1 direction line', '1 subgraph link']) expect(result.note).toContain(s);
  });
  it('reads title, comments, init, semicolons and reverse direction', () => {
    const { result, parsed } = converted('---\ntitle: "A title"\n---\n%%{init: {"theme":"base"}}%%\n%% comment\nflowchart BT; a["semi; colon"] --> b; b --> c');
    expect(result.title).toBe('A title'); expect(result.note).toContain('drawn top-down');
    expect(parsed.nodes[0].label).toBe('semi; colon'); expect(parsed.edges).toHaveLength(2);
  });
  it('bounds input and makes the kit limits visible', () => {
    expect(converted(`flowchart TD\n${Array.from({ length: 61 }, (_, i) => `n${i}`).join('\n')}`).result.note).toContain('1 node not shown');
    expect(importMermaid('a'.repeat(65537)).ok).toBe(false);
    expect(importMermaid('flowchart TD\n' + 'a\n'.repeat(2000)).ok).toBe(false);
    expect(importMermaid('flowchart TD\n' + 'a\n'.repeat(1999)).ok).toBe(true);
    expect(importMermaid('flowchart TD').ok).toBe(false);
  });
  it('defaults direction, assigns late groups and retains later valid lines', () => {
    const { result, parsed } = converted('flowchart\nA-->B\nsubgraph S\nA\nB\nend\nA[unclosed\nC-->D');
    expect(result.direction).toBe('down');
    expect(parsed.nodes.slice(0, 2).map(n => n.group)).toEqual(['S', 'S']);
    expect(parsed.edges.some(e => e.from === 'C' && e.to === 'D')).toBe(true);
    expect(result.note).toContain('1 unrecognized statement');
  });
  it('strips actual icon prefixes and markdown styling without eating words', () => {
    const { parsed } = converted('graph\na["sofa:bed fa:fa-home"]\nb["`**Bold** and _italic_`"]');
    expect(parsed.nodes.map(n => n.label)).toEqual(['sofa:bed', 'Bold and italic']);
    expect(converted('flowchart TD\na["`snake_case_id`"]\nb["`line1\nline2`"]') .parsed.nodes.map(n => n.label)).toEqual(['snake_case_id', 'line1 line2']);
  });
  it('keeps the innermost group and the first sibling membership', () => {
    const { parsed } = converted('graph TD\nsubgraph A\nsubgraph Inner\nx\nend\nx\nend\nsubgraph B\nx\nend');
    expect(parsed.nodes[0].group).toBe('Inner');
    expect(importMermaid('flowchart-elk\na-->b').ok).toBe(false);
  });
  it('bounds expansion work and counts omitted fan-out arithmetically', () => {
    const side = (prefix: string) => Array.from({ length: 5000 }, (_, i) => `${prefix}${i}`).join('&');
    const { result } = converted(`flowchart TD\n${side('a')} --> ${side('b')}`);
    expect(result.counts.nodes).toBe(60);
    expect(result.counts.links + result.counts.droppedLinks).toBe(25_000_000);
    expect(result.flow.split('\n').length).toBeLessThan(14_010);
  });
  it.each(['sequenceDiagram', 'classDiagram', 'stateDiagram-v2', 'erDiagram', 'gantt', 'pie', 'journey', 'mindmap', 'gitGraph'])('leaves %s as source', kind => {
    expect(importMermaid(kind)).toMatchObject({ ok: false, reason: 'Hollow draws Mermaid flowcharts.' });
  });
  it('escapes hostile content and prevents line/attribute injection', () => {
    const { result } = converted('---\ntitle: \'"><script>bad()</script>\'\n---\nflowchart TD\na["</hc-flow><script>bad()</script>"] --> b["safe"]');
    const html = flowHtml(result);
    expect(html).not.toContain('<script>'); expect(html).toContain('&lt;/hc-flow&gt;'); expect(html).toContain('&quot;');
  });
  it('sanitizes colliding Unicode identifiers without merging them', () => {
    const { parsed } = converted('flowchart TD\né --> è');
    expect(new Set(parsed.nodes.map(n => n.id)).size).toBe(2);
    expect(parsed.edges[0].from).not.toBe(parsed.edges[0].to);
  });
  it('recognizes diagram files without changing video files', () => {
    expect(chatArtifactKind('hello.mmd')).toBe('diagram'); expect(chatArtifactKind('hello.MERMAID')).toBe('diagram');
    expect(chatArtifactKind('hello.mp4')).toBe('video');
    expect(fileRefFromCode('hello.mmd')).toBe('hello.mmd');
    expect(fileRefFromCode('hello.mermaid')).toBe('hello.mermaid');
  });
});
