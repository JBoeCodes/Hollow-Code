import { invoke } from '../api';
import type { CanvasItemSource, CanvasPlanItems } from '../generated/ipc';

// Dialog-only calls stay in its lazy chunk; retain shared mock routing and IPC tallying.
export const canvasItemSource = (workspaceId: string, id: string) => invoke<CanvasItemSource>('canvas_item_source', { workspaceId, id });
export const canvasPlanItems = (request: CanvasPlanItems) => invoke<string>('canvas_plan_items', { request });
