// Which sidebar rows show their nested chats (3.6). A row with a working or waiting child opens
// by itself; the person's own fold or unfold wins from then on, until the window reloads.
import { createSignal } from 'solid-js';

const [explicit, setExplicit] = createSignal<ReadonlyMap<string, boolean>>(new Map());

/** Whether `key`'s children show: the person's choice, else open while a child needs a look. */
export function childrenOpen(key: string, needsLook: boolean): boolean {
  return explicit().get(key) ?? needsLook;
}

export function toggleChildren(key: string, now: boolean) {
  setExplicit(prev => new Map(prev).set(key, !now));
}
