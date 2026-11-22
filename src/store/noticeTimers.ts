import { createSignal, untrack } from 'solid-js';

// The App's single focus detector owns this state. Do not substitute
// document.hidden: an uncovered Hollow window can still be behind Terminal.
const [windowAway, writeAway] = createSignal(false);
export { windowAway };
const timers = new Set<NoticeTimer>();
export const setWindowAway = (value: boolean | ((previous: boolean) => boolean)): boolean => {
  const next = writeAway(value);
  for (const timer of timers) timer.away(next);
  return next;
};

export interface NoticeTimer {
  hold(reason?: string): void;
  release(reason?: string): void;
  held(): boolean;
  away(value: boolean): void;
  dispose(): void;
}

/** Oldest-first live transient ids. Held actions and the incoming row survive. */
export function trimNotices(ids: string[], clocks: ReadonlyMap<string, NoticeTimer>, evict: (id: string) => void, incoming?: string) {
  let excess = ids.length - 5;
  for (const id of ids) {
    if (excess <= 0) break;
    if (id === incoming || clocks.get(id)?.held()) continue;
    evict(id); excess--;
  }
}

/** One countdown with independent interaction/away holds and a return floor.
 * A nonpositive duration is sticky. Disposing is safe during expiry/eviction. */
export function noticeTimer(duration: number, expire: () => void): NoticeTimer {
  let left = duration, ends = 0;
  let handle: ReturnType<typeof setTimeout> | undefined;
  const holds = new Set<string>();
  let away = untrack(windowAway), disposed = false;
  const pause = () => {
    if (handle === undefined) return;
    clearTimeout(handle); handle = undefined;
    left = Math.max(0, ends - Date.now());
  };
  const start = (returning = false) => {
    if (disposed || holds.size || away || duration <= 0 || handle !== undefined) return;
    if (returning) left = Math.max(left, 3000);
    ends = Date.now() + left;
    handle = setTimeout(() => { timer.dispose(); expire(); }, left);
  };
  const timer: NoticeTimer = {
    hold(reason = 'interaction') { holds.add(reason); pause(); },
    release(reason = 'interaction') { if (holds.delete(reason)) start(true); },
    held() { return holds.size > 0; },
    away(value) { away = value; if (away) pause(); else start(true); },
    dispose() { disposed = true; clearTimeout(handle); handle = undefined; timers.delete(timer); },
  };
  timers.add(timer);
  start();
  return timer;
}
