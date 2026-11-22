import { describe, it, expect } from 'vitest';
import { layoutFlow, MAX_EDGES, MAX_NODES, parseFlow, wrapLabel } from './kit/flow';

const DEPLOY = `
start: Change merged {start}
ci: CI green? {decision}
ship: Deploy {success}
fix: Fix failures {danger}
start -> ci
ci -> ship: yes
ci -> fix: no
fix -> ci
`;

describe('hc-flow text', () => {
  it('reads nodes with shapes and tones, and labelled links', () => {
    const f = parseFlow(DEPLOY);
    expect(f.nodes.map(n => [n.id, n.label, n.shape, n.tone])).toEqual([
      ['start', 'Change merged', 'start', 'neutral'],
      ['ci', 'CI green?', 'decision', 'neutral'],
      ['ship', 'Deploy', 'box', 'success'],
      ['fix', 'Fix failures', 'box', 'danger'],
    ]);
    expect(f.edges).toEqual([
      { from: 'start', to: 'ci', label: '' },
      { from: 'ci', to: 'ship', label: 'yes' },
      { from: 'ci', to: 'fix', label: 'no' },
      { from: 'fix', to: 'ci', label: '' },
    ]);
  });

  it('makes nodes for unknown ends, matches ends by label, and splits chains', () => {
    const f = parseFlow('a: Chat view\nChat view -> api.ts -> Command: invoke');
    expect(f.nodes.map(n => n.id)).toEqual(['a', 'api.ts', 'Command']);
    expect(f.edges).toEqual([
      { from: 'a', to: 'api.ts', label: '' },
      { from: 'api.ts', to: 'Command', label: 'invoke' },
    ]);
  });

  it('puts nodes in the group named above them until the next heading', () => {
    const f = parseFlow('[Frontend]\nview: Chat view\napi: api.ts\n[Backend]\ncmd: Command\n[]\nnote: Loose {note}\n[Empty]');
    expect(f.nodes.map(n => [n.id, n.group, n.shape])).toEqual([
      ['view', 'Frontend', 'box'], ['api', 'Frontend', 'box'], ['cmd', 'Backend', 'box'], ['note', '', 'note'],
    ]);
    expect(f.groups).toEqual(['Frontend', 'Backend']);
  });

  it('a later line can relabel a node; neutral is a word, an unknown word keeps the braces in the label', () => {
    expect(parseFlow('a -> b\na: First {neutral, warning}').nodes[0]).toMatchObject({ id: 'a', label: 'First', shape: 'box', tone: 'warning' });
    expect(parseFlow('a: First {sparkly, warning}').nodes[0]).toMatchObject({ label: 'First {sparkly, warning}', tone: 'neutral' });
  });

  it('is bounded: nodes and links past the limits are dropped and counted', () => {
    const many = Array.from({ length: MAX_NODES + 5 }, (_, i) => `n${i}`).join('\n');
    expect(parseFlow(many)).toMatchObject({ droppedNodes: 5, droppedLinks: 0 });
    expect(parseFlow(many).nodes).toHaveLength(MAX_NODES);
    const pairs: string[] = [];
    for (let a = 0; a < 20 && pairs.length < MAX_EDGES + 3; a++) for (let b = 0; b < 20 && pairs.length < MAX_EDGES + 3; b++) if (a !== b) pairs.push(`n${a} -> n${b}`);
    const f = parseFlow(pairs.join('\n'));
    expect(f.edges).toHaveLength(MAX_EDGES);
    expect(f.droppedLinks).toBe(3);
  });
});

describe('hc-flow layout', () => {
  const at = (l: ReturnType<typeof layoutFlow>, id: string) => l.nodes.find(n => n.id === id)!;

  it('puts each node one layer past its parent, and survives a cycle', () => {
    const l = layoutFlow(parseFlow(DEPLOY));
    expect(['start', 'ci', 'ship', 'fix'].map(id => at(l, id).layer)).toEqual([0, 1, 2, 2]);
    expect(at(l, 'ci').y).toBeGreaterThan(at(l, 'start').y);
    expect(at(l, 'ship').y).toBe(at(l, 'fix').y);
    expect(l.edges.find(e => e.from === 'fix' && e.to === 'ci')!.back).toBe(true);
    expect(l.edges.filter(e => e.back)).toHaveLength(1);
  });

  it('never overlaps two nodes, and keeps everything inside the drawing', () => {
    const text = Array.from({ length: 12 }, (_, i) => `root -> leaf${i}: link ${i}`).join('\n') + '\nleaf3 -> leaf7\nleaf7 -> root';
    for (const dir of ['down', 'right'] as const) {
      const l = layoutFlow(parseFlow(text), dir);
      for (const a of l.nodes) {
        expect(a.x).toBeGreaterThanOrEqual(0);
        expect(a.y).toBeGreaterThanOrEqual(0);
        expect(a.x + a.w).toBeLessThanOrEqual(l.width);
        expect(a.y + a.h).toBeLessThanOrEqual(l.height);
        for (const b of l.nodes) {
          if (a === b) continue;
          const apart = a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
          expect(apart, `${a.id} overlaps ${b.id} (${dir})`).toBe(true);
        }
      }
    }
  });

  it('runs across the page when the direction is right', () => {
    const l = layoutFlow(parseFlow('a -> b -> c'), 'right');
    expect(at(l, 'b').x).toBeGreaterThan(at(l, 'a').x);
    expect(at(l, 'c').x).toBeGreaterThan(at(l, 'b').x);
    expect(at(l, 'a').y).toBe(at(l, 'c').y);
    expect(l.width).toBeGreaterThan(l.height);
  });

  it('keeps a group together, inside one box around its members', () => {
    const text = '[Frontend]\nview\napi\n[Backend]\ncmd\ndb\n[]\nhub -> view\nhub -> cmd\nhub -> api\nhub -> db';
    const l = layoutFlow(parseFlow(text));
    const xs = ['view', 'api', 'cmd', 'db'].map(id => at(l, id).x);
    const order = ['view', 'api', 'cmd', 'db'].sort((a, b) => at(l, a).x - at(l, b).x);
    expect([order.indexOf('view'), order.indexOf('api')].sort().join()).toMatch(/^(0,1|2,3)$/);
    expect(xs).toHaveLength(4);
    for (const g of l.groups) {
      for (const n of l.nodes.filter(n => n.group === g.name)) {
        expect(n.x).toBeGreaterThan(g.x);
        expect(n.y).toBeGreaterThan(g.y);
        expect(n.x + n.w).toBeLessThan(g.x + g.w);
        expect(n.y + n.h).toBeLessThan(g.y + g.h);
      }
    }
  });

  it('is deterministic, and draws nothing broken for an empty or one-node flow', () => {
    expect(layoutFlow(parseFlow(DEPLOY))).toEqual(layoutFlow(parseFlow(DEPLOY)));
    expect(layoutFlow(parseFlow(''))).toMatchObject({ nodes: [], edges: [] });
    const one = layoutFlow(parseFlow('only: Just me'));
    expect(one.nodes).toHaveLength(1);
    expect(one.width).toBeGreaterThan(0);
  });

  it('wraps a long label into at most three lines', () => {
    expect(wrapLabel('Short')).toEqual(['Short']);
    const long = wrapLabel('A very long label that keeps on going well past three lines of text for sure');
    expect(long).toHaveLength(3);
    expect(long[2].endsWith('…')).toBe(true);
    for (const line of long) expect(line.length).toBeLessThanOrEqual(24);
  });
});

// Review of the first cut (2026-10-07): each case below failed before its fix.
describe('hc-flow review fixes', () => {
  const boxOf = (l: ReturnType<typeof layoutFlow>, name: string) => l.groups.find(g => g.name === name)!;
  const inside = (n: { x: number; y: number; w: number; h: number }, g: { x: number; y: number; w: number; h: number }) =>
    n.x < g.x + g.w && g.x < n.x + n.w && n.y < g.y + g.h && g.y < n.y + n.h;

  it('a group box holds only its members, and two group boxes never overlap, in either direction', () => {
    const cases = [
      '[A]\na1\na2\n[B]\nb1\nb2\n[]\nb1 -> a2\na1 -> b2',
      '[G]\na\nb\n[]\na -> c\nc -> b',
      '[A]\na\n[B]\nb\n[]\nhub -> a\nhub -> b',
    ];
    for (const text of cases) {
      for (const dir of ['down', 'right'] as const) {
        const l = layoutFlow(parseFlow(text), dir);
        for (const g of l.groups) {
          for (const n of l.nodes) {
            if (n.group !== g.name) expect(inside(n, g), `${n.id} inside ${g.name} (${dir}): ${text}`).toBe(false);
          }
          for (const o of l.groups) if (o !== g) expect(inside(o, g), `${o.name} overlaps ${g.name} (${dir})`).toBe(false);
        }
      }
    }
    expect(boxOf(layoutFlow(parseFlow(cases[0])), 'A')).toBeTruthy();
  });

  it('link labels fit inside the drawing, including on a link back', () => {
    for (const text of ['a -> b: this is a long label', 'a -> b\nb -> a: try again please']) {
      for (const dir of ['down', 'right'] as const) {
        const l = layoutFlow(parseFlow(text), dir);
        for (const e of l.edges.filter(e => e.label)) {
          expect(e.lx - e.lw / 2, text).toBeGreaterThanOrEqual(0);
          expect(e.lx + e.lw / 2, text).toBeLessThanOrEqual(l.width);
          expect(e.ly - 9).toBeGreaterThanOrEqual(0);
          expect(e.ly + 9).toBeLessThanOrEqual(l.height);
        }
      }
    }
  });

  it('merges repeated links into one, joining their labels', () => {
    expect(parseFlow('a -> b: yes\na -> b: no\na -> b').edges).toEqual([{ from: 'a', to: 'b', label: 'yes / no' }]);
  });

  it('keeps braces that are not shape or tone words in the label', () => {
    expect(parseFlow('api: GET /users/{id}').nodes[0].label).toBe('GET /users/{id}');
    expect(parseFlow('api: GET /users/{id} {note}').nodes[0]).toMatchObject({ label: 'GET /users/{id}', shape: 'note' });
  });

  it('a bare line naming a node adds its words without resetting its label', () => {
    expect(parseFlow('check: Is it valid?\ncheck {decision}').nodes[0]).toMatchObject({ label: 'Is it valid?', shape: 'decision' });
  });

  it('reads -> and : by position: a node label may hold ->, a link label may hold -> and :', () => {
    expect(parseFlow('step: Click -> Save')).toMatchObject({ nodes: [{ id: 'step', label: 'Click -> Save' }], edges: [] });
    expect(parseFlow('a -> b: yes -> c').edges).toEqual([{ from: 'a', to: 'b', label: 'yes -> c' }]);
    expect(parseFlow('a -> b: x: y').edges).toEqual([{ from: 'a', to: 'b', label: 'x: y' }]);
    const f = parseFlow('a {start} -> b {danger}');
    expect(f.nodes.map(n => [n.id, n.shape, n.tone])).toEqual([['a', 'start', 'neutral'], ['b', 'box', 'danger']]);
  });

  it('counts each node left out once, and the links lost with it', () => {
    const full = Array.from({ length: MAX_NODES }, (_, i) => `n${i}`).join('\n');
    const f = parseFlow(`${full}\nextra -> n1\nextra -> n2\nextra\nextra`);
    expect(f.droppedNodes).toBe(1);
    expect(f.droppedLinks).toBe(2);
  });

  it('draws a self-loop as a loop beside the node, not a spike', () => {
    const l = layoutFlow(parseFlow('a -> a'));
    const e = l.edges[0];
    expect(e.self).toBe(true);
    expect(e.y1).not.toBe(e.y2);
    expect(l.width).toBeGreaterThan(l.nodes[0].x + l.nodes[0].w + 20);
  });

  it('a node defined after a link that named it by label is the same node', () => {
    const f = parseFlow('x -> Login\nlogin: Login {success}');
    expect(f.nodes.map(n => [n.id, n.label, n.tone])).toEqual([['x', 'x', 'neutral'], ['login', 'Login', 'success']]);
    expect(f.edges).toEqual([{ from: 'x', to: 'login', label: '' }]);
  });

  it('a decision diamond is big enough for its text', () => {
    const n = layoutFlow(parseFlow('d: Is the build green and signed? {decision}')).nodes[0];
    const textW = Math.max(...n.lines.map(l => l.length)) * 7;
    const textH = n.lines.length * 16;
    // The text box at the centre fits inside the diamond: |dx|/(w/2) + |dy|/(h/2) <= 1 at its corner.
    expect(textW / 2 / (n.w / 2) + textH / 2 / (n.h / 2)).toBeLessThanOrEqual(1);
  });

  it('groups that follow each other share a lane, so a pipeline of groups stays on one row', () => {
    const text = '[Frontend]\nview: Chat view\napi: api.ts\n[Rust backend]\ncmd: Command\ndb: Store\n[]\nview -> api: calls\napi -> cmd -> db: writes';
    const l = layoutFlow(parseFlow(text), 'right');
    expect(new Set(l.nodes.map(n => n.y)).size).toBe(1);
    const [front, back] = l.groups;
    expect(front.x + front.w).toBeLessThan(back.x);
  });

  // Review round 2.
  it('in right mode the gap between layers fits a forward link label', () => {
    const l = layoutFlow(parseFlow('a -> b: this is a long label'), 'right');
    const [a, b] = l.nodes;
    const e = l.edges[0];
    expect(e.lx - e.lw / 2).toBeGreaterThanOrEqual(a.x + a.w);
    expect(e.lx + e.lw / 2).toBeLessThanOrEqual(b.x);
  });

  it('a back link or loop label starts past its node, not under it', () => {
    for (const text of ['a -> b\nb -> a: try again please', 'a -> a: retry until the queue drains']) {
      const l = layoutFlow(parseFlow(text));
      const e = l.edges.find(x => x.label)!;
      const from = l.nodes.find(n => n.id === e.from)!;
      expect(e.lx - e.lw / 2, text).toBeGreaterThanOrEqual(from.x + from.w);
    }
  });

  it('a long group name fits its box (widened, or cut short with the whole name kept)', () => {
    const text = '[Authentication and session services]\nauth\n[Billing]\npay';
    for (const dir of ['down', 'right'] as const) {
      const l = layoutFlow(parseFlow(text), dir);
      for (const g of l.groups) {
        expect(g.text.length * 6.6 + 20, `${g.name} (${dir})`).toBeLessThanOrEqual(g.w + 0.5);
        expect(g.name.startsWith(g.text.replace(/…$/, ''))).toBe(true);
      }
    }
  });

  it('counts a link lost with a dropped node, never an empty end', () => {
    expect(parseFlow('a ->').droppedLinks).toBe(0);
    expect(parseFlow('-> b').droppedLinks).toBe(0);
    const full = Array.from({ length: MAX_NODES }, (_, i) => `n${i}`).join('\n');
    expect(parseFlow(`${full}\nx -> y`)).toMatchObject({ droppedNodes: 2, droppedLinks: 1 });
  });

  it('a link between bracketed names is a link, not a group heading', () => {
    const f = parseFlow('[client] -> [server]');
    expect(f.groups).toEqual([]);
    expect(f.edges).toEqual([{ from: '[client]', to: '[server]', label: '' }]);
  });

  it('a bare line naming a node by its label updates that node', () => {
    const f = parseFlow('login: Log in\nLog in {danger}');
    expect(f.nodes.map(n => [n.id, n.tone])).toEqual([['login', 'danger']]);
  });

  // Review round 3.
  it('a label never sits on a node, also for a link that skips a layer or a long way back', () => {
    const cases: Array<[string, 'down' | 'right']> = [
      ['a -> b -> c\na -> c: skip ahead', 'down'],
      ['a -> b -> c\na -> c: a long label here', 'right'],
      ['a -> b -> c\na -> d\nc -> a: go around again and again', 'down'],
    ];
    for (const [text, dir] of cases) {
      const l = layoutFlow(parseFlow(text), dir);
      for (const e of l.edges.filter(x => x.text)) {
        for (const n of l.nodes) {
          const apart = e.lx + e.lw / 2 <= n.x || n.x + n.w <= e.lx - e.lw / 2 || e.ly + 9 <= n.y || n.y + n.h <= e.ly - 9;
          expect(apart, `"${e.text}" on ${n.id} (${dir})`).toBe(true);
        }
      }
    }
  });

  it('a link that skips a layer goes round the node there, through its own slot', () => {
    for (const dir of ['down', 'right'] as const) {
      const l = layoutFlow(parseFlow('a -> b -> c\na -> c: skip'), dir);
      const skip = l.edges.find(e => e.from === 'a' && e.to === 'c')!;
      expect(skip.points).toHaveLength(3);
      const [x, y] = skip.points[1];
      const b = l.nodes.find(n => n.id === 'b')!;
      const inB = x >= b.x && x <= b.x + b.w && y >= b.y && y <= b.y + b.h;
      expect(inB, dir).toBe(false);
      // Sample the eased path: it never enters b.
      for (let i = 1; i < skip.points.length; i++) {
        const [px, py] = skip.points[i - 1], [qx, qy] = skip.points[i];
        for (let t = 0; t <= 1; t += 0.05) {
          const ease = t * t * (3 - 2 * t);
          const sx = dir === 'right' ? px + (qx - px) * t : px + (qx - px) * ease;
          const sy = dir === 'right' ? py + (qy - py) * ease : py + (qy - py) * t;
          expect(sx > b.x && sx < b.x + b.w && sy > b.y && sy < b.y + b.h, `${dir} t=${t}`).toBe(false);
        }
      }
    }
  });

  // Review round 4.
  it('a label on a link that bends sideways sits on the drawn curve', () => {
    // root -> end skips a layer of six siblings, so its first stretch runs
    // from root to a slot well to the side, and its label's spot (the gap
    // after root) is not that stretch's middle.
    const text = ['root -> end: outer', ...Array.from({ length: 6 }, (_, i) => `root -> m${i} -> end`)].join('\n');
    const l = layoutFlow(parseFlow(text));
    const e = l.edges.find(x => x.text)!;
    expect(e.points.length).toBe(3);
    const [[px, py], [qx, qy]] = e.points;
    expect(Math.abs(qx - px)).toBeGreaterThan(100);
    expect(Math.abs((py + qy) / 2 - e.ly)).toBeGreaterThan(5);
    // The renderer's cubic: (px,py) (px,m) (qx,m) (qx,qy). Find its x at the label's y.
    const m = (py + qy) / 2;
    const at = (t: number) => {
      const u = 1 - t;
      return [u * u * u * px + 3 * u * u * t * px + 3 * u * t * t * qx + t * t * t * qx,
        u * u * u * py + 3 * u * u * t * m + 3 * u * t * t * m + t * t * t * qy];
    };
    let lo = 0, hi = 1;
    for (let i = 0; i < 40; i++) { const mid = (lo + hi) / 2; if (at(mid)[1] < e.ly) lo = mid; else hi = mid; }
    expect(Math.abs(at(lo)[0] - e.lx)).toBeLessThan(1);
  });

});
