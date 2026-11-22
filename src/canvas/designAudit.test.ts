import { describe, expect, it } from 'vitest';
import { DESIGN_SHOWN, canvasFindingsRequest, designFindings, designRequestLines, measuredFindings, staticDesignFindings } from './designAudit';
import { noteCanvasProblems, problemsLabel, runCanvasProblems } from './canvasProblems';
import type { DesignItem } from '../canvasBridge';

const canvas = (inner: string) => `<hc-canvas id="d" title="Design">${inner}</hc-canvas>`;
const rules = (source: string) => staticDesignFindings(source).map(f => `${f.rule}:${f.tier}`);

describe('design check: the static pass over the source', () => {
  it('leaves a kit-only canvas, tokens and keywords alone', () => {
    expect(staticDesignFindings(canvas('<hc-stats><hc-stat label="Runs" value="12"></hc-stat></hc-stats><hc-chart type="donut">A: 1</hc-chart>'))).toEqual([]);
    const tokens = canvas('<style>.a { color: var(--app-ink); background-color: #fff; border-color: red } .b{color:inherit} .c{color:currentColor !important} .d{color: color-mix(in srgb, var(--app-muted) 70%, transparent)}</style><p class="a" style="color: var(--app-muted); font-size: 13px">Ok</p>');
    expect(staticDesignFindings(tokens)).toEqual([]);
  });
  it('flags literal text colours, in style blocks and attributes, never background-color', () => {
    const found = staticDesignFindings(canvas('<style>.a{color:#666} .b { color: rgb(20 20 20) }</style><p style="color: white">x</p><p style="background-color:#000">y</p>'));
    expect(found).toHaveLength(1);
    expect(found[0]).toMatchObject({ rule: 'literal-color', tier: 'fail' });
    expect(found[0].text).toBe('3 text colours are written as literals (#666, rgb(20 20 20), white) instead of --app-* tokens.');
    expect(found[0].fix).toContain('--app-ink');
  });
  it('flags px and pt sizes under 11px, the font shorthand too, but leaves em and rem to the frame', () => {
    expect(rules(canvas('<p style="font-size: 10px">a</p>'))).toEqual(['small-text:fail']);
    expect(rules(canvas('<style>.t{font: 600 9.5px/1.2 var(--app-mono)}</style>'))).toEqual(['small-text:fail']);
    expect(staticDesignFindings(canvas('<p style="font-size:7pt">a</p>'))[0].text).toContain('smallest 9.3px');
    expect(rules(canvas('<p style="font-size: 11px">a</p><p style="font-size:.6rem">b</p><p style="font-size: 9pt">c</p>'))).toEqual([]);
  });
  it('flags transitions of every property, named or implied', () => {
    expect(rules(canvas('<style>.a{transition: all .2s ease}</style>'))).toEqual(['transition-all:advisory']);
    expect(rules(canvas('<style>.a{transition: 200ms}</style>'))).toEqual(['transition-all:advisory']);
    expect(rules(canvas('<style>.a{transition-property: opacity, all}</style>'))).toEqual(['transition-all:advisory']);
    expect(rules(canvas('<style>.a{transition: opacity 120ms ease-out, transform 120ms cubic-bezier(.2, 0, 0, 1)} .b{transition: none} .c{transition: inherit}</style>'))).toEqual([]);
  });
  it('flags motion of the canvas\'s own: @keyframes and requestAnimationFrame', () => {
    const found = staticDesignFindings(canvas('<style>@keyframes spin{to{transform:rotate(1turn)}}</style><script>function f(){requestAnimationFrame(f)} requestAnimationFrame (f);</script>'));
    expect(found.map(f => f.rule)).toEqual(['motion']);
    expect(found[0].text).toBe('1 hand-written @keyframes rule and 2 requestAnimationFrame calls: motion of its own.');
    // A word in visible text is not code.
    expect(rules(canvas('<p>requestAnimationFrame() and @keyframes, explained</p>'))).toEqual([]);
  });
  it('flags emoji used as icons, in text and attributes, but not text marks', () => {
    const found = staticDesignFindings(canvas('<hc-stat label="🚀 Launches" value="3"></hc-stat><p>✅ Done · ⚠️ Late</p>'));
    expect(found.map(f => f.rule)).toEqual(['emoji-icon']);
    expect(found[0].text).toBe('3 emoji are used as icons (🚀 ✅ ⚠️).');
    expect(rules(canvas('<p>✓ done → next © 2026 ★</p>'))).toEqual([]);
    // Emoji inside a script or a comment is not on the page.
    expect(rules(canvas('<script>const s = "🚀";</script><!-- 🎉 -->'))).toEqual([]);
  });
  it('flags inline SVG with no label unless it is hidden or titled', () => {
    expect(rules(canvas('<svg viewBox="0 0 10 10"><circle r="4"/></svg>'))).toEqual(['svg-label:advisory']);
    for (const ok of ['<svg aria-hidden="true"><path/></svg>', '<svg role="img" aria-label="Trend up"><path/></svg>', '<svg role="presentation"></svg>', '<svg aria-labelledby="t"><path/></svg>', '<svg><title>Trend</title><path/></svg>']) {
      expect(rules(canvas(ok)), ok).toEqual([]);
    }
    expect(rules(canvas('<svg aria-label=" "><path/></svg>'))).toEqual(['svg-label:advisory']);
  });
  it('is deterministic and never reads the kit: comments and attribute values are not CSS', () => {
    const source = canvas('<!-- <style>.x{color:#000}</style> --><p data-note="color: red">x</p>');
    expect(staticDesignFindings(source)).toEqual([]);
    const busy = canvas('<style>.a{color:#111;transition:all 1s}@keyframes k{}</style><p style="font-size:9px">🎉</p><svg></svg>');
    expect(staticDesignFindings(busy)).toEqual(staticDesignFindings(busy));
    expect(rules(busy)).toEqual(['literal-color:fail', 'small-text:fail', 'transition-all:advisory', 'motion:advisory', 'emoji-icon:advisory', 'svg-label:advisory']);
  });
});

describe('design check: the measured report and the row', () => {
  const contrast = (ratio: number, need: 3 | 4.5 = 4.5, text = 'Revenue'): DesignItem => ({ rule: 'contrast', text, ratio, need });
  it('turns measured items into findings: near misses are advisory, texture is unresolved', () => {
    const found = measuredFindings([contrast(4.2), contrast(2.1, 3), { rule: 'small-text', text: 'fine print', size: 9.25 }, { rule: 'texture-text', text: 'Over the dots', need: 4.5, box: [0, 0, 100, 20] }]);
    expect(found.map(f => `${f.rule}:${f.tier}`)).toEqual(['contrast:advisory', 'contrast:fail', 'small-text:fail', 'texture-text:unresolved']);
    expect(found[0].text).toBe('Text "Revenue" has contrast 4.2:1 against its ground (needs 4.5:1).');
    expect(found[2].text).toBe('Text "fine print" renders at 9.3px (under 11px).');
    expect(found[3].text).toContain('unresolved');
    expect(measuredFindings([contrast(3, 4.5, 'x'.repeat(80))])[0].text).toContain(`"${'x'.repeat(39)}…"`);
  });
  it('orders fail, unresolved, advisory; measured sizes replace the source guess; the total counts past the cap', () => {
    const source = canvas('<style>.a{transition:all 1s}</style><p style="font-size:9px">x</p>');
    const plain = designFindings(source, null);
    expect(plain.list.map(f => f.rule)).toEqual(['small-text', 'transition-all']);
    expect(plain.total).toBe(2);
    const measured = designFindings(source, { items: [{ rule: 'texture-text', text: 'a', need: 4.5, box: [0, 0, 10, 10] }, { rule: 'small-text', text: 'x', size: 9 }, contrast(4.3)], more: 7, complete: true });
    expect(measured.list.map(f => `${f.rule}:${f.tier}`)).toEqual(['small-text:fail', 'texture-text:unresolved', 'transition-all:advisory', 'contrast:advisory']);
    expect(measured.list.filter(f => f.rule === 'small-text')).toHaveLength(1);
    expect(measured.total).toBe(4 + 7);
    // Rendered text that measured fine overrules the declaration.
    expect(designFindings(source, { items: [], more: 0, complete: true }).list.map(f => f.rule)).toEqual(['transition-all']);
    // An incomplete pass (a cap reached, text mid-animation, an error) can't
    // clear it: the source's warning stays, marked unverified.
    const partial = designFindings(source, { items: [], more: 0, complete: false });
    expect(partial.list.map(f => `${f.rule}:${f.tier}`)).toEqual(['small-text:unresolved', 'transition-all:advisory']);
    expect(partial.list[0].text).toBe('1 font size is under 11px (smallest 9px). Not every text was measured, so this is unverified.');
    const both = designFindings(source, { items: [{ rule: 'small-text', text: 'x', size: 9 }], more: 0, complete: false });
    expect(both.list.map(f => `${f.rule}:${f.tier}`)).toEqual(['small-text:fail', 'small-text:unresolved', 'transition-all:advisory']);
  });
  it('ideas 10-27 #4: an unresolved text measured on the picture is settled, or says why it stays', () => {
    const box = (text: string) => ({ rule: 'texture-text' as const, text, need: 4.5 as const, box: [0, 0, 100, 20] as [number, number, number, number] });
    const items = [box('Passes'), box('Fails'), box('Near'), box('Tiny'), box('Lost'), box('Not yet'), { ...box('Mixed'), box: null }, box('Huge')];
    const pictures = new Map([
      [0, { kind: 'measured' as const, ratio: 7.1, need: 4.5 as const, pixels: 300 }],
      [1, { kind: 'measured' as const, ratio: 1.6, need: 4.5 as const, pixels: 300 }],
      [2, { kind: 'measured' as const, ratio: 4.2, need: 4.5 as const, pixels: 300 }],
      [3, { kind: 'unresolved' as const, why: 'few-pixels' as const }],
      [4, { kind: 'unresolved' as const, why: 'outside' as const }],
      [7, { kind: 'unresolved' as const, why: 'too-large' as const }],
    ]);
    const found = measuredFindings(items, pictures);
    expect(found.map(f => `${f.rule}:${f.tier}`)).toEqual(['contrast:fail', 'contrast:advisory', 'texture-text:unresolved', 'texture-text:unresolved', 'texture-text:unresolved', 'texture-text:unresolved', 'texture-text:unresolved']);
    expect(found[0].text).toBe('Text "Fails" measures 1.6:1 on the picture, against what lies under it (needs 4.5:1).');
    expect(found[2].text).toContain('Too little of it shows');
    expect(found[3].text).toContain("couldn't be found on the picture");
    expect(found[4].text).toBe('Text "Not yet" sits on a gradient, image or texture; its contrast is unresolved.');
    expect(found[5].text).toContain('other text or a field shares its box');
    expect(found[6].text).toContain('too large to measure');
    // The row's total follows: the passing text is gone.
    expect(designFindings(canvas('<p>x</p>'), { items, more: 0, complete: true, pictures }).total).toBe(7);
  });

  it('ideas 10-27 #4 review: counted findings stay in the draft when none is listed', () => {
    const text = canvasFindingsRequest({ id: 'c', title: 'C' }, [{ label: 'Flow', findings: [{ text: 'Crossed lines' }] }], { list: [], total: 3 });
    expect(text).toContain('In the hand-written HTML and CSS:');
    expect(text).toContain('And 3 more design problems.');
  });

  it('drafts at most five findings plus the count of the rest, bounded and plain', () => {
    const list = measuredFindings(Array.from({ length: 8 }, (_, i) => contrast(2, 4.5, `<b>${i}</b>`)));
    const lines = designRequestLines(list, 10);
    expect(lines.split('\n')).toHaveLength(DESIGN_SHOWN + 1);
    expect(lines).toContain('And 5 more design problems.');
    expect(lines).not.toMatch(/[<>]/);
    const text = canvasFindingsRequest({ id: 'release', title: 'Release <plan>' }, [{ label: 'Flow', findings: Array.from({ length: 30 }, () => ({ text: 'x '.repeat(200) })) }], { list, total: 10 });
    expect(text.length).toBeLessThanOrEqual(2000);
    expect(text).toContain('(id release)');
    expect(text).toMatch(/same id\. Layout: .* Design: /);
    expect(text).not.toMatch(/[<>]/);
    // Many long layout findings never push the design findings out.
    expect(text).toContain('In the hand-written HTML and CSS:\n1. Fails:');
    const designOnly = canvasFindingsRequest({ id: 'r', title: 'R' }, [], { list, total: 10 });
    expect(designOnly).toContain('And 5 more design problems.');
  });
  it('says the row\'s count and keeps it per run for the receipt', () => {
    expect(problemsLabel(0, 0)).toBeNull();
    expect(problemsLabel(1, 0)).toBe('1 layout problem');
    expect(problemsLabel(0, 3)).toBe('3 design problems');
    expect(problemsLabel(2, 3)).toBe('5 layout and design problems');
    expect(problemsLabel(20, 1, true)).toBe('21+ layout and design problems');
    noteCanvasProblems('run-a', 'one', { layout: 1, design: 2 });
    noteCanvasProblems('run-a', 'two', { layout: 0, design: 4 });
    expect(runCanvasProblems('run-a')).toEqual({ layout: 1, design: 6, more: false });
    noteCanvasProblems('run-a', 'one', null);
    noteCanvasProblems('run-a', 'two', { layout: 0, design: 0 });
    expect(runCanvasProblems('run-a')).toBeNull();
    expect(runCanvasProblems(undefined)).toBeNull();
    // A row that unmounts after its replacement mounted forgets only its own.
    const first = {}, second = {};
    noteCanvasProblems('run-b', 'c', { layout: 1, design: 0 }, first);
    noteCanvasProblems('run-b', 'c', { layout: 0, design: 2 }, second);
    noteCanvasProblems('run-b', 'c', null, first);
    expect(runCanvasProblems('run-b')).toEqual({ layout: 0, design: 2, more: false });
    noteCanvasProblems('run-b', 'c', null, second);
    expect(runCanvasProblems('run-b')).toBeNull();
    for (let i = 0; i < 205; i++) noteCanvasProblems(`run-${i}`, 'c', { layout: 1, design: 0 });
    expect(runCanvasProblems('run-0')).toBeNull();
    expect(runCanvasProblems('run-204')).toEqual({ layout: 1, design: 0, more: false });
  });
});
