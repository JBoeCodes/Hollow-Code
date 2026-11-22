import { createSignal } from 'solid-js';
export const CANVAS_PINS_KEY = 'hollow.canvasPins.v1';
function present() { try { const raw = localStorage.getItem(CANVAS_PINS_KEY); return !!raw && raw !== '[]'; } catch { return false; } }
/** Launch only needs this hint; validation and all readers stay in the lazy chunk. */
export const [hasCanvasPins, setHasCanvasPins] = createSignal(present());
