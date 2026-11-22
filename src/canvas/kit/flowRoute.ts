/** Bounded rectilinear visibility grid. Routes keep an eight-unit clearance
 * from obstacles; six-unit rounded corners therefore cannot cut into a box.
 * Pure geometry, shared by layout and the renderer's regression tests. */
export type Point = [
    number,
    number
];
export interface Rect {
    x: number;
    y: number;
    w: number;
    h: number;
}
const inside = ([x, y]: Point, r: Rect) => x > r.x && x < r.x + r.w && y > r.y && y < r.y + r.h;
const clear = (a: Point, b: Point, rs: Rect[]) => !rs.some(r => a[0] === b[0]
    ? a[0] > r.x && a[0] < r.x + r.w && Math.max(a[1], b[1]) > r.y && Math.min(a[1], b[1]) < r.y + r.h
    : a[1] > r.y && a[1] < r.y + r.h && Math.max(a[0], b[0]) > r.x && Math.min(a[0], b[0]) < r.x + r.w);
export function routeFlow(start: Point, end: Point, obstacles: Rect[], budget = { left: 12000 }): Point[] {
    const rs = obstacles.map(r => ({ x: r.x - 8, y: r.y - 8, w: r.w + 16, h: r.h + 16 }));
    // Ports are outside endpoint nodes. Never drop clearance to find a route:
    // an impossible route is disclosed rather than rounded into an obstacle.
    if (rs.some(r => inside(start, r) || inside(end, r)))
        return [];
    const xs = [...new Set([start[0], end[0], ...rs.flatMap(r => [r.x, r.x + r.w])])].sort((a, b) => a - b);
    const ys = [...new Set([start[1], end[1], ...rs.flatMap(r => [r.y, r.y + r.h])])].sort((a, b) => a - b);
    const w = xs.length, count = w * ys.length;
    const point = (i: number): Point => [xs[i % w], ys[Math.floor(i / w)]];
    const sid = ys.indexOf(start[1]) * w + xs.indexOf(start[0]), tid = ys.indexOf(end[1]) * w + xs.indexOf(end[0]);
    const dist = new Float64Array(count).fill(Infinity), prev = new Int32Array(count).fill(-1);
    const blocked = new Int8Array(count).fill(-1);
    const free = (i: number) => { if (blocked[i] < 0)
        blocked[i] = Number(rs.some(r => inside(point(i), r))); return !blocked[i]; };
    const heap: Array<[
        number,
        number
    ]> = [];
    const push = (v: [
        number,
        number
    ]) => { let i = heap.length; heap.push(v); while (i) {
        const p = (i - 1) >> 1;
        if (heap[p][0] <= v[0])
            break;
        heap[i] = heap[p];
        i = p;
    } heap[i] = v; };
    const pop = () => { const top = heap[0], last = heap.pop()!; if (heap.length) {
        let i = 0;
        while (i * 2 + 1 < heap.length) {
            let c = i * 2 + 1;
            if (c + 1 < heap.length && heap[c + 1][0] < heap[c][0])
                c++;
            if (heap[c][0] >= last[0])
                break;
            heap[i] = heap[c];
            i = c;
        }
        heap[i] = last;
    } return top; };
    const guess = (i: number) => { const p = point(i); return Math.abs(p[0] - end[0]) + Math.abs(p[1] - end[1]); };
    dist[sid] = 0;
    push([guess(sid), sid]);
    let expansions = 0;
    while (heap.length) {
        if (++expansions > 12000 || --budget.left < 0)
            return [];
        const [score, i] = pop();
        if (score > dist[i] + guess(i) + 0.001)
            continue;
        if (i === tid)
            break;
        const p = point(i), col = i % w, row = Math.floor(i / w);
        const near = [col ? i - 1 : -1, col + 1 < w ? i + 1 : -1, row ? i - w : -1, row + 1 < ys.length ? i + w : -1];
        for (const j of near) {
            if (j < 0 || !free(j))
                continue;
            const q = point(j);
            if (!clear(p, q, rs))
                continue;
            const d = dist[i] + Math.abs(p[0] - q[0]) + Math.abs(p[1] - q[1]);
            if (d < dist[j]) {
                dist[j] = d;
                prev[j] = i;
                push([d + guess(j), j]);
            }
        }
    }
    if (!Number.isFinite(dist[tid]))
        return []; // Disclose an unroutable link; never draw through a box.
    const path: Point[] = [];
    for (let i = tid; i >= 0; i = prev[i]) {
        path.push(point(i));
        if (i === sid)
            break;
    }
    path.reverse();
    return path.filter((p, i) => i === 0 || i === path.length - 1 || !((path[i - 1][0] === p[0] && p[0] === path[i + 1][0]) || (path[i - 1][1] === p[1] && p[1] === path[i + 1][1])));
}
/** The rounded path's actual line/quadratic segments, also used by tests. */
export type RouteSegment = {
    from: Point;
    to: Point;
    control?: Point;
};
export function routeSegments(points: Point[]): RouteSegment[] {
    if (points.length < 2)
        return [];
    const segments: RouteSegment[] = [];
    let from = points[0];
    for (let i = 1; i < points.length - 1; i++) {
        const a = points[i - 1], b = points[i], c = points[i + 1];
        const ab = Math.hypot(b[0] - a[0], b[1] - a[1]), bc = Math.hypot(c[0] - b[0], c[1] - b[1]), r = Math.min(6, ab / 2, bc / 2);
        const before: Point = [b[0] + (a[0] - b[0]) * r / ab, b[1] + (a[1] - b[1]) * r / ab];
        const after: Point = [b[0] + (c[0] - b[0]) * r / bc, b[1] + (c[1] - b[1]) * r / bc];
        segments.push({ from, to: before }, { from: before, to: after, control: b });
        from = after;
    }
    segments.push({ from, to: points[points.length - 1] });
    return segments;
}
export function routePath(points: Point[]): string {
    return points.length ? `M${points[0]}` + routeSegments(points).map(s => s.control ? ` Q${s.control} ${s.to}` : ` L${s.to}`).join('') : '';
}
