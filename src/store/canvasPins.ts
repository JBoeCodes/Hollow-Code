import { createSignal } from 'solid-js';
import { decodePins, MAX_PINS, pinKey, type CanvasPin } from '../canvas/liveModel';
import { CANVAS_PINS_KEY as KEY, setHasCanvasPins } from './canvasPinPresence';

function read() { try { return decodePins(localStorage.getItem(KEY)); } catch { return []; } }
export const [canvasPins, setCanvasPins] = createSignal(read());
export function saveCanvasPin(pin: CanvasPin, expected?: CanvasPin) {
  const list = read(); const others = list.filter(p => pinKey(p) !== pinKey(pin));
  if (expected && !list.some(p => pinKey(p) === pinKey(expected) && p.revision === expected.revision && p.sha256 === expected.sha256)) {
    throw Error('This pin changed or was removed. Refresh before updating it.');
  }
  if (others.length >= MAX_PINS) throw Error(`Unpin a canvas first (at most ${MAX_PINS}).`);
  const next = decodePins(JSON.stringify([...others, pin]));
  if (next.length !== others.length + 1) throw Error('This canvas cannot be pinned.');
  localStorage.setItem(KEY, JSON.stringify(next)); setCanvasPins(next); setHasCanvasPins(next.length > 0);
}
export function removeCanvasPin(pin: CanvasPin) {
  const next = read().filter(p => pinKey(p) !== pinKey(pin));
  localStorage.setItem(KEY, JSON.stringify(next)); setCanvasPins(next); setHasCanvasPins(next.length > 0);
}
