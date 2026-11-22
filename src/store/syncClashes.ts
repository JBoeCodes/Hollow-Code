// Whether the context sync clash cards (CX6c) are open, and for which root
// (null: every root). Its own tiny module so the shell can mount the lazy
// view without importing it.
import { createSignal } from 'solid-js';

const [open, setOpen] = createSignal(false);
const [root, setRoot] = createSignal<string | null>(null);
export const syncClashesOpen = open;
export const syncClashesRoot = root;
export const openSyncClashes = (only: string | null = null) => { setRoot(only); setOpen(true); };
export const closeSyncClashes = () => setOpen(false);
