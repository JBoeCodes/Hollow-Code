// Keyboard focus for the hand-made dialogs (overnight B8): Tab stays inside
// the dialog, and closing it puts focus back where it was. ConfirmDialog keeps
// its own fixed-order trap; these are the `rbk-`/`wsmap-` cards.

/** What Tab can land on. */
export const FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** The stop Tab moves to in a ring of `count`: forward from `at`, or back
 *  with Shift. From outside the ring (`at` < 0) it enters at the first stop,
 *  or the last going back. */
export function nextInRing(count: number, at: number, back: boolean): number {
  if (count <= 0) return -1;
  if (at < 0) return back ? count - 1 : 0;
  return (at + (back ? count - 1 : 1)) % count;
}

/** Keep Tab inside `container` and remember what had focus; the returned
 *  function stops that and puts focus back (if that element still exists).
 *  Listens on the document, so focus that fell to the page (a button that
 *  got disabled or removed) is pulled back in; yields while focus is in
 *  another modal on top (a confirm, the palette). */
export function holdFocus(container: HTMLElement): () => void {
  const before = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const onKey = (e: KeyboardEvent) => {
    if (e.key !== 'Tab' || e.defaultPrevented) return;
    const active = document.activeElement;
    const otherModal = active instanceof Element ? active.closest('[aria-modal="true"]') : null;
    if (otherModal && otherModal !== container && !container.contains(otherModal)) return;
    const stops = [...container.querySelectorAll<HTMLElement>(FOCUSABLE)]
      .filter(el => el.getClientRects().length > 0);
    e.preventDefault();
    const next = nextInRing(stops.length, stops.indexOf(document.activeElement as HTMLElement), e.shiftKey);
    if (next >= 0) stops[next].focus();
  };
  document.addEventListener('keydown', onKey, true);
  return () => {
    document.removeEventListener('keydown', onKey, true);
    if (before?.isConnected) before.focus();
  };
}
