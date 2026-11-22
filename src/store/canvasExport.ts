import { createSignal } from 'solid-js';
import type { CanvasExportSource } from '../canvas/canvasExport';
export const [canvasExportTarget, setCanvasExportTarget] = createSignal<CanvasExportSource | null>(null);
export const [canvasExportBusy, setCanvasExportBusy] = createSignal(false);
export function openCanvasExport(source: CanvasExportSource) { if (!canvasExportBusy()) setCanvasExportTarget(source); }
export function closeCanvasExport() { if (!canvasExportBusy()) setCanvasExportTarget(null); }
