// The last canvas the person used (any surface): the palette's target for
// "Export canvas…" and the `recent-canvas` gates. Its own tiny module so the
// shell can read it without importing a canvas surface. Split out of the
// canvas page export's store when that was removed (2026-11-10).
import { createSignal } from 'solid-js';

/** A canvas as the palette needs it: its title and its fence's source. */
export interface CanvasSource { title: string; source: string; ref?: import('../canvas/canvasExport').CanvasExportRef }

const [last, setLast] = createSignal<CanvasSource | null>(null);
/** The canvas the person last pointed at or focused (any surface). */
export const lastCanvas = last;
export const noteCanvasUsed = (c: CanvasSource) => setLast(c);
