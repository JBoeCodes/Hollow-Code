/**
 * Pinned workspaces (2026-10-21), the pure half: which workspaces are
 * pinned and in what order, and where a drag or Move up / Move down puts
 * one. The pin itself is `pinned: <rank>` in the workspace's brief
 * (src-tauri/src/workspace_pins.rs), so the sidebar's Pinned group, Home and
 * agents all read one state. store/workspacePins.ts writes it.
 */
import { isBuiltinWorkspace, type WorkWorkspace } from './workspaceTypes';

/** Whether a workspace may be pinned: not built in, not archived. */
export function canPinWorkspace(w: WorkWorkspace): boolean {
  return !isBuiltinWorkspace(w) && w.status.trim().toLowerCase() !== 'archived';
}

/** The pinned workspaces' ids, lower rank first, ties by id (as the backend
 *  orders them). The backend already drops archived and built-in pins; this
 *  drops them again so a stale list can't show one. */
export function pinOrder(all: readonly WorkWorkspace[]): string[] {
  return all
    .filter(w => w.pinned != null && canPinWorkspace(w))
    .sort((a, b) => a.pinned! - b.pinned! || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))
    .map(w => w.id);
}

/** The 1-based place `id` lands at when dropped on `target` (before it, or
 *  after it when `after`), counted in the list without `id`. */
export function dropPosition(pins: readonly string[], id: string, target: string, after: boolean): number | null {
  if (id === target) return null;
  const rest = pins.filter(p => p !== id);
  const at = rest.indexOf(target);
  if (at < 0) return null;
  const position = at + (after ? 1 : 0) + 1;
  // Dropped where it already is: nothing to do.
  return pins.indexOf(id) === position - 1 ? null : position;
}

/** Where Move up (`dir` -1) or Move down (1) puts `id`: the 1-based place
 *  among all pins that steps it over the next pin the sidebar shows, so
 *  pins hidden from the sidebar never make a move look like nothing
 *  (ENG-116). A pin that is itself hidden steps among all of them. Null at
 *  either end. */
export function pinStep(pins: readonly string[], hidden: readonly string[], id: string, dir: -1 | 1): number | null {
  const shown = hidden.includes(id) ? pins : pins.filter(p => p === id || !hidden.includes(p));
  const at = shown.indexOf(id);
  const next = at < 0 ? undefined : shown[at + dir];
  return next === undefined ? null : pins.indexOf(next) + 1;
}

/** What a pin change says to a screen reader (and the snackbar). */
export function pinAnnouncement(kind: 'pin' | 'unpin' | 'move', title: string, position?: number, count?: number): string {
  if (kind === 'pin') return `Pinned ${title}`;
  if (kind === 'unpin') return `Unpinned ${title}`;
  return `Moved ${title} to ${position} of ${count} in Pinned`;
}
