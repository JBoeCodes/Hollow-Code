import { createSignal } from 'solid-js';
import type { CanvasExportSource } from '../canvas/canvasExport';
export const [canvasItemsTarget, setCanvasItemsTarget] = createSignal<CanvasExportSource | null>(null);
export const [canvasItemsBusy, setCanvasItemsBusy] = createSignal(false);
export function openCanvasItems(target: CanvasExportSource) { if (!canvasItemsBusy()) setCanvasItemsTarget(target); }
export function closeCanvasItems() { if (!canvasItemsBusy()) setCanvasItemsTarget(null); }
