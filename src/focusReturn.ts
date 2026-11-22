// Focus back to the opener when an overlay is dismissed (H12, matrix V7c
// "nested overlays"): the command palette dismissed with Escape over the Main
// agent drawer left focus on the page body, so the next key went nowhere.
// Its own module, imported only by lazy chunks (the palette; the Preview
// pane since run 15 P1), so none of it joins the launch chunk (focusHold.ts
// is reached from Home).
import { TRANSITIONING } from './viewTransitions';

/** Where focus sits as an overlay closes: on nothing (the page body), still
 *  inside the closing overlay, or somewhere the person (or an action) put it. */
export type FocusNow = 'body' | 'overlay' | 'elsewhere';

/** Whether a dismissed overlay gives focus back to the element that opened
 *  it: only while that element is still on the page, and only if focus hasn't
 *  gone somewhere real meanwhile. */
export function returnsFocus(openerConnected: boolean, now: FocusNow): boolean {
  return openerConnected && now !== 'elsewhere';
}

/** Whether a row that ran from the overlay gives focus back to its opener
 *  (A05): only while the location stamp read before the row ran still holds,
 *  the opener is still there and focus is on the page body. A row that
 *  changed the page, workspace or view leaves focus to where it went (#652
 *  dropped a blind return because it stole focus from rows that navigate).
 *  By then the run's overlay is gone, so focus inside a matching overlay
 *  means the row opened it again (`act:jump`): that is left alone too. */
export function returnsFocusAfterRun(before: string, after: string, openerConnected: boolean, now: FocusNow): boolean {
  return before === after && openerConnected && now === 'body';
}

/** What had focus as the overlay opened, unless it is inside the overlay. */
export function focusOpener(overlaySel: string): HTMLElement | null {
  const active = document.activeElement;
  return active instanceof HTMLElement && active !== document.body && !active.closest(overlaySel) ? active : null;
}

function focusNow(overlaySel: string): FocusNow {
  const active = document.activeElement;
  return !active || active === document.body ? 'body' : active.closest(overlaySel) ? 'overlay' : 'elsewhere';
}

/** Give focus back to `opener` after the overlay matching `overlaySel` was
 *  dismissed, in a microtask so the overlay's own close (a blur) runs first. */
export function giveFocusBack(opener: HTMLElement | null, overlaySel: string): void {
  queueMicrotask(() => {
    if (opener && returnsFocus(opener.isConnected, focusNow(overlaySel))) opener.focus({ preventScroll: true });
  });
}

/** Past the palette's 140 ms exit hold, matching its recordRecent delay. */
const AFTER_RUN_MS = 180;

/** Give focus back to `opener` after a row run from the overlay returns
 *  (A05; call it from the run's `finally`): past the overlay's exit hold and
 *  one frame more, so a view the row opened has mounted and focused, and
 *  only while `readStamp()` still equals `stampBefore` (`returnsFocusAfterRun`).
 *  Shared action rows (`runAction`) return at once and run on their own, so
 *  for them the wait is this delay, not the action's end: a row that loads
 *  for longer and then navigates may see focus come back first, then move to
 *  where it lands. A page change still in its view transition, whose update
 *  may not have moved the stamp yet (held up to START_LIMIT_MS), is left
 *  alone. */
export function giveFocusBackAfterRun(opener: HTMLElement | null, overlaySel: string, stampBefore: string, readStamp: () => string): void {
  if (!opener) return;
  setTimeout(() => requestAnimationFrame(() => {
    if (document.documentElement.classList.contains(TRANSITIONING)) return;
    const now = focusNow(overlaySel);
    if (returnsFocusAfterRun(stampBefore, readStamp(), opener.isConnected, now)) opener.focus({ preventScroll: true });
  }), AFTER_RUN_MS);
}
