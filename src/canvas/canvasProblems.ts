/** Repo ideas 2026-10-27 #1: each reply canvas's layout and design problem
 *  counts, by run, so that run's receipt can say them in its Canvases row.
 *  The findings row (`CanvasLayoutAudit`) notes them as it judges a settled
 *  reply's canvas; a view without that canvas on screen (Activity, a
 *  workflow's receipt) has none, which shows nothing, never zero. Kept
 *  small and outside the lazy row's chunk: the receipt is in the app's. */
import { createSignal } from 'solid-js';

export interface CanvasProblemCounts { layout: number; design: number; more?: boolean }
type Held = CanvasProblemCounts & { owner?: object };

const MAX_RUNS = 200;
const [byRun, setByRun] = createSignal<ReadonlyMap<string, ReadonlyMap<string, Held>>>(new Map());

/** Record (or with null, forget) one canvas's counts in a run's reply.
 *  With an `owner` (the row that judged it), a null only forgets that
 *  owner's entry, so a row unmounting after its replacement mounted leaves
 *  the replacement's count alone. */
export function noteCanvasProblems(runId: string, canvasId: string, counts: CanvasProblemCounts | null, owner?: object): void {
  setByRun(prev => {
    const held = prev.get(runId)?.get(canvasId);
    if (!counts && owner && held && held.owner !== owner) return prev;
    const next = new Map(prev);
    const canvases = new Map(next.get(runId) ?? []);
    if (counts && counts.layout + counts.design > 0) canvases.set(canvasId, { ...counts, owner });
    else canvases.delete(canvasId);
    next.delete(runId);
    if (canvases.size) next.set(runId, canvases);
    while (next.size > MAX_RUNS) next.delete(next.keys().next().value!);
    return next;
  });
}

/** A run's canvases' counts summed, or null when none was noted. */
export function runCanvasProblems(runId: string | null | undefined): CanvasProblemCounts | null {
  const canvases = runId ? byRun().get(runId) : undefined;
  if (!canvases?.size) return null;
  let layout = 0, design = 0, more = false;
  for (const c of canvases.values()) { layout += c.layout; design += c.design; more ||= !!c.more; }
  return { layout, design, more };
}

/** The row's and the receipt's words: "1 layout problem", "3 design
 *  problems", "21+ layout and design problems". Null for none. */
export function problemsLabel(layout: number, design: number, more = false): string | null {
  const n = layout + design;
  if (!n) return null;
  const what = layout && design ? 'layout and design' : layout ? 'layout' : 'design';
  return `${n}${more ? '+' : ''} ${what} problem${n === 1 && !more ? '' : 's'}`;
}
