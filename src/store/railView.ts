// The Quick chats list's current view and the saved ones (3.5). The rules are pure in
// `railView.ts`; both persist in localStorage on this Mac.
import { createSignal } from 'solid-js';
import { DEFAULT_RAIL_VIEW, addSavedView, readRailView, readSavedRailViews, removeSavedView, type RailView, type SavedRailView } from '../railView';

const VIEW_KEY = 'hollow.railView.v1';
const SAVED_KEY = 'hollow.railViews.v1';

function get(key: string): string | null { try { return localStorage.getItem(key); } catch { return null; } }
function put(key: string, value: string | null) { try { if (value === null) localStorage.removeItem(key); else localStorage.setItem(key, value); } catch { /* memory still holds it */ } }

export const [railView, setRailViewSignal] = createSignal<RailView>(readRailView(get(VIEW_KEY)));
export const [savedRailViews, setSavedRailViews] = createSignal<SavedRailView[]>(readSavedRailViews(get(SAVED_KEY)));

export function setRailView(next: RailView) {
  setRailViewSignal(next);
  put(VIEW_KEY, JSON.stringify(next) === JSON.stringify(DEFAULT_RAIL_VIEW) ? null : JSON.stringify(next));
}
export const patchRailView = (patch: Partial<RailView>) => setRailView({ ...railView(), ...patch });
export const resetRailView = () => setRailView({ ...DEFAULT_RAIL_VIEW });

export function saveCurrentRailView(name: string) {
  const next = addSavedView(savedRailViews(), name, railView());
  setSavedRailViews(next);
  put(SAVED_KEY, next.length ? JSON.stringify(next) : null);
}
export function deleteSavedRailView(id: string) {
  const next = removeSavedView(savedRailViews(), id);
  setSavedRailViews(next);
  put(SAVED_KEY, next.length ? JSON.stringify(next) : null);
}
export function applySavedRailView(id: string) {
  const v = savedRailViews().find(x => x.id === id);
  if (v) setRailView({ sort: v.sort, filter: v.filter, group: v.group });
}
