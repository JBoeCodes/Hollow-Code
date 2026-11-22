// M5 offline UI: this Mac's offline continuations (`chat_offline_rows`),
// oldest first. ChatBody reads it to show the chat's notice on an offline
// copy, which has no home record; the lazy chatOffline.ts fills it.
import { createSignal } from 'solid-js';
import type { OfflineRow } from '../generated/ipc';

export const [offlineRows, setOfflineRows] = createSignal<readonly OfflineRow[]>([]);

/** Whether chat `key` is an offline copy this Mac keeps a record of. */
export function isOfflineCopy(key: string | null | undefined): boolean {
  return !!key && offlineRows().some(r => r.offlineKey === key);
}
