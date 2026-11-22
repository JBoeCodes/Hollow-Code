import { createSignal } from 'solid-js';
import type { WorkWorkspace } from '../workspaceTypes';

/** Eager menu state; the palette cache loads with a frame or the picker. */
export const [canvasLookTarget, setCanvasLookTarget] = createSignal<WorkWorkspace | null>(null);
