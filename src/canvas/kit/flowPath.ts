/** Shared edge geometry for the live kit and standalone flow exports. */
import { routePath } from './flowRoute';
import type { PlacedEdge } from './flow';

export function edgePath(e: PlacedEdge, across: boolean): string {
  if (e.route) return routePath(e.route);
  const { x1, y1, x2, y2, bulge: out } = e;
  if (e.back) {
    return across
      ? `M${x1},${y1} C${x1},${y1 + out} ${x2},${y2 + out} ${x2},${y2}`
      : `M${x1},${y1} C${x1 + out},${y1} ${x2 + out},${y2} ${x2},${y2}`;
  }
  // Through every point, each stretch easing out of one and into the next.
  let d = `M${x1},${y1}`;
  for (let i = 1; i < e.points.length; i++) {
    const [px, py] = e.points[i - 1], [qx, qy] = e.points[i];
    d += across
      ? ` C${(px + qx) / 2},${py} ${(px + qx) / 2},${qy} ${qx},${qy}`
      : ` C${px},${(py + qy) / 2} ${qx},${(py + qy) / 2} ${qx},${qy}`;
  }
  return d;
}
