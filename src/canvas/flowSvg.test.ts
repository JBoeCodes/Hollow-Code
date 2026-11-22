import { describe, expect, it } from 'vitest';
import { flowColors, flowSvg } from './flowSvg';
import { layoutFlow, parseFlow } from './kit/flow';
import { edgePath } from './kit/flowPath';
import golden from './flowSvg.golden.svg?raw';

describe('static flow SVG', () => {
  const flow = { label: 'Plan & build', text: '[Team]\na: Plan {start, info}\nb: Ready {decision, success}\na -> b: Go\nb -> a: Retry', direction: 'down' as const };
  it('pins a host-produced SVG also accepted by the Rust writer', () => {
    const svg = flowSvg(flow, 800, flowColors('dark')).svg;
    // The text fixture ends in LF; the export API returns the SVG bytes only.
    expect(`${svg}\n`).toBe(golden);
  });
  it('is deterministic, keeps every label and uses the kit edge geometry', () => {
    const colors = flowColors('light');
    const first = flowSvg(flow, 800, colors);
    expect(flowSvg(flow, 800, colors)).toEqual(first);
    for (const text of ['Plan &amp; build', 'Plan', 'Ready', 'Team', 'Go', 'Retry']) expect(first.svg).toContain(text);
    for (const edge of layoutFlow(parseFlow(flow.text), 'down').edges) {
      if (!edge.route || edge.route.length) expect(first.svg).toContain(`d="${edgePath(edge, false)}"`);
    }
    expect(first.svg).not.toMatch(/<script|foreignObject|href=|var\(/i);
    expect(first.svg).toContain('<style>');
  });
  it('escapes hostile labels and refuses stylesheet injection', () => {
    const svg = flowSvg({ ...flow, label: '</title><script>alert(1)</script>', text: 'a: <img onerror="bad"> & stuff' }, 600, flowColors('dark')).svg;
    expect(svg).not.toContain('<script');
    expect(svg).toContain('&lt;img onerror=&quot;bad&quot;&gt; &amp; stuff');
    expect(() => flowSvg(flow, 800, { ...flowColors('light'), ink: 'red} @import url(https://bad)' })).toThrow();
  });
  it('uses stock theme tokens and safe supplied colours', () => {
    expect(flowColors('light').canvas).toBe('#ffffff');
    expect(flowColors('dark').canvas).toBe('#1a1a1a');
    expect(flowColors('dark', [['--app-primary', '#123456']]).primary).toBe('#123456');
    expect(flowColors('light', [['--app-ink', 'url(https://bad)']]).ink).toBe('#1a1a1a');
  });
  it('keeps forty nodes, reports truncation and rejects empty flows', () => {
    const text = Array.from({ length: 40 }, (_, i) => `n${i}: Step ${i}`).join('\n');
    const result = flowSvg({ ...flow, text }, 1280, flowColors('light'));
    for (let i = 0; i < 40; i++) expect(result.svg).toContain(`Step ${i}`);
    expect(result.warning).toBeNull();
    expect(flowSvg({ ...flow, text: Array.from({ length: 61 }, (_, i) => `n${i}: Step ${i}`).join('\n') }, 800, flowColors('light')).warning).toContain('1 nodes');
    expect(() => flowSvg({ ...flow, text: '' }, 800, flowColors('light'))).toThrow('no nodes');
  });
});
