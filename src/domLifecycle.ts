import { onCleanup } from 'solid-js';

type EventTargetWithMaps =
  | Document
  | Window
  | HTMLElement;

export function listen<K extends keyof DocumentEventMap>(
  target: Document,
  type: K,
  listener: (event: DocumentEventMap[K]) => void,
  options?: boolean | AddEventListenerOptions,
): void;
export function listen<K extends keyof WindowEventMap>(
  target: Window,
  type: K,
  listener: (event: WindowEventMap[K]) => void,
  options?: boolean | AddEventListenerOptions,
): void;
export function listen<K extends keyof HTMLElementEventMap>(
  target: HTMLElement,
  type: K,
  listener: (event: HTMLElementEventMap[K]) => void,
  options?: boolean | AddEventListenerOptions,
): void;
export function listen(
  target: EventTargetWithMaps,
  type: string,
  listener: EventListener,
  options?: boolean | AddEventListenerOptions,
): void {
  target.addEventListener(type, listener, options);
  onCleanup(() => target.removeEventListener(type, listener, options));
}
