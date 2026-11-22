import { onCleanup, onMount } from 'solid-js';
import { listen, type UnlistenFn } from '../nativeEvents';

/** Queue events lack a folder; reread the backend guard for any transition.
 * This also catches one lane finishing while another keeps the folder busy. */
export function watchBuildFreshness(id: () => string, element: () => HTMLElement | undefined, refresh: () => Promise<unknown>) {
  let disposed = false;
  const unlisteners: UnlistenFn[] = [];
  const visible = () => document.visibilityState !== 'hidden' && !!element()?.getClientRects().length;
  const read = () => { if (visible()) void refresh(); };
  const register = (pending: Promise<UnlistenFn>) => {
    void pending.then(fn => { if (disposed) fn(); else unlisteners.push(fn); }).catch(() => { /* polling remains available */ });
  };
  onMount(() => {
    register(listen('run-queue-event', read));
    register(listen<string>('build-changed', e => { if (e.payload === id()) read(); }));
    const timer = setInterval(read, 15000);
    document.addEventListener('visibilitychange', read);
    onCleanup(() => { clearInterval(timer); document.removeEventListener('visibilitychange', read); });
  });
  onCleanup(() => { disposed = true; unlisteners.forEach(fn => fn()); });
}
