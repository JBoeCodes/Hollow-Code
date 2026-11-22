// The visit watermark's state for one transcript (parity 0.3): the divider shown on opening a
// conversation that has turns the person had not seen, and the record of how far they read while it
// was open. `seen` follows what was on screen (the end of the thread when they are at it), flushed
// to storage when they leave, switch conversations or close the window.
import { createEffect, createSignal, on, onCleanup } from 'solid-js';
import { recordVisit, visitedSeen, watermarkAt } from '../../chatVisitMark';

export interface VisitMarkDeps {
  key: () => string | null | undefined;
  total: () => number;
  loading: () => boolean;
  atBottom: () => boolean;
  /** The last turn index on screen, or null. */
  lastOnScreen: () => number | null;
  /** Changes when the conversation's turns are replaced. */
  version: () => unknown;
}

export function createVisitMark(deps: VisitMarkDeps) {
  const [mark, setMark] = createSignal<{ index: number; count: number } | null>(null);
  let current: { key: string; seen: number } | null = null;
  const flush = () => { if (current && current.seen > 0) recordVisit(current.key, current.seen); };
  // Opening (or reloading) a conversation: read what was seen last, show the divider for what is new.
  createEffect(on(() => [deps.key(), deps.version(), deps.loading()] as const, ([key, , loading]) => {
    flush();
    current = null;
    setMark(null);
    const total = deps.total();
    if (!key || loading || total === 0) return;
    const prior = visitedSeen(key);
    const index = watermarkAt(prior, total);
    if (index !== null) setMark({ index, count: total - index });
    current = { key, seen: prior ?? 0 };
  }));
  // At the end of the thread, everything drawn has been seen.
  createEffect(() => {
    const total = deps.total();
    if (current && !deps.loading() && deps.atBottom()) current.seen = Math.max(current.seen, total);
  });
  let timer: ReturnType<typeof setTimeout> | undefined;
  /** The reader scrolled: after a pause, what is on screen counts as seen. */
  function noteRead() {
    clearTimeout(timer);
    timer = setTimeout(() => {
      const last = deps.lastOnScreen();
      if (current && last !== null) current.seen = Math.max(current.seen, last + 1);
    }, 300);
  }
  const onHide = () => flush();
  window.addEventListener('pagehide', onHide);
  onCleanup(() => { clearTimeout(timer); window.removeEventListener('pagehide', onHide); flush(); });
  return { mark, dismiss: () => setMark(null), noteRead };
}
