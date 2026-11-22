import { describe, expect, it } from 'vitest';
import GUIDE from '../../src-tauri/src/canvas_design_prompt.md?raw';
import { parseRows, toChartData } from './kit/data';
import { parseFlow } from './kit/flow';
import { KIT_COMPONENTS } from './kit/manifest';

// 10-11 B2: the visual guide's exemplars are what agents copy, so they must
// be canvases the kit draws as intended: known elements, chart rows that are
// numbers, a flow with every node linked.
const inner = (tag: string) => {
  const m = new RegExp(`<${tag}[^>]*>([\\s\\S]*?)</${tag}>`).exec(GUIDE);
  if (!m) throw new Error(`no ${tag}`);
  return m[1];
};

describe('the design guide’s exemplars', () => {
  it('use only kit elements', () => {
    const named = new Set([...GUIDE.matchAll(/<(hc-[a-z-]+)/g)].map(m => m[1]));
    named.delete('hc-canvas');
    const known = new Set<string>(KIT_COMPONENTS);
    for (const n of named) expect(known.has(n), n).toBe(true);
  });
  it('chart rows are numbers', () => {
    const data = toChartData(parseRows(inner('hc-chart')));
    expect(data.categories).toEqual(['Mon', 'Tue', 'Wed']);
    expect(data.series[0].values).toEqual([980, 1140, 1284]);
  });
  it('the flow links every node', () => {
    const flow = parseFlow(inner('hc-flow'));
    expect(flow.nodes.map(n => n.label)).toEqual(['App', 'API', 'Postgres']);
    expect(flow.edges).toHaveLength(2);
    expect(flow.nodes.find(n => n.id === 'api')?.tone).toBe('success');
  });
  it('stat values stay short, as the guide says', () => {
    for (const m of GUIDE.matchAll(/<hc-stat [^>]*value="([^"]*)"/g)) expect(m[1].length, m[1]).toBeLessThan(10);
  });
});
