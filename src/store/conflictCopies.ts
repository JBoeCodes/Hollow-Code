// Whether the conflict-copies dialog (overnight A10) is open. Its own tiny
// module so the shell can mount the lazy view without importing it.
import { createSignal } from 'solid-js';

const [open, setOpen] = createSignal(false);
export const conflictCopiesOpen = open;
export const openConflictCopies = () => setOpen(true);
export const closeConflictCopies = () => setOpen(false);
