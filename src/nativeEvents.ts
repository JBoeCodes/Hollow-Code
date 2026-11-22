/**
 * Native (Tauri) event subscriptions behind one adapter (review F05,
 * 2026-10-13). Import `listen` from here, never from `@tauri-apps/api/event`:
 * in the app it is Tauri's own `listen`; on the browser mock (`?mock=1`) it is
 * a small in-page event bus, so a full-app mount never calls into the missing
 * native bridge (the old `transformCallback` TypeError) and a spec can deliver
 * an event with `window.__mockEmit(name, payload)`.
 *
 * `onNative` is the safe form for components: it returns a synchronous
 * disposer that also works before the subscription has resolved, and a
 * subscription that fails is reported once, never left as an unhandled
 * rejection. `scripts/check-native-events.mjs` (via test:scripts) refuses a
 * direct import of `@tauri-apps/api/event` anywhere else in src/.
 */
import { listen as tauriListen, type Event, type EventCallback, type UnlistenFn } from '@tauri-apps/api/event';
import { isBrowserMockMode } from './mockMode';

export type { Event, EventCallback, UnlistenFn };

type Handler = (e: Event<unknown>) => void;
const bus = new Map<string, Set<Handler>>();
let nextId = 1;

/** Deliver `payload` to every mock subscriber of `event` (mock mode only). */
export function emitMockEvent(event: string, payload: unknown): number {
  const handlers = [...(bus.get(event) ?? [])];
  for (const h of handlers) h({ event, id: nextId++, payload } as Event<unknown>);
  return handlers.length;
}

if (isBrowserMockMode()) {
  (window as unknown as { __mockEmit?: typeof emitMockEvent }).__mockEmit = emitMockEvent;
}

/** On the mock, use the bus unless a spec installed its own fake Tauri
 *  bridge (`window.__TAURI_INTERNALS__` with `__emitTauriEvent`): then Tauri's
 *  `listen` goes through that fake, as those specs expect. */
const useBus = () => isBrowserMockMode() && !('__TAURI_INTERNALS__' in window);

/** Drop-in for Tauri's `listen`. */
export function listen<T>(event: string, handler: EventCallback<T>): Promise<UnlistenFn> {
  if (useBus()) {
    const set = bus.get(event) ?? new Set<Handler>();
    bus.set(event, set);
    const h = handler as unknown as Handler;
    set.add(h);
    return Promise.resolve(() => { set.delete(h); });
  }
  return tauriListen<T>(event, handler);
}

/** Subscribe with a synchronous disposer: disposing before the subscription
 *  resolves unsubscribes as soon as it does; a failed subscription calls
 *  `onError` (default: a console warning) instead of rejecting unhandled. */
export function onNative<T>(event: string, handler: (payload: T) => void, onError?: (e: unknown) => void): () => void {
  let disposed = false;
  let off: UnlistenFn | undefined;
  listen<T>(event, e => { if (!disposed) handler(e.payload); })
    .then(u => { if (disposed) u(); else off = u; })
    .catch(e => { if (onError) onError(e); else console.warn(`Hollow: could not subscribe to ${event}`, e); });
  return () => { disposed = true; off?.(); off = undefined; };
}
