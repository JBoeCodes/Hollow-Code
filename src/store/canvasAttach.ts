// A saved canvas's "Attach to issue…" (A19): which canvas the dialog is
// for. Its own tiny module so the shell can mount the lazy dialog
// (`components/AttachCanvasDialog.tsx`) without importing it.
import { createSignal } from 'solid-js';

/** A saved canvas and the revision the person was looking at. */
export interface CanvasAttachTarget { workspaceId: string; canvasId: string; revision: number; title: string }

const [target, setTarget] = createSignal<CanvasAttachTarget | null>(null);
export const attachCanvasTarget = target;
export const openAttachCanvas = (t: CanvasAttachTarget) => setTarget(t);
export const closeAttachCanvas = () => setTarget(null);
