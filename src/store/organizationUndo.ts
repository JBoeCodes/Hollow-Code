// The newest chat organization action (settle, snooze, pin, wake) whose Undo
// snackbar still shows. Edit → Undo Chat Action runs it (3.1): there is no
// ⌘Z of our own, so text fields keep theirs. Gone when the snackbar leaves.
import { createSignal } from 'solid-js';
import { leavingSnackbars, performUndo, snackbars } from './snackbars';

/** How long an organization action's Undo stays (ms). */
export const ORGANIZATION_UNDO_MS = 5000;

const [lastId, setLastId] = createSignal<string | null>(null);

/** Remember the snackbar `flashWithUndo` returned for an organization action. */
export function rememberOrganizationUndo(id: string | undefined) { setLastId(id ?? null); }

/** True while an organization Undo can still run. */
export function organizationUndoReady(): boolean {
  const id = lastId();
  return !!id && !leavingSnackbars().has(id) && snackbars().some(b => b.id === id && !!b.undo);
}

/** Run the newest organization Undo; false when none shows. */
export function undoLastOrganization(): boolean {
  const id = lastId();
  if (!id || !organizationUndoReady()) return false;
  setLastId(null);
  void performUndo(id);
  return true;
}
