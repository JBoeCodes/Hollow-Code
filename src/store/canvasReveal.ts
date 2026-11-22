import { createSignal } from 'solid-js';

/**
 * "Waiting on you" reopens an ask-canvas by its exact id (Canvas C4 A3): the
 * row asks for that canvas revision here, then opens its conversation by key
 * (`openSessionByKey`); the chat that shows that conversation takes the
 * request once its transcript has loaded and scrolls the canvas into view
 * (`ChatBody`). One request at a time; a newer one replaces it.
 */
export interface CanvasReveal { key: string; canvasId: string; revision: number }

const [canvasReveal, setCanvasReveal] = createSignal<CanvasReveal | null>(null);
export { canvasReveal };

export function requestCanvasReveal(key: string, canvasId: string, revision: number): void {
  setCanvasReveal({ key, canvasId, revision });
}

/** The request for conversation `key`, taken (so it runs once), or null. */
export function takeCanvasReveal(key: string | null): CanvasReveal | null {
  const want = canvasReveal();
  if (!want || !key || want.key !== key) return null;
  setCanvasReveal(null);
  return want;
}

export function clearCanvasReveal(): void {
  setCanvasReveal(null);
}
