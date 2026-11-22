/**
 * Small per-machine preferences that used to live inside single components
 * (PR A4). Each has one signal and one setter here, the only writer of its
 * localStorage key, so a component re-renders when anything changes it: the
 * user's click or a Hollow app tools change arriving through `appBridge.ts`.
 */
import { createSignal } from 'solid-js';

/** localStorage key shared with Settings, Usage's Claude plan card: nothing
 *  may call `claude_quota` (which can read the Keychain) before it is set.
 *  Here, not in chatUsage.ts, so the shell chunk doesn't carry that module. */
export const QUOTA_OPT_IN_KEY = 'claudeQuotaOptIn';

function read<T>(key: string, parse: (raw: string | null) => T): T {
  try { return parse(localStorage.getItem(key)); } catch { return parse(null); }
}
function write(key: string, value: string | null) {
  try {
    if (value === null) localStorage.removeItem(key); else localStorage.setItem(key, value);
  } catch { /* memory still holds it */ }
}
const idList = (raw: string | null): string[] => {
  try {
    const v = JSON.parse(raw ?? '[]');
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [];
  } catch { return []; }
};

/** Claude plan usage: shown only after the user opts in (it reads the Claude
 *  login from the Keychain and asks Anthropic). */
export const [quotaOptIn, setQuotaOptInSignal] = createSignal(read(QUOTA_OPT_IN_KEY, v => v === '1'));
export function setQuotaOptIn(on: boolean) {
  setQuotaOptInSignal(on);
  write(QUOTA_OPT_IN_KEY, on ? '1' : null);
}

/** The notification center's filter. */
export const NOTIF_FILTERS = ['all', 'attention', 'routines', 'nudges', 'agent'] as const;
export type NotifFilter = typeof NOTIF_FILTERS[number];
const FILTER_KEY = 'notifFilter';
export const [notifFilter, setNotifFilterSignal] = createSignal<NotifFilter>(
  read(FILTER_KEY, v => (NOTIF_FILTERS as readonly string[]).includes(v ?? '') ? v as NotifFilter : 'all'),
);
export function setNotifFilter(id: NotifFilter) {
  setNotifFilterSignal(id);
  write(FILTER_KEY, id);
}

/** The command palette's recent items, newest first. */
const RECENTS_KEY = 'paletteRecents';
export const [paletteRecents, setPaletteRecentsSignal] = createSignal<string[]>(read(RECENTS_KEY, idList));
export function setPaletteRecents(ids: string[]) {
  const next = ids.slice(0, 24);
  setPaletteRecentsSignal(next);
  write(RECENTS_KEY, JSON.stringify(next));
}
export function recordPaletteRecent(id: string) {
  setPaletteRecents([id, ...paletteRecents().filter(x => x !== id)]);
}

/** Workspaces whose conversations were folded in the rail. The rail lists
 *  no conversations since L8 (11-10); the key stays readable and writable
 *  through the app bridge so a saved settings file still loads. */
const COLLAPSED_KEY = 'hollow.railCollapsed';
export const [railCollapsed, setRailCollapsedSignal] = createSignal<string[]>(read(COLLAPSED_KEY, idList));
export function setRailCollapsed(ids: string[]) {
  setRailCollapsedSignal(ids);
  write(COLLAPSED_KEY, JSON.stringify(ids));
}
/** The sidebar's folded sections (overnight 11-04 S1): `projects`. Its
 *  `projects-more` (Show N more) went in L1, 11-12: a saved one is ignored. */
const RAIL_SECTIONS_KEY = 'hollow.railSections.v1';
export const [railFoldedSections, setRailFoldedSectionsSignal] = createSignal<string[]>(read(RAIL_SECTIONS_KEY, idList));
export function toggleRailSection(id: string) {
  const ids = railFoldedSections();
  const next = ids.includes(id) ? ids.filter(x => x !== id) : [...ids, id];
  setRailFoldedSectionsSignal(next);
  write(RAIL_SECTIONS_KEY, next.length ? JSON.stringify(next) : null);
}

/** The sidebar's width, dragged at its right edge (overnight 11-04 S1);
 *  null is the stylesheet's `--rail-w`. Double-click the edge to reset. */
export const RAIL_WIDTH_DEFAULT = 240;
export const RAIL_WIDTH_MIN = 200;
export const RAIL_WIDTH_MAX = 420;
const RAIL_WIDTH_KEY = 'hollow.railWidth';
export const clampRailWidth = (w: number) => Math.round(Math.max(RAIL_WIDTH_MIN, Math.min(RAIL_WIDTH_MAX, w)));
export const [railWidth, setRailWidthSignal] = createSignal<number | null>(read(RAIL_WIDTH_KEY, raw => {
  const v = Number(raw);
  return raw && Number.isFinite(v) && v > 0 ? clampRailWidth(v) : null;
}));
/** While dragging: the width only. `persistRailWidth` saves it once. */
export function setRailWidth(w: number | null) { setRailWidthSignal(w == null ? null : clampRailWidth(w)); }
export function persistRailWidth() {
  const w = railWidth();
  write(RAIL_WIDTH_KEY, w == null || w === RAIL_WIDTH_DEFAULT ? null : String(w));
}

/** The user backs the vault up some other way (Settings, Advanced). */
const BACKUP_KEY = 'backupManagedElsewhere.v1';
export const [backupManagedElsewhere, setBackupManagedSignal] = createSignal(read(BACKUP_KEY, v => v === 'true'));
export function setBackupManagedElsewhere(on: boolean) {
  setBackupManagedSignal(on);
  write(BACKUP_KEY, String(on));
}

/** Two-finger trackpad swipe steps Back and Forward (historySwipe.ts). On by
 *  default; the buttons and ⌘[ ⌘] work either way. */
const SWIPE_NAV_KEY = 'hollow.nav.swipe';
export const [swipeNav, setSwipeNavSignal] = createSignal(read(SWIPE_NAV_KEY, v => v !== '0'));
export function setSwipeNav(on: boolean) {
  setSwipeNavSignal(on);
  write(SWIPE_NAV_KEY, on ? null : '0');
}

/** Composer send key. On (the default, as in Claude, Codex and Conductor):
 *  Enter sends and Shift/Option+Enter starts a new line. Off: Enter is a new
 *  line and only ⌘Enter sends. ⌘Enter sends either way. */
const ENTER_SENDS_KEY = 'hollow.chat.enterSends';
export const [enterSends, setEnterSendsSignal] = createSignal(read(ENTER_SENDS_KEY, v => v !== '0'));
export function setEnterSends(on: boolean) {
  setEnterSendsSignal(on);
  write(ENTER_SENDS_KEY, on ? null : '0');
}
