import { createSignal, onCleanup } from 'solid-js';

const [now, setNow] = createSignal(Date.now());
let users = 0;
let timer: ReturnType<typeof setInterval> | undefined;

/** Shared one-second clock for live elapsed labels (sidebar run timers).
 *  Only mounted consumers keep the one timer alive. */
export function secondClock() {
  if (users++ === 0) {
    setNow(Date.now());
    timer = setInterval(() => setNow(Date.now()), 1000);
  }
  onCleanup(() => { if (--users === 0) { clearInterval(timer); timer = undefined; } });
  return now;
}
