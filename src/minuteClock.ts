import { createSignal, onCleanup } from 'solid-js';

const [now, setNow] = createSignal(Date.now());
let users = 0;
let timer: ReturnType<typeof setInterval> | undefined;

/** Shared age clock. Only mounted consumers keep the one timer alive. */
export function minuteClock() {
  if (users++ === 0) {
    setNow(Date.now());
    timer = setInterval(() => setNow(Date.now()), 30_000);
  }
  onCleanup(() => { if (--users === 0) { clearInterval(timer); timer = undefined; } });
  return now;
}
