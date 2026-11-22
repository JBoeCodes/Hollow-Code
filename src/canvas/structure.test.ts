import { describe, expect, it } from 'vitest';
import { innerElementCounts } from './fence';
import { canvasStructure, structureOfInner, structureSummary } from './structure';
import { KIT_COMPONENTS } from './kit/manifest';

const canvas = (inner: string, attrs = 'id="release" title="Release"') => `<hc-canvas ${attrs}>\n${inner}\n</hc-canvas>\n`;

describe('a saved canvas described for its Results card (Canvas C5b)', () => {
  it('counts kit parts in display order: "3 stats · 1 chart · 1 table"', () => {
    const body = canvas(`
      <hc-stats><hc-stat label="Bed" value="44"></hc-stat><hc-stat label="Fail" value="0"></hc-stat><hc-stat label="Skip" value="13"></hc-stat></hc-stats>
      <hc-table>Name|Pass\nC2|ok</hc-table>
      <hc-chart type="line">Day|Runs\nMon|3</hc-chart>`);
    const s = canvasStructure(body)!;
    expect(structureSummary(s)).toBe('3 stats · 1 chart · 1 table');
    // hc-stats is a container: counted as kit, never named as a part.
    expect(s.kitElements).toBe(6);
    expect(s.scripted).toBe(false);
  });

  it('names every kit@1 display component except the stats container', () => {
    const inner = KIT_COMPONENTS.map(tag => `<${tag}></${tag}>`).join('');
    const s = structureOfInner(inner);
    expect(s.kitElements).toBe(KIT_COMPONENTS.length);
    expect(s.parts.map(p => p.label)).toEqual([
      '1 stat', '1 chart', '1 diagram', '1 bar list', '1 table', '1 progress bar', '1 timeline', '1 comparison', '1 callout',
      '1 tab set', '1 fold', '1 code block', '1 file link', '1 badge',
    ]);
    expect(structureSummary(s)).toBe('1 stat · 1 chart · 1 diagram · 1 bar list · +10 more');
    expect(structureSummary(s, 20).split(' · ')).toHaveLength(14);
  });

  it('counts plain HTML tables, drawings and images; unknown hc- tags are not kit', () => {
    const s = structureOfInner('<table></table><TABLE></TABLE><svg><rect/></svg><canvas></canvas><img src="data:,"><hc-ask></hc-ask><p>text</p>');
    expect(s.parts.map(p => p.label)).toEqual(['2 tables', '2 drawings', '1 image']);
    expect(s.unknownKit).toBe(1);
    expect(s.kitElements).toBe(0);
  });

  it('reads tags the way the fence parser does: comments, strings and raw text are not parts', () => {
    const inner = [
      '<!-- <hc-chart></hc-chart> -->',
      '<script>const s = "<hc-chart>"; document.body.append("<hc-table>")</script>',
      '<style>hc-stat::before { content: "<hc-stat>" }</style>',
      '<textarea><hc-callout></textarea>',
      '<div title="<hc-badge>" data-x=\'<hc-steps>\'></div>',
      '</hc-chart><!doctype html><?php ?>',
      '<hc-progress value="3" max="5"></hc-progress>',
    ].join('\n');
    const s = structureOfInner(inner);
    expect(s.parts.map(p => p.label)).toEqual(['1 progress bar']);
    expect(s.scripted).toBe(true);
    const counts = innerElementCounts(inner);
    expect([...counts.keys()].sort()).toEqual(['div', 'hc-progress', 'script', 'style', 'textarea']);
  });

  it('stops at a cut-off tag or plaintext instead of guessing', () => {
    expect(structureOfInner('<hc-stat></hc-stat><hc-chart type="li').parts.map(p => p.label)).toEqual(['1 stat']);
    expect(structureOfInner('<plaintext><hc-chart></hc-chart>').parts).toEqual([]);
    expect(structureOfInner('<script>never closed <hc-chart>').scripted).toBe(true);
    expect(structureOfInner('a < b and <3').parts).toEqual([]);
  });

  it('a canvas with no named parts reads as text and layout; a broken body is null', () => {
    expect(structureSummary(canvasStructure(canvas('<h2>Plain</h2><p>Only words.</p>'))!)).toBe('Text and layout');
    expect(canvasStructure('<div>no root</div>')).toBeNull();
    expect(canvasStructure(canvas('<hc-canvas id="x" title="Nested"></hc-canvas>'))).toBeNull();
    expect(canvasStructure(canvas('', 'title="No id"'))).toBeNull();
  });
});
