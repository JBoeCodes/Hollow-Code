/**
 * Pinned workspaces (2026-10-21): the one pin state, read from the
 * workspace list (each brief's `pinned`) and written through
 * `update_workspace`, which is compare-and-swap on the brief. The sidebar's
 * Pinned group, Home's rows and filter and the workspace menus all use this.
 *
 * It replaces the window-local `hollow.studio.pins` list Home kept before; a
 * list left from then is moved into the briefs once ([`adoptLegacyPins`]).
 */
import { api } from '../api';
import { flashFooter } from './runtime';
import { lifecycleFailed, projectWorkspaces, refreshWorkspaces, workspaceListLoaded } from './workspaces';
import { canPinWorkspace, pinAnnouncement, pinOrder } from '../workspacePins';
import type { WorkWorkspace } from '../workspaceTypes';

let orderFrom: readonly WorkWorkspace[] | null = null;
let orderList: string[] = [];
/** The pinned workspaces' ids, in pin order. */
export function pinnedWorkspaces(): string[] {
  const all = projectWorkspaces();
  if (all !== orderFrom) { orderFrom = all; orderList = pinOrder(all); }
  return orderList;
}
export const isPinnedWorkspace = (id: string) => pinnedWorkspaces().includes(id);

async function change(w: WorkWorkspace, update: { pinned?: boolean; pinPosition?: number }, done: () => string) {
  try {
    await api.updateWorkspace(w.id, update);
    await refreshWorkspaces();
    flashFooter(done());
  } catch (e) {
    // Pin and Unpin run from the sidebar on any page (P1-41): banner plus toast.
    lifecycleFailed(String(e));
    // A move that stopped partway may have renumbered some pins already.
    void refreshWorkspaces();
  }
}

export function pinWorkspace(w: WorkWorkspace) {
  return change(w, { pinned: true }, () => pinAnnouncement('pin', w.title));
}
export function unpinWorkspace(w: WorkWorkspace) {
  return change(w, { pinned: false }, () => pinAnnouncement('unpin', w.title));
}
/** Pin `w`, or move its pin, to 1-based `position` among the pins. */
export function moveWorkspacePin(w: WorkWorkspace, position: number) {
  return change(w, { pinPosition: position }, () => pinAnnouncement('move', w.title, pinnedWorkspaces().indexOf(w.id) + 1, pinnedWorkspaces().length));
}
export function toggleWorkspacePin(w: WorkWorkspace) {
  return isPinnedWorkspace(w.id) ? unpinWorkspace(w) : pinWorkspace(w);
}

const LEGACY_KEY = 'hollow.studio.pins';
let adopting = false;
/** Pins Home kept in this window before they moved into the briefs: pin each
 *  (in its order, after any already pinned) and drop the list once every
 *  write succeeded. Only after the workspace list has loaded and isn't empty,
 *  so a failed first read can't make every old pin look gone. */
export async function adoptLegacyPins() {
  let raw: string | null = null;
  try { raw = localStorage.getItem(LEGACY_KEY); } catch { return; }
  if (raw == null || adopting || !workspaceListLoaded() || !projectWorkspaces().length) return;
  adopting = true;
  try {
    let ids: unknown = [];
    try { ids = JSON.parse(raw); } catch { /* unreadable: nothing to adopt */ }
    const byId = new Map(projectWorkspaces().map(w => [w.id, w]));
    let ok = true;
    for (const id of Array.isArray(ids) ? [...new Set(ids)] : []) {
      const w = typeof id === 'string' ? byId.get(id) : undefined;
      if (!w || !canPinWorkspace(w) || w.pinned != null) continue;
      try { await api.updateWorkspace(w.id, { pinned: true }); } catch { ok = false; }
    }
    if (ok) { try { localStorage.removeItem(LEGACY_KEY); } catch { /* try again next launch */ } }
    await refreshWorkspaces();
  } finally { adopting = false; }
}
