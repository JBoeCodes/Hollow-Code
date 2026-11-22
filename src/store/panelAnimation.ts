// Panel-animation duration (0.2, 2026-11-20): one persisted setting, 0 to 400 ms,
// default 0 (no panel motion). It is the value `--panel-duration` reads (tokens.css).
// Panels animate only when the duration is positive, motion is not reduced (the app
// setting or the system preference) and navigation is not suppressing it. Suppression
// lasts until two animation frames after the new page paints, so a page change never
// slides panels into place. A closing panel stays mounted for its exit duration and a
// reopen cancels the exit. This is not every animation: only panels read it.
import { createEffect, createSignal, on, onCleanup, type Accessor } from 'solid-js';
import { motionReduced } from '../motion';

export const PANEL_ANIMATION_MAX_MS = 400;
export const PANEL_ANIMATION_DEFAULT_MS = 0;
const KEY = 'hollow.studio.panelAnimationMs';

/** A whole number of milliseconds in 0..400; anything unreadable is the default. */
export function normalizePanelDuration(value: unknown): number {
  const n = typeof value === 'number' ? value : typeof value === 'string' && value.trim() !== '' ? Number(value) : NaN;
  if (!Number.isFinite(n)) return PANEL_ANIMATION_DEFAULT_MS;
  return Math.min(PANEL_ANIMATION_MAX_MS, Math.max(0, Math.round(n)));
}

function stored(): string | null { try { return localStorage.getItem(KEY); } catch { return null; } }
function save(ms: number) { try { localStorage.setItem(KEY, String(ms)); } catch { /* session value still works */ } }
function root(): HTMLElement | undefined { return typeof document === 'undefined' ? undefined : document.documentElement; }

/** The saved duration in ms (not yet gated by reduced motion or navigation). */
export const [panelAnimationMs, writeMs] = createSignal(normalizePanelDuration(stored()));
const [suppressed, setSuppressed] = createSignal(false);
export const panelAnimationSuppressed = suppressed;

function mirror(ms: number) { root()?.style?.setProperty('--panel-duration-user', `${ms}ms`); }
mirror(panelAnimationMs());

export function setPanelAnimationMs(value: unknown) {
  const ms = normalizePanelDuration(value);
  writeMs(ms); save(ms); mirror(ms);
}

/** The duration a panel may use right now: 0 when reduced, suppressed or off. */
export function panelDurationMs(
  setting = panelAnimationMs(), reduced = motionReduced(), navigating = suppressed(),
): number {
  return reduced || navigating ? 0 : normalizePanelDuration(setting);
}

let token = 0;
/** Hold panel motion off for a navigation: until two frames after the next paint.
 *  `raf` is injectable for tests; with no animation frames it clears at once. */
export function suppressPanelAnimation(raf: ((cb: () => void) => unknown) | undefined =
  typeof requestAnimationFrame === 'function' ? cb => requestAnimationFrame(() => cb()) : undefined) {
  const mine = ++token;
  setSuppressed(true); root()?.setAttribute?.('data-panel-still', '');
  const clear = () => { if (mine === token) { setSuppressed(false); root()?.removeAttribute?.('data-panel-still'); } };
  if (!raf) { clear(); return; }
  raf(() => raf(clear));
}

/** Presence for a panel that animates out: `mounted` stays true for the exit duration
 *  after `open` turns false, and a reopen during the exit cancels it. A duration of 0
 *  unmounts at once. `closing` is true during the exit, for the exit style. */
export function createPanelPresence(open: Accessor<boolean>, ms: () => number = () => panelDurationMs()) {
  const [mounted, setMounted] = createSignal(open());
  const [closing, setClosing] = createSignal(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const stop = () => { if (timer !== undefined) { clearTimeout(timer); timer = undefined; } };
  createEffect(on(open, isOpen => {
    stop();
    if (isOpen) { setMounted(true); setClosing(false); return; }
    if (!mounted()) return;
    const d = ms();
    if (d <= 0) { setMounted(false); setClosing(false); return; }
    setClosing(true);
    timer = setTimeout(() => { timer = undefined; setMounted(false); setClosing(false); }, d);
  }, { defer: true }));
  onCleanup(stop);
  return { mounted, closing };
}
