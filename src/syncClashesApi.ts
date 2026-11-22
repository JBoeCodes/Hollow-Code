// The context sync clash cards' IPC (CX6c; Rust `sync_clash.rs`). Its own
// module (calls keep api.ts's mock routing), so the App chunk carries none
// of it: only the lazy dialog and the Settings, Context panel import it.
import { invoke } from './api';
import type { ClashResolved, SyncClash } from './generated/ipc';

export type ClashChoice = 'keep_mine' | 'keep_theirs' | 'keep_both' | 'edit' | 'done';

/** The open cards, with both versions' first lines. */
export const syncClashes = () => invoke<SyncClash[]>('sync_clashes');
/** The person's choice on one card, named by its root, file and copy. */
export const resolveSyncClash = (root: string, path: string, copy: string, choice: ClashChoice) =>
  invoke<ClashResolved>('resolve_sync_clash', { root, path, copy, choice });
