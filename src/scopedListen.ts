/**
 * A listener whose registration resolves later (Tauri's `listen`), with a
 * disposer that also covers the race where the view closes first
 * (overnight 10-11/12 P6). `listen(...).then(fn => { unlisten = fn })` with
 * `onCleanup(() => unlisten?.())` never unlistens when cleanup runs before
 * the promise resolves: the handler then outlives its view for the app's
 * lifetime, writing to disposed signals on every event.
 *
 * ```ts
 * onMount(() => onCleanup(scopedListen(() => listen('workflow-event', handler))));
 * ```
 */
export function scopedListen(start: () => Promise<() => void>): () => void {
  let disposed = false;
  let stop: (() => void) | null = null;
  start().then(fn => { if (disposed) fn(); else stop = fn; }).catch(() => { /* not in Tauri: nothing registered */ });
  return () => {
    disposed = true;
    const fn = stop;
    stop = null;
    fn?.();
  };
}
