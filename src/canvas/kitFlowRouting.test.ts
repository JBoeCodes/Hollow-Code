import { describe, expect, it } from 'vitest';
import { routeFlow, routeSegments } from './kit/flowRoute';
import { layoutFlow, parseFlow } from './kit/flow';
const cases = [
    '[A]\na1\na2\n[B]\nb1\nb2\n[C]\nc1\nc2\n[]\na1 -> b2\nb1 -> c2\nc1 -> a2\na2 -> a1',
    '[Outer]\na\nb\n[Middle]\nc\nd\n[]\na -> c -> d -> b\na -> b: shortcut\nb -> a: return',
    '[A]\na\n[B]\nb\n[C]\nc\n[]\na -> b -> c\nc -> a: retry',
    '[A]\na\nb\n[B]\nc\nd\n[]\na -> a: loop\nb -> b\nc -> c\na -> c -> b -> d',
];
describe('flow obstacle routing', () => {
    for (const direction of ['down', 'right'] as const)
        it(`never draws through an unrelated group (${direction})`, () => {
            for (const text of cases) {
                const l = layoutFlow(parseFlow(text), direction);
                for (const e of l.edges) {
                    const own = [l.nodes.find(n => n.id === e.from)!.group, l.nodes.find(n => n.id === e.to)!.group];
                    expect(e.route?.length, `${e.from}->${e.to} routed`).toBeGreaterThan(1);
                    for (const segment of routeSegments(e.route!))
                        for (let j = 0; j <= 100; j++) {
                            const t = j / 100, u = 1 - t, { from: a, to: b, control: c } = segment;
                            const [x, y] = c ? [u * u * a[0] + 2 * u * t * c[0] + t * t * b[0], u * u * a[1] + 2 * u * t * c[1] + t * t * b[1]] : [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                            for (const g of l.groups.filter(g => !own.includes(g.name))) {
                                expect(x > g.x && x < g.x + g.w && y > g.y && y < g.y + g.h, `${e.from}->${e.to} crosses ${g.name}`).toBe(false);
                            }
                            for (const n of l.nodes.filter(n => n.id !== e.from && n.id !== e.to)) {
                                expect(x > n.x && x < n.x + n.w && y > n.y && y < n.y + n.h, `${e.from}->${e.to} crosses node ${n.id}`).toBe(false);
                            }
                        }
                }
            }
        });
    it('parses active edges without changing their endpoint or label', () => {
        const f = parseFlow('a -> b: running {active}\nb -> c {active}\na -> b: running');
        expect(f.edges).toEqual([{ from: 'a', to: 'b', label: 'running', active: true }, { from: 'b', to: 'c', label: '', active: true }]);
        expect(f.nodes.map(n => n.id)).toEqual(['a', 'b', 'c']);
    });
});
// Diverse bounded graphs exercise returns, skips, disconnected groups and loops.
for (const direction of ['down', 'right'] as const)
    it(`routes dense bounded graphs (${direction})`, () => {
        const started = performance.now();
        for (let seed = 1; seed <= 12; seed++) {
            let rand = seed;
            const next = () => { rand = (rand * 1664525 + 1013904223) >>> 0; return rand; };
            const rows = Array.from({ length: 18 }, (_, i) => `${i % 3 === 0 ? `[Group ${Math.floor(i / 3)}]\n` : ''}n${i}`);
            rows.push('[]');
            for (let j = 0; j < 30; j++)
                rows.push(`n${next() % 18} -> n${next() % 18}`);
            const l = layoutFlow(parseFlow(rows.join('\n')), direction);
            for (const e of l.edges) {
                expect(e.route?.length).toBeGreaterThan(1);
                const own = [l.nodes.find(n => n.id === e.from)!.group, l.nodes.find(n => n.id === e.to)!.group];
                const obstacles = [...l.groups.filter(g => !own.includes(g.name)), ...l.nodes];
                for (const { from: a, to: b, control: c } of routeSegments(e.route!))
                    for (let i = 1; i < 20; i++) {
                        const t = i / 20, u = 1 - t;
                        const [x, y] = c ? [u * u * a[0] + 2 * u * t * c[0] + t * t * b[0], u * u * a[1] + 2 * u * t * c[1] + t * t * b[1]] : [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                        for (const r of obstacles)
                            if (x > r.x + 0.01 && x < r.x + r.w - 0.01 && y > r.y + 0.01 && y < r.y + r.h - 0.01)
                                throw new Error(`${seed} ${e.from}->${e.to} crosses ${JSON.stringify(r)}`);
                    }
            }
        }
        expect(performance.now() - started).toBeLessThan(5000);
    });
it('measurement skips routing and exhausted work budgets fail closed', () => {
    const flow = parseFlow(cases[0]);
    expect(layoutFlow(flow, 'right', false).edges.every(e => e.route === undefined)).toBe(true);
    expect(routeFlow([0, 0], [30, 30], [{ x: 8, y: 8, w: 4, h: 4 }], { left: 0 })).toEqual([]);
});
it('ungrouped kit@1 flows retain their existing curves', () => {
    const l = layoutFlow(parseFlow('a -> b -> c\na -> c: skip\nc -> a: return'));
    expect(l.edges.every(e => e.route === undefined)).toBe(true);
});
it('caps worst-case routing and discloses exhausted routes', () => {
    const rows = Array.from({ length: 60 }, (_, i) => `[G${i}]\nn${i}`);
    rows.push('[]');
    for (let i = 0; i < 60; i++) {
        rows.push(`n${i} -> n${(i + 17) % 60}`);
        rows.push(`n${i} -> n${(i + 31) % 60}`);
    }
    const start = performance.now(), l = layoutFlow(parseFlow(rows.join('\n')));
    expect(l.edges).toHaveLength(120);
    expect(l.edges.every(e => Array.isArray(e.route))).toBe(true);
    expect(performance.now() - start).toBeLessThan(2500);
});

it('keeps grouped branch labels apart in the release gallery', () => {
    const flow = parseFlow('[Build]\nsource: Change ready {start}\nchecks: Local gate {info}\n[Review]\nreview: Review findings? {decision}\nfix: Repair findings {warning}\n[Delivery]\nmerge: Merge {success}\nready: Native checks {end}\n[]\nsource -> checks -> review\nreview -> fix: needs work\nfix -> checks: recheck\nreview -> merge: green {active}\nmerge -> ready {active}\nsource -> ready: overview');
    for (const direction of ['right', 'down'] as const) {
        const l = layoutFlow(flow, direction), labels = l.edges.filter(e => e.text);
        expect(labels).toHaveLength(4);
        for (let i = 0; i < labels.length; i++) for (let j = i + 1; j < labels.length; j++) {
            const a = labels[i], b = labels[j];
            expect(Math.abs(a.lx - b.lx) < (a.lw + b.lw) / 2 && Math.abs(a.ly - b.ly) < 18, `${direction}: ${a.text} overlaps ${b.text}`).toBe(false);
        }
    }
});

it('omits crowded visual labels without changing the connection data', () => {
    const lines = Array.from({length:8},(_,i)=>`[G${i}]\nn${i}`);
    for(let a=0;a<8;a++) for(let b=0;b<8;b++) if(a!==b) lines.push(`n${a} -> n${b}: Connection ${a} to ${b}`);
    const layout=layoutFlow(parseFlow(lines.join('\n')));
    expect(layout.edges.some(e=>e.label && !e.text && e.route?.length)).toBe(true);
    expect(layout.edges.every(e=>e.label.startsWith('Connection '))).toBe(true);
});
