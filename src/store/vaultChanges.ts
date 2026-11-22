import { createSignal } from 'solid-js';
import { listen } from '../nativeEvents';
import type { VaultChange } from '../types';

/**
 * The latest `vault-changed` event, as a signal (Canvas C5a P2). Views that
 * show one file (the Overview and Brief views, `WorkFileEditor`) watch it and
 * re-read when their path is in it (`sectionSource.vaultChangeTouches`), so an
 * edit made in another app shows without "Reload file". `seq` makes two equal
 * payloads two changes. The backend already debounces (300 ms); the
 * workspace-detail refresh in `store/workspaces.ts` keeps its own timer.
 */
export const [vaultChange, setVaultChange] = createSignal<(VaultChange & { seq: number }) | null>(null);

let seq = 0;
listen<VaultChange>('vault-changed', e => {
  if (e?.payload) setVaultChange({ folders: e.payload.folders ?? [], paths: e.payload.paths ?? [], seq: ++seq });
}).catch(() => { /* browser preview */ });
